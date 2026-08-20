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
//! and prints `SKIP:` on stderr — CI without Zellij stays green while the
//! output states explicitly that live evidence was NOT gathered. A skip is
//! never reported as a pass.
//!
//! Empirical record (printed by the mouse-mode test against the installed
//! version): Zellij 0.44.3 enables mouse tracking with single-parameter
//! DECSETs (`CSI ? 1002 h`, `CSI ? 1006 h`, ...); the multi-parameter form
//! `CSI ? 1002;1006 h` does not appear in its attach stream. The parser's
//! multi-parameter DECSET handling (fixed in PR #113) therefore stays
//! covered by the `noren-terminal` unit suites; this harness additionally
//! prints the live multi-parameter count so a future Zellij that changes
//! shape surfaces as visible drift.

use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

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
const CONFIG_FILE_NAME: &str = "config.kdl";

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
fn report_skip(test: &str) {
    eprintln!(
        "SKIP [{test}]: zellij is not installed (or `zellij --version` failed); \
         live pass-through evidence was NOT gathered. This is a skip, not a pass."
    );
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
    /// user configuration interferes with the session.
    fn spawn(version: &str) -> Self {
        let dirs = ZellijDirs::new();

        let mut dump = Command::new("zellij");
        dump.args(["setup", "--dump-config"]);
        for (key, value) in dirs.env_pairs() {
            dump.env(key, value);
        }
        let dump = dump.output().expect("run zellij setup --dump-config");
        assert!(
            dump.status.success(),
            "zellij setup --dump-config failed: {}",
            String::from_utf8_lossy(&dump.stderr)
        );
        fs::write(dirs.config.join(CONFIG_FILE_NAME), &dump.stdout)
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
            loop {
                match self.events.try_recv() {
                    Ok(chunk) => {
                        self.terminal.feed_bytes(&chunk);
                        self.raw.extend_from_slice(&chunk);
                    }
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                }
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

impl Drop for LiveZellij {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let mut cleanup = Command::new("zellij");
        cleanup.args(["delete-all-sessions", "--yes"]);
        for (key, value) in self.dirs.env_pairs() {
            cleanup.env(key, value);
        }
        let _ = cleanup.output();
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
/// plus the palette chord `Super+p`.
fn palette_policy() -> PassthroughPolicy {
    let palette_claim = PassthroughClaim {
        id: CLAIM_ID_PALETTE,
        action: PassthroughAction::OpenCommandPalette,
        seq: ChordSeq::single(
            Chord::new(GateKeyCode::Char('p'), GateModifiers::empty().super_key())
                .expect("normalized Super+p"),
        ),
        justification: "live harness mirrors main.rs: Super+p lives in the Super/Cmd \
                        modifier space the Zellij defaults never bind",
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
        let mut quoted = None;
        for character in bound_part.chars() {
            match quoted {
                None if character == '"' => quoted = Some(String::new()),
                Some(current) if character == '"' => {
                    chords.push(current.clone());
                    quoted = None;
                }
                Some(current) => current.push(character),
                None => {}
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

#[test]
fn live_zellij_default_keybinds_bind_no_super_chord() {
    let Some(version) = installed_zellij_version() else {
        report_skip("live_zellij_default_keybinds_bind_no_super_chord");
        return;
    };
    let dirs = ZellijDirs::new();
    let mut dump = Command::new("zellij");
    dump.args(["setup", "--dump-config"]);
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
        .filter(|chord| chord.to_ascii_lowercase().contains("super"))
        .collect();
    assert!(
        super_chords.is_empty(),
        "zellij {version}: its default keybinds bind Super chords {super_chords:?}, so \
         Noren's Super+Escape and Super+p claims would collide with the installed version"
    );
    println!(
        "live zellij {version}: {} default bind chords verified, none binds the Super \
         modifier (Noren claims Super+Escape and Super+p)",
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
