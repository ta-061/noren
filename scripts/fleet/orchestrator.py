#!/usr/bin/env python3
"""Event-driven Noren fleet orchestrator.

A plain Python process — not a model. It reads the task queue, resolves
dependencies, checks quota, assigns worktrees and file leases, dispatches lanes,
watches for real state changes, and runs the merge gate. It escalates to Claude
Code only for decisions, never for waiting.

What it deliberately does NOT do: architecture decisions, ignoring a BLOCKER,
merging with unresolved threads or without review, pushing to main, touching
credentials, assigning two writers to one file, or believing a lane's
self-report as proof of completion.

    orchestrator.py start        run the loop (foreground; use nohup for bg)
    orchestrator.py status       print queue, lanes, leases, escalations
    orchestrator.py stop         request a graceful stop
    orchestrator.py once         one reconcile pass, then exit (for testing)
"""
from __future__ import annotations

import hashlib
import json
import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
FLEET = REPO / ".fleet"
QUEUE = FLEET / "queue.json"
EVENTS = FLEET / "events"
LOCKS = FLEET / "locks"
LOGS = FLEET / "logs"
PACKETS = FLEET / "decision-packets"
SESSIONS = FLEET / "sessions"
PIDFILE = FLEET / "orchestrator.pid"
STOPFILE = FLEET / "orchestrator.stop"
SEEN = FLEET / "seen-events.json"
TASKS_DIR = REPO / "docs" / "coordination" / "tasks"
HANDOFF_DIR = REPO / "docs" / "coordination" / "handoffs"

# Concurrency policy: target 6, floor 4, ceiling 8. Never invent duplicate work
# just to reach the target.
TARGET_LANES = 6
MAX_LANES = 8

# Files where two concurrent writers reliably conflict. Only a lane holding the
# integration lease may edit these, and only one such lane runs at a time.
INTEGRATION_PATHS = {
    "crates/noren-terminal/src/lib.rs",
    "crates/noren-app/src/lib.rs",
    "crates/noren-pty/src/lib.rs",
    "crates/noren-app/src/main.rs",
    "Cargo.toml",
    "Cargo.lock",
}

POLL_SECONDS = 60
# A lane whose log has not grown in this long and which produced no branch is
# treated as an init stall (died before reaching the model), not a slow start.
INIT_STALL_SECONDS = 240
INIT_STALL_MAX_BYTES = 4096


def log(msg: str) -> None:
    print("[%s] %s" % (time.strftime("%Y-%m-%dT%H:%M:%S"), msg), flush=True)


def sh(args, cwd=None, timeout=180):
    """Run a command, returning (rc, stdout). Never raises on non-zero."""
    try:
        p = subprocess.run(args, cwd=cwd or REPO, capture_output=True,
                           text=True, timeout=timeout)
        return p.returncode, (p.stdout or "") + (p.stderr or "")
    except subprocess.TimeoutExpired:
        return 124, "timeout"
    except FileNotFoundError as exc:
        return 127, str(exc)


def load_json(path: Path, default):
    try:
        return json.loads(path.read_text())
    except Exception:
        return default


def save_json(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(value, indent=2, ensure_ascii=False))
    tmp.replace(path)


# --------------------------------------------------------------------------
# Leases: the mechanism that makes parallel lanes safe.
# --------------------------------------------------------------------------

def held_leases() -> dict:
    """path -> lane, for every currently held file lease."""
    out = {}
    for f in LOCKS.glob("*.json"):
        rec = load_json(f, {})
        for path in rec.get("paths", []):
            out[path] = rec.get("lane")
    return out


def lease_conflict(task: dict) -> str | None:
    """Return a human reason if this task cannot take its lease right now."""
    want = set(task.get("file_lease", []))
    if not want:
        return "task declares no file lease"
    held = held_leases()
    for path in want:
        owner = held.get(path)
        if owner and owner != task["lane"]:
            return "lease on %s held by %s" % (path, owner)
    integration = want & INTEGRATION_PATHS
    if integration:
        # Only one integration-lease lane at a time, and only if the task says
        # it holds that lease.
        if not task.get("integration_lease"):
            return "touches integration path %s without integration_lease" % sorted(integration)
        for f in LOCKS.glob("*.json"):
            rec = load_json(f, {})
            if rec.get("integration_lease") and rec.get("lane") != task["lane"]:
                return "integration lease held by %s" % rec.get("lane")
    return None


def take_lease(task: dict) -> None:
    save_json(LOCKS / ("%s.json" % task["lane"]), {
        "lane": task["lane"],
        "task": task["id"],
        "paths": task.get("file_lease", []),
        "integration_lease": bool(task.get("integration_lease")),
        "taken_at": time.time(),
    })


def release_lease(lane: str) -> None:
    f = LOCKS / ("%s.json" % lane)
    if f.exists():
        f.unlink()


# --------------------------------------------------------------------------
# Events: emitted once each, deduplicated by hash.
# --------------------------------------------------------------------------

def emit(kind: str, task_id: str, detail: dict, needs_claude: bool) -> bool:
    """Record an event. Returns True if it is new (not already emitted)."""
    key = hashlib.sha256(
        json.dumps([kind, task_id, detail.get("hash_key", detail)],
                   sort_keys=True, default=str).encode()).hexdigest()[:16]
    seen = load_json(SEEN, {})
    if key in seen:
        return False
    seen[key] = {"kind": kind, "task": task_id, "at": time.time()}
    save_json(SEEN, seen)
    rec = {"kind": kind, "task": task_id, "needs_claude": needs_claude,
           "at": time.time(), "detail": detail}
    EVENTS.mkdir(parents=True, exist_ok=True)
    (EVENTS / ("%s-%s.json" % (int(time.time()), key))).write_text(
        json.dumps(rec, indent=2, ensure_ascii=False, default=str))
    log("event %s task=%s needs_claude=%s" % (kind, task_id, needs_claude))
    if needs_claude:
        write_packet(kind, task_id, detail)
    return True


def write_packet(kind: str, task_id: str, detail: dict) -> None:
    """A decision packet: the minimum a coordinator needs to decide."""
    PACKETS.mkdir(parents=True, exist_ok=True)
    p = PACKETS / ("%s-%s.md" % (task_id, kind))
    lines = [
        "# Decision packet: %s" % kind,
        "",
        "- Task: `%s`" % task_id,
        "- Raised: %s" % time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "",
        "## Why this needs a decision",
        detail.get("why", "(unstated)"),
        "",
        "## State",
    ]
    for k in ("base_sha", "head_sha", "branch", "pr", "diff_summary",
              "ci_summary", "review_findings", "gate"):
        if detail.get(k):
            lines.append("- %s: %s" % (k, detail[k]))
    if detail.get("options"):
        lines += ["", "## Options"] + ["- %s" % o for o in detail["options"]]
    if detail.get("recommended"):
        lines += ["", "## Recommended", detail["recommended"]]
    if detail.get("evidence"):
        lines += ["", "## Evidence paths"] + ["- `%s`" % e for e in detail["evidence"]]
    p.write_text("\n".join(lines) + "\n")


# --------------------------------------------------------------------------
# Lane execution
# --------------------------------------------------------------------------

def quota_ok(account: str) -> bool:
    rc, _ = sh(["python3", str(REPO / "scripts/fleet/quota.py"), "--gate", account])
    return rc == 0


def lane_running(lane: str) -> bool:
    rc, out = sh(["ps", "ax", "-o", "command="])
    return ("dispatch.sh %s " % lane) in out


def newest_log(lane: str) -> Path | None:
    files = sorted(LOGS.glob("%s.*.log" % lane), key=lambda f: f.stat().st_mtime)
    return files[-1] if files else None


def branch_exists(branch: str) -> bool:
    rc, _ = sh(["git", "rev-parse", "--verify", "--quiet", "refs/heads/%s" % branch])
    return rc == 0


def dispatch(task: dict) -> None:
    lane = task["lane"]
    wt = task["worktree"]
    prompt = REPO / task["prompt"]
    if not Path(wt).exists():
        rc, out = sh(["git", "worktree", "add", "-q", "-b",
                      "pool/%s" % lane, wt, "origin/main"])
        if rc != 0:
            log("worktree add failed for %s: %s" % (lane, out.strip()[:200]))
    take_lease(task)
    # Absolute paths only: a relative prompt path silently resolves to nothing
    # when the dispatcher is invoked from a worktree.
    cmd = [str(REPO / "scripts/fleet/dispatch.sh"), lane, wt, str(prompt)]
    logf = open(LOGS / ("%s.orchestrator.log" % lane), "a")
    subprocess.Popen(cmd, cwd=str(REPO), stdout=logf, stderr=logf,
                     start_new_session=True)
    task["state"] = "running"
    task["dispatched_at"] = time.time()
    log("dispatched %s task=%s worktree=%s" % (lane, task["id"], wt))


def classify_stall(task: dict) -> str | None:
    """Distinguish an init stall from a permission stall from healthy work.

    An init stall means the lane died before reaching the model: tiny log, no
    branch, no growth. Relaunching costs nothing. A permission stall means the
    lane is blocked on an interactive prompt but has usually already finished
    its work, so its branch must be inspected before discarding anything.
    """
    lf = newest_log(task["lane"])
    if lf is None:
        return None
    size = lf.stat().st_size
    age = time.time() - lf.stat().st_mtime
    if age < INIT_STALL_SECONDS:
        return None
    has_branch = branch_exists(task.get("branch", ""))
    text = ""
    try:
        text = lf.read_text(errors="replace")[-4000:]
    except Exception:
        pass
    if "message=asking" in text or "permission=" in text:
        return "permission_stall"
    if size <= INIT_STALL_MAX_BYTES and not has_branch:
        return "init_stall"
    return None


def verdict_of(task: dict) -> str | None:
    lf = newest_log(task["lane"])
    if lf is None or not task.get("verdict_token"):
        return None
    try:
        text = lf.read_text(errors="replace")
    except Exception:
        return None
    for line in reversed(text.splitlines()):
        if task["verdict_token"] in line:
            return line.strip()
    return None


# --------------------------------------------------------------------------
# PR gating
# --------------------------------------------------------------------------

def gate(pr: int) -> tuple[bool, str]:
    rc, out = sh(["python3", str(REPO / "scripts/fleet/merge_gate.py"), str(pr)])
    return rc == 0, out.strip()


def reconcile(state: dict) -> None:
    tasks = state["tasks"]
    running = [t for t in tasks if t.get("state") == "running"]

    # 1. Lanes that finished, stalled, or produced a verdict.
    for t in list(running):
        if lane_running(t["lane"]):
            kind = classify_stall(t)
            if kind == "init_stall":
                log("%s init-stalled; relaunching (nothing produced)" % t["lane"])
                sh(["pkill", "-f", "dispatch.sh %s " % t["lane"]])
                release_lease(t["lane"])
                t["state"] = "queued"
                t["relaunches"] = t.get("relaunches", 0) + 1
                emit("lane_init_stall", t["id"], {
                    "why": "Lane died during init; relaunching.",
                    "hash_key": "%s-%s" % (t["lane"], t.get("relaunches")),
                }, needs_claude=False)
            elif kind == "permission_stall":
                emit("lane_permission_stall", t["id"], {
                    "why": ("Lane is blocked on an interactive permission prompt. "
                            "Its branch usually already holds finished work — "
                            "inspect before discarding."),
                    "branch": t.get("branch"),
                    "evidence": [str(newest_log(t["lane"]))],
                }, needs_claude=True)
            continue
        # Lane process is gone.
        v = verdict_of(t)
        release_lease(t["lane"])
        if v:
            t["state"] = "lane_done"
            t["verdict"] = v
            emit("lane_done", t["id"], {
                "why": "Lane reported a verdict; needs independent verification.",
                "verdict": v, "branch": t.get("branch"),
                "hash_key": v,
            }, needs_claude=False)
        else:
            t["state"] = "lane_ended_no_verdict"
            emit("lane_no_verdict", t["id"], {
                "why": ("Lane exited without printing its verdict line. Check "
                        "whether it committed work before concluding it failed."),
                "branch": t.get("branch"),
                "evidence": [str(newest_log(t["lane"]) or "")],
                "hash_key": "%s-%s" % (t["lane"], t.get("dispatched_at")),
            }, needs_claude=True)

    # 2. PR gating for tasks that have one.
    for t in tasks:
        pr = t.get("pr")
        if not pr or t.get("state") in ("merged", "closed"):
            continue
        ok, detail = gate(int(pr))
        if ok:
            emit("ready_to_merge", t["id"], {
                "why": "merge_gate.py PASS — checks green, review on head, 0 unresolved.",
                "pr": pr, "gate": detail, "hash_key": detail,
            }, needs_claude=True)
        elif "not successful" in detail:
            emit("ci_failed", t["id"], {
                "why": ("A required check failed. Read the job log before assuming "
                        "a code defect — infrastructure errors look identical here."),
                "pr": pr, "ci_summary": detail, "hash_key": detail,
            }, needs_claude=True)

    # 3. Dispatch queued tasks whose dependencies and leases allow it.
    active = sum(1 for t in tasks if t.get("state") == "running")
    done_ids = {t["id"] for t in tasks
                if t.get("state") in ("lane_done", "verified", "merged")}
    for t in tasks:
        if active >= min(TARGET_LANES, MAX_LANES):
            break
        if t.get("state") != "queued":
            continue
        missing = [d for d in t.get("depends_on", []) if d not in done_ids]
        if missing:
            continue
        if not quota_ok(t["account"]):
            emit("quota_blocked", t["id"], {
                "why": "Engine has no headroom; will retry when it recovers.",
                "hash_key": "%s-%s" % (t["account"], time.strftime("%Y%m%d%H")),
            }, needs_claude=False)
            continue
        reason = lease_conflict(t)
        if reason:
            emit("lease_conflict", t["id"], {
                "why": "Cannot dispatch: %s" % reason,
                "hash_key": "%s-%s" % (t["id"], reason),
                "options": ["re-scope the lease", "sequence behind the holder"],
            }, needs_claude=True)
            continue
        dispatch(t)
        active += 1

    save_json(QUEUE, state)


def cmd_status() -> int:
    state = load_json(QUEUE, {"tasks": []})
    pid = PIDFILE.read_text().strip() if PIDFILE.exists() else "-"
    alive = "no"
    if pid.isdigit():
        rc, _ = sh(["kill", "-0", pid])
        alive = "yes" if rc == 0 else "no"
    print("orchestrator pid=%s alive=%s" % (pid, alive))
    by = {}
    for t in state["tasks"]:
        by.setdefault(t.get("state", "?"), []).append(t["id"])
    for k in sorted(by):
        print("  %-22s %s" % (k, ", ".join(sorted(by[k]))))
    leases = held_leases()
    if leases:
        print("file leases:")
        for path, lane in sorted(leases.items()):
            print("  %-52s %s" % (path, lane))
    pend = sorted(PACKETS.glob("*.md"))
    print("decision packets awaiting coordinator: %d" % len(pend))
    for p in pend:
        print("  %s" % p.name)
    return 0


def cmd_start(once: bool = False) -> int:
    if PIDFILE.exists():
        old = PIDFILE.read_text().strip()
        if old.isdigit():
            rc, _ = sh(["kill", "-0", old])
            if rc == 0:
                log("already running as pid %s; refusing to double-start" % old)
                return 1
    for d in (EVENTS, LOCKS, LOGS, PACKETS, SESSIONS):
        d.mkdir(parents=True, exist_ok=True)
    if STOPFILE.exists():
        STOPFILE.unlink()
    PIDFILE.write_text(str(os.getpid()))

    stopping = {"v": False}

    def handle(signum, frame):
        stopping["v"] = True
        log("signal %s received; stopping after this pass" % signum)

    signal.signal(signal.SIGTERM, handle)
    signal.signal(signal.SIGINT, handle)

    log("orchestrator started pid=%d repo=%s" % (os.getpid(), REPO))
    try:
        while True:
            state = load_json(QUEUE, {"tasks": []})
            try:
                reconcile(state)
            except Exception as exc:  # keep the loop alive on transient errors
                log("reconcile error: %r" % exc)
            if once or stopping["v"] or STOPFILE.exists():
                break
            time.sleep(POLL_SECONDS)
    finally:
        if PIDFILE.exists():
            PIDFILE.unlink()
        log("orchestrator stopped")
    return 0


def cmd_stop() -> int:
    STOPFILE.write_text("stop")
    if PIDFILE.exists():
        pid = PIDFILE.read_text().strip()
        if pid.isdigit():
            sh(["kill", "-TERM", pid])
            print("graceful stop requested for pid %s" % pid)
            return 0
    print("no running orchestrator; stop flag written")
    return 0


def main() -> int:
    cmd = sys.argv[1] if len(sys.argv) > 1 else "status"
    if cmd == "start":
        return cmd_start()
    if cmd == "once":
        return cmd_start(once=True)
    if cmd == "status":
        return cmd_status()
    if cmd == "stop":
        return cmd_stop()
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main())
