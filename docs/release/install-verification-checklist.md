# Install verification checklist — Noren `0.1.0-preview` (macOS)

This page is written to be executed, not skimmed. It assumes nothing about
Noren and verifies every step that can be verified. It was executed end to end
on the machine that built the release candidate (macOS 26.4.1, arm64); steps
marked **verified** were actually run there, and anything not verified on a
second machine says so.

## READ THIS FIRST — the binary is unsigned and macOS will fight you

**This preview is not code-signed and not notarized.** It carries only the
automatic ad-hoc signature the macOS linker applies to every arm64 binary.
Verified on the release candidate:

```text
$ codesign -dvvv noren-app
Executable=.../noren-app
Format=Mach-O thin (arm64)
CodeDirectory v=20400 ... flags=0x20002(adhoc,linker-signed)
Signature=adhoc
TeamIdentifier=not set
```

What that means for you, plainly:

- **Gatekeeper will warn on first launch, and on current macOS it refuses
  outright until you override it.** A binary downloaded from the internet gets
  a quarantine attribute; macOS blocks unsigned software with a dialog like
  "cannot be opened because it is from an unidentified developer" (on some
  versions: "...cannot be checked for malicious software" or a spurious
  "...is damaged"). **This is expected. It does not mean the download is
  corrupt** — the checksum step below is how you check that — and **it does
  not mean the app is broken.**
- An unsigned binary is also an unverified binary: no one vouches for its
  publisher. That is exactly why the checksum verification in this checklist
  comes **before** any Gatekeeper override. Do the checksum first, always.
- Signing with a real Developer ID and notarizing are deliberately **not**
  done: they are reserved owner decisions under
  [D-M8-001](../coordination/decisions/D-M8-001-preview-scope.md), and this
  preview stops at a release candidate.

**How to override Gatekeeper on macOS 15 (Sequoia) and later** (verified
paths as of macOS 26; exact wording varies by version):

1. Try to launch once; macOS blocks it and shows the dialog.
2. Open **System Settings → Privacy & Security**, scroll to the security
   section, and click **Open Anyway** for the blocked app, then confirm.
   (The old right-click → Open shortcut no longer works on Sequoia+.)
3. Alternatively, remove the quarantine attribute before the first launch —
   this is the common route for a bare command-line binary like this one:

   ```text
   xattr -d com.apple.quarantine noren-app
   ```

   Understand what this does: it tells macOS to skip the Gatekeeper check for
   this file. Only do it after the checksum below passes, and only on files
   you downloaded from the source you chose to trust.

**Second thing to read before launching:** this is an explicitly dated
developer preview whose limits are enumerated in
[known limitations](../known-limitations.md). The short version of what you
will and will not see: a window titled **"Noren" followed by the crate
version** (`window_title()` in `crates/noren-app/src/main.rs` builds it from
`PRODUCT_NAME` and `CARGO_PKG_VERSION`, so it always states the version the
binary was built as) with a workspace
sidebar and a working local `zsh` — but **no visible cursor**, a 5x7 bitmap
font with bounded coverage (CJK and emoji draw as replacement boxes),
discarded IME input, no accessibility surface, and no native tabs or panes
(panes are Zellij's job by design). Programs that emit colour do render
colour; the default dark theme clears the WCAG AA floor on every
theme-owned slot (the issue-168 fix lifted the five entries that used to
fall below), as do `light` and `high-contrast` — the contract does not
cover the 256-colour tail, truecolor, or program-paired colour
combinations. If any of that
would read to you as "broken", read the full page first — it is the honest
description of what this is.

## Fresh-machine assumptions

- An **Apple Silicon Mac** (`uname -m` prints `arm64`). The binary is
  `aarch64-apple-darwin` only; it will not run on Intel Macs or Linux.
- **macOS**: built and executed on macOS 26.4.1 (arm64). Other macOS
  versions are **unverified** — nothing here claims them.
- About 10 MB of free disk space. No installer, no `.app` bundle, no admin
  rights, no Rust toolchain, and no network connection are needed.
- `/bin/zsh` exists (it is the fixed launch policy; stock macOS has it).

## Step 1 — download and verify the checksum

Download **both** the tarball and `SHA256SUMS` from the release you chose,
into the same directory, then from that directory:

```text
shasum -a 256 -c SHA256SUMS
```

Expected (the digest and file name as published; shown for the candidate
built at commit `23ce308`):

```text
923f14d10215b5792c204d3dfebc8f699c0297b2a765e5b0f6f5b2c676e398b8  noren-0.1.0-preview-23ce308-aarch64-apple-darwin.tar.gz
noren-0.1.0-preview-23ce308-aarch64-apple-darwin.tar.gz: OK
```

The `OK` line only appears when the file on disk matches the published
digest. **If it does not say OK: stop. Do not extract, do not open, and
report the mismatch** — a wrong digest means a corrupt download or a tampered
file. The checksums are generated by `scripts/release/build.sh` at build
time, never by hand.

Optional extras, before extracting: the tarball contains exactly two
files — the `noren-app` binary and `RELEASE-NOTES.md`, the release notes
generated by the build at exactly the commit in the file name (they are
not a hand-written or separately committed document; the build refuses to
pack notes whose pinned head is anything other than the commit it is
packaging). After extraction (Step 2) you can check both against the
`BUILD-PROVENANCE.txt` of the build that produced them:

```text
shasum -a 256 noren-<version>-<sha>-aarch64-apple-darwin/noren-app      # binary_sha256=
shasum -a 256 noren-<version>-<sha>-aarch64-apple-darwin/RELEASE-NOTES.md  # notes_sha256=
```

and the `- Head:` line inside `RELEASE-NOTES.md` names the same `<sha>`
that appears in the tarball's file name — the notes and the artifact
describe the same history.

## Step 2 — extract

```text
tar -xzf noren-0.1.0-preview-<sha>-aarch64-apple-darwin.tar.gz
```

This creates one directory containing two files:

```text
noren-0.1.0-preview-<sha>-aarch64-apple-darwin/
├── noren-app          (about 6.9 MB, executable)
└── RELEASE-NOTES.md   (generated release notes for exactly this commit)
```

## Step 3 — handle Gatekeeper, then launch

Do the checksum first (Step 1). Then apply one of the Gatekeeper overrides
from the top of this page. Then launch **from Terminal** — this is a bare
binary, not a `.app` bundle, so double-clicking it in Finder is not the
supported path:

```text
cd noren-0.1.0-preview-<sha>-aarch64-apple-darwin
./noren-app
```

### First-launch expectations

What you should observe, and when to worry:

- **Within a couple of seconds** a window titled **"Noren" followed by the
  crate version** opens. Its
  left columns are the workspace sidebar (a session row for the shell it
  started, plus any discovered git worktrees or configured SSH hosts,
  projects, and agents); the rest is the terminal.
- **The terminal is a live local `zsh`.** Typing works; `ls`, `vim`, and
  anything interactive respond. If you run them you will notice the limits
  described above (no cursor block, one shade for glyphs outside the font's
  coverage, and so on) — those are documented preview limits, not launch
  failures. **Verified** on the build machine: process starts, spawns a
  direct `zsh` child, stays up, and exits cleanly with the child reaped.
- **On the very first launch after a download**, the expected Gatekeeper
  dialog (see the top of this page) is the only alarm. No other error dialog
  is expected. A crash on launch with Gatekeeper already satisfied is a real
  defect: report it with your macOS version and the checksum output.

### Quitting

Close the window (red traffic light). The app exits and reaps the shell
child (**verified** on the build machine, along with clean `SIGTERM`
handling). There is no menu-bar quit item because this is not a `.app`.

## Step 4 — uninstall cleanly

The binary is self-contained and the app writes state in exactly one place:
`$HOME/Library/Application Support/Noren/` (its `config.toml` and
`sessions.toml` — the latter's creation was **verified** on the build
machine). To remove everything:

```text
# quit the app first
rm -rf noren-0.1.0-preview-<sha>-aarch64-apple-darwin
rm -rf "$HOME/Library/Application Support/Noren"
rm -f noren-0.1.0-preview-<sha>-aarch64-apple-darwin.tar.gz SHA256SUMS
```

That is the complete uninstall: no `.app` bundle was registered with
LaunchServices, no receipts, no daemons, no preferences outside that
directory, nothing in `/Library` or `/usr/local`.

## If something is wrong

Open an issue with: the `shasum -c` output, your macOS version
(`sw_vers`), `uname -m`, and what you saw versus what this page said to
expect. Security-relevant reports follow [SECURITY](../../SECURITY.md).
