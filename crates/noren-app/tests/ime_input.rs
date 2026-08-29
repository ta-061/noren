//! Independent integration guards for committed IME input.
//!
//! The binary's winit event adapter is private, so the externally observable
//! byte contract is tested through the public encoder and a real PTY. Small
//! source contracts separately pin the platform-only window calls and the
//! private adapter wiring; those are the two seams a headless integration
//! target cannot invoke through winit itself.

use noren_app::{
    BRACKET_PASTE_BEGIN, BRACKET_PASTE_END, InputMode, KeyDropReason, KeyEncoder, READ_CHUNK_BYTES,
    diagnostics, encode_paste,
};
use noren_pty::{PtyEvent, PtySession, PtySize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const MAIN_SOURCE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs"));
const INPUT_TRANSLATION_SOURCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/input_translation.rs"
));

fn source_function<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("production source no longer contains {signature:?}"));
    let body = &source[start..];
    let open = body
        .find('{')
        .unwrap_or_else(|| panic!("{signature:?} has no function body"));
    let mut depth = 0_usize;
    for (offset, character) in body[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &body[..open + offset + 1];
                }
            }
            _ => {}
        }
    }
    panic!("{signature:?} has an unterminated function body");
}

#[test]
fn multibyte_japanese_commit_is_exact_utf8() {
    assert_eq!(
        KeyEncoder::encode_committed_text("日本語", InputMode::normal()),
        "日本語".as_bytes()
    );
}

#[test]
fn dead_key_composition_is_one_complete_utf8_scalar() {
    assert_eq!(
        KeyEncoder::encode_committed_text("é", InputMode::normal()),
        [0xc3, 0xa9]
    );
}

#[test]
fn empty_commit_encodes_no_pty_input() {
    assert!(KeyEncoder::encode_committed_text("", InputMode::normal()).is_empty());
}

#[test]
fn committed_newline_uses_the_normal_enter_encoding() {
    assert_eq!(
        KeyEncoder::encode_committed_text("\n", InputMode::normal()),
        b"\r"
    );
}

#[test]
fn committed_control_character_uses_the_normal_control_key_encoding() {
    assert_eq!(
        KeyEncoder::encode_committed_text("\u{3}", InputMode::normal()),
        b"\x03"
    );
}

#[test]
fn committed_text_is_not_encoded_as_bracketed_paste() {
    let committed = KeyEncoder::encode_committed_text("日本語", InputMode::normal());
    let pasted = encode_paste("日本語", true).expect("mode 2004 permits a bracketed paste");

    assert_eq!(committed, "日本語".as_bytes());
    assert!(pasted.starts_with(BRACKET_PASTE_BEGIN));
    assert!(pasted.ends_with(BRACKET_PASTE_END));
    for marker in [BRACKET_PASTE_BEGIN, BRACKET_PASTE_END] {
        assert!(
            !committed
                .windows(marker.len())
                .any(|window| window == marker),
            "typed IME input must not contain a bracketed-paste marker"
        );
    }
}

#[test]
fn alternate_screen_mode_does_not_change_committed_text_encoding() {
    let mut terminal = noren_terminal::TerminalState::new(24, 80).expect("valid terminal grid");
    terminal.feed_bytes(b"\x1b[?1049h");
    assert!(terminal.modes().is_alternate_screen_active());

    assert_eq!(
        KeyEncoder::encode_committed_text("日本語", InputMode::normal()),
        "日本語".as_bytes(),
        "screen selection is output state and must not gate typed input"
    );
}

#[test]
fn large_commit_reassembles_in_order_across_every_chunk_boundary() {
    let commit = boundary_distinct_commit();
    let encoded = KeyEncoder::encode_committed_text(&commit, InputMode::normal());
    let chunks: Vec<&[u8]> = encoded.chunks(READ_CHUNK_BYTES).collect();

    assert_eq!(chunks.len(), 3, "fixture must cross two writer boundaries");
    assert_eq!(chunks[0].len(), READ_CHUNK_BYTES);
    assert_eq!(chunks[1].len(), READ_CHUNK_BYTES);
    assert_eq!(chunks.concat(), commit.as_bytes());
    assert_eq!(chunks[0].last(), Some(&b'A'));
    assert_eq!(chunks[1].first(), Some(&0xe6));
    assert_eq!(chunks[2].last(), Some(&b'Z'));
}

#[test]
fn large_encoded_commit_reaches_a_real_pty_in_order() {
    let home = TestHome::new();
    let capture = home.path().join("ime-integration-capture.bin");
    let commit = boundary_distinct_commit();
    let encoded = KeyEncoder::encode_committed_text(&commit, InputMode::normal());
    assert!(encoded.len() > READ_CHUNK_BYTES);

    let mut session = PtySession::spawn_in_home(
        home.path(),
        PtySize::from_raw(24, 80).expect("24x80 is a valid PTY size"),
    )
    .expect("spawn the fixed isolated zsh session");
    wait_for_any_output(&session);

    let command = format!(
        "/bin/stty raw -echo; /bin/dd bs=1 count={} of=\"$HOME/ime-integration-capture.bin\" 2>/dev/null; \
         /bin/stty sane; printf '\\r\\nIME_INTEGRATION_DONE\\r\\n'\r",
        encoded.len()
    );
    session
        .send_input(command.as_bytes())
        .expect("queue the raw capture command");
    for chunk in encoded.chunks(READ_CHUNK_BYTES) {
        session
            .send_input(chunk)
            .expect("queue every encoded commit chunk");
    }

    wait_for_marker(&session, b"IME_INTEGRATION_DONE");
    let received = std::fs::read(&capture).expect("the child wrote its raw input capture");
    assert_eq!(received, encoded);
    session.shutdown().expect("reap the integration PTY child");
}

#[test]
fn production_commit_handler_uses_the_encoder_and_forward_chunk_loop() {
    let handler = source_function(MAIN_SOURCE, "fn handle_ime_window_event");
    assert!(
        handler.contains("KeyEncoder::encode_committed_text(text, self.current_input_mode())"),
        "the private winit adapter must call the normal committed-text encoder"
    );
    assert!(
        handler.contains("for chunk in bytes.chunks(READ_CHUNK_BYTES) {"),
        "large commits must traverse chunks in their original iterator order"
    );
    assert!(
        handler.contains("if !self.send_input(chunk) {") && handler.contains("break;"),
        "every complete chunk must reach the production PTY writer until its first failure"
    );
}

#[test]
fn production_window_initialization_enables_ime_delivery() {
    let initialize = source_function(MAIN_SOURCE, "fn initialize(");
    assert_eq!(
        initialize.matches("window.set_ime_allowed(true);").count(),
        1,
        "winit disables IME events by default; the created window must opt in exactly once"
    );
    let create = initialize
        .find("event_loop.create_window(attributes)")
        .expect("initialize creates the production window");
    let enable = initialize
        .find("window.set_ime_allowed(true);")
        .expect("initialize enables IME delivery");
    assert!(enable > create, "IME must be enabled on the created window");
}

#[test]
fn commit_path_cannot_record_a_drop_while_genuine_drop_recording_remains() {
    let handler = source_function(MAIN_SOURCE, "fn handle_ime_window_event");
    assert!(
        !handler.contains("record_ime_drop"),
        "an accepted Ime::Commit must never increment the drop diagnostic"
    );
    assert!(
        INPUT_TRANSLATION_SOURCE
            .contains("WinitKey::Dead(_) => return Err(KeyDropReason::ImeOrDeadKey)"),
        "an uncomposed physical dead key remains an explicit genuine drop"
    );

    let genuine_drop = KeyDropReason::ImeOrDeadKey;
    let before = diagnostics::ime_drop_count();
    if genuine_drop == KeyDropReason::ImeOrDeadKey {
        diagnostics::record_ime_drop();
    }
    assert_eq!(
        diagnostics::ime_drop_count(),
        before + 1,
        "the payload-free diagnostic must remain reachable for genuine drops"
    );
}

fn boundary_distinct_commit() -> String {
    let mut commit = "A".repeat(READ_CHUNK_BYTES);
    commit.push_str("日本語");
    commit.push_str(&"B".repeat(READ_CHUNK_BYTES - "日本語".len()));
    commit.push('Z');
    commit
}

fn wait_for_any_output(session: &PtySession) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match session.try_recv() {
            Ok(Some(PtyEvent::Output(bytes))) if !bytes.is_empty() => return,
            Ok(Some(_)) | Ok(None) => {}
            Err(error) => panic!("PTY failed before its prompt: {error}"),
        }
        assert!(Instant::now() < deadline, "isolated zsh produced no prompt");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_marker(session: &PtySession, marker: &[u8]) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut output = Vec::new();
    loop {
        match session.try_recv() {
            Ok(Some(PtyEvent::Output(bytes))) => output.extend_from_slice(&bytes),
            Ok(Some(PtyEvent::Error(error))) => panic!("PTY capture failed: {error}"),
            Ok(Some(PtyEvent::Eof | PtyEvent::Exited { .. })) | Ok(None) => {}
            Err(error) => panic!("PTY channel failed: {error}"),
        }
        if output.windows(marker.len()).any(|window| window == marker) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "PTY child never completed the capture; output={:?}",
            String::from_utf8_lossy(&output)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct TestHome(PathBuf);

impl TestHome {
    fn new() -> Self {
        static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
        let sequence = SEQUENCE.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "noren-ime-integration-home-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&path).expect("create isolated integration-test home");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestHome {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove isolated integration-test home");
    }
}
