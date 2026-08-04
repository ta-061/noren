//! Application input modes and bounded keypad key model.
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
/// The set is deliberately bounded to a standard numeric keypad; function,
/// editing, and PF keys remain a later integration concern and are not modeled
/// by this checkpoint.
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
