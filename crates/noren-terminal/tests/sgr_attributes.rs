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
    // A hostile SGR exceeding the CSI parameter cap is dropped wholesale so the
    // parser's memory stays bounded; the pen is left untouched. The cap is 32,
    // so 40 bold-codes overflows and the whole sequence is discarded.
    let mut hostile = b"\x1b[0m\x1b[".to_vec();
    hostile.extend(b"1;".repeat(40));
    hostile.extend(b"mB");
    state.feed_bytes(&hostile);
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
fn extended_colors_apply_without_leaking_channel_values_into_style_flags() {
    let mut state = TerminalState::new(1, 3).expect("valid terminal");
    state.feed_bytes(b"\x1b[38;5;1mI\x1b[48;2;1;4;7mR\x1b[1;38;5;123;4mS");

    // `38;5;1` selects an indexed foreground; its `5` channel is consumed and
    // never reinterpreted, so the cell keeps default flags.
    let indexed = state
        .screen()
        .cell(0, 0)
        .expect("indexed cell")
        .attributes();
    assert_eq!(indexed.foreground(), Color::Indexed(1));
    assert_eq!(indexed.background(), Color::Default);
    assert!(!indexed.is_bold());
    assert!(!indexed.is_underlined());
    assert!(!indexed.is_reversed());

    // `48;2;1;4;7` is a direct RGB background; the `1`, `4`, `7` channels are
    // RGB data, not bold/underline/reverse controls.
    let direct = state.screen().cell(0, 1).expect("direct cell").attributes();
    assert_eq!(direct.foreground(), Color::Indexed(1));
    assert_eq!(direct.background(), Color::Rgb(1, 4, 7));
    assert!(!direct.is_bold());
    assert!(!direct.is_underlined());
    assert!(!direct.is_reversed());

    // Codes surrounding `38;5;123` apply to their own attributes; the indexed
    // color applies instead of leaking `5` or `123` into other slots.
    let surrounded = state
        .screen()
        .cell(0, 2)
        .expect("surrounded extended color")
        .attributes();
    assert!(surrounded.is_bold());
    assert!(surrounded.is_underlined());
    assert!(!surrounded.is_reversed());
    assert_eq!(surrounded.foreground(), Color::Indexed(123));
    assert_eq!(surrounded.background(), Color::Rgb(1, 4, 7));
}

#[test]
fn indexed_and_truecolor_sgr_colors_apply_to_all_three_color_targets() {
    let mut state = TerminalState::new(1, 7).expect("valid terminal");
    // Semicolon forms: indexed (`38;5;N` / `48;5;N` / `58;5;N`) and direct
    // RGB (`38;2;R;G;B` / `48;2;R;G;B` / `58;2;R;G;B`).
    state.feed_bytes(b"\x1b[38;5;10mF\x1b[48;5;20mG\x1b[58;5;30mU");
    state.feed_bytes(b"\x1b[38;2;1;2;3mr\x1b[48;2;4;5;6mg\x1b[58;2;7;8;9mb");

    assert_eq!(
        state
            .screen()
            .cell(0, 0)
            .expect("indexed fg")
            .attributes()
            .foreground(),
        Color::Indexed(10)
    );
    assert_eq!(
        state
            .screen()
            .cell(0, 1)
            .expect("indexed bg")
            .attributes()
            .background(),
        Color::Indexed(20)
    );
    assert_eq!(
        state
            .screen()
            .cell(0, 2)
            .expect("indexed underline")
            .attributes()
            .underline_color(),
        Color::Indexed(30)
    );
    assert_eq!(
        state
            .screen()
            .cell(0, 3)
            .expect("rgb fg")
            .attributes()
            .foreground(),
        Color::Rgb(1, 2, 3)
    );
    assert_eq!(
        state
            .screen()
            .cell(0, 4)
            .expect("rgb bg")
            .attributes()
            .background(),
        Color::Rgb(4, 5, 6)
    );
    assert_eq!(
        state
            .screen()
            .cell(0, 5)
            .expect("rgb underline")
            .attributes()
            .underline_color(),
        Color::Rgb(7, 8, 9)
    );
}

#[test]
fn colon_sub_parameter_forms_parse_for_indexed_and_truecolor() {
    let mut state = TerminalState::new(1, 7).expect("valid terminal");
    // ITU-T T.416 colon forms: `38:5:N` indexed and `38:2::R:G:B` direct RGB.
    // The empty slot after `2` is the colour-space id; it must not corrupt the
    // following RGB channels.
    state.feed_bytes(b"\x1b[38:5:10mF\x1b[48:5:20mG\x1b[58:5:30mU");
    state.feed_bytes(b"\x1b[38:2::1:2:3mr\x1b[48:2::4:5:6mg\x1b[58:2::7:8:9mb");

    assert_eq!(
        state
            .screen()
            .cell(0, 0)
            .expect("colon idx fg")
            .attributes()
            .foreground(),
        Color::Indexed(10)
    );
    assert_eq!(
        state
            .screen()
            .cell(0, 1)
            .expect("colon idx bg")
            .attributes()
            .background(),
        Color::Indexed(20)
    );
    assert_eq!(
        state
            .screen()
            .cell(0, 2)
            .expect("colon idx ul")
            .attributes()
            .underline_color(),
        Color::Indexed(30)
    );
    assert_eq!(
        state
            .screen()
            .cell(0, 3)
            .expect("colon rgb fg")
            .attributes()
            .foreground(),
        Color::Rgb(1, 2, 3)
    );
    assert_eq!(
        state
            .screen()
            .cell(0, 4)
            .expect("colon rgb bg")
            .attributes()
            .background(),
        Color::Rgb(4, 5, 6)
    );
    assert_eq!(
        state
            .screen()
            .cell(0, 5)
            .expect("colon rgb ul")
            .attributes()
            .underline_color(),
        Color::Rgb(7, 8, 9)
    );
}

#[test]
fn colon_rgb_with_an_explicit_colour_space_slot_is_accepted() {
    let mut state = TerminalState::new(1, 1).expect("valid terminal");
    // A non-empty colour-space id (`38:2:Pi:R:G:B`) is accepted and skipped;
    // the following three slots are still red/green/blue.
    state.feed_bytes(b"\x1b[38:2:3:1:2:3mX");
    assert_eq!(
        state
            .screen()
            .cell(0, 0)
            .expect("cell")
            .attributes()
            .foreground(),
        Color::Rgb(1, 2, 3)
    );
}

#[test]
fn truncated_and_out_of_range_extended_colors_leave_the_pen_unchanged() {
    let mut state = TerminalState::new(1, 2).expect("valid terminal");
    // A truncated RGB selector (`38;2;1` has one component) and an out-of-range
    // index (`38;5;300`) drop the color and never reinterpret their channel
    // values as bold/underline/reverse controls.
    state.feed_bytes(b"\x1b[38;2;1mI\x1b[1;38;5;300;4mR");

    let first = state
        .screen()
        .cell(0, 0)
        .expect("truncated cell")
        .attributes();
    assert_eq!(first.foreground(), Color::Default);
    assert!(!first.is_bold());
    assert!(!first.is_underlined());
    assert!(!first.is_reversed());

    let second = state
        .screen()
        .cell(0, 1)
        .expect("out-of-range cell")
        .attributes();
    assert!(
        second.is_bold(),
        "the `1` before the selector still applies"
    );
    assert!(
        second.is_underlined(),
        "the `4` after the dropped color still applies"
    );
    assert_eq!(
        second.foreground(),
        Color::Default,
        "the out-of-range index is dropped, not set"
    );
}

#[test]
fn sgr_reset_and_default_selectors_clear_extended_colors() {
    let mut state = TerminalState::new(1, 4).expect("valid terminal");
    // Set extended colors on all three targets plus a style flag. Each SGR is
    // sent separately so the combined parameter list stays under the CSI cap.
    state.feed_bytes(b"\x1b[1;38;5;5m\x1b[48;5;6m\x1b[58;5;7mA");

    let styled = state.screen().cell(0, 0).expect("styled cell").attributes();
    assert!(styled.is_bold());
    assert_eq!(styled.foreground(), Color::Indexed(5));
    assert_eq!(styled.background(), Color::Indexed(6));
    assert_eq!(styled.underline_color(), Color::Indexed(7));

    // A full reset (`CSI m`) clears extended colors along with everything else.
    state.feed_bytes(b"\x1b[mB");
    assert_eq!(
        state.screen().cell(0, 1).expect("reset cell").attributes(),
        &CellAttributes::default()
    );

    // `39`/`49`/`59` restore only their target, leaving other colors and flags.
    state.feed_bytes(b"\x1b[1;38;5;5m\x1b[48;5;6m\x1b[58;5;7mC\x1b[39;49;59mD");
    let after = state
        .screen()
        .cell(0, 3)
        .expect("selective reset cell")
        .attributes();
    assert!(after.is_bold(), "bold survives the color resets");
    assert_eq!(after.foreground(), Color::Default);
    assert_eq!(after.background(), Color::Default);
    assert_eq!(after.underline_color(), Color::Default);
}

#[test]
fn extended_colors_survive_an_alternate_screen_switch() {
    let mut state = TerminalState::new(1, 2).expect("valid terminal");
    // The SGR pen is terminal-global, so an extended color survives a screen
    // switch exactly as ANSI SGR state does.
    state.feed_bytes(b"\x1b[38;5;7m");
    state.feed_bytes(b"\x1b[?1049h");
    state.feed_bytes(b"\x1b[38:2::1:2:3mA");
    state.feed_bytes(b"\x1b[?1049l");
    state.feed_bytes(b"B");

    assert_eq!(state.snapshot().lines(), ["B".to_owned()]);
    assert_eq!(
        state
            .screen()
            .cell(0, 0)
            .expect("carried pen")
            .attributes()
            .foreground(),
        Color::Rgb(1, 2, 3)
    );
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

#[test]
fn combined_foreground_and_background_truecolor_in_one_sgr_sets_both() {
    // The normal way to set fg and bg truecolor together needs 10 parameter
    // slots (`38;2;R;G;B;48;2;R;G;B`). With the old cap of 8 the whole SGR was
    // dropped and both colors were silently lost.
    let mut state = TerminalState::new(1, 1).expect("valid terminal");
    state.feed_bytes(b"\x1b[38;2;1;2;3;48;2;4;5;6mA");

    let attributes = state
        .screen()
        .cell(0, 0)
        .expect("printed cell")
        .attributes();
    assert_eq!(attributes.foreground(), Color::Rgb(1, 2, 3));
    assert_eq!(attributes.background(), Color::Rgb(4, 5, 6));
}

#[test]
fn combined_foreground_background_and_underline_truecolor_in_one_sgr() {
    // All three extended-color targets in a single SGR run: 15 parameter slots,
    // comfortably inside the new capacity.
    let mut state = TerminalState::new(1, 1).expect("valid terminal");
    state.feed_bytes(b"\x1b[38;2;1;2;3;48;2;4;5;6;58;2;7;8;9mA");

    let attributes = state
        .screen()
        .cell(0, 0)
        .expect("printed cell")
        .attributes();
    assert_eq!(attributes.foreground(), Color::Rgb(1, 2, 3));
    assert_eq!(attributes.background(), Color::Rgb(4, 5, 6));
    assert_eq!(attributes.underline_color(), Color::Rgb(7, 8, 9));
}

#[test]
fn hostile_sgr_beyond_the_capacity_is_dropped_and_memory_stays_bounded() {
    // A hostile `\x1b[1;1;…;1m` with thousands of parameters must be dropped
    // safely: the parser never grows its fixed parameter arrays past the cap,
    // and the pen is left untouched.
    let mut state = TerminalState::new(1, 2).expect("valid terminal");
    state.feed_bytes(b"\x1b[31mR");
    let mut hostile = b"\x1b[".to_vec();
    hostile.extend(b"1;".repeat(5000));
    hostile.extend(b"mB");
    state.feed_bytes(&hostile);

    let red = state.screen().cell(0, 0).expect("red cell").attributes();
    assert_eq!(red.foreground(), Color::Ansi(AnsiColor::Red));
    // The hostile SGR was discarded wholesale, so the pen kept its red
    // foreground; B was printed red, not reset.
    let untouched = state
        .screen()
        .cell(0, 1)
        .expect("post-hostile cell")
        .attributes();
    assert_eq!(untouched.foreground(), Color::Ansi(AnsiColor::Red));
    assert!(!untouched.is_bold());
}

#[test]
fn unsupported_colon_underline_styles_leave_flags_untouched() {
    // `4:0`, `4:1`, `4:3` are modern (curly/dotted) underline styles this slice
    // does not render. Each is one unsupported parameter group and must be
    // skipped whole — the trailing sub-parameter must never reach the pen, so
    // it must not turn underline on nor reset existing attributes.
    for sequence in [b"\x1b[4:0m", b"\x1b[4:1m", b"\x1b[4:3m"] {
        let mut state = TerminalState::new(1, 1).expect("valid terminal");
        state.feed_bytes(b"\x1b[1m");
        state.feed_bytes(sequence);
        state.feed_bytes(b"A");

        let attributes = state
            .screen()
            .cell(0, 0)
            .expect("printed cell")
            .attributes();
        assert!(attributes.is_bold(), "bold survived {sequence:?}");
        assert!(
            !attributes.is_underlined(),
            "underline not turned on by {sequence:?}"
        );
        assert!(
            !attributes.is_reversed(),
            "reverse not flipped by {sequence:?}"
        );
    }

    // A compound colon attribute with several sub-parameters is skipped whole
    // as well, leaving the pen exactly as it was.
    let mut state = TerminalState::new(1, 1).expect("valid terminal");
    state.feed_bytes(b"\x1b[1;31m");
    state.feed_bytes(b"\x1b[21:0:1mA");
    let attributes = state
        .screen()
        .cell(0, 0)
        .expect("printed cell")
        .attributes();
    assert!(attributes.is_bold());
    assert_eq!(attributes.foreground(), Color::Ansi(AnsiColor::Red));
}

#[test]
fn itu_colon_truecolor_still_works_alongside_solidus_forms() {
    // Regression guard: the ITU-T T.416 colon form (`38:2::R:G:B`) must keep
    // working after the colon-group skipping change, and SGR reset plus the
    // `39`/`49` default selectors still behave on the semicolon form.
    let mut state = TerminalState::new(1, 8).expect("valid terminal");
    state.feed_bytes(b"\x1b[38:2::1:2:3mA");

    let cell = state
        .screen()
        .cell(0, 0)
        .expect("colon rgb cell")
        .attributes();
    assert_eq!(cell.foreground(), Color::Rgb(1, 2, 3));

    // Semicolon-form truecolor still works.
    state.feed_bytes(b"\x1b[48;2;9;9;9mB");
    let cell = state
        .screen()
        .cell(0, 1)
        .expect("semicolon rgb cell")
        .attributes();
    assert_eq!(cell.foreground(), Color::Rgb(1, 2, 3));
    assert_eq!(cell.background(), Color::Rgb(9, 9, 9));

    // Full reset and selective `39`/`49` defaults still clear colors.
    state.feed_bytes(b"\x1b[mC");
    assert_eq!(
        state.screen().cell(0, 2).expect("reset cell").attributes(),
        &CellAttributes::default()
    );

    state.feed_bytes(b"\x1b[38;2;1;1;1;48;2;2;2;2mD\x1b[39;49mE");
    let selective = state
        .screen()
        .cell(0, 4)
        .expect("selective reset cell")
        .attributes();
    assert_eq!(selective.foreground(), Color::Default);
    assert_eq!(selective.background(), Color::Default);
}
