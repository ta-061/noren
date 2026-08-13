use noren_app::mouse::MouseModes;

/// Passive scanner that observes DECSET (`CSI ? Pn h`) and DECRST
/// (`CSI ? Pn l`) sequences in PTY *output* and updates the app's
/// [`MouseModes`].
///
/// This is a legacy compatibility observer while the application still keeps
/// its encoder-facing [`MouseModes`] separately. `TerminalState::modes()` now
/// exposes the authoritative terminal mode state, so later wiring should read
/// that state and remove this scanner. Until then, this observer consumes no
/// bytes and alters no terminal parsing.
///
/// Cross-chunk boundaries: the DFA retains its state across calls, so a
/// `CSI ? 1000 h` split across two `PtyEvent::Output` chunks is still detected.
#[derive(Default)]
pub(super) struct MouseModeScanner {
    state: ScanState,
    /// Parsed parameter values; supports multi-param sequences
    /// (`CSI ? 1000 ; 1006 h`).
    params: Vec<u16>,
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
enum ScanState {
    #[default]
    Ground,
    Esc,
    Csi,
    /// After `ESC [ ?` or after a `;` separator — expecting the first digit
    /// of the next parameter.
    CsiQuestion,
    /// Accumulating digits of the current parameter.
    Param,
}

impl MouseModeScanner {
    /// Feed one PTY output byte. Updates `modes` when a complete DECSET/DECRST
    /// for a recognized mouse mode is observed.
    fn feed(&mut self, byte: u8, modes: &mut MouseModes) {
        // ESC always starts a fresh sequence regardless of current state.
        if byte == 0x1b {
            self.params.clear();
            self.state = ScanState::Esc;
            return;
        }
        match (self.state, byte) {
            (ScanState::Esc, b'[') => {
                self.state = ScanState::Csi;
            }
            (ScanState::Csi, b'?') => {
                self.params.clear();
                self.state = ScanState::CsiQuestion;
            }
            (ScanState::CsiQuestion, digit @ b'0'..=b'9') => {
                self.params.push(u16::from(digit - b'0'));
                self.state = ScanState::Param;
            }
            (ScanState::Param, digit @ b'0'..=b'9') => {
                if let Some(last) = self.params.last_mut() {
                    *last = last
                        .saturating_mul(10)
                        .saturating_add(u16::from(digit - b'0'));
                }
            }
            (ScanState::Param, b';') => {
                // Multi-parameter: wait for the next digit.
                self.state = ScanState::CsiQuestion;
            }
            (ScanState::Param, b'h') => {
                for &mode in &self.params {
                    *modes = modes.set(mode, true);
                }
                self.params.clear();
                self.state = ScanState::Ground;
            }
            (ScanState::Param, b'l') => {
                for &mode in &self.params {
                    *modes = modes.set(mode, false);
                }
                self.params.clear();
                self.state = ScanState::Ground;
            }
            _ => {
                self.params.clear();
                self.state = ScanState::Ground;
            }
        }
    }

    /// Convenience: feed an entire byte slice.
    pub(super) fn scan(&mut self, bytes: &[u8], modes: &mut MouseModes) {
        for &byte in bytes {
            self.feed(byte, modes);
        }
    }
}
