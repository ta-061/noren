use noren_terminal::{AnsiColor, Cell, CellAttributes, Color, TerminalState};

const STYLED: CellAttributes = CellAttributes::new()
    .with_foreground(Color::ansi(AnsiColor::BrightGreen))
    .with_background(Color::ansi(AnsiColor::Blue))
    .with_bold(true)
    .with_underline(true)
    .with_reverse(true);

#[test]
fn ansi_colors_have_stable_palette_indexes() {
    for (index, color) in AnsiColor::ALL.into_iter().enumerate() {
        assert_eq!(usize::from(color.palette_index()), index);
    }
}

#[test]
fn color_default_is_contextual_and_ansi_colors_round_trip() {
    assert_eq!(Color::default(), Color::Default);
    assert!(Color::Default.is_default());
    assert_eq!(Color::Default.ansi_color(), None);

    let red = Color::ansi(AnsiColor::Red);
    assert!(!red.is_default());
    assert_eq!(red.ansi_color(), Some(AnsiColor::Red));
}

#[test]
fn cell_attributes_have_a_stable_baseline_default() {
    let attributes = CellAttributes::default();

    assert_eq!(attributes, CellAttributes::DEFAULT);
    assert_eq!(attributes, CellAttributes::new());
    assert_eq!(attributes.foreground(), Color::Default);
    assert_eq!(attributes.background(), Color::Default);
    assert!(!attributes.is_bold());
    assert!(!attributes.is_underlined());
    assert!(!attributes.is_reversed());
}

#[test]
fn cell_attribute_builders_are_const_and_independent() {
    assert_eq!(STYLED.foreground(), Color::Ansi(AnsiColor::BrightGreen));
    assert_eq!(STYLED.background(), Color::Ansi(AnsiColor::Blue));
    assert!(STYLED.is_bold());
    assert!(STYLED.is_underlined());
    assert!(STYLED.is_reversed());

    let changed = STYLED.with_bold(false).with_background(Color::Default);
    assert_eq!(changed.foreground(), STYLED.foreground());
    assert_eq!(changed.background(), Color::Default);
    assert!(!changed.is_bold());
    assert!(changed.is_underlined());
    assert!(changed.is_reversed());
}

#[test]
fn cells_default_to_baseline_attributes_without_changing_cell_new() {
    let cell = Cell::new("x", 1);

    assert_eq!(cell.text(), "x");
    assert_eq!(cell.width(), 1);
    assert_eq!(cell.attributes(), &CellAttributes::default());
    assert_eq!(Cell::blank().attributes(), &CellAttributes::default());
}

#[test]
fn ordered_sgr_combinations_apply_supported_codes_around_unsupported_codes() {
    let mut state = TerminalState::new(1, 2).expect("valid terminal");
    state.feed_bytes(b"\x1b[1;31;999;44;22;4;7mX");

    let attributes = state
        .screen()
        .cell(0, 0)
        .expect("printed cell")
        .attributes();
    assert_eq!(attributes.foreground(), Color::Ansi(AnsiColor::Red));
    assert_eq!(attributes.background(), Color::Ansi(AnsiColor::Blue));
    assert!(!attributes.is_bold());
    assert!(attributes.is_underlined());
    assert!(attributes.is_reversed());
}

#[test]
fn ordered_reset_and_selective_resets_only_change_their_attributes() {
    let mut state = TerminalState::new(1, 3).expect("valid terminal");
    state.feed_bytes(b"\x1b[1;4;7;93;46mA\x1b[22;24;27;39;49mB\x1b[1;0;4mC");

    let first = state.screen().cell(0, 0).expect("first cell").attributes();
    assert!(first.is_bold());
    assert!(first.is_underlined());
    assert!(first.is_reversed());
    assert_eq!(first.foreground(), Color::Ansi(AnsiColor::BrightYellow));
    assert_eq!(first.background(), Color::Ansi(AnsiColor::Cyan));

    assert_eq!(
        state.screen().cell(0, 1).expect("second cell").attributes(),
        &CellAttributes::default()
    );

    let third = state.screen().cell(0, 2).expect("third cell").attributes();
    assert!(!third.is_bold());
    assert!(third.is_underlined());
    assert!(!third.is_reversed());
    assert_eq!(third.foreground(), Color::Default);
    assert_eq!(third.background(), Color::Default);
}

#[test]
fn all_sixteen_ansi_colors_are_captured_for_foreground_and_background() {
    let mut state = TerminalState::new(1, 32).expect("valid terminal");
    let mut input = Vec::new();
    for index in 0_u8..16 {
        let foreground = if index < 8 {
            30 + index
        } else {
            90 + index - 8
        };
        let background = if index < 8 {
            40 + index
        } else {
            100 + index - 8
        };
        input.extend_from_slice(format!("\x1b[{foreground}mF\x1b[{background}mB").as_bytes());
    }
    state.feed_bytes(&input);

    for (index, color) in AnsiColor::ALL.into_iter().enumerate() {
        let foreground = state
            .screen()
            .cell(0, u16::try_from(index * 2).expect("bounded index"))
            .expect("foreground cell")
            .attributes();
        let background = state
            .screen()
            .cell(0, u16::try_from(index * 2 + 1).expect("bounded index"))
            .expect("background cell")
            .attributes();
        assert_eq!(foreground.foreground(), Color::Ansi(color));
        assert_eq!(background.background(), Color::Ansi(color));
    }
}

#[test]
fn printed_cells_capture_the_pen_without_retroactive_changes() {
    let mut state = TerminalState::new(1, 3).expect("valid terminal");
    state.feed_bytes(b"\x1b[31mR\x1b[32mG\x1b[mD");

    assert_eq!(
        state
            .screen()
            .cell(0, 0)
            .expect("red cell")
            .attributes()
            .foreground(),
        Color::Ansi(AnsiColor::Red)
    );
    assert_eq!(
        state
            .screen()
            .cell(0, 1)
            .expect("green cell")
            .attributes()
            .foreground(),
        Color::Ansi(AnsiColor::Green)
    );
    assert_eq!(
        state
            .screen()
            .cell(0, 2)
            .expect("default cell")
            .attributes(),
        &CellAttributes::default()
    );
    assert_eq!(state.attributes(), &CellAttributes::default());
}

#[test]
fn snapshots_and_resize_preserve_captured_cell_attributes() {
    let mut state = TerminalState::new(1, 2).expect("valid terminal");
    state.feed_bytes(b"\x1b[1;95mX");
    let snapshot = state.snapshot();

    state.feed_bytes(b"\x1b[0mY");
    state.resize(2, 3).expect("valid resize");

    let captured = snapshot
        .screen()
        .cell(0, 0)
        .expect("snapshot cell")
        .attributes();
    assert!(captured.is_bold());
    assert_eq!(captured.foreground(), Color::Ansi(AnsiColor::BrightMagenta));
    assert_eq!(snapshot.lines(), ["X".to_owned()]);

    let resized = state
        .screen()
        .cell(0, 0)
        .expect("resized cell")
        .attributes();
    assert_eq!(resized, captured);
    assert_eq!(
        state.screen().cell(1, 2).expect("new blank").attributes(),
        &CellAttributes::default()
    );
}

#[test]
fn alternate_screen_cells_are_isolated_while_the_pen_is_preserved() {
    let mut state = TerminalState::new(1, 3).expect("valid terminal");
    state.feed_bytes(b"\x1b[31mP\x1b[?1049h\x1b[32mA");

    assert_eq!(state.snapshot().lines(), ["A".to_owned()]);
    assert_eq!(
        state
            .screen()
            .cell(0, 0)
            .expect("alternate cell")
            .attributes()
            .foreground(),
        Color::Ansi(AnsiColor::Green)
    );

    state.feed_bytes(b"\x1b[?1049lG");
    assert_eq!(state.snapshot().lines(), ["PG".to_owned()]);
    assert_eq!(
        state
            .screen()
            .cell(0, 0)
            .expect("primary cell")
            .attributes()
            .foreground(),
        Color::Ansi(AnsiColor::Red)
    );
    assert_eq!(
        state
            .screen()
            .cell(0, 1)
            .expect("post-switch cell")
            .attributes()
            .foreground(),
        Color::Ansi(AnsiColor::Green)
    );
}

#[test]
fn unsupported_codes_are_ignored_and_parameter_overflow_drops_the_sgr() {
    let mut state = TerminalState::new(1, 3).expect("valid terminal");
    state.feed_bytes(b"\x1b[31;1234;1mA");
    state.feed_bytes(b"\x1b[0m\x1b[31;1;4;7;40;90;100;22;24mB");
    state.feed_bytes(b"\x1b[32mC");

    let first = state.screen().cell(0, 0).expect("first cell").attributes();
    assert_eq!(first.foreground(), Color::Ansi(AnsiColor::Red));
    assert!(first.is_bold());
    assert_eq!(
        state
            .screen()
            .cell(0, 1)
            .expect("overflow cell")
            .attributes(),
        &CellAttributes::default()
    );
    assert_eq!(
        state
            .screen()
            .cell(0, 2)
            .expect("recovered cell")
            .attributes()
            .foreground(),
        Color::Ansi(AnsiColor::Green)
    );
}

#[test]
fn deferred_extended_colors_do_not_leak_channel_values_into_style_flags() {
    let mut state = TerminalState::new(1, 3).expect("valid terminal");
    state.feed_bytes(b"\x1b[38;5;1mI\x1b[48;2;1;4;7mR\x1b[1;38;5;123;4mS");

    assert_eq!(
        state
            .screen()
            .cell(0, 0)
            .expect("indexed cell")
            .attributes(),
        &CellAttributes::default()
    );
    assert_eq!(
        state.screen().cell(0, 1).expect("direct cell").attributes(),
        &CellAttributes::default()
    );

    let surrounded = state
        .screen()
        .cell(0, 2)
        .expect("surrounded extended color")
        .attributes();
    assert!(surrounded.is_bold());
    assert!(surrounded.is_underlined());
    assert_eq!(surrounded.foreground(), Color::Default);
    assert_eq!(surrounded.background(), Color::Default);
}

#[test]
fn erase_and_inserted_blank_cells_use_default_attributes() {
    let mut state = TerminalState::new(1, 4).expect("valid terminal");
    state.feed_bytes(b"\x1b[31mAB\x1b[1G\x1b[@");

    assert_eq!(
        state
            .screen()
            .cell(0, 0)
            .expect("inserted blank")
            .attributes(),
        &CellAttributes::default()
    );
    assert_eq!(
        state
            .screen()
            .cell(0, 1)
            .expect("shifted cell")
            .attributes()
            .foreground(),
        Color::Ansi(AnsiColor::Red)
    );

    state.feed_bytes(b"\x1b[2K");
    assert!(
        state
            .screen()
            .cells()
            .iter()
            .all(|cell| cell.attributes() == &CellAttributes::default())
    );
    assert_eq!(state.attributes().foreground(), Color::Ansi(AnsiColor::Red));
}
