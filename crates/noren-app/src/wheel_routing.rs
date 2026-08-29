//! Terminal wheel-ownership policy shared by the window adapter and
//! integration tests.
//!
//! This module owns only the boundary decision. Platform delta translation,
//! local viewport mutation, and mouse-report encoding stay with their existing
//! owners; both paths must first pass through [`terminal_wheel_owner`].

use noren_terminal::TerminalModes;

/// The layer that owns a wheel event over the terminal surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalWheelOwner {
    /// Noren navigates retained primary-screen history.
    LocalHistory,
    /// The application receives a terminal mouse report.
    Application,
}

/// Decide wheel ownership from the terminal parser's authoritative modes.
///
/// Tracking is an application claim even when Shift is held. Shift bypasses
/// button and motion reporting for local selection, but it does not transfer a
/// tracked wheel back to Noren.
#[must_use]
pub const fn terminal_wheel_owner(modes: TerminalModes) -> TerminalWheelOwner {
    if modes.is_mouse_normal_tracking_enabled()
        || modes.is_mouse_button_event_tracking_enabled()
        || modes.is_mouse_any_event_tracking_enabled()
    {
        TerminalWheelOwner::Application
    } else {
        TerminalWheelOwner::LocalHistory
    }
}
