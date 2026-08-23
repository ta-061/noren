#[test]
fn winit_space_variants_encode_ascii_space() {
    let variants = [
        WinitKey::Named(NamedKey::Space),
        WinitKey::Character(" ".into()),
    ];
    for logical_key in variants {
        let input = translate_logical_key(&logical_key, KeyPhase::Pressed, Modifiers::empty())
            .expect("space is supported terminal input");
        assert_eq!(KeyEncoder::encode(input), Ok(vec![0x20]));
    }
}

#[test]
fn physical_keypad_mapping_is_bounded_to_numpad_codes() {
    let cases = [
        (KeyCode::Numpad0, KeypadKey::Zero),
        (KeyCode::Numpad1, KeypadKey::One),
        (KeyCode::Numpad2, KeypadKey::Two),
        (KeyCode::Numpad3, KeypadKey::Three),
        (KeyCode::Numpad4, KeypadKey::Four),
        (KeyCode::Numpad5, KeypadKey::Five),
        (KeyCode::Numpad6, KeypadKey::Six),
        (KeyCode::Numpad7, KeypadKey::Seven),
        (KeyCode::Numpad8, KeypadKey::Eight),
        (KeyCode::Numpad9, KeypadKey::Nine),
        (KeyCode::NumpadDecimal, KeypadKey::Decimal),
        (KeyCode::NumpadAdd, KeypadKey::Plus),
        (KeyCode::NumpadSubtract, KeypadKey::Minus),
        (KeyCode::NumpadMultiply, KeypadKey::Star),
        (KeyCode::NumpadDivide, KeypadKey::Slash),
        (KeyCode::NumpadEnter, KeypadKey::Enter),
    ];
    for (code, expected) in cases {
        assert_eq!(keypad_key(PhysicalKey::Code(code)), Some(expected));
    }
    assert_eq!(keypad_key(PhysicalKey::Code(KeyCode::Digit1)), None);
}

#[test]
fn navigation_and_function_named_keys_translate_to_app_keys() {
    let cases = [
        (NamedKey::Delete, Key::Delete),
        (NamedKey::Insert, Key::Insert),
        (NamedKey::Home, Key::Home),
        (NamedKey::End, Key::End),
        (NamedKey::PageUp, Key::PageUp),
        (NamedKey::PageDown, Key::PageDown),
        (NamedKey::F1, Key::Function(FunctionKey::F1)),
        (NamedKey::F2, Key::Function(FunctionKey::F2)),
        (NamedKey::F3, Key::Function(FunctionKey::F3)),
        (NamedKey::F4, Key::Function(FunctionKey::F4)),
        (NamedKey::F5, Key::Function(FunctionKey::F5)),
        (NamedKey::F6, Key::Function(FunctionKey::F6)),
        (NamedKey::F7, Key::Function(FunctionKey::F7)),
        (NamedKey::F8, Key::Function(FunctionKey::F8)),
        (NamedKey::F9, Key::Function(FunctionKey::F9)),
        (NamedKey::F10, Key::Function(FunctionKey::F10)),
        (NamedKey::F11, Key::Function(FunctionKey::F11)),
        (NamedKey::F12, Key::Function(FunctionKey::F12)),
    ];
    for (named, expected) in cases {
        let logical_key = WinitKey::Named(named);
        let input = translate_logical_key(&logical_key, KeyPhase::Pressed, Modifiers::empty())
            .expect("stage one key is supported terminal input");
        assert_eq!(input.key(), expected);
        assert_eq!(input.phase(), KeyPhase::Pressed);
    }
}

#[test]
fn untranslated_named_keys_still_report_a_drop() {
    for named in [NamedKey::F13, NamedKey::ScrollLock, NamedKey::Pause] {
        let logical_key = WinitKey::Named(named);
        assert_eq!(
            translate_logical_key(&logical_key, KeyPhase::Pressed, Modifiers::empty()),
            Err(KeyDropReason::UnsupportedKey)
        );
    }
}

#[test]
fn diagnostics_chord_is_a_super_d_press_only() {
    let super_modifiers = Modifiers::empty().super_key();
    let chord = WinitKey::Character("d".into());
    for (state, repeat, modifiers, expected) in [
        (ElementState::Pressed, false, super_modifiers, true),
        (ElementState::Released, false, super_modifiers, false),
        (ElementState::Pressed, true, super_modifiers, false),
        (ElementState::Pressed, false, Modifiers::empty(), false),
        (
            ElementState::Pressed,
            false,
            Modifiers::empty().shift(),
            false,
        ),
    ] {
        assert_eq!(
            diagnostics_chord_pressed(&chord, state, repeat, modifiers),
            expected,
            "state={state:?} repeat={repeat}"
        );
    }
    for other in [
        WinitKey::Character("x".into()),
        WinitKey::Character("dd".into()),
        WinitKey::Named(NamedKey::Enter),
    ] {
        assert!(
            !diagnostics_chord_pressed(&other, ElementState::Pressed, false, super_modifiers),
            "only D toggles diagnostics"
        );
    }
    let shifted = WinitKey::Character("D".into());
    assert!(diagnostics_chord_pressed(
        &shifted,
        ElementState::Pressed,
        false,
        super_modifiers
    ));
}

/// A multi-chord leader whose first chord is held and then mismatched must
/// replay the held chord byte-for-byte before the mismatching chord. This
/// is the replay path whose failure would silently break Zellij.
#[test]
fn leader_mismatch_replays_held_chord_bytes_in_order() {
    // A two-chord palette leader on chords absent from the Zellij corpus:
    // bare 'a' then 'g'. Both encode non-empty bytes so a lost or
    // reordered replay changes what the child receives.
    let claim = PassthroughClaim {
        id: CLAIM_ID_PALETTE,
        action: PassthroughAction::OpenCommandPalette,
        seq: ChordSeq::new(vec![
            Chord::new(GateKeyCode::Char('a'), GateModifiers::empty()).unwrap(),
            Chord::new(GateKeyCode::Char('g'), GateModifiers::empty()).unwrap(),
        ])
        .unwrap(),
        justification: "test",
    };
    let policy = PassthroughPolicy::try_new(vec![
        PassthroughClaim {
            id: passthrough::CLAIM_ID_EXIT,
            action: PassthroughAction::ExitToWorkspace,
            seq: ChordSeq::single(
                Chord::new(GateKeyCode::Escape, GateModifiers::empty().super_key()).unwrap(),
            ),
            justification: "test",
        },
        claim,
    ])
    .expect("valid manifest");

    let mode = InputMode::normal();
    let mut gate = PassthroughGate::new();

    // Press 'a': the first chord of the palette leader — held as pending.
    let chord_a = Chord::new(GateKeyCode::Char('a'), GateModifiers::empty()).expect("normalized");
    let decision = gate.press(&policy, chord_a);
    assert_eq!(decision.kind, GateKind::Pending);
    assert!(decision.replayed.is_empty(), "pending must not replay");

    // Press 'x': not the second chord. The gate forwards, replaying 'a'
    // first. The replay must arrive byte-for-byte before 'x'.
    let chord_x = Chord::new(GateKeyCode::Char('x'), GateModifiers::empty()).expect("normalized");
    let decision = gate.press(&policy, chord_x);
    assert_eq!(decision.kind, GateKind::Forwarded);
    assert_eq!(
        decision.replayed,
        vec![chord_a],
        "the held leader chord must be replayed"
    );

    // Verify the replay bytes match direct encoding.
    let replay_bytes = encode_chord(&decision.replayed[0], mode).expect("encodes");
    assert_eq!(replay_bytes, b"a", "replayed 'a' must encode to b\"a\"");

    // After the mismatch, the gate is clean: a subsequent 'x' forwards
    // with no replay.
    let decision = gate.press(
        &policy,
        Chord::new(GateKeyCode::Char('x'), GateModifiers::empty()).unwrap(),
    );
    assert_eq!(decision.kind, GateKind::Forwarded);
    assert!(decision.replayed.is_empty(), "gate is clean after mismatch");
}

/// When the palette is closed, the gate forwards unclaimed chords. The
/// forwarded encoding must match what the encoder produces directly —
/// byte-identical to the pre-gate behaviour.
#[test]
fn closed_palette_forwarded_key_is_byte_identical_to_direct_encode() {
    let policy = palette_policy(KeymapConfig::default());
    let mode = InputMode::normal();
    let mut gate = PassthroughGate::new();

    for (code, modifiers) in [
        (GateKeyCode::Char('a'), GateModifiers::empty()),
        (GateKeyCode::Char('z'), GateModifiers::empty()),
        (GateKeyCode::Enter, GateModifiers::empty()),
        (GateKeyCode::Char('c'), GateModifiers::empty().ctrl()),
        (GateKeyCode::Char('f'), GateModifiers::empty().alt()),
    ] {
        let chord = Chord::new(code, modifiers).expect("normalized");
        let decision = gate.press(&policy, chord);
        assert_eq!(decision.kind, GateKind::Forwarded, "chord must forward");
        let forwarded = encode_chord(&chord, mode).expect("encodes");

        let app_key = gate_key_to_app(code).expect("maps to app key");
        let app_mods = app_modifiers_from_gate(modifiers);
        let direct =
            KeyEncoder::encode_with(KeyInput::new(app_key, KeyPhase::Pressed, app_mods), mode)
                .expect("encodes");
        assert_eq!(
            forwarded, direct,
            "forwarded bytes must match direct encode for {code:?}"
        );
    }
}

// ── Button and wheel mapping ────────────────────────────────────────

/// Left/Middle/Right map to the encoder's button enum; Back/Forward/Other
/// return None and are never reported.
#[test]
fn encode_button_maps_known_and_ignores_extended() {
    assert_eq!(encode_button(MouseButton::Left), Some(EncoderButton::Left));
    assert_eq!(
        encode_button(MouseButton::Middle),
        Some(EncoderButton::Middle)
    );
    assert_eq!(
        encode_button(MouseButton::Right),
        Some(EncoderButton::Right)
    );
    assert_eq!(encode_button(MouseButton::Back), None);
    assert_eq!(encode_button(MouseButton::Forward), None);
    assert_eq!(encode_button(MouseButton::Other(1)), None);
}

/// Wheel delta sign maps to direction, and magnitude maps to click count.
///
/// winit 0.30 `MouseScrollDelta` docs (from the source at
/// `winit-0.30.13/src/event.rs`):
///
///   LineDelta:  "Positive values indicate that the content that is being
///                scrolled should move right and down (revealing more
///                content left and up)."
///   PixelDelta: "Positive values indicate that the content being scrolled
///                should move right/down."
///
/// Positive y = content moves down = the user scrolled **up** (xterm button
/// 4, `Cb=64`). Negative y = scroll down (`Cb=65`). Both variants share the
/// same sign convention.
#[test]
fn wheel_clicks_direction_and_count() {
    let metrics = GridGeometry::poc().cell_metrics();
    // LineDelta: positive y = wheel up (content moves down, revealing
    // earlier content). See the winit sentence quoted above.
    let up = wheel_clicks(MouseScrollDelta::LineDelta(0.0, 1.0), metrics);
    assert_eq!(up, vec![WheelDirection::Up]);

    // Negative y = wheel down.
    let down = wheel_clicks(MouseScrollDelta::LineDelta(0.0, -3.0), metrics);
    assert_eq!(down, vec![WheelDirection::Down; 3]);

    // PixelDelta shares the same sign convention: positive y = wheel up.
    let pixel_up = wheel_clicks(
        MouseScrollDelta::PixelDelta(PhysicalPosition::new(
            0.0,
            f64::from(metrics.height()) * 2.0,
        )),
        metrics,
    );
    assert_eq!(pixel_up, vec![WheelDirection::Up; 2]);

    // Pin the full path through the encoder so the emitted bytes are fixed:
    // wheel up → Cb=64, wheel down → Cb=65 (SGR form, 1-based col/row).
    let grid = MouseGrid::new(10, 40).expect("grid");
    let modes = MouseModes::disabled().with_normal(true).with_sgr(true);
    let up_event = PointerEvent::wheel(up[0], 0, 0, PointerModifiers::empty());
    assert_eq!(
        MouseEncoder::encode(up_event, modes, grid).as_deref(),
        Some(b"\x1b[<64;1;1M".as_slice()),
        "wheel up must emit Cb=64"
    );
    let down_event = PointerEvent::wheel(down[0], 0, 0, PointerModifiers::empty());
    assert_eq!(
        MouseEncoder::encode(down_event, modes, grid).as_deref(),
        Some(b"\x1b[<65;1;1M".as_slice()),
        "wheel down must emit Cb=65"
    );
}

/// A resting trackpad reports `PixelDelta{y:0}`, which is a genuine zero
/// delta and must produce no wheel reports. The `handle_mouse_wheel` loop
/// consumes the returned vec, so an empty vec means no bytes are written to
/// the PTY — a spurious `Down` click would corrupt the application.
#[test]
fn wheel_clicks_zero_delta_produces_nothing() {
    let metrics = GridGeometry::poc().cell_metrics();
    // PixelDelta zero — the resting-trackpad case the bug shipped on.
    let pixel_zero = wheel_clicks(
        MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, 0.0)),
        metrics,
    );
    assert!(pixel_zero.is_empty(), "zero PixelDelta must emit nothing");

    // LineDelta zero must likewise emit nothing.
    let line_zero = wheel_clicks(MouseScrollDelta::LineDelta(0.0, 0.0), metrics);
    assert!(line_zero.is_empty(), "zero LineDelta must emit nothing");
}

/// A `PixelDelta` must convert pixels to lines at the **configured** cell
/// height, never a compile-time constant. Built the same way the app builds
/// its geometry — from parsed configuration — at `cell_height = 40`, double
/// the PoC default of 20. A 40px scroll is exactly one line here; if
/// `wheel_clicks` divided by the hardcoded default it would yield two. This
/// is the guard that stops the constant creeping back into the pixel→line
/// conversion.
#[test]
fn wheel_clicks_pixel_delta_uses_configured_cell_height() {
    let config = AppConfig::parse("[font]\ncell_height = 40\n").expect("valid configuration");
    let metrics = GridGeometry::with_cells(config.font().cell_width(), config.font().cell_height())
        .expect("valid geometry")
        .cell_metrics();
    assert_eq!(metrics.height(), 40);

    // One configured cell height of pixels up = exactly one wheel-up click.
    // A hardcoded POC_CELL_HEIGHT (20) would divide 40px into two clicks.
    let one_line_up = wheel_clicks(
        MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, f64::from(metrics.height()))),
        metrics,
    );
    assert_eq!(
        one_line_up,
        vec![WheelDirection::Up],
        "at cell_height=40, 40px is one line, not the two a hardcoded 20 would give"
    );

    // Two configured cell heights down = exactly two wheel-down clicks.
    let two_lines_down = wheel_clicks(
        MouseScrollDelta::PixelDelta(PhysicalPosition::new(
            0.0,
            -f64::from(metrics.height()) * 2.0,
        )),
        metrics,
    );
    assert_eq!(
        two_lines_down,
        vec![WheelDirection::Down; 2],
        "at cell_height=40, -80px is two lines, not the four a hardcoded 20 would give"
    );
}
