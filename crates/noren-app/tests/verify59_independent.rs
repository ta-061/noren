//! Independent verification of Issue #59 claims, written by the GLM
//! verification lane. These tests were authored without reading the lane's
//! own tests first and probe the public API the way the task describes.

use noren_app::config::{AppConfig, ConfigError};
use noren_app::diagnostics::{self, PtyChildStatus};
use noren_terminal::{MAX_SCREEN_CELLS, MAX_SCROLLBACK_LINES, TerminalState};

const SECRET: &str = "GLM_SECRET_4a3b2c1d-very-private-pty-bytes";

// ── Claim 1: missing or empty config behaves exactly as today ───────────

#[test]
fn verify_default_config_is_byte_for_byte_the_poc_constants() {
    let cfg = AppConfig::default();
    assert_eq!(cfg.font().cell_width(), noren_app::POC_CELL_WIDTH);
    assert_eq!(cfg.font().cell_height(), noren_app::POC_CELL_HEIGHT);
}

#[test]
fn verify_empty_file_yields_defaults() {
    assert_eq!(AppConfig::parse(""), Ok(AppConfig::default()));
    assert_eq!(AppConfig::parse("\n\n\n"), Ok(AppConfig::default()));
    assert_eq!(
        AppConfig::parse("# nothing here\n# or here\n"),
        Ok(AppConfig::default())
    );
}

#[test]
fn verify_whitespace_only_file_yields_defaults() {
    assert_eq!(
        AppConfig::parse("   \t  \n  \t\n"),
        Ok(AppConfig::default())
    );
}

// ── Claim 2: each supported key overrides its default ──────────────────

#[test]
fn verify_cell_width_overrides_default() {
    let cfg = AppConfig::parse("[font]\ncell_width = 11\n").expect("valid");
    assert_eq!(cfg.font().cell_width(), 11);
    // cell_height keeps its default.
    assert_eq!(cfg.font().cell_height(), noren_app::POC_CELL_HEIGHT);
}

#[test]
fn verify_cell_height_overrides_default() {
    let cfg = AppConfig::parse("[font]\ncell_height = 22\n").expect("valid");
    assert_eq!(cfg.font().cell_height(), 22);
    assert_eq!(cfg.font().cell_width(), noren_app::POC_CELL_WIDTH);
}

#[test]
fn verify_both_keys_override_independently() {
    let cfg = AppConfig::parse("[font]\ncell_width = 13\ncell_height = 26\n").expect("valid");
    assert_eq!(cfg.font().cell_width(), 13);
    assert_eq!(cfg.font().cell_height(), 26);
}

// ── Claim 3: hard caps cannot be raised ─────────────────────────────────

#[test]
fn verify_scrollback_key_does_not_exist_and_cannot_raise_cap() {
    // An absurd scrollback value must be rejected outright (UnknownKey),
    // never honored or clamped to a working setting.
    let result = AppConfig::parse("[terminal]\nscrollback_lines = 99999999\n");
    assert!(
        matches!(result, Err(ConfigError::UnknownKey(_))),
        "absurd scrollback must be rejected, got {result:?}"
    );
}

#[test]
fn verify_scrollback_zero_and_negative_are_rejected() {
    for hostile in [
        "[terminal]\nscrollback_lines = 0\n",
        "[terminal]\nscrollback_lines = -5\n",
    ] {
        assert!(
            matches!(AppConfig::parse(hostile), Err(ConfigError::UnknownKey(_))),
            "{hostile:?}"
        );
    }
}

#[test]
fn verify_cell_edge_absurd_values_are_rejected() {
    // cell_width/height are the only grid-affecting keys; absurd values
    // must never produce a grid exceeding the hard caps.
    let hostiles: Vec<String> = vec![
        format!("[font]\ncell_width = {}\n", u32::MAX),
        "[font]\ncell_height = 99999999\n".to_owned(),
        "[font]\ncell_width = 9223372036854775807\n".to_owned(),
    ];
    for hostile in &hostiles {
        assert!(
            matches!(
                AppConfig::parse(hostile),
                Err(ConfigError::OutOfRange { .. })
            ),
            "absurd cell size {hostile:?} must be rejected"
        );
    }
}

#[test]
fn verify_any_configured_grid_stays_within_screen_cell_cap() {
    // Even at the largest accepted cell edge, the derived grid (clamped by
    // the renderer's render cap) must stay inside MAX_SCREEN_CELLS.
    use noren_app::{GridGeometry, Resize};
    // Use edges that satisfy both floors (width>=10, height>=20).
    for (w, h) in [
        (noren_app::POC_CELL_WIDTH, noren_app::POC_CELL_HEIGHT),
        (20, 20),
        (1024, 1024),
    ] {
        let cfg = AppConfig::parse(&format!("[font]\ncell_width = {w}\ncell_height = {h}\n"))
            .expect("valid edge");
        let mut geo = GridGeometry::with_cells(cfg.font().cell_width(), cfg.font().cell_height())
            .expect("valid geometry");
        let grid = geo
            .update(Resize::new(u32::MAX, u32::MAX))
            .expect("non-zero resize");
        let cells = usize::from(grid.rows()) * usize::from(grid.cols());
        assert!(
            cells <= MAX_SCREEN_CELLS,
            "w={w} h={h}: grid {}x{} = {cells} exceeds MAX_SCREEN_CELLS={MAX_SCREEN_CELLS}",
            grid.rows(),
            grid.cols()
        );
    }
}

#[test]
fn verify_scrollback_cap_constant_is_unchanged() {
    // The terminal foundation's hard cap is a constant; config never
    // changes what the diagnostics ceiling reports.
    assert_eq!(MAX_SCROLLBACK_LINES, 10_000);
    assert_eq!(MAX_SCREEN_CELLS, 1024 * 1024);
}

// ── Claim 4: malformed input errors rather than panics ─────────────────

#[test]
fn verify_broken_toml_is_an_error_not_a_panic() {
    let result = std::panic::catch_unwind(|| AppConfig::parse("[font\n"));
    let parsed = result.expect("must not panic");
    assert!(
        matches!(parsed, Err(ConfigError::Parse { .. })),
        "{parsed:?}"
    );
}

#[test]
fn verify_truncated_assignment_is_an_error_not_a_panic() {
    let result = std::panic::catch_unwind(|| AppConfig::parse("[font]\ncell_width = "));
    let parsed = result.expect("must not panic");
    assert!(
        matches!(parsed, Err(ConfigError::Parse { .. })),
        "{parsed:?}"
    );
}

#[test]
fn verify_non_utf8_bytes_are_an_error_not_a_panic() {
    let result =
        std::panic::catch_unwind(|| AppConfig::from_bytes(&[0xff, 0xfe, 0x00, b'[', b'f', b'o']));
    let parsed = result.expect("must not panic");
    assert_eq!(parsed, Err(ConfigError::NotUtf8));
}

#[test]
fn verify_random_garbage_bytes_are_an_error_not_a_panic() {
    let bytes: &[u8] = &[0x80, 0x81, 0x82, 0x9f, 0xc0, 0xc1];
    let result = std::panic::catch_unwind(|| AppConfig::from_bytes(bytes));
    let parsed = result.expect("must not panic");
    assert!(parsed.is_err());
}

// ── Claim 5: diagnostics never emits PTY content ───────────────────────

#[test]
fn verify_diagnostics_excludes_screen_text() {
    let mut term = TerminalState::new(3, 60).expect("valid terminal");
    term.feed_bytes(SECRET.as_bytes());
    let snap = term.snapshot();
    // The secret IS on screen...
    assert!(
        snap.lines().iter().any(|l| l.contains(SECRET)),
        "fixture must place secret on screen: {:?}",
        snap.lines()
    );
    // ...but must NOT appear in the report.
    let line = diagnostics::report(&diagnostics::from_snapshot(
        Some(&snap),
        PtyChildStatus::Running,
    ));
    assert!(!line.contains(SECRET), "LEAK: {line}");
    assert!(!line.contains("GLM_SECRET"), "LEAK: {line}");
}

#[test]
fn verify_diagnostics_excludes_scrollback_text() {
    let mut term = TerminalState::new(2, 60).expect("valid terminal");
    term.feed_bytes(SECRET.as_bytes());
    term.feed_bytes(b"\n\n\n\n"); // push into scrollback
    let snap = term.snapshot();
    assert!(
        snap.scrollback_lines().iter().any(|l| l.contains(SECRET)),
        "fixture must place secret in scrollback: {:?}",
        snap.scrollback_lines()
    );
    let line = diagnostics::report(&diagnostics::from_snapshot(
        Some(&snap),
        PtyChildStatus::Running,
    ));
    assert!(!line.contains(SECRET), "LEAK: {line}");
}

#[test]
fn verify_diagnostics_report_is_bounded_ascii() {
    let mut term = TerminalState::new(2, 2).expect("valid terminal");
    term.feed_bytes(b"\x1b[?1h\x1b=");
    let snap = term.snapshot();
    let line = diagnostics::report(&diagnostics::from_snapshot(
        Some(&snap),
        PtyChildStatus::Exited { code: Some(42) },
    ));
    assert!(line.is_ascii(), "no free text: {line}");
    assert!(line.len() < 256, "bounded: {line}");
    assert!(line.contains("child=exited(code=42)"), "{line}");
}

#[test]
fn verify_diagnostics_input_has_no_content_field() {
    // The API struct itself cannot carry screen text: it is constructed
    // only from a snapshot + child status, and its fields are numeric/flag.
    let input = diagnostics::from_snapshot(None, PtyChildStatus::NotLaunched);
    let debug = format!("{input:?}");
    assert!(
        !debug.contains(SECRET) && !debug.contains("line") && !debug.contains("text"),
        "DiagnosticsInput debug must not expose content: {debug}"
    );
}
