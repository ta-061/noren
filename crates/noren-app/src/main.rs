//! macOS entry point for the bounded local-zsh PTY PoC.

mod renderer;

use noren_app::{
    Arrow, CursorKeyMode, FunctionKey, GridGeometry, GridSize, InputMode, Key, KeyDropReason,
    KeyEncoder, KeyInput, KeyPhase, KeypadInput, KeypadKey, KeypadMode, MAX_RENDER_COLS,
    MAX_RENDER_ROWS, Modifiers, PARSE_BUDGET_BYTES_PER_TURN, POC_CELL_HEIGHT, POC_CELL_WIDTH,
    PasteReject, Resize, SystemClipboard,
    config::AppConfig,
    diagnostics::{self, PtyChildStatus},
    encode_paste,
    palette::Palette,
    session::{SessionAction, SessionError, SessionEvent, SessionId, SessionKind, SessionRegistry},
    sidebar::{SidebarEntry, SidebarView},
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

/// The dispatchable intent behind each palette command.
///
/// The palette module is action-agnostic by design ([`Palette`] is generic
/// over `A`); this enum binds the four canonical Noren commands to workspace
/// intents without introducing a parallel vocabulary. Select and close need
/// a target session resolved by the UI layer (step 2); this step carries the
/// intent only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkspaceAction {
    /// Create a new local terminal session.
    CreateSession,
    /// Begin selecting a session (the UI resolves which).
    SelectSession,
    /// Begin closing a session (the UI resolves which).
    CloseSession,
    /// Focus the sidebar.
    FocusSidebar,
}

/// Application workspace state: owns the session registry, the sidebar view
/// derived from it, and the command palette.
///
/// Every mutation — create, select, close — routes through
/// [`SessionRegistry::apply`] and then rebuilds the sidebar from the
/// registry's current sessions and selection, so the view and the model can
/// never disagree.
struct WorkspaceState {
    registry: SessionRegistry,
    sidebar: SidebarView,
    /// Owned by the workspace; dispatched in step 2 (palette UI).
    #[allow(dead_code)]
    palette: Palette<WorkspaceAction>,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceState {
    /// Empty workspace: no sessions, empty sidebar, the canonical palette.
    fn new() -> Self {
        Self {
            registry: SessionRegistry::new(),
            sidebar: SidebarView::build(&[], None),
            palette: Palette::noren(
                WorkspaceAction::CreateSession,
                WorkspaceAction::SelectSession,
                WorkspaceAction::CloseSession,
                WorkspaceAction::FocusSidebar,
            ),
        }
    }

    /// Create a new session and rebuild the sidebar.
    ///
    /// Creation is infallible: the registry mints a fresh id and accepts every
    /// [`SessionKind`]. The new session starts at `Starting` status; advancing
    /// it to `Running` is the supervisor's job (a later step).
    fn create_session(&mut self, kind: SessionKind) -> SessionId {
        let events = self
            .registry
            .apply(SessionAction::Create { kind })
            .expect("SessionAction::Create is infallible");
        self.rebuild_sidebar();
        created_session_id(events)
    }

    /// Select a session by id and rebuild the sidebar.
    ///
    /// A stale id returns [`SessionError::UnknownSession`] without mutating the
    /// view — the registry did not change, so the sidebar is still correct.
    fn select_session(&mut self, id: SessionId) -> Result<(), SessionError> {
        self.registry.apply(SessionAction::Select { id })?;
        self.rebuild_sidebar();
        Ok(())
    }

    /// Close a session by id and rebuild the sidebar.
    ///
    /// Closing the selected session clears the selection (the registry handles
    /// this), so the rebuilt sidebar shows no selection and no viewport.
    fn close_session(&mut self, id: SessionId) -> Result<(), SessionError> {
        self.registry.apply(SessionAction::Close { id })?;
        self.rebuild_sidebar();
        Ok(())
    }

    /// Rebuild the sidebar from the registry's current sessions and selection.
    ///
    /// Called after every mutation so the view never lags the model.
    fn rebuild_sidebar(&mut self) {
        let entries: Vec<SidebarEntry> = self
            .registry
            .sessions()
            .into_iter()
            .map(SidebarEntry::Session)
            .collect();
        self.sidebar = SidebarView::build(&entries, self.registry.selected());
    }

    /// The current sidebar view (immutable snapshot for the renderer).
    fn sidebar(&self) -> &SidebarView {
        &self.sidebar
    }

    /// The command palette.
    #[allow(dead_code)]
    fn palette(&self) -> &Palette<WorkspaceAction> {
        &self.palette
    }

    /// The session registry.
    #[allow(dead_code)]
    fn registry(&self) -> &SessionRegistry {
        &self.registry
    }
}

/// Extract the created session id from the events emitted by a `Create` action.
fn created_session_id(events: Vec<SessionEvent>) -> SessionId {
    events
        .into_iter()
        .find_map(|event| match event {
            SessionEvent::Created(id) => Some(id),
            _ => None,
        })
        .expect("SessionAction::Create yields exactly one Created event")
}

struct NorenApp {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    geometry: GridGeometry,
    pending_grid: Option<GridSize>,
    terminal: Option<TerminalState>,
    pty: Option<PtySession>,
    pty_child: PtyChildStatus,
    modifiers: Modifiers,
    status: &'static str,
    show_status: bool,
    diagnostics_visible: bool,
    diagnostics_line: String,
    redraw_needed: bool,
    // User-initiated selection state. The renderer does not highlight it yet;
    // copy still extracts it. Any PTY output or resize invalidates it because
    // grid coordinates only address the content they were captured on.
    selection: Option<Selection>,
    drag_origin: Option<GridPoint>,
    drag_mode: SelectionMode,
    cursor_position: Option<PhysicalPosition<f64>>,
    workspace: WorkspaceState,
    active_session: Option<SessionId>,
}

impl Default for NorenApp {
    fn default() -> Self {
        Self::new(AppConfig::default())
    }
}

impl NorenApp {
    fn new(config: AppConfig) -> Self {
        // Configuration is already range-checked; the fallback only guards
        // the programmatic constructor path.
        let geometry =
            GridGeometry::with_cells(config.font().cell_width(), config.font().cell_height())
                .unwrap_or_else(GridGeometry::poc);
        Self {
            window: None,
            renderer: None,
            geometry,
            pending_grid: None,
            terminal: None,
            pty: None,
            pty_child: PtyChildStatus::NotLaunched,
            modifiers: Modifiers::empty(),
            status: "Noren PoC starting",
            show_status: true,
            diagnostics_visible: false,
            diagnostics_line: String::new(),
            redraw_needed: true,
            selection: None,
            drag_origin: None,
            drag_mode: SelectionMode::Char,
            cursor_position: None,
            workspace: WorkspaceState::new(),
            active_session: None,
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

        let Ok(terminal) = TerminalState::new(grid.rows(), terminal_cols(grid.cols())) else {
            eprintln!("Noren terminal state creation failed");
            event_loop.exit();
            return;
        };
        self.terminal = Some(terminal);
        self.pty =
            match pty_size(grid.rows(), terminal_cols(grid.cols())).and_then(PtySession::spawn) {
                Ok(session) => {
                    self.status = "Noren PoC ready";
                    self.show_status = false;
                    self.pty_child = PtyChildStatus::Running;
                    let session_id = self.workspace.create_session(SessionKind::Local);
                    self.workspace
                        .select_session(session_id)
                        .expect("freshly created session is live");
                    self.active_session = Some(session_id);
                    Some(session)
                }
                Err(_) => {
                    self.status = "Noren PTY start failed";
                    self.show_status = true;
                    self.pty_child = PtyChildStatus::NotLaunched;
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
        if diagnostics_chord_pressed(
            &event.logical_key,
            event.state,
            event.repeat,
            self.modifiers,
        ) {
            self.toggle_diagnostics();
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
        // The sidebar occupies the leftmost SIDEBAR_COLS cell columns; clicks
        // inside it do not address the terminal grid.
        if position.x < sidebar_pixel_width() {
            return None;
        }
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
        let column = terminal_column_at(position.x, cols)?;
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

    /// Toggle the opt-in diagnostics overlay.
    ///
    /// Each activation emits exactly one bounded report line (window title
    /// and standard error); no screen or PTY content is ever included. See
    /// [`noren_app::diagnostics`].
    fn toggle_diagnostics(&mut self) {
        self.diagnostics_visible = !self.diagnostics_visible;
        if !self.diagnostics_visible {
            self.diagnostics_line.clear();
            if let Some(window) = &self.window {
                window.set_title("Noren PoC");
            }
            return;
        }
        let snapshot = self.terminal.as_ref().map(TerminalEngine::snapshot);
        let input = diagnostics::from_snapshot(snapshot.as_ref(), self.pty_child);
        let line = diagnostics::report(&input);
        eprintln!("{line}");
        if let Some(window) = &self.window {
            window.set_title(&line);
        }
        self.diagnostics_line = line;
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
        let cols = terminal_cols(grid.cols());
        if let Some(terminal) = &mut self.terminal {
            if terminal.resize(grid.rows(), cols).is_err() {
                self.status = "Noren terminal resize failed";
                self.show_status = true;
            }
        }
        if let (Some(session), Ok(size)) = (&self.pty, pty_size(grid.rows(), cols)) {
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
                    self.pty_child = PtyChildStatus::Exited { code: None };
                    terminal_status = Some("Noren shell reached EOF");
                    break;
                }
                PtyEvent::Exited { code } => {
                    self.pty_child = PtyChildStatus::Exited { code };
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
        if let Some(id) = self.active_session.take() {
            let _ = self.workspace.close_session(id);
        }
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
        let sidebar_lines = sidebar_text_lines(self.workspace.sidebar());
        let outcome = self
            .renderer
            .as_mut()
            .map(|renderer| renderer.render(snapshot.as_ref(), Some(&sidebar_lines), status));
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
        if let Some(id) = self.active_session.take() {
            let _ = self.workspace.close_session(id);
        }
        if let Some(mut session) = self.pty.take() {
            self.pty_child = PtyChildStatus::NotLaunched;
            if session.shutdown().is_err() {
                eprintln!("Noren PTY shutdown reached its failure fallback");
            }
        }
        event_loop.exit();
    }
}

/// Super+D press toggles diagnostics. Super chords are dropped by the key
/// encoder anyway, so this intercept consumes no terminal input.
fn diagnostics_chord_pressed(
    logical_key: &WinitKey,
    state: ElementState,
    repeat: bool,
    modifiers: Modifiers,
) -> bool {
    state == ElementState::Pressed
        && !repeat
        && modifiers.is_super()
        && matches!(logical_key,
            WinitKey::Character(text) if text.eq_ignore_ascii_case("d"))
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

fn pty_size(rows: u16, cols: u16) -> Result<PtySize, noren_pty::PtyError> {
    PtySize::from_raw(rows, cols).ok_or(noren_pty::PtyError::InvalidSize)
}

/// Terminal column count for a given window column count, reserving
/// [`renderer::SIDEBAR_COLS`] columns on the left for the sidebar and clamping
/// the remainder to the renderer's drawable budget. The PTY winsize, terminal
/// state's column count, and the renderer's drawn region all use this value so
/// they never disagree.
///
/// Reserve the sidebar first, then clamp the terminal to
/// `MAX_RENDER_COLS - SIDEBAR_COLS` (floored at one). The sidebar sits *inside*
/// the renderer's `MAX_RENDER_COLS` ceiling, so the terminal must never be told
/// it owns more columns than the renderer can draw beside the sidebar —
/// otherwise columns are clipped invisibly. `renderer::glyph_vertices` applies
/// the identical formula independently; the sidebar geometry test pins that the
/// two sites agree.
fn terminal_cols(window_cols: u16) -> u16 {
    let sidebar = u16::try_from(renderer::SIDEBAR_COLS).unwrap_or(u16::MAX);
    let budget = MAX_RENDER_COLS.saturating_sub(sidebar).max(1);
    window_cols.saturating_sub(sidebar).clamp(1, budget)
}

/// Convert the sidebar view into text lines the renderer can draw.
///
/// Each row is prefixed with `>` when selected, space otherwise, followed by
/// the label and optional detail — using [`SidebarRow::label`] and
/// [`SidebarRow::detail`] verbatim. When the sidebar is empty the
/// empty-state message is returned as the sole line.
fn sidebar_text_lines(sidebar: &SidebarView) -> Vec<String> {
    if sidebar.is_empty() {
        return sidebar
            .empty_state()
            .map(|state| vec![state.message().to_string()])
            .unwrap_or_default();
    }
    sidebar
        .rows()
        .iter()
        .map(|row| {
            let marker = if row.is_selected() { '>' } else { ' ' };
            match row.detail() {
                Some(detail) => format!("{marker} {} {}", row.label(), detail),
                None => format!("{marker} {}", row.label()),
            }
        })
        .collect()
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

/// Pixel width of the sidebar's left strip: `SIDEBAR_COLS` cell columns. The
/// terminal is drawn to the right of this edge, so a click at exactly this x is
/// the first terminal column.
fn sidebar_pixel_width() -> f64 {
    f64::from((renderer::SIDEBAR_COLS as u32) * POC_CELL_WIDTH)
}

/// Terminal cell column under pixel x, or `None` when the click lands in the
/// sidebar strip, on a non-finite coordinate, or past the grid. The sidebar
/// boundary is exclusive: x exactly at [`sidebar_pixel_width`] is the first
/// terminal column and maps to cell 0; anything strictly left of it is the
/// sidebar and is rejected.
fn terminal_column_at(pixel_x: f64, terminal_cols: u16) -> Option<usize> {
    if !pixel_x.is_finite() || pixel_x < sidebar_pixel_width() {
        return None;
    }
    pixel_row_index(pixel_x - sidebar_pixel_width(), POC_CELL_WIDTH)
        .map(|raw| raw.min(usize::from(terminal_cols).saturating_sub(1)))
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
    let config = match AppConfig::load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("Noren configuration is unusable: {error}");
            eprintln!(
                "see docs/configuration.md; fix or remove the file (or unset NOREN_CONFIG) to continue"
            );
            std::process::exit(1);
        }
    };
    let Ok(event_loop) = EventLoop::new() else {
        eprintln!("Noren event loop creation failed");
        return;
    };
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = NorenApp::new(config);
    if event_loop.run_app(&mut app).is_err() {
        eprintln!("Noren event loop failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use noren_app::palette::CommandId;
    use noren_app::sidebar::EntryKind;

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

    #[test]
    fn diagnostics_chord_is_a_super_d_press_only() {
        let super_modifiers = Modifiers::empty().super_key();
        let chord = WinitKey::Character("d".into());
        for (state, repeat, modifiers, expected) in [
            (ElementState::Pressed, false, super_modifiers, true),
            (ElementState::Released, false, super_modifiers, false),
            (ElementState::Pressed, true, super_modifiers, false),
            (ElementState::Pressed, false, Modifiers::empty(), false),
            (
                ElementState::Pressed,
                false,
                Modifiers::empty().shift(),
                false,
            ),
        ] {
            assert_eq!(
                diagnostics_chord_pressed(&chord, state, repeat, modifiers),
                expected,
                "state={state:?} repeat={repeat}"
            );
        }
        for other in [
            WinitKey::Character("x".into()),
            WinitKey::Character("dd".into()),
            WinitKey::Named(NamedKey::Enter),
        ] {
            assert!(
                !diagnostics_chord_pressed(&other, ElementState::Pressed, false, super_modifiers),
                "only D toggles diagnostics"
            );
        }
        let shifted = WinitKey::Character("D".into());
        assert!(diagnostics_chord_pressed(
            &shifted,
            ElementState::Pressed,
            false,
            super_modifiers
        ));
    }

    #[test]
    fn toggle_diagnostics_reports_live_state_and_clears_on_exit() {
        let mut app = NorenApp::default();
        let mut terminal = TerminalState::new(4, 8).expect("valid terminal");
        terminal.feed_bytes(b"\x1b[?1h");
        app.terminal = Some(terminal);

        app.toggle_diagnostics();
        assert!(app.diagnostics_visible);
        assert!(
            app.diagnostics_line.contains("grid=4x8"),
            "diagnostics: {}",
            app.diagnostics_line
        );
        assert!(
            app.diagnostics_line
                .contains("modes=alt:0 cursor:1 keypad:0"),
            "diagnostics: {}",
            app.diagnostics_line
        );
        assert!(
            app.diagnostics_line.contains("child=not launched"),
            "diagnostics: {}",
            app.diagnostics_line
        );

        app.toggle_diagnostics();
        assert!(!app.diagnostics_visible);
        assert!(app.diagnostics_line.is_empty());
    }

    #[test]
    fn toggle_diagnostics_never_repeats_terminal_content() {
        let mut app = NorenApp::default();
        let mut terminal = TerminalState::new(2, 40).expect("valid terminal");
        terminal.feed_bytes(b"SECRET-MARKER-9f8e7d6c\n\n\n\n");
        app.terminal = Some(terminal);

        app.toggle_diagnostics();
        assert!(app.diagnostics_visible);
        assert!(
            !app.diagnostics_line.contains("SECRET"),
            "diagnostics: {}",
            app.diagnostics_line
        );
        assert!(
            !app.diagnostics_line.contains("9f8e7d6c"),
            "diagnostics: {}",
            app.diagnostics_line
        );
    }

    #[test]
    fn configured_cell_sizes_drive_the_app_geometry() {
        let config = AppConfig::parse("[font]\ncell_width = 20\ncell_height = 40\n")
            .expect("valid configuration");
        let app = NorenApp::new(config);
        let mut expected = GridGeometry::with_cells(20, 40).expect("valid geometry");
        let mut actual = app.geometry;
        let grid = actual.update(Resize::new(900, 600)).expect("grid");
        assert_eq!(grid, expected.update(Resize::new(900, 600)).expect("grid"));
        assert_eq!((grid.rows(), grid.cols()), (15, 45));
    }

    #[test]
    fn workspace_starts_empty_with_no_sidebar_rows() {
        let state = WorkspaceState::new();
        assert!(state.registry().is_empty());
        assert!(state.sidebar().is_empty());
        assert!(state.sidebar().rows().is_empty());
        assert!(
            state.sidebar().empty_state().is_some(),
            "empty sidebar must carry an empty-state notice"
        );
        assert_eq!(state.sidebar().viewport(), None);
        assert_eq!(state.sidebar().selected_row_count(), 0);
    }

    #[test]
    fn creating_a_session_adds_a_session_row_to_the_sidebar() {
        let mut state = WorkspaceState::new();
        let id = state.create_session(SessionKind::Local);

        assert_eq!(state.registry().len(), 1);
        let rows = state.sidebar().rows();
        assert_eq!(rows.len(), 1, "sidebar must reflect the new session");
        assert_eq!(rows[0].kind(), EntryKind::Session);
        assert_eq!(rows[0].label(), id.to_string());
        assert!(
            rows[0].detail().is_some_and(|d| d.contains("local")),
            "session detail should mention the kind, got {:?}",
            rows[0].detail()
        );
    }

    #[test]
    fn selecting_a_session_marks_exactly_one_row_selected() {
        let mut state = WorkspaceState::new();
        let first = state.create_session(SessionKind::Local);
        let _second = state.create_session(SessionKind::Local);
        assert_eq!(
            state.sidebar().selected_row_count(),
            0,
            "no session is selected initially"
        );

        state.select_session(first).expect("first session is live");

        assert_eq!(state.sidebar().selected_row_count(), 1);
        let selected = state
            .sidebar()
            .rows()
            .iter()
            .find(|row| row.is_selected())
            .expect("exactly one selected row");
        assert_eq!(selected.label(), first.to_string());
        let viewport = state
            .sidebar()
            .viewport()
            .expect("a selected session yields a viewport");
        assert_eq!(viewport.session_id(), first);
    }

    #[test]
    fn selecting_the_other_session_moves_the_single_selection() {
        let mut state = WorkspaceState::new();
        let first = state.create_session(SessionKind::Local);
        let second = state.create_session(SessionKind::Local);
        state.select_session(first).expect("first is live");
        assert_eq!(state.sidebar().selected_row_count(), 1);

        state.select_session(second).expect("second is live");

        assert_eq!(state.sidebar().selected_row_count(), 1);
        let selected_label = state
            .sidebar()
            .rows()
            .iter()
            .find(|row| row.is_selected())
            .map(|row| row.label())
            .expect("one selected row");
        assert_eq!(selected_label, second.to_string());
    }

    #[test]
    fn closing_the_selected_session_leaves_a_coherent_view() {
        let mut state = WorkspaceState::new();
        let first = state.create_session(SessionKind::Local);
        let _second = state.create_session(SessionKind::Local);
        state.select_session(first).expect("first is live");
        assert!(state.sidebar().viewport().is_some());

        state.close_session(first).expect("first is live");

        assert_eq!(state.registry().len(), 1);
        let rows = state.sidebar().rows();
        assert_eq!(rows.len(), 1, "closed session must vanish from sidebar");
        assert!(
            !rows.iter().any(|row| row.label() == first.to_string()),
            "closed session id must not appear"
        );
        assert_eq!(
            state.sidebar().selected_row_count(),
            0,
            "closing the selected session clears the selection"
        );
        assert!(
            state.sidebar().viewport().is_none(),
            "no viewport without a selection"
        );
    }

    #[test]
    fn closing_all_sessions_shows_the_empty_state() {
        let mut state = WorkspaceState::new();
        let id = state.create_session(SessionKind::Local);
        state.close_session(id).expect("session is live");

        assert!(state.registry().is_empty());
        assert!(state.sidebar().is_empty());
        assert!(
            state.sidebar().empty_state().is_some(),
            "empty registry must produce an empty-state sidebar"
        );
        assert_eq!(state.sidebar().viewport(), None);
    }

    #[test]
    fn selecting_a_stale_id_does_not_panic_or_mutate_the_view() {
        let mut state = WorkspaceState::new();
        let id = state.create_session(SessionKind::Local);
        state.close_session(id).expect("session is live");
        let rows_before = state.sidebar().rows().len();

        let result = state.select_session(id);

        assert_eq!(result, Err(SessionError::UnknownSession));
        assert_eq!(
            state.sidebar().rows().len(),
            rows_before,
            "a failed select must not change the view"
        );
    }

    #[test]
    fn palette_carries_all_four_canonical_commands() {
        let state = WorkspaceState::new();
        let palette = state.palette();
        assert_eq!(palette.len(), 4);
        for id in [
            CommandId::SESSION_CREATE,
            CommandId::SESSION_SELECT,
            CommandId::SESSION_CLOSE,
            CommandId::SIDEBAR_FOCUS,
        ] {
            assert!(palette.get(id).is_some(), "palette must include {id}");
        }
        let hits = palette.search("session");
        assert!(
            hits.iter()
                .any(|hit| hit.command().id() == CommandId::SESSION_CREATE),
            "searching 'session' must find the create command"
        );
    }

    // =========================================================================
    // Sidebar geometry: the terminal width, the PTY winsize, and the renderer's
    // drawn region must all agree once the sidebar reserves 16 columns.
    // =========================================================================

    /// Number of terminal cell columns the renderer drew, measured from its
    /// vertex output rather than restating the column formula. Each terminal
    /// column is fed a glyph the renderer lights starting at the cell's left
    /// pixel edge (`B` lights glyph column 0), so a drawn column is detectable
    /// as a glyph rect whose LEFT edge sits on that boundary. Scanning runs
    /// rightward from the first terminal column (`SIDEBAR_COLS`) until a column
    /// has no glyph — terminal content is contiguous, so the first gap marks
    /// the end of the drawn region.
    ///
    /// Matching a rect's left edge (its top-left corner) — not *any* vertex on
    /// the boundary — is essential: a glyph's rightmost lit pixel column (e.g.
    /// `B`, whose rows `17 = 0b10001` light glyph column 4) produces a rect
    /// whose RIGHT edge lands exactly on the next column's left edge. Matching
    /// arbitrary vertices would count that bleed as a drawn column and over-
    /// count by one. Each rect is emitted as a 6-vertex fan whose first vertex
    /// is its top-left corner, so the left edges are read from every 6th group.
    fn rendered_terminal_columns(vertices: &[[f32; 2]], width: u32) -> usize {
        let rect_lefts: Vec<f32> = vertices.chunks_exact(6).map(|rect| rect[0][0]).collect();
        let mut drawn = 0;
        for col in renderer::SIDEBAR_COLS..usize::from(MAX_RENDER_COLS) {
            let edge = ((col as u32) * POC_CELL_WIDTH) as f32 / width as f32 * 2.0 - 1.0;
            if rect_lefts.iter().any(|left| (left - edge).abs() < 1e-5) {
                drawn += 1;
            } else {
                break;
            }
        }
        drawn
    }

    /// Drive the three terminal-width consumers — `TerminalState`'s stored
    /// column count, the PTY winsize, and the columns the renderer actually
    /// draws — at one window width, asserting they all agree on
    /// `terminal_cols(window_cols)`. Asserting `terminal_cols() == window_cols -
    /// 16` would merely restate the formula and pass any consistent-but-wrong
    /// value; instead this exercises the three real consumers and is shared by
    /// the swept agreement test below across every regime where they can drift
    /// apart.
    fn assert_three_consumers_agree_at(width: u32) {
        let height = 600_u32;
        // Grid columns and the renderer's visible columns both derive
        // `width / CELL_WIDTH`, so they share this `window_cols`.
        let window_cols = u16::try_from(width / POC_CELL_WIDTH).expect("fits in u16");
        let cols = terminal_cols(window_cols);

        // Consumer 1: the terminal state stores the sidebar-adjusted width.
        let rows = u16::try_from(height / POC_CELL_HEIGHT).expect("fits in u16");
        let mut terminal = TerminalState::new(rows, cols).expect("valid terminal");
        // Fill every terminal column of row 0 so each drawn column is visible
        // to `rendered_terminal_columns` via its left pixel edge.
        terminal.feed_bytes(&vec![b'B'; usize::from(cols)]);
        let (_, term_cols) = terminal.size();
        assert_eq!(
            term_cols, cols,
            "at {width}px: terminal must store terminal_cols({window_cols}) = {cols}"
        );

        // Consumer 2: the PTY winsize carries the same column count.
        let pty = pty_size(rows, cols).expect("valid pty size");
        assert_eq!(
            pty.cols(),
            cols,
            "at {width}px: PTY winsize must agree with the terminal"
        );

        // Consumer 3: the renderer draws exactly that many terminal columns —
        // measured from vertex output, independent of `terminal_cols`.
        let snapshot = terminal.snapshot();
        let sidebar: Vec<String> = Vec::new();
        let vertices = renderer::glyph_vertices(
            Some(&snapshot),
            Some(sidebar.as_slice()),
            None,
            width,
            height,
        );
        let drawn = rendered_terminal_columns(&vertices, width);
        assert_eq!(
            drawn,
            usize::from(cols),
            "at {width}px (window_cols={window_cols}): renderer drew {drawn} terminal columns but \
             terminal/PTY agree on {cols} — the sidebar width is not consistently subtracted or \
             the upper clamp is missing"
        );
    }

    /// The PR's headline property: the three consumers agree once the sidebar
    /// reserves 16 columns — swept across the input range rather than pinned at
    /// one width. A single point cannot support "agreement across the range",
    /// and the original 900px point sits squarely inside the band (17..=160
    /// columns) where the pre-fix geometry already agreed. Each swept width
    /// targets a distinct regime so a regression of either pre-fix defect is
    /// caught:
    ///   - 80px   (8 cols, below the sidebar width): the floor regime. A
    ///     renderer that floored at zero terminal columns while the terminal
    ///     and PTY held one is exposed.
    ///   - 900px  (90 cols, a typical window): the common case — the very width
    ///     a prior commit message wrongly cited as the divergence (both the
    ///     pre- and post-fix geometry agree at 74 here).
    ///   - 1600px (160 cols, exactly `MAX_RENDER_COLS`): the budget boundary,
    ///     where the terminal fills the whole drawable budget (144).
    ///   - 2000px (200 cols, above `MAX_RENDER_COLS`): the upper-clamp regime.
    ///     A `terminal_cols` with no upper clamp would claim 184 columns while
    ///     the renderer draws 144, silently clipping 40 columns of output.
    ///
    /// This is a regression guard, not a reproduced bug: at the moment this
    /// test was added all three consumers already agreed at every swept width,
    /// and it exists to hold that line. Mutating `terminal_cols` to drop the
    /// sidebar subtraction breaks the typical/above-max widths, and dropping
    /// the upper clamp breaks the 2000px width.
    #[test]
    fn terminal_cols_pty_winsize_and_renderer_agree_across_the_width_range() {
        for width in [80_u32, 900, 1600, 2000] {
            assert_three_consumers_agree_at(width);
        }
    }

    /// MINOR-1: below ~160px the window fits inside the sidebar. `terminal_cols`
    /// floors at one (the terminal/PTY reject zero columns); the renderer must
    /// floor at the same one rather than drawing zero terminal columns while the
    /// terminal still holds one. Drives the real renderer so the agreement is
    /// measured, not assumed.
    #[test]
    fn terminal_cols_and_renderer_floor_at_one_below_the_sidebar() {
        // A window exactly SIDEBAR_COLS wide: visible_cols == SIDEBAR_COLS, so
        // the terminal region has no room — both floors must keep it at one.
        let width = (renderer::SIDEBAR_COLS as u32) * POC_CELL_WIDTH;
        let height = 600_u32;
        let window_cols = u16::try_from(width / POC_CELL_WIDTH).expect("fits in u16");
        assert_eq!(window_cols, u16::try_from(renderer::SIDEBAR_COLS).unwrap());
        let cols = terminal_cols(window_cols);
        assert_eq!(cols, 1, "terminal_cols floors at one, never zero");

        let mut terminal = TerminalState::new(2, cols).expect("valid terminal");
        terminal.feed_bytes(&vec![b'B'; usize::from(cols)]);
        let snapshot = terminal.snapshot();
        let sidebar: Vec<String> = Vec::new();
        let vertices = renderer::glyph_vertices(
            Some(&snapshot),
            Some(sidebar.as_slice()),
            None,
            width,
            height,
        );
        let drawn = rendered_terminal_columns(&vertices, width);
        assert_eq!(
            drawn,
            usize::from(cols),
            "renderer must draw the terminal's one column, not zero — the floor \
             disagrees with terminal_cols below the sidebar width"
        );
    }

    /// MINOR-2: `sidebar_text_lines` is the seam between the view model and the
    /// renderer. Drive a real `SidebarView` built by the workspace from a live
    /// registry — not hardcoded strings — so a change in how rows are formatted
    /// is caught here.
    #[test]
    fn sidebar_text_lines_format_a_real_workspace_sidebar() {
        // Empty workspace: the empty-state notice is the sole line.
        let empty = WorkspaceState::new();
        assert_eq!(
            sidebar_text_lines(empty.sidebar()),
            vec!["No sessions".to_string()],
            "empty workspace renders its empty-state notice"
        );

        let mut state = WorkspaceState::new();
        let first = state.create_session(SessionKind::Local);
        let second = state.create_session(SessionKind::Local);
        state.select_session(first).expect("first session is live");

        let lines = sidebar_text_lines(state.sidebar());
        assert_eq!(lines.len(), 2, "one formatted line per sidebar row");

        // The selected row is prefixed with '>' and the unselected with a
        // space; both carry the real descriptor's label and detail.
        assert!(
            lines[0].starts_with("> "),
            "selected row must be marked with '>': {:?}",
            lines[0]
        );
        assert!(
            lines[1].starts_with("  "),
            "unselected row must be marked with a space: {:?}",
            lines[1]
        );
        assert!(
            lines[0].contains(first.to_string().as_str()),
            "selected row carries the session label: {:?}",
            lines[0]
        );
        assert!(
            lines[1].contains(second.to_string().as_str()),
            "unselected row carries the session label: {:?}",
            lines[1]
        );
        // A freshly created session sits at the Starting status, so the detail
        // is derived from the real descriptor, not a constant.
        assert!(
            lines[0].contains("local · starting"),
            "detail comes from the real descriptor: {:?}",
            lines[0]
        );
    }

    /// MINOR-3: `grid_point_at`'s sidebar boundary. A click in the last sidebar
    /// column must be rejected, and a click at the first terminal column must
    /// map to terminal cell 0. `grid_point_at` itself needs a live window this
    /// harness cannot create, so this drives the extracted column mapper that
    /// `grid_point_at` delegates to.
    #[test]
    fn terminal_column_at_rejects_the_sidebar_and_starts_the_terminal_at_zero() {
        let cols = 40_u16;
        let sidebar_edge = sidebar_pixel_width();

        // The last sidebar column — just inside the sidebar's right edge — does
        // not address the terminal grid.
        assert_eq!(
            terminal_column_at(sidebar_edge - 1.0, cols),
            None,
            "a click in the last sidebar column must be rejected"
        );
        // The first terminal column, exactly at the sidebar's right edge, maps
        // to terminal cell 0.
        assert_eq!(
            terminal_column_at(sidebar_edge, cols),
            Some(0),
            "the first terminal column must map to cell 0"
        );
        // One cell width further in lands in terminal cell 1.
        assert_eq!(
            terminal_column_at(sidebar_edge + f64::from(POC_CELL_WIDTH), cols),
            Some(1)
        );
        // The last terminal column maps to the highest valid cell.
        assert_eq!(
            terminal_column_at(
                sidebar_edge + f64::from(POC_CELL_WIDTH) * f64::from(cols - 1),
                cols
            ),
            Some(usize::from(cols - 1))
        );
        // A click past the last column clamps to the last cell, never overflows.
        assert_eq!(
            terminal_column_at(
                sidebar_edge + f64::from(POC_CELL_WIDTH) * f64::from(cols),
                cols
            ),
            Some(usize::from(cols - 1))
        );
        // Negative and non-finite clicks are rejected.
        assert_eq!(terminal_column_at(-1.0, cols), None);
        assert_eq!(terminal_column_at(f64::NAN, cols), None);
    }
}
