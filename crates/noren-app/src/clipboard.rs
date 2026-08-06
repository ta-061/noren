//! User-initiated clipboard copy and paste for the macOS PoC.
//!
//! # Security policy (Issue #57)
//!
//! The clipboard is a policy boundary with exactly two user-initiated gates:
//!
//! - **Copy**: the user's grid selection is written to the system clipboard
//!   (`Command+C`).
//! - **Paste**: the system clipboard is read and inserted into the PTY as
//!   input (`Command+V`), bracketed when the application enabled DEC private
//!   mode 2004.
//!
//! Application-driven clipboard access (OSC 52) is explicitly out of scope:
//! the terminal core swallows every OSC payload without acting on it, and this
//! module exposes no path by which terminal output bytes can read or write the
//! clipboard. A future OSC 52 feature must never answer read queries — a
//! program must not be able to exfiltrate the clipboard by asking.

use std::fmt;
use std::io::Write;
use std::process::{Command, Stdio};

/// Bracketed paste begin marker (`CSI 200 ~`), sent before pasted text when
/// the application has enabled DEC private mode 2004.
pub const BRACKET_PASTE_BEGIN: &[u8] = b"\x1b[200~";

/// Bracketed paste end marker (`CSI 201 ~`), sent after pasted text when the
/// application has enabled DEC private mode 2004.
pub const BRACKET_PASTE_END: &[u8] = b"\x1b[201~";

/// Maximum pasted bytes accepted in one user-initiated paste.
///
/// Larger pastes are rejected rather than truncated, so a paste is always
/// either complete or never happens; truncation could split a UTF-8 character
/// or a bracketed-paste marker.
pub const MAX_PASTE_BYTES: usize = 1024 * 1024;

/// Why a user-initiated paste was gated instead of sent to the PTY.
///
/// Noren never pastes unbracketed and never pretends a gated paste succeeded:
/// every rejection leaves the PTY input stream untouched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteReject {
    /// The application has not enabled bracketed paste (DEC private mode
    /// 2004), or the terminal state tracking it is unavailable.
    Unbracketed,
    /// The clipboard held no text at all.
    Empty,
    /// The clipboard text exceeds [`MAX_PASTE_BYTES`].
    Oversized,
}

impl fmt::Display for PasteReject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unbracketed => {
                f.write_str("paste gated: application did not enable bracketed paste (mode 2004)")
            }
            Self::Empty => f.write_str("paste gated: clipboard text is empty"),
            Self::Oversized => f.write_str("paste gated: clipboard text exceeds the paste bound"),
        }
    }
}

impl std::error::Error for PasteReject {}

/// A failure of the system clipboard helper itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardError {
    /// The fixed macOS helper could not be spawned or did not exit cleanly.
    HelperFailed,
    /// The clipboard contents could not be decoded as UTF-8 text.
    NotUtf8,
}

impl fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HelperFailed => f.write_str("macOS clipboard helper failed"),
            Self::NotUtf8 => f.write_str("clipboard contents are not UTF-8 text"),
        }
    }
}

impl std::error::Error for ClipboardError {}

/// macOS system clipboard access via the fixed `/usr/bin/pbcopy` and
/// `/usr/bin/pbpaste` helpers.
///
/// No shell, no interpolation: clipboard contents travel as piped bytes and
/// are data, never authority. Both directions serve user-initiated actions
/// only; nothing here is reachable from PTY output.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClipboard;

impl SystemClipboard {
    /// Construct the clipboard handle.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Write text to the system clipboard.
    ///
    /// The payload is piped verbatim to `/usr/bin/pbcopy`; any spawn, write,
    /// or non-zero exit is a typed failure the caller surfaces visibly.
    pub fn write(&self, text: &str) -> Result<(), ClipboardError> {
        let mut child = Command::new("/usr/bin/pbcopy")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| ClipboardError::HelperFailed)?;
        let written = child
            .stdin
            .as_mut()
            .is_some_and(|stdin| stdin.write_all(text.as_bytes()).is_ok());
        // Close stdin so pbcopy sees EOF before waiting for it.
        drop(child.stdin.take());
        let exited = child.wait().is_ok_and(|status| status.success());
        if written && exited {
            Ok(())
        } else {
            Err(ClipboardError::HelperFailed)
        }
    }

    /// Read the system clipboard as UTF-8 text.
    ///
    /// Non-UTF-8 clipboard contents (files, images, undecodable bytes) are a
    /// typed failure rather than a lossy conversion, so no corrupted bytes can
    /// reach the PTY through a paste.
    pub fn read(&self) -> Result<String, ClipboardError> {
        let output = Command::new("/usr/bin/pbpaste")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .map_err(|_| ClipboardError::HelperFailed)?;
        if !output.status.success() {
            return Err(ClipboardError::HelperFailed);
        }
        String::from_utf8(output.stdout).map_err(|_| ClipboardError::NotUtf8)
    }
}

/// Encode a user-initiated paste for the PTY, or gate it.
///
/// When `bracketed_paste_enabled` (DEC private mode 2004 is set in the
/// terminal state) the text is wrapped in [`BRACKET_PASTE_BEGIN`] /
/// [`BRACKET_PASTE_END`]. When the mode is off the paste is refused
/// unconditionally; an empty or oversized clipboard is refused as well. The
/// `Err` cases carry a payload-free reason so the caller can report the gate
/// instead of silently sending nothing — or worse, unbracketed text.
pub fn encode_paste(text: &str, bracketed_paste_enabled: bool) -> Result<Vec<u8>, PasteReject> {
    if !bracketed_paste_enabled {
        return Err(PasteReject::Unbracketed);
    }
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return Err(PasteReject::Empty);
    }
    if bytes.len() > MAX_PASTE_BYTES {
        return Err(PasteReject::Oversized);
    }
    let mut encoded =
        Vec::with_capacity(BRACKET_PASTE_BEGIN.len() + bytes.len() + BRACKET_PASTE_END.len());
    encoded.extend_from_slice(BRACKET_PASTE_BEGIN);
    encoded.extend_from_slice(bytes);
    encoded.extend_from_slice(BRACKET_PASTE_END);
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_is_wrapped_in_bracketed_markers_when_2004_is_on() {
        let encoded = encode_paste("ls -la\n", true).expect("enabled mode 2004 brackets the paste");
        assert_eq!(encoded, b"\x1b[200~ls -la\n\x1b[201~");
        assert!(encoded.starts_with(BRACKET_PASTE_BEGIN));
        assert!(encoded.ends_with(BRACKET_PASTE_END));
    }

    #[test]
    fn paste_keeps_cjk_and_multibyte_content_byte_exact() {
        let encoded = encode_paste("日本語😀", true).expect("enabled mode 2004");
        assert_eq!(
            encoded,
            [
                BRACKET_PASTE_BEGIN,
                "日本語😀".as_bytes(),
                BRACKET_PASTE_END
            ]
            .concat()
        );
    }

    #[test]
    fn paste_is_gated_not_unbracketed_when_2004_is_off() {
        // The gate holds even for content that would otherwise be pasteable;
        // nothing ever reaches the PTY without the markers.
        assert_eq!(
            encode_paste("ls -la\n", false),
            Err(PasteReject::Unbracketed)
        );
        assert_eq!(encode_paste("", false), Err(PasteReject::Unbracketed));
    }

    #[test]
    fn empty_and_oversized_pastes_are_gated_even_with_2004_on() {
        assert_eq!(encode_paste("", true), Err(PasteReject::Empty));
        let oversized = "a".repeat(MAX_PASTE_BYTES + 1);
        assert_eq!(encode_paste(&oversized, true), Err(PasteReject::Oversized));
        // Exactly at the bound the paste is accepted.
        let at_bound = "a".repeat(MAX_PASTE_BYTES);
        let encoded = encode_paste(&at_bound, true).expect("bound-sized paste fits");
        assert_eq!(
            encoded.len(),
            BRACKET_PASTE_BEGIN.len() + MAX_PASTE_BYTES + BRACKET_PASTE_END.len()
        );
    }

    /// Manual system-clipboard round trip: `cargo test -- --ignored`.
    /// Kept ignored so automated runs never mutate the user's clipboard.
    #[test]
    #[ignore = "touches the real macOS system clipboard"]
    fn system_clipboard_round_trips_user_text() {
        let clipboard = SystemClipboard::new();
        let payload = "noren clipboard round trip 日本";
        clipboard.write(payload).expect("pbcopy writes");
        assert_eq!(clipboard.read().as_deref(), Ok(payload));
    }
}
