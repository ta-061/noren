#!/usr/bin/env bash
# Start, stop, or inspect the orchestrator as a detached background process.
#
# Detached means it survives the shell and the Claude Code session that started
# it: no controlling terminal, its own process group, output to .fleet/logs.
# No sudo, no system-wide daemon, no secrets outside the repo.
set -uo pipefail
REPO="/Users/yoshinagatatsuya/Documents/apps/noren"
PY="$REPO/scripts/fleet/orchestrator.py"
OUT="$REPO/.fleet/logs/orchestrator.out"
PIDF="$REPO/.fleet/orchestrator.pid"

case "${1:-status}" in
  start)
    if [ -f "$PIDF" ] && kill -0 "$(cat "$PIDF")" 2>/dev/null; then
      echo "already running as pid $(cat "$PIDF")"; exit 1
    fi
    mkdir -p "$REPO/.fleet/logs"
    # Double-fork via python so the child is reparented away from this shell.
    python3 - "$PY" "$OUT" <<'PYEOF'
import os, sys
py, out = sys.argv[1], sys.argv[2]
if os.fork() > 0: os._exit(0)          # parent returns to the shell
os.setsid()                            # new session, no controlling tty
if os.fork() > 0: os._exit(0)          # ensure we cannot reacquire one
fd = os.open(out, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o644)
os.dup2(fd, 1); os.dup2(fd, 2)
os.dup2(os.open(os.devnull, os.O_RDONLY), 0)
os.execv(sys.executable, [sys.executable, py, "start"])
PYEOF
    sleep 4
    if [ -f "$PIDF" ]; then echo "started pid $(cat "$PIDF")"; else
      echo "failed to start; see $OUT"; tail -5 "$OUT" 2>/dev/null; exit 1; fi
    ;;
  stop)    python3 "$PY" stop ;;
  status)  python3 "$PY" status ;;
  logs)    tail -n "${2:-40}" "$OUT" ;;
  *) echo "usage: orchestratord.sh {start|stop|status|logs [n]}"; exit 2 ;;
esac
