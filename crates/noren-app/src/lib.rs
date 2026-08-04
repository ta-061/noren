//! App-owned event model and bounded wiring budgets for the macOS local-PTY
//! PoC.
//!
//! This crate defines the application's own lifecycle, resize, and key types
//! plus the exact channel-capacity and byte-budget constants from the
//! [minimum architecture](https://github.com/ta-061/noren/blob/main/docs/architecture/minimal-local-pty-poc.md).
//! The binary translates `winit` callbacks into these types; platform and GPU
//! types stay inside `noren-app` and never cross into the PTY or terminal
//! crates.

mod input;

pub use input::{CursorKeyMode, FunctionKey, InputMode, KeypadInput, KeypadKey, KeypadMode};

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

/// Fixed PoC cell width in physical pixels.
pub const POC_CELL_WIDTH: u32 = 10;
/// Fixed PoC cell height in physical pixels.
pub const POC_CELL_HEIGHT: u32 = 20;

/// Maximum terminal grid rows the PoC renderer can draw.
///
/// [`GridGeometry::update`] clamps every grid handed to the terminal state and
/// the PTY to this cap, so the PTY, terminal, and rendered grids always agree.
/// The renderer module imports this constant rather than redefining it.
pub const MAX_RENDER_ROWS: u16 = 60;
/// Maximum terminal grid columns the PoC renderer can draw.
///
/// See [`MAX_RENDER_ROWS`] for the shared-cap invariant.
pub const MAX_RENDER_COLS: u16 = 160;

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

/// Non-zero terminal grid calculated from physical window pixels, bounded by
/// the renderer's drawable grid ([`MAX_RENDER_ROWS`] by [`MAX_RENDER_COLS`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    rows: u16,
    cols: u16,
}

impl GridSize {
    /// Row count, always non-zero and at most [`MAX_RENDER_ROWS`].
    #[must_use]
    pub const fn rows(self) -> u16 {
        self.rows
    }

    /// Column count, always non-zero and at most [`MAX_RENDER_COLS`].
    #[must_use]
    pub const fn cols(self) -> u16 {
        self.cols
    }
}

/// Deterministic fixed-cell geometry and resize coalescing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridGeometry {
    cell_width: u32,
    cell_height: u32,
    current: Option<GridSize>,
}

impl GridGeometry {
    /// PoC geometry: 10 physical pixels wide by 20 physical pixels high.
    #[must_use]
    pub const fn poc() -> Self {
        Self {
            cell_width: POC_CELL_WIDTH,
            cell_height: POC_CELL_HEIGHT,
            current: None,
        }
    }

    /// Current last valid grid.
    #[must_use]
    pub const fn current(self) -> Option<GridSize> {
        self.current
    }

    /// Convert a physical resize and return only a changed, non-zero grid.
    ///
    /// A zero physical dimension keeps the previous grid. Pixel sizes smaller
    /// than one cell still produce a one-by-one PTY. Values are capped to the
    /// renderer's drawable grid ([`MAX_RENDER_ROWS`] by [`MAX_RENDER_COLS`])
    /// before crossing the PTY boundary, so the PTY, terminal state, and
    /// rendered grids can never disagree. This is the only place a grid is
    /// calculated; every consumer observes the same clamp.
    pub fn update(&mut self, resize: Resize) -> Option<GridSize> {
        if resize.is_zero() {
            return None;
        }
        let cols = (resize.width() / self.cell_width)
            .clamp(1, u32::from(MAX_RENDER_COLS))
            .try_into()
            .unwrap_or(MAX_RENDER_COLS);
        let rows = (resize.height() / self.cell_height)
            .clamp(1, u32::from(MAX_RENDER_ROWS))
            .try_into()
            .unwrap_or(MAX_RENDER_ROWS);
        let next = GridSize { rows, cols };
        if self.current == Some(next) {
            None
        } else {
            self.current = Some(next);
            Some(next)
        }
    }
}

/// Active modifier keys on an app-owned key event.
///
/// The key encoder consumes `ctrl` for control bytes and `alt` as the xterm
/// `ESC` prefix, and drops Super/Cmd combinations. IME and dead-key input is
/// dropped at translation. This baseline cannot distinguish Option-as-Alt
/// from Option-as-compose on macOS; every Option event is treated as Alt.
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

    /// Whether Shift is held.
    #[must_use]
    pub const fn is_shift(self) -> bool {
        self.shift
    }

    /// Whether Alt/Option is held.
    #[must_use]
    pub const fn is_alt(self) -> bool {
        self.alt
    }

    /// Whether Super/Command is held.
    #[must_use]
    pub const fn is_super(self) -> bool {
        self.super_key
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
/// The PoC encodes printable UTF-8, Enter, Backspace, Tab, Escape, arrows,
/// Delete, Insert, Home, End, PageUp, PageDown, F1-F12, Ctrl control bytes,
/// Ctrl with the base bytes of Enter/Backspace/Tab/Escape, and Alt as an
/// `ESC` prefix over any of those encodings. Releases and still-unsupported
/// combinations emit zero bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A printable UTF-8 character.
    Character(char),
    Enter,
    Backspace,
    Tab,
    Escape,
    Arrow(Arrow),
    /// Forward delete (xterm `CSI 3 ~`), distinct from [`Key::Backspace`].
    Delete,
    /// Insert / toggle key (xterm `CSI 2 ~`).
    Insert,
    /// Home (xterm `CSI H`, or `SS3 H` under DECCKM).
    Home,
    /// End (xterm `CSI F`, or `SS3 F` under DECCKM).
    End,
    /// Page up (xterm `CSI 5 ~`).
    PageUp,
    /// Page down (xterm `CSI 6 ~`).
    PageDown,
    /// Function key F1 through F12.
    Function(FunctionKey),
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

/// Payload-free reason that a key event produced no terminal bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyDropReason {
    Released,
    UnsupportedModifier,
    UnsupportedKey,
    UnsupportedControl,
    ImeOrDeadKey,
}

/// Pure encoder from app-owned key events to terminal bytes.
pub struct KeyEncoder;

impl KeyEncoder {
    /// Encode one pressed or repeated key event in the PoC default
    /// ([`InputMode::normal`]) mode.
    ///
    /// This entry point keeps a mode-free signature for callers that do not
    /// yet track terminal modes. Mode-aware callers use
    /// [`KeyEncoder::encode_with`].
    pub fn encode(input: KeyInput) -> Result<Vec<u8>, KeyDropReason> {
        Self::encode_with(input, InputMode::normal())
    }

    /// Encode one pressed or repeated key event for the active application
    /// input mode.
    ///
    /// Arrow keys, Home, and End observe [`CursorKeyMode`]; Delete, Insert,
    /// PageUp, PageDown, and F1-F12 are mode-independent. Ctrl converts
    /// printable characters to control bytes while Enter, Backspace, Tab, and
    /// Escape keep their base bytes, like xterm. Alt prefixes `ESC` to the
    /// bytes the key would otherwise emit, matching xterm's meta behavior.
    /// Release, Super, and still-unsupported combinations are drops.
    /// [`KeyEncoder::encode`] applies the same rules in the normal mode.
    pub fn encode_with(input: KeyInput, mode: InputMode) -> Result<Vec<u8>, KeyDropReason> {
        if input.phase() == KeyPhase::Released {
            return Err(KeyDropReason::Released);
        }
        let modifiers = input.modifiers();
        if modifiers.is_super() {
            return Err(KeyDropReason::UnsupportedModifier);
        }
        let alt = modifiers.is_alt();
        let key = if modifiers.is_ctrl() {
            match input.key() {
                Key::Character(character) => {
                    let Some(byte) = control_byte(character) else {
                        return Err(KeyDropReason::UnsupportedControl);
                    };
                    return Ok(Self::alt_prefixed(alt, vec![byte]));
                }
                // xterm keeps the base bytes of these named keys under Ctrl.
                key @ (Key::Enter | Key::Backspace | Key::Tab | Key::Escape) => key,
                _ => return Err(KeyDropReason::UnsupportedControl),
            }
        } else {
            input.key()
        };

        let bytes = match key {
            Key::Character(character) if !character.is_control() => {
                let mut buffer = [0_u8; 4];
                character.encode_utf8(&mut buffer).as_bytes().to_vec()
            }
            Key::Character(_) => return Err(KeyDropReason::UnsupportedKey),
            Key::Enter => vec![0x0d],
            Key::Backspace => vec![0x7f],
            Key::Tab => vec![0x09],
            Key::Escape => vec![0x1b],
            Key::Arrow(arrow) => input::cursor_bytes(arrow, mode.cursor()).to_vec(),
            Key::Delete => b"\x1b[3~".to_vec(),
            Key::Insert => b"\x1b[2~".to_vec(),
            Key::Home => input::home_bytes(mode.cursor()).to_vec(),
            Key::End => input::end_bytes(mode.cursor()).to_vec(),
            Key::PageUp => b"\x1b[5~".to_vec(),
            Key::PageDown => b"\x1b[6~".to_vec(),
            Key::Function(function_key) => input::function_key_bytes(function_key).to_vec(),
        };
        Ok(Self::alt_prefixed(alt, bytes))
    }

    /// Prepend the `ESC` byte that Alt adds in front of a key's base bytes.
    fn alt_prefixed(alt: bool, mut bytes: Vec<u8>) -> Vec<u8> {
        if alt {
            bytes.insert(0, 0x1b);
        }
        bytes
    }

    /// Encode one pressed or repeated keypad key event in the PoC default
    /// ([`InputMode::normal`]) numeric-keypad mode.
    pub fn encode_keypad(input: KeypadInput) -> Result<Vec<u8>, KeyDropReason> {
        Self::encode_keypad_with(input, InputMode::normal())
    }

    /// Encode one pressed or repeated keypad key event for the active keypad
    /// mode.
    ///
    /// Numeric mode emits the literal key character; application mode emits the
    /// `SS3` (`ESC O`) sequence. Release, Control, Option, and Command handling
    /// follows the main key path; Shift does not alter these bounded sequences.
    pub fn encode_keypad_with(
        input: KeypadInput,
        mode: InputMode,
    ) -> Result<Vec<u8>, KeyDropReason> {
        if input.phase() == KeyPhase::Released {
            return Err(KeyDropReason::Released);
        }
        let modifiers = input.modifiers();
        if modifiers.is_alt() || modifiers.is_super() {
            return Err(KeyDropReason::UnsupportedModifier);
        }
        if modifiers.is_ctrl() {
            return Err(KeyDropReason::UnsupportedControl);
        }
        Ok(input::keypad_bytes(input.key(), mode.keypad()).to_vec())
    }
}

fn control_byte(character: char) -> Option<u8> {
    match character.to_ascii_uppercase() {
        '@' | ' ' => Some(0x00),
        'A'..='Z' => Some((character.to_ascii_uppercase() as u8) - b'A' + 1),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' => Some(0x1f),
        _ => None,
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
    fn geometry_coalesces_duplicate_and_zero_resizes() {
        let mut geometry = GridGeometry::poc();
        let first = geometry.update(Resize::new(900, 600)).expect("new grid");
        assert_eq!((first.rows(), first.cols()), (30, 90));
        assert_eq!(geometry.update(Resize::new(900, 600)), None);
        assert_eq!(geometry.update(Resize::new(0, 0)), None);
        assert_eq!(geometry.current(), Some(first));
        let tiny = geometry.update(Resize::new(1, 1)).expect("changed grid");
        assert_eq!((tiny.rows(), tiny.cols()), (1, 1));
    }

    #[test]
    fn geometry_clamps_extreme_resizes_without_overflow() {
        let mut geometry = GridGeometry::poc();
        let grid = geometry
            .update(Resize::new(u32::MAX, u32::MAX))
            .expect("new grid");
        assert_eq!(
            (grid.rows(), grid.cols()),
            (MAX_RENDER_ROWS, MAX_RENDER_COLS)
        );
    }

    #[test]
    fn geometry_clamps_oversized_resizes_to_the_render_grid() {
        let mut geometry = GridGeometry::poc();
        let beyond = geometry
            .update(Resize::new(
                u32::from(MAX_RENDER_COLS + 40) * POC_CELL_WIDTH,
                u32::from(MAX_RENDER_ROWS + 20) * POC_CELL_HEIGHT,
            ))
            .expect("new grid");
        assert_eq!(
            (beyond.rows(), beyond.cols()),
            (MAX_RENDER_ROWS, MAX_RENDER_COLS)
        );

        let mut fresh = GridGeometry::poc();
        let one_dimension_over = fresh
            .update(Resize::new(
                u32::from(MAX_RENDER_COLS + 1) * POC_CELL_WIDTH,
                u32::from(MAX_RENDER_ROWS) * POC_CELL_HEIGHT,
            ))
            .expect("new grid");
        assert_eq!(
            (one_dimension_over.rows(), one_dimension_over.cols()),
            (MAX_RENDER_ROWS, MAX_RENDER_COLS)
        );
    }

    #[test]
    fn geometry_keeps_resizes_within_the_render_cap_unaffected() {
        let mut geometry = GridGeometry::poc();
        let normal = geometry.update(Resize::new(900, 600)).expect("new grid");
        assert_eq!((normal.rows(), normal.cols()), (30, 90));

        let at_cap = geometry
            .update(Resize::new(
                u32::from(MAX_RENDER_COLS) * POC_CELL_WIDTH,
                u32::from(MAX_RENDER_ROWS) * POC_CELL_HEIGHT,
            ))
            .expect("changed grid");
        assert_eq!(
            (at_cap.rows(), at_cap.cols()),
            (MAX_RENDER_ROWS, MAX_RENDER_COLS)
        );
    }

    #[test]
    fn geometry_rejects_zero_resizes_and_keeps_the_last_grid() {
        let mut geometry = GridGeometry::poc();
        let valid = geometry.update(Resize::new(900, 600)).expect("new grid");
        assert_eq!(geometry.update(Resize::new(0, 600)), None);
        assert_eq!(geometry.update(Resize::new(900, 0)), None);
        assert_eq!(geometry.update(Resize::new(0, 0)), None);
        assert_eq!(geometry.current(), Some(valid));
    }

    #[test]
    fn key_input_records_identity_phase_and_modifiers() {
        let event = KeyInput::new(Key::Enter, KeyPhase::Pressed, Modifiers::empty().ctrl());
        assert_eq!(event.key(), Key::Enter);
        assert_eq!(event.phase(), KeyPhase::Pressed);
        assert!(event.modifiers().is_ctrl());
    }

    #[test]
    fn key_encoder_emits_the_poc_byte_contract() {
        let plain = Modifiers::empty();
        let cases = [
            (Key::Character(' '), b" ".as_slice()),
            (Key::Character('é'), "é".as_bytes()),
            (Key::Enter, b"\r".as_slice()),
            (Key::Backspace, b"\x7f".as_slice()),
            (Key::Tab, b"\t".as_slice()),
            (Key::Escape, b"\x1b".as_slice()),
            (Key::Arrow(Arrow::Up), b"\x1b[A".as_slice()),
            (Key::Arrow(Arrow::Down), b"\x1b[B".as_slice()),
            (Key::Arrow(Arrow::Right), b"\x1b[C".as_slice()),
            (Key::Arrow(Arrow::Left), b"\x1b[D".as_slice()),
        ];
        for (key, expected) in cases {
            let input = KeyInput::new(key, KeyPhase::Pressed, plain);
            assert_eq!(KeyEncoder::encode(input).as_deref(), Ok(expected));
        }

        for (character, byte) in [
            ('a', 1),
            ('Z', 26),
            ('[', 27),
            ('\\', 28),
            (']', 29),
            ('^', 30),
            ('_', 31),
            (' ', 0),
        ] {
            let input = KeyInput::new(
                Key::Character(character),
                KeyPhase::Repeat,
                Modifiers::empty().ctrl(),
            );
            assert_eq!(KeyEncoder::encode(input), Ok(vec![byte]));
        }
    }

    #[test]
    fn common_shell_input_regression_preserves_spaces_and_control_bytes() {
        fn encode_text(text: &str) -> Vec<u8> {
            text.chars()
                .flat_map(|character| {
                    KeyEncoder::encode(KeyInput::new(
                        Key::Character(character),
                        KeyPhase::Pressed,
                        Modifiers::empty(),
                    ))
                    .expect("plain shell text is supported")
                })
                .collect()
        }

        assert_eq!(encode_text("abc XYZ123"), b"abc XYZ123");
        assert_eq!(encode_text("cd ~/Documents"), b"cd ~/Documents");
        assert_eq!(
            KeyEncoder::encode(KeyInput::new(
                Key::Enter,
                KeyPhase::Pressed,
                Modifiers::empty(),
            )),
            Ok(vec![0x0d])
        );
        assert_eq!(
            KeyEncoder::encode(KeyInput::new(
                Key::Backspace,
                KeyPhase::Pressed,
                Modifiers::empty(),
            )),
            Ok(vec![0x7f])
        );
        for (character, byte) in [('c', 0x03), ('d', 0x04)] {
            assert_eq!(
                KeyEncoder::encode(KeyInput::new(
                    Key::Character(character),
                    KeyPhase::Pressed,
                    Modifiers::empty().ctrl(),
                )),
                Ok(vec![byte])
            );
        }
    }

    #[test]
    fn key_encoder_drops_releases_and_unsupported_modifiers() {
        let released = KeyInput::new(Key::Character('x'), KeyPhase::Released, Modifiers::empty());
        assert_eq!(KeyEncoder::encode(released), Err(KeyDropReason::Released));

        for modifiers in [
            Modifiers::empty().super_key(),
            Modifiers::empty().alt().super_key(),
        ] {
            let input = KeyInput::new(Key::Character('x'), KeyPhase::Pressed, modifiers);
            assert_eq!(
                KeyEncoder::encode(input),
                Err(KeyDropReason::UnsupportedModifier)
            );
        }

        let unsupported_control = KeyInput::new(
            Key::Character('1'),
            KeyPhase::Pressed,
            Modifiers::empty().ctrl(),
        );
        assert_eq!(
            KeyEncoder::encode(unsupported_control),
            Err(KeyDropReason::UnsupportedControl)
        );
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
        let mut engine = noren_terminal::TerminalState::new(size.rows(), size.cols())
            .expect("valid terminal state");
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

    #[test]
    fn input_mode_defaults_to_normal_cursor_and_numeric_keypad() {
        let mode = InputMode::normal();
        assert_eq!(mode.cursor(), CursorKeyMode::Normal);
        assert_eq!(mode.keypad(), KeypadMode::Numeric);
        assert_eq!(InputMode::default(), mode);
    }

    #[test]
    fn cursor_keys_select_normal_or_application_sequences_by_mode() {
        let normal = InputMode::normal();
        let application = normal.with_cursor(CursorKeyMode::Application);
        let cases = [
            (Arrow::Up, b"\x1b[A".as_slice(), b"\x1bOA".as_slice()),
            (Arrow::Down, b"\x1b[B", b"\x1bOB"),
            (Arrow::Right, b"\x1b[C", b"\x1bOC"),
            (Arrow::Left, b"\x1b[D", b"\x1bOD"),
        ];
        for (arrow, normal_bytes, application_bytes) in cases {
            let input = KeyInput::new(Key::Arrow(arrow), KeyPhase::Pressed, Modifiers::empty());
            assert_eq!(KeyEncoder::encode(input).as_deref(), Ok(normal_bytes));
            assert_eq!(
                KeyEncoder::encode_with(input, normal).as_deref(),
                Ok(normal_bytes)
            );
            assert_eq!(
                KeyEncoder::encode_with(input, application).as_deref(),
                Ok(application_bytes)
            );
        }
    }

    #[test]
    fn application_cursor_mode_prefixes_alt_and_keeps_super_control_drops() {
        let application = InputMode::normal().with_cursor(CursorKeyMode::Application);
        let alt_arrow = KeyInput::new(
            Key::Arrow(Arrow::Up),
            KeyPhase::Pressed,
            Modifiers::empty().alt(),
        );
        assert_eq!(
            KeyEncoder::encode_with(alt_arrow, application).as_deref(),
            Ok(b"\x1b\x1bOA".as_slice())
        );
        let super_arrow = KeyInput::new(
            Key::Arrow(Arrow::Up),
            KeyPhase::Pressed,
            Modifiers::empty().super_key(),
        );
        assert_eq!(
            KeyEncoder::encode_with(super_arrow, application),
            Err(KeyDropReason::UnsupportedModifier)
        );
        let ctrl_arrow = KeyInput::new(
            Key::Arrow(Arrow::Up),
            KeyPhase::Pressed,
            Modifiers::empty().ctrl(),
        );
        assert_eq!(
            KeyEncoder::encode_with(ctrl_arrow, application),
            Err(KeyDropReason::UnsupportedControl)
        );
        let printable = KeyInput::new(Key::Character('a'), KeyPhase::Pressed, Modifiers::empty());
        assert_eq!(
            KeyEncoder::encode_with(printable, application).as_deref(),
            Ok(b"a".as_slice())
        );
    }

    #[test]
    fn keypad_keys_select_numeric_or_application_sequences_by_mode() {
        let numeric = InputMode::normal();
        let application = numeric.with_keypad(KeypadMode::Application);
        let cases = [
            (KeypadKey::Zero, b"0".as_slice(), b"\x1bOp".as_slice()),
            (KeypadKey::One, b"1", b"\x1bOq"),
            (KeypadKey::Two, b"2", b"\x1bOr"),
            (KeypadKey::Three, b"3", b"\x1bOs"),
            (KeypadKey::Four, b"4", b"\x1bOt"),
            (KeypadKey::Five, b"5", b"\x1bOu"),
            (KeypadKey::Six, b"6", b"\x1bOv"),
            (KeypadKey::Seven, b"7", b"\x1bOw"),
            (KeypadKey::Eight, b"8", b"\x1bOx"),
            (KeypadKey::Nine, b"9", b"\x1bOy"),
            (KeypadKey::Decimal, b".", b"\x1bOn"),
            (KeypadKey::Plus, b"+", b"\x1bOk"),
            (KeypadKey::Minus, b"-", b"\x1bOm"),
            (KeypadKey::Star, b"*", b"\x1bOj"),
            (KeypadKey::Slash, b"/", b"\x1bOo"),
            (KeypadKey::Enter, b"\r", b"\x1bOM"),
        ];
        for (key, numeric_bytes, application_bytes) in cases {
            let input = KeypadInput::new(key, KeyPhase::Pressed);
            assert_eq!(
                KeyEncoder::encode_keypad(input).as_deref(),
                Ok(numeric_bytes)
            );
            assert_eq!(
                KeyEncoder::encode_keypad_with(input, numeric).as_deref(),
                Ok(numeric_bytes)
            );
            assert_eq!(
                KeyEncoder::encode_keypad_with(input, application).as_deref(),
                Ok(application_bytes)
            );
        }
    }

    #[test]
    fn keypad_encoder_drops_releases_in_both_modes() {
        let numeric = InputMode::normal();
        let application = numeric.with_keypad(KeypadMode::Application);
        let released = KeypadInput::new(KeypadKey::Five, KeyPhase::Released);
        assert_eq!(
            KeyEncoder::encode_keypad(released),
            Err(KeyDropReason::Released)
        );
        assert_eq!(
            KeyEncoder::encode_keypad_with(released, application),
            Err(KeyDropReason::Released)
        );
    }

    #[test]
    fn keypad_encoder_preserves_the_existing_modifier_policy() {
        let mode = InputMode::normal().with_keypad(KeypadMode::Application);
        let pressed = KeypadInput::new(KeypadKey::One, KeyPhase::Pressed);

        assert_eq!(
            KeyEncoder::encode_keypad_with(pressed.with_modifiers(Modifiers::empty().ctrl()), mode),
            Err(KeyDropReason::UnsupportedControl)
        );
        for modifiers in [Modifiers::empty().alt(), Modifiers::empty().super_key()] {
            assert_eq!(
                KeyEncoder::encode_keypad_with(pressed.with_modifiers(modifiers), mode),
                Err(KeyDropReason::UnsupportedModifier)
            );
        }
        assert_eq!(
            KeyEncoder::encode_keypad_with(
                pressed.with_modifiers(Modifiers::empty().shift()),
                mode
            )
            .as_deref(),
            Ok(b"\x1bOq".as_slice())
        );
    }

    #[test]
    fn input_mode_setters_are_idempotent_and_independent() {
        let base = InputMode::normal();
        let once = base
            .with_cursor(CursorKeyMode::Application)
            .with_keypad(KeypadMode::Application);
        let twice = once
            .with_cursor(CursorKeyMode::Application)
            .with_keypad(KeypadMode::Application);
        assert_eq!(once, twice);

        // Resetting to the already-active selector is a no-op.
        assert_eq!(
            InputMode::normal().with_cursor(CursorKeyMode::Normal),
            InputMode::normal()
        );
        assert_eq!(
            InputMode::normal().with_keypad(KeypadMode::Numeric),
            InputMode::normal()
        );

        // Cursor and keypad selectors are independent.
        let cursor_only = base.with_cursor(CursorKeyMode::Application);
        assert_eq!(cursor_only.cursor(), CursorKeyMode::Application);
        assert_eq!(cursor_only.keypad(), KeypadMode::Numeric);

        // Idempotent modes produce byte-identical encodings for every key.
        let arrow = KeyInput::new(Key::Arrow(Arrow::Up), KeyPhase::Pressed, Modifiers::empty());
        assert_eq!(
            KeyEncoder::encode_with(arrow, once),
            KeyEncoder::encode_with(arrow, twice)
        );
        let keypad = KeypadInput::new(KeypadKey::Enter, KeyPhase::Pressed);
        assert_eq!(
            KeyEncoder::encode_keypad_with(keypad, once),
            KeyEncoder::encode_keypad_with(keypad, twice)
        );
    }

    fn all_function_keys() -> [FunctionKey; 12] {
        [
            FunctionKey::F1,
            FunctionKey::F2,
            FunctionKey::F3,
            FunctionKey::F4,
            FunctionKey::F5,
            FunctionKey::F6,
            FunctionKey::F7,
            FunctionKey::F8,
            FunctionKey::F9,
            FunctionKey::F10,
            FunctionKey::F11,
            FunctionKey::F12,
        ]
    }

    #[test]
    fn navigation_keys_emit_their_xterm_bytes() {
        let cases = [
            (Key::Delete, b"\x1b[3~".as_slice()),
            (Key::Insert, b"\x1b[2~"),
            (Key::Home, b"\x1b[H"),
            (Key::End, b"\x1b[F"),
            (Key::PageUp, b"\x1b[5~"),
            (Key::PageDown, b"\x1b[6~"),
        ];
        for (key, expected) in cases {
            let input = KeyInput::new(key, KeyPhase::Pressed, Modifiers::empty());
            assert_eq!(KeyEncoder::encode(input).as_deref(), Ok(expected));
        }
    }

    #[test]
    fn function_keys_emit_the_xterm_ss3_and_csi_bytes() {
        let cases = [
            (FunctionKey::F1, b"\x1bOP".as_slice()),
            (FunctionKey::F2, b"\x1bOQ"),
            (FunctionKey::F3, b"\x1bOR"),
            (FunctionKey::F4, b"\x1bOS"),
            (FunctionKey::F5, b"\x1b[15~"),
            (FunctionKey::F6, b"\x1b[17~"),
            (FunctionKey::F7, b"\x1b[18~"),
            (FunctionKey::F8, b"\x1b[19~"),
            (FunctionKey::F9, b"\x1b[20~"),
            (FunctionKey::F10, b"\x1b[21~"),
            (FunctionKey::F11, b"\x1b[23~"),
            (FunctionKey::F12, b"\x1b[24~"),
        ];
        for (function_key, expected) in cases {
            let input = KeyInput::new(
                Key::Function(function_key),
                KeyPhase::Pressed,
                Modifiers::empty(),
            );
            assert_eq!(KeyEncoder::encode(input).as_deref(), Ok(expected));
        }
    }

    #[test]
    fn home_and_end_switch_to_ss3_under_application_cursor_mode() {
        let application = InputMode::normal().with_cursor(CursorKeyMode::Application);
        let cases = [
            (Key::Home, b"\x1b[H".as_slice(), b"\x1bOH".as_slice()),
            (Key::End, b"\x1b[F".as_slice(), b"\x1bOF".as_slice()),
        ];
        for (key, normal_bytes, application_bytes) in cases {
            let input = KeyInput::new(key, KeyPhase::Pressed, Modifiers::empty());
            assert_eq!(
                KeyEncoder::encode_with(input, InputMode::normal()).as_deref(),
                Ok(normal_bytes)
            );
            assert_eq!(
                KeyEncoder::encode_with(input, application).as_deref(),
                Ok(application_bytes)
            );
        }
    }

    #[test]
    fn mode_independent_keys_ignore_application_cursor_mode() {
        let application = InputMode::normal().with_cursor(CursorKeyMode::Application);
        for key in [Key::Delete, Key::Insert, Key::PageUp, Key::PageDown] {
            let input = KeyInput::new(key, KeyPhase::Pressed, Modifiers::empty());
            let expected = KeyEncoder::encode(input);
            assert_eq!(KeyEncoder::encode_with(input, application), expected);
        }
        for function_key in all_function_keys() {
            let input = KeyInput::new(
                Key::Function(function_key),
                KeyPhase::Pressed,
                Modifiers::empty(),
            );
            let expected = KeyEncoder::encode(input);
            assert_eq!(KeyEncoder::encode_with(input, application), expected);
        }
    }

    #[test]
    fn stage_one_key_releases_emit_nothing() {
        let application = InputMode::normal().with_cursor(CursorKeyMode::Application);
        for key in [
            Key::Delete,
            Key::Insert,
            Key::Home,
            Key::End,
            Key::PageUp,
            Key::PageDown,
        ] {
            let released = KeyInput::new(key, KeyPhase::Released, Modifiers::empty());
            assert_eq!(KeyEncoder::encode(released), Err(KeyDropReason::Released));
            assert_eq!(
                KeyEncoder::encode_with(released, application),
                Err(KeyDropReason::Released)
            );
        }
        for function_key in all_function_keys() {
            let released = KeyInput::new(
                Key::Function(function_key),
                KeyPhase::Released,
                Modifiers::empty(),
            );
            assert_eq!(KeyEncoder::encode(released), Err(KeyDropReason::Released));
            assert_eq!(
                KeyEncoder::encode_with(released, application),
                Err(KeyDropReason::Released)
            );
        }
    }

    #[test]
    fn stage_one_key_repeats_emit_exactly_one_sequence() {
        let cases = [
            (Key::Delete, b"\x1b[3~".as_slice()),
            (Key::Insert, b"\x1b[2~"),
            (Key::Home, b"\x1b[H"),
            (Key::End, b"\x1b[F"),
            (Key::PageUp, b"\x1b[5~"),
            (Key::PageDown, b"\x1b[6~"),
            (Key::Function(FunctionKey::F1), b"\x1bOP"),
            (Key::Function(FunctionKey::F12), b"\x1b[24~"),
        ];
        for (key, expected) in cases {
            let repeated = KeyInput::new(key, KeyPhase::Repeat, Modifiers::empty());
            assert_eq!(KeyEncoder::encode(repeated).as_deref(), Ok(expected));
            let pressed = KeyInput::new(key, KeyPhase::Pressed, Modifiers::empty());
            assert_eq!(KeyEncoder::encode(repeated), KeyEncoder::encode(pressed));
        }
    }

    #[test]
    fn stage_one_ctrl_combinations_remain_dropped() {
        for key in [
            Key::Delete,
            Key::Home,
            Key::PageUp,
            Key::Function(FunctionKey::F1),
            Key::Function(FunctionKey::F12),
        ] {
            let input = KeyInput::new(key, KeyPhase::Pressed, Modifiers::empty().ctrl());
            assert_eq!(
                KeyEncoder::encode(input),
                Err(KeyDropReason::UnsupportedControl)
            );
        }
    }

    #[test]
    fn alt_characters_emit_esc_followed_by_utf8() {
        for (character, expected) in [
            ('f', b"\x1bf".as_slice()),
            ('a', b"\x1ba"),
            ('Z', b"\x1bZ"),
            ('x', b"\x1bx"),
            ('q', b"\x1bq"),
        ] {
            let input = KeyInput::new(
                Key::Character(character),
                KeyPhase::Pressed,
                Modifiers::empty().alt(),
            );
            assert_eq!(KeyEncoder::encode(input).as_deref(), Ok(expected));
        }
    }

    #[test]
    fn alt_non_ascii_characters_emit_esc_followed_by_the_full_utf8() {
        for (character, expected) in [
            ('é', b"\x1b\xc3\xa9".as_slice()),
            ('界', b"\x1b\xe7\x95\x8c"),
        ] {
            let input = KeyInput::new(
                Key::Character(character),
                KeyPhase::Pressed,
                Modifiers::empty().alt(),
            );
            assert_eq!(KeyEncoder::encode(input).as_deref(), Ok(expected));
        }
    }

    #[test]
    fn alt_ctrl_characters_emit_esc_then_the_control_byte() {
        for (character, byte) in [
            ('c', 0x03),
            ('d', 0x04),
            ('@', 0x00),
            (' ', 0x00),
            ('z', 0x1a),
        ] {
            let input = KeyInput::new(
                Key::Character(character),
                KeyPhase::Pressed,
                Modifiers::empty().alt().ctrl(),
            );
            assert_eq!(
                KeyEncoder::encode(input),
                Ok(vec![0x1b, byte]),
                "Alt+Ctrl+{character}"
            );
        }
    }

    #[test]
    fn ctrl_named_keys_keep_their_base_bytes() {
        let cases = [
            (Key::Enter, b"\x0d".as_slice()),
            (Key::Backspace, b"\x7f"),
            (Key::Tab, b"\x09"),
            (Key::Escape, b"\x1b"),
        ];
        for (key, expected) in cases {
            let input = KeyInput::new(key, KeyPhase::Pressed, Modifiers::empty().ctrl());
            assert_eq!(KeyEncoder::encode(input).as_deref(), Ok(expected));
            let repeated = KeyInput::new(key, KeyPhase::Repeat, Modifiers::empty().ctrl());
            assert_eq!(KeyEncoder::encode(repeated).as_deref(), Ok(expected));
        }
    }

    #[test]
    fn alt_ctrl_named_keys_prefix_esc_to_the_ctrl_bytes() {
        let cases = [
            (Key::Enter, b"\x1b\x0d".as_slice()),
            (Key::Backspace, b"\x1b\x7f"),
            (Key::Tab, b"\x1b\x09"),
            (Key::Escape, b"\x1b\x1b"),
        ];
        for (key, expected) in cases {
            let input = KeyInput::new(key, KeyPhase::Pressed, Modifiers::empty().alt().ctrl());
            assert_eq!(KeyEncoder::encode(input).as_deref(), Ok(expected));
        }
    }

    #[test]
    fn alt_named_and_navigation_keys_prefix_esc_to_base_sequences() {
        let application = InputMode::normal().with_cursor(CursorKeyMode::Application);
        let cases = [
            (Key::Enter, b"\x1b\x0d".as_slice()),
            (Key::Backspace, b"\x1b\x7f"),
            (Key::Tab, b"\x1b\x09"),
            (Key::Escape, b"\x1b\x1b"),
            (Key::Arrow(Arrow::Up), b"\x1b\x1b[A"),
            (Key::Arrow(Arrow::Left), b"\x1b\x1b[D"),
            (Key::Delete, b"\x1b\x1b[3~"),
            (Key::PageDown, b"\x1b\x1b[6~"),
            (Key::Function(FunctionKey::F5), b"\x1b\x1b[15~"),
        ];
        for (key, expected) in cases {
            let input = KeyInput::new(key, KeyPhase::Pressed, Modifiers::empty().alt());
            assert_eq!(KeyEncoder::encode(input).as_deref(), Ok(expected));
        }
        let alt_home = KeyInput::new(Key::Home, KeyPhase::Pressed, Modifiers::empty().alt());
        assert_eq!(
            KeyEncoder::encode_with(alt_home, application).as_deref(),
            Ok(b"\x1b\x1bOH".as_slice())
        );
        let alt_up = KeyInput::new(
            Key::Arrow(Arrow::Up),
            KeyPhase::Pressed,
            Modifiers::empty().alt(),
        );
        assert_eq!(
            KeyEncoder::encode_with(alt_up, application).as_deref(),
            Ok(b"\x1b\x1bOA".as_slice())
        );
    }

    #[test]
    fn unsupported_modifier_combinations_still_drop() {
        for key in [Key::Delete, Key::Home, Key::Arrow(Arrow::Up)] {
            let ctrl = KeyInput::new(key, KeyPhase::Pressed, Modifiers::empty().ctrl());
            assert_eq!(
                KeyEncoder::encode(ctrl),
                Err(KeyDropReason::UnsupportedControl)
            );
            let alt_ctrl = KeyInput::new(key, KeyPhase::Pressed, Modifiers::empty().alt().ctrl());
            assert_eq!(
                KeyEncoder::encode(alt_ctrl),
                Err(KeyDropReason::UnsupportedControl)
            );
        }
        let control_character = KeyInput::new(
            Key::Character('\x03'),
            KeyPhase::Pressed,
            Modifiers::empty().alt(),
        );
        assert_eq!(
            KeyEncoder::encode(control_character),
            Err(KeyDropReason::UnsupportedKey)
        );
        let ctrl_digit = KeyInput::new(
            Key::Character('1'),
            KeyPhase::Pressed,
            Modifiers::empty().alt().ctrl(),
        );
        assert_eq!(
            KeyEncoder::encode(ctrl_digit),
            Err(KeyDropReason::UnsupportedControl)
        );
    }

    #[test]
    fn alt_and_ctrl_combination_releases_emit_nothing() {
        let releases = [
            KeyInput::new(
                Key::Character('f'),
                KeyPhase::Released,
                Modifiers::empty().alt(),
            ),
            KeyInput::new(Key::Enter, KeyPhase::Released, Modifiers::empty().ctrl()),
            KeyInput::new(
                Key::Character('c'),
                KeyPhase::Released,
                Modifiers::empty().alt().ctrl(),
            ),
            KeyInput::new(
                Key::Backspace,
                KeyPhase::Released,
                Modifiers::empty().ctrl(),
            ),
        ];
        for released in releases {
            assert_eq!(KeyEncoder::encode(released), Err(KeyDropReason::Released));
        }
    }
}
