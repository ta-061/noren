//! macOS entry point for the Noren local-PTY PoC.
//!
//! This baseline produces the eventual macOS binary target so the workspace
//! links end-to-end. The `winit` event loop, renderer, and PTY supervisor are
//! not part of this baseline; opening a window lands in a later step behind the
//! [`noren_app`] app-owned seams.

fn main() {
    // No window is opened by this baseline. The binary exists so the workspace
    // builds the macOS entry-point target; the event loop is wired later.
}
