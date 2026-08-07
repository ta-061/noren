//! Byte-exact verification of the mouse input encoder (`src/mouse.rs`).
//!
//! The mouse module is not yet wired into `lib.rs` (export wiring is a
//! separate serial commit owned by another lane). It is self-contained — no
//! `crate::` items — so this target compiles it directly from its source path
//! and exercises the public API the way the future terminal-mode wiring will.
//!
//! Coordinate convention used throughout: `col`/`row` are 0-based cell indices;
//! the encoder clamps to the grid and emits 1-based `Cx`/`Cy`. A click at
//! `col=9, row=4` on any grid that contains it resolves to `Cx=10, Cy=5`.

#[path = "../src/mouse.rs"]
mod mouse;

use mouse::{
    MODE_ANY_EVENT, MODE_BUTTON_EVENT, MODE_NORMAL, MODE_SGR, MODE_URXVT, MODE_UTF8, MouseButton,
    MouseEncoder, MouseGrid, MouseModes, PointerEvent, PointerModifiers, WheelDirection,
    X10_MAX_COORD,
};

/// 80x24 grid; `(9, 4)` is always in range and resolves to `Cx=10, Cy=5`.
const GRID: MouseGrid = match MouseGrid::new(80, 24) {
    Some(grid) => grid,
    None => panic!("80x24 is a valid grid"),
};
const P9: u32 = 9;
const ROW4: u32 = 4;
const NO_MODS: PointerModifiers = PointerModifiers::empty();

/// Modes the Zellij client enables on attach: tracking 1000/1002/1003 plus
/// encodings 1006 and 1015. SGR (1006) wins the precedence.
fn zellij_modes() -> MouseModes {
    MouseModes::disabled()
        .with_normal(true)
        .with_button_event(true)
        .with_any_event(true)
        .with_sgr(true)
        .with_urxvt(true)
}

/// Plain SGR tracking for the byte-form suites.
fn sgr_modes() -> MouseModes {
    MouseModes::disabled().with_normal(true).with_sgr(true)
}

/// X10 byte form: only normal tracking, no parameterized encoding.
fn x10_modes() -> MouseModes {
    MouseModes::disabled().with_normal(true)
}

// ── SGR (1006) byte-exact forms ─────────────────────────────────────────

#[test]
fn sgr_press_left_is_byte_exact() {
    let event = PointerEvent::press(MouseButton::Left, P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(event, sgr_modes(), GRID).as_deref(),
        Some(b"\x1b[<0;10;5M".as_slice())
    );
}

#[test]
fn sgr_release_left_uses_lowercase_m_and_keeps_button_code() {
    let event = PointerEvent::release(MouseButton::Left, P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(event, sgr_modes(), GRID).as_deref(),
        Some(b"\x1b[<0;10;5m".as_slice())
    );
}

#[test]
fn sgr_release_middle_keeps_button_one() {
    let event = PointerEvent::release(MouseButton::Middle, P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(event, sgr_modes(), GRID).as_deref(),
        Some(b"\x1b[<1;10;5m".as_slice())
    );
}

#[test]
fn sgr_drag_left_adds_the_motion_flag() {
    // A drag is motion with a held button: requires button-event (1002) or
    // any-event (1003) tracking, not plain normal (1000).
    let modes = MouseModes::disabled()
        .with_button_event(true)
        .with_sgr(true);
    let event = PointerEvent::move_to(Some(MouseButton::Left), P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(event, modes, GRID).as_deref(),
        Some(b"\x1b[<32;10;5M".as_slice())
    );
}

#[test]
fn sgr_wheel_up_is_button_sixty_four() {
    let event = PointerEvent::wheel(WheelDirection::Up, P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(event, sgr_modes(), GRID).as_deref(),
        Some(b"\x1b[<64;10;5M".as_slice())
    );
}

#[test]
fn sgr_wheel_down_is_button_sixty_five() {
    let event = PointerEvent::wheel(WheelDirection::Down, P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(event, sgr_modes(), GRID).as_deref(),
        Some(b"\x1b[<65;10;5M".as_slice())
    );
}

#[test]
fn sgr_middle_and_right_press_codes_are_one_and_two() {
    let middle = PointerEvent::press(MouseButton::Middle, P9, ROW4, NO_MODS);
    let right = PointerEvent::press(MouseButton::Right, P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(middle, sgr_modes(), GRID).as_deref(),
        Some(b"\x1b[<1;10;5M".as_slice())
    );
    assert_eq!(
        MouseEncoder::encode(right, sgr_modes(), GRID).as_deref(),
        Some(b"\x1b[<2;10;5M".as_slice())
    );
}

#[test]
fn sgr_modifier_bits_fold_into_cb() {
    let shift = PointerEvent::press(
        MouseButton::Left,
        P9,
        ROW4,
        PointerModifiers::empty().shift(),
    );
    let alt = PointerEvent::press(MouseButton::Left, P9, ROW4, PointerModifiers::empty().alt());
    let ctrl = PointerEvent::press(
        MouseButton::Left,
        P9,
        ROW4,
        PointerModifiers::empty().ctrl(),
    );
    let all = PointerEvent::press(
        MouseButton::Left,
        P9,
        ROW4,
        PointerModifiers::empty().shift().alt().ctrl(),
    );
    assert_eq!(
        MouseEncoder::encode(shift, sgr_modes(), GRID).as_deref(),
        Some(b"\x1b[<4;10;5M".as_slice()) // 0 + 4
    );
    assert_eq!(
        MouseEncoder::encode(alt, sgr_modes(), GRID).as_deref(),
        Some(b"\x1b[<8;10;5M".as_slice()) // 0 + 8
    );
    assert_eq!(
        MouseEncoder::encode(ctrl, sgr_modes(), GRID).as_deref(),
        Some(b"\x1b[<16;10;5M".as_slice()) // 0 + 16
    );
    assert_eq!(
        MouseEncoder::encode(all, sgr_modes(), GRID).as_deref(),
        Some(b"\x1b[<28;10;5M".as_slice()) // 0 + 4 + 8 + 16
    );
}

#[test]
fn sgr_drag_with_modifier_combines_motion_and_modifier_bits() {
    let modes = MouseModes::disabled()
        .with_button_event(true)
        .with_sgr(true);
    let event = PointerEvent::move_to(
        Some(MouseButton::Left),
        P9,
        ROW4,
        PointerModifiers::empty().shift().ctrl(),
    );
    assert_eq!(
        MouseEncoder::encode(event, modes, GRID).as_deref(),
        Some(b"\x1b[<52;10;5M".as_slice()) // 0 + 32 + 4 + 16
    );
}

// ── X10 byte-form fallback (only normal tracking, no encoding) ───────────

#[test]
fn x10_press_left_is_csi_m_plus_offset_bytes() {
    let event = PointerEvent::press(MouseButton::Left, P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(event, x10_modes(), GRID).as_deref(),
        Some(b"\x1b[M\x20\x2a\x25".as_slice()) // Cb=0+32, Cx=10+32, Cy=5+32
    );
}

#[test]
fn x10_release_collapses_every_button_to_cb_three() {
    let event = PointerEvent::release(MouseButton::Left, P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(event, x10_modes(), GRID).as_deref(),
        Some(b"\x1b[M\x23\x2a\x25".as_slice()) // Cb=3+32
    );
    // A middle-button release is indistinguishable from a left release in X10.
    let middle = PointerEvent::release(MouseButton::Middle, P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(middle, x10_modes(), GRID).as_deref(),
        Some(b"\x1b[M\x23\x2a\x25".as_slice())
    );
}

#[test]
fn x10_wheel_up_is_cb_sixty_four() {
    let event = PointerEvent::wheel(WheelDirection::Up, P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(event, x10_modes(), GRID).as_deref(),
        Some(b"\x1b[M\x60\x2a\x25".as_slice()) // Cb=64+32
    );
}

#[test]
fn x10_middle_press_and_shift_modifier_byte_forms() {
    let middle = PointerEvent::press(MouseButton::Middle, P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(middle, x10_modes(), GRID).as_deref(),
        Some(b"\x1b[M\x21\x2a\x25".as_slice()) // Cb=1+32
    );
    let shifted = PointerEvent::press(
        MouseButton::Left,
        P9,
        ROW4,
        PointerModifiers::empty().shift(),
    );
    assert_eq!(
        MouseEncoder::encode(shifted, x10_modes(), GRID).as_deref(),
        Some(b"\x1b[M\x24\x2a\x25".as_slice()) // Cb=4+32
    );
}

#[test]
fn x10_drag_left_adds_motion_flag_byte() {
    // 1002 must be on for motion to report at all.
    let modes = MouseModes::disabled()
        .with_normal(true)
        .with_button_event(true);
    let event = PointerEvent::move_to(Some(MouseButton::Left), P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(event, modes, GRID).as_deref(),
        Some(b"\x1b[M\x40\x2a\x25".as_slice()) // Cb=32+32
    );
}

#[test]
fn mode_1005_falls_through_to_the_x10_byte_form() {
    // 1005 is tracked but its UTF-8 coordinate extension is unimplemented;
    // with neither 1006 nor 1015 active the X10 byte form is used.
    let modes = MouseModes::disabled().with_normal(true).with_utf8(true);
    let event = PointerEvent::press(MouseButton::Left, P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(event, modes, GRID).as_deref(),
        Some(b"\x1b[M\x20\x2a\x25".as_slice())
    );
}

// ── Nothing emitted when no tracking mode is enabled ────────────────────

#[test]
fn no_tracking_emits_nothing_for_every_kind() {
    let modes = MouseModes::disabled().with_sgr(true).with_urxvt(true);
    let press = PointerEvent::press(MouseButton::Left, P9, ROW4, NO_MODS);
    let release = PointerEvent::release(MouseButton::Left, P9, ROW4, NO_MODS);
    let wheel = PointerEvent::wheel(WheelDirection::Up, P9, ROW4, NO_MODS);
    let motion = PointerEvent::move_to(Some(MouseButton::Left), P9, ROW4, NO_MODS);
    assert_eq!(MouseEncoder::encode(press, modes, GRID), None);
    assert_eq!(MouseEncoder::encode(release, modes, GRID), None);
    assert_eq!(MouseEncoder::encode(wheel, modes, GRID), None);
    assert_eq!(MouseEncoder::encode(motion, modes, GRID), None);
}

#[test]
fn plain_one_thousand_reports_no_motion_at_all() {
    // Normal tracking (1000) reports press/release/wheel but never motion.
    let modes = x10_modes();
    let with_button = PointerEvent::move_to(Some(MouseButton::Left), P9, ROW4, NO_MODS);
    let without_button = PointerEvent::move_to(None, P9, ROW4, NO_MODS);
    assert_eq!(MouseEncoder::encode(with_button, modes, GRID), None);
    assert_eq!(MouseEncoder::encode(without_button, modes, GRID), None);
    // Press still reports.
    let press = PointerEvent::press(MouseButton::Left, P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(press, modes, GRID).as_deref(),
        Some(b"\x1b[M\x20\x2a\x25".as_slice())
    );
}

// ── 1002 versus 1003 differ on drag-without-button ──────────────────────

#[test]
fn button_event_one_thousand_two_drops_motion_without_a_button() {
    let modes = MouseModes::disabled()
        .with_normal(true)
        .with_button_event(true)
        .with_sgr(true);
    let hover = PointerEvent::move_to(None, P9, ROW4, NO_MODS);
    assert_eq!(MouseEncoder::encode(hover, modes, GRID), None);

    // A drag with a held button still reports under 1002.
    let drag = PointerEvent::move_to(Some(MouseButton::Left), P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(drag, modes, GRID).as_deref(),
        Some(b"\x1b[<32;10;5M".as_slice())
    );
}

#[test]
fn any_event_one_thousand_three_reports_motion_without_a_button() {
    let modes = MouseModes::disabled()
        .with_normal(true)
        .with_any_event(true)
        .with_sgr(true);
    let hover = PointerEvent::move_to(None, P9, ROW4, NO_MODS);
    // Cb = release(3) + motion(32) = 35, capital M.
    assert_eq!(
        MouseEncoder::encode(hover, modes, GRID).as_deref(),
        Some(b"\x1b[<35;10;5M".as_slice())
    );

    // 1003 reports a drag identically to 1002.
    let drag = PointerEvent::move_to(Some(MouseButton::Left), P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(drag, modes, GRID).as_deref(),
        Some(b"\x1b[<32;10;5M".as_slice())
    );
}

#[test]
fn one_thousand_two_without_sgr_drops_hover_in_x10_form() {
    // Same 1002-vs-1003 distinction holds in the X10 byte form too.
    let modes = MouseModes::disabled()
        .with_normal(true)
        .with_button_event(true);
    let hover = PointerEvent::move_to(None, P9, ROW4, NO_MODS);
    assert_eq!(MouseEncoder::encode(hover, modes, GRID), None);
}

// ── Coordinates clamp to the grid and to 1x1 ────────────────────────────

#[test]
fn coordinates_clamp_at_the_right_and_bottom_edges() {
    let beyond_col = PointerEvent::press(MouseButton::Left, 100, ROW4, NO_MODS); // col 100 -> 79
    assert_eq!(
        MouseEncoder::encode(beyond_col, sgr_modes(), GRID).as_deref(),
        Some(b"\x1b[<0;80;5M".as_slice())
    );
    let beyond_row = PointerEvent::press(MouseButton::Left, P9, 30, NO_MODS); // row 30 -> 23
    assert_eq!(
        MouseEncoder::encode(beyond_row, sgr_modes(), GRID).as_deref(),
        Some(b"\x1b[<0;10;24M".as_slice())
    );
    let beyond_both = PointerEvent::press(MouseButton::Left, 200, 30, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(beyond_both, sgr_modes(), GRID).as_deref(),
        Some(b"\x1b[<0;80;24M".as_slice())
    );
}

#[test]
fn coordinates_floor_at_one_one_at_the_top_left() {
    let top_left = PointerEvent::press(MouseButton::Left, 0, 0, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(top_left, sgr_modes(), GRID).as_deref(),
        Some(b"\x1b[<0;1;1M".as_slice())
    );
}

#[test]
fn one_by_one_grid_clamps_everything_to_cell_one_one() {
    let grid = MouseGrid::new(1, 1).expect("1x1 is valid");
    let event = PointerEvent::press(MouseButton::Left, 5, 5, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(event, sgr_modes(), grid).as_deref(),
        Some(b"\x1b[<0;1;1M".as_slice())
    );
}

#[test]
fn mouse_grid_reports_its_dimensions() {
    let grid = MouseGrid::new(132, 50).expect("132x50 is valid");
    assert_eq!((grid.cols(), grid.rows()), (132, 50));
}

#[test]
fn mouse_grid_rejects_zero_dimensions() {
    assert_eq!(MouseGrid::new(0, 0), None);
    assert_eq!(MouseGrid::new(0, 24), None);
    assert_eq!(MouseGrid::new(80, 0), None);
}

// ── X10 223-column limit: chosen rule is DROP (never misreport) ──────────
//
// SGR and urxvt are decimal and have no limit. The X10 byte form offsets each
// coordinate by 32, so a 1-based coordinate above 223 would overflow a byte.
// Rule chosen here: drop the report entirely (`None`) rather than emit a
// saturated, wrong position. The module docs record this rule.

#[test]
fn x10_drops_when_column_exceeds_two_hundred_twenty_three() {
    let grid = MouseGrid::new(300, 50).expect("300x50 is valid");
    let modes = x10_modes();
    // col 230 -> Cx 231 > 223.
    let beyond = PointerEvent::press(MouseButton::Left, 230, 4, NO_MODS);
    assert_eq!(MouseEncoder::encode(beyond, modes, grid), None);
}

#[test]
fn x10_drops_when_row_exceeds_two_hundred_twenty_three() {
    let grid = MouseGrid::new(50, 300).expect("50x300 is valid");
    let modes = x10_modes();
    // row 230 -> Cy 231 > 223.
    let beyond = PointerEvent::press(MouseButton::Left, 4, 230, NO_MODS);
    assert_eq!(MouseEncoder::encode(beyond, modes, grid), None);
}

#[test]
fn x10_reports_at_the_two_hundred_twenty_three_boundary() {
    let grid = MouseGrid::new(300, 50).expect("300x50 is valid");
    let modes = x10_modes();
    // col 222 -> Cx 223 (the last representable column); Cx byte = 223+32 = 255.
    let at_edge = PointerEvent::press(MouseButton::Left, 222, 4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(at_edge, modes, grid).as_deref(),
        Some(b"\x1b[M\x20\xff\x25".as_slice()) // Cb=0+32, Cx=223+32=255, Cy=5+32
    );
    // col 223 -> Cx 224: one past the boundary, dropped.
    let past_edge = PointerEvent::press(MouseButton::Left, 223, 4, NO_MODS);
    assert_eq!(MouseEncoder::encode(past_edge, modes, grid), None);
}

#[test]
fn sgr_has_no_two_hundred_twenty_three_limit() {
    let grid = MouseGrid::new(300, 50).expect("300x50 is valid");
    let modes = sgr_modes();
    let beyond = PointerEvent::press(MouseButton::Left, 230, 4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(beyond, modes, grid).as_deref(),
        Some(b"\x1b[<0;231;5M".as_slice())
    );
}

// ── Encoding precedence and urxvt (1015) byte forms ─────────────────────

#[test]
fn sgr_takes_precedence_over_urxvt_when_both_enabled() {
    // The Zellij attach preset turns on both 1006 and 1015; SGR wins.
    let modes = zellij_modes();
    let event = PointerEvent::press(MouseButton::Left, P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(event, modes, GRID).as_deref(),
        Some(b"\x1b[<0;10;5M".as_slice()) // angle bracket => SGR
    );
}

#[test]
fn urxvt_press_is_decimal_without_angle_bracket() {
    let modes = MouseModes::disabled().with_normal(true).with_urxvt(true);
    let event = PointerEvent::press(MouseButton::Left, P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(event, modes, GRID).as_deref(),
        Some(b"\x1b[0;10;5M".as_slice())
    );
}

#[test]
fn urxvt_release_collapses_to_cb_three() {
    let modes = MouseModes::disabled().with_normal(true).with_urxvt(true);
    let event = PointerEvent::release(MouseButton::Left, P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(event, modes, GRID).as_deref(),
        Some(b"\x1b[3;10;5M".as_slice())
    );
}

#[test]
fn urxvt_wheel_and_drag_decimal_forms() {
    let modes = MouseModes::disabled()
        .with_normal(true)
        .with_button_event(true)
        .with_urxvt(true);
    let wheel = PointerEvent::wheel(WheelDirection::Up, P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(wheel, modes, GRID).as_deref(),
        Some(b"\x1b[64;10;5M".as_slice())
    );
    let drag = PointerEvent::move_to(Some(MouseButton::Left), P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(drag, modes, GRID).as_deref(),
        Some(b"\x1b[32;10;5M".as_slice())
    );
}

// ── Mode-number driven tracking (future DECSET/DECRST wiring) ────────────

#[test]
fn set_by_mode_number_drives_tracking_and_encoding() {
    let modes = MouseModes::disabled()
        .set(MODE_SGR, true)
        .set(MODE_NORMAL, true);
    assert!(modes.is_tracked());
    let event = PointerEvent::press(MouseButton::Left, P9, ROW4, NO_MODS);
    assert_eq!(
        MouseEncoder::encode(event, modes, GRID).as_deref(),
        Some(b"\x1b[<0;10;5M".as_slice())
    );
}

#[test]
fn set_toggles_modes_off_and_ignores_unrelated_numbers() {
    let modes = MouseModes::disabled()
        .set(MODE_NORMAL, true)
        .set(MODE_BUTTON_EVENT, true)
        .set(MODE_ANY_EVENT, true)
        .set(MODE_UTF8, true)
        .set(MODE_SGR, true)
        .set(MODE_URXVT, true);
    assert!(modes.is_tracked());
    // Turning tracking off leaves encodings untouched but silences output.
    let silent = modes
        .set(MODE_NORMAL, false)
        .set(MODE_BUTTON_EVENT, false)
        .set(MODE_ANY_EVENT, false);
    assert!(!silent.is_tracked());
    let event = PointerEvent::press(MouseButton::Left, P9, ROW4, NO_MODS);
    assert_eq!(MouseEncoder::encode(event, silent, GRID), None);

    // An unrelated mode number (e.g. 1049 alternate screen) is ignored.
    let unchanged = modes.set(1049, true);
    assert_eq!(unchanged, modes);
}

#[test]
fn x10_max_coord_constant_is_two_hundred_twenty_three() {
    assert_eq!(X10_MAX_COORD, 223);
}
