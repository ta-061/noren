use noren_terminal::{AnsiColor, CellAttributes, Color};

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
