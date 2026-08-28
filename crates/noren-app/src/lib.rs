//! App-owned event model and bounded wiring budgets for the macOS local-PTY
//! PoC.
//!
//! This crate defines the application's own lifecycle, resize, and key types
//! plus the exact channel-capacity and byte-budget constants from the
//! [minimum architecture](https://github.com/ta-061/noren/blob/main/docs/architecture/minimal-local-pty-poc.md).
//! The binary translates `winit` callbacks into these types; platform and GPU
//! types stay inside `noren-app` and never cross into the PTY or terminal
//! crates.

mod clipboard;
pub mod config;
pub mod cursor;
pub mod diagnostics;
pub mod git_worktree;
mod input;
pub mod mouse;
pub mod palette;
pub mod passthrough;
pub mod session;
pub mod session_persistence;
pub mod session_supervisor;
pub mod sidebar;
pub mod sidebar_text;
pub mod ssh_config;
pub mod theme;
pub mod ui;

pub use clipboard::{
    BRACKET_PASTE_BEGIN, BRACKET_PASTE_END, ClipboardError, PasteReject, SystemClipboard,
};
pub use input::{CursorKeyMode, FunctionKey, InputMode, KeypadInput, KeypadKey, KeypadMode};

/// Encode a user-initiated paste for the PTY, gated on DEC private mode 2004.
///
/// Re-exported so both the library tests and the binary share one policy:
/// bracketed when the application enabled mode 2004, refused (never sent
/// unbracketed) when it is off or unavailable.
pub use clipboard::encode_paste;

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

/// The product string in every user-visible surface: the window title, the
/// status-row texts, and any release artifact naming all read this constant
/// (issue #185) so they cannot drift apart again.
pub const PRODUCT_NAME: &str = "Noren";

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

/// Runtime cell metrics: the configured width and height of one terminal grid
/// cell in physical pixels.
///
/// `GridGeometry` produces this from configuration; every consumer — the
/// renderer's `glyph_vertices`, the offscreen capture path, and the binary's
/// click-to-grid mappers — reads width and height from this single value
/// rather than from a compile-time constant. Bundling the two dimensions
/// prevents width and height from drifting to different origins at a call
/// site (the defect from issue #76: the renderer drew at the constant while
/// the geometry used the configured value).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellMetrics {
    width: u32,
    height: u32,
}

impl CellMetrics {
    /// Cell width in physical pixels.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Cell height in physical pixels.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
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

    /// Geometry with configuration-chosen cell dimensions.
    ///
    /// A zero edge is rejected because grid division would fault; the
    /// configuration loader ([`crate::config`]) range-checks values before
    /// they reach here.
    #[must_use]
    pub fn with_cells(cell_width: u32, cell_height: u32) -> Option<Self> {
        if cell_width == 0 || cell_height == 0 {
            return None;
        }
        Some(Self {
            cell_width,
            cell_height,
            current: None,
        })
    }

    /// Configured cell width in physical pixels.
    ///
    /// This is the single runtime source of truth for cell width: the renderer,
    /// the click-to-grid mapper, and the sidebar boundary all read it from here
    /// rather than from a compile-time constant.
    #[must_use]
    pub const fn cell_width(self) -> u32 {
        self.cell_width
    }

    /// Configured cell height in physical pixels.
    ///
    /// See [`cell_width`](Self::cell_width) for the single-source rationale.
    #[must_use]
    pub const fn cell_height(self) -> u32 {
        self.cell_height
    }

    /// The configured [`CellMetrics`] — the single runtime source of truth
    /// for cell size, threaded to the renderer and click-handling code so
    /// every consumer reads the same width and height.
    #[must_use]
    pub const fn cell_metrics(self) -> CellMetrics {
        CellMetrics {
            width: self.cell_width,
            height: self.cell_height,
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
/// The key encoder consumes `ctrl` for control bytes, encodes Shift, Alt,
/// and Ctrl as the xterm modifier parameter on the navigation keys, and
/// treats Alt as the `ESC` prefix on plain characters and the named keys
/// without a parameter form. Super/Cmd combinations drop; IME and dead-key
/// input is dropped at translation. This baseline cannot distinguish
/// Option-as-Alt from Option-as-compose on macOS; every Option event is
/// treated as Alt.
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
/// Ctrl with the base bytes of Enter/Backspace/Tab/Escape, Alt as an `ESC`
/// prefix over those encodings, and the xterm modifier parameter for
/// Shift/Alt/Ctrl combinations of the navigation keys. Releases and
/// still-unsupported combinations emit zero bytes.
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
    /// PageUp, PageDown, and F1-F12 are mode-independent. Shift, Alt, and
    /// Ctrl combinations of the navigation keys (arrows, Home, End, Delete,
    /// Insert, PageUp, PageDown, F1-F12) carry the xterm modifier parameter:
    /// `CSI 1 ; <mod> <final>` for arrows, Home, End, and F1-F4 and
    /// `CSI <n> ; <mod> ~` for the tilde-style keys, where
    /// `mod = 1 + shift + 2 * alt + 4 * ctrl`; there Alt counts in the
    /// parameter instead of prefixing `ESC`, and the modified form always
    /// uses `CSI`, even under DECCKM. Shift+Tab emits the backtab `CSI Z`.
    /// The remaining keys follow xterm: Ctrl converts printable characters to
    /// control bytes while Enter, Backspace, Tab, and Escape keep their base
    /// bytes, and Alt prefixes `ESC` to the bytes the key would otherwise
    /// emit. Release, Super, and still-unsupported combinations are drops.
    /// [`KeyEncoder::encode`] applies the same rules in the normal mode.
    pub fn encode_with(input: KeyInput, mode: InputMode) -> Result<Vec<u8>, KeyDropReason> {
        if input.phase() == KeyPhase::Released {
            return Err(KeyDropReason::Released);
        }
        let modifiers = input.modifiers();
        if modifiers.is_super() {
            return Err(KeyDropReason::UnsupportedModifier);
        }

        // Navigation-class keys carry the xterm modifier parameter (Alt is
        // bit 2 of the parameter there) before the Alt-as-ESC-prefix rule
        // that applies to the remaining keys.
        if let Some(bytes) = encode_modified_navigation(input.key(), modifiers) {
            return Ok(bytes);
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
            // Ctrl keeps Tab's base byte even with Shift; only a bare Shift
            // produces the backtab.
            Key::Tab if modifiers.is_shift() && !modifiers.is_ctrl() => b"\x1b[Z".to_vec(),
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

/// Encode a navigation-class key carrying the xterm modifier parameter.
///
/// Returns `None` when no Shift/Alt/Ctrl modifier is held or the key is
/// outside the navigation class (arrows, Home, End, Delete, Insert, PageUp,
/// PageDown, F1-F12), leaving the key to the bare byte contract and the
/// remaining modifier policy. The modified form always uses `CSI`, even
/// under DECCKM.
fn encode_modified_navigation(key: Key, modifiers: Modifiers) -> Option<Vec<u8>> {
    let parameter = input::modifier_parameter(modifiers);
    if parameter == 1 {
        return None;
    }
    match key {
        Key::Arrow(arrow) => Some(input::modified_final_bytes(
            input::cursor_final_byte(arrow),
            parameter,
        )),
        Key::Home => Some(input::modified_final_bytes(b'H', parameter)),
        Key::End => Some(input::modified_final_bytes(b'F', parameter)),
        Key::Delete => Some(input::modified_tilde_bytes(3, parameter)),
        Key::Insert => Some(input::modified_tilde_bytes(2, parameter)),
        Key::PageUp => Some(input::modified_tilde_bytes(5, parameter)),
        Key::PageDown => Some(input::modified_tilde_bytes(6, parameter)),
        Key::Function(function_key) => {
            Some(input::modified_function_key_bytes(function_key, parameter))
        }
        _ => None,
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

    /// The stage-1 test at this site asserted that Alt/Ctrl cursor keys stay
    /// dropped under DECCKM; stage 4 encodes their modifier parameter instead.
    /// Super/Command still drops and printables stay byte-identical.
    #[test]
    fn stage_four_modified_cursor_keys_use_csi_even_under_decckm() {
        let application = InputMode::normal().with_cursor(CursorKeyMode::Application);
        let cases = [
            (Key::Arrow(Arrow::Up), b"\x1bOA".as_slice(), b'A'),
            (Key::Arrow(Arrow::Down), b"\x1bOB", b'B'),
            (Key::Arrow(Arrow::Right), b"\x1bOC", b'C'),
            (Key::Arrow(Arrow::Left), b"\x1bOD", b'D'),
            (Key::Home, b"\x1bOH", b'H'),
            (Key::End, b"\x1bOF", b'F'),
        ];
        for (key, unmodified_bytes, final_byte) in cases {
            // Unmodified cursor keys keep the SS3 form under DECCKM.
            let unmodified = KeyInput::new(key, KeyPhase::Pressed, Modifiers::empty());
            assert_eq!(
                KeyEncoder::encode_with(unmodified, application).as_deref(),
                Ok(unmodified_bytes)
            );
            // Modified cursor keys keep the CSI form under DECCKM; the
            // modifier parameter suppresses the SS3 application form.
            for (modifiers, parameter) in stage_four_modifier_parameters() {
                let modified = KeyInput::new(key, KeyPhase::Pressed, modifiers);
                let expected = format!("\x1b[1;{parameter}{}", char::from(final_byte));
                assert_eq!(
                    KeyEncoder::encode_with(modified, application).as_deref(),
                    Ok(expected.as_bytes())
                );
            }
        }
        let super_arrow = KeyInput::new(
            Key::Arrow(Arrow::Up),
            KeyPhase::Pressed,
            Modifiers::empty().super_key(),
        );
        assert_eq!(
            KeyEncoder::encode_with(super_arrow, application),
            Err(KeyDropReason::UnsupportedModifier)
        );
        let printable = KeyInput::new(Key::Character('a'), KeyPhase::Pressed, Modifiers::empty());
        assert_eq!(
            KeyEncoder::encode_with(printable, application).as_deref(),
            Ok(b"a".as_slice())
        );
    }

    /// The stage-2 test at this site asserted Alt prefixes `ESC` to the SS3
    /// sequence under DECCKM and Ctrl arrows drop; stage 4 supersedes both:
    /// Alt is bit 2 and Ctrl bit 4 of the modifier parameter on the
    /// navigation class, and modified cursor keys keep the `CSI` form even
    /// under DECCKM. Super/Command still drops and printables stay
    /// byte-identical.
    #[test]
    fn application_cursor_mode_encodes_alt_and_ctrl_arrows_with_modifier_parameters() {
        let application = InputMode::normal().with_cursor(CursorKeyMode::Application);
        let alt_arrow = KeyInput::new(
            Key::Arrow(Arrow::Up),
            KeyPhase::Pressed,
            Modifiers::empty().alt(),
        );
        assert_eq!(
            KeyEncoder::encode_with(alt_arrow, application).as_deref(),
            Ok(b"\x1b[1;3A".as_slice())
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
            KeyEncoder::encode_with(ctrl_arrow, application).as_deref(),
            Ok(b"\x1b[1;5A".as_slice())
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

    /// The stage-1 test at this site asserted these Ctrl combinations stay
    /// dropped; stage 4 encodes the xterm Ctrl modifier parameter (1 + 4 = 5).
    #[test]
    fn stage_four_ctrl_combinations_encode_modifier_parameters() {
        let cases = [
            (Key::Delete, b"\x1b[3;5~".as_slice()),
            (Key::Home, b"\x1b[1;5H"),
            (Key::PageUp, b"\x1b[5;5~"),
            (Key::Function(FunctionKey::F1), b"\x1b[1;5P"),
            (Key::Function(FunctionKey::F12), b"\x1b[24;5~"),
        ];
        for (key, expected) in cases {
            let input = KeyInput::new(key, KeyPhase::Pressed, Modifiers::empty().ctrl());
            assert_eq!(KeyEncoder::encode(input).as_deref(), Ok(expected));
        }
    }

    /// The xterm modifier parameter table: `1 + shift + 2 * alt + 4 * ctrl`.
    fn stage_four_modifier_parameters() -> [(Modifiers, u8); 7] {
        [
            (Modifiers::empty().shift(), 2),
            (Modifiers::empty().alt(), 3),
            (Modifiers::empty().shift().alt(), 4),
            (Modifiers::empty().ctrl(), 5),
            (Modifiers::empty().shift().ctrl(), 6),
            (Modifiers::empty().alt().ctrl(), 7),
            (Modifiers::empty().shift().alt().ctrl(), 8),
        ]
    }

    #[test]
    fn stage_four_arrow_keys_encode_the_full_modifier_table() {
        let arrows = [
            (Arrow::Up, 'A'),
            (Arrow::Down, 'B'),
            (Arrow::Right, 'C'),
            (Arrow::Left, 'D'),
        ];
        for (arrow, final_char) in arrows {
            for (modifiers, parameter) in stage_four_modifier_parameters() {
                let input = KeyInput::new(Key::Arrow(arrow), KeyPhase::Pressed, modifiers);
                let expected = format!("\x1b[1;{parameter}{final_char}");
                assert_eq!(
                    KeyEncoder::encode(input).as_deref(),
                    Ok(expected.as_bytes())
                );
            }
        }

        // Autorepeat of a modified arrow emits exactly one sequence.
        let repeated = KeyInput::new(
            Key::Arrow(Arrow::Up),
            KeyPhase::Repeat,
            Modifiers::empty().shift(),
        );
        assert_eq!(
            KeyEncoder::encode(repeated).as_deref(),
            Ok(b"\x1b[1;2A".as_slice())
        );
    }

    #[test]
    fn stage_four_shift_tab_is_the_backtab_sequence() {
        let shifted = KeyInput::new(Key::Tab, KeyPhase::Pressed, Modifiers::empty().shift());
        assert_eq!(
            KeyEncoder::encode(shifted).as_deref(),
            Ok(b"\x1b[Z".as_slice())
        );
        let unmodified = KeyInput::new(Key::Tab, KeyPhase::Pressed, Modifiers::empty());
        assert_eq!(
            KeyEncoder::encode(unmodified).as_deref(),
            Ok(b"\t".as_slice())
        );

        // Stage 3 encodes the Ctrl and Alt combinations of Tab: Ctrl keeps
        // the base HT byte even with Shift, and Alt prefixes ESC to the
        // bytes Tab would otherwise emit. Super/Command still drops, and no
        // combination degrades to the bare backtab.
        let ctrl_tab = KeyInput::new(Key::Tab, KeyPhase::Pressed, Modifiers::empty().ctrl());
        assert_eq!(
            KeyEncoder::encode(ctrl_tab).as_deref(),
            Ok(b"\x09".as_slice())
        );
        let ctrl_shift_tab = KeyInput::new(
            Key::Tab,
            KeyPhase::Pressed,
            Modifiers::empty().ctrl().shift(),
        );
        assert_eq!(
            KeyEncoder::encode(ctrl_shift_tab).as_deref(),
            Ok(b"\x09".as_slice())
        );
        let alt_tab = KeyInput::new(Key::Tab, KeyPhase::Pressed, Modifiers::empty().alt());
        assert_eq!(
            KeyEncoder::encode(alt_tab).as_deref(),
            Ok(b"\x1b\x09".as_slice())
        );
        let alt_shift_tab = KeyInput::new(
            Key::Tab,
            KeyPhase::Pressed,
            Modifiers::empty().alt().shift(),
        );
        assert_eq!(
            KeyEncoder::encode(alt_shift_tab).as_deref(),
            Ok(b"\x1b\x1b[Z".as_slice())
        );
        for modifiers in [
            Modifiers::empty().super_key(),
            Modifiers::empty().super_key().shift(),
        ] {
            let input = KeyInput::new(Key::Tab, KeyPhase::Pressed, modifiers);
            assert_eq!(
                KeyEncoder::encode(input),
                Err(KeyDropReason::UnsupportedModifier)
            );
        }
    }

    #[test]
    fn stage_four_tilde_keys_carry_the_modifier_as_second_parameter() {
        let cases = [
            (Key::Delete, 3_u8),
            (Key::Insert, 2),
            (Key::PageUp, 5),
            (Key::PageDown, 6),
        ];
        for (key, parameter) in cases {
            for (modifiers, modifier_parameter) in stage_four_modifier_parameters() {
                let input = KeyInput::new(key, KeyPhase::Pressed, modifiers);
                let expected = format!("\x1b[{parameter};{modifier_parameter}~");
                assert_eq!(
                    KeyEncoder::encode(input).as_deref(),
                    Ok(expected.as_bytes())
                );
            }
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

    /// Stage 2 prefixed `ESC` to every Alt combination; stage 4 keeps the
    /// `ESC` prefix for the named keys without a parameter form (Enter,
    /// Backspace, Tab, Escape) and moves the navigation class onto the
    /// modifier parameter with Alt as bit 2, in the `CSI` form even under
    /// DECCKM.
    #[test]
    fn alt_named_and_navigation_keys_use_esc_prefix_or_modifier_parameter() {
        let application = InputMode::normal().with_cursor(CursorKeyMode::Application);
        let cases = [
            (Key::Enter, b"\x1b\x0d".as_slice()),
            (Key::Backspace, b"\x1b\x7f"),
            (Key::Tab, b"\x1b\x09"),
            (Key::Escape, b"\x1b\x1b"),
            (Key::Arrow(Arrow::Up), b"\x1b[1;3A"),
            (Key::Arrow(Arrow::Left), b"\x1b[1;3D"),
            (Key::Delete, b"\x1b[3;3~"),
            (Key::PageDown, b"\x1b[6;3~"),
            (Key::Function(FunctionKey::F5), b"\x1b[15;3~"),
        ];
        for (key, expected) in cases {
            let input = KeyInput::new(key, KeyPhase::Pressed, Modifiers::empty().alt());
            assert_eq!(KeyEncoder::encode(input).as_deref(), Ok(expected));
        }
        let alt_home = KeyInput::new(Key::Home, KeyPhase::Pressed, Modifiers::empty().alt());
        assert_eq!(
            KeyEncoder::encode_with(alt_home, application).as_deref(),
            Ok(b"\x1b[1;3H".as_slice())
        );
        let alt_up = KeyInput::new(
            Key::Arrow(Arrow::Up),
            KeyPhase::Pressed,
            Modifiers::empty().alt(),
        );
        assert_eq!(
            KeyEncoder::encode_with(alt_up, application).as_deref(),
            Ok(b"\x1b[1;3A".as_slice())
        );
    }

    /// Stage 2 dropped Ctrl and Alt+Ctrl on the navigation class; stage 4
    /// encodes the xterm modifier parameter instead (Ctrl is bit 4, Alt+Ctrl
    /// is 7). The character-class drops survive: a control character still
    /// has no encoding, and a digit still has no control byte.
    #[test]
    fn ctrl_and_alt_ctrl_navigation_keys_encode_modifier_parameters() {
        let cases = [
            (
                Key::Delete,
                b"\x1b[3;5~".as_slice(),
                b"\x1b[3;7~".as_slice(),
            ),
            (Key::Home, b"\x1b[1;5H", b"\x1b[1;7H"),
            (Key::Arrow(Arrow::Up), b"\x1b[1;5A", b"\x1b[1;7A"),
        ];
        for (key, ctrl_bytes, alt_ctrl_bytes) in cases {
            let ctrl = KeyInput::new(key, KeyPhase::Pressed, Modifiers::empty().ctrl());
            assert_eq!(KeyEncoder::encode(ctrl).as_deref(), Ok(ctrl_bytes));
            let alt_ctrl = KeyInput::new(key, KeyPhase::Pressed, Modifiers::empty().alt().ctrl());
            assert_eq!(KeyEncoder::encode(alt_ctrl).as_deref(), Ok(alt_ctrl_bytes));
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

    #[test]
    fn stage_four_modified_function_keys_use_modifier_parameters() {
        // F1-F4 switch from SS3 to CSI 1;<mod> <final> when modified.
        let ss3_cases = [
            (FunctionKey::F1, b"\x1bOP".as_slice(), 'P'),
            (FunctionKey::F2, b"\x1bOQ", 'Q'),
            (FunctionKey::F3, b"\x1bOR", 'R'),
            (FunctionKey::F4, b"\x1bOS", 'S'),
        ];
        for (function_key, unmodified_bytes, final_char) in ss3_cases {
            let unmodified = KeyInput::new(
                Key::Function(function_key),
                KeyPhase::Pressed,
                Modifiers::empty(),
            );
            assert_eq!(
                KeyEncoder::encode(unmodified).as_deref(),
                Ok(unmodified_bytes)
            );
            for (modifiers, parameter) in stage_four_modifier_parameters() {
                let modified =
                    KeyInput::new(Key::Function(function_key), KeyPhase::Pressed, modifiers);
                let expected = format!("\x1b[1;{parameter}{final_char}");
                assert_eq!(
                    KeyEncoder::encode(modified).as_deref(),
                    Ok(expected.as_bytes())
                );
            }
        }
        // F5-F12 keep their xterm parameter and append the modifier.
        let tilde_cases = [
            (FunctionKey::F5, 15_u8),
            (FunctionKey::F6, 17),
            (FunctionKey::F7, 18),
            (FunctionKey::F8, 19),
            (FunctionKey::F9, 20),
            (FunctionKey::F10, 21),
            (FunctionKey::F11, 23),
            (FunctionKey::F12, 24),
        ];
        for (function_key, parameter) in tilde_cases {
            for (modifiers, modifier_parameter) in stage_four_modifier_parameters() {
                let modified =
                    KeyInput::new(Key::Function(function_key), KeyPhase::Pressed, modifiers);
                let expected = format!("\x1b[{parameter};{modifier_parameter}~");
                assert_eq!(
                    KeyEncoder::encode(modified).as_deref(),
                    Ok(expected.as_bytes())
                );
            }
        }
    }

    /// Every unmodified stage-1 encoding stays byte-identical after the
    /// modifier-parameter stage, in both cursor key modes. Ctrl characters
    /// keep their control bytes, and Alt characters take the stage-2 `ESC`
    /// prefix instead of dropping.
    #[test]
    fn stage_four_unmodified_stage_one_bytes_are_unchanged() {
        let application = InputMode::normal().with_cursor(CursorKeyMode::Application);
        let cases = [
            (Key::Character(' '), b" ".as_slice(), b" ".as_slice()),
            (Key::Enter, b"\r", b"\r"),
            (Key::Backspace, b"\x7f", b"\x7f"),
            (Key::Tab, b"\t", b"\t"),
            (Key::Escape, b"\x1b", b"\x1b"),
            (Key::Arrow(Arrow::Up), b"\x1b[A", b"\x1bOA"),
            (Key::Arrow(Arrow::Down), b"\x1b[B", b"\x1bOB"),
            (Key::Arrow(Arrow::Right), b"\x1b[C", b"\x1bOC"),
            (Key::Arrow(Arrow::Left), b"\x1b[D", b"\x1bOD"),
            (Key::Delete, b"\x1b[3~", b"\x1b[3~"),
            (Key::Insert, b"\x1b[2~", b"\x1b[2~"),
            (Key::Home, b"\x1b[H", b"\x1bOH"),
            (Key::End, b"\x1b[F", b"\x1bOF"),
            (Key::PageUp, b"\x1b[5~", b"\x1b[5~"),
            (Key::PageDown, b"\x1b[6~", b"\x1b[6~"),
        ];
        for (key, normal_bytes, application_bytes) in cases {
            let input = KeyInput::new(key, KeyPhase::Pressed, Modifiers::empty());
            assert_eq!(KeyEncoder::encode(input).as_deref(), Ok(normal_bytes));
            assert_eq!(
                KeyEncoder::encode_with(input, application).as_deref(),
                Ok(application_bytes)
            );
        }
        let function_cases = [
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
        for (function_key, expected) in function_cases {
            let input = KeyInput::new(
                Key::Function(function_key),
                KeyPhase::Pressed,
                Modifiers::empty(),
            );
            assert_eq!(KeyEncoder::encode(input).as_deref(), Ok(expected));
            assert_eq!(
                KeyEncoder::encode_with(input, application).as_deref(),
                Ok(expected)
            );
        }
        let ctrl_character = KeyInput::new(
            Key::Character('c'),
            KeyPhase::Pressed,
            Modifiers::empty().ctrl(),
        );
        assert_eq!(KeyEncoder::encode(ctrl_character), Ok(vec![0x03]));
        let alt_character = KeyInput::new(
            Key::Character('f'),
            KeyPhase::Pressed,
            Modifiers::empty().alt(),
        );
        assert_eq!(
            KeyEncoder::encode(alt_character).as_deref(),
            Ok(b"\x1bf".as_slice())
        );
    }

    #[test]
    fn stage_four_releases_still_emit_nothing() {
        let application = InputMode::normal().with_cursor(CursorKeyMode::Application);
        let cases = [
            Key::Arrow(Arrow::Up),
            Key::Arrow(Arrow::Left),
            Key::Tab,
            Key::Delete,
            Key::Insert,
            Key::Home,
            Key::End,
            Key::PageUp,
            Key::PageDown,
            Key::Function(FunctionKey::F1),
            Key::Function(FunctionKey::F12),
        ];
        for key in cases {
            for modifiers in [
                Modifiers::empty().shift(),
                Modifiers::empty().ctrl(),
                Modifiers::empty().alt(),
                Modifiers::empty().shift().alt().ctrl(),
            ] {
                let released = KeyInput::new(key, KeyPhase::Released, modifiers);
                assert_eq!(KeyEncoder::encode(released), Err(KeyDropReason::Released));
                assert_eq!(
                    KeyEncoder::encode_with(released, application),
                    Err(KeyDropReason::Released)
                );
            }
        }
    }

    /// The combined Alt rule, matching xterm: keys that own a modifier
    /// parameter form (arrows, Home, End, Delete, Insert, PageUp, PageDown,
    /// F1-F12) take Alt as bit 2 of the parameter, while plain characters
    /// and the named keys without a parameter form keep Alt as the `ESC`
    /// prefix.
    #[test]
    fn stage_four_alt_splits_between_modifier_parameter_and_esc_prefix() {
        let parameter_form = [
            (Key::Arrow(Arrow::Up), b"\x1b[1;3A".as_slice()),
            (Key::Home, b"\x1b[1;3H"),
            (Key::End, b"\x1b[1;3F"),
            (Key::Delete, b"\x1b[3;3~"),
            (Key::Insert, b"\x1b[2;3~"),
            (Key::PageUp, b"\x1b[5;3~"),
            (Key::PageDown, b"\x1b[6;3~"),
            (Key::Function(FunctionKey::F1), b"\x1b[1;3P"),
            (Key::Function(FunctionKey::F12), b"\x1b[24;3~"),
        ];
        for (key, expected) in parameter_form {
            let input = KeyInput::new(key, KeyPhase::Pressed, Modifiers::empty().alt());
            assert_eq!(KeyEncoder::encode(input).as_deref(), Ok(expected));
        }
        let esc_prefix = [
            (Key::Character('f'), b"\x1bf".as_slice()),
            (Key::Character('é'), b"\x1b\xc3\xa9"),
            (Key::Enter, b"\x1b\x0d"),
            (Key::Backspace, b"\x1b\x7f"),
            (Key::Tab, b"\x1b\x09"),
            (Key::Escape, b"\x1b\x1b"),
        ];
        for (key, expected) in esc_prefix {
            let input = KeyInput::new(key, KeyPhase::Pressed, Modifiers::empty().alt());
            assert_eq!(KeyEncoder::encode(input).as_deref(), Ok(expected));
        }
    }
}
