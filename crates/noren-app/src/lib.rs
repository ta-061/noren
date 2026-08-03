//! App-owned event model and bounded wiring budgets for the macOS local-PTY
//! PoC.
//!
//! This baseline defines the application's own lifecycle, resize, and key
//! types plus the exact channel-capacity and byte-budget constants from the
//! [minimum architecture](https://github.com/ta-061/noren/blob/main/docs/architecture/minimal-local-pty-poc.md).
//! No `winit` / `wgpu` / `swash` dependency belongs here yet: the window event
//! adapter, renderer, and supervisor wiring land in later steps behind these
//! app-owned seams so no platform type crosses a crate boundary.

use std::fmt;
use std::time::Duration;

/// Maximum bytes carried by a single PTY output chunk read from the master.
pub const READ_CHUNK_BYTES: usize = 16 * 1024;

/// Maximum number of output chunks buffered between the reader thread and the
/// main loop. At [`READ_CHUNK_BYTES`] each this is 1 MiB of queued payload.
pub const OUTPUT_CHANNEL_CAPACITY: usize = 64;

/// Maximum number of ordered input/resize/reply commands buffered for the PTY
/// supervisor.
pub const COMMAND_CHANNEL_CAPACITY: usize = 256;

/// Maximum PTY bytes parsed by the main loop in a single turn.
pub const PARSE_BUDGET_BYTES_PER_TURN: usize = 64 * 1024;

/// Maximum opaque reply bytes forwarded to the PTY per main-loop turn.
pub const REPLY_BUDGET_BYTES_PER_TURN: usize = 4 * 1024;

/// Maximum opaque reply bytes forwarded to the PTY per second.
pub const REPLY_BUDGET_BYTES_PER_SECOND: usize = 64 * 1024;

/// Deadline for orderly shutdown: stop input, close the writer, reap the child,
/// and join both worker threads. The retained-slave fallback detaches within
/// the same deadline rather than hang.
pub const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(2);

/// Physical window size reported by the platform, before pixel-to-cell
/// conversion.
///
/// A zero-sized window retains the last valid grid and never sends zero
/// dimensions to the PTY; [`Resize::is_zero`] supports that coalescing rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resize {
    physical_width: u32,
    physical_height: u32,
}

impl Resize {
    /// Create a physical window size.
    #[must_use]
    pub const fn new(physical_width: u32, physical_height: u32) -> Self {
        Self {
            physical_width,
            physical_height,
        }
    }

    /// Physical width in pixels.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.physical_width
    }

    /// Physical height in pixels.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.physical_height
    }

    /// Whether either physical dimension is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.physical_width == 0 || self.physical_height == 0
    }
}

/// Active modifier keys on an app-owned key event.
///
/// The PoC key encoder consumes `ctrl` for control bytes and treats Cmd/Option/
/// IME/dead-key combinations as unsupported drops. The full policy is wired
/// later; this baseline only carries the typed shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    shift: bool,
    ctrl: bool,
    alt: bool,
    super_key: bool,
}

impl Modifiers {
    /// Create an empty modifier set.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            shift: false,
            ctrl: false,
            alt: false,
            super_key: false,
        }
    }

    /// Set the Shift modifier.
    #[must_use]
    pub const fn shift(mut self) -> Self {
        self.shift = true;
        self
    }

    /// Set the Control modifier.
    #[must_use]
    pub const fn ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }

    /// Set the Alt/Option modifier.
    #[must_use]
    pub const fn alt(mut self) -> Self {
        self.alt = true;
        self
    }

    /// Set the Super/Command modifier.
    #[must_use]
    pub const fn super_key(mut self) -> Self {
        self.super_key = true;
        self
    }

    /// Whether Control is held.
    #[must_use]
    pub const fn is_ctrl(self) -> bool {
        self.ctrl
    }
}

/// Arrow key direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrow {
    Up,
    Down,
    Left,
    Right,
}

/// Supported app-owned key identities.
///
/// The PoC encodes printable UTF-8, Enter, Backspace, Tab, Escape, arrows, and
/// Ctrl control bytes. Releases and unsupported combinations emit zero bytes;
/// they are not part of this baseline's encoding step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A printable UTF-8 character.
    Character(char),
    Enter,
    Backspace,
    Tab,
    Escape,
    Arrow(Arrow),
}

/// Whether a key event is a press, an autorepeat, or a release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyPhase {
    Pressed,
    Repeat,
    Released,
}

/// An app-owned key event translated from platform callbacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyInput {
    key: Key,
    phase: KeyPhase,
    modifiers: Modifiers,
}

impl KeyInput {
    /// Create a key event.
    #[must_use]
    pub const fn new(key: Key, phase: KeyPhase, modifiers: Modifiers) -> Self {
        Self {
            key,
            phase,
            modifiers,
        }
    }

    /// The key identity.
    #[must_use]
    pub const fn key(self) -> Key {
        self.key
    }

    /// The press phase.
    #[must_use]
    pub const fn phase(self) -> KeyPhase {
        self.phase
    }

    /// The active modifiers.
    #[must_use]
    pub const fn modifiers(self) -> Modifiers {
        self.modifiers
    }
}

/// App-owned window lifecycle events, translated from platform callbacks.
///
/// The future adapter attaches timestamps; this baseline carries the typed
/// lifecycle shape that the shutdown and redraw state machines observe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleEvent {
    /// The window became active/resumed.
    Resumed,
    /// The window was suspended/backgrounded.
    Suspended,
    /// The event loop is about to wait (coalesce resize here).
    AboutToWait,
    /// A redraw was requested.
    RedrawRequested,
    /// The loop is exiting.
    Exited,
}

/// Typed application errors.
#[derive(Debug)]
pub enum AppError {
    /// A PTY reader could not join within [`SHUTDOWN_DEADLINE`]; the reader was
    /// detached for process-exit cleanup. This is a visible failed acceptance
    /// case, never silent success.
    ReaderJoinTimeout,
    /// The PTY supervisor rejected or failed a command.
    PtyCommand,
    /// A bounded channel disconnected unexpectedly.
    ChannelDisconnected,
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReaderJoinTimeout => {
                write!(f, "PTY reader did not join within {SHUTDOWN_DEADLINE:?}")
            }
            Self::PtyCommand => f.write_str("PTY supervisor command failed"),
            Self::ChannelDisconnected => f.write_str("bounded channel disconnected"),
        }
    }
}

impl std::error::Error for AppError {}

#[cfg(test)]
mod tests {
    use super::*;
    use noren_terminal::TerminalEngine;
    use std::num::NonZeroU16;

    #[test]
    fn budgets_match_minimum_architecture() {
        assert_eq!(READ_CHUNK_BYTES, 16 * 1024);
        assert_eq!(OUTPUT_CHANNEL_CAPACITY, 64);
        assert_eq!(COMMAND_CHANNEL_CAPACITY, 256);
        assert_eq!(PARSE_BUDGET_BYTES_PER_TURN, 64 * 1024);
        assert_eq!(REPLY_BUDGET_BYTES_PER_TURN, 4 * 1024);
        assert_eq!(REPLY_BUDGET_BYTES_PER_SECOND, 64 * 1024);
        assert_eq!(SHUTDOWN_DEADLINE, Duration::from_secs(2));
    }

    #[test]
    fn output_queue_capacity_is_one_mebibyte() {
        assert_eq!(
            OUTPUT_CHANNEL_CAPACITY.checked_mul(READ_CHUNK_BYTES),
            Some(1024 * 1024)
        );
    }

    #[test]
    fn resize_detects_zero_dimension() {
        assert!(Resize::new(0, 0).is_zero());
        assert!(Resize::new(0, 600).is_zero());
        assert!(Resize::new(800, 0).is_zero());
        assert!(!Resize::new(800, 600).is_zero());
    }

    #[test]
    fn key_input_records_identity_phase_and_modifiers() {
        let event = KeyInput::new(Key::Enter, KeyPhase::Pressed, Modifiers::empty().ctrl());
        assert_eq!(event.key(), Key::Enter);
        assert_eq!(event.phase(), KeyPhase::Pressed);
        assert!(event.modifiers().is_ctrl());
    }

    #[test]
    fn lifecycle_events_are_distinct() {
        assert_ne!(LifecycleEvent::Resumed, LifecycleEvent::AboutToWait);
        assert_ne!(LifecycleEvent::RedrawRequested, LifecycleEvent::Exited);
    }

    #[test]
    fn reader_join_timeout_mentions_deadline() {
        assert!(AppError::ReaderJoinTimeout.to_string().contains("2s"));
    }

    /// Wiring smoke test: the app crate resolves its local path dependencies
    /// and a validated PTY size flows through the candidate terminal adapter
    /// without either type crossing the other crate's public boundary.
    #[test]
    fn crates_wire_without_boundary_leak() {
        let size =
            noren_pty::PtySize::new(NonZeroU16::new(4).unwrap(), NonZeroU16::new(8).unwrap());
        let mut engine = noren_terminal::AvtEngine::new(size.rows(), size.cols());
        engine.feed_bytes(b"x");
        let snapshot = engine.snapshot();
        assert_eq!(
            (snapshot.rows(), snapshot.cols()),
            (size.rows(), size.cols())
        );
        assert!(
            snapshot
                .lines()
                .first()
                .is_some_and(|line| line.contains('x'))
        );
    }
}
