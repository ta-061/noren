//! macOS entry point for the bounded local-zsh PTY PoC.

mod renderer;

use noren_app::{
    Arrow, CursorKeyMode, FunctionKey, GridGeometry, GridSize, InputMode, Key, KeyDropReason,
    KeyEncoder, KeyInput, KeyPhase, KeypadInput, KeypadKey, KeypadMode, MAX_RENDER_ROWS, Modifiers,
    PARSE_BUDGET_BYTES_PER_TURN, POC_CELL_HEIGHT, POC_CELL_WIDTH, PasteReject, Resize,
    SystemClipboard, encode_paste,
};
use noren_pty::{PtyEvent, PtySession, PtySize};
use noren_terminal::{Cell, GridPoint, Selection, SelectionMode, TerminalEngine, TerminalState};
use renderer::{RenderOutcome, Renderer};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WinitKey, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

const WINDOW_WIDTH: u32 = 900;
const WINDOW_HEIGHT: u32 = 600;
const POLL_INTERVAL: Duration = Duration::from_millis(16);

struct NorenApp {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    geometry: GridGeometry,
    pending_grid: Option<GridSize>,
    terminal: Option<TerminalState>,
    pty: Option<PtySession>,
    modifiers: Modifiers,
    status: &'static str,
    show_status: bool,
    redraw_needed: bool,
    // User-initiated selection state. The renderer does not highlight it yet;
    // copy still extracts it. Any PTY output or resize invalidates it because
    // grid coordinates only address the content they were captured on.
    selection: Option<Selection>,
    drag_origin: Option<GridPoint>,
    drag_mode: SelectionMode,
    cursor_position: Option<PhysicalPosition<f64>>,
}

impl Default for NorenApp {
    fn default() -> Self {
        Self {
            window: None,
            renderer: None,
            geometry: GridGeometry::poc(),
            pending_grid: None,
            terminal: None,
            pty: None,
            modifiers: Modifiers::empty(),
            status: "Noren PoC starting",
            show_status: true,
            redraw_needed: true,
            selection: None,
            drag_origin: None,
            drag_mode: SelectionMode::Char,
            cursor_position: None,
        }
    }
}

impl NorenApp {
    fn initialize(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Noren PoC")
            .with_inner_size(PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT));
        let Ok(window) = event_loop.create_window(attributes) else {
            eprintln!("Noren window creation failed");
            event_loop.exit();
            return;
        };
        let window = Arc::new(window);
        let physical = window.inner_size();
        let Some(grid) = self
            .geometry
            .update(Resize::new(physical.width, physical.height))
        else {
            eprintln!("Noren initial window size was invalid");
            event_loop.exit();
            return;
        };

        let Ok(terminal) = TerminalState::new(grid.rows(), grid.cols()) else {
            eprintln!("Noren terminal state creation failed");
            event_loop.exit();
            return;
        };
        self.terminal = Some(terminal);
        self.pty = match pty_size(grid).and_then(PtySession::spawn) {
            Ok(session) => {
                self.status = "Noren PoC ready";
                self.show_status = false;
                Some(session)
            }
            Err(_) => {
                self.status = "Noren PTY start failed";
                self.show_status = true;
                None
            }
        };
        self.renderer = match Renderer::new(Arc::clone(&window)) {
            Ok(renderer) => Some(renderer),
            Err(_) => {
                self.status = "Noren renderer start failed";
                self.show_status = true;
                window.set_title(self.status);
                None
            }
        };
        window.request_redraw();
        self.window = Some(window);
    }

    fn update_modifiers(&mut self, state: ModifiersState) {
        let mut modifiers = Modifiers::empty();
        if state.shift_key() {
            modifiers = modifiers.shift();
        }
        if state.control_key() {
            modifiers = modifiers.ctrl();
        }
        if state.alt_key() {
            modifiers = modifiers.alt();
        }
        if state.super_key() {
            modifiers = modifiers.super_key();
        }
        self.modifiers = modifiers;
    }

    fn handle_key(&mut self, event: &KeyEvent) {
        if self.handle_clipboard_shortcut(event) {
            return;
        }
        let input_mode = self.current_input_mode();
        let encoded = if let Some(input) = translate_keypad_key(event) {
            KeyEncoder::encode_keypad_with(input.with_modifiers(self.modifiers), input_mode)
        } else {
            translate_key(event, self.modifiers)
                .and_then(|input| KeyEncoder::encode_with(input, input_mode))
        };
        let Ok(bytes) = encoded else {
            return;
        };
        self.send_input(&bytes);
    }

    /// User-initiated selection and clipboard shortcuts.
    ///
    /// Cmd+A selects the whole grid, Cmd+C copies the selection to the system
    /// clipboard, and Cmd+V pastes the clipboard into the PTY — but only as a
    /// bracketed paste when the application enabled DEC private mode 2004;
    /// otherwise the paste is gated and reported, never sent unbracketed.
    fn handle_clipboard_shortcut(&mut self, event: &KeyEvent) -> bool {
        if event.state != ElementState::Pressed || event.repeat || !self.modifiers.is_super() {
            return false;
        }
        let WinitKey::Character(text) = &event.logical_key else {
            return false;
        };
        let mut characters = text.chars();
        let Some(character) = characters.next() else {
            return false;
        };
        if characters.next().is_some() {
            return false;
        }
        match character {
            'a' | 'A' => self.select_entire_grid(),
            'c' | 'C' => self.copy_selection(),
            'v' | 'V' => self.paste_clipboard(),
            _ => return false,
        }
        true
    }

    fn select_entire_grid(&mut self) {
        if let Some(terminal) = &self.terminal {
            self.selection = Some(Selection::entire_grid(terminal));
        }
    }

    fn copy_selection(&mut self) {
        let Some(terminal) = &self.terminal else {
            return;
        };
        let Some(selection) = &self.selection else {
            return;
        };
        if !selection.is_valid(terminal) {
            self.selection = None;
            return;
        }
        let text = selection.extract(terminal);
        if text.is_empty() {
            return;
        }
        if SystemClipboard::new().write(&text).is_err() {
            self.status = "Noren clipboard copy failed";
            self.show_status = true;
            self.redraw_needed = true;
        }
    }

    fn paste_clipboard(&mut self) {
        let text = match SystemClipboard::new().read() {
            Ok(text) => text,
            Err(_) => {
                self.status = "Noren clipboard paste failed";
                self.show_status = true;
                self.redraw_needed = true;
                return;
            }
        };
        match self.paste_bytes(&text) {
            Ok(bytes) => self.send_input(&bytes),
            Err(reject @ (PasteReject::Unbracketed | PasteReject::Oversized)) => {
                self.show_paste_gate(reject);
            }
            Err(PasteReject::Empty) => {}
        }
    }

    /// Encode a user-initiated paste against the live terminal mode.
    ///
    /// Returns the bracketed bytes when DEC private mode 2004 is enabled, and a
    /// typed [`PasteReject`] otherwise. Never yields unbracketed bytes: when the
    /// mode is off, or the terminal state is unavailable, the paste is gated.
    fn paste_bytes(&self, text: &str) -> Result<Vec<u8>, PasteReject> {
        let bracketed = self
            .terminal
            .as_ref()
            .is_some_and(|terminal| terminal.modes().is_bracketed_paste_enabled());
        encode_paste(text, bracketed)
    }

    /// Surface a gated paste visibly instead of sending nothing silently.
    fn show_paste_gate(&mut self, reject: PasteReject) {
        // Status is a &'static str, so map the typed reason to fixed text.
        self.status = match reject {
            PasteReject::Unbracketed => {
                "Noren paste gated: application did not enable bracketed paste (mode 2004)"
            }
            PasteReject::Oversized => "Noren paste gated: clipboard text exceeds the paste bound",
            PasteReject::Empty => "Noren paste gated: clipboard text is empty",
        };
        self.show_status = true;
        self.redraw_needed = true;
    }

    fn handle_mouse_move(&mut self, position: PhysicalPosition<f64>) {
        self.cursor_position = Some(position);
        let Some(origin) = self.drag_origin else {
            return;
        };
        let Some(point) = self.grid_point_at(position) else {
            return;
        };
        if let Some(terminal) = &self.terminal {
            self.selection = Some(Selection::new(terminal, self.drag_mode, origin, point));
        }
    }

    fn handle_mouse_button(&mut self, state: ElementState, button: MouseButton) {
        if button != MouseButton::Left {
            return;
        }
        match state {
            ElementState::Pressed => {
                let Some(position) = self.cursor_position else {
                    return;
                };
                let Some(point) = self.grid_point_at(position) else {
                    return;
                };
                let Some(terminal) = &self.terminal else {
                    return;
                };
                // Option-drag selects word-wise, Cmd-drag line-wise.
                let mode = if self.modifiers.is_alt() {
                    SelectionMode::Word
                } else if self.modifiers.is_super() {
                    SelectionMode::Line
                } else {
                    SelectionMode::Char
                };
                self.drag_mode = mode;
                self.drag_origin = Some(point);
                self.selection = Some(Selection::new(terminal, mode, point, point));
            }
            ElementState::Released => {
                self.drag_origin = None;
            }
        }
    }

    /// Map a window pixel position to grid coordinates, mirroring the
    /// renderer's bottom-aligned layout of the trimmed visible lines and the
    /// optional status row. Returns `None` outside the rendered content.
    fn grid_point_at(&self, position: PhysicalPosition<f64>) -> Option<GridPoint> {
        if position.x < 0.0 || position.y < 0.0 {
            return None;
        }
        let terminal = self.terminal.as_ref()?;
        let window = self.window.as_ref()?;
        let physical = window.inner_size();
        let visible_rows = usize::try_from(physical.height / POC_CELL_HEIGHT)
            .unwrap_or(0)
            .clamp(1, usize::from(MAX_RENDER_ROWS));
        let content_rows = visible_content_rows(terminal);
        let total_lines = content_rows + usize::from(self.show_status);
        let displayed = total_lines.min(visible_rows);
        let top_blank_rows = visible_rows - displayed;
        let first_line = total_lines - displayed;
        let row = pixel_row_index(position.y, POC_CELL_HEIGHT)?;
        if row < top_blank_rows {
            return None;
        }
        let line_index = first_line + (row - top_blank_rows);
        if line_index >= content_rows {
            return None;
        }
        let (rows, cols) = terminal.size();
        if line_index >= usize::from(rows) {
            return None;
        }
        let column = pixel_row_index(position.x, POC_CELL_WIDTH)?.min(usize::from(cols) - 1);
        Some(GridPoint::new(
            terminal.scrollback_len() + line_index,
            column,
        ))
    }

    fn send_input(&mut self, bytes: &[u8]) {
        if let Some(session) = &self.pty {
            if session.send_input(bytes).is_err() {
                self.status = "Noren PTY input failed";
                self.show_status = true;
                self.redraw_needed = true;
            }
        }
    }

    fn current_input_mode(&self) -> InputMode {
        let Some(modes) = self.terminal.as_ref().map(TerminalState::modes) else {
            return InputMode::normal();
        };
        let cursor_mode = if modes.is_application_cursor_key_mode() {
            CursorKeyMode::Application
        } else {
            CursorKeyMode::Normal
        };
        let keypad_mode = if modes.is_application_keypad_mode() {
            KeypadMode::Application
        } else {
            KeypadMode::Numeric
        };
        InputMode::normal()
            .with_cursor(cursor_mode)
            .with_keypad(keypad_mode)
    }

    fn handle_resize(&mut self, physical: PhysicalSize<u32>) {
        if let Some(renderer) = &mut self.renderer {
            renderer.resize(physical);
        }
        if let Some(grid) = self
            .geometry
            .update(Resize::new(physical.width, physical.height))
        {
            self.pending_grid = Some(grid);
        }
        self.redraw_needed = true;
    }

    fn apply_pending_resize(&mut self) {
        let Some(grid) = self.pending_grid.take() else {
            return;
        };
        // Resize re-addresses the grid, so captured coordinates expire.
        self.selection = None;
        self.drag_origin = None;
        if let Some(terminal) = &mut self.terminal {
            if terminal.resize(grid.rows(), grid.cols()).is_err() {
                self.status = "Noren terminal resize failed";
                self.show_status = true;
            }
        }
        if let (Some(session), Ok(size)) = (&self.pty, pty_size(grid)) {
            if session.resize(size).is_err() {
                self.status = "Noren PTY resize failed";
                self.show_status = true;
            }
        }
        self.redraw_needed = true;
    }

    fn drain_pty(&mut self) {
        let mut remaining = PARSE_BUDGET_BYTES_PER_TURN;
        let mut terminal_status = None;
        let mut output_consumed = false;
        while remaining >= noren_pty::READ_CHUNK_BYTES {
            let event = match self.pty.as_ref().map(PtySession::try_recv) {
                Some(Ok(Some(event))) => event,
                Some(Ok(None)) | None => break,
                Some(Err(_)) => {
                    terminal_status = Some("Noren PTY channel closed");
                    break;
                }
            };

            match event {
                PtyEvent::Output(bytes) => {
                    if bytes.len() > remaining {
                        self.status = "Noren PTY parse budget exceeded";
                        self.show_status = true;
                        break;
                    }
                    remaining -= bytes.len();
                    if let Some(terminal) = &mut self.terminal {
                        terminal.feed_bytes(&bytes);
                    }
                    output_consumed = true;
                    self.redraw_needed = true;
                }
                PtyEvent::Eof => {
                    terminal_status = Some("Noren shell reached EOF");
                    break;
                }
                PtyEvent::Exited { code } => {
                    terminal_status = Some(if code == Some(0) {
                        "Noren shell exited"
                    } else {
                        "Noren shell exited with failure"
                    });
                    break;
                }
                PtyEvent::Error(_) => {
                    terminal_status = Some("Noren PTY operation failed");
                    break;
                }
            }
        }
        // Any output may have moved or overwritten the selected content; the
        // selection model treats every state change as expiration, so the app
        // drops captured coordinates rather than risk stale text.
        if output_consumed {
            self.selection = None;
            self.drag_origin = None;
        }
        if let Some(status) = terminal_status {
            self.finish_pty(status);
        }
    }

    // This one-session PoC preserves the final frame and status until the user
    // closes the window; it has no inactive-session input or restart path.
    fn finish_pty(&mut self, status: &'static str) {
        self.status = status;
        self.show_status = true;
        self.redraw_needed = true;
        if let Some(mut session) = self.pty.take()
            && session.shutdown().is_err()
        {
            self.status = "Noren PTY shutdown failed";
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let snapshot = self.terminal.as_ref().map(TerminalEngine::snapshot);
        let status = if self.show_status
            || snapshot
                .as_ref()
                .is_none_or(|snapshot| snapshot.lines().is_empty())
        {
            Some(self.status)
        } else {
            None
        };
        let outcome = self
            .renderer
            .as_mut()
            .map(|renderer| renderer.render(snapshot.as_ref(), status));
        match outcome {
            Some(RenderOutcome::DeviceLost) => {
                self.status = "Noren renderer device lost";
                self.show_status = true;
                self.close(event_loop);
            }
            Some(RenderOutcome::Reconfigured) => {
                self.redraw_needed = true;
            }
            Some(RenderOutcome::Presented | RenderOutcome::Skipped) | None => {}
        }
    }

    fn close(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(mut session) = self.pty.take() {
            if session.shutdown().is_err() {
                eprintln!("Noren PTY shutdown reached its failure fallback");
            }
        }
        event_loop.exit();
    }
}

impl ApplicationHandler for NorenApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.initialize(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window.as_ref().map(|window| window.id()) != Some(window_id) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => self.close(event_loop),
            WindowEvent::Resized(physical) => self.handle_resize(physical),
            WindowEvent::ModifiersChanged(modifiers) => self.update_modifiers(modifiers.state()),
            WindowEvent::CursorMoved { position, .. } => self.handle_mouse_move(position),
            WindowEvent::MouseInput { state, button, .. } => {
                self.handle_mouse_button(state, button)
            }
            WindowEvent::KeyboardInput { event, .. } => self.handle_key(&event),
            WindowEvent::Ime(_) => {
                let _ = KeyDropReason::ImeOrDeadKey;
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.apply_pending_resize();
        self.drain_pty();
        if self.redraw_needed {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            self.redraw_needed = false;
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + POLL_INTERVAL));
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(mut session) = self.pty.take() {
            let _ = session.shutdown();
        }
    }
}

fn pty_size(grid: GridSize) -> Result<PtySize, noren_pty::PtyError> {
    PtySize::from_raw(grid.rows(), grid.cols()).ok_or(noren_pty::PtyError::InvalidSize)
}

/// Index of the cell row containing a non-negative pixel coordinate, or
/// `None` when the coordinate is not finite. The cast saturates on overflow,
/// and downstream clamping keeps any saturated index inside the grid.
fn pixel_row_index(pixel: f64, cell_size: u32) -> Option<usize> {
    if !pixel.is_finite() {
        return None;
    }
    Some((pixel / f64::from(cell_size)) as usize)
}

/// Number of visible grid rows the renderer will draw: rows up to and
/// including the last row with non-blank content. Mirrors the snapshot
/// `lines` trimming without cloning the grid (or the scrollback), so mouse
/// mapping never pays for an immutable snapshot per event.
fn visible_content_rows(terminal: &TerminalState) -> usize {
    let screen = terminal.screen();
    let cols = usize::from(screen.cols());
    let cells = screen.cells();
    (0..usize::from(screen.rows()))
        .filter(|row| {
            !cells[row * cols..(row + 1) * cols]
                .iter()
                .all(Cell::is_blank)
        })
        .next_back()
        .map_or(0, |row| row + 1)
}

fn translate_key(event: &KeyEvent, modifiers: Modifiers) -> Result<KeyInput, KeyDropReason> {
    translate_logical_key(&event.logical_key, key_phase(event), modifiers)
}

fn key_phase(event: &KeyEvent) -> KeyPhase {
    match event.state {
        ElementState::Released => KeyPhase::Released,
        ElementState::Pressed if event.repeat => KeyPhase::Repeat,
        ElementState::Pressed => KeyPhase::Pressed,
    }
}

fn translate_keypad_key(event: &KeyEvent) -> Option<KeypadInput> {
    keypad_key(event.physical_key).map(|key| KeypadInput::new(key, key_phase(event)))
}

fn keypad_key(physical_key: PhysicalKey) -> Option<KeypadKey> {
    Some(match physical_key {
        PhysicalKey::Code(KeyCode::Numpad0) => KeypadKey::Zero,
        PhysicalKey::Code(KeyCode::Numpad1) => KeypadKey::One,
        PhysicalKey::Code(KeyCode::Numpad2) => KeypadKey::Two,
        PhysicalKey::Code(KeyCode::Numpad3) => KeypadKey::Three,
        PhysicalKey::Code(KeyCode::Numpad4) => KeypadKey::Four,
        PhysicalKey::Code(KeyCode::Numpad5) => KeypadKey::Five,
        PhysicalKey::Code(KeyCode::Numpad6) => KeypadKey::Six,
        PhysicalKey::Code(KeyCode::Numpad7) => KeypadKey::Seven,
        PhysicalKey::Code(KeyCode::Numpad8) => KeypadKey::Eight,
        PhysicalKey::Code(KeyCode::Numpad9) => KeypadKey::Nine,
        PhysicalKey::Code(KeyCode::NumpadDecimal) => KeypadKey::Decimal,
        PhysicalKey::Code(KeyCode::NumpadAdd) => KeypadKey::Plus,
        PhysicalKey::Code(KeyCode::NumpadSubtract) => KeypadKey::Minus,
        PhysicalKey::Code(KeyCode::NumpadMultiply) => KeypadKey::Star,
        PhysicalKey::Code(KeyCode::NumpadDivide) => KeypadKey::Slash,
        PhysicalKey::Code(KeyCode::NumpadEnter) => KeypadKey::Enter,
        _ => return None,
    })
}

fn translate_logical_key(
    logical_key: &WinitKey,
    phase: KeyPhase,
    modifiers: Modifiers,
) -> Result<KeyInput, KeyDropReason> {
    let key = match logical_key {
        WinitKey::Character(text) => {
            let mut characters = text.chars();
            let character = characters.next().ok_or(KeyDropReason::UnsupportedKey)?;
            if characters.next().is_some() {
                return Err(KeyDropReason::ImeOrDeadKey);
            }
            Key::Character(character)
        }
        WinitKey::Named(NamedKey::Enter) => Key::Enter,
        WinitKey::Named(NamedKey::Backspace) => Key::Backspace,
        WinitKey::Named(NamedKey::Tab) => Key::Tab,
        WinitKey::Named(NamedKey::Escape) => Key::Escape,
        WinitKey::Named(NamedKey::Space) => Key::Character(' '),
        WinitKey::Named(NamedKey::ArrowUp) => Key::Arrow(Arrow::Up),
        WinitKey::Named(NamedKey::ArrowDown) => Key::Arrow(Arrow::Down),
        WinitKey::Named(NamedKey::ArrowLeft) => Key::Arrow(Arrow::Left),
        WinitKey::Named(NamedKey::ArrowRight) => Key::Arrow(Arrow::Right),
        WinitKey::Named(NamedKey::Delete) => Key::Delete,
        WinitKey::Named(NamedKey::Insert) => Key::Insert,
        WinitKey::Named(NamedKey::Home) => Key::Home,
        WinitKey::Named(NamedKey::End) => Key::End,
        WinitKey::Named(NamedKey::PageUp) => Key::PageUp,
        WinitKey::Named(NamedKey::PageDown) => Key::PageDown,
        WinitKey::Named(NamedKey::F1) => Key::Function(FunctionKey::F1),
        WinitKey::Named(NamedKey::F2) => Key::Function(FunctionKey::F2),
        WinitKey::Named(NamedKey::F3) => Key::Function(FunctionKey::F3),
        WinitKey::Named(NamedKey::F4) => Key::Function(FunctionKey::F4),
        WinitKey::Named(NamedKey::F5) => Key::Function(FunctionKey::F5),
        WinitKey::Named(NamedKey::F6) => Key::Function(FunctionKey::F6),
        WinitKey::Named(NamedKey::F7) => Key::Function(FunctionKey::F7),
        WinitKey::Named(NamedKey::F8) => Key::Function(FunctionKey::F8),
        WinitKey::Named(NamedKey::F9) => Key::Function(FunctionKey::F9),
        WinitKey::Named(NamedKey::F10) => Key::Function(FunctionKey::F10),
        WinitKey::Named(NamedKey::F11) => Key::Function(FunctionKey::F11),
        WinitKey::Named(NamedKey::F12) => Key::Function(FunctionKey::F12),
        WinitKey::Dead(_) => return Err(KeyDropReason::ImeOrDeadKey),
        _ => return Err(KeyDropReason::UnsupportedKey),
    };
    Ok(KeyInput::new(key, phase, modifiers))
}

fn main() {
    let Ok(event_loop) = EventLoop::new() else {
        eprintln!("Noren event loop creation failed");
        return;
    };
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = NorenApp::default();
    if event_loop.run_app(&mut app).is_err() {
        eprintln!("Noren event loop failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winit_space_variants_encode_ascii_space() {
        let variants = [
            WinitKey::Named(NamedKey::Space),
            WinitKey::Character(" ".into()),
        ];
        for logical_key in variants {
            let input = translate_logical_key(&logical_key, KeyPhase::Pressed, Modifiers::empty())
                .expect("space is supported terminal input");
            assert_eq!(KeyEncoder::encode(input), Ok(vec![0x20]));
        }
    }

    #[test]
    fn terminal_modes_drive_cursor_and_keypad_encoding() {
        let mut app = NorenApp::default();
        assert_eq!(app.current_input_mode(), InputMode::normal());

        let mut terminal = TerminalState::new(2, 4).expect("valid terminal");
        terminal.feed_bytes(b"\x1b[?1h\x1b=");
        app.terminal = Some(terminal);
        let mode = app.current_input_mode();

        let arrow = KeyInput::new(Key::Arrow(Arrow::Up), KeyPhase::Pressed, Modifiers::empty());
        assert_eq!(
            KeyEncoder::encode_with(arrow, mode).as_deref(),
            Ok(b"\x1bOA".as_slice())
        );
        assert_eq!(
            KeyEncoder::encode_keypad_with(
                KeypadInput::new(KeypadKey::One, KeyPhase::Pressed),
                mode
            )
            .as_deref(),
            Ok(b"\x1bOq".as_slice())
        );
    }

    #[test]
    fn physical_keypad_mapping_is_bounded_to_numpad_codes() {
        let cases = [
            (KeyCode::Numpad0, KeypadKey::Zero),
            (KeyCode::Numpad1, KeypadKey::One),
            (KeyCode::Numpad2, KeypadKey::Two),
            (KeyCode::Numpad3, KeypadKey::Three),
            (KeyCode::Numpad4, KeypadKey::Four),
            (KeyCode::Numpad5, KeypadKey::Five),
            (KeyCode::Numpad6, KeypadKey::Six),
            (KeyCode::Numpad7, KeypadKey::Seven),
            (KeyCode::Numpad8, KeypadKey::Eight),
            (KeyCode::Numpad9, KeypadKey::Nine),
            (KeyCode::NumpadDecimal, KeypadKey::Decimal),
            (KeyCode::NumpadAdd, KeypadKey::Plus),
            (KeyCode::NumpadSubtract, KeypadKey::Minus),
            (KeyCode::NumpadMultiply, KeypadKey::Star),
            (KeyCode::NumpadDivide, KeypadKey::Slash),
            (KeyCode::NumpadEnter, KeypadKey::Enter),
        ];
        for (code, expected) in cases {
            assert_eq!(keypad_key(PhysicalKey::Code(code)), Some(expected));
        }
        assert_eq!(keypad_key(PhysicalKey::Code(KeyCode::Digit1)), None);
    }

    #[test]
    fn navigation_and_function_named_keys_translate_to_app_keys() {
        let cases = [
            (NamedKey::Delete, Key::Delete),
            (NamedKey::Insert, Key::Insert),
            (NamedKey::Home, Key::Home),
            (NamedKey::End, Key::End),
            (NamedKey::PageUp, Key::PageUp),
            (NamedKey::PageDown, Key::PageDown),
            (NamedKey::F1, Key::Function(FunctionKey::F1)),
            (NamedKey::F2, Key::Function(FunctionKey::F2)),
            (NamedKey::F3, Key::Function(FunctionKey::F3)),
            (NamedKey::F4, Key::Function(FunctionKey::F4)),
            (NamedKey::F5, Key::Function(FunctionKey::F5)),
            (NamedKey::F6, Key::Function(FunctionKey::F6)),
            (NamedKey::F7, Key::Function(FunctionKey::F7)),
            (NamedKey::F8, Key::Function(FunctionKey::F8)),
            (NamedKey::F9, Key::Function(FunctionKey::F9)),
            (NamedKey::F10, Key::Function(FunctionKey::F10)),
            (NamedKey::F11, Key::Function(FunctionKey::F11)),
            (NamedKey::F12, Key::Function(FunctionKey::F12)),
        ];
        for (named, expected) in cases {
            let logical_key = WinitKey::Named(named);
            let input = translate_logical_key(&logical_key, KeyPhase::Pressed, Modifiers::empty())
                .expect("stage one key is supported terminal input");
            assert_eq!(input.key(), expected);
            assert_eq!(input.phase(), KeyPhase::Pressed);
        }
    }

    #[test]
    fn untranslated_named_keys_still_report_a_drop() {
        for named in [NamedKey::F13, NamedKey::ScrollLock, NamedKey::Pause] {
            let logical_key = WinitKey::Named(named);
            assert_eq!(
                translate_logical_key(&logical_key, KeyPhase::Pressed, Modifiers::empty()),
                Err(KeyDropReason::UnsupportedKey)
            );
        }
    }

    #[test]
    fn pixel_row_index_truncates_and_rejects_non_finite() {
        assert_eq!(pixel_row_index(0.0, 20), Some(0));
        assert_eq!(pixel_row_index(39.0, 20), Some(1));
        assert_eq!(pixel_row_index(40.0, 20), Some(2));
        assert_eq!(pixel_row_index(f64::NAN, 20), None);
        assert_eq!(pixel_row_index(f64::INFINITY, 20), None);
    }

    #[test]
    fn visible_content_rows_counts_through_the_last_non_blank_row() {
        let mut terminal = TerminalState::new(4, 8).expect("valid terminal");
        terminal.feed_bytes(b"ab\r\ncd");
        assert_eq!(visible_content_rows(&terminal), 2);

        terminal.feed_bytes(b"\r\n\r\nef");
        assert_eq!(visible_content_rows(&terminal), 4);
    }

    #[test]
    fn paste_is_gated_in_the_app_without_a_terminal() {
        // With no terminal state, mode 2004 is unavailable, so encode_paste
        // gates rather than emitting an unbracketed paste.
        assert_eq!(encode_paste("hello", false), Err(PasteReject::Unbracketed));
    }

    #[test]
    fn paste_is_bracketed_when_mode_2004_is_enabled() {
        let mut app = NorenApp::default();
        let mut terminal = TerminalState::new(2, 4).expect("valid terminal");
        terminal.feed_bytes(b"\x1b[?2004h");
        app.terminal = Some(terminal);

        assert_eq!(
            app.paste_bytes("ls -la"),
            Ok(b"\x1b[200~ls -la\x1b[201~".to_vec())
        );
    }

    #[test]
    fn paste_is_gated_when_mode_2004_is_off_or_terminal_unavailable() {
        let mut app = NorenApp::default();
        // No terminal state at all: bracketed paste cannot be enabled.
        assert_eq!(app.paste_bytes("ls"), Err(PasteReject::Unbracketed));

        // Terminal state present but the application never enabled 2004.
        let terminal = TerminalState::new(2, 4).expect("valid terminal");
        app.terminal = Some(terminal);
        assert_eq!(app.paste_bytes("ls"), Err(PasteReject::Unbracketed));
    }

    #[test]
    fn copy_selection_drops_an_expired_selection_without_copying() {
        let mut app = NorenApp::default();
        let mut terminal = TerminalState::new(2, 6).expect("valid terminal");
        terminal.feed_bytes(b"hello");
        app.selection = Some(Selection::new(
            &terminal,
            SelectionMode::Char,
            GridPoint::new(0, 0),
            GridPoint::new(0, 4),
        ));
        terminal.resize(3, 8).expect("valid resize");
        app.terminal = Some(terminal);

        // The resize expired the selection's stamp; copy clears the selection
        // and returns before any system clipboard access.
        app.copy_selection();
        assert!(app.selection.is_none());
    }

    #[test]
    fn select_entire_grid_captures_all_visible_content() {
        let mut app = NorenApp::default();
        let mut terminal = TerminalState::new(3, 6).expect("valid terminal");
        terminal.feed_bytes(b"abc\r\ndef");
        app.terminal = Some(terminal);

        app.select_entire_grid();
        let terminal = app.terminal.as_ref().expect("terminal present");
        assert_eq!(
            app.selection
                .as_ref()
                .map(|selection| selection.extract(terminal)),
            Some("abc\ndef".to_owned())
        );
    }

    #[test]
    fn terminal_event_finishes_the_session_without_closing_the_window() {
        let mut app = NorenApp::default();
        app.finish_pty("Noren shell reached EOF");

        assert!(app.pty.is_none());
        assert_eq!(app.status, "Noren shell reached EOF");
        assert!(app.show_status);
        assert!(app.redraw_needed);
    }
}
