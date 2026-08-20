# Install verification checklist

An executable checklist for the unsigned Noren release-candidate binary. Run
it on a machine that did **not** build the artifact. Every step states what
success looks like; if any step fails, stop and record the failure — do not
continue to a later step.

## Before anything: what this artifact is and is not

**The binary is unsigned and not notarized.** See
[the release process](README.md#what-is-not-covered--read-this-first).
On first launch macOS Gatekeeper **will** show a security warning for any
copy that did not originate on the building machine, and depending on the
macOS version and local policy it **may refuse to open the binary at all**.
Hitting that warning is expected. It does not mean the download is corrupt
(step 2 proves whether it is) and it does not mean the app is broken.

## Fresh-machine assumptions

Record each before starting; a wrong assumption invalidates the check:

- [ ] macOS with a logged-in GUI session (`sw_vers` prints 14 or later on
      Apple Silicon). The binary is built for `aarch64-apple-darwin`;
      `uname -m` prints `arm64`.
- [ ] `/bin/zsh` exists (it ships with macOS; `ls -l /bin/zsh`).
- [ ] No other dependencies: running needs neither Homebrew, nor Rust, nor
      any library install. (Building from source needs the pinned Rust
      toolchain; running the artifact does not.)
- [ ] You have the artifact (`noren-<version>-aarch64-apple-darwin`) and
      `SHA256SUMS` in one directory. If `BUILD-PROVENANCE.txt` and
      `release-notes.md` were shipped too, keep them beside them; the
      manifest covers them.
- [ ] No prior Noren install: `ls "$HOME/Library/Application Support/Noren"`
      fails with "No such file or directory". If it exists, record it and
      remove it only if you are prepared to discard that state.

## Step 1 — verify the bytes

From the directory holding the artifact:

```
shasum -a 256 --check SHA256SUMS
```

- [ ] Every line reports `OK`. A mismatch means the bytes are not the bytes
      the builder staged: **do not launch**; report the mismatch.

(On a Linux host with GNU coreutils the equivalent is `sha256sum -c
SHA256SUMS`; only the checksum step is meaningful there — the binary itself
does not run on Linux.)

## Step 2 — first launch

```
chmod +x noren-<version>-aarch64-apple-darwin
./noren-<version>-aarch64-apple-darwin
```

- [ ] If you built this binary on this machine, it launches with no security
      dialog (nothing attached a quarantine attribute).
- [ ] If the file arrived from anywhere else, expect a Gatekeeper dialog:
      roughly *"…cannot be opened because the developer cannot be
      verified"*. This is the documented behaviour of an unsigned binary —
      not corruption, and not an app defect. Whether macOS offers
      "Open Anyway" (System Settings → Privacy & Security) depends on its
      version and local policy; if it refuses entirely, that refusal is the
      expected outcome of shipping unsigned and is itself a finding to
      record, not a failure of this checklist.
- [ ] Do not bypass Gatekeeper reflexively. Deciding how unsigned previews
      may be opened on shared machines is an owner policy decision.

## Step 3 — what a correct launch looks like

The known visual defects below are pinned in
[known limitations](../known-limitations.md); they are expected, not new
findings.

- [ ] A roughly 900x600 dark window opens; the left columns are the
      workspace sidebar; a `zsh` prompt appears to its right.
- [ ] **No visible cursor** is normal (tracked, never drawn).
- [ ] **`a` and `A` look identical** and **non-ASCII shows as `?`** — both
      are the documented bitmap-font defects.
- [ ] Typing `stty size` and Enter prints `30 90` at the default window
      size; `printf 'ok\n'` echoes `ok`.
- [ ] `pgrep -fl noren-` shows exactly one Noren process, and
      `pgrep -P <that-pid> -l` shows a direct `zsh` child.
- [ ] Typing `exit` and Enter: the shell exits and a status line reading
      `Noren shell exited` appears; the window stays open on the final
      frame (by design — the PoC preserves it until the window is closed).
- [ ] Closing the window ends the process: a later
      `pgrep -fl noren-` finds nothing and no orphaned `zsh` from your
      session remains.

## Step 4 — state landed where expected

- [ ] `ls "$HOME/Library/Application Support/Noren"` now shows
      `sessions.toml`. Nothing else is written outside this directory: the
      app reads `~/.ssh/config` read-only when present, uses the system
      clipboard service, and installs nothing system-wide.

## Step 5 — uninstall cleanly

- [ ] Quit Noren (close the window) and confirm `pgrep -fl noren-` is empty.
- [ ] Delete the binary.
- [ ] `rm -rf "$HOME/Library/Application Support/Noren"` (this discards
      `sessions.toml` and any `config.toml` you created; see
      [configuration](../configuration.md)).
- [ ] Verify: the directory is gone, `pgrep -fl noren-` is empty, and
      `ls /Library /Library/LaunchAgents "$HOME/Library/LaunchAgents"`
      shows no Noren entries (none are ever created; checking proves it).

## What to report

The outcome of every checkbox, the exact `sw_vers` and `uname -m` output,
the Gatekeeper dialog text you saw verbatim (or "none, built locally"), and
the `shasum --check` output. Unverified steps must be marked unverified, not
omitted.
