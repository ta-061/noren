//! Pure adapters from window-system input into Noren's input vocabulary.

use noren_app::{
    Arrow, CellMetrics, FunctionKey, InputMode, Key, KeyDropReason, KeyEncoder, KeyInput, KeyPhase,
    KeypadInput, KeypadKey, Modifiers,
    config::KeymapConfig,
    mouse::{MouseButton as EncoderButton, WheelDirection},
    palette::CommandId,
    passthrough::{Chord, KeyCode as GateKeyCode, Modifiers as GateModifiers},
};
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta};
use winit::keyboard::{Key as WinitKey, KeyCode, NamedKey, PhysicalKey};

/// Super+D press toggles diagnostics. Super chords are dropped by the key
/// encoder anyway, so this intercept consumes no terminal input.
pub(super) fn diagnostics_chord_pressed(
    logical_key: &WinitKey,
    state: ElementState,
    repeat: bool,
    modifiers: Modifiers,
) -> bool {
    state == ElementState::Pressed
        && !repeat
        && modifiers.is_super()
        && matches!(logical_key,
            WinitKey::Character(text) if text.eq_ignore_ascii_case("d"))
}

/// Map a winit key event and app modifiers to a pass-through chord.
///
/// Returns `None` for keys that cannot be normalized into a [`Chord`]
/// (whitespace characters, dead keys, multi-codepoint IME sequences). Such
/// keys bypass the gate and follow the normal encode-and-send path.
pub(super) fn chord_from_event(event: &KeyEvent, modifiers: Modifiers) -> Option<Chord> {
    chord_from_logical(&event.logical_key, modifiers)
}

/// Map a logical key and app modifiers to a pass-through chord.
///
/// The window-free core of [`chord_from_event`], shared by the pass-through
/// gate and the open palette's command-chord matching.
pub(super) fn chord_from_logical(key: &WinitKey, modifiers: Modifiers) -> Option<Chord> {
    let code = winit_to_gate_key(key)?;
    let gate_mods = gate_modifiers(modifiers);
    Chord::new(code, gate_mods).ok()
}

/// Resolve a pressed key inside the open palette to its command.
///
/// An exact chord match honors configured modifiers. A modifier-free
/// character binding additionally matches by logical character regardless of
/// held modifiers, preserving the palette's pre-configuration behavior for
/// the default `c`/`s`/`x`/`f` bindings; `bare` is `None` for non-character
/// keys, which only ever match exactly.
pub(super) fn palette_command_for(
    keys: &KeymapConfig,
    pressed: Option<Chord>,
    bare: Option<GateKeyCode>,
) -> Option<CommandId> {
    let bindings = [
        (keys.session_create(), CommandId::SESSION_CREATE),
        (keys.session_select(), CommandId::SESSION_SELECT),
        (keys.session_close(), CommandId::SESSION_CLOSE),
        (keys.sidebar_focus(), CommandId::SIDEBAR_FOCUS),
    ];
    for (binding, id) in bindings {
        if pressed == Some(binding) {
            return Some(id);
        }
        if binding.modifiers() == GateModifiers::empty()
            && bare.is_some_and(|bare| binding.code() == bare)
        {
            return Some(id);
        }
    }
    None
}

/// Encode a pass-through chord into PTY bytes for replay.
///
/// Used when a held leader prefix is replayed after a mismatch. The encoding
/// mirrors what [`KeyEncoder::encode_with`] would produce for the equivalent
/// key event. Returns `None` for chords that the encoder would drop (e.g.
/// Super-modified chords, which produce no PTY bytes).
pub(super) fn encode_chord(chord: &Chord, mode: InputMode) -> Option<Vec<u8>> {
    let key = gate_key_to_app(chord.code())?;
    let mods = app_modifiers_from_gate(chord.modifiers());
    let input = KeyInput::new(key, KeyPhase::Pressed, mods);
    KeyEncoder::encode_with(input, mode).ok()
}

pub(super) fn winit_to_gate_key(key: &WinitKey) -> Option<GateKeyCode> {
    match key {
        WinitKey::Character(text) => {
            let ch = text.chars().next()?;
            if text.chars().count() > 1 {
                return None;
            }
            Some(GateKeyCode::Char(ch))
        }
        WinitKey::Named(NamedKey::Escape) => Some(GateKeyCode::Escape),
        WinitKey::Named(NamedKey::Enter) => Some(GateKeyCode::Enter),
        WinitKey::Named(NamedKey::Tab) => Some(GateKeyCode::Tab),
        WinitKey::Named(NamedKey::Backspace) => Some(GateKeyCode::Backspace),
        WinitKey::Named(NamedKey::Space) => Some(GateKeyCode::Space),
        WinitKey::Named(NamedKey::ArrowUp) => Some(GateKeyCode::Up),
        WinitKey::Named(NamedKey::ArrowDown) => Some(GateKeyCode::Down),
        WinitKey::Named(NamedKey::ArrowLeft) => Some(GateKeyCode::Left),
        WinitKey::Named(NamedKey::ArrowRight) => Some(GateKeyCode::Right),
        WinitKey::Named(NamedKey::Home) => Some(GateKeyCode::Home),
        WinitKey::Named(NamedKey::End) => Some(GateKeyCode::End),
        WinitKey::Named(NamedKey::PageUp) => Some(GateKeyCode::PageUp),
        WinitKey::Named(NamedKey::PageDown) => Some(GateKeyCode::PageDown),
        WinitKey::Named(NamedKey::Delete) => Some(GateKeyCode::Delete),
        WinitKey::Named(NamedKey::Insert) => Some(GateKeyCode::Insert),
        // Function keys F1–F24 round out the chord vocabulary the keymap
        // parser accepts (`f1`–`f24`); higher F-keys stay unmapped.
        WinitKey::Named(NamedKey::F1) => Some(GateKeyCode::Function(1)),
        WinitKey::Named(NamedKey::F2) => Some(GateKeyCode::Function(2)),
        WinitKey::Named(NamedKey::F3) => Some(GateKeyCode::Function(3)),
        WinitKey::Named(NamedKey::F4) => Some(GateKeyCode::Function(4)),
        WinitKey::Named(NamedKey::F5) => Some(GateKeyCode::Function(5)),
        WinitKey::Named(NamedKey::F6) => Some(GateKeyCode::Function(6)),
        WinitKey::Named(NamedKey::F7) => Some(GateKeyCode::Function(7)),
        WinitKey::Named(NamedKey::F8) => Some(GateKeyCode::Function(8)),
        WinitKey::Named(NamedKey::F9) => Some(GateKeyCode::Function(9)),
        WinitKey::Named(NamedKey::F10) => Some(GateKeyCode::Function(10)),
        WinitKey::Named(NamedKey::F11) => Some(GateKeyCode::Function(11)),
        WinitKey::Named(NamedKey::F12) => Some(GateKeyCode::Function(12)),
        WinitKey::Named(NamedKey::F13) => Some(GateKeyCode::Function(13)),
        WinitKey::Named(NamedKey::F14) => Some(GateKeyCode::Function(14)),
        WinitKey::Named(NamedKey::F15) => Some(GateKeyCode::Function(15)),
        WinitKey::Named(NamedKey::F16) => Some(GateKeyCode::Function(16)),
        WinitKey::Named(NamedKey::F17) => Some(GateKeyCode::Function(17)),
        WinitKey::Named(NamedKey::F18) => Some(GateKeyCode::Function(18)),
        WinitKey::Named(NamedKey::F19) => Some(GateKeyCode::Function(19)),
        WinitKey::Named(NamedKey::F20) => Some(GateKeyCode::Function(20)),
        WinitKey::Named(NamedKey::F21) => Some(GateKeyCode::Function(21)),
        WinitKey::Named(NamedKey::F22) => Some(GateKeyCode::Function(22)),
        WinitKey::Named(NamedKey::F23) => Some(GateKeyCode::Function(23)),
        WinitKey::Named(NamedKey::F24) => Some(GateKeyCode::Function(24)),
        _ => None,
    }
}

pub(super) fn gate_key_to_app(code: GateKeyCode) -> Option<Key> {
    match code {
        GateKeyCode::Char(ch) => Some(Key::Character(ch)),
        GateKeyCode::Enter => Some(Key::Enter),
        GateKeyCode::Tab => Some(Key::Tab),
        GateKeyCode::Backspace => Some(Key::Backspace),
        GateKeyCode::Escape => Some(Key::Escape),
        GateKeyCode::Space => Some(Key::Character(' ')),
        GateKeyCode::Up => Some(Key::Arrow(Arrow::Up)),
        GateKeyCode::Down => Some(Key::Arrow(Arrow::Down)),
        GateKeyCode::Left => Some(Key::Arrow(Arrow::Left)),
        GateKeyCode::Right => Some(Key::Arrow(Arrow::Right)),
        GateKeyCode::Home => Some(Key::Home),
        GateKeyCode::End => Some(Key::End),
        GateKeyCode::PageUp => Some(Key::PageUp),
        GateKeyCode::PageDown => Some(Key::PageDown),
        GateKeyCode::Delete => Some(Key::Delete),
        GateKeyCode::Insert => Some(Key::Insert),
        GateKeyCode::Function(_) => None,
    }
}

pub(super) fn gate_modifiers(mods: Modifiers) -> GateModifiers {
    let mut gate = GateModifiers::empty();
    if mods.is_ctrl() {
        gate = gate.ctrl();
    }
    if mods.is_alt() {
        gate = gate.alt();
    }
    if mods.is_shift() {
        gate = gate.shift();
    }
    if mods.is_super() {
        gate = gate.super_key();
    }
    gate
}

pub(super) fn app_modifiers_from_gate(mods: GateModifiers) -> Modifiers {
    let mut app = Modifiers::empty();
    if mods.is_ctrl() {
        app = app.ctrl();
    }
    if mods.is_alt() {
        app = app.alt();
    }
    if mods.is_shift() {
        app = app.shift();
    }
    if mods.is_super() {
        app = app.super_key();
    }
    app
}

/// Map a winit mouse button to the encoder's button type. `Back`, `Forward`,
/// and `Other` are not reportable and return `None`.
pub(super) fn encode_button(button: MouseButton) -> Option<EncoderButton> {
    match button {
        MouseButton::Left => Some(EncoderButton::Left),
        MouseButton::Middle => Some(EncoderButton::Middle),
        MouseButton::Right => Some(EncoderButton::Right),
        MouseButton::Back | MouseButton::Forward | MouseButton::Other(_) => None,
    }
}

/// Convert a winit scroll delta to a sequence of wheel directions (one per
/// line scrolled).
///
/// Both `LineDelta` and `PixelDelta` share the same vertical sign convention.
/// From the winit 0.30 source (`event.rs`, `MouseScrollDelta`):
///
///   LineDelta:   "Positive values indicate that the content that is being
///                 scrolled should move right and down (revealing more content
///                 left and up)."
///   PixelDelta:  "Positive values indicate that the content being scrolled
///                 should move right/down."
///
/// Positive y therefore means the user scrolled **up** (content moves down,
/// revealing earlier content). xterm sends button 4 (`Cb=64`,
/// `WheelDirection::Up`) for scroll-up; negative y is scroll-down (`Cb=65`).
///
/// A non-zero delta that rounds to zero lines still produces one click so a
/// single-notch wheel is never lost.
///
/// `metrics` carries the configured cell height — the same runtime
/// [`CellMetrics`] the renderer and the click-to-grid mappers read — so a
/// `PixelDelta` is converted to lines at the configured stride. Dividing by a
/// compile-time constant instead would convert at the PoC height regardless of
/// `[font] cell_height`, halving the line count at the default and doubling it
/// wherever the height is raised.
pub(super) fn wheel_clicks(delta: MouseScrollDelta, metrics: CellMetrics) -> Vec<WheelDirection> {
    let lines = match delta {
        MouseScrollDelta::LineDelta(_, y) => y,
        MouseScrollDelta::PixelDelta(pos) => (pos.y / f64::from(metrics.height())) as f32,
    };
    let count = lines.abs().floor().max(0.0) as usize;
    let count = if count == 0 && lines != 0.0 { 1 } else { count };
    let direction = if lines < 0.0 {
        WheelDirection::Down
    } else {
        WheelDirection::Up
    };
    vec![direction; count]
}

pub(super) fn translate_key(
    event: &KeyEvent,
    modifiers: Modifiers,
) -> Result<KeyInput, KeyDropReason> {
    translate_logical_key(&event.logical_key, key_phase(event), modifiers)
}

pub(super) fn key_phase(event: &KeyEvent) -> KeyPhase {
    match event.state {
        ElementState::Released => KeyPhase::Released,
        ElementState::Pressed if event.repeat => KeyPhase::Repeat,
        ElementState::Pressed => KeyPhase::Pressed,
    }
}

pub(super) fn translate_keypad_key(event: &KeyEvent) -> Option<KeypadInput> {
    keypad_key(event.physical_key).map(|key| KeypadInput::new(key, key_phase(event)))
}

pub(super) fn keypad_key(physical_key: PhysicalKey) -> Option<KeypadKey> {
    Some(match physical_key {
        PhysicalKey::Code(KeyCode::Numpad0) => KeypadKey::Zero,
        PhysicalKey::Code(KeyCode::Numpad1) => KeypadKey::One,
        PhysicalKey::Code(KeyCode::Numpad2) => KeypadKey::Two,
        PhysicalKey::Code(KeyCode::Numpad3) => KeypadKey::Three,
        PhysicalKey::Code(KeyCode::Numpad4) => KeypadKey::Four,
        PhysicalKey::Code(KeyCode::Numpad5) => KeypadKey::Five,
        PhysicalKey::Code(KeyCode::Numpad6) => KeypadKey::Six,
        PhysicalKey::Code(KeyCode::Numpad7) => KeypadKey::Seven,
        PhysicalKey::Code(KeyCode::Numpad8) => KeypadKey::Eight,
        PhysicalKey::Code(KeyCode::Numpad9) => KeypadKey::Nine,
        PhysicalKey::Code(KeyCode::NumpadDecimal) => KeypadKey::Decimal,
        PhysicalKey::Code(KeyCode::NumpadAdd) => KeypadKey::Plus,
        PhysicalKey::Code(KeyCode::NumpadSubtract) => KeypadKey::Minus,
        PhysicalKey::Code(KeyCode::NumpadMultiply) => KeypadKey::Star,
        PhysicalKey::Code(KeyCode::NumpadDivide) => KeypadKey::Slash,
        PhysicalKey::Code(KeyCode::NumpadEnter) => KeypadKey::Enter,
        _ => return None,
    })
}

pub(super) fn translate_logical_key(
    logical_key: &WinitKey,
    phase: KeyPhase,
    modifiers: Modifiers,
) -> Result<KeyInput, KeyDropReason> {
    let key = match logical_key {
        WinitKey::Character(text) => {
            let mut characters = text.chars();
            let character = characters.next().ok_or(KeyDropReason::UnsupportedKey)?;
            if characters.next().is_some() {
                return Err(KeyDropReason::ImeOrDeadKey);
            }
            Key::Character(character)
        }
        WinitKey::Named(NamedKey::Enter) => Key::Enter,
        WinitKey::Named(NamedKey::Backspace) => Key::Backspace,
        WinitKey::Named(NamedKey::Tab) => Key::Tab,
        WinitKey::Named(NamedKey::Escape) => Key::Escape,
        WinitKey::Named(NamedKey::Space) => Key::Character(' '),
        WinitKey::Named(NamedKey::ArrowUp) => Key::Arrow(Arrow::Up),
        WinitKey::Named(NamedKey::ArrowDown) => Key::Arrow(Arrow::Down),
        WinitKey::Named(NamedKey::ArrowLeft) => Key::Arrow(Arrow::Left),
        WinitKey::Named(NamedKey::ArrowRight) => Key::Arrow(Arrow::Right),
        WinitKey::Named(NamedKey::Delete) => Key::Delete,
        WinitKey::Named(NamedKey::Insert) => Key::Insert,
        WinitKey::Named(NamedKey::Home) => Key::Home,
        WinitKey::Named(NamedKey::End) => Key::End,
        WinitKey::Named(NamedKey::PageUp) => Key::PageUp,
        WinitKey::Named(NamedKey::PageDown) => Key::PageDown,
        WinitKey::Named(NamedKey::F1) => Key::Function(FunctionKey::F1),
        WinitKey::Named(NamedKey::F2) => Key::Function(FunctionKey::F2),
        WinitKey::Named(NamedKey::F3) => Key::Function(FunctionKey::F3),
        WinitKey::Named(NamedKey::F4) => Key::Function(FunctionKey::F4),
        WinitKey::Named(NamedKey::F5) => Key::Function(FunctionKey::F5),
        WinitKey::Named(NamedKey::F6) => Key::Function(FunctionKey::F6),
        WinitKey::Named(NamedKey::F7) => Key::Function(FunctionKey::F7),
        WinitKey::Named(NamedKey::F8) => Key::Function(FunctionKey::F8),
        WinitKey::Named(NamedKey::F9) => Key::Function(FunctionKey::F9),
        WinitKey::Named(NamedKey::F10) => Key::Function(FunctionKey::F10),
        WinitKey::Named(NamedKey::F11) => Key::Function(FunctionKey::F11),
        WinitKey::Named(NamedKey::F12) => Key::Function(FunctionKey::F12),
        WinitKey::Dead(_) => return Err(KeyDropReason::ImeOrDeadKey),
        _ => return Err(KeyDropReason::UnsupportedKey),
    };
    Ok(KeyInput::new(key, phase, modifiers))
}
