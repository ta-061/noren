//! Small, bounded byte parser for the first Terminal Core slice.

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const MAX_CSI_PARAMS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Print(u8),
    LineFeed,
    CarriageReturn,
    Backspace,
    Index,
    NextLine,
    ReverseIndex,
    MoveUp(u16),
    MoveDown(u16),
    MoveRight(u16),
    MoveLeft(u16),
    MoveNextLine(u16),
    MovePreviousLine(u16),
    MoveTo { row: u16, col: u16 },
    MoveToColumn(u16),
    MoveToRow(u16),
    SetScrollRegion { top: u16, bottom: Option<u16> },
    ScrollUp(u16),
    ScrollDown(u16),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Parser {
    state: ParserState,
}

impl Parser {
    pub(crate) fn advance(&mut self, byte: u8) -> Option<Action> {
        let state = std::mem::take(&mut self.state);
        match state {
            ParserState::Ground => self.advance_ground(byte),
            ParserState::Escape => self.advance_escape(byte),
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
            ParserState::Osc => {
                self.state = match byte {
                    BEL => ParserState::Ground,
                    ESC => ParserState::OscEscape,
                    _ => ParserState::Osc,
                };
                None
            }
            ParserState::OscEscape => {
                self.state = match byte {
                    b'\\' => ParserState::Ground,
                    ESC => ParserState::OscEscape,
                    _ => ParserState::Osc,
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
            b'\n' | 0x0b | 0x0c => Some(Action::LineFeed),
            b'\r' => Some(Action::CarriageReturn),
            0x08 => Some(Action::Backspace),
            0x20..=0x7e => Some(Action::Print(byte)),
            _ => None,
        }
    }

    fn advance_escape(&mut self, byte: u8) -> Option<Action> {
        match byte {
            b'[' => self.state = ParserState::Csi(Csi::default()),
            b']' => self.state = ParserState::Osc,
            ESC => self.state = ParserState::Escape,
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
    Csi(Csi),
    Osc,
    OscEscape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Csi {
    params: [u16; MAX_CSI_PARAMS],
    len: usize,
    current: u16,
    has_current: bool,
    overflowed: bool,
    ignored: bool,
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
            b'?' | b'>' if self.len == 0 && !self.has_current => {
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
        let count = self.param_or(0, 1);
        match final_byte {
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
            b'S' if self.len <= 1 => Some(Action::ScrollUp(count)),
            b'T' if self.len <= 1 => Some(Action::ScrollDown(count)),
            b'd' => Some(Action::MoveToRow(count.saturating_sub(1))),
            b'r' if self.len <= 2 => Some(Action::SetScrollRegion {
                top: self.param_or(0, 1).saturating_sub(1),
                bottom: self.zero_based_param(1),
            }),
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
    fn swallows_unsupported_csi_and_osc_payloads() {
        assert_eq!(actions(b"a\x1b[?2004hb\x1b]0;secret\x07c"), actions(b"abc"));
        assert!(actions(b"\x1b[?2A\x1b[1:2A").is_empty());
    }

    #[test]
    fn escape_restarts_an_incomplete_csi_sequence() {
        assert_eq!(actions(b"\x1b[9\x1b[2A"), [Action::MoveUp(2)]);
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
}
