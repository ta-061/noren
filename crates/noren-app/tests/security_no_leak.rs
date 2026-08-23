//! TM-08 sentinel verification: terminal input, PTY output, working
//! directory, and environment values must never surface in logs or
//! diagnostics.
//!
//! The [threat model](../../docs/security/threat-model.md) names TM-08 and
//! requires "a test logger that rejects known sentinel terminal/input/
//! environment values". This suite injects one unique sentinel into each
//! named channel and scans every observable log and diagnostics surface for
//! them:
//!
//! - **Terminal input** — the input sentinel is typed through
//!   [`KeyEncoder`], the exact path real keystrokes take.
//! - **PTY output** — the output sentinel travels through a live PTY session
//!   (or, when no session can spawn, is fed directly) into [`TerminalState`],
//!   the buffer the diagnostics snapshot reads.
//! - **Working directory** — the child process runs inside a directory whose
//!   name is the cwd sentinel.
//! - **Environment** — a sentinel variable name and value are placed in the
//!   child environment; the [`CONFIG_ENV_VAR`] override additionally embeds
//!   the cwd sentinel in an environment value.
//!
//! The scan runs in a parent/child split: the parent re-spawns this test
//! binary for the child scenario and captures its complete stdout+stderr —
//! the whole log surface — then asserts no sentinel appears. The suite never
//! mutates its own process environment or cwd in-process (the workspace
//! denies hand-written `unsafe`, which both `set_var` and fd redirection
//! would require). Diagnostics is additionally exercised in-process so a
//! leak into [`noren_app::diagnostics::report`] fails directly.
//!
//! The scanner is self-tested against a planted leak, so the suite cannot
//! pass vacuously: if a future change starts logging content, the sentinel
//! scan fails — which is the point of writing it.

use noren_app::config::{AppConfig, CONFIG_ENV_VAR};
use noren_app::diagnostics::{self, PtyChildStatus};
use noren_app::{Key, KeyEncoder, KeyInput, KeyPhase, Modifiers};
use noren_pty::{PtyEvent, PtySession, PtySize};
use noren_terminal::{Cell, TerminalState};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Name of the child scenario test so the parent can run exactly it.
const CHILD_TEST_NAME: &str = "tm08_sentinel_child_scenario";
/// The child runs the scenario only when the parent sets this variable.
const CHILD_TRIGGER_ENV: &str = "NOREN_TM08_CHILD_TRIGGER";
/// Transport variables the parent uses to hand the sentinels to the child.
const INPUT_SENTINEL_ENV: &str = "NOREN_TM08_INPUT_TRANSPORT";
const OUTPUT_SENTINEL_ENV: &str = "NOREN_TM08_OUTPUT_TRANSPORT";
const CWD_SENTINEL_ENV: &str = "NOREN_TM08_CWD_TRANSPORT";
const ENV_NAME_TRANSPORT_ENV: &str = "NOREN_TM08_ENV_NAME_TRANSPORT";
const ENV_VALUE_TRANSPORT_ENV: &str = "NOREN_TM08_ENV_VALUE_TRANSPORT";
/// Printed by the child once the scenario completed, so the parent can
/// distinguish "scenario ran and logged nothing sensitive" from "scenario
/// never ran at all".
const CHILD_COMPLETED_MARKER: &str = "NOREN-TM08-CHILD-SCENARIO-COMPLETED";

static NONCE: AtomicU64 = AtomicU64::new(0);

/// The unique values planted in each TM-08 channel.
struct Sentinels {
    /// Planted as terminal input (typed through the key encoder).
    input: String,
    /// Planted as PTY output (fed through the terminal state).
    output: String,
    /// Planted as the child's working-directory name.
    cwd: String,
    /// Planted as an environment variable name.
    env_name: String,
    /// Planted as that variable's value.
    env_value: String,
    /// Suffix shared by every sentinel: any leak of any sentinel contains
    /// it, so it catches partial truncation of a planted value.
    suffix: String,
}

impl Sentinels {
    fn new() -> Self {
        let pid = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let nonce = NONCE.fetch_add(1, Ordering::Relaxed);
        let suffix = format!("{pid}-{nanos:x}-{nonce}");
        Self {
            input: format!("NOREN-TM08-IN-{suffix}"),
            output: format!("NOREN-TM08-OUT-{suffix}"),
            cwd: format!("NOREN-TM08-CWD-{suffix}"),
            env_name: format!("NOREN_TM08_ENVNAME_{pid}_{nanos:x}_{nonce}"),
            env_value: format!("NOREN-TM08-ENV-{suffix}"),
            suffix,
        }
    }

    /// Every value the scanner must reject in any log or diagnostics output.
    fn fragments(&self) -> [&str; 6] {
        [
            &self.input,
            &self.output,
            &self.cwd,
            &self.env_name,
            &self.env_value,
            &self.suffix,
        ]
    }
}

/// The test logger mandated by TM-08: the first sentinel fragment found in
/// `haystack`, or `None` when the output is clean.
fn leaked_fragment<'a>(haystack: &str, sentinels: &'a Sentinels) -> Option<&'a str> {
    sentinels
        .fragments()
        .into_iter()
        .find(|fragment| haystack.contains(fragment))
}

/// Type one string through the same encoder path the app uses for keystrokes.
fn encode_typed(text: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    for character in text.chars() {
        let press = KeyInput::new(
            Key::Character(character),
            KeyPhase::Pressed,
            Modifiers::empty(),
        );
        bytes.extend(KeyEncoder::encode(press).expect("printable characters encode"));
    }
    bytes
}

/// The scanner must detect a planted leak, otherwise a clean result would be
/// meaningless.
#[test]
fn scanner_flags_planted_leaks_and_accepts_clean_output() {
    let sentinels = Sentinels::new();
    let clean = "noren diagnostics: grid=4x8 modes=alt:0 cursor:0 keypad:0 \
                 scrollback=0/1000 child=running ime_drops=0 state=ok";
    assert!(leaked_fragment(clean, &sentinels).is_none());
    for fragment in sentinels.fragments() {
        let polluted = format!("some log line carrying {fragment}");
        assert_eq!(leaked_fragment(&polluted, &sentinels), Some(fragment));
    }
}

/// Guard the latent `Debug` surface itself. This deliberately formats each
/// content-bearing value directly instead of depending on production to grow
/// a log statement first; mutation M4 (`#[derive(Debug)]`) must fail here.
#[test]
fn terminal_content_debug_is_redacted_without_a_production_formatter() {
    let sentinels = Sentinels::new();
    let content_cell = Cell::new(sentinels.output.clone(), 1);

    let mut terminal = TerminalState::new(2, 160).expect("2x160 is a valid grid");
    terminal.feed_bytes(sentinels.output.as_bytes());
    terminal.feed_bytes(b"\r\n");
    terminal.feed_bytes(sentinels.output.as_bytes());
    terminal.feed_bytes(b"\r\n");
    let snapshot = terminal.snapshot();
    assert!(
        snapshot
            .lines()
            .iter()
            .any(|line| line.contains(&sentinels.output)),
        "fixture must retain the output sentinel on screen"
    );
    assert!(
        snapshot
            .scrollback_lines()
            .iter()
            .any(|line| line.contains(&sentinels.output)),
        "fixture must retain the output sentinel in scrollback"
    );

    let cell_debug = format!("{content_cell:?}");
    let screen_debug = format!("{:?}", snapshot.screen());
    let snapshot_debug = format!("{snapshot:?}");
    let event_debug = format!(
        "{:?}",
        PtyEvent::Output(sentinels.output.as_bytes().to_vec())
    );

    for (carrier, inspected) in [
        ("Cell", cell_debug.as_str()),
        ("ScreenBuffer", screen_debug.as_str()),
        ("TerminalSnapshot", snapshot_debug.as_str()),
        ("PtyEvent", event_debug.as_str()),
    ] {
        assert!(
            leaked_fragment(inspected, &sentinels).is_none(),
            "{carrier} Debug leaked terminal content: {inspected}"
        );
    }
    assert!(
        cell_debug.contains(&format!("text_bytes: {}", sentinels.output.len())),
        "Cell Debug must retain byte-count shape: {cell_debug}"
    );
    assert_eq!(
        screen_debug,
        "ScreenBuffer { rows: 2, cols: 160, cell_count: 320 }"
    );
    assert!(
        snapshot_debug.contains("size: (2, 160)")
            && snapshot_debug.contains("cursor: Cursor { row: 1, column: 0 }")
            && snapshot_debug.contains("scrollback_rows: 1"),
        "TerminalSnapshot Debug must retain grid shape: {snapshot_debug}"
    );
    assert_eq!(
        event_debug,
        format!("Output {{ byte_count: {} }}", sentinels.output.len())
    );
}

/// The diagnostics claim under test: PTY content placed where diagnostics
/// reads its snapshot never reaches the report, and typed input never
/// reaches it either.
#[test]
fn diagnostics_report_never_carries_injected_sentinels() {
    let sentinels = Sentinels::new();

    // PTY output sentinel on the visible screen, in scrollback, and on the
    // alternate screen.
    let mut terminal = TerminalState::new(4, 40).expect("4x40 is a valid grid");
    terminal.feed_bytes(sentinels.output.as_bytes());
    terminal.feed_bytes(b"\n\n\n\n\n");
    terminal.feed_bytes(b"\x1b[?1049h");
    terminal.feed_bytes(sentinels.output.as_bytes());
    let state = terminal.snapshot();
    assert!(
        state
            .lines()
            .iter()
            .chain(&state.scrollback_lines())
            .any(|line| line.contains(&sentinels.output)),
        "fixture must place the output sentinel into terminal content"
    );

    // Terminal input sentinel on the key path.
    let typed = encode_typed(&sentinels.input);
    assert_eq!(typed, sentinels.input.as_bytes());

    for child in [
        PtyChildStatus::NotLaunched,
        PtyChildStatus::Running,
        PtyChildStatus::Exited { code: None },
    ] {
        let report = diagnostics::report(
            &diagnostics::from_snapshot(Some(&state), child)
                .with_persistence_conflict(true)
                .with_persistence_unverified(true),
        );
        assert!(
            leaked_fragment(&report, &sentinels).is_none(),
            "diagnostics must never carry sentinel content: {report}"
        );
        assert!(report.is_ascii(), "no free text can reach the report");
    }
    let report = diagnostics::report(&diagnostics::from_snapshot(None, PtyChildStatus::Running));
    assert!(
        leaked_fragment(&report, &sentinels).is_none(),
        "diagnostics must never carry sentinel content: {report}"
    );
}

/// A process whose input, output, cwd, and environment are saturated with
/// sentinels must produce logs and diagnostics containing none of them.
#[test]
fn sentinels_in_input_output_cwd_and_env_never_reach_logs_or_diagnostics() {
    let sentinels = Sentinels::new();

    // The working-directory injection: the child runs inside a directory
    // whose name is the cwd sentinel.
    let scratch = std::env::temp_dir().join(format!("noren-tm08-{}", std::process::id()));
    let cwd_dir = scratch.join(&sentinels.cwd);
    std::fs::create_dir_all(&cwd_dir).expect("create the sentinel working directory");
    // A valid configuration file inside the sentinel directory; the override
    // environment value therefore embeds the cwd sentinel as well.
    let config_path = cwd_dir.join("config.toml");
    std::fs::write(&config_path, "[font]\ncell_width = 10\ncell_height = 20\n")
        .expect("write the sentinel-directory configuration file");

    let exe = std::env::current_exe().expect("the harness binary is the scenario runner");
    let output = Command::new(exe)
        .arg(CHILD_TEST_NAME)
        .arg("--exact")
        .arg("--nocapture")
        .env(CHILD_TRIGGER_ENV, "1")
        .env(INPUT_SENTINEL_ENV, &sentinels.input)
        .env(OUTPUT_SENTINEL_ENV, &sentinels.output)
        .env(CWD_SENTINEL_ENV, &sentinels.cwd)
        .env(ENV_NAME_TRANSPORT_ENV, &sentinels.env_name)
        .env(ENV_VALUE_TRANSPORT_ENV, &sentinels.env_value)
        .env(&sentinels.env_name, &sentinels.env_value)
        .env(CONFIG_ENV_VAR, &config_path)
        .current_dir(&cwd_dir)
        .output()
        .expect("the sentinel child process runs");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let captured = format!("{stdout}{stderr}");
    let _ = std::fs::remove_dir_all(&scratch);

    assert!(output.status.success(), "child scenario failed: {captured}");
    // Guard against vacuous passes: the scenario must actually have run and
    // actually have emitted the diagnostics line, including the IME drop
    // count recorded for the simulated Ime events.
    assert!(
        captured.contains(CHILD_COMPLETED_MARKER),
        "scenario did not run to completion: {captured}"
    );
    assert!(
        captured.contains("noren diagnostics:"),
        "the diagnostics line was not emitted: {captured}"
    );
    assert!(
        captured.contains("ime_drops=3"),
        "the IME drop count is missing from the emitted line: {captured}"
    );

    // The security assertion itself.
    assert!(
        leaked_fragment(&captured, &sentinels).is_none(),
        "sentinel content reached logs or diagnostics: {captured}"
    );
}

/// Child half of the sentinel scenario. Runs only when re-spawned by
/// [`sentinels_in_input_output_cwd_and_env_never_reach_logs_or_diagnostics`]
/// with the trigger variable set; during a normal test run it is a no-op.
#[test]
fn tm08_sentinel_child_scenario() {
    if std::env::var_os(CHILD_TRIGGER_ENV).is_none() {
        return;
    }

    // Verify every injection actually took effect before trusting the scan.
    let input = std::env::var(INPUT_SENTINEL_ENV).expect("parent supplies the input sentinel");
    let output_sentinel =
        std::env::var(OUTPUT_SENTINEL_ENV).expect("parent supplies the output sentinel");
    let cwd_sentinel = std::env::var(CWD_SENTINEL_ENV).expect("parent supplies the cwd sentinel");
    let env_name = std::env::var(ENV_NAME_TRANSPORT_ENV).expect("parent supplies the env name");
    let env_value = std::env::var(ENV_VALUE_TRANSPORT_ENV).expect("parent supplies the env value");
    assert_eq!(
        std::env::current_dir()
            .expect("the child has a working directory")
            .file_name()
            .and_then(|name| name.to_str()),
        Some(cwd_sentinel.as_str()),
        "the child must run inside the sentinel working directory"
    );
    assert_eq!(
        std::env::var(&env_name).expect("the sentinel environment variable is set"),
        env_value
    );

    // Terminal input: the exact bytes the app would write to the PTY for
    // these keystrokes.
    let typed = encode_typed(&input);
    assert_eq!(typed, input.as_bytes());

    // PTY output: prefer a live PTY session so the sentinels travel the real
    // path; fall back to feeding the identical bytes directly.
    let mut terminal = TerminalState::new(24, 80).expect("24x80 is a valid grid");
    if !route_sentinels_through_live_pty(&mut terminal, &typed, &output_sentinel) {
        terminal.feed_bytes(&typed);
        terminal.feed_bytes(b"\r\n");
        terminal.feed_bytes(output_sentinel.as_bytes());
        terminal.feed_bytes(b"\r\n");
    }
    let state = terminal.snapshot();
    assert!(
        state
            .lines()
            .iter()
            .chain(&state.scrollback_lines())
            .any(|line| line.contains(&output_sentinel)),
        "fixture must place the output sentinel into terminal content"
    );

    // Simulate the Ime arm of `main.rs` observing three IME events: drops
    // are recorded, never their content (the record call accepts none).
    diagnostics::record_ime_drop();
    diagnostics::record_ime_drop();
    diagnostics::record_ime_drop();

    // Emit the diagnostics line exactly as `toggle_diagnostics` in `main.rs`
    // does: one bounded line to standard error.
    let report = diagnostics::report(
        &diagnostics::from_snapshot(Some(&state), PtyChildStatus::Running)
            .with_persistence_conflict(false)
            .with_persistence_unverified(false),
    );
    eprintln!("{report}");

    // The configuration path reads the NOREN_CONFIG environment value, which
    // embeds the cwd sentinel; report failures exactly like `main.rs` does.
    match AppConfig::load() {
        Ok(_) => {}
        Err(error) => eprintln!("Noren configuration is unusable: {error}"),
    }
    let missing = std::env::current_dir()
        .expect("cwd")
        .join("no-such-config.toml");
    if let Err(error) = AppConfig::load_from(&missing) {
        eprintln!("Noren configuration is unusable: {error}");
    }

    println!("{CHILD_COMPLETED_MARKER}");
}

/// Route the sentinels through a live `/bin/zsh` PTY: the encoded keystrokes
/// are written exactly as the app writes input, and the shell's echoed and
/// generated output is fed into the terminal state. Returns `false` when no
/// session can spawn or the markers never surface within the deadline.
fn route_sentinels_through_live_pty(
    terminal: &mut TerminalState,
    typed_input: &[u8],
    output_sentinel: &str,
) -> bool {
    let Ok(mut session) =
        PtySession::spawn(PtySize::from_raw(24, 80).expect("24x80 is a valid PTY size"))
    else {
        return false;
    };

    // Type the input sentinel (its echo and the shell's "command not found"
    // notice both carry it), then make the shell emit the output sentinel on
    // its own line.
    let emit = format!("printf '%s\\n' '{output_sentinel}'\n");
    if session.send_input(typed_input).is_err()
        || session.send_input(b"\r").is_err()
        || session.send_input(emit.as_bytes()).is_err()
    {
        let _ = session.shutdown();
        return false;
    }

    let needle = output_sentinel.as_bytes();
    let mut seen: Vec<u8> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_output = Instant::now();
    let mut marker_seen = false;
    loop {
        match session.try_recv() {
            Ok(Some(PtyEvent::Output(bytes))) => {
                seen.extend_from_slice(&bytes);
                last_output = Instant::now();
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                marker_seen = marker_seen || seen.windows(needle.len()).any(|w| w == needle);
                // Settle only once the marker surfaced, so a pause during
                // shell startup cannot end the drain before the sentinels
                // flow through.
                let settled = marker_seen && last_output.elapsed() > Duration::from_millis(150);
                if settled || Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break,
        }
    }
    let _ = session.shutdown();

    terminal.feed_bytes(&seen);
    marker_seen && seen.windows(needle.len()).any(|window| window == needle)
}
