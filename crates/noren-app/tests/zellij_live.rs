//! Live-Zellij pass-through evidence: the pass-through policy, the terminal
//! parser, and the key encoder are exercised against an INSTALLED Zellij, not
//! only the pinned corpus (ROADMAP's Zellij pass-through row previously
//! recorded "no test drives a live Zellij").
//!
//! What is live here:
//!
//! - The PTY is created by the same `portable-pty` backend `noren-pty` uses,
//!   with `TERM`/`TERM_PROGRAM` read from `noren_pty`'s real
//!   `ZshLaunchPolicy` metadata, so the harness cannot drift from the
//!   production environment policy. The product crate's launch policy is
//!   deliberately fixed to `/bin/zsh`, so the harness spawns Zellij itself
//!   (a dev-dependency; no product surface changes).
//! - Every output byte is parsed by the real `noren_terminal::TerminalState`.
//! - Every input key is routed through the real `noren_app::passthrough`
//!   gate and `KeyEncoder`, mirroring `main.rs`'s `handle_passthrough_key`
//!   ordering: encode first, gate-decide, forward on `GateKind::Forwarded`.
//!
//! Skip policy: when `zellij` is not on `PATH`, each live test returns early
//! after printing `SKIP: [...]` to the REAL stderr (bypassing the harness's
//! output capture) — CI without Zellij stays green while the output states
//! explicitly that live evidence was NOT gathered. A skip is never reported
//! as gathered evidence.
//!
//! Empirical record against the INSTALLED Zellij (drift-printed by the
//! mouse-mode test): Zellij 0.44.3 requests mouse tracking with
//! single-parameter DECSETs (`CSI ? 1002 h`, `CSI ? 1006 h`, ...). Across the
//! FULL lifecycle probed while building this harness — attach, tab/pane
//! interaction, typed input, and quit/restore — zero multi-parameter private
//! CSI sequences appeared, and Zellij does not forward a pane program's
//! multi-parameter DECSET to the host terminal (it re-renders panes itself
//! and owns the host mode set). The multi-parameter form
//! `CSI ? 1002;1006 h` is therefore NOT live-Zellij wire shape for this
//! version.
//!
//! Popup control (issue #147): a fresh session's pane area is NOT popup-free
//! by default. With an untouched default configuration, Zellij's session
//! startup adds ONE floating `about`-plugin popup over the pane area:
//! first-run setup wizard (absent here — the harness seeds `config.kdl`),
//! else release notes (drawn every run whose cache has not recorded them for
//! the installed version — the harness isolates `HOME`, so its cache is
//! always fresh and the release-notes popup, which carries the sponsor
//! notice, appears on EVERY harness run), else a random startup tip. The
//! popups are suppressible by top-level configuration — `show_release_notes
//! false` and `show_startup_tips false` (options parsed from `config.kdl` by
//! the installed 0.44.3, exactly the pinned `ZELLIJ_FIXTURE_TAG`) — so the
//! harness pins both in the config it seeds instead of inheriting whatever
//! popup state the developer's machine would draw. The pin is enforced by
//! `live_zellij_session_starts_without_popups_over_the_pane_area`, which
//! fails if the suppression is removed and a popup is drawn.
//!
//! Hermetic teardown (issue #147 follow-up): `zellij delete-all-sessions
//! --yes` returns success WITHOUT reaping the server process it talks to —
//! every harness run used to leak one idle server (~100 MB RSS each)
//! forever, and accumulated leaks starved later runs' pane spawns into
//! failing the tab/pane test (the machine, not the code, decided the
//! verdict; diagnosed in detail on [`LiveZellij::reap_session_server`]).
//! Teardown is therefore bounded ([`TEARDOWN_BUDGET`]) and reaps this run's
//! server by its unique socket-directory command line. Likewise, every
//! spawned Zellij gets the inherited `ZELLIJ_*` environment scrubbed
//! ([`SCRUBBED_ZELLIJ_ENV`]) before the isolation variables are set, so
//! developer exports — notably `ZELLIJ_CONFIG_FILE`, which clap prefers
//! over `ZELLIJ_CONFIG_DIR` — cannot escape the harness's control.
//!
//! The multi-parameter DECSET path is a proven regression site (fixed in
//! PR #113), so the mouse-mode test pins it EXPLICITLY beside the live
//! evidence: after asserting on the real attach stream, it drives
//! `CSI ? 1002;1006 h` through the SAME `TerminalState` instance that just
//! parsed that stream. That pin is a regression guard co-located with the
//! live evidence, not a claim about what Zellij sent; a parser that bails on
//! multi-parameter DECSETs fails this test file. The harness also prints the
//! live multi-parameter count, so a future Zellij that changes wire shape
//! surfaces as visible drift.

use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use noren_app::config::KeymapConfig;
use noren_app::passthrough::{
    CLAIM_ID_PALETTE, Chord, ChordSeq, GateKind, KeyCode as GateKeyCode,
    Modifiers as GateModifiers, PassthroughAction, PassthroughClaim, PassthroughGate,
    PassthroughPolicy, ZELLIJ_FIXTURE_TAG, default_exit_claim,
};
use noren_app::{
    CursorKeyMode, InputMode, Key, KeyEncoder, KeyInput, KeyPhase, KeypadMode, Modifiers,
};
use noren_pty::ZshLaunchPolicy;
use noren_terminal::TerminalState;
use portable_pty::{Child, CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};

const ROWS: u16 = 40;
const COLS: u16 = 120;
const ATTACH_BUDGET: Duration = Duration::from_secs(20);
const INTERACTION_BUDGET: Duration = Duration::from_secs(8);
/// How long the popup-absence pin keeps draining after the tab bar renders,
/// giving an async-loading popup pane time to draw before absence is
/// asserted. This is a settle window for detecting a popup that SHOULD be
/// there under a mutation — never a fixed sleep that a dismissal waits on.
const POPUP_SETTLE_BUDGET: Duration = Duration::from_secs(2);
/// How long a teardown `zellij` CLI subprocess may run before the harness
/// kills it. `zellij delete-all-sessions --yes` has no timeout of its own;
/// pointed at a starved or wedged server its connect/retry loop can block
/// for tens of seconds, which is exactly the machine state teardown runs in
/// after a failing test (issue #147: failing runs previously took ~90 s just
/// to unwind). The bound keeps worst-case teardown linear and small.
const TEARDOWN_BUDGET: Duration = Duration::from_secs(5);
/// Attempts (each a pgrep + SIGKILL round) to reap this run's leaked Zellij
/// server before giving up and printing a visible notice. The server is
/// normally gone after the first SIGKILL; the retries only cover the window
/// where it is mid-exit when pgrep still lists it.
const SERVER_REAP_ATTEMPTS: usize = 5;
const CONFIG_FILE_NAME: &str = "config.kdl";

/// Every `ZELLIJ_*` environment variable the installed Zellij binary is
/// known to read (verified against the 0.44.3 binary's own strings). The
/// harness scrubs all of them from every Zellij subprocess it spawns and
/// then sets the three isolation variables explicitly, so a developer's
/// exported Zellij state cannot escape the harness's control:
///
/// - `ZELLIJ_CONFIG_FILE` is clap's `-c/--config` environment form and takes
///   precedence over `--config-dir`/`ZELLIJ_CONFIG_DIR` (independent review
///   of the popup fix, issue #147): with it set, the attached session would
///   read the developer's config file instead of the harness-seeded one and
///   the popup-suppression pin would test the wrong configuration.
/// - `ZELLIJ_SESSION_NAME` is the clap default for `--session`; scrubbed so
///   the session name is exactly what the harness passes, never inherited.
/// - `ZELLIJ_AUTO_ATTACH`/`ZELLIJ_AUTO_EXIT` change client behaviour inside
///   sessions; `ZELLIJ_PANE_ID` marks a process as a pane child. None may
///   leak into a harness-spawned client.
/// - `ZELLIJ_CONFIG_DIR`/`ZELLIJ_DATA_DIR`/`ZELLIJ_SOCKET_DIR` are the
///   isolation variables themselves: scrub-then-set makes the explicit value
///   authoritative even if the environment already exports one.
const SCRUBBED_ZELLIJ_ENV: [&str; 8] = [
    "ZELLIJ_CONFIG_FILE",
    "ZELLIJ_CONFIG_DIR",
    "ZELLIJ_DATA_DIR",
    "ZELLIJ_SOCKET_DIR",
    "ZELLIJ_SESSION_NAME",
    "ZELLIJ_AUTO_ATTACH",
    "ZELLIJ_AUTO_EXIT",
    "ZELLIJ_PANE_ID",
];

/// Scrub the inherited Zellij environment of a std `Command` (see
/// [`SCRUBBED_ZELLIJ_ENV`]); the caller sets the isolation variables
/// afterwards.
fn scrub_zellij_env(command: &mut Command) {
    for key in SCRUBBED_ZELLIJ_ENV {
        command.env_remove(key);
    }
}

/// Scrub the inherited Zellij environment of a portable-pty
/// `CommandBuilder` (see [`SCRUBBED_ZELLIJ_ENV`]); the caller sets the
/// isolation variables afterwards.
fn scrub_zellij_env_pty(command: &mut CommandBuilder) {
    for key in SCRUBBED_ZELLIJ_ENV {
        command.env_remove(key);
    }
}

/// Session-start popup suppression appended to the seeded configuration
/// (issue #147). Both are top-level `config.kdl` options of the installed
/// Zellij 0.44.3 (the pinned `ZELLIJ_FIXTURE_TAG`): `show_release_notes`
/// skips the once-per-version release-notes popup — which carries the
/// sponsor notice — before its cache check, and `show_startup_tips` skips
/// the random startup-tip popup. With both false, session startup adds no
/// `about`-plugin floating pane at all, so the pane area shows only pane
/// output on every machine regardless of cache state. A Zellij that renames
/// these options rejects the seeded config and the session never renders
/// its tab bar, which fails every live spawn loudly instead of silently
/// regressing to popup-covered screens.
const POPUP_SUPPRESSION_CONFIG: &str = "show_release_notes false\nshow_startup_tips false\n";

/// Marker strings that only a session-start popup draws over the pane area
/// (titles and the two help/sponsor lines the 0.44.3 popups render). None of
/// these can appear in a suppressed session's screen.
const POPUP_MARKERS: [&str; 4] = [
    "Release Notes",
    "Zellij Tip #",
    "Please support the Zellij developer",
    "Help: <↓↑> - Navigate, <ESC> - Dismiss",
];

static DIR_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

/// Parsed `zellij --version` output of an installed Zellij, or `None` when
/// Zellij is absent (or unusable), which every live test treats as a skip.
fn installed_zellij_version() -> Option<String> {
    let output = Command::new("zellij").arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_version_line(&text).map(str::to_owned)
}

/// The version token of a `zellij <version>` line; `None` when no
/// digit-leading token is present.
fn parse_version_line(line: &str) -> Option<&str> {
    line.split_whitespace()
        .last()
        .filter(|token| token.starts_with(|c: char| c.is_ascii_digit()))
}

/// Print the skip notice shared by every live test when Zellij is absent.
///
/// The notice is written straight to the process's stderr file descriptor so
/// it survives the test harness's output capture: an early-returning test
/// otherwise reads as a silent pass under default `cargo test` output, and a
/// skip must never be mistaken for gathered evidence.
fn report_skip(test: &str) {
    let notice = format!(
        "SKIP [{test}]: zellij is not installed (or `zellij --version` failed); \
         live pass-through evidence was NOT gathered. This is a skip, not a pass."
    );
    write_raw_stderr(&notice);
}

/// Write a notice to the real stderr file descriptor, bypassing the test
/// harness's output capture (see [`report_skip`]).
fn write_raw_stderr(notice: &str) {
    match fs::OpenOptions::new().write(true).open("/dev/stderr") {
        Ok(mut file) => {
            let _ = file.write_all(notice.as_bytes());
            let _ = file.write_all(b"\n");
        }
        Err(_) => eprintln!("{notice}"),
    }
}

/// Isolated config/data/socket/home directories for one live Zellij run.
///
/// The base lives directly under `/tmp` because Zellij derives its session
/// socket path from `ZELLIJ_SOCKET_DIR`, and a long `std::env::temp_dir()`
/// prefix plus a session name exceeds the OS unix-domain socket path limit
/// (Zellij then rejects the session name with a misleading length error).
struct ZellijDirs {
    base: PathBuf,
    home: PathBuf,
    config: PathBuf,
    data: PathBuf,
    socket: PathBuf,
}

impl ZellijDirs {
    fn new() -> Self {
        let sequence = DIR_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let base = PathBuf::from("/tmp").join(format!("zn-live-{}-{sequence}", std::process::id()));
        let dirs = Self {
            home: base.join("h"),
            config: base.join("c"),
            data: base.join("d"),
            socket: base.join("s"),
            base,
        };
        for dir in [&dirs.home, &dirs.config, &dirs.data, &dirs.socket] {
            fs::create_dir_all(dir).expect("create isolated zellij directory");
        }
        dirs
    }

    fn env_pairs(&self) -> [(OsString, OsString); 3] {
        [
            ("ZELLIJ_CONFIG_DIR".into(), self.config.clone().into()),
            ("ZELLIJ_DATA_DIR".into(), self.data.clone().into()),
            ("ZELLIJ_SOCKET_DIR".into(), self.socket.clone().into()),
        ]
    }
}

impl Drop for ZellijDirs {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

/// One live Zellij client attached inside a Noren-shaped PTY.
struct LiveZellij {
    version: String,
    terminal: TerminalState,
    raw: Vec<u8>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    _master: Box<dyn MasterPty + Send>,
    events: Receiver<Vec<u8>>,
    dirs: ZellijDirs,
}

impl LiveZellij {
    /// Spawn an attached Zellij with an isolated, deterministic state: the
    /// default configuration is dumped from the installed binary into the
    /// isolated config dir first, so no first-run configuration dialog and no
    /// user configuration interferes with the session. The dump is seeded
    /// with [`POPUP_SUPPRESSION_CONFIG`] so session startup draws no popup
    /// over the pane area (issue #147).
    fn spawn(version: &str) -> Self {
        let dirs = ZellijDirs::new();

        let mut dump = Command::new("zellij");
        dump.args(["setup", "--dump-config"]);
        scrub_zellij_env(&mut dump);
        for (key, value) in dirs.env_pairs() {
            dump.env(key, value);
        }
        let dump = dump.output().expect("run zellij setup --dump-config");
        assert!(
            dump.status.success(),
            "zellij setup --dump-config failed: {}",
            String::from_utf8_lossy(&dump.stderr)
        );
        let mut seeded_config = dump.stdout;
        seeded_config.extend_from_slice(POPUP_SUPPRESSION_CONFIG.as_bytes());
        fs::write(dirs.config.join(CONFIG_FILE_NAME), &seeded_config)
            .expect("seed isolated config with the default configuration");

        let launch_policy =
            ZshLaunchPolicy::from_environment().expect("HOME for the live launch policy");
        let metadata = launch_policy.metadata();

        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows: ROWS,
                cols: COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open the live PTY");
        let reader = pair
            .master
            .try_clone_reader()
            .expect("clone the live PTY reader");
        let writer = pair.master.take_writer().expect("take the PTY writer");

        let session = format!(
            "zlv{}x{}",
            std::process::id(),
            DIR_SEQUENCE.fetch_add(1, Ordering::Relaxed) % 100
        );
        let mut command = CommandBuilder::new("zellij");
        command.arg("--session");
        command.arg(&session);
        command.cwd(dirs.home.as_os_str());
        command.env("HOME", dirs.home.as_os_str());
        command.env("TERM", metadata.term);
        command.env("TERM_PROGRAM", metadata.term_program);
        command.env_remove("COLUMNS");
        command.env_remove("LINES");
        scrub_zellij_env_pty(&mut command);
        for (key, value) in dirs.env_pairs() {
            command.env(key, value);
        }
        let child = pair
            .slave
            .spawn_command(command)
            .expect("spawn zellij inside the live PTY");
        drop(pair.slave);

        let (event_tx, events) = mpsc::channel();
        thread::Builder::new()
            .name("zellij-live-reader".to_owned())
            .spawn(move || read_pty(reader, event_tx))
            .expect("spawn the live PTY reader thread");
        println!("live zellij {version}: session {session} attached at {ROWS}x{COLS}");

        Self {
            version: version.to_owned(),
            terminal: TerminalState::new(ROWS, COLS).expect("valid live terminal state"),
            raw: Vec::new(),
            writer,
            child,
            _master: pair.master,
            events,
            dirs,
        }
    }

    /// Drain pending PTY output into the terminal state until `ready`
    /// accepts the current state, or the budget expires.
    fn pump_until(
        &mut self,
        budget: Duration,
        ready: impl Fn(&TerminalState, &[u8]) -> bool,
    ) -> bool {
        let deadline = Instant::now() + budget;
        loop {
            while let Ok(chunk) = self.events.try_recv() {
                self.terminal.feed_bytes(&chunk);
                self.raw.extend_from_slice(&chunk);
            }
            if ready(&self.terminal, &self.raw) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    /// Mirror of `main.rs`'s `current_input_mode`: encode keys for the mode
    /// the live session actually selected.
    fn input_mode(&self) -> InputMode {
        let modes = self.terminal.modes();
        let cursor = if modes.is_application_cursor_key_mode() {
            CursorKeyMode::Application
        } else {
            CursorKeyMode::Normal
        };
        let keypad = if modes.is_application_keypad_mode() {
            KeypadMode::Application
        } else {
            KeypadMode::Numeric
        };
        InputMode::normal().with_cursor(cursor).with_keypad(keypad)
    }

    fn send_key(&mut self, key: Key, modifiers: Modifiers) {
        let input = KeyInput::new(key, KeyPhase::Pressed, modifiers);
        let bytes = KeyEncoder::encode_with(input, self.input_mode())
            .expect("live harness keys are encodable");
        self.write_bytes(&bytes);
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        self.writer
            .write_all(bytes)
            .and_then(|()| self.writer.flush())
            .expect("write to the live PTY");
    }

    /// Gate one chord, assert Noren forwards it untouched, then send the
    /// encoded key exactly as `handle_passthrough_key` would.
    fn forward_stroke(
        &mut self,
        gate: &mut PassthroughGate,
        policy: &PassthroughPolicy,
        stroke: Stroke,
    ) {
        let decision = gate.press(policy, stroke.chord);
        assert!(
            matches!(decision.kind, GateKind::Forwarded),
            "zellij {}: Noren's gate must forward {} byte-for-byte; got {:?} instead",
            self.version,
            stroke.label,
            decision.kind
        );
        assert!(
            decision.replayed.is_empty(),
            "zellij {}: forwarded {} must replay no held chords",
            self.version,
            stroke.label
        );
        self.send_key(stroke.key, stroke.modifiers);
    }

    /// Every DECSET/DECRST sequence seen so far, for drift diagnostics.
    fn decsets(&self) -> Vec<String> {
        let raw = &self.raw;
        let mut found = Vec::new();
        let mut index = 0;
        while index + 3 <= raw.len() {
            if raw[index] == 0x1b && raw[index + 1] == b'[' && raw[index + 2] == b'?' {
                let mut end = index + 3;
                while end < raw.len() && (raw[end].is_ascii_digit() || raw[end] == b';') {
                    end += 1;
                }
                if end < raw.len() && (raw[end] == b'h' || raw[end] == b'l') {
                    found.push(String::from_utf8_lossy(&raw[index..=end]).into_owned());
                    index = end + 1;
                    continue;
                }
            }
            index += 1;
        }
        found
    }
}

/// Run one teardown `zellij` CLI to completion under [`TEARDOWN_BUDGET`],
/// killing it if it overruns. Returns whether it exited (by any means) and
/// its success flag when it did. Teardown subprocesses must never hang the
/// test: [`Drop`] also runs while a panic unwinds, and an unbounded child
/// turns every failing live test into a minutes-long failure.
fn run_zellij_bounded(command: &mut Command) -> Option<bool> {
    let mut child = command.spawn().ok()?;
    let deadline = Instant::now() + TEARDOWN_BUDGET;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.success()),
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Err(_) => return None,
        }
    }
}

impl Drop for LiveZellij {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let mut cleanup = Command::new("zellij");
        cleanup.args(["delete-all-sessions", "--yes"]);
        scrub_zellij_env(&mut cleanup);
        for (key, value) in self.dirs.env_pairs() {
            cleanup.env(key, value);
        }
        let _ = run_zellij_bounded(&mut cleanup);
        self.reap_session_server();
    }
}

impl LiveZellij {
    /// Kill the Zellij server this run spawned, by finding the process whose
    /// command line carries this run's isolated socket directory
    /// (`zellij --server <socket dir>/contract_version_1/<session>`).
    ///
    /// Why reaping is load-bearing (issue #147): `zellij delete-all-sessions
    /// --yes` reports success while the server process itself stays alive —
    /// verified by leaving a live test's server running ~2 minutes past a
    /// successful teardown, and by the ~140 idle leaked servers (each
    /// pinning ~100 MB RSS) found while diagnosing this issue. Because every
    /// run — passing or failing — used to leak one server per spawned
    /// session, repeated runs (verification loops, review runs, suites)
    /// pushed a 16 GB machine into double-digit GB of swap; fork/exec of a
    /// new pane's shell then took 30–60 s in D-state, Zellij's soft 1 s
    /// action-completion deadline (`ACTION_COMPLETION_TIMEOUT`, route.rs —
    /// a diagnostic, the action still completes) and this harness's 8 s
    /// assertion budget blew, and `live_zellij_tab_and_pane_chords…` failed
    /// at "forwarded n did not create Zellij pane 2" despite healthy code.
    /// The harness owns the processes it spawns, so it reaps them itself.
    ///
    /// Scoped to this run's socket directory, so concurrent harness runs
    /// (different directories) are never affected. `pgrep`/`kill` are the
    /// portable crutches for "find process by command line" without unsafe
    /// `libc` calls (the workspace denies hand-written `unsafe`); if either
    /// is missing, teardown degrades to the delete-all-sessions path above
    /// and a visible notice is printed — never a panic: this runs during
    /// unwind.
    fn reap_session_server(&self) {
        let pattern = format!("zellij --server {}", self.dirs.socket.display());
        for _ in 0..SERVER_REAP_ATTEMPTS {
            let Ok(pgrep) = Command::new("pgrep").arg("-f").arg(&pattern).output() else {
                return;
            };
            let pids = String::from_utf8_lossy(&pgrep.stdout);
            let pids: Vec<&str> = pids.split_whitespace().collect();
            if pids.is_empty() {
                return;
            }
            for pid in &pids {
                let _ = Command::new("kill").arg("-9").arg(pid).status();
            }
            thread::sleep(Duration::from_millis(100));
        }
        write_raw_stderr(&format!(
            "NOTICE [zellij_live]: a Zellij server for socket dir {} is still alive after \
             teardown; it pins ~100 MB RSS and, if left to accumulate across runs, starves \
             later live runs (issue #147). Kill it manually: pkill -9 -f '{pattern}'",
            self.dirs.socket.display()
        ));
    }
}

fn read_pty(mut reader: Box<dyn Read + Send>, events: mpsc::Sender<Vec<u8>>) {
    let mut buffer = vec![0_u8; 16 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                if events.send(buffer[..count].to_vec()).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

/// One gated key press: the pass-through chord the gate decides on, the app
/// key event that encodes the forwarded bytes, and a label for assertions.
struct Stroke {
    chord: Chord,
    key: Key,
    modifiers: Modifiers,
    label: String,
}

fn ctrl_stroke(character: char) -> Stroke {
    Stroke {
        chord: Chord::new(GateKeyCode::Char(character), GateModifiers::empty().ctrl())
            .expect("normalized Ctrl chord"),
        key: Key::Character(character),
        modifiers: Modifiers::empty().ctrl(),
        label: format!("Ctrl+{character}"),
    }
}

fn plain_stroke(character: char) -> Stroke {
    Stroke {
        chord: Chord::new(GateKeyCode::Char(character), GateModifiers::empty())
            .expect("normalized plain chord"),
        key: Key::Character(character),
        modifiers: Modifiers::empty(),
        label: format!("{character}"),
    }
}

/// Mirror of `main.rs`'s `palette_policy`: the exit leader `Super+Escape`
/// plus the configured palette chord (`KeymapConfig::default()` is Super+p).
///
/// This is a hand-copy because the production `palette_policy` is a private
/// `fn` in `main.rs` — it CAN drift from production. The production policy
/// itself is pinned by `palette_policy_claims_exactly_super_escape_and_super_p`
/// in `src/main/tests.rs`; if this mirror is ever edited, re-check that pin
/// still describes what is mirrored here (found by independent review).
fn palette_policy() -> PassthroughPolicy {
    let palette_claim = PassthroughClaim {
        id: CLAIM_ID_PALETTE,
        action: PassthroughAction::OpenCommandPalette,
        seq: ChordSeq::single(KeymapConfig::default().palette_open()),
        justification: "live harness mirrors main.rs: the default palette chord (super+p) \
                        lives in the Super/Cmd modifier space the Zellij defaults never bind",
    };
    PassthroughPolicy::try_new(vec![default_exit_claim(), palette_claim])
        .expect("live harness policy is valid and collision-free")
}

/// Key chords bound by `bind "..."` lines of a Zellij configuration dump.
fn bind_chords(config: &str) -> Vec<String> {
    let mut chords = Vec::new();
    for line in config.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("bind ") else {
            continue;
        };
        let bound_part = rest.split('{').next().unwrap_or(rest);
        let mut current: Option<String> = None;
        for character in bound_part.chars() {
            if character == '"' {
                if let Some(chord) = current.take() {
                    chords.push(chord);
                } else {
                    current = Some(String::new());
                }
            } else if let Some(buffer) = current.as_mut() {
                buffer.push(character);
            }
        }
    }
    chords
}

#[test]
fn live_zellij_version_is_reported_against_the_pinned_fixture_tag() {
    let Some(version) = installed_zellij_version() else {
        report_skip("live_zellij_version_is_reported_against_the_pinned_fixture_tag");
        return;
    };
    let pinned = ZELLIJ_FIXTURE_TAG
        .strip_prefix('v')
        .unwrap_or(ZELLIJ_FIXTURE_TAG);
    let relation = if version == pinned {
        "matches"
    } else {
        "DIFFERS from"
    };
    println!("live zellij {version} {relation} the pinned fixture tag {ZELLIJ_FIXTURE_TAG}");
    assert_eq!(
        parse_version_line(&format!("zellij {version}\n")),
        Some(version.as_str()),
        "the installed version should round-trip through the parser"
    );
}

#[test]
fn live_zellij_attach_enables_mouse_modes_in_noren_terminal_state() {
    let Some(version) = installed_zellij_version() else {
        report_skip("live_zellij_attach_enables_mouse_modes_in_noren_terminal_state");
        return;
    };
    let mut session = LiveZellij::spawn(&version);
    let ready = session.pump_until(ATTACH_BUDGET, |terminal, raw| {
        let bytes = |sequence: &[u8]| raw.windows(sequence.len()).any(|w| w == sequence);
        bytes(b"\x1b[?1002h")
            && bytes(b"\x1b[?1006h")
            && terminal.modes().is_mouse_button_event_tracking_enabled()
            && terminal.modes().is_mouse_sgr_encoding_enabled()
    });
    let decsets = session.decsets();
    let multi_parameter = decsets.iter().filter(|s| s.contains(';')).count();
    println!(
        "live zellij {version}: {} DECSET/DECRST sequences on attach, \
         {multi_parameter} of them multi-parameter",
        decsets.len()
    );
    assert!(
        ready,
        "live zellij {version}: DECSET 1002/1006 mouse modes did not reach Noren's \
         terminal state; seen sequences: {decsets:?}; modes: {:?}",
        session.terminal.modes()
    );

    // Co-located regression pin for the multi-parameter DECSET path (PR #113).
    // See the module docs: the installed Zellij 0.44.3 sends 1002 and 1006 as
    // SEPARATE single-parameter DECSETs (the inventory printed above is the
    // live evidence), so the combined form — a proven regression site that
    // other terminal multiplexers do send — is pinned here by driving it
    // through the SAME TerminalState instance that just parsed the live
    // attach stream. A parser that bails on multi-parameter DECSETs fails
    // this test.
    let state = &mut session.terminal;
    state.feed_bytes(b"\x1b[?1002l\x1b[?1006l");
    assert!(
        !state.modes().is_mouse_button_event_tracking_enabled()
            && !state.modes().is_mouse_sgr_encoding_enabled(),
        "zellij {version}: DECRST through the live parser must disable both mouse modes \
         before the multi-parameter pin runs"
    );
    state.feed_bytes(b"\x1b[?1002;1006h");
    assert!(
        state.modes().is_mouse_button_event_tracking_enabled()
            && state.modes().is_mouse_sgr_encoding_enabled(),
        "zellij {version}: the multi-parameter DECSET `CSI ? 1002;1006 h` (the PR #113 \
         regression site) no longer enables both mouse modes in Noren's terminal state"
    );
}

#[test]
fn live_zellij_tab_and_pane_chords_are_forwarded_and_handled_by_zellij() {
    let Some(version) = installed_zellij_version() else {
        report_skip("live_zellij_tab_and_pane_chords_are_forwarded_and_handled_by_zellij");
        return;
    };
    let mut session = LiveZellij::spawn(&version);
    let policy = palette_policy();
    let mut gate = PassthroughGate::new();

    assert!(
        session.pump_until(ATTACH_BUDGET, |terminal, _| snapshot_has(
            terminal, "Tab #1"
        )),
        "live zellij {version}: the session never rendered its tab bar; screen:\n{}",
        session.terminal.snapshot().display_lines().join("\n")
    );

    let exit = Chord::new(GateKeyCode::Escape, GateModifiers::empty().super_key())
        .expect("normalized Super+Escape");
    let decision = gate.press(&policy, exit);
    assert!(
        matches!(
            decision.kind,
            GateKind::Intercepted(PassthroughAction::ExitToWorkspace)
        ),
        "zellij {version}: Noren's own Super+Escape exit claim must still intercept while \
         a live session runs; got {:?} instead",
        decision.kind
    );

    session.forward_stroke(&mut gate, &policy, ctrl_stroke('t'));
    assert!(
        session.pump_until(INTERACTION_BUDGET, |terminal, _| snapshot_has(
            terminal, "New"
        )),
        "live zellij {version}: forwarded Ctrl+t did not switch Zellij into TAB mode; screen:\n{}",
        session.terminal.snapshot().display_lines().join("\n")
    );

    session.forward_stroke(&mut gate, &policy, plain_stroke('n'));
    assert!(
        session.pump_until(INTERACTION_BUDGET, |terminal, _| snapshot_has(
            terminal, "Tab #2"
        )),
        "live zellij {version}: forwarded n did not create Zellij tab 2; screen:\n{}",
        session.terminal.snapshot().display_lines().join("\n")
    );

    session.forward_stroke(&mut gate, &policy, ctrl_stroke('p'));
    assert!(
        session.pump_until(INTERACTION_BUDGET, |terminal, _| snapshot_has(
            terminal, "New"
        )),
        "live zellij {version}: forwarded Ctrl+p did not switch Zellij into PANE mode; screen:\n{}",
        session.terminal.snapshot().display_lines().join("\n")
    );

    session.forward_stroke(&mut gate, &policy, plain_stroke('n'));
    assert!(
        session.pump_until(INTERACTION_BUDGET, |terminal, _| snapshot_has(
            terminal, "Pane #2"
        )),
        "live zellij {version}: forwarded n did not create Zellij pane 2; screen:\n{}",
        session.terminal.snapshot().display_lines().join("\n")
    );

    for character in "hello-noren".chars() {
        session.send_key(Key::Character(character), Modifiers::empty());
    }
    assert!(
        session.pump_until(INTERACTION_BUDGET, |terminal, _| snapshot_has(
            terminal,
            "hello-noren"
        )),
        "live zellij {version}: forwarded text did not reach the pane's shell; screen:\n{}",
        session.terminal.snapshot().display_lines().join("\n")
    );
}

/// Issue #147 regression pin, co-located with the live evidence it guards:
/// the harness's seeded configuration must suppress every session-start
/// popup (release notes with the sponsor notice, random startup tips), so
/// the pane area shows only pane output and the screen assertions of the
/// tab/pane test depend on the code under test, not on which popup the
/// installed Zellij happens to draw. Removing the suppression from the
/// seeded config makes the release-notes popup appear on every fresh
/// harness run (the harness isolates `HOME`, so the seen-release-notes
/// cache is always empty) and this test FAILS — the suppression is
/// load-bearing, not decorative. The extra drain window after the tab bar
/// exists because the popup is a WASM plugin pane that loads slightly after
/// the first frame; absence is only meaningful once the session is fully
/// attached.
#[test]
fn live_zellij_session_starts_without_popups_over_the_pane_area() {
    let Some(version) = installed_zellij_version() else {
        report_skip("live_zellij_session_starts_without_popups_over_the_pane_area");
        return;
    };
    let mut session = LiveZellij::spawn(&version);
    assert!(
        session.pump_until(ATTACH_BUDGET, |terminal, _| snapshot_has(
            terminal, "Tab #1"
        )),
        "live zellij {version}: the session never rendered its tab bar; screen:\n{}",
        session.terminal.snapshot().display_lines().join("\n")
    );
    // Drain a full settle window so a popup pane loading after the first
    // frame cannot slip past the check below.
    session.pump_until(POPUP_SETTLE_BUDGET, |_, _| false);

    let drawn: Vec<&str> = POPUP_MARKERS
        .iter()
        .copied()
        .filter(|marker| snapshot_has(&session.terminal, marker))
        .collect();
    println!(
        "live zellij {version}: attach screen checked against {} popup markers, \
         {} drawn",
        POPUP_MARKERS.len(),
        drawn.len()
    );
    assert!(
        drawn.is_empty(),
        "live zellij {version}: a session-start popup drew {drawn:?} over the pane \
         area — the harness's seeded popup suppression (issue #147) is not in \
         effect; screen:\n{}",
        session.terminal.snapshot().display_lines().join("\n")
    );
}

#[test]
fn live_zellij_default_keybinds_bind_no_super_chord() {
    let Some(version) = installed_zellij_version() else {
        report_skip("live_zellij_default_keybinds_bind_no_super_chord");
        return;
    };
    let dirs = ZellijDirs::new();
    let mut dump = Command::new("zellij");
    dump.args(["setup", "--dump-config"]);
    scrub_zellij_env(&mut dump);
    for (key, value) in dirs.env_pairs() {
        dump.env(key, value);
    }
    let dump = dump.output().expect("run zellij setup --dump-config");
    assert!(
        dump.status.success(),
        "zellij {version} setup --dump-config failed: {}",
        String::from_utf8_lossy(&dump.stderr)
    );
    let config = String::from_utf8_lossy(&dump.stdout).into_owned();
    let chords = bind_chords(&config);
    assert!(
        !chords.is_empty(),
        "zellij {version}: no bind chords parsed from the default configuration dump"
    );
    let super_chords: Vec<&String> = chords
        .iter()
        .filter(|chord| {
            let lowered = chord.to_ascii_lowercase();
            // Zellij spells the Super modifier "Super" (and on some builds
            // "Cmd"/"Meta"); any bound chord using that modifier space would
            // collide with Noren's claims.
            lowered.contains("super") || lowered.contains("cmd") || lowered.contains("meta")
        })
        .collect();
    assert!(
        super_chords.is_empty(),
        "zellij {version}: its default keybinds bind Super/Cmd/Meta chords {super_chords:?}, so \
         Noren's Super+Escape exit leader and Super+p palette chords would collide with the \
         installed version"
    );
    println!(
        "live zellij {version}: {} default bind chords verified, none binds the \
         Super/Cmd/Meta modifier space (Noren claims Super+Escape and Super+p)",
        chords.len()
    );
}

fn snapshot_has(terminal: &TerminalState, needle: &str) -> bool {
    terminal
        .snapshot()
        .display_lines()
        .iter()
        .any(|line| line.contains(needle))
}

#[test]
fn version_line_parser_reads_zellij_output_shape() {
    assert_eq!(parse_version_line("zellij 0.44.3\n"), Some("0.44.3"));
    assert_eq!(parse_version_line("zellij 0.41.2\r\n"), Some("0.41.2"));
    assert_eq!(parse_version_line("zellij\n"), None);
    assert_eq!(parse_version_line(""), None);
}
