//! Bounded, opt-in terminal diagnostics without a debugger.
//!
//! One key chord (Super+D, see `main.rs`) asks this module for a single-line
//! report of the live state: grid geometry, active modes, scrollback length,
//! PTY child status, and the IME/dead-key input drop count. Each trigger
//! emits exactly one bounded line — to the window title and standard error —
//! so the feature is opt-in and cannot grow into an unbounded log.
//!
//! # Privacy rule
//!
//! Diagnostics report counters and flags only: grid dimensions, mode bits,
//! scrollback length against its hard cap, the child exit code, and the
//! IME/dead-key drop counter. They never include PTY output bytes, screen
//! cell text, scrollback contents, terminal replies, or input, because that
//! content is user data and may contain secrets. There is deliberately no
//! opt-in for content: no API in this module accepts or returns screen text,
//! and [`report`] cannot name it. Drop counting is a number, not content:
//! [`record_ime_drop`] takes no arguments at all, so there is no path by
//! which a dropped payload could reach the counter or the report. Any future
//! feature that would emit content requires a threat-model change (TM-08)
//! before it is designed.

use noren_terminal::{MAX_SCROLLBACK_LINES, TerminalModes, TerminalSnapshot};
use std::fmt::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};

/// IME and dead-key drops recorded by [`record_ime_drop`].
///
/// The counter is process-wide because drops are observed on the event loop
/// while reports are assembled from terminal snapshots. It can only ever be
/// a number: the recording API accepts no payload, so the count can never
/// carry the dropped character.
static IME_DROP_COUNT: AtomicU64 = AtomicU64::new(0);

/// Record that one IME composition or dead-key event was dropped.
///
/// Deliberately argument-free: only the fact of the drop crosses into
/// diagnostics, never the composed or dead-key text.
pub fn record_ime_drop() {
    IME_DROP_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// Number of IME/dead-key drops recorded in this process so far.
#[must_use]
pub fn ime_drop_count() -> u64 {
    IME_DROP_COUNT.load(Ordering::Relaxed)
}

/// PTY child status as observed by the application.
///
/// The observation is control-plane only (spawn, reap, exit code); it never
/// inspects or reports child output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PtyChildStatus {
    /// No PTY session was started (or it was already torn down).
    NotLaunched,
    /// The child process is expected to be running.
    Running,
    /// The child stream ended; `code` is `None` when only EOF was observed.
    Exited { code: Option<u32> },
}

impl fmt::Display for PtyChildStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotLaunched => f.write_str("not launched"),
            Self::Running => f.write_str("running"),
            Self::Exited { code: Some(code) } => write!(f, "exited(code={code})"),
            Self::Exited { code: None } => f.write_str("exited"),
        }
    }
}

/// Inputs for one diagnostics report: counters and flags only.
///
/// Construct via [`from_snapshot`] from live terminal state. The fields
/// intentionally cannot carry screen text or PTY bytes (see module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiagnosticsInput {
    grid_rows: Option<u16>,
    grid_cols: Option<u16>,
    modes: Option<TerminalModes>,
    scrollback_len: usize,
    scrollback_cap: usize,
    pty: PtyChildStatus,
    persistence_conflict: bool,
    persistence_unverified: bool,
}

/// Build diagnostics input from the live terminal snapshot.
///
/// Only the snapshot's geometry, mode flags, and scrollback length are read;
/// its line text is never touched. The reported scrollback ceiling is the
/// terminal foundation's hard cap [`MAX_SCROLLBACK_LINES`], which
/// configuration cannot raise.
#[must_use]
pub fn from_snapshot(snapshot: Option<&TerminalSnapshot>, pty: PtyChildStatus) -> DiagnosticsInput {
    match snapshot {
        Some(snapshot) => DiagnosticsInput {
            grid_rows: Some(snapshot.rows()),
            grid_cols: Some(snapshot.cols()),
            modes: Some(snapshot.modes()),
            scrollback_len: snapshot.scrollback().len(),
            scrollback_cap: MAX_SCROLLBACK_LINES,
            pty,
            persistence_conflict: false,
            persistence_unverified: false,
        },
        None => DiagnosticsInput {
            grid_rows: None,
            grid_cols: None,
            modes: None,
            scrollback_len: 0,
            scrollback_cap: MAX_SCROLLBACK_LINES,
            pty,
            persistence_conflict: false,
            persistence_unverified: false,
        },
    }
}

impl DiagnosticsInput {
    /// Add the workspace's best-effort external-state warning to the report.
    #[must_use]
    pub const fn with_persistence_conflict(mut self, conflict: bool) -> Self {
        self.persistence_conflict = conflict;
        self
    }

    /// Record that the last persistence attempt could not be fully inspected,
    /// saved, or verified.
    #[must_use]
    pub const fn with_persistence_unverified(mut self, unverified: bool) -> Self {
        self.persistence_unverified = unverified;
        self
    }
}

/// Render one bounded diagnostics line.
///
/// The output is a fixed field sequence with no free text, so its length is
/// bounded by its numeric fields; hostile terminal state cannot inject
/// content into the overlay or log. The `ime_drops` field reads the
/// process-wide drop counter recorded on the event loop (see
/// [`record_ime_drop`]).
#[must_use]
pub fn report(input: &DiagnosticsInput) -> String {
    let mut out = String::with_capacity(128);
    let _ = write!(out, "noren diagnostics: grid=");
    match (input.grid_rows, input.grid_cols) {
        (Some(rows), Some(cols)) => {
            let _ = write!(out, "{rows}x{cols}");
        }
        _ => out.push_str("none"),
    }
    match input.modes {
        Some(modes) => {
            let _ = write!(
                out,
                " modes=alt:{} cursor:{} keypad:{}",
                bit(modes.is_alternate_screen_active()),
                bit(modes.is_application_cursor_key_mode()),
                bit(modes.is_application_keypad_mode())
            );
        }
        None => out.push_str(" modes=none"),
    }
    let _ = write!(
        out,
        " scrollback={}/{} child={} ime_drops={} state={}",
        input.scrollback_len,
        input.scrollback_cap,
        input.pty,
        ime_drop_count(),
        if input.persistence_unverified {
            "unverified"
        } else if input.persistence_conflict {
            "changed-underneath"
        } else {
            "ok"
        }
    );
    out
}

fn bit(flag: bool) -> u8 {
    u8::from(flag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use noren_terminal::TerminalState;

    fn snapshot(rows: u16, cols: u16, bytes: &[u8]) -> TerminalSnapshot {
        let mut terminal = TerminalState::new(rows, cols).expect("valid test terminal");
        terminal.feed_bytes(bytes);
        terminal.snapshot()
    }

    #[test]
    fn report_matches_geometry_modes_and_scrollback_exactly() {
        // DECCKM, application keypad, scrolling lines off the primary screen,
        // and only then the alternate screen.
        let mut terminal = TerminalState::new(4, 8).expect("valid test terminal");
        terminal.feed_bytes(b"\x1b[?1h\x1b=");
        terminal.feed_bytes(b"1\n2\n3\n4\n5\n6\n");
        terminal.feed_bytes(b"\x1b[?1049h");
        let state = terminal.snapshot();

        let input = from_snapshot(Some(&state), PtyChildStatus::Running);
        assert_eq!(input.grid_rows, Some(4));
        assert_eq!(input.grid_cols, Some(8));
        assert_eq!(input.scrollback_len, terminal.scrollback_len());
        assert_eq!(input.scrollback_len, state.scrollback().len());

        let line = report(&input);
        assert!(line.contains("grid=4x8"), "{line}");
        assert!(line.contains("modes=alt:1 cursor:1 keypad:1"), "{line}");
        assert!(
            line.contains(&format!(
                "scrollback={}/{}",
                terminal.scrollback_len(),
                MAX_SCROLLBACK_LINES
            )),
            "{line}"
        );
        assert!(line.contains("child=running"), "{line}");
        assert!(terminal.scrollback_len() >= 2, "scroll occurred in fixture");
    }

    #[test]
    fn report_reflects_cleared_modes_and_no_scrollback() {
        let state = snapshot(3, 5, b"hello");
        let line = report(&from_snapshot(Some(&state), PtyChildStatus::NotLaunched));
        assert!(line.contains("grid=3x5"), "{line}");
        assert!(line.contains("modes=alt:0 cursor:0 keypad:0"), "{line}");
        assert!(
            line.contains(&format!("scrollback=0/{MAX_SCROLLBACK_LINES}")),
            "{line}"
        );
        assert!(line.contains("child=not launched"), "{line}");
    }

    #[test]
    fn report_without_terminal_state_never_panics() {
        let input = from_snapshot(None, PtyChildStatus::Exited { code: Some(2) });
        let line = report(&input);
        assert!(line.contains("grid=none"), "{line}");
        assert!(line.contains("modes=none"), "{line}");
        assert!(
            line.contains(&format!("scrollback=0/{MAX_SCROLLBACK_LINES}")),
            "{line}"
        );
        assert!(line.contains("child=exited(code=2)"), "{line}");
    }

    #[test]
    fn child_status_displays_every_variant() {
        assert_eq!(PtyChildStatus::NotLaunched.to_string(), "not launched");
        assert_eq!(PtyChildStatus::Running.to_string(), "running");
        assert_eq!(PtyChildStatus::Exited { code: None }.to_string(), "exited");
        assert_eq!(
            PtyChildStatus::Exited { code: Some(130) }.to_string(),
            "exited(code=130)"
        );
    }

    /// The privacy rule proven here: screen text fed through the terminal
    /// never appears in diagnostics, even though the snapshot used as input
    /// demonstrably contains it.
    #[test]
    fn report_excludes_screen_and_scrollback_content() {
        let secret = "SECRET-MARKER-9f8e7d6c";
        let mut terminal = TerminalState::new(2, 40).expect("valid test terminal");
        terminal.feed_bytes(secret.as_bytes());
        terminal.feed_bytes(b"\n\n\n\n"); // push the secret line into scrollback
        let state = terminal.snapshot();
        assert!(
            state
                .lines()
                .iter()
                .chain(&state.scrollback_lines())
                .any(|line| line.contains(secret)),
            "fixture must place the secret into terminal content"
        );

        let line = report(&from_snapshot(Some(&state), PtyChildStatus::Running));
        assert!(!line.contains(secret), "{line}");
        assert!(!line.contains("SECRET"), "{line}");
        // No screen text at all: only the fixed counters and flags.
        for token in ["9f8e7d6c", "MARKE"] {
            assert!(!line.contains(token), "{line}");
        }
    }

    #[test]
    fn report_length_is_bounded_for_extreme_inputs() {
        // 1024x1024 is exactly MAX_SCREEN_CELLS, the largest valid grid.
        let mut terminal = TerminalState::new(1024, 1024).expect("within MAX_SCREEN_CELLS");
        terminal.feed_bytes(b"\x1b[?1h\x1b=\x1b[?1049h");
        let state = terminal.snapshot();
        let input = from_snapshot(Some(&state), PtyChildStatus::Exited { code: None })
            .with_persistence_unverified(true);
        let line = report(&input);
        assert!(line.len() < 200, "report must stay bounded: {line}");
        assert!(line.is_ascii(), "no free text can reach the report");
        assert!(line.ends_with("state=unverified"), "{line}");
    }

    /// A sticky historical conflict remains recorded, but it must never hide
    /// the current attempt's unsafe outcome in the single diagnostics field.
    #[test]
    fn current_unverified_state_has_priority_over_sticky_conflict() {
        let input = from_snapshot(None, PtyChildStatus::NotLaunched)
            .with_persistence_conflict(true)
            .with_persistence_unverified(true);
        let line = report(&input);
        assert!(line.ends_with("state=unverified"), "{line}");
        assert!(!line.contains("state=changed-underneath"), "{line}");
    }

    /// The scrollback ceiling diagnostics reports is the terminal
    /// foundation's hard cap. It is a fixed constant, so a configuration can
    /// neither raise it nor (yet) lower it; the report can never name a
    /// different ceiling.
    #[test]
    fn scrollback_is_always_reported_against_the_hard_cap() {
        let state = snapshot(2, 2, b"x");
        let line = report(&from_snapshot(Some(&state), PtyChildStatus::Running));
        assert!(
            line.contains(&format!("scrollback=0/{MAX_SCROLLBACK_LINES}")),
            "{line}"
        );
        assert_eq!(
            from_snapshot(Some(&state), PtyChildStatus::Running).scrollback_cap,
            MAX_SCROLLBACK_LINES
        );
    }

    /// IME/dead-key drops surface in the report as a pure number. The
    /// recording API is argument-free, so the count cannot carry dropped
    /// content by construction; the report must stay bounded ASCII with the
    /// persistence state last.
    #[test]
    fn ime_drops_are_counted_in_the_report_as_payload_free_numbers() {
        let before = ime_drop_count();
        record_ime_drop();
        record_ime_drop();
        record_ime_drop();
        assert_eq!(ime_drop_count(), before + 3);

        let line = report(&from_snapshot(None, PtyChildStatus::NotLaunched));
        assert!(
            line.contains(&format!("ime_drops={}", before + 3)),
            "{line}"
        );
        assert!(line.ends_with("state=ok"), "{line}");
        assert!(line.is_ascii(), "no free text can reach the report");
        assert!(line.len() < 200, "report must stay bounded: {line}");
    }
}
