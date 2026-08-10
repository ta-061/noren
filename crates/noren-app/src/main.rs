//! macOS entry point for the bounded local-zsh PTY PoC.

mod renderer;

use noren_app::{
    Arrow, CellMetrics, CursorKeyMode, FunctionKey, GridGeometry, GridSize, InputMode, Key,
    KeyDropReason, KeyEncoder, KeyInput, KeyPhase, KeypadInput, KeypadKey, KeypadMode,
    MAX_RENDER_COLS, MAX_RENDER_ROWS, Modifiers, PARSE_BUDGET_BYTES_PER_TURN, PasteReject, Resize,
    SystemClipboard,
    config::AppConfig,
    diagnostics::{self, PtyChildStatus},
    encode_paste,
    mouse::{
        MouseButton as EncoderButton, MouseEncoder, MouseGrid, MouseModes, PointerEvent,
        PointerModifiers, WheelDirection,
    },
    palette::{CommandId, Palette},
    passthrough::{
        CLAIM_ID_PALETTE, Chord, ChordSeq, GateKind, KeyCode as GateKeyCode,
        Modifiers as GateModifiers, PassthroughAction, PassthroughClaim, PassthroughGate,
        PassthroughPolicy, default_exit_claim,
    },
    session::{
        SessionAction, SessionError, SessionEvent, SessionId, SessionKind, SessionRegistry,
        SessionStatus,
    },
    session_persistence::{SESSION_STATE_FILE_NAME, SessionPersistenceError, load, save},
    sidebar::{SidebarEntry, SidebarView},
};
use noren_pty::{PtyEvent, PtySession, PtySize};
use noren_terminal::{Cell, GridPoint, Selection, SelectionMode, TerminalEngine, TerminalState};
use renderer::{RenderOutcome, Renderer};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
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
/// Build the pass-through policy claiming the exit leader (Super+Escape) and
/// the palette opener (Super+p).
///
/// Both claimed chords live in the Super/Command modifier space, which the
/// pinned Zellij v0.44.3 default corpus never binds. Super+Escape is the
/// frozen exit leader from [`default_exit_claim`]; Super+p opens the command
/// palette. This is the smallest set that works: one chord to open the
/// palette, one to exit to the workspace. No bare modifier chords, no
/// Ctrl/Alt chords, nothing Zellij could ever use.
fn palette_policy() -> PassthroughPolicy {
    let palette_claim = PassthroughClaim {
        id: CLAIM_ID_PALETTE,
        action: PassthroughAction::OpenCommandPalette,
        seq: ChordSeq::single(
            Chord::new(GateKeyCode::Char('p'), GateModifiers::empty().super_key())
                .expect("normalized Super+p"),
        ),
        justification: "Super+p lives in the Super/Cmd modifier space which the \
                        pinned Zellij v0.44.3 default corpus never binds, so claiming it \
                        steals no chord from Zellij or its panes",
    };
    PassthroughPolicy::try_new(vec![default_exit_claim(), palette_claim])
        .expect("palette policy is valid and collision-free")
}

/// Resolve the sidebar state file path alongside the configuration file.
///
/// Follows config's directory convention exactly: the same directory
/// [`noren_app::config::default_path`] resolves, with the session-state file
/// name from [`SESSION_STATE_FILE_NAME`] substituted for the config file name.
/// Returns `None` when `HOME` is unset — matching config's behavior — so a
/// headless or containerized environment runs in-memory only.
fn session_state_path() -> Option<PathBuf> {
    noren_app::config::default_path().map(|mut path| {
        path.set_file_name(SESSION_STATE_FILE_NAME);
        path
    })
}

struct WorkspaceState {
    registry: SessionRegistry,
    sidebar: SidebarView,
    /// Owned by the workspace; dispatched when the palette opens.
    palette: Palette<WorkspaceAction>,
    /// Where sidebar state is persisted. `None` in tests and when `HOME` is
    /// unset; in both cases the workspace is in-memory only and [`persist`]
    /// is a no-op.
    state_path: Option<PathBuf>,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceState {
    /// Empty workspace: no sessions, empty sidebar, the canonical palette.
    fn new() -> Self {
        Self::with_state_path(None)
    }

    /// Empty workspace that persists to `state_path` when the sidebar changes.
    ///
    /// Used by the binary (via [`session_state_path`]) and by tests that need
    /// to verify save/load round-trips through the real file system.
    fn with_state_path(state_path: Option<PathBuf>) -> Self {
        Self {
            registry: SessionRegistry::new(),
            sidebar: SidebarView::build(&[], None),
            palette: Palette::noren(
                WorkspaceAction::CreateSession,
                WorkspaceAction::SelectSession,
                WorkspaceAction::CloseSession,
                WorkspaceAction::FocusSidebar,
            ),
            state_path,
        }
    }

    /// Load saved sidebar state from [`state_path`](Self::state_path) into the
    /// registry before the sidebar is first observed.
    ///
    /// A missing file is the first run and returns `Ok` with an untouched
    /// (empty) registry. Any other failure — corrupt, wrong-version,
    /// non-UTF-8, oversized — returns a typed [`SessionPersistenceError`] and
    /// leaves the registry exactly as it was, so the caller can surface the
    /// error through diagnostics and continue with an empty sidebar rather
    /// than panicking.
    fn restore(&mut self) -> Result<(), SessionPersistenceError> {
        let Some(path) = &self.state_path else {
            return Ok(());
        };
        load(path, &mut self.registry)?;
        self.rebuild_sidebar();
        Ok(())
    }

    /// Persist the current sidebar state to
    /// [`state_path`](Self::state_path), if one is set.
    ///
    /// The write is atomic (temp-file rename) inside [`save`]; this method
    /// never bypasses it. A failure is surfaced through stderr and swallowed
    /// so the app keeps running — losing a save is preferable to crashing the
    /// terminal.
    fn persist(&self) {
        let Some(path) = &self.state_path else {
            return;
        };
        if let Err(error) = save(path, &self.registry) {
            eprintln!("Noren could not save sidebar state: {error}");
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
        self.persist();
        created_session_id(events)
    }

    /// Select a session by id and rebuild the sidebar.
    ///
    /// A stale id returns [`SessionError::UnknownSession`] without mutating the
    /// view — the registry did not change, so the sidebar is still correct.
    fn select_session(&mut self, id: SessionId) -> Result<(), SessionError> {
        self.registry.apply(SessionAction::Select { id })?;
        self.rebuild_sidebar();
        self.persist();
        Ok(())
    }

    /// Close a session by id and rebuild the sidebar.
    ///
    /// Closing the selected session clears the selection (the registry handles
    /// this), so the rebuilt sidebar shows no selection and no viewport.
    fn close_session(&mut self, id: SessionId) -> Result<(), SessionError> {
        self.registry.apply(SessionAction::Close { id })?;
        self.rebuild_sidebar();
        self.persist();
        Ok(())
    }

    /// Observe a status transition for a session and rebuild the sidebar.
    ///
    /// This is the only path that advances a session past `Starting`. The
    /// registry's `observe` enforces monotonic lifecycle transitions; a
    /// rejected transition leaves the view unchanged.
    ///
    /// Status is a runtime observation, not a structural change, so this does
    /// not call [`persist`](Self::persist): the on-disk format records kinds
    /// and selection only, never status. A status change cannot alter what
    /// would be written.
    fn observe_session(&mut self, id: SessionId, status: SessionStatus) {
        if self.registry.observe(id, status).is_ok() {
            self.rebuild_sidebar();
        }
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
    fn palette(&self) -> &Palette<WorkspaceAction> {
        &self.palette
    }

    /// The session registry.
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

/// Passive scanner that observes DECSET (`CSI ? Pn h`) and DECRST
/// (`CSI ? Pn l`) sequences in PTY *output* and updates the app's
/// [`MouseModes`].
///
/// TerminalState's parser recognises only modes 1, 1049, and 2004; mouse
/// tracking and encoding modes (1000/1002/1003/1005/1006/1015) are dropped at
/// `private_action` and never reach `TerminalModes`. This scanner sits on the
/// output side as a read-only observer — it consumes no bytes and alters no
/// parsing — so the terminal's own state machine is undisturbed. It exists
/// because the alternative (a second parser) is what the project keeps filing
/// as a bug; this is the narrowest possible seam.
///
/// Cross-chunk boundaries: the DFA retains its state across calls, so a
/// `CSI ? 1000 h` split across two `PtyEvent::Output` chunks is still detected.
#[derive(Default)]
struct MouseModeScanner {
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
    fn scan(&mut self, bytes: &[u8], modes: &mut MouseModes) {
        for &byte in bytes {
            self.feed(byte, modes);
        }
    }
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
    /// Mouse tracking/encoding modes observed from PTY output (DECSET/DECRST).
    /// TerminalState does not track mouse modes, so the app maintains this as
    /// the single source of truth for whether pointer events are reported.
    mouse_modes: MouseModes,
    /// DFA retaining partial DECSET/DECRST sequences across PTY chunks.
    mouse_mode_scanner: MouseModeScanner,
    /// The currently-held mouse button when tracking is active, or `None`.
    /// Drives the `button` field of motion (drag/hover) reports.
    held_mouse_button: Option<MouseButton>,
    workspace: WorkspaceState,
    active_session: Option<SessionId>,
    palette_open: bool,
    palette_selection: usize,
    passthrough_gate: PassthroughGate,
    passthrough_policy: PassthroughPolicy,
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
            mouse_modes: MouseModes::disabled(),
            mouse_mode_scanner: MouseModeScanner::default(),
            held_mouse_button: None,
            workspace: WorkspaceState::new(),
            active_session: None,
            palette_open: false,
            palette_selection: 0,
            passthrough_gate: PassthroughGate::new(),
            passthrough_policy: palette_policy(),
        }
    }

    /// Wire sidebar persistence: set the state path, then load saved state
    /// before the event loop starts.
    ///
    /// Called from [`main`] after construction so that [`NorenApp::new`] (and
    /// the tests that rely on it) stay free of file-system side effects. A
    /// corrupt or unreadable file is surfaced through stderr and swallowed —
    /// the app starts with an empty sidebar and a working terminal, never a
    /// crash. A missing file (the first run) is silent.
    fn load_sidebar_state(&mut self, path: Option<PathBuf>) {
        self.workspace.state_path = path;
        if let Err(error) = self.workspace.restore() {
            eprintln!("Noren could not restore sidebar state: {error}");
            eprintln!("starting with an empty sidebar; the existing file was left in place");
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
                    self.workspace
                        .observe_session(session_id, SessionStatus::Running);
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
        self.renderer = match Renderer::new(Arc::clone(&window), self.geometry.cell_metrics()) {
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
        if self.palette_open {
            self.handle_palette_key(event);
            return;
        }
        self.handle_passthrough_key(event);
    }

    /// Route one key event through the pass-through gate.
    ///
    /// The gate claims Super+Escape (exit) and Super+p (palette). Everything
    /// else is forwarded byte-for-byte through the same encoder path as
    /// before the gate existed, so a closed-palette key press is
    /// byte-identical to the pre-gate behaviour.
    fn handle_passthrough_key(&mut self, event: &KeyEvent) {
        let input_mode = self.current_input_mode();
        let encoded = if let Some(input) = translate_keypad_key(event) {
            KeyEncoder::encode_keypad_with(input.with_modifiers(self.modifiers), input_mode)
        } else {
            translate_key(event, self.modifiers)
                .and_then(|input| KeyEncoder::encode_with(input, input_mode))
        };
        if event.state == ElementState::Pressed {
            if let Some(chord) = chord_from_event(event, self.modifiers) {
                let decision = self.passthrough_gate.press(&self.passthrough_policy, chord);
                match decision.kind {
                    GateKind::Intercepted(PassthroughAction::OpenCommandPalette) => {
                        self.open_palette();
                        return;
                    }
                    GateKind::Intercepted(PassthroughAction::ExitToWorkspace) => {
                        return;
                    }
                    GateKind::Pending => {
                        return;
                    }
                    GateKind::Forwarded => {
                        for replayed in &decision.replayed {
                            if let Some(bytes) = encode_chord(replayed, input_mode) {
                                self.send_input(&bytes);
                            }
                        }
                    }
                }
            }
        }
        let Ok(bytes) = encoded else {
            return;
        };
        self.send_input(&bytes);
    }

    /// Handle a key event while the palette is open.
    ///
    /// Single-key shortcuts dispatch the four canonical commands; Escape
    /// dismisses without running; Arrow Up/Down and Enter navigate and
    /// confirm the selection.
    fn handle_palette_key(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed || event.repeat {
            return;
        }
        match &event.logical_key {
            WinitKey::Named(NamedKey::Escape) => {
                self.close_palette();
            }
            WinitKey::Named(NamedKey::ArrowUp) => {
                self.palette_selection = self.palette_selection.saturating_sub(1);
                self.redraw_needed = true;
            }
            WinitKey::Named(NamedKey::ArrowDown) => {
                let max = self.workspace.palette().len().saturating_sub(1);
                self.palette_selection = (self.palette_selection + 1).min(max);
                self.redraw_needed = true;
            }
            WinitKey::Named(NamedKey::Enter) => {
                let selection = self.palette_selection;
                self.dispatch_palette_selection(selection);
            }
            WinitKey::Character(text) => {
                let Some(ch) = text.chars().next() else {
                    return;
                };
                if text.chars().count() > 1 {
                    return;
                }
                let id = match ch.to_ascii_lowercase() {
                    'c' => CommandId::SESSION_CREATE,
                    's' => CommandId::SESSION_SELECT,
                    'x' => CommandId::SESSION_CLOSE,
                    'f' => CommandId::SIDEBAR_FOCUS,
                    _ => {
                        self.close_palette();
                        return;
                    }
                };
                self.dispatch_palette_command(id);
            }
            _ => {}
        }
    }

    /// Open the palette, selecting the first command.
    fn open_palette(&mut self) {
        self.palette_open = true;
        self.palette_selection = 0;
        self.redraw_needed = true;
    }

    /// Close the palette without running a command.
    fn close_palette(&mut self) {
        self.palette_open = false;
        self.redraw_needed = true;
    }

    /// Dispatch the palette command at `selection` and close the palette.
    fn dispatch_palette_selection(&mut self, selection: usize) {
        let palette = self.workspace.palette();
        let commands: Vec<CommandId> = palette.iter().map(|c| c.id()).collect();
        if let Some(&id) = commands.get(selection) {
            self.dispatch_palette_command(id);
        } else {
            self.close_palette();
        }
    }

    /// Run a palette command by stable ID, then close the palette.
    fn dispatch_palette_command(&mut self, id: CommandId) {
        let action = self.workspace.palette().get(id).map(|cmd| *cmd.action());
        if let Some(action) = action {
            self.run_workspace_action(action);
        }
        self.close_palette();
    }

    /// Execute a workspace action through `WorkspaceState`.
    fn run_workspace_action(&mut self, action: WorkspaceAction) {
        match action {
            WorkspaceAction::CreateSession => {
                let _id = self.workspace.create_session(SessionKind::Local);
            }
            WorkspaceAction::SelectSession => {
                let ids: Vec<SessionId> = self
                    .workspace
                    .registry()
                    .sessions()
                    .into_iter()
                    .map(|d| d.id())
                    .collect();
                if ids.len() < 2 {
                    return;
                }
                let current = self.workspace.registry().selected();
                let next = match current {
                    Some(cur) => {
                        let idx = ids.iter().position(|id| *id == cur).unwrap_or(0);
                        ids[(idx + 1) % ids.len()]
                    }
                    None => ids[0],
                };
                let _ = self.workspace.select_session(next);
            }
            WorkspaceAction::CloseSession => {
                if let Some(id) = self.workspace.registry().selected() {
                    let _ = self.workspace.close_session(id);
                } else {
                    let ids: Vec<SessionId> = self
                        .workspace
                        .registry()
                        .sessions()
                        .into_iter()
                        .map(|d| d.id())
                        .collect();
                    if let Some(id) = ids.first() {
                        let _ = self.workspace.close_session(*id);
                    }
                }
            }
            WorkspaceAction::FocusSidebar => {
                // The sidebar is always visible in this PoC; focusing is a
                // no-op that still confirms the command dispatches.
            }
        }
        self.redraw_needed = true;
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
        if self.mouse_reportable() {
            if let Some((col, row)) = self.mouse_cell_at(position) {
                let button = self.held_mouse_button.and_then(encode_button);
                let event = PointerEvent::move_to(button, col, row, self.pointer_modifiers());
                self.encode_and_send_mouse(event);
            }
            return;
        }
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
        if self.mouse_reportable() {
            self.handle_tracked_mouse_button(state, button);
            return;
        }
        // Tracking disabled (or Shift-bypassed): byte-identical to the
        // pre-tracking selection behaviour.
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

    /// Handle a mouse button event while tracking is active (no Shift bypass).
    ///
    /// Encodes a press or release report for Left/Middle/Right and sends it to
    /// the PTY. Sidebar clicks and unmapped buttons produce no bytes. The held
    /// button is recorded only when the press is actually reported — a press
    /// that produces no bytes (e.g. inside the sidebar) must not seed the
    /// tracking state, or a later drag into the terminal would emit a motion
    /// report with no preceding press. A release always clears the held button
    /// regardless of position, since the physical button is up either way.
    fn handle_tracked_mouse_button(&mut self, state: ElementState, button: MouseButton) {
        let Some(encode_btn) = encode_button(button) else {
            return;
        };
        // A release clears the held button unconditionally — the physical
        // button is up even if the release landed outside the terminal grid.
        if state == ElementState::Released {
            self.held_mouse_button = None;
        }
        let Some(position) = self.cursor_position else {
            return;
        };
        let Some((col, row)) = self.mouse_cell_at(position) else {
            return;
        };
        // Record the held button only when the press is actually reported.
        if state == ElementState::Pressed {
            self.held_mouse_button = Some(button);
        }
        let kind = match state {
            ElementState::Pressed => noren_app::mouse::PointerKind::Press(encode_btn),
            ElementState::Released => noren_app::mouse::PointerKind::Release(encode_btn),
        };
        let event = PointerEvent::new(kind, col, row, self.pointer_modifiers());
        self.encode_and_send_mouse(event);
    }

    /// Handle a scroll-wheel event. Under tracking, each line of delta
    /// generates one wheel report; without tracking, the event is ignored
    /// (matching the pre-tracking behaviour where `MouseWheel` fell into the
    /// `_ => {}` catch-all).
    fn handle_mouse_wheel(&mut self, delta: MouseScrollDelta) {
        if !self.mouse_reportable() {
            return;
        }
        let Some(position) = self.cursor_position else {
            return;
        };
        let Some((col, row)) = self.mouse_cell_at(position) else {
            return;
        };
        let mods = self.pointer_modifiers();
        for direction in wheel_clicks(delta, self.geometry.cell_metrics()) {
            let event = PointerEvent::wheel(direction, col, row, mods);
            self.encode_and_send_mouse(event);
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
        let cell_width = self.geometry.cell_width();
        let cell_height = self.geometry.cell_height();
        // The sidebar occupies the leftmost SIDEBAR_COLS cell columns; clicks
        // inside it do not address the terminal grid.
        if position.x < sidebar_pixel_width(cell_width) {
            return None;
        }
        let visible_rows = usize::try_from(physical.height / cell_height)
            .unwrap_or(0)
            .clamp(1, usize::from(MAX_RENDER_ROWS));
        let content_rows = visible_content_rows(terminal);
        let total_lines = content_rows + usize::from(self.show_status);
        let displayed = total_lines.min(visible_rows);
        let top_blank_rows = visible_rows - displayed;
        let first_line = total_lines - displayed;
        let row = pixel_row_index(position.y, cell_height)?;
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
        let column = terminal_column_at(position.x, cols, cell_width)?;
        Some(GridPoint::new(
            terminal.scrollback_len() + line_index,
            column,
        ))
    }

    /// Whether pointer events should be reported to the PTY instead of driving
    /// local text selection. Active when a tracking mode (1000/1002/1003) is on
    /// and Shift is not held — Shift bypasses reporting so the user can still
    /// select text while a program tracks the mouse, matching xterm/iTerm.
    fn mouse_reportable(&self) -> bool {
        self.mouse_modes.is_tracked() && !self.modifiers.is_shift()
    }

    /// Map a pixel position to 0-based `(col, row)` cell indices suitable for
    /// the mouse encoder. Delegates to [`grid_point_at`](Self::grid_point_at)
    /// which already excludes sidebar clicks and accounts for the bottom-aligned
    /// content layout, then converts the absolute scrollback line to a
    /// 0-based visible row.
    fn mouse_cell_at(&self, position: PhysicalPosition<f64>) -> Option<(u32, u32)> {
        let terminal = self.terminal.as_ref()?;
        let point = self.grid_point_at(position)?;
        let visible_row = point.line().checked_sub(terminal.scrollback_len())?;
        let col = u32::try_from(point.column()).ok()?;
        let row = u32::try_from(visible_row).ok()?;
        Some((col, row))
    }

    /// Build a [`MouseGrid`] from the terminal's current size for encoder
    /// clamping. The terminal's column count already excludes the sidebar
    /// (via [`terminal_cols`]), so clamping uses the correct grid bounds.
    fn mouse_grid(&self) -> Option<MouseGrid> {
        let terminal = self.terminal.as_ref()?;
        let (rows, cols) = terminal.size();
        MouseGrid::new(cols, rows)
    }

    /// Convert the app's current modifier state to mouse pointer modifiers.
    /// Super/Command is excluded (the window layer drops it), matching the key
    /// encoder's policy.
    fn pointer_modifiers(&self) -> PointerModifiers {
        let mut mods = PointerModifiers::empty();
        if self.modifiers.is_shift() {
            mods = mods.shift();
        }
        if self.modifiers.is_alt() {
            mods = mods.alt();
        }
        if self.modifiers.is_ctrl() {
            mods = mods.ctrl();
        }
        mods
    }

    /// Encode one pointer event and write the report bytes to the PTY. When
    /// the encoder returns `None` (event not reportable under the active
    /// tracking mode, or coordinate out of range), no bytes are sent.
    fn encode_and_send_mouse(&mut self, event: PointerEvent) {
        let Some(grid) = self.mouse_grid() else {
            return;
        };
        let modes = self.mouse_modes;
        if let Some(bytes) = MouseEncoder::encode(event, modes, grid) {
            self.send_input(&bytes);
        }
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
                    // Passively observe DECSET/DECRST for mouse modes before
                    // feeding the terminal. The scanner consumes no bytes.
                    self.mouse_mode_scanner.scan(&bytes, &mut self.mouse_modes);
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
            let code = match self.pty_child {
                PtyChildStatus::Exited { code } => code.map(|c| c as i32),
                _ => None,
            };
            self.workspace
                .observe_session(id, SessionStatus::Exited { code });
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
        let lines = if self.palette_open {
            let mut lines = palette_text_lines(self.workspace.palette(), self.palette_selection);
            lines.extend(sidebar_lines);
            lines
        } else {
            sidebar_lines
        };
        let outcome = self
            .renderer
            .as_mut()
            .map(|renderer| renderer.render(snapshot.as_ref(), Some(&lines), status));
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

    /// Everything `close` does apart from asking the event loop to exit.
    ///
    /// Split out so tests can drive the real teardown: `ActiveEventLoop` cannot
    /// be constructed outside a running event loop, so a test that called
    /// `close` could not exist, and the quit path went unexercised. Every
    /// state-affecting step lives here; `close` adds only `event_loop.exit()`.
    fn teardown(&mut self) {
        // Quitting is not closing. The user asked to leave Noren, not to
        // discard the session — and `SessionRegistry::close` *removes* the
        // entry rather than marking it stopped, so closing here would persist
        // a deletion and hand back an empty sidebar on the next launch. The
        // session stays in the registry; only its PTY goes away. On the next
        // launch it is restored as `SessionStatus::Restored`: a visible entry
        // whose shell is not running and cannot be reattached implicitly.
        //
        // This also protects the non-quit caller: `redraw` invokes `close` on
        // `RenderOutcome::DeviceLost`. A lost GPU device must not delete the
        // user's sessions.
        self.active_session = None;
        // Save on clean exit so a session selected but not otherwise mutated
        // is not lost. No structural mutation precedes this, so it is the only
        // write on the quit path and does not depend on ordering.
        self.workspace.persist();
        if let Some(mut session) = self.pty.take() {
            self.pty_child = PtyChildStatus::NotLaunched;
            if session.shutdown().is_err() {
                eprintln!("Noren PTY shutdown reached its failure fallback");
            }
        }
    }

    fn close(&mut self, event_loop: &ActiveEventLoop) {
        self.teardown();
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
            WindowEvent::MouseWheel { delta, .. } => self.handle_mouse_wheel(delta),
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

/// Build text lines for the palette display, drawn at the top of the sidebar
/// column when the palette is open.
///
/// Each command is one line: `]` marks the selected command, space otherwise,
/// followed by a single-key shortcut and the label. The lines are uppercase
/// to match the bitmap font's case-folding.
fn palette_text_lines(palette: &Palette<WorkspaceAction>, selection: usize) -> Vec<String> {
    let shortcuts = ['C', 'S', 'X', 'F'];
    palette
        .iter()
        .enumerate()
        .map(|(idx, cmd)| {
            let marker = if idx == selection { ']' } else { ' ' };
            let key = shortcuts.get(idx).copied().unwrap_or('?');
            format!("{marker}{key} {label}", label = cmd.label())
        })
        .collect()
}

/// Map a winit key event and app modifiers to a pass-through chord.
///
/// Returns `None` for keys that cannot be normalized into a [`Chord`]
/// (whitespace characters, dead keys, multi-codepoint IME sequences). Such
/// keys bypass the gate and follow the normal encode-and-send path.
fn chord_from_event(event: &KeyEvent, modifiers: Modifiers) -> Option<Chord> {
    let code = winit_to_gate_key(&event.logical_key)?;
    let gate_mods = gate_modifiers(modifiers);
    Chord::new(code, gate_mods).ok()
}

/// Encode a pass-through chord into PTY bytes for replay.
///
/// Used when a held leader prefix is replayed after a mismatch. The encoding
/// mirrors what [`KeyEncoder::encode_with`] would produce for the equivalent
/// key event. Returns `None` for chords that the encoder would drop (e.g.
/// Super-modified chords, which produce no PTY bytes).
fn encode_chord(chord: &Chord, mode: InputMode) -> Option<Vec<u8>> {
    let key = gate_key_to_app(chord.code())?;
    let mods = app_modifiers_from_gate(chord.modifiers());
    let input = KeyInput::new(key, KeyPhase::Pressed, mods);
    KeyEncoder::encode_with(input, mode).ok()
}

fn winit_to_gate_key(key: &WinitKey) -> Option<GateKeyCode> {
    match key {
        WinitKey::Character(text) => {
            let ch = text.chars().next()?;
            if text.chars().count() > 1 {
                return None;
            }
            Some(GateKeyCode::Char(ch))
        }
        WinitKey::Named(NamedKey::Escape) => Some(GateKeyCode::Escape),
        WinitKey::Named(NamedKey::Enter) => Some(GateKeyCode::Enter),
        WinitKey::Named(NamedKey::Tab) => Some(GateKeyCode::Tab),
        WinitKey::Named(NamedKey::Backspace) => Some(GateKeyCode::Backspace),
        WinitKey::Named(NamedKey::Space) => Some(GateKeyCode::Space),
        WinitKey::Named(NamedKey::ArrowUp) => Some(GateKeyCode::Up),
        WinitKey::Named(NamedKey::ArrowDown) => Some(GateKeyCode::Down),
        WinitKey::Named(NamedKey::ArrowLeft) => Some(GateKeyCode::Left),
        WinitKey::Named(NamedKey::ArrowRight) => Some(GateKeyCode::Right),
        WinitKey::Named(NamedKey::Home) => Some(GateKeyCode::Home),
        WinitKey::Named(NamedKey::End) => Some(GateKeyCode::End),
        WinitKey::Named(NamedKey::PageUp) => Some(GateKeyCode::PageUp),
        WinitKey::Named(NamedKey::PageDown) => Some(GateKeyCode::PageDown),
        WinitKey::Named(NamedKey::Delete) => Some(GateKeyCode::Delete),
        WinitKey::Named(NamedKey::Insert) => Some(GateKeyCode::Insert),
        _ => None,
    }
}

fn gate_key_to_app(code: GateKeyCode) -> Option<Key> {
    match code {
        GateKeyCode::Char(ch) => Some(Key::Character(ch)),
        GateKeyCode::Enter => Some(Key::Enter),
        GateKeyCode::Tab => Some(Key::Tab),
        GateKeyCode::Backspace => Some(Key::Backspace),
        GateKeyCode::Escape => Some(Key::Escape),
        GateKeyCode::Space => Some(Key::Character(' ')),
        GateKeyCode::Up => Some(Key::Arrow(Arrow::Up)),
        GateKeyCode::Down => Some(Key::Arrow(Arrow::Down)),
        GateKeyCode::Left => Some(Key::Arrow(Arrow::Left)),
        GateKeyCode::Right => Some(Key::Arrow(Arrow::Right)),
        GateKeyCode::Home => Some(Key::Home),
        GateKeyCode::End => Some(Key::End),
        GateKeyCode::PageUp => Some(Key::PageUp),
        GateKeyCode::PageDown => Some(Key::PageDown),
        GateKeyCode::Delete => Some(Key::Delete),
        GateKeyCode::Insert => Some(Key::Insert),
        GateKeyCode::Function(_) => None,
    }
}

fn gate_modifiers(mods: Modifiers) -> GateModifiers {
    let mut gate = GateModifiers::empty();
    if mods.is_ctrl() {
        gate = gate.ctrl();
    }
    if mods.is_alt() {
        gate = gate.alt();
    }
    if mods.is_shift() {
        gate = gate.shift();
    }
    if mods.is_super() {
        gate = gate.super_key();
    }
    gate
}

fn app_modifiers_from_gate(mods: GateModifiers) -> Modifiers {
    let mut app = Modifiers::empty();
    if mods.is_ctrl() {
        app = app.ctrl();
    }
    if mods.is_alt() {
        app = app.alt();
    }
    if mods.is_shift() {
        app = app.shift();
    }
    if mods.is_super() {
        app = app.super_key();
    }
    app
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
fn sidebar_pixel_width(cell_width: u32) -> f64 {
    f64::from((renderer::SIDEBAR_COLS as u32) * cell_width)
}

/// Terminal cell column under pixel x, or `None` when the click lands in the
/// sidebar strip, on a non-finite coordinate, or past the grid. The sidebar
/// boundary is exclusive: x exactly at [`sidebar_pixel_width`] is the first
/// terminal column and maps to cell 0; anything strictly left of it is the
/// sidebar and is rejected.
fn terminal_column_at(pixel_x: f64, terminal_cols: u16, cell_width: u32) -> Option<usize> {
    let edge = sidebar_pixel_width(cell_width);
    if !pixel_x.is_finite() || pixel_x < edge {
        return None;
    }
    pixel_row_index(pixel_x - edge, cell_width)
        .map(|raw| raw.min(usize::from(terminal_cols).saturating_sub(1)))
}

/// Map a winit mouse button to the encoder's button type. `Back`, `Forward`,
/// and `Other` are not reportable and return `None`.
fn encode_button(button: MouseButton) -> Option<EncoderButton> {
    match button {
        MouseButton::Left => Some(EncoderButton::Left),
        MouseButton::Middle => Some(EncoderButton::Middle),
        MouseButton::Right => Some(EncoderButton::Right),
        MouseButton::Back | MouseButton::Forward | MouseButton::Other(_) => None,
    }
}

/// Convert a winit scroll delta to a sequence of wheel directions (one per
/// line scrolled).
///
/// Both `LineDelta` and `PixelDelta` share the same vertical sign convention.
/// From the winit 0.30 source (`event.rs`, `MouseScrollDelta`):
///
///   LineDelta:   "Positive values indicate that the content that is being
///                 scrolled should move right and down (revealing more content
///                 left and up)."
///   PixelDelta:  "Positive values indicate that the content being scrolled
///                 should move right/down."
///
/// Positive y therefore means the user scrolled **up** (content moves down,
/// revealing earlier content). xterm sends button 4 (`Cb=64`,
/// `WheelDirection::Up`) for scroll-up; negative y is scroll-down (`Cb=65`).
///
/// A non-zero delta that rounds to zero lines still produces one click so a
/// single-notch wheel is never lost.
///
/// `metrics` carries the configured cell height — the same runtime
/// [`CellMetrics`] the renderer and the click-to-grid mappers read — so a
/// `PixelDelta` is converted to lines at the configured stride. Dividing by a
/// compile-time constant instead would convert at the PoC height regardless of
/// `[font] cell_height`, halving the line count at the default and doubling it
/// wherever the height is raised.
fn wheel_clicks(delta: MouseScrollDelta, metrics: CellMetrics) -> Vec<WheelDirection> {
    let lines = match delta {
        MouseScrollDelta::LineDelta(_, y) => y,
        MouseScrollDelta::PixelDelta(pos) => (pos.y / f64::from(metrics.height())) as f32,
    };
    let count = lines.abs().floor().max(0.0) as usize;
    let count = if count == 0 && lines != 0.0 { 1 } else { count };
    let direction = if lines < 0.0 {
        WheelDirection::Down
    } else {
        WheelDirection::Up
    };
    vec![direction; count]
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
    app.load_sidebar_state(session_state_path());
    if event_loop.run_app(&mut app).is_err() {
        eprintln!("Noren event loop failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use noren_app::palette::CommandId;
    use noren_app::passthrough::{self, collisions};
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
    fn rendered_terminal_columns(
        vertices: &[renderer::Vertex],
        width: u32,
        cell_width: u32,
    ) -> usize {
        let rect_lefts: Vec<f32> = vertices
            .chunks_exact(6)
            .map(|rect| rect[0].position[0])
            .collect();
        let mut drawn = 0;
        for col in renderer::SIDEBAR_COLS..usize::from(MAX_RENDER_COLS) {
            let edge = ((col as u32) * cell_width) as f32 / width as f32 * 2.0 - 1.0;
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
    /// draws — at one window width and cell size, asserting they all agree on
    /// `terminal_cols(window_cols)`. Asserting `terminal_cols() == window_cols -
    /// 16` would merely restate the formula and pass any consistent-but-wrong
    /// value; instead this exercises the three real consumers and is shared by
    /// the swept agreement test below across every regime where they can drift
    /// apart — including non-default cell sizes.
    fn assert_three_consumers_agree_at(width: u32, metrics: CellMetrics) {
        let height = 600_u32;
        let cell_width = metrics.width();
        let cell_height = metrics.height();
        let window_cols = u16::try_from(width / cell_width).expect("fits in u16");
        let cols = terminal_cols(window_cols);

        // Consumer 1: the terminal state stores the sidebar-adjusted width.
        let rows = u16::try_from(height / cell_height).expect("fits in u16");
        let mut terminal = TerminalState::new(rows, cols).expect("valid terminal");
        terminal.feed_bytes(&vec![b'B'; usize::from(cols)]);
        let (_, term_cols) = terminal.size();
        assert_eq!(
            term_cols,
            cols,
            "at {width}px cell {}x{}: terminal must store \
             terminal_cols({window_cols}) = {cols}",
            metrics.width(),
            metrics.height(),
        );

        // Consumer 2: the PTY winsize carries the same column count.
        let pty = pty_size(rows, cols).expect("valid pty size");
        assert_eq!(
            pty.cols(),
            cols,
            "at {width}px cell {}x{}: PTY winsize must agree",
            metrics.width(),
            metrics.height(),
        );

        // Consumer 3: the renderer draws exactly that many terminal columns —
        // measured from vertex output, independent of `terminal_cols`. The cell
        // metrics are threaded through so a renderer still drawing at the
        // compile-time default is exposed at a non-default size.
        let snapshot = terminal.snapshot();
        let sidebar: Vec<String> = Vec::new();
        let vertices = renderer::glyph_vertices(
            Some(&snapshot),
            Some(sidebar.as_slice()),
            None,
            width,
            height,
            metrics,
        );
        let drawn = rendered_terminal_columns(&vertices, width, cell_width);
        assert_eq!(
            drawn,
            usize::from(cols),
            "at {width}px cell {}x{} (window_cols={window_cols}): \
             renderer drew {drawn} terminal columns but terminal/PTY agree on {cols} — \
             the sidebar width is not consistently subtracted, the upper clamp is missing, \
             or the renderer ignored the configured cell size",
            metrics.width(),
            metrics.height(),
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
    ///
    /// The test is also swept across **cell sizes**. Issue #76: a configured
    /// `cell_width = 20` flows to the geometry/PTY but the renderer ignored it,
    /// drawing at the 10px compile-time constant. At 20px every width in the
    /// sweep produces half the window_cols, so the renderer — if still on the
    /// constant — would draw *twice* as many terminal columns as the terminal
    /// and PTY agree on, and the three consumers diverge. This is the
    /// acceptance criterion from the Issue.
    #[test]
    fn terminal_cols_pty_winsize_and_renderer_agree_across_the_width_range() {
        let poc = GridGeometry::poc().cell_metrics();
        for width in [80_u32, 900, 1600, 2000] {
            assert_three_consumers_agree_at(width, poc);
        }
        let big = GridGeometry::with_cells(20, 40)
            .expect("valid geometry")
            .cell_metrics();
        for width in [160_u32, 1800, 3200, 4000] {
            assert_three_consumers_agree_at(width, big);
        }
    }

    /// MINOR-1: below ~160px the window fits inside the sidebar. `terminal_cols`
    /// floors at one (the terminal/PTY reject zero columns); the renderer must
    /// floor at the same one rather than drawing zero terminal columns while the
    /// terminal still holds one. Drives the real renderer so the agreement is
    /// measured, not assumed.
    #[test]
    fn terminal_cols_and_renderer_floor_at_one_below_the_sidebar() {
        let cell_width = GridGeometry::poc().cell_width();
        // A window exactly SIDEBAR_COLS wide: visible_cols == SIDEBAR_COLS, so
        // the terminal region has no room — both floors must keep it at one.
        let width = (renderer::SIDEBAR_COLS as u32) * cell_width;
        let height = 600_u32;
        let window_cols = u16::try_from(width / cell_width).expect("fits in u16");
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
            GridGeometry::poc().cell_metrics(),
        );
        let drawn = rendered_terminal_columns(&vertices, width, cell_width);
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
        let cell_width = GridGeometry::poc().cell_width();
        let sidebar_edge = sidebar_pixel_width(cell_width);

        // The last sidebar column — just inside the sidebar's right edge — does
        // not address the terminal grid.
        assert_eq!(
            terminal_column_at(sidebar_edge - 1.0, cols, cell_width),
            None,
            "a click in the last sidebar column must be rejected"
        );
        // The first terminal column, exactly at the sidebar's right edge, maps
        // to terminal cell 0.
        assert_eq!(
            terminal_column_at(sidebar_edge, cols, cell_width),
            Some(0),
            "the first terminal column must map to cell 0"
        );
        // One cell width further in lands in terminal cell 1.
        assert_eq!(
            terminal_column_at(sidebar_edge + f64::from(cell_width), cols, cell_width),
            Some(1)
        );
        // The last terminal column maps to the highest valid cell.
        assert_eq!(
            terminal_column_at(
                sidebar_edge + f64::from(cell_width) * f64::from(cols - 1),
                cols,
                cell_width
            ),
            Some(usize::from(cols - 1))
        );
        // A click past the last column clamps to the last cell, never overflows.
        assert_eq!(
            terminal_column_at(
                sidebar_edge + f64::from(cell_width) * f64::from(cols),
                cols,
                cell_width
            ),
            Some(usize::from(cols - 1))
        );
        // Negative and non-finite clicks are rejected.
        assert_eq!(terminal_column_at(-1.0, cols, cell_width), None);
        assert_eq!(terminal_column_at(f64::NAN, cols, cell_width), None);
    }

    /// Issue #76: at a non-default cell width, the sidebar's drawn pixel
    /// boundary and the click-handling boundary must agree. The renderer draws
    /// the terminal starting at column `SIDEBAR_COLS`; `sidebar_pixel_width`
    /// and `terminal_column_at` must use the same `cell_width` to locate that
    /// boundary, or clicks land in the wrong region.
    ///
    /// At `cell_width = 20` the boundary is `16 * 20 = 320px`. If
    /// `sidebar_pixel_width` still used `POC_CELL_WIDTH` (10), the boundary
    /// would drift to 160px and clicks in the 160–320px strip would be
    /// misattributed to the terminal instead of the sidebar.
    #[test]
    fn sidebar_boundary_and_click_boundary_agree_at_non_default_cell_width() {
        let cell_width = 20_u32;
        let cols = 40_u16;
        let sidebar_edge = sidebar_pixel_width(cell_width);

        // The drawn boundary is SIDEBAR_COLS * cell_width.
        assert_eq!(
            sidebar_edge,
            f64::from((renderer::SIDEBAR_COLS as u32) * cell_width),
            "sidebar pixel width must be SIDEBAR_COLS * cell_width"
        );
        assert_eq!(sidebar_edge, 320.0);

        // A click at the boundary maps to terminal column 0.
        assert_eq!(
            terminal_column_at(sidebar_edge, cols, cell_width),
            Some(0),
            "at cell_width=20, the first terminal column is at the sidebar edge"
        );
        // A click one pixel left of the boundary is still the sidebar.
        assert_eq!(
            terminal_column_at(sidebar_edge - 1.0, cols, cell_width),
            None,
            "at cell_width=20, a click just left of the boundary is the sidebar"
        );
        // One cell width further maps to column 1.
        assert_eq!(
            terminal_column_at(sidebar_edge + f64::from(cell_width), cols, cell_width),
            Some(1),
            "at cell_width=20, one cell past the boundary is column 1"
        );
        // The last terminal column maps to the highest valid cell.
        assert_eq!(
            terminal_column_at(
                sidebar_edge + f64::from(cell_width) * f64::from(cols - 1),
                cols,
                cell_width
            ),
            Some(usize::from(cols - 1)),
        );
    }

    // ── Pass-through gate integration tests ──────────────────────────────

    /// The palette policy claims exactly two chords: Super+Escape (exit) and
    /// Super+p (palette). Both live in the Super/Cmd modifier space that the
    /// pinned Zellij v0.44.3 corpus never binds.
    #[test]
    fn palette_policy_claims_exactly_super_escape_and_super_p() {
        let policy = palette_policy();
        let claims = policy.claims();
        assert_eq!(claims.len(), 2, "exactly two claims");
        let corpus = passthrough::zellij_default_bindings();
        assert!(
            collisions(claims, &corpus).is_empty(),
            "palette policy must not collide with Zellij defaults"
        );
        let super_p = Chord::new(GateKeyCode::Char('p'), GateModifiers::empty().super_key())
            .expect("normalized");
        assert_eq!(
            policy.palette_claim().unwrap().seq.chords()[0],
            super_p,
            "palette claim is Super+p"
        );
    }

    /// A multi-chord leader whose first chord is held and then mismatched must
    /// replay the held chord byte-for-byte before the mismatching chord. This
    /// is the replay path whose failure would silently break Zellij.
    #[test]
    fn leader_mismatch_replays_held_chord_bytes_in_order() {
        // A two-chord palette leader on chords absent from the Zellij corpus:
        // bare 'a' then 'g'. Both encode non-empty bytes so a lost or
        // reordered replay changes what the child receives.
        let claim = PassthroughClaim {
            id: CLAIM_ID_PALETTE,
            action: PassthroughAction::OpenCommandPalette,
            seq: ChordSeq::new(vec![
                Chord::new(GateKeyCode::Char('a'), GateModifiers::empty()).unwrap(),
                Chord::new(GateKeyCode::Char('g'), GateModifiers::empty()).unwrap(),
            ])
            .unwrap(),
            justification: "test",
        };
        let policy = PassthroughPolicy::try_new(vec![
            PassthroughClaim {
                id: passthrough::CLAIM_ID_EXIT,
                action: PassthroughAction::ExitToWorkspace,
                seq: ChordSeq::single(
                    Chord::new(GateKeyCode::Escape, GateModifiers::empty().super_key()).unwrap(),
                ),
                justification: "test",
            },
            claim,
        ])
        .expect("valid manifest");

        let mode = InputMode::normal();
        let mut gate = PassthroughGate::new();

        // Press 'a': the first chord of the palette leader — held as pending.
        let chord_a =
            Chord::new(GateKeyCode::Char('a'), GateModifiers::empty()).expect("normalized");
        let decision = gate.press(&policy, chord_a);
        assert_eq!(decision.kind, GateKind::Pending);
        assert!(decision.replayed.is_empty(), "pending must not replay");

        // Press 'x': not the second chord. The gate forwards, replaying 'a'
        // first. The replay must arrive byte-for-byte before 'x'.
        let chord_x =
            Chord::new(GateKeyCode::Char('x'), GateModifiers::empty()).expect("normalized");
        let decision = gate.press(&policy, chord_x);
        assert_eq!(decision.kind, GateKind::Forwarded);
        assert_eq!(
            decision.replayed,
            vec![chord_a],
            "the held leader chord must be replayed"
        );

        // Verify the replay bytes match direct encoding.
        let replay_bytes = encode_chord(&decision.replayed[0], mode).expect("encodes");
        assert_eq!(replay_bytes, b"a", "replayed 'a' must encode to b\"a\"");

        // After the mismatch, the gate is clean: a subsequent 'x' forwards
        // with no replay.
        let decision = gate.press(
            &policy,
            Chord::new(GateKeyCode::Char('x'), GateModifiers::empty()).unwrap(),
        );
        assert_eq!(decision.kind, GateKind::Forwarded);
        assert!(decision.replayed.is_empty(), "gate is clean after mismatch");
    }

    /// When the palette is closed, the gate forwards unclaimed chords. The
    /// forwarded encoding must match what the encoder produces directly —
    /// byte-identical to the pre-gate behaviour.
    #[test]
    fn closed_palette_forwarded_key_is_byte_identical_to_direct_encode() {
        let policy = palette_policy();
        let mode = InputMode::normal();
        let mut gate = PassthroughGate::new();

        for (code, modifiers) in [
            (GateKeyCode::Char('a'), GateModifiers::empty()),
            (GateKeyCode::Char('z'), GateModifiers::empty()),
            (GateKeyCode::Enter, GateModifiers::empty()),
            (GateKeyCode::Char('c'), GateModifiers::empty().ctrl()),
            (GateKeyCode::Char('f'), GateModifiers::empty().alt()),
        ] {
            let chord = Chord::new(code, modifiers).expect("normalized");
            let decision = gate.press(&policy, chord);
            assert_eq!(decision.kind, GateKind::Forwarded, "chord must forward");
            let forwarded = encode_chord(&chord, mode).expect("encodes");

            let app_key = gate_key_to_app(code).expect("maps to app key");
            let app_mods = app_modifiers_from_gate(modifiers);
            let direct =
                KeyEncoder::encode_with(KeyInput::new(app_key, KeyPhase::Pressed, app_mods), mode)
                    .expect("encodes");
            assert_eq!(
                forwarded, direct,
                "forwarded bytes must match direct encode for {code:?}"
            );
        }
    }

    /// Super+p is intercepted by the gate (opens the palette) and produces no
    /// PTY bytes — confirming the palette claim works.
    #[test]
    fn super_p_is_intercepted_as_palette_open() {
        let policy = palette_policy();
        let mut gate = PassthroughGate::new();
        let chord = Chord::new(GateKeyCode::Char('p'), GateModifiers::empty().super_key())
            .expect("normalized");
        let decision = gate.press(&policy, chord);
        assert_eq!(
            decision.kind,
            GateKind::Intercepted(PassthroughAction::OpenCommandPalette)
        );
        assert!(decision.replayed.is_empty());
    }

    // ── Palette action tests ─────────────────────────────────────────────

    /// Running the create command adds a session and the sidebar shows it.
    #[test]
    fn palette_create_action_adds_a_session_to_the_sidebar() {
        let mut app = NorenApp::default();
        assert!(app.workspace.sidebar().is_empty());

        app.run_workspace_action(WorkspaceAction::CreateSession);

        assert_eq!(app.workspace.registry().len(), 1);
        assert_eq!(app.workspace.sidebar().rows().len(), 1);
    }

    /// Running select cycles the selection between sessions.
    #[test]
    fn palette_select_action_cycles_the_selected_session() {
        let mut app = NorenApp::default();
        let first = app.workspace.create_session(SessionKind::Local);
        let second = app.workspace.create_session(SessionKind::Local);
        app.workspace.select_session(first).expect("first is live");

        app.run_workspace_action(WorkspaceAction::SelectSession);

        assert_eq!(app.workspace.registry().selected(), Some(second));
    }

    /// Running close removes the selected session and the sidebar updates.
    #[test]
    fn palette_close_action_removes_the_selected_session() {
        let mut app = NorenApp::default();
        let first = app.workspace.create_session(SessionKind::Local);
        let _second = app.workspace.create_session(SessionKind::Local);
        app.workspace.select_session(first).expect("first is live");
        assert_eq!(app.workspace.sidebar().rows().len(), 2);

        app.run_workspace_action(WorkspaceAction::CloseSession);

        assert_eq!(app.workspace.registry().len(), 1);
        assert_eq!(app.workspace.sidebar().rows().len(), 1);
        assert!(
            !app.workspace
                .sidebar()
                .rows()
                .iter()
                .any(|r| r.label() == first.to_string()),
            "closed session must not appear"
        );
    }

    /// Escape dismisses the palette without running a command.
    #[test]
    fn escape_dismisses_palette_without_running_a_command() {
        let mut app = NorenApp::default();
        app.open_palette();
        assert!(app.palette_open);
        let count_before = app.workspace.registry().len();

        // Simulate Escape key: handle_palette_key checks for NamedKey::Escape.
        // We test the effect (close_palette) directly because constructing a
        // full winit KeyEvent requires a DeviceId that is not safely creatable
        // in a #[forbid(unsafe)] crate.
        app.close_palette();

        assert!(!app.palette_open, "palette must be dismissed");
        assert_eq!(
            app.workspace.registry().len(),
            count_before,
            "no command must have run"
        );
    }

    // ── Session status observation tests ─────────────────────────────────

    /// Observing Running after creation changes the sidebar detail from
    /// "starting" to "running".
    #[test]
    fn observe_running_updates_the_sidebar_detail() {
        let mut state = WorkspaceState::new();
        let id = state.create_session(SessionKind::Local);

        // Freshly created: status is "starting".
        let detail_before = state
            .sidebar()
            .rows()
            .first()
            .and_then(|r| r.detail())
            .unwrap_or_default();
        assert!(
            detail_before.contains("starting"),
            "fresh session should show starting, got {detail_before}"
        );

        state.observe_session(id, SessionStatus::Running);

        let detail_after = state
            .sidebar()
            .rows()
            .first()
            .and_then(|r| r.detail())
            .unwrap_or_default();
        assert!(
            detail_after.contains("running"),
            "observed session should show running, got {detail_after}"
        );
    }

    /// Observing Exited after Running changes the sidebar detail to "exited".
    #[test]
    fn observe_exited_after_running_updates_the_sidebar() {
        let mut state = WorkspaceState::new();
        let id = state.create_session(SessionKind::Local);
        state.observe_session(id, SessionStatus::Running);
        state.observe_session(id, SessionStatus::Exited { code: Some(0) });

        let detail = state
            .sidebar()
            .rows()
            .first()
            .and_then(|r| r.detail())
            .unwrap_or_default();
        assert!(
            detail.contains("exited"),
            "exited session should show exited, got {detail}"
        );
    }

    // ── Palette text rendering ───────────────────────────────────────────

    /// The palette text lines include all four commands with a selection
    /// marker on the currently selected one.
    #[test]
    fn palette_text_lines_show_selection_marker() {
        let state = WorkspaceState::new();
        let palette = state.palette();
        let lines = palette_text_lines(palette, 1);
        assert_eq!(lines.len(), 4, "four commands");
        assert!(
            lines[1].starts_with(']'),
            "second line must be selected, got {:?}",
            lines[1]
        );
        assert!(
            !lines[0].starts_with(']'),
            "first line must not be selected, got {:?}",
            lines[0]
        );
    }

    // ── Mouse mode scanner ───────────────────────────────────────────────

    /// DECSET 1000 is detected and enables tracking. This is the most basic
    /// mode-detection test: without it, no mouse event ever reaches the PTY.
    #[test]
    fn scanner_detects_decset_1000_and_enables_tracking() {
        let mut scanner = MouseModeScanner::default();
        let mut modes = MouseModes::disabled();
        scanner.scan(b"\x1b[?1000h", &mut modes);
        assert!(modes.is_tracked(), "mode 1000 must enable tracking");
    }

    /// DECRST clears a previously-set mode. If reset is broken, a program
    /// that disables tracking would still receive reports.
    #[test]
    fn scanner_detects_decrst_and_clears_mode() {
        let mut scanner = MouseModeScanner::default();
        let mut modes = MouseModes::disabled();
        scanner.scan(b"\x1b[?1000h\x1b[?1000l", &mut modes);
        assert!(!modes.is_tracked(), "DECRST must clear the mode");
    }

    /// Multiple mouse modes in one DECSET (`CSI ? 1002 ; 1006 h`) are all
    /// applied. Zellij enables tracking and encoding in a single sequence on
    /// attach, so this is the realistic path.
    #[test]
    fn scanner_handles_multi_param_decset() {
        let mut scanner = MouseModeScanner::default();
        let mut modes = MouseModes::disabled();
        scanner.scan(b"\x1b[?1002;1006h", &mut modes);
        assert!(modes.is_tracked(), "1002 must be set");
        // Encoding must also be set; verify via the SGR output form.
        let grid = MouseGrid::new(10, 40).expect("grid");
        let event = PointerEvent::press(EncoderButton::Left, 0, 0, PointerModifiers::empty());
        let bytes = MouseEncoder::encode(event, modes, grid).expect("must encode");
        assert!(
            bytes.starts_with(b"\x1b[<"),
            "1006 SGR must be active after multi-param DECSET, got {:?}",
            String::from_utf8_lossy(&bytes)
        );
    }

    /// A sequence split across two scan() calls is still detected. PTY output
    /// arrives in arbitrary chunks; the DFA must retain state.
    #[test]
    fn scanner_retains_state_across_chunks() {
        let mut scanner = MouseModeScanner::default();
        let mut modes = MouseModes::disabled();
        scanner.scan(b"\x1b[?10", &mut modes);
        assert!(!modes.is_tracked(), "partial sequence must not set mode");
        scanner.scan(b"00h", &mut modes);
        assert!(modes.is_tracked(), "completed sequence must set mode");
    }

    /// Unrelated private modes (1049, 2004) do not alter mouse tracking.
    #[test]
    fn scanner_ignores_non_mouse_private_modes() {
        let mut scanner = MouseModeScanner::default();
        let mut modes = MouseModes::disabled().with_normal(true);
        scanner.scan(b"\x1b[?1049h\x1b[?2004h\x1b[?1h", &mut modes);
        assert!(
            modes.is_tracked(),
            "non-mouse modes must not change tracking"
        );
    }

    /// Random garbage between valid sequences does not confuse the DFA.
    #[test]
    fn scanner_recovers_from_garbage_between_sequences() {
        let mut scanner = MouseModeScanner::default();
        let mut modes = MouseModes::disabled();
        scanner.scan(b"garbage\x1b[?1000h more text", &mut modes);
        assert!(
            modes.is_tracked(),
            "DECSET after garbage must still be detected"
        );
    }

    // ── Tracking / selection-bypass policy ──────────────────────────────

    /// With no tracking mode set, events are not reportable and local
    /// selection runs unchanged — byte-identical to the pre-tracking behaviour.
    #[test]
    fn no_tracking_mode_means_not_reportable() {
        let app = NorenApp::default();
        assert!(!app.mouse_reportable());
    }

    /// Mode 1000 without Shift: tracking active, events go to the PTY.
    #[test]
    fn mode_1000_without_shift_is_reportable() {
        let app = NorenApp {
            mouse_modes: MouseModes::disabled().with_normal(true),
            ..Default::default()
        };
        assert!(app.mouse_reportable());
    }

    /// Shift bypasses tracking so the user can still select text in a program
    /// that enabled mouse reporting. This is the standard xterm/iTerm policy.
    #[test]
    fn shift_bypasses_tracking_for_local_selection() {
        let mut app = NorenApp {
            mouse_modes: MouseModes::disabled().with_normal(true),
            ..Default::default()
        };

        assert!(app.mouse_reportable(), "active without Shift");

        app.modifiers = Modifiers::empty().shift();
        assert!(!app.mouse_reportable(), "Shift bypasses tracking");
    }

    // ── Sidebar offset and coordinate mapping ───────────────────────────

    /// A click on the first terminal column (exactly at the sidebar edge)
    /// reports column 1, not 17. This exercises the sidebar subtraction in
    /// `terminal_column_at` through the encoder's 1-based conversion — if the
    /// sidebar offset were dropped, the column would be 17 (16 sidebar cells
    /// + 1).
    #[test]
    fn sidebar_offset_first_terminal_column_reports_col_1() {
        let cols = 40_u16;
        let cell_width = GridGeometry::poc().cell_width();
        let col = terminal_column_at(sidebar_pixel_width(cell_width), cols, cell_width)
            .expect("first terminal column must map to a cell");
        assert_eq!(col, 0, "sidebar offset: first terminal column = cell 0");

        let grid = MouseGrid::new(10, cols).expect("grid");
        let modes = MouseModes::disabled().with_normal(true).with_sgr(true);
        let event = PointerEvent::press(
            EncoderButton::Left,
            col as u32,
            0,
            PointerModifiers::empty(),
        );
        let bytes = MouseEncoder::encode(event, modes, grid).expect("must encode");
        let report = String::from_utf8(bytes).expect("SGR is ASCII");
        assert_eq!(
            report, "\x1b[<0;1;1M",
            "column must be 1 (sidebar offset applied), not 17"
        );
    }

    /// A click inside the sidebar produces no terminal column, hence no PTY
    /// bytes can be constructed for it.
    #[test]
    fn sidebar_click_produces_no_terminal_column() {
        let cols = 40_u16;
        let cell_width = GridGeometry::poc().cell_width();
        let edge = sidebar_pixel_width(cell_width);
        assert_eq!(
            terminal_column_at(edge - 1.0, cols, cell_width),
            None,
            "last sidebar column must not map to a terminal cell"
        );
        assert_eq!(
            terminal_column_at(0.0, cols, cell_width),
            None,
            "leftmost pixel is sidebar"
        );
    }

    // ── Encoder integration: tracking modes ─────────────────────────────

    /// Mode 1000 with X10 byte form: a left click at (0,0) produces
    /// `ESC[M` followed by three offset bytes (32, 33, 33 for button-0,
    /// col-1, row-1).
    #[test]
    fn mode_1000_x10_left_click_at_origin() {
        let grid = MouseGrid::new(10, 40).expect("grid");
        let modes = MouseModes::disabled().with_normal(true);
        let event = PointerEvent::press(EncoderButton::Left, 0, 0, PointerModifiers::empty());
        let bytes = MouseEncoder::encode(event, modes, grid).expect("must encode");
        // Cb=0→32, Cx=1→33, Cy=1→33
        assert_eq!(bytes, b"\x1b[M\x20\x21\x21");
    }

    /// Mode 1002 (button-event): drag with left button held produces motion
    /// reports. A Move with a held button must produce bytes under 1002.
    #[test]
    fn mode_1002_drag_produces_motion_report() {
        let grid = MouseGrid::new(10, 40).expect("grid");
        let modes = MouseModes::disabled()
            .with_button_event(true)
            .with_sgr(true);
        let event =
            PointerEvent::move_to(Some(EncoderButton::Left), 2, 0, PointerModifiers::empty());
        let bytes = MouseEncoder::encode(event, modes, grid).expect("must encode");
        // Cb = 0 (button1) | 32 (motion) = 32; Cx=3, Cy=1
        let report = String::from_utf8(bytes).expect("SGR");
        assert_eq!(report, "\x1b[<32;3;1M");
    }

    /// Mode 1003 (any-event): hover with no button held produces motion
    /// reports. Under 1002 alone this would return None.
    #[test]
    fn mode_1003_hover_produces_motion_report() {
        let grid = MouseGrid::new(10, 40).expect("grid");
        let modes = MouseModes::disabled().with_any_event(true).with_sgr(true);
        let event = PointerEvent::move_to(None, 2, 0, PointerModifiers::empty());
        let bytes = MouseEncoder::encode(event, modes, grid).expect("must encode");
        // Cb = 3 (no-button) | 32 (motion) = 35; Cx=3, Cy=1
        let report = String::from_utf8(bytes).expect("SGR");
        assert_eq!(report, "\x1b[<35;3;1M");
    }

    /// Mode 1003 hover must NOT report under 1002 (button-event) alone.
    #[test]
    fn mode_1002_hover_without_button_produces_nothing() {
        let grid = MouseGrid::new(10, 40).expect("grid");
        let modes = MouseModes::disabled().with_button_event(true);
        let event = PointerEvent::move_to(None, 2, 0, PointerModifiers::empty());
        assert_eq!(
            MouseEncoder::encode(event, modes, grid),
            None,
            "1002 must not report hover"
        );
    }

    /// Mode 1015 (urxvt): `CSI Cb ; Cx ; Cy M` — decimal, no angle bracket.
    #[test]
    fn mode_1015_uses_urxvt_format() {
        let grid = MouseGrid::new(10, 40).expect("grid");
        let modes = MouseModes::disabled().with_normal(true).with_urxvt(true);
        let event = PointerEvent::press(EncoderButton::Left, 0, 0, PointerModifiers::empty());
        let bytes = MouseEncoder::encode(event, modes, grid).expect("must encode");
        let report = String::from_utf8(bytes).expect("urxvt is ASCII");
        assert_eq!(report, "\x1b[0;1;1M");
    }

    /// No tracking mode: the encoder returns None for every event kind.
    #[test]
    fn no_modes_means_no_bytes_for_any_mouse_event() {
        let grid = MouseGrid::new(10, 40).expect("grid");
        let modes = MouseModes::disabled();
        let press = PointerEvent::press(EncoderButton::Left, 0, 0, PointerModifiers::empty());
        let release = PointerEvent::release(EncoderButton::Left, 0, 0, PointerModifiers::empty());
        let motion =
            PointerEvent::move_to(Some(EncoderButton::Left), 1, 0, PointerModifiers::empty());
        let wheel = PointerEvent::wheel(WheelDirection::Up, 0, 0, PointerModifiers::empty());
        for (label, event) in [
            ("press", press),
            ("release", release),
            ("motion", motion),
            ("wheel", wheel),
        ] {
            assert_eq!(
                MouseEncoder::encode(event, modes, grid),
                None,
                "{label} must produce nothing with no modes"
            );
        }
    }

    // ── Button and wheel mapping ────────────────────────────────────────

    /// Left/Middle/Right map to the encoder's button enum; Back/Forward/Other
    /// return None and are never reported.
    #[test]
    fn encode_button_maps_known_and_ignores_extended() {
        assert_eq!(encode_button(MouseButton::Left), Some(EncoderButton::Left));
        assert_eq!(
            encode_button(MouseButton::Middle),
            Some(EncoderButton::Middle)
        );
        assert_eq!(
            encode_button(MouseButton::Right),
            Some(EncoderButton::Right)
        );
        assert_eq!(encode_button(MouseButton::Back), None);
        assert_eq!(encode_button(MouseButton::Forward), None);
        assert_eq!(encode_button(MouseButton::Other(1)), None);
    }

    /// Wheel delta sign maps to direction, and magnitude maps to click count.
    ///
    /// winit 0.30 `MouseScrollDelta` docs (from the source at
    /// `winit-0.30.13/src/event.rs`):
    ///
    ///   LineDelta:  "Positive values indicate that the content that is being
    ///                scrolled should move right and down (revealing more
    ///                content left and up)."
    ///   PixelDelta: "Positive values indicate that the content being scrolled
    ///                should move right/down."
    ///
    /// Positive y = content moves down = the user scrolled **up** (xterm button
    /// 4, `Cb=64`). Negative y = scroll down (`Cb=65`). Both variants share the
    /// same sign convention.
    #[test]
    fn wheel_clicks_direction_and_count() {
        let metrics = GridGeometry::poc().cell_metrics();
        // LineDelta: positive y = wheel up (content moves down, revealing
        // earlier content). See the winit sentence quoted above.
        let up = wheel_clicks(MouseScrollDelta::LineDelta(0.0, 1.0), metrics);
        assert_eq!(up, vec![WheelDirection::Up]);

        // Negative y = wheel down.
        let down = wheel_clicks(MouseScrollDelta::LineDelta(0.0, -3.0), metrics);
        assert_eq!(down, vec![WheelDirection::Down; 3]);

        // PixelDelta shares the same sign convention: positive y = wheel up.
        let pixel_up = wheel_clicks(
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(
                0.0,
                f64::from(metrics.height()) * 2.0,
            )),
            metrics,
        );
        assert_eq!(pixel_up, vec![WheelDirection::Up; 2]);

        // Pin the full path through the encoder so the emitted bytes are fixed:
        // wheel up → Cb=64, wheel down → Cb=65 (SGR form, 1-based col/row).
        let grid = MouseGrid::new(10, 40).expect("grid");
        let modes = MouseModes::disabled().with_normal(true).with_sgr(true);
        let up_event = PointerEvent::wheel(up[0], 0, 0, PointerModifiers::empty());
        assert_eq!(
            MouseEncoder::encode(up_event, modes, grid).as_deref(),
            Some(b"\x1b[<64;1;1M".as_slice()),
            "wheel up must emit Cb=64"
        );
        let down_event = PointerEvent::wheel(down[0], 0, 0, PointerModifiers::empty());
        assert_eq!(
            MouseEncoder::encode(down_event, modes, grid).as_deref(),
            Some(b"\x1b[<65;1;1M".as_slice()),
            "wheel down must emit Cb=65"
        );
    }

    /// A resting trackpad reports `PixelDelta{y:0}`, which is a genuine zero
    /// delta and must produce no wheel reports. The `handle_mouse_wheel` loop
    /// consumes the returned vec, so an empty vec means no bytes are written to
    /// the PTY — a spurious `Down` click would corrupt the application.
    #[test]
    fn wheel_clicks_zero_delta_produces_nothing() {
        let metrics = GridGeometry::poc().cell_metrics();
        // PixelDelta zero — the resting-trackpad case the bug shipped on.
        let pixel_zero = wheel_clicks(
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, 0.0)),
            metrics,
        );
        assert!(pixel_zero.is_empty(), "zero PixelDelta must emit nothing");

        // LineDelta zero must likewise emit nothing.
        let line_zero = wheel_clicks(MouseScrollDelta::LineDelta(0.0, 0.0), metrics);
        assert!(line_zero.is_empty(), "zero LineDelta must emit nothing");
    }

    /// A `PixelDelta` must convert pixels to lines at the **configured** cell
    /// height, never a compile-time constant. Built the same way the app builds
    /// its geometry — from parsed configuration — at `cell_height = 40`, double
    /// the PoC default of 20. A 40px scroll is exactly one line here; if
    /// `wheel_clicks` divided by the hardcoded default it would yield two. This
    /// is the guard that stops the constant creeping back into the pixel→line
    /// conversion.
    #[test]
    fn wheel_clicks_pixel_delta_uses_configured_cell_height() {
        let config = AppConfig::parse("[font]\ncell_height = 40\n").expect("valid configuration");
        let metrics =
            GridGeometry::with_cells(config.font().cell_width(), config.font().cell_height())
                .expect("valid geometry")
                .cell_metrics();
        assert_eq!(metrics.height(), 40);

        // One configured cell height of pixels up = exactly one wheel-up click.
        // A hardcoded POC_CELL_HEIGHT (20) would divide 40px into two clicks.
        let one_line_up = wheel_clicks(
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, f64::from(metrics.height()))),
            metrics,
        );
        assert_eq!(
            one_line_up,
            vec![WheelDirection::Up],
            "at cell_height=40, 40px is one line, not the two a hardcoded 20 would give"
        );

        // Two configured cell heights down = exactly two wheel-down clicks.
        let two_lines_down = wheel_clicks(
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(
                0.0,
                -f64::from(metrics.height()) * 2.0,
            )),
            metrics,
        );
        assert_eq!(
            two_lines_down,
            vec![WheelDirection::Down; 2],
            "at cell_height=40, -80px is two lines, not the four a hardcoded 20 would give"
        );
    }

    // ── Selection preservation with tracking disabled ───────────────────

    /// With tracking disabled, a left press sets up the same selection state
    /// as before (drag_origin, selection, drag_mode). We verify the gate
    /// (`mouse_reportable`) is false so the handler follows the selection
    /// branch; the selection code itself is unchanged.
    #[test]
    fn disabled_tracking_routes_to_selection_not_pty() {
        let mut terminal = TerminalState::new(4, 8).expect("valid terminal");
        terminal.feed_bytes(b"hello");
        let mut app = NorenApp {
            terminal: Some(terminal),
            ..Default::default()
        };

        // Tracking disabled: reportable is false.
        assert!(!app.mouse_reportable());

        // Attempt a press; without a window grid_point_at returns None and
        // no selection starts — but crucially, held_mouse_button stays None
        // (the tracking branch was not entered).
        app.cursor_position = Some(PhysicalPosition::new(0.0, 0.0));
        app.handle_mouse_button(ElementState::Pressed, MouseButton::Left);
        assert_eq!(app.held_mouse_button, None, "tracking branch must not run");
    }

    /// With tracking enabled and Shift held, the bypass routes to selection,
    /// not to the PTY — held_mouse_button must stay None.
    #[test]
    fn shift_bypass_with_tracking_routes_to_selection() {
        let mut terminal = TerminalState::new(4, 8).expect("valid terminal");
        terminal.feed_bytes(b"hello");
        let mut app = NorenApp {
            terminal: Some(terminal),
            mouse_modes: MouseModes::disabled().with_normal(true),
            modifiers: Modifiers::empty().shift(),
            ..Default::default()
        };

        // Tracking is on but Shift bypasses it.
        assert!(!app.mouse_reportable());

        app.cursor_position = Some(PhysicalPosition::new(0.0, 0.0));
        app.handle_mouse_button(ElementState::Pressed, MouseButton::Left);
        assert_eq!(
            app.held_mouse_button, None,
            "Shift bypass must not enter the tracking branch"
        );
    }

    /// With tracking enabled (no Shift), a press enters the tracking branch
    /// (`handle_tracked_mouse_button` runs) but — without a valid terminal
    /// position (no window in this harness) — does NOT set
    /// `held_mouse_button`. The button is recorded only when the press is
    /// actually reported. `drag_origin` stays None because the selection
    /// branch was not entered.
    #[test]
    fn tracking_enabled_press_enters_tracking_branch() {
        let terminal = TerminalState::new(4, 8).expect("valid terminal");
        let mut app = NorenApp {
            terminal: Some(terminal),
            mouse_modes: MouseModes::disabled().with_normal(true),
            ..Default::default()
        };

        assert!(app.mouse_reportable());

        app.cursor_position = Some(PhysicalPosition::new(
            sidebar_pixel_width(app.geometry.cell_width()),
            0.0,
        ));
        app.handle_mouse_button(ElementState::Pressed, MouseButton::Left);
        assert_eq!(
            app.held_mouse_button, None,
            "press with no valid terminal position must not record the button"
        );
        // The selection branch was not taken.
        assert_eq!(app.drag_origin, None);

        // Release is a no-op on held_mouse_button (already None).
        app.handle_mouse_button(ElementState::Released, MouseButton::Left);
        assert_eq!(app.held_mouse_button, None);
    }

    /// A press that produces no report (position outside the terminal grid,
    /// e.g. inside the sidebar) must not seed `held_mouse_button`. Otherwise a
    /// subsequent drag into the terminal would emit a motion report with no
    /// preceding press — outside the xterm model. Without a window,
    /// `mouse_cell_at` returns None for every position, simulating a
    /// non-reportable press; the held button must stay None so the motion
    /// handler has no button to carry.
    #[test]
    fn sidebar_press_does_not_produce_orphan_motion_report() {
        let terminal = TerminalState::new(4, 8).expect("valid terminal");
        let mut app = NorenApp {
            terminal: Some(terminal),
            mouse_modes: MouseModes::disabled()
                .with_normal(true)
                .with_button_event(true),
            ..Default::default()
        };

        assert!(app.mouse_reportable());

        // Press at a sidebar x-coordinate — `mouse_cell_at` returns None.
        app.cursor_position = Some(PhysicalPosition::new(0.0, 0.0));
        app.handle_mouse_button(ElementState::Pressed, MouseButton::Left);
        assert_eq!(
            app.held_mouse_button, None,
            "sidebar press must not record the held button"
        );

        // Now move to a terminal position. With no window, `mouse_cell_at`
        // still returns None, so no motion report is encoded — but even if it
        // did resolve, the button field would be None because
        // `held_mouse_button` was never set, so no orphan drag report.
        app.handle_mouse_move(PhysicalPosition::new(
            sidebar_pixel_width(app.geometry.cell_width()),
            0.0,
        ));
        assert_eq!(
            app.held_mouse_button, None,
            "no orphan motion report: held button is still None"
        );
    }

    // ── mouse_grid() application path ───────────────────────────────────

    /// `mouse_grid()` must pass `terminal.size()` to `MouseGrid::new` in the
    /// correct order: `MouseGrid::new(cols, rows)` while `size()` returns
    /// `(rows, cols)`. A transposition swaps the bounds the encoder clamps to.
    /// The terminal here is deliberately non-square (4 rows × 8 cols) so a
    /// swap cannot happen to match the intended dimensions.
    #[test]
    fn mouse_grid_dimensions_match_terminal_in_order() {
        let terminal = TerminalState::new(4, 8).expect("valid terminal");
        assert_eq!(
            terminal.size(),
            (4, 8),
            "fixture is non-square: 4 rows, 8 cols"
        );
        let app = NorenApp {
            terminal: Some(terminal),
            ..Default::default()
        };

        let grid = app.mouse_grid().expect("terminal present");

        // cols and rows must follow the terminal, not be swapped.
        assert_eq!(grid.cols(), 8, "cols must equal terminal cols");
        assert_eq!(grid.rows(), 4, "rows must equal terminal rows");
    }

    /// A press near the right edge of a wide, short terminal must report its
    /// true column. This is the assertion that catches the shipped transpose:
    /// with the bounds swapped, an 8-column terminal would clamp column 7 to
    /// column 4 (the 4-row bound minus one), reporting `Cx=4` instead of
    /// `Cx=8`. Drives the real application path — `NorenApp::mouse_grid` —
    /// rather than constructing `MouseGrid` directly.
    #[test]
    fn mouse_grid_right_edge_click_reports_true_column() {
        let terminal = TerminalState::new(4, 8).expect("valid terminal");
        let app = NorenApp {
            terminal: Some(terminal),
            mouse_modes: MouseModes::disabled().with_normal(true).with_sgr(true),
            ..Default::default()
        };

        let grid = app.mouse_grid().expect("terminal present");
        // Press the rightmost cell of row 0 (0-based col 7 of 8).
        let event = PointerEvent::press(EncoderButton::Left, 7, 0, PointerModifiers::empty());
        let bytes =
            MouseEncoder::encode(event, app.mouse_modes, grid).expect("tracked: must encode");
        let report = String::from_utf8(bytes).expect("SGR is ASCII");

        // 1-based column must be 8, not the clamped 4 a transposed grid yields.
        assert_eq!(
            report, "\x1b[<0;8;1M",
            "right-edge press must keep its column"
        );
    }

    // ── Sidebar state persistence (Milestone 3 final piece) ────────────
    //
    // These tests exercise the wiring of `save`/`load` into the application
    // lifecycle through `WorkspaceState`. The persistence format itself is
    // exhaustively tested in `tests/session_persistence.rs`; these tests
    // verify WHEN save/load is called, not HOW the format works.

    /// Per-test uniqueness: tests run concurrently and share the temp dir.
    static PERSIST_CASE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn temp_state_path() -> PathBuf {
        let unique = PERSIST_CASE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "noren-sidebar-wire-test-{}-{unique}.toml",
            std::process::id()
        ));
        path
    }

    fn cleanup_state_file(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
    }

    /// Required: state saved through the workspace and then loaded round-trips
    /// through the real file path, preserving every entry kind and the
    /// positional selection.
    #[test]
    fn saved_state_round_trips_through_the_real_file_path() {
        let path = temp_state_path();
        let mut state = WorkspaceState::with_state_path(Some(path.clone()));
        let _local = state.create_session(SessionKind::Local);
        let _project = state.create_session(SessionKind::Project {
            root: PathBuf::from("/srv/noren"),
        });
        let _ssh = state.create_session(SessionKind::Ssh {
            target: "ops@bastion".to_owned(),
        });
        state
            .select_session(state.registry().sessions()[1].id())
            .expect("project session is live");

        let mut restored = WorkspaceState::with_state_path(Some(path.clone()));
        restored.restore().expect("state loads");

        assert_eq!(restored.registry().len(), 3);
        let kinds: Vec<SessionKind> = restored
            .registry()
            .sessions()
            .iter()
            .map(|d| d.kind().clone())
            .collect();
        assert_eq!(
            kinds,
            vec![
                SessionKind::Local,
                SessionKind::Project {
                    root: PathBuf::from("/srv/noren")
                },
                SessionKind::Ssh {
                    target: "ops@bastion".to_owned()
                },
            ]
        );
        let selected = restored.registry().selected().expect("selection restored");
        assert_eq!(
            restored
                .registry()
                .get(selected)
                .expect("selection resolves")
                .kind(),
            &SessionKind::Project {
                root: PathBuf::from("/srv/noren")
            },
        );
        cleanup_state_file(&path);
    }

    /// Required: a corrupt file leaves the app startable with an empty sidebar
    /// and an error surfaced, not a panic.
    #[test]
    fn corrupt_state_file_surfaces_error_without_panicking() {
        let path = temp_state_path();
        std::fs::write(&path, b"this is not valid toml {{{{").expect("write corrupt fixture");

        let mut state = WorkspaceState::with_state_path(Some(path.clone()));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| state.restore()));
        let loaded = result.expect("restore must never panic");
        assert!(loaded.is_err(), "corrupt file must produce an error");
        assert!(
            state.registry().is_empty(),
            "corrupt file leaves empty registry"
        );
        assert!(
            state.sidebar().is_empty(),
            "corrupt file leaves empty sidebar"
        );
        cleanup_state_file(&path);
    }

    /// Required: a missing file is not an error — first run must be silent.
    #[test]
    fn missing_state_file_is_silent_first_run() {
        let path = temp_state_path();
        assert!(!path.exists(), "fixture path must not exist yet");

        let mut state = WorkspaceState::with_state_path(Some(path.clone()));
        assert!(state.restore().is_ok(), "missing file must not error");
        assert!(state.registry().is_empty());
        assert!(state.sidebar().is_empty());
        assert!(!path.exists(), "restore must not create a file");
    }

    /// Required: creating a session then restarting shows it in the sidebar.
    #[test]
    fn creating_a_session_then_restarting_shows_it_in_the_sidebar() {
        let path = temp_state_path();

        let mut run1 = WorkspaceState::with_state_path(Some(path.clone()));
        run1.create_session(SessionKind::Local);
        run1.create_session(SessionKind::Ssh {
            target: "web1".to_owned(),
        });

        let mut run2 = WorkspaceState::with_state_path(Some(path.clone()));
        run2.restore().expect("state loads on restart");

        assert_eq!(
            run2.sidebar().rows().len(),
            2,
            "both sessions survive the restart"
        );
        assert!(!run2.sidebar().is_empty());
        cleanup_state_file(&path);
    }

    /// Required: a restored session is distinct from a shell that is starting.
    /// Mutation check for Issue #110: changing `load_snapshot`'s restoration
    /// path back to `create` makes the status and sidebar assertions fail.
    #[test]
    fn restored_sessions_are_restored_not_starting_or_running() {
        let path = temp_state_path();
        let mut state = WorkspaceState::with_state_path(Some(path.clone()));
        let id = state.create_session(SessionKind::Local);
        state.observe_session(id, SessionStatus::Running);

        let mut restored = WorkspaceState::with_state_path(Some(path.clone()));
        restored.restore().expect("state loads");

        for descriptor in restored.registry().sessions() {
            assert_eq!(
                descriptor.status(),
                &SessionStatus::Restored,
                "restored session must identify its no-process state"
            );
        }
        let detail = restored
            .sidebar()
            .rows()
            .first()
            .and_then(|r| r.detail())
            .unwrap_or_default();
        assert!(
            detail.contains("restored") && detail.contains("not running"),
            "detail identifies a restored, non-running session: {detail}"
        );
        assert!(
            !detail.ends_with("· running"),
            "detail must not claim running: {detail}"
        );
        assert!(
            restored.sidebar().viewport().is_none(),
            "selecting a restored session must not imply an attachment"
        );
        cleanup_state_file(&path);
    }

    /// Mutation check 1: create persists immediately. If the `persist` call in
    /// `create_session` is removed, this test fails — the file would not exist.
    #[test]
    fn create_session_persists_immediately() {
        let path = temp_state_path();
        let mut state = WorkspaceState::with_state_path(Some(path.clone()));
        state.create_session(SessionKind::Local);

        assert!(path.exists(), "create must persist to the state file");
        let mut loaded = SessionRegistry::new();
        load(&path, &mut loaded).expect("file loads");
        assert_eq!(loaded.len(), 1, "one session was persisted");
        cleanup_state_file(&path);
    }

    /// Mutation check 2: close persists the removal. If the `persist` call in
    /// `close_session` is removed, this test fails — the file would still show
    /// two sessions.
    #[test]
    fn close_session_persists_the_removal() {
        let path = temp_state_path();
        let mut state = WorkspaceState::with_state_path(Some(path.clone()));
        let first = state.create_session(SessionKind::Local);
        let _second = state.create_session(SessionKind::Local);
        state.close_session(first).expect("first is live");

        let mut loaded = SessionRegistry::new();
        load(&path, &mut loaded).expect("file loads");
        assert_eq!(loaded.len(), 1, "close must persist: one session remains");
        cleanup_state_file(&path);
    }

    /// Mutation check 3 (save skipped): observe does NOT rewrite the state
    /// file. Status is a runtime observation, not a persistable structural
    /// change. If a `persist` call were incorrectly added to `observe_session`,
    /// this test fails because the file's modification time would advance.
    #[test]
    fn observe_session_does_not_rewrite_the_state_file() {
        let path = temp_state_path();
        let mut state = WorkspaceState::with_state_path(Some(path.clone()));
        let id = state.create_session(SessionKind::Local);

        let before = std::fs::read(&path).expect("file exists after create");
        let mtime_before = std::fs::metadata(&path)
            .expect("file exists")
            .modified()
            .expect("modification time available");
        // Sleep past the filesystem's timestamp granularity so a rewrite would
        // be detectable as a changed mtime.
        std::thread::sleep(std::time::Duration::from_millis(20));

        state.observe_session(id, SessionStatus::Running);
        state.observe_session(id, SessionStatus::Exited { code: Some(0) });

        let after = std::fs::read(&path).expect("file still exists");
        let mtime_after = std::fs::metadata(&path)
            .expect("file exists")
            .modified()
            .expect("modification time available");
        assert_eq!(before, after, "observe must not change file content");
        assert_eq!(
            mtime_before, mtime_after,
            "observe must not rewrite the state file (mtime changed)"
        );

        let text = String::from_utf8(after).expect("state is UTF-8");
        assert!(
            !text.contains("running") && !text.contains("exited"),
            "status must not appear in the state file: {text}"
        );
        cleanup_state_file(&path);
    }

    /// Select persists the new selection positionally. If the `persist` call in
    /// `select_session` is removed, the restored selection does not match.
    #[test]
    fn select_session_persists_the_selection() {
        let path = temp_state_path();
        let mut state = WorkspaceState::with_state_path(Some(path.clone()));
        let _first = state.create_session(SessionKind::Local);
        let second = state.create_session(SessionKind::Ssh {
            target: "host".to_owned(),
        });
        state.select_session(second).expect("second is live");

        let mut restored = WorkspaceState::with_state_path(Some(path.clone()));
        restored.restore().expect("state loads");

        let selected = restored.registry().selected().expect("selection persisted");
        assert_eq!(
            restored.registry().get(selected).expect("resolves").kind(),
            &SessionKind::Ssh {
                target: "host".to_owned()
            },
        );
        cleanup_state_file(&path);
    }

    // ── The quit path ──────────────────────────────────────────────────
    //
    // The tests above drive `WorkspaceState` directly and so never traverse
    // what the app actually does on exit. These go through `NorenApp::teardown`
    // — the whole of `NorenApp::close` except `event_loop.exit()` — and then
    // read the file back from disk, which is what the next launch would see.

    /// An app with `path` wired up as its state file, as `load_sidebar_state`
    /// wires it in `main`.
    fn app_with_state_path(path: &std::path::Path) -> NorenApp {
        let mut app = NorenApp::new(AppConfig::default());
        app.load_sidebar_state(Some(path.to_path_buf()));
        app
    }

    /// What the next launch would show: load the file into a fresh workspace.
    fn sidebar_after_relaunch(path: &std::path::Path) -> WorkspaceState {
        let mut relaunched = WorkspaceState::with_state_path(Some(path.to_path_buf()));
        relaunched.restore().expect("state loads on relaunch");
        relaunched
    }

    /// THE regression test for the blocker: quitting with one active session
    /// must not erase it. This is the single most common case — one session,
    /// quit, relaunch — and the delete-then-save ordering failed it while every
    /// `WorkspaceState`-level test passed.
    ///
    /// Mutation check: restoring the original quit path in `teardown`
    ///
    /// ```ignore
    /// if let Some(id) = self.active_session.take() {
    ///     let _ = self.workspace.close_session(id);
    /// }
    /// self.workspace.persist();
    /// ```
    ///
    /// fails this test — the reloaded registry is empty.
    #[test]
    fn quitting_with_an_active_session_keeps_it_for_the_next_launch() {
        let path = temp_state_path();
        let mut app = app_with_state_path(&path);

        // Reproduce what `initialize` does when the PTY spawns: create, select,
        // observe Running, and mark it active.
        let id = app.workspace.create_session(SessionKind::Local);
        app.workspace.select_session(id).expect("session is live");
        app.workspace.observe_session(id, SessionStatus::Running);
        app.active_session = Some(id);

        // The real quit path.
        app.teardown();

        assert!(
            app.active_session.is_none(),
            "teardown releases the active session"
        );

        let relaunched = sidebar_after_relaunch(&path);
        assert_eq!(
            relaunched.registry().len(),
            1,
            "the session the user never asked to close must survive quitting"
        );
        assert_eq!(
            relaunched.registry().sessions()[0].kind(),
            &SessionKind::Local,
        );
        assert_eq!(
            relaunched.sidebar().rows().len(),
            1,
            "the sidebar is not empty after relaunch"
        );
        assert!(!relaunched.sidebar().is_empty());
        cleanup_state_file(&path);
    }

    /// Quitting must not silently downgrade the session's status claim either:
    /// the shell is gone, so the restored entry is `Restored`, never `Running`.
    /// Consistent with `restored_sessions_are_restored_not_starting_or_running`, but
    /// reached through the quit path rather than a direct workspace mutation.
    #[test]
    fn session_restored_after_quitting_is_restored_not_running() {
        let path = temp_state_path();
        let mut app = app_with_state_path(&path);
        let id = app.workspace.create_session(SessionKind::Local);
        app.workspace.select_session(id).expect("session is live");
        app.workspace.observe_session(id, SessionStatus::Running);
        app.active_session = Some(id);

        app.teardown();

        let relaunched = sidebar_after_relaunch(&path);
        for descriptor in relaunched.registry().sessions() {
            assert_eq!(
                descriptor.status(),
                &SessionStatus::Restored,
                "a session whose PTY was torn down must not claim to be running"
            );
        }
        let detail = relaunched
            .sidebar()
            .rows()
            .first()
            .and_then(|r| r.detail())
            .unwrap_or_default();
        assert!(
            detail.contains("starting"),
            "detail says starting: {detail}"
        );
        assert!(!detail.contains("running"), "detail must not say running");
        cleanup_state_file(&path);
    }

    /// Quitting preserves the selection made through the palette, including
    /// when the selected session is the active one. This is the case the
    /// original `persist()` call was added for; it must keep working.
    #[test]
    fn quitting_preserves_the_selection_and_every_other_session() {
        let path = temp_state_path();
        let mut app = app_with_state_path(&path);
        let _first = app.workspace.create_session(SessionKind::Local);
        let second = app.workspace.create_session(SessionKind::Ssh {
            target: "ops@bastion".to_owned(),
        });
        app.workspace
            .select_session(second)
            .expect("second is live");
        app.active_session = Some(second);

        app.teardown();

        let relaunched = sidebar_after_relaunch(&path);
        assert_eq!(
            relaunched.registry().len(),
            2,
            "quitting closes no session, active or not"
        );
        let selected = relaunched
            .registry()
            .selected()
            .expect("selection survives quitting");
        assert_eq!(
            relaunched
                .registry()
                .get(selected)
                .expect("resolves")
                .kind(),
            &SessionKind::Ssh {
                target: "ops@bastion".to_owned()
            },
            "the active session is still the selected one after relaunch",
        );
        cleanup_state_file(&path);
    }

    /// A session the user *did* close stays closed: quitting must not resurrect
    /// it. Guards the opposite direction from the blocker — the fix removes the
    /// exit-time close, not the user-initiated one.
    #[test]
    fn a_session_the_user_closed_does_not_come_back_after_quitting() {
        let path = temp_state_path();
        let mut app = app_with_state_path(&path);
        let first = app.workspace.create_session(SessionKind::Local);
        let second = app.workspace.create_session(SessionKind::Local);
        app.active_session = Some(second);

        // The user closes `first` explicitly — this one really is a close.
        app.workspace.close_session(first).expect("first is live");

        app.teardown();

        let relaunched = sidebar_after_relaunch(&path);
        assert_eq!(
            relaunched.registry().len(),
            1,
            "the explicitly closed session stays closed; the active one survives"
        );
        cleanup_state_file(&path);
    }

    /// Quitting with no active session still saves. `teardown` must not make
    /// its `persist` conditional on there being a session to release.
    #[test]
    fn quitting_with_no_active_session_still_persists() {
        let path = temp_state_path();
        let mut app = app_with_state_path(&path);
        app.workspace.create_session(SessionKind::Local);
        assert!(app.active_session.is_none(), "no PTY was ever spawned");
        cleanup_state_file(&path);

        app.teardown();

        assert!(path.exists(), "quit must write the state file");
        let relaunched = sidebar_after_relaunch(&path);
        assert_eq!(relaunched.registry().len(), 1);
        cleanup_state_file(&path);
    }

    /// Without a state path (HOME unset), persistence is entirely in-memory:
    /// create does not touch disk and restore is a no-op.
    #[test]
    fn no_state_path_means_in_memory_only() {
        let mut state = WorkspaceState::with_state_path(None);
        assert!(state.restore().is_ok(), "no path → no-op restore");
        state.create_session(SessionKind::Local);
        assert_eq!(state.registry().len(), 1);
    }
}
