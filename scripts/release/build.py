#!/usr/bin/env python3
"""Reproducible release-candidate build for the macOS Noren binary.

This script builds the one distributable artifact (the `noren-app` release
binary), stages it under `dist/` with a versioned name, writes a
`SHA256SUMS` manifest covering every staged artifact, records the toolchain
provenance NFR-008 requires, and generates the release-notes template from
the merged git history. It stops there.

What it deliberately does NOT do (owner decisions, see docs/release/README.md):
no code signing, no notarization, no Gatekeeper handling, no git tag, no
GitHub release, no upload of any artifact to anywhere.

Usage:
    python3 scripts/release/build.py [--dry-run] [--skip-notes]
                                     [--smoke] [--smoke-gui]
                                     [--allow-dirty] [--since <ref>]

`--dry-run` prints the exact commands it would run and the artifact paths it
would stage, then exits. `--smoke` runs the no-window configuration-failure
launch check (deterministic exit code 1). `--smoke-gui` additionally launches
the real windowed binary against a scratch HOME and verifies it stays alive
and owns a direct `/bin/zsh` child; it requires a logged-in macOS GUI session
and is therefore never run by CI.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import time
import tempfile
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DIST = ROOT / "dist"
PACKAGE = "noren-app"
BINARY = "noren-app"
MANIFEST_NAME = "SHA256SUMS"
PROVENANCE_NAME = "BUILD-PROVENANCE.txt"
NOTES_NAME = "release-notes.md"
SMOKE_GUI_SETTLE_SECONDS = 6


def run(command: list[str], *, env: dict[str, str] | None = None,
        capture: bool = True, check: bool = True) -> subprocess.CompletedProcess:
    """Run a command, echoing the exact invocation to stdout first."""
    printable = " ".join(command)
    print(f"+ {printable}")
    return subprocess.run(
        command, cwd=ROOT, env=env, check=check, text=True,
        capture_output=capture,
    )


def cargo_build_command() -> list[str]:
    """The exact release build command. `--locked` is load-bearing: the
    release must be built against the committed Cargo.lock, never a
    re-resolved one."""
    return ["cargo", "build", "--release", "--locked", "-p", PACKAGE]


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def manifest_entry(digest: str, name: str) -> str:
    """One `sha256sum`-compatible line: digest, two spaces, file name."""
    return f"{digest}  {name}"


def artifact_name(version: str, triple: str) -> str:
    return f"noren-{version}-{triple}"


def host_triple(rustc_vv: str) -> str:
    for line in rustc_vv.splitlines():
        if line.startswith("host: "):
            return line[len("host: "):].strip()
    raise ValueError("rustc -vV output contains no host triple")


def noren_app_version(metadata_json: str) -> str:
    metadata = json.loads(metadata_json)
    for package in metadata["packages"]:
        if package["name"] == PACKAGE:
            return package["version"]
    raise ValueError(f"cargo metadata reports no package named {PACKAGE}")


def assert_macos(platform: str) -> None:
    if platform != "darwin":
        print(
            f"error: this is the macOS artifact build; refusing on {platform}",
            file=sys.stderr,
        )
        raise SystemExit(1)


def tree_state(porcelain: str) -> str:
    return "dirty" if porcelain.strip() else "clean"


def stage_binary(built: Path, dist: Path, name: str) -> Path:
    dist.mkdir(parents=True, exist_ok=True)
    staged = dist / name
    shutil.copy2(built, staged)
    staged.chmod(0o755)
    return staged


def write_manifest(dist: Path, artifacts: list[Path]) -> Path:
    entries = [
        manifest_entry(sha256_file(path), path.name) for path in sorted_artifacts(artifacts)
    ]
    manifest = dist / MANIFEST_NAME
    manifest.write_text("\n".join(entries) + "\n", encoding="utf-8")
    return manifest


def sorted_artifacts(artifacts: list[Path]) -> list[Path]:
    """Checksummed artifacts in manifest order: the binary first, then every
    other staged file except the manifest itself (a file cannot contain its
    own checksum)."""
    others = sorted(
        (path for path in artifacts if path.name != MANIFEST_NAME),
        key=lambda path: (path.name == PROVENANCE_NAME, path.name),
    )
    return others


def smoke_config_error(binary: Path) -> bool:
    """Launch check that needs no window: `NOREN_CONFIG` naming a missing
    file must make the binary exit 1 with the documented stderr message
    (`AppConfig::load` in crates/noren-app/src/config.rs)."""
    missing = ROOT / "target" / "release" / ".noren-smoke-missing.toml"
    completed = subprocess.run(
        [str(binary)],
        env=dict(os.environ, NOREN_CONFIG=str(missing)),
        capture_output=True, text=True, timeout=30,
    )
    ok = completed.returncode == 1 and "Noren configuration is unusable" in completed.stderr
    print(
        f"smoke(config-error): exit={completed.returncode} "
        f"message={'found' if 'Noren configuration is unusable' in completed.stderr else 'MISSING'}"
    )
    return ok


def smoke_gui(binary: Path, settle_seconds: int = SMOKE_GUI_SETTLE_SECONDS) -> bool:
    """Launch the real windowed binary against a scratch HOME and verify it
    stays alive and owns a direct `/bin/zsh` child, then terminate it."""
    scratch = Path(tempfile.mkdtemp(prefix="noren-release-smoke-"))
    env = dict(os.environ, HOME=str(scratch))
    env.pop("NOREN_CONFIG", None)
    process = subprocess.Popen(
        [str(binary)], env=env,
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    try:
        deadline = time.monotonic() + settle_seconds
        while time.monotonic() < deadline:
            if process.poll() is not None:
                print(f"smoke(gui): binary exited early with {process.returncode}")
                return False
            time.sleep(0.5)
        children = subprocess.run(
            ["pgrep", "-P", str(process.pid), "-l"],
            capture_output=True, text=True,
        ).stdout.strip()
        print(f"smoke(gui): pid={process.pid} alive after {settle_seconds}s; children:\n{children}")
        alive = process.poll() is None
        has_zsh = "zsh" in children
        print(f"smoke(gui): alive={alive} direct_zsh_child={has_zsh}")
        return alive and has_zsh
    finally:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        shutil.rmtree(scratch, ignore_errors=True)


def collect_provenance(since: str | None, allow_dirty: bool) -> str:
    head = run(["git", "rev-parse", "HEAD"]).stdout.strip()
    porcelain = run(["git", "status", "--porcelain"]).stdout
    state = tree_state(porcelain)
    if state == "dirty" and not allow_dirty:
        print(
            "error: working tree is dirty; a release candidate must build from a "
            "committed tree (use --allow-dirty to record and proceed anyway)",
            file=sys.stderr,
        )
        raise SystemExit(1)
    rustc_vv = run(["rustc", "-vV"]).stdout
    cargo_vv = run(["cargo", "--version", "--verbose"]).stdout
    targets = run(["rustup", "target", "list", "--installed"]).stdout
    lock_digest = sha256_file(ROOT / "Cargo.lock")
    lines = [
        "Noren release-candidate build provenance (NFR-008)",
        f"recorded_utc: {datetime.now(timezone.utc).isoformat(timespec='seconds')}",
        f"git_head: {head}",
        f"git_tree: {state}"
        + (" (built with --allow-dirty; NOT a release candidate)" if state == "dirty" else ""),
        f"build_command: {' '.join(cargo_build_command())}",
        f"rustc: {rustc_vv.strip().splitlines()[0]}",
        f"host_triple: {host_triple(rustc_vv)}",
        f"cargo: {cargo_vv.strip()}",
        "installed_targets:",
        *("  " + line for line in targets.strip().splitlines()),
        f"cargo_lock_sha256: {lock_digest}",
        f"uname: {run(['uname', '-a']).stdout.strip()}",
        f"sw_vers: {run(['sw_vers', '--productVersion']).stdout.strip()}",
        f"notes_baseline_arg: {since or 'default (see scripts/release/notes.py)'}",
        "",
        "Reproducibility statement: this file records the toolchain, host, and",
        "dependency-lock provenance of the build. It does NOT claim bit-for-bit",
        "reproducibility of the binary. The artifact is unsigned; see",
        "docs/release/README.md before distributing anything.",
    ]
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--dry-run", action="store_true",
                        help="print the exact commands and artifact paths, build nothing")
    parser.add_argument("--skip-notes", action="store_true",
                        help="do not generate dist/release-notes.md")
    parser.add_argument("--smoke", action="store_true",
                        help="run the no-window launch check after staging")
    parser.add_argument("--smoke-gui", action="store_true",
                        help="launch the real windowed binary against a scratch HOME")
    parser.add_argument("--allow-dirty", action="store_true",
                        help="proceed on a dirty tree, recording it in provenance")
    parser.add_argument("--since", default=None, metavar="REF",
                        help="baseline ref for release notes (default: resolved by notes.py)")
    args = parser.parse_args(argv)

    assert_macos(sys.platform)

    rustc_vv = run(["rustc", "-vV"]).stdout
    triple = host_triple(rustc_vv)
    metadata = run(["cargo", "metadata", "--no-deps", "--format-version", "1"]).stdout
    version = noren_app_version(metadata)
    name = artifact_name(version, triple)
    built = ROOT / "target" / "release" / BINARY
    staged = DIST / name
    provenance = DIST / PROVENANCE_NAME
    manifest = DIST / MANIFEST_NAME
    notes = DIST / NOTES_NAME

    print(f"artifact: {staged}")
    if args.dry_run:
        print(f"manifest: {manifest}")
        print(f"provenance: {provenance}")
        print(f"notes: {notes}")
        print("dry run; no build performed")
        return 0

    run(cargo_build_command())
    if not built.is_file():
        print(f"error: build did not produce {built}", file=sys.stderr)
        return 1

    DIST.mkdir(parents=True, exist_ok=True)
    binary_path = stage_binary(built, DIST, name)
    provenance.write_text(collect_provenance(args.since, args.allow_dirty),
                          encoding="utf-8")
    artifacts: list[Path] = [binary_path, provenance]
    if not args.skip_notes:
        notes_cmd = [sys.executable, str(ROOT / "scripts" / "release" / "notes.py")]
        if args.since:
            notes_cmd += ["--since", args.since]
        notes_cmd += ["--output", str(notes)]
        run(notes_cmd, capture=False)
        if not notes.is_file():
            print(f"error: notes generation did not produce {notes}", file=sys.stderr)
            return 1
        artifacts.append(notes)

    manifest_path = write_manifest(DIST, artifacts)
    size_mb = binary_path.stat().st_size / (1024 * 1024)
    print(f"staged: {binary_path} ({size_mb:.1f} MiB)")
    for line in manifest_path.read_text(encoding="utf-8").splitlines():
        print(f"checksum: {line}")

    ok = True
    if args.smoke:
        ok = smoke_config_error(binary_path) and ok
    if args.smoke_gui:
        ok = smoke_gui(binary_path) and ok
    if not ok:
        print("error: launch smoke check failed", file=sys.stderr)
        return 1
    print("build complete; artifact is UNSIGNED — see docs/release/README.md")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
