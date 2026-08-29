//! Integration coverage for the app's terminal-wheel ownership boundary.
//!
//! Mouse encoder tests prove byte formats. This suite instead feeds real PTY
//! output through `TerminalState` and asks the same app-layer routing seam the
//! window adapter calls before it mutates scrollback or encodes a report.

use noren_app::wheel_routing::{TerminalWheelOwner, terminal_wheel_owner};
use noren_terminal::TerminalState;

#[test]
fn app_routes_wheel_by_authoritative_terminal_tracking_mode() {
    let mut terminal = TerminalState::new(3, 8).expect("valid terminal");
    assert_eq!(
        terminal_wheel_owner(terminal.modes()),
        TerminalWheelOwner::LocalHistory,
        "without an application claim Noren owns the wheel"
    );

    for (set, reset, label) in [
        (b"\x1b[?1000h".as_slice(), b"\x1b[?1000l".as_slice(), "1000"),
        (b"\x1b[?1002h".as_slice(), b"\x1b[?1002l".as_slice(), "1002"),
        (b"\x1b[?1003h".as_slice(), b"\x1b[?1003l".as_slice(), "1003"),
    ] {
        terminal.feed_bytes(set);
        assert_eq!(
            terminal_wheel_owner(terminal.modes()),
            TerminalWheelOwner::Application,
            "mode {label} must transfer wheel ownership to the application"
        );
        terminal.feed_bytes(reset);
        assert_eq!(
            terminal_wheel_owner(terminal.modes()),
            TerminalWheelOwner::LocalHistory,
            "resetting mode {label} must restore local wheel ownership"
        );
    }
}
