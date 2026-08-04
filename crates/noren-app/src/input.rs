//! Application input modes and bounded keypad/function key model.
//!
//! This is the input-side mirror of DECCKM (`CSI ?1 h/l`, DEC cursor key mode)
//! and DECKPAM / DECKPNM (`ESC =` / `ESC >`, keypad application / numeric
//! mode). `noren-terminal` owns the parser and mode state; this module owns the
//! app-side encoding selection.

use crate::{Arrow, KeyPhase, Modifiers};

/// DEC cursor key mode (DECCKM) input-side selector.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorKeyMode {
    /// Arrow keys emit the normal cursor sequences (`ESC [ A` ... `ESC [ D`).
    #[default]
    Normal,
    /// Arrow keys emit the application cursor sequences (`ESC O A` ... `ESC O D`).
    Application,
}

/// Keypad mode (DECKPAM / DECKPNM) input-side selector.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeypadMode {
    /// Keypad keys emit their literal numeric / operator characters.
    #[default]
    Numeric,
    /// Keypad keys emit the application `SS3` sequences.
    Application,
}

/// Explicit, immutable application input-mode snapshot consumed by the key
/// encoder.
///
/// The two selectors are independent: a program may set application cursor keys
/// while leaving the keypad numeric, matching how DECCKM and DECKPAM/DECKPNM
/// are distinct private modes. Builders are pure value transforms, so repeated
/// application of the same selector is idempotent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputMode {
    cursor: CursorKeyMode,
    keypad: KeypadMode,
}

impl InputMode {
    /// Normal cursor sequences and numeric keypad sequences (the PoC default).
    #[must_use]
    pub const fn normal() -> Self {
        Self {
            cursor: CursorKeyMode::Normal,
            keypad: KeypadMode::Numeric,
        }
    }

    /// Replace the cursor key mode, leaving the keypad mode unchanged.
    #[must_use]
    pub const fn with_cursor(self, cursor: CursorKeyMode) -> Self {
        Self { cursor, ..self }
    }

    /// Replace the keypad mode, leaving the cursor key mode unchanged.
    #[must_use]
    pub const fn with_keypad(self, keypad: KeypadMode) -> Self {
        Self { keypad, ..self }
    }

    /// Active cursor key mode.
    #[must_use]
    pub const fn cursor(self) -> CursorKeyMode {
        self.cursor
    }

    /// Active keypad mode.
    #[must_use]
    pub const fn keypad(self) -> KeypadMode {
        self.keypad
    }
}

/// Bounded set of numeric-keypad keys the input encoder explicitly supports.
///
/// The set is deliberately bounded to a standard numeric keypad; PF keys
/// remain a later integration concern and are not modeled by this checkpoint.
/// Function keys are modeled separately as [`FunctionKey`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeypadKey {
    /// `0`.
    Zero,
    /// `1`.
    One,
    /// `2`.
    Two,
    /// `3`.
    Three,
    /// `4`.
    Four,
    /// `5`.
    Five,
    /// `6`.
    Six,
    /// `7`.
    Seven,
    /// `8`.
    Eight,
    /// `9`.
    Nine,
    /// Decimal separator (`.`).
    Decimal,
    /// `+`.
    Plus,
    /// `-`.
    Minus,
    /// `*`.
    Star,
    /// `/`.
    Slash,
    /// Keypad Enter.
    Enter,
}

/// An app-owned numeric-keypad key event translated from platform callbacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeypadInput {
    key: KeypadKey,
    phase: KeyPhase,
    modifiers: Modifiers,
}

impl KeypadInput {
    /// Create a keypad key event without active modifiers.
    #[must_use]
    pub const fn new(key: KeypadKey, phase: KeyPhase) -> Self {
        Self {
            key,
            phase,
            modifiers: Modifiers::empty(),
        }
    }

    /// Return this event with the active modifiers captured by the window layer.
    #[must_use]
    pub const fn with_modifiers(mut self, modifiers: Modifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    /// The keypad key identity.
    #[must_use]
    pub const fn key(self) -> KeypadKey {
        self.key
    }

    /// The press phase.
    #[must_use]
    pub const fn phase(self) -> KeyPhase {
        self.phase
    }

    /// Active modifiers captured with this event.
    #[must_use]
    pub const fn modifiers(self) -> Modifiers {
        self.modifiers
    }
}

/// Select the cursor key byte sequence for the active mode.
pub(crate) fn cursor_bytes(arrow: Arrow, mode: CursorKeyMode) -> &'static [u8] {
    let (normal, application) = match arrow {
        Arrow::Up => (b"\x1b[A", b"\x1bOA"),
        Arrow::Down => (b"\x1b[B", b"\x1bOB"),
        Arrow::Right => (b"\x1b[C", b"\x1bOC"),
        Arrow::Left => (b"\x1b[D", b"\x1bOD"),
    };
    match mode {
        CursorKeyMode::Normal => normal,
        CursorKeyMode::Application => application,
    }
}

/// Select the keypad byte sequence for the active mode.
pub(crate) fn keypad_bytes(key: KeypadKey, mode: KeypadMode) -> &'static [u8] {
    let (numeric, application) = match key {
        KeypadKey::Zero => (b"0", b"\x1bOp"),
        KeypadKey::One => (b"1", b"\x1bOq"),
        KeypadKey::Two => (b"2", b"\x1bOr"),
        KeypadKey::Three => (b"3", b"\x1bOs"),
        KeypadKey::Four => (b"4", b"\x1bOt"),
        KeypadKey::Five => (b"5", b"\x1bOu"),
        KeypadKey::Six => (b"6", b"\x1bOv"),
        KeypadKey::Seven => (b"7", b"\x1bOw"),
        KeypadKey::Eight => (b"8", b"\x1bOx"),
        KeypadKey::Nine => (b"9", b"\x1bOy"),
        KeypadKey::Decimal => (b".", b"\x1bOn"),
        KeypadKey::Plus => (b"+", b"\x1bOk"),
        KeypadKey::Minus => (b"-", b"\x1bOm"),
        KeypadKey::Star => (b"*", b"\x1bOj"),
        KeypadKey::Slash => (b"/", b"\x1bOo"),
        KeypadKey::Enter => (b"\r", b"\x1bOM"),
    };
    match mode {
        KeypadMode::Numeric => numeric,
        KeypadMode::Application => application,
    }
}

/// Bounded set of function keys the input encoder explicitly supports.
///
/// F1 through F4 emit `SS3` sequences while F5 through F12 emit `CSI <n> ~`
/// sequences, matching xterm's asymmetry. Function keys ignore both
/// application cursor and application keypad modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FunctionKey {
    /// `F1`.
    F1,
    /// `F2`.
    F2,
    /// `F3`.
    F3,
    /// `F4`.
    F4,
    /// `F5`.
    F5,
    /// `F6`.
    F6,
    /// `F7`.
    F7,
    /// `F8`.
    F8,
    /// `F9`.
    F9,
    /// `F10`.
    F10,
    /// `F11`.
    F11,
    /// `F12`.
    F12,
}

/// Select the function key byte sequence.
///
/// F1-F4 use the `SS3` finals `P`-`S`; F5-F12 use `CSI <n> ~` with xterm's
/// parameter gaps at 16 and 22. DECCKM does not change any of these.
pub(crate) fn function_key_bytes(key: FunctionKey) -> &'static [u8] {
    match key {
        FunctionKey::F1 => b"\x1bOP",
        FunctionKey::F2 => b"\x1bOQ",
        FunctionKey::F3 => b"\x1bOR",
        FunctionKey::F4 => b"\x1bOS",
        FunctionKey::F5 => b"\x1b[15~",
        FunctionKey::F6 => b"\x1b[17~",
        FunctionKey::F7 => b"\x1b[18~",
        FunctionKey::F8 => b"\x1b[19~",
        FunctionKey::F9 => b"\x1b[20~",
        FunctionKey::F10 => b"\x1b[21~",
        FunctionKey::F11 => b"\x1b[23~",
        FunctionKey::F12 => b"\x1b[24~",
    }
}

/// Select the Home byte sequence for the active cursor key mode.
///
/// xterm sends `CSI H` in normal mode and `SS3 H` under DECCKM, matching the
/// terminfo `khome=\EOH` capability that applies between `smkx` and `rmkx`.
pub(crate) fn home_bytes(mode: CursorKeyMode) -> &'static [u8] {
    match mode {
        CursorKeyMode::Normal => b"\x1b[H",
        CursorKeyMode::Application => b"\x1bOH",
    }
}

/// Select the End byte sequence for the active cursor key mode.
///
/// xterm sends `CSI F` in normal mode and `SS3 F` under DECCKM
/// (`kend=\EOF` between `smkx` and `rmkx`).
pub(crate) fn end_bytes(mode: CursorKeyMode) -> &'static [u8] {
    match mode {
        CursorKeyMode::Normal => b"\x1b[F",
        CursorKeyMode::Application => b"\x1bOF",
    }
}

/// Final byte of the bare cursor key sequence for an arrow.
pub(crate) fn cursor_final_byte(arrow: Arrow) -> u8 {
    match arrow {
        Arrow::Up => b'A',
        Arrow::Down => b'B',
        Arrow::Right => b'C',
        Arrow::Left => b'D',
    }
}

/// The xterm modifier parameter for the active modifiers.
///
/// xterm encodes modifiers on function and cursor keys as
/// `1 + shift + 2 * alt + 4 * ctrl + 8 * meta`, so a bare Shift gives 2,
/// Alt gives 3, Ctrl gives 5, and Shift+Alt+Ctrl gives 8. The app drops
/// Super/Command before consulting this value, and Meta is not tracked in
/// the PoC, so the result is bounded to `1..=8`; `1` means no modifier is
/// active and the key keeps its bare stage-1 sequence.
pub(crate) fn modifier_parameter(modifiers: Modifiers) -> u8 {
    let mut parameter = 1;
    if modifiers.is_shift() {
        parameter += 1;
    }
    if modifiers.is_alt() {
        parameter += 2;
    }
    if modifiers.is_ctrl() {
        parameter += 4;
    }
    parameter
}

/// Encode the modified `CSI 1 ; <mod> <final>` form shared by cursor-class
/// keys (arrows, Home, End, F1-F4) when any modifier is held.
///
/// xterm emits this `CSI` form even under DECCKM: the modifier parameter
/// suppresses the `SS3` application cursor form.
pub(crate) fn modified_final_bytes(final_byte: u8, modifier_parameter: u8) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(6);
    bytes.extend_from_slice(b"\x1b[1;");
    append_decimal(&mut bytes, modifier_parameter);
    bytes.push(final_byte);
    bytes
}

/// Encode the modified `CSI <n> ; <mod> ~` form used by the tilde-style
/// keys (Delete, Insert, PageUp, PageDown, F5-F12) when any modifier is
/// held; `parameter` is the key's bare `CSI <n> ~` parameter.
pub(crate) fn modified_tilde_bytes(parameter: u8, modifier_parameter: u8) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(7);
    bytes.push(0x1b);
    bytes.push(b'[');
    append_decimal(&mut bytes, parameter);
    bytes.push(b';');
    append_decimal(&mut bytes, modifier_parameter);
    bytes.push(b'~');
    bytes
}

/// Select the modified byte sequence for a function key.
///
/// F1-F4 switch from `SS3 P`-`SS3 S` to `CSI 1 ; <mod> P`-`CSI 1 ; <mod> S`
/// when a modifier is held; F5-F12 keep their xterm parameters
/// (15, 17-21, 23-24) and append the modifier as the second parameter.
pub(crate) fn modified_function_key_bytes(key: FunctionKey, modifier_parameter: u8) -> Vec<u8> {
    match key {
        FunctionKey::F1 => modified_final_bytes(b'P', modifier_parameter),
        FunctionKey::F2 => modified_final_bytes(b'Q', modifier_parameter),
        FunctionKey::F3 => modified_final_bytes(b'R', modifier_parameter),
        FunctionKey::F4 => modified_final_bytes(b'S', modifier_parameter),
        FunctionKey::F5 => modified_tilde_bytes(15, modifier_parameter),
        FunctionKey::F6 => modified_tilde_bytes(17, modifier_parameter),
        FunctionKey::F7 => modified_tilde_bytes(18, modifier_parameter),
        FunctionKey::F8 => modified_tilde_bytes(19, modifier_parameter),
        FunctionKey::F9 => modified_tilde_bytes(20, modifier_parameter),
        FunctionKey::F10 => modified_tilde_bytes(21, modifier_parameter),
        FunctionKey::F11 => modified_tilde_bytes(23, modifier_parameter),
        FunctionKey::F12 => modified_tilde_bytes(24, modifier_parameter),
    }
}

fn append_decimal(bytes: &mut Vec<u8>, value: u8) {
    if value >= 10 {
        bytes.push(b'0' + value / 10);
    }
    bytes.push(b'0' + value % 10);
}
