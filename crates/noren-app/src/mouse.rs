//! Mouse input encoding: turn pointer events into xterm mouse report bytes
//! written to PTY *input*.
//!
//! # Direction (read this before treating any `CSI M` handling as a parser bug)
//!
//! `TerminalState::feed_bytes` parses PTY *output*. A parameterless `CSI M` in
//! output is the valid Delete-Line command. xterm mouse reports share the
//! `CSI M` prefix but travel the opposite direction: the terminal *generates*
//! them and writes them to PTY *input*. They never reach the output parser.
//! This module lives on the input side and never touches output-side parsing.
//! (Issue #46 and PR #52 were closed for exactly this confusion.)
//!
//! # Mode model
//!
//! Tracking modes decide *whether* a report is produced:
//! - **1000** — normal mouse tracking: report press, release, and wheel.
//! - **1002** — button-event tracking: also report motion while a button is held.
//! - **1003** — any-event tracking: report all motion, with or without a button.
//!
//! Encoding modes decide *how* a report is formatted. The precedence is
//! fixed so the caller cannot pick a broken combination:
//! - **1006** — SGR (`CSI < Cb ; Cx ; Cy M` press/wheel/motion, `m` release).
//!   Preferred when enabled because it alone distinguishes the released button.
//! - **1015** — urxvt (`CSI Cb ; Cx ; Cy M`, decimal, no angle bracket, release
//!   collapses to `Cb = 3`).
//! - **1005** — UTF-8 coordinate extension. Recognized and tracked so
//!   DECSET/DECRST state stays correct, but its UTF-8 byte extension is not
//!   implemented in this slice; with neither 1006 nor 1015 active the encoder
//!   falls through to the X10 byte form below.
//! - neither 1006 nor 1015 active — X10 byte form (`CSI M` plus three bytes
//!   each offset by 32). This is the legacy form that cannot represent a
//!   coordinate above 223.
//!
//! With **no tracking mode enabled, nothing is emitted at all.**
//!
//! # Coordinate rules
//!
//! Coordinates are 1-based and clamped to the grid. The encoder takes 0-based
//! cell indices from the window layer, clamps them to `[0, cols-1]` and
//! `[0, rows-1]`, then emits `col + 1` and `row + 1`. An out-of-range
//! coordinate is never emitted.
//!
//! The X10 byte form cannot represent a coordinate above 223 (the offset byte
//! saturates at 255). The rule chosen for this slice: **when the X10 byte
//! form is active and either 1-based coordinate exceeds 223, the report is
//! dropped (nothing emitted) rather than silently misreporting the position.**
//! SGR and urxvt have no such limit and are unaffected, so on a wide grid the
//! preferred SGR encoding never hits this rule.
//!
//! This module is deliberately self-contained: it declares no `crate::`
//! items, so it can be compiled by the test target via `#[path]` before the
//! export-wiring commit adds `pub mod mouse;` to `lib.rs`.

/// DEC private mode numbers for mouse tracking and encoding.
///
/// These are the values carried by `CSI ? <mode> h` (DECSET) and
/// `CSI ? <mode> l` (DECRST). Zellij's client enables all of these on attach.
pub const MODE_NORMAL: u16 = 1000;
pub const MODE_BUTTON_EVENT: u16 = 1002;
pub const MODE_ANY_EVENT: u16 = 1003;
pub const MODE_UTF8: u16 = 1005;
pub const MODE_SGR: u16 = 1006;
pub const MODE_URXVT: u16 = 1015;

/// Highest 1-based coordinate the X10 byte form can carry (255 - 32).
pub const X10_MAX_COORD: u32 = 223;

const CB_BUTTON1: u32 = 0;
const CB_BUTTON2: u32 = 1;
const CB_BUTTON3: u32 = 2;
/// Generic "no button" code: legacy/X10/urxvt release, and motion with no
/// button held under any-event (1003) tracking.
const CB_RELEASE: u32 = 3;
/// Base of wheel events: wheel up is `64`, wheel down is `65`.
const CB_WHEEL: u32 = 64;
/// Bit added to `Cb` for motion (drag/hover) reports.
const CB_MOTION: u32 = 32;
const CB_SHIFT: u32 = 4;
const CB_ALT: u32 = 8;
const CB_CTRL: u32 = 16;

/// Active mouse modes set by the application through DECSET/DECRST.
///
/// Tracking flags (1000/1002/1003) gate *whether* a report is produced;
/// encoding flags (1005/1006/1015) gate *how* it is formatted. The two
/// groups are independent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseModes {
    normal: bool,
    button_event: bool,
    any_event: bool,
    utf8: bool,
    sgr: bool,
    urxvt: bool,
}

impl MouseModes {
    /// All modes off (nothing is emitted). Equal to [`Default`].
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            normal: false,
            button_event: false,
            any_event: false,
            utf8: false,
            sgr: false,
            urxvt: false,
        }
    }

    /// Set normal mouse tracking (mode 1000): press/release/wheel.
    #[must_use]
    pub const fn with_normal(mut self, on: bool) -> Self {
        self.normal = on;
        self
    }

    /// Set button-event tracking (mode 1002): adds motion while a button is held.
    #[must_use]
    pub const fn with_button_event(mut self, on: bool) -> Self {
        self.button_event = on;
        self
    }

    /// Set any-event tracking (mode 1003): reports all motion, button or not.
    #[must_use]
    pub const fn with_any_event(mut self, on: bool) -> Self {
        self.any_event = on;
        self
    }

    /// Set UTF-8 encoding (mode 1005). Tracked; its byte extension is not
    /// implemented, so it falls through to the X10 byte form.
    #[must_use]
    pub const fn with_utf8(mut self, on: bool) -> Self {
        self.utf8 = on;
        self
    }

    /// Set SGR encoding (mode 1006). Preferred when enabled.
    #[must_use]
    pub const fn with_sgr(mut self, on: bool) -> Self {
        self.sgr = on;
        self
    }

    /// Set urxvt encoding (mode 1015).
    #[must_use]
    pub const fn with_urxvt(mut self, on: bool) -> Self {
        self.urxvt = on;
        self
    }

    /// Apply a DECSET/DECRST transition by mode number. Unrecognized mode
    /// numbers are left for their own handlers and return `self` unchanged.
    ///
    /// This is the entry point the future terminal-mode wiring will call when
    /// it observes `CSI ? <mode> h/l`; the named builders exist for tests and
    /// for code that already knows which flag it is setting.
    #[must_use]
    pub fn set(self, mode: u16, on: bool) -> Self {
        match mode {
            MODE_NORMAL => self.with_normal(on),
            MODE_BUTTON_EVENT => self.with_button_event(on),
            MODE_ANY_EVENT => self.with_any_event(on),
            MODE_UTF8 => self.with_utf8(on),
            MODE_SGR => self.with_sgr(on),
            MODE_URXVT => self.with_urxvt(on),
            _ => self,
        }
    }

    /// Whether any tracking mode is on, i.e. whether the encoder may emit at
    /// all. With this false every event produces `None`.
    #[must_use]
    pub const fn is_tracked(self) -> bool {
        self.normal || self.button_event || self.any_event
    }

    /// Whether motion events may be reported (button-event or any-event).
    const fn is_motion_tracked(self) -> bool {
        self.button_event || self.any_event
    }

    /// Whether any-event (1003) tracking is on, i.e. whether no-button motion
    /// (hover) is reported.
    const fn is_any_event(self) -> bool {
        self.any_event
    }

    /// Whether SGR (1006) is the active encoding.
    const fn is_sgr(self) -> bool {
        self.sgr
    }

    /// Whether urxvt (1015) is the active encoding (only meaningful when SGR
    /// is off, since SGR takes precedence).
    const fn is_urxvt(self) -> bool {
        self.urxvt
    }
}

/// Mouse buttons this encoder can report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

/// Scroll-wheel direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelDirection {
    Up,
    Down,
}

/// Pointer-event kind.
///
/// `Move` carries the currently-held button, if any, because the report's `Cb`
/// differs for a drag (button + motion flag) versus a no-button hover (the
/// generic release code plus the motion flag).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerKind {
    /// Button press.
    Press(MouseButton),
    /// Button release.
    Release(MouseButton),
    /// Pointer motion. `button` is the held button for a drag, or `None` for a
    /// hover; the latter only reports under any-event (1003) tracking.
    Move { button: Option<MouseButton> },
    /// Scroll wheel click.
    Wheel(WheelDirection),
}

/// Pointer-event modifiers that fold into the `Cb` parameter.
///
/// Super/Command is not modeled: the window layer drops it before reaching
/// the encoder, matching the key encoder's policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PointerModifiers {
    shift: bool,
    alt: bool,
    ctrl: bool,
}

impl PointerModifiers {
    /// No modifiers.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            shift: false,
            alt: false,
            ctrl: false,
        }
    }

    /// Set Shift.
    #[must_use]
    pub const fn shift(mut self) -> Self {
        self.shift = true;
        self
    }

    /// Set Alt/Option.
    #[must_use]
    pub const fn alt(mut self) -> Self {
        self.alt = true;
        self
    }

    /// Set Control.
    #[must_use]
    pub const fn ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }
}

/// Grid bounds used for coordinate clamping.
///
/// Both dimensions are non-zero: a zero edge is rejected at construction so the
/// clamp range is always well defined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseGrid {
    cols: u16,
    rows: u16,
}

impl MouseGrid {
    /// Create a grid from its column and row counts. Returns `None` when
    /// either dimension is zero.
    #[must_use]
    pub const fn new(cols: u16, rows: u16) -> Option<Self> {
        if cols == 0 || rows == 0 {
            None
        } else {
            Some(Self { cols, rows })
        }
    }

    /// Column count.
    #[must_use]
    pub const fn cols(self) -> u16 {
        self.cols
    }

    /// Row count.
    #[must_use]
    pub const fn rows(self) -> u16 {
        self.rows
    }
}

/// An app-owned pointer event translated from platform callbacks.
///
/// `col` and `row` are 0-based cell indices relative to the terminal grid's
/// top-left corner; the encoder clamps them to the grid and converts to the
/// 1-based coordinates the mouse protocols carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointerEvent {
    kind: PointerKind,
    col: u32,
    row: u32,
    modifiers: PointerModifiers,
}

impl PointerEvent {
    /// Create a pointer event.
    #[must_use]
    pub const fn new(kind: PointerKind, col: u32, row: u32, modifiers: PointerModifiers) -> Self {
        Self {
            kind,
            col,
            row,
            modifiers,
        }
    }

    /// Button press at the given 0-based cell.
    #[must_use]
    pub const fn press(
        button: MouseButton,
        col: u32,
        row: u32,
        modifiers: PointerModifiers,
    ) -> Self {
        Self::new(PointerKind::Press(button), col, row, modifiers)
    }

    /// Button release at the given 0-based cell.
    #[must_use]
    pub const fn release(
        button: MouseButton,
        col: u32,
        row: u32,
        modifiers: PointerModifiers,
    ) -> Self {
        Self::new(PointerKind::Release(button), col, row, modifiers)
    }

    /// Pointer motion at the given 0-based cell. `button` is the held button
    /// for a drag, or `None` for a hover.
    #[must_use]
    pub const fn move_to(
        button: Option<MouseButton>,
        col: u32,
        row: u32,
        modifiers: PointerModifiers,
    ) -> Self {
        Self::new(PointerKind::Move { button }, col, row, modifiers)
    }

    /// Scroll wheel at the given 0-based cell.
    #[must_use]
    pub const fn wheel(
        direction: WheelDirection,
        col: u32,
        row: u32,
        modifiers: PointerModifiers,
    ) -> Self {
        Self::new(PointerKind::Wheel(direction), col, row, modifiers)
    }
}

/// Pure encoder from pointer events to xterm mouse report bytes.
///
/// The encoder is stateless: callers track [`MouseModes`] (observing the
/// application's DECSET/DECRST transitions) and [`MouseGrid`] (from resize)
/// and pass them with each event. A return of `None` means "emit nothing" —
/// either no tracking mode is on, the event kind is not reported under the
/// active tracking mode, or the active byte form cannot represent the
/// coordinate (see the X10 223 rule in the module docs).
pub struct MouseEncoder;

impl MouseEncoder {
    /// Encode one pointer event.
    ///
    /// Returns the report bytes to write to PTY input, or `None` when nothing
    /// should be emitted.
    #[must_use]
    pub fn encode(event: PointerEvent, modes: MouseModes, grid: MouseGrid) -> Option<Vec<u8>> {
        if !modes.is_tracked() {
            return None;
        }

        let modifiers = modifier_bits(event.modifiers);
        let (cx, cy) = clamp_to_grid(event.col, event.row, grid);

        // Resolve `Cb` and whether this is an SGR release. Motion is gated by
        // the tracking mode: never under plain 1000, only with a held button
        // under 1002, always under 1003.
        let (cb, is_release) = match event.kind {
            PointerKind::Press(button) => (button_code(button) | modifiers, false),
            PointerKind::Wheel(direction) => (wheel_code(direction) | modifiers, false),
            PointerKind::Release(button) => {
                // SGR is the only form that distinguishes the released button
                // (it keeps the button code and switches the terminator to 'm').
                // Legacy X10 and urxvt collapse every release to Cb = 3.
                if modes.is_sgr() {
                    (button_code(button) | modifiers, true)
                } else {
                    (CB_RELEASE | modifiers, false)
                }
            }
            PointerKind::Move { button } => {
                if !modes.is_motion_tracked() {
                    return None;
                }
                // Button-event tracking (1002) reports motion only while a
                // button is held; any-event tracking (1003) reports it either
                // way. `is_any_event` is false under plain 1002.
                if !modes.is_any_event() && button.is_none() {
                    return None;
                }
                let base = button.map(button_code).unwrap_or(CB_RELEASE);
                (base | CB_MOTION | modifiers, false)
            }
        };

        if modes.is_sgr() {
            Some(sgr_bytes(cb, cx, cy, is_release))
        } else if modes.is_urxvt() {
            Some(urxvt_bytes(cb, cx, cy))
        } else {
            // X10 byte form: drop when a coordinate cannot be represented.
            if cx > X10_MAX_COORD || cy > X10_MAX_COORD {
                return None;
            }
            Some(x10_bytes(cb, cx, cy))
        }
    }
}

fn button_code(button: MouseButton) -> u32 {
    match button {
        MouseButton::Left => CB_BUTTON1,
        MouseButton::Middle => CB_BUTTON2,
        MouseButton::Right => CB_BUTTON3,
    }
}

fn wheel_code(direction: WheelDirection) -> u32 {
    match direction {
        WheelDirection::Up => CB_WHEEL,
        WheelDirection::Down => CB_WHEEL + 1,
    }
}

/// Fold Shift/Alt/Ctrl into the xterm modifier bits of `Cb`
/// (4 / 8 / 16 respectively).
fn modifier_bits(modifiers: PointerModifiers) -> u32 {
    let mut bits = 0;
    if modifiers.shift {
        bits |= CB_SHIFT;
    }
    if modifiers.alt {
        bits |= CB_ALT;
    }
    if modifiers.ctrl {
        bits |= CB_CTRL;
    }
    bits
}

/// Clamp a 0-based cell index to the grid and convert to 1-based coordinates.
fn clamp_to_grid(col: u32, row: u32, grid: MouseGrid) -> (u32, u32) {
    let max_col = u32::from(grid.cols - 1);
    let max_row = u32::from(grid.rows - 1);
    (col.clamp(0, max_col) + 1, row.clamp(0, max_row) + 1)
}

/// SGR (1006): `CSI < Cb ; Cx ; Cy M` for press/wheel/motion, `m` for release.
fn sgr_bytes(cb: u32, cx: u32, cy: u32, is_release: bool) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(16);
    bytes.extend_from_slice(b"\x1b[<");
    push_decimal(&mut bytes, cb);
    bytes.push(b';');
    push_decimal(&mut bytes, cx);
    bytes.push(b';');
    push_decimal(&mut bytes, cy);
    bytes.push(if is_release { b'm' } else { b'M' });
    bytes
}

/// urxvt (1015): `CSI Cb ; Cx ; Cy M`, decimal, no angle bracket.
fn urxvt_bytes(cb: u32, cx: u32, cy: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(16);
    bytes.push(0x1b);
    bytes.push(b'[');
    push_decimal(&mut bytes, cb);
    bytes.push(b';');
    push_decimal(&mut bytes, cx);
    bytes.push(b';');
    push_decimal(&mut bytes, cy);
    bytes.push(b'M');
    bytes
}

/// X10 legacy byte form: `CSI M` followed by `(Cb+32) (Cx+32) (Cy+32)`.
///
/// Callers must guarantee `cx <= 223` and `cy <= 223` and that `cb` fits in a
/// byte (it always does: the largest value is wheel down plus every modifier,
/// `65 + 28 == 93`).
fn x10_bytes(cb: u32, cx: u32, cy: u32) -> Vec<u8> {
    debug_assert!(cb + 32 <= u32::from(u8::MAX));
    debug_assert!(cx <= X10_MAX_COORD && cy <= X10_MAX_COORD);
    let mut bytes = Vec::with_capacity(6);
    bytes.extend_from_slice(b"\x1b[M");
    bytes.push(offset_byte(cb));
    bytes.push(offset_byte(cx));
    bytes.push(offset_byte(cy));
    bytes
}

/// A value plus the xterm 32 offset, as a single report byte.
fn offset_byte(value: u32) -> u8 {
    (value + 32) as u8
}

/// Append a non-negative decimal integer to `bytes` without allocating.
fn push_decimal(bytes: &mut Vec<u8>, value: u32) {
    if value == 0 {
        bytes.push(b'0');
        return;
    }
    let mut buffer = [0u8; 10];
    let mut cursor = buffer.len();
    let mut remaining = value;
    while remaining > 0 {
        cursor -= 1;
        buffer[cursor] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
    }
    bytes.extend_from_slice(&buffer[cursor..]);
}
