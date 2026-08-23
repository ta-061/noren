//! Parse-error sentinel verification for TOML state files: when the
//! malformed token is a key, table header, or unterminated string, the
//! third-party parser's message quotes the offending source line — file
//! content — so the `Parse` variants of both `SessionPersistenceError`
//! and `ConfigError` must carry only a classification and a position,
//! never the parser's text (Issue #145).
//!
//! Modeled on `tests/ssh_security_no_leak.rs`: one unique sentinel is
//! planted into every malformed shape and both `Display` and `Debug` of
//! the typed error are scanned for it. The scanner is self-tested against
//! a planted leak so the suite cannot pass vacuously.
//!
//! The shapes enumerate every position the malformed token can occupy,
//! because the original report was nearly dismissed after a too-narrow
//! probe (a malformed *value* through the typed variants) showed no leak:
//!
//! - **Bare key** — a sentinel-bearing bare key followed by a space is not
//!   a valid key, and the parser's message quotes the whole line.
//! - **Value** — a bare unquoted sentinel is not a TOML value.
//! - **Table name** — a space inside a table header is malformed.
//! - **Inside a quoted string** — an unterminated basic string swallows
//!   the rest of the line, which the parser quotes.
//! - **Duplicate key** — the duplicate-key message embeds the key and
//!   quotes the line.
//!
//! The typed variants (`WrongType`, `UnknownKey`, ...) name a key and
//! carry no payload by design; probes for those shapes are included as
//! regression guards so the split cannot silently rot.
//!
//! Both real sinks are exercised: `main.rs` prints the session-state
//! error with `eprintln!("Noren could not restore sidebar state:
//! {error}")` and the configuration error with `eprintln!("Noren
//! configuration is unusable: {error}")`; those exact strings are
//! reconstructed and scanned.

use noren_app::config::{AppConfig, ConfigError};
use noren_app::session::SessionRegistry;
use noren_app::session_persistence::{SessionPersistenceError, load_bytes};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NONCE: AtomicU64 = AtomicU64::new(0);

/// The unique secret-shaped value planted into every malformed shape.
///
/// Hyphens keep it a legal TOML bare key, so the duplicate-key shape
/// exercises duplicate detection rather than a malformed key.
fn sentinel() -> String {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock advances")
        .as_nanos();
    let nonce = NONCE.fetch_add(1, Ordering::Relaxed);
    format!("NOREN-STATE-hunter2-{pid}-{nanos:x}-{nonce}")
}

/// The test logger contract: reject any sentinel fragment in `haystack`.
fn leaked(haystack: &str, secret: &str) -> bool {
    haystack.contains(secret)
}

/// The scanner must detect a planted leak, or a clean result would be
/// meaningless (mirrors `ssh_security_no_leak.rs`).
#[test]
fn scanner_flags_planted_leaks_and_accepts_clean_output() {
    let secret = sentinel();
    let clean = "session state is not valid TOML at line 2, column 5";
    assert!(!leaked(clean, &secret));
    assert!(leaked(
        &format!("session state is not valid TOML: {secret} = 1"),
        &secret
    ));
}

/// Push one malformed document through `load_bytes` and scan both error
/// renders for the sentinel; also pins that the shape is a `Parse` failure
/// so a shape that silently became valid cannot pass vacuously.
fn assert_no_leak(document: &str) {
    let secret = sentinel();
    let text = document.replace("SENTINEL", &secret);
    let mut registry = SessionRegistry::new();
    let error = load_bytes(text.as_bytes(), &mut registry)
        .expect_err("every probe document must fail to load");
    assert!(
        matches!(error, SessionPersistenceError::Parse { .. }),
        "the probe must fail as a TOML parse error, got {error:?} for {text:?}"
    );
    assert!(
        !leaked(&error.to_string(), &secret),
        "Display leaked file content: {error} (document {text:?})"
    );
    assert!(
        !leaked(&format!("{error:?}"), &secret),
        "Debug leaked file content: {error:?} (document {text:?})"
    );
    assert!(
        registry.is_empty(),
        "a malformed document must load nothing"
    );
}

#[test]
fn a_sentinel_in_a_malformed_bare_key_never_reaches_display_or_debug() {
    // The space after the key makes the bare key malformed; the sentinel
    // itself is legal bare-key text, so the quoted line is the only way
    // it could surface.
    assert_no_leak("version = 1\nSENTINEL notakey = 2\n");
}

#[test]
fn a_sentinel_as_a_bare_value_never_reaches_display_or_debug() {
    assert_no_leak("version = SENTINEL\n");
}

#[test]
fn a_sentinel_in_a_malformed_table_name_never_reaches_display_or_debug() {
    assert_no_leak("version = 1\n[SENTINEL table]\nkind = \"local\"\n");
}

#[test]
fn a_sentinel_inside_an_unterminated_string_never_reaches_display_or_debug() {
    assert_no_leak("version = 1\nselected = \"SENTINEL");
}

#[test]
fn a_sentinel_in_a_duplicate_key_never_reaches_display_or_debug() {
    // Both keys are individually legal TOML; only the duplication is
    // malformed, so the duplicate-key message embeds the sentinel-bearing
    // key name and quotes the line.
    assert_no_leak("SENTINEL = 1\nSENTINEL = 2\n");
}

// ── The typed variants name keys only; lock that in as regression guards ──

#[test]
fn typed_variant_probes_stay_content_free() {
    let probes = [
        "version = 1\nselected = \"SENTINEL\"\n", // wrong type: value not echoed
        "version = \"SENTINEL\"\n",               // wrong type: value not echoed
        "version = 1\n[extra]\nkind = \"SENTINEL\"\n", // unknown table: value not echoed
    ];
    for document in probes {
        let secret = sentinel();
        let text = document.replace("SENTINEL", &secret);
        let mut registry = SessionRegistry::new();
        let error = load_bytes(text.as_bytes(), &mut registry)
            .expect_err("every typed probe must fail to load");
        assert!(
            !matches!(error, SessionPersistenceError::Parse { .. }),
            "typed probes must fail through a typed variant, got {error:?}"
        );
        assert!(
            !leaked(&error.to_string(), &secret),
            "Display leaked file content: {error} (document {text:?})"
        );
        assert!(
            !leaked(&format!("{error:?}"), &secret),
            "Debug leaked file content: {error:?} (document {text:?})"
        );
    }
}

// ── Position retention: the error must stay actionable, not opaque ─────────

#[test]
fn parse_errors_keep_a_1_based_line_and_column() {
    let mut registry = SessionRegistry::new();
    let error = load_bytes(b"version = 1\nnot a key\n", &mut registry)
        .expect_err("the malformed document must fail");
    let SessionPersistenceError::Parse { line, column } = error else {
        panic!("expected a parse error, got {error:?}");
    };
    assert_eq!(line, 2, "the malformed token is on the second line");
    assert!(column >= 1, "columns are 1-based, got {column}");
    let display = error.to_string();
    let expected_prefix = "session state is not valid TOML at line 2, column ";
    assert!(
        display.starts_with(expected_prefix),
        "Display must name the position, got {display:?}"
    );
    let tail = &display[expected_prefix.len()..];
    assert!(
        tail.parse::<usize>().is_ok(),
        "Display must end in the column number, got {display:?}"
    );
}

// ── The real sink: the exact string `main.rs` prints to stderr ─────────────

#[test]
fn the_string_main_prints_on_restore_failure_carries_no_file_content() {
    let secret = sentinel();
    // The issue's reproducing shape: the malformed token is a key.
    let document = format!("version = 1\n{secret} notakey = 2\n");
    let mut registry = SessionRegistry::new();
    let error = load_bytes(document.as_bytes(), &mut registry)
        .expect_err("the malformed document must fail");
    // Byte-for-byte the format `main.rs` uses on the restore-failure path.
    let stderr_line = format!("Noren could not restore sidebar state: {error}");
    assert!(!leaked(&stderr_line, &secret), "{stderr_line}");
}

// ── The same leak class through the configuration surface ──────────────────
//
// `ConfigError::Parse` forwarded the identical `toml_edit` text, and
// `main.rs` prints it live at startup
// (`eprintln!("Noren configuration is unusable: {error}")`). The same
// five shapes apply, with the sentinel placed where a configuration
// file's content would be.

/// Push one malformed configuration through `AppConfig::parse` and scan
/// both error renders for the sentinel; pins the `Parse` variant so a
/// shape that silently became valid cannot pass vacuously.
fn assert_no_config_leak(document: &str) {
    let secret = sentinel();
    let text = document.replace("SENTINEL", &secret);
    let error = AppConfig::parse(&text).expect_err("every config probe must fail to parse");
    assert!(
        matches!(error, ConfigError::Parse { .. }),
        "the config probe must fail as a TOML parse error, got {error:?} for {text:?}"
    );
    assert!(
        !leaked(&error.to_string(), &secret),
        "Display leaked file content: {error} (document {text:?})"
    );
    assert!(
        !leaked(&format!("{error:?}"), &secret),
        "Debug leaked file content: {error:?} (document {text:?})"
    );
}

#[test]
fn a_sentinel_in_a_malformed_config_bare_key_never_reaches_display_or_debug() {
    assert_no_config_leak("[font]\ncell_width = 12\nSENTINEL notakey = 2\n");
}

#[test]
fn a_sentinel_as_a_config_bare_value_never_reaches_display_or_debug() {
    assert_no_config_leak("[font]\ncell_width = SENTINEL\n");
}

#[test]
fn a_sentinel_in_a_malformed_config_table_name_never_reaches_display_or_debug() {
    assert_no_config_leak("[SENTINEL table]\ncell_width = 12\n");
}

#[test]
fn a_sentinel_inside_an_unterminated_config_string_never_reaches_display_or_debug() {
    assert_no_config_leak("[keys]\nsession_create = \"SENTINEL");
}

#[test]
fn a_sentinel_in_a_duplicate_config_key_never_reaches_display_or_debug() {
    assert_no_config_leak("[font]\ncell_width = 12\nSENTINEL = 1\nSENTINEL = 2\n");
}

#[test]
fn config_parse_errors_keep_a_1_based_line_and_column() {
    let error = AppConfig::parse("[font]\ncell_width = 12\nnot a key\n")
        .expect_err("the malformed configuration must fail");
    let ConfigError::Parse { line, column } = error else {
        panic!("expected a parse error, got {error:?}");
    };
    assert_eq!(line, 3, "the malformed token is on the third line");
    assert!(column >= 1, "columns are 1-based, got {column}");
    let display = error.to_string();
    let expected_prefix = "configuration is not valid TOML at line 3, column ";
    assert!(
        display.starts_with(expected_prefix),
        "Display must name the position, got {display:?}"
    );
    let tail = &display[expected_prefix.len()..];
    assert!(
        tail.parse::<usize>().is_ok(),
        "Display must end in the column number, got {display:?}"
    );
}

#[test]
fn the_string_main_prints_on_config_failure_carries_no_file_content() {
    let secret = sentinel();
    let document = format!("[font]\ncell_width = 12\n{secret} notakey = 2\n");
    let error = AppConfig::parse(&document).expect_err("the malformed config must fail");
    // Byte-for-byte the format `main.rs` uses on the startup-failure path.
    let stderr_line = format!("Noren configuration is unusable: {error}");
    assert!(!leaked(&stderr_line, &secret), "{stderr_line}");
}
