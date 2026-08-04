//! Small, bounded byte parser for the first Terminal Core slice.

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const MAX_CSI_PARAMS: usize = 8;

/// Map a C0 control byte (0x00..=0x1f) to the same action Ground emits.
///
/// Most C0 controls (NUL, CAN, SUB, ...) produce no action; the actionable
/// ones are shared between Ground and an in-progress CSI so that a control
/// embedded in a sequence executes without aborting it.
fn c0_action(byte: u8) -> Option<Action> {
    match byte {
        b'\n' | 0x0b | 0x0c => Some(Action::LineFeed),
        b'\r' => Some(Action::CarriageReturn),
        b'\t' => Some(Action::Tab),
        0x08 => Some(Action::Backspace),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Print(u8),
    LineFeed,
    CarriageReturn,
    Backspace,
    Tab,
    Index,
    NextLine,
    ReverseIndex,
    MoveUp(u16),
    MoveDown(u16),
    MoveRight(u16),
    MoveLeft(u16),
    MoveNextLine(u16),
    MovePreviousLine(u16),
    MoveTo {
        row: u16,
        col: u16,
    },
    MoveToColumn(u16),
    MoveToRow(u16),
    SetScrollRegion {
        top: u16,
        bottom: Option<u16>,
    },
    ScrollUp(u16),
    ScrollDown(u16),
    EraseInDisplay(EraseMode),
    EraseInLine(EraseMode),
    EraseCharacters(u16),
    InsertCharacters(u16),
    DeleteCharacters(u16),
    InsertLines(u16),
    DeleteLines(u16),
    SelectGraphicRendition {
        params: [u16; MAX_CSI_PARAMS],
        len: usize,
    },
    SaveCursor,
    RestoreCursor,
    SetKeypadApplication(bool),
    SetPrivateMode {
        mode: PrivateMode,
        enabled: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EraseMode {
    ToEnd,
    ToBeginning,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrivateMode {
    AlternateScreen,
    ApplicationCursorKey,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Parser {
    state: ParserState,
}

impl Parser {
    pub(crate) fn advance(&mut self, byte: u8) -> Option<Action> {
        let state = self.state;
        match state {
            ParserState::Ground => self.advance_ground(byte),
            ParserState::Escape => self.advance_escape(byte),
            ParserState::EscapeIntermediate => {
                self.state = match byte {
                    ESC => ParserState::Escape,
                    0x30..=0x7e => ParserState::Ground,
                    _ => ParserState::EscapeIntermediate,
                };
                None
            }
            ParserState::Csi(mut csi) => {
                if byte == ESC {
                    self.state = ParserState::Escape;
                    return None;
                }
                let action = csi.advance(byte);
                self.state = if action.finished {
                    ParserState::Ground
                } else {
                    ParserState::Csi(csi)
                };
                action.action
            }
            // Control-string payload swallowing. Entered by OSC (`ESC ]`),
            // DCS (`ESC P`), SOS (`ESC X`), PM (`ESC ^`), and APC (`ESC _`).
            // The payload is consumed byte-by-byte and never stored, so the
            // machine stays bounded regardless of payload length. A payload
            // terminates on BEL, or on ST (`ESC \`).
            ParserState::ControlString => {
                self.state = match byte {
                    BEL => ParserState::Ground,
                    ESC => ParserState::ControlStringEscape,
                    _ => ParserState::ControlString,
                };
                None
            }
            // `ESC` seen mid-payload begins ST detection, mirroring OSC. The
            // next byte decides: `\\` completes ST and ends the string; another
            // `ESC` keeps waiting for a terminator (so `ESC ESC \` still ends
            // the string); any other byte returns to payload swallowing. This
            // matches the Williams reference state machine's string-escape
            // transition and keeps the parser bounded.
            ParserState::ControlStringEscape => {
                self.state = match byte {
                    b'\\' => ParserState::Ground,
                    ESC => ParserState::ControlStringEscape,
                    _ => ParserState::ControlString,
                };
                None
            }
        }
    }

    fn advance_ground(&mut self, byte: u8) -> Option<Action> {
        match byte {
            ESC => {
                self.state = ParserState::Escape;
                None
            }
            0x20..=0x7e => Some(Action::Print(byte)),
            _ => c0_action(byte),
        }
    }

    fn advance_escape(&mut self, byte: u8) -> Option<Action> {
        match byte {
            b'[' => self.state = ParserState::Csi(Csi::default()),
            // Control-string introducers all share one swallowing state: OSC
            // (`]`), DCS (`P`), SOS (`X`), PM (`^`), APC (`_`). None of their
            // payload may reach Ground as printable text; ST/BEL ends them.
            b']' | b'P' | b'X' | b'^' | b'_' => self.state = ParserState::ControlString,
            ESC => self.state = ParserState::Escape,
            b'=' => {
                self.state = ParserState::Ground;
                return Some(Action::SetKeypadApplication(true));
            }
            b'>' => {
                self.state = ParserState::Ground;
                return Some(Action::SetKeypadApplication(false));
            }
            b'D' => {
                self.state = ParserState::Ground;
                return Some(Action::Index);
            }
            b'E' => {
                self.state = ParserState::Ground;
                return Some(Action::NextLine);
            }
            b'M' => {
                self.state = ParserState::Ground;
                return Some(Action::ReverseIndex);
            }
            b'7' => {
                self.state = ParserState::Ground;
                return Some(Action::SaveCursor);
            }
            b'8' => {
                self.state = ParserState::Ground;
                return Some(Action::RestoreCursor);
            }
            // Intermediate bytes (0x20..=0x2f) such as `(`, `)`, `#`, and SP
            // begin a multi-byte escape (e.g. SCS `ESC ( B`). Collect them and
            // the following final byte without emitting anything, so the final
            // never leaks to Ground as printable text.
            0x20..=0x2f => self.state = ParserState::EscapeIntermediate,
            _ => self.state = ParserState::Ground,
        }
        None
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ParserState {
    #[default]
    Ground,
    Escape,
    EscapeIntermediate,
    Csi(Csi),
    ControlString,
    ControlStringEscape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Csi {
    params: [u16; MAX_CSI_PARAMS],
    len: usize,
    current: u16,
    has_current: bool,
    overflowed: bool,
    ignored: bool,
    private_marker: Option<u8>,
}

impl Default for Csi {
    fn default() -> Self {
        Self {
            params: [0; MAX_CSI_PARAMS],
            len: 0,
            current: 0,
            has_current: false,
            overflowed: false,
            ignored: false,
            private_marker: None,
        }
    }
}

impl Csi {
    fn advance(&mut self, byte: u8) -> CsiAdvance {
        match byte {
            b'0'..=b'9' => {
                self.has_current = true;
                self.current = self
                    .current
                    .saturating_mul(10)
                    .saturating_add(u16::from(byte - b'0'));
                CsiAdvance::pending()
            }
            b';' => {
                self.push_current();
                CsiAdvance::pending()
            }
            // ECMA-48 private markers occupy 0x3c..=0x3f: `<`, `=`, `>`, `?`.
            // The first one (before any param) seeds the marker; later ones, or
            // any private marker that is not the DEC `?`, must poison the whole
            // sequence so a mangled private CSI never executes its final byte
            // (e.g. SGR-mouse-shaped `CSI < 2 M` must not become DeleteLines).
            b'?' | b'>' | b'<' | b'='
                if self.len == 0 && !self.has_current && self.private_marker.is_none() =>
            {
                self.private_marker = Some(byte);
                CsiAdvance::pending()
            }
            b'?' | b'>' | b'<' | b'=' => {
                self.ignored = true;
                CsiAdvance::pending()
            }
            b':' | 0x20..=0x2f => {
                self.ignored = true;
                CsiAdvance::pending()
            }
            0x40..=0x7e => {
                self.push_current();
                CsiAdvance::finished(if self.overflowed || self.ignored {
                    None
                } else {
                    self.action(byte)
                })
            }
            // C0 controls embedded in a control sequence execute immediately
            // via the Ground action WITHOUT aborting the sequence (DEC VT and
            // xterm). ESC never reaches here: Parser::advance intercepts it
            // first and restarts the escape. CAN/SUB yield no action via
            // c0_action, preserving the prior swallow-and-continue behavior.
            0x00..=0x1f => CsiAdvance::embedded(c0_action(byte)),
            _ => CsiAdvance::pending(),
        }
    }

    fn push_current(&mut self) {
        if self.len < MAX_CSI_PARAMS {
            self.params[self.len] = self.current;
            self.len += 1;
        } else {
            self.overflowed = true;
        }
        self.current = 0;
        self.has_current = false;
    }

    fn action(&self, final_byte: u8) -> Option<Action> {
        match self.private_marker {
            None => self.standard_action(final_byte),
            Some(b'?') => self.private_action(final_byte),
            Some(_) => None,
        }
    }

    fn standard_action(&self, final_byte: u8) -> Option<Action> {
        let count = self.param_or(0, 1);
        match final_byte {
            b'@' if self.len == 1 => Some(Action::InsertCharacters(count)),
            b'A' => Some(Action::MoveUp(count)),
            b'B' => Some(Action::MoveDown(count)),
            b'C' => Some(Action::MoveRight(count)),
            b'D' => Some(Action::MoveLeft(count)),
            b'E' => Some(Action::MoveNextLine(count)),
            b'F' => Some(Action::MovePreviousLine(count)),
            b'G' => Some(Action::MoveToColumn(count.saturating_sub(1))),
            b'H' | b'f' => Some(Action::MoveTo {
                row: self.param_or(0, 1).saturating_sub(1),
                col: self.param_or(1, 1).saturating_sub(1),
            }),
            b'J' => self.erase_mode().map(Action::EraseInDisplay),
            b'K' => self.erase_mode().map(Action::EraseInLine),
            b'L' if self.len == 1 => Some(Action::InsertLines(count)),
            b'P' if self.len == 1 => Some(Action::DeleteCharacters(count)),
            b'S' if self.len <= 1 => Some(Action::ScrollUp(count)),
            b'T' if self.len <= 1 => Some(Action::ScrollDown(count)),
            b'X' if self.len == 1 => Some(Action::EraseCharacters(count)),
            b'd' => Some(Action::MoveToRow(count.saturating_sub(1))),
            b'M' if self.len == 1 => Some(Action::DeleteLines(count)),
            b'r' if self.len <= 2 => Some(Action::SetScrollRegion {
                top: self.param_or(0, 1).saturating_sub(1),
                bottom: self.zero_based_param(1),
            }),
            b'm' => Some(Action::SelectGraphicRendition {
                params: self.params,
                len: self.len,
            }),
            b's' if self.is_default_only() => Some(Action::SaveCursor),
            b'u' if self.is_default_only() => Some(Action::RestoreCursor),
            _ => None,
        }
    }

    fn private_action(&self, final_byte: u8) -> Option<Action> {
        if self.len != 1 {
            return None;
        }
        let mode = match self.params[0] {
            1 => PrivateMode::ApplicationCursorKey,
            1049 => PrivateMode::AlternateScreen,
            _ => return None,
        };
        match final_byte {
            b'h' => Some(Action::SetPrivateMode {
                mode,
                enabled: true,
            }),
            b'l' => Some(Action::SetPrivateMode {
                mode,
                enabled: false,
            }),
            _ => None,
        }
    }

    fn is_default_only(&self) -> bool {
        self.len == 1 && self.params[0] == 0
    }

    fn erase_mode(&self) -> Option<EraseMode> {
        if self.len != 1 {
            return None;
        }
        match self.params[0] {
            0 => Some(EraseMode::ToEnd),
            1 => Some(EraseMode::ToBeginning),
            2 => Some(EraseMode::All),
            _ => None,
        }
    }

    fn param_or(&self, index: usize, default: u16) -> u16 {
        self.params
            .get(index)
            .copied()
            .filter(|value| *value != 0)
            .unwrap_or(default)
    }

    fn zero_based_param(&self, index: usize) -> Option<u16> {
        if index >= self.len {
            return None;
        }
        self.params[index].checked_sub(1)
    }
}

struct CsiAdvance {
    action: Option<Action>,
    finished: bool,
}

impl CsiAdvance {
    const fn pending() -> Self {
        Self {
            action: None,
            finished: false,
        }
    }

    const fn finished(action: Option<Action>) -> Self {
        Self {
            action,
            finished: true,
        }
    }

    const fn embedded(action: Option<Action>) -> Self {
        Self {
            action,
            finished: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actions(bytes: &[u8]) -> Vec<Action> {
        let mut parser = Parser::default();
        bytes
            .iter()
            .filter_map(|byte| parser.advance(*byte))
            .collect()
    }

    #[test]
    fn parses_basic_text_controls_and_cursor_sequences() {
        assert_eq!(
            actions(b"A\n\x0b\x0c\r\x08\x1b[2A\x1b[3;4H"),
            [
                Action::Print(b'A'),
                Action::LineFeed,
                Action::LineFeed,
                Action::LineFeed,
                Action::CarriageReturn,
                Action::Backspace,
                Action::MoveUp(2),
                Action::MoveTo { row: 2, col: 3 },
            ]
        );
    }

    #[test]
    fn horizontal_tab_emits_a_tab_action() {
        assert_eq!(
            actions(b"\ta\tb"),
            [
                Action::Tab,
                Action::Print(b'a'),
                Action::Tab,
                Action::Print(b'b'),
            ]
        );
    }

    #[test]
    fn escape_intermediate_sequences_emit_nothing_and_keep_the_final_byte() {
        // Regression for the byte-leak: `ESC ( B` previously printed 'B'.
        for sequence in [
            b"\x1b(B".as_slice(),
            b"\x1b)0",
            b"\x1b#8",
            b"\x1b F",
            b"\x1b()B",
        ] {
            assert!(actions(sequence).is_empty(), "sequence {sequence:?}");
        }
        // An unsupported single-byte escape final is still consumed whole.
        assert!(actions(b"\x1bc").is_empty());

        // The final byte after an intermediate must not leak when followed by
        // printable text.
        let mut parser = Parser::default();
        let emitted: Vec<Action> = b"\x1b(BX"
            .iter()
            .filter_map(|byte| parser.advance(*byte))
            .collect();
        assert_eq!(emitted, [Action::Print(b'X')]);
    }

    #[test]
    fn escape_intermediate_aborts_on_a_new_escape() {
        assert_eq!(actions(b"\x1b(\x1b[D"), [Action::MoveLeft(1)]);
    }

    #[test]
    fn swallows_unsupported_csi_and_osc_payloads() {
        assert_eq!(actions(b"a\x1b[?2004hb\x1b]0;secret\x07c"), actions(b"abc"));
        assert!(actions(b"\x1b[?2A\x1b[1:2A").is_empty());
    }

    #[test]
    fn escape_restarts_an_incomplete_csi_sequence() {
        assert_eq!(actions(b"\x1b[9\x1b[2A"), [Action::MoveUp(2)]);
    }

    #[test]
    fn embedded_c0_in_csi_executes_without_aborting() {
        // A C0 control inside a CSI executes its Ground action immediately and
        // the sequence keeps collecting parameters (DEC VT / xterm). The LF
        // is no longer swallowed mid-sequence.
        assert_eq!(actions(b"\x1b[\n2A"), [Action::LineFeed, Action::MoveUp(2)]);
        // CR and BS likewise execute mid-sequence without aborting.
        assert_eq!(
            actions(b"\x1b[2\rA"),
            [Action::CarriageReturn, Action::MoveUp(2)]
        );
        // Digits on both sides of an embedded C0 concatenate into one
        // parameter: the execute action does not commit the current param.
        assert_eq!(
            actions(b"\x1b[1\n2A"),
            [Action::LineFeed, Action::MoveUp(12)]
        );
    }

    #[test]
    fn embedded_esc_still_aborts_the_csi() {
        // ESC inside a CSI restarts the escape; the partial sequence is
        // dropped. (ESC is intercepted by Parser::advance before Csi::advance.)
        assert_eq!(actions(b"\x1b[1\x1b[2A"), [Action::MoveUp(2)]);
    }

    #[test]
    fn embedded_can_and_sub_are_unchanged() {
        // CAN (0x18) and SUB (0x1a) emit no action and do not abort the CSI;
        // the sequence runs to completion with its parameters intact, exactly
        // as before the embedded-C0 fix.
        assert_eq!(actions(b"\x1b[2\x18A"), [Action::MoveUp(2)]);
        assert_eq!(actions(b"\x1b[2\x1aA"), [Action::MoveUp(2)]);
    }

    #[test]
    fn parses_index_scroll_region_and_extended_cursor_actions() {
        assert_eq!(
            actions(b"\x1bD\x1bE\x1bM\x1b[2;4r\x1b[3S\x1b[T\x1b[2E\x1b[F\x1b[4d"),
            [
                Action::Index,
                Action::NextLine,
                Action::ReverseIndex,
                Action::SetScrollRegion {
                    top: 1,
                    bottom: Some(3),
                },
                Action::ScrollUp(3),
                Action::ScrollDown(1),
                Action::MoveNextLine(2),
                Action::MovePreviousLine(1),
                Action::MoveToRow(3),
            ]
        );
        assert_eq!(
            actions(b"\x1b[r\x1b[2r"),
            [
                Action::SetScrollRegion {
                    top: 0,
                    bottom: None,
                },
                Action::SetScrollRegion {
                    top: 1,
                    bottom: None,
                },
            ]
        );
    }

    #[test]
    fn parses_cursor_save_restore_and_alternate_screen_mode() {
        assert_eq!(
            actions(b"\x1b7\x1b8\x1b[s\x1b[u\x1b[?1049h\x1b[?1049l"),
            [
                Action::SaveCursor,
                Action::RestoreCursor,
                Action::SaveCursor,
                Action::RestoreCursor,
                Action::SetPrivateMode {
                    mode: PrivateMode::AlternateScreen,
                    enabled: true,
                },
                Action::SetPrivateMode {
                    mode: PrivateMode::AlternateScreen,
                    enabled: false,
                },
            ]
        );
        assert!(actions(b"\x1b[?2004h\x1b[?1049;1h\x1b[>1049h").is_empty());
    }
}
