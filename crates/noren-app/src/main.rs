//! macOS entry point for the bounded local-zsh PTY PoC.

mod persistence_state;
mod renderer;

use persistence_state::{AttemptOutcome, Observation, PersistenceState, SaveOutcome};

use noren_app::{
    Arrow, CellMetrics, CursorKeyMode, FunctionKey, GridGeometry, GridSize, InputMode, Key,
    KeyDropReason, KeyEncoder, KeyInput, KeyPhase, KeypadInput, KeypadKey, KeypadMode,
    MAX_RENDER_COLS, Modifiers, PARSE_BUDGET_BYTES_PER_TURN, PasteReject, Resize, SystemClipboard,
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
    session_persistence::{
        SESSION_STATE_FILE_NAME, SessionPersistenceError, load_snapshot, save_snapshot, snapshot,
    },
    sidebar::{SidebarEntry, SidebarView},
    ssh_config::{HostDiscoveryKind, SshConfig},
};
use noren_pty::{PtyEvent, PtySession, PtySize};
use noren_terminal::{
    GridPoint, Selection, SelectionMode, TerminalEngine, TerminalError, TerminalState,
};
use renderer::{RenderOutcome, Renderer};
use std::collections::HashMap;
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
/// Keep configured-host memory and identity work bounded independently of the
/// frame height. The sidebar exposes a scroll window over this bounded list.
const MAX_SSH_SIDEBAR_HOSTS: usize = 24;
/// Sidebar rows begin with a selection marker and one separating space.
const SIDEBAR_ROW_PREFIX_CHARS: usize = 2;
/// ASCII-only connection state that is always inside the first 16 columns.
const SSH_SIDEBAR_LABEL_PREFIX: &str = "SSH-OFF ";
const SSH_SIDEBAR_LABEL_PREFIX_CHARS: usize = 8;
const SSH_SIDEBAR_TRUNCATION_MARKER: &str = "...";
const SSH_SIDEBAR_TRUNCATION_MARKER_CHARS: usize = 3;
const SSH_SIDEBAR_LABEL_CHARS: usize = renderer::SIDEBAR_COLS - SIDEBAR_ROW_PREFIX_CHARS;
const SSH_SIDEBAR_TARGET_CHARS: usize = SSH_SIDEBAR_LABEL_CHARS - SSH_SIDEBAR_LABEL_PREFIX_CHARS;
const SSH_SIDEBAR_TRUNCATED_TARGET_CHARS: usize =
    SSH_SIDEBAR_TARGET_CHARS - SSH_SIDEBAR_TRUNCATION_MARKER_CHARS;
const SSH_SIDEBAR_DETAIL: &str = "not connected";
/// Keep the complete source identity and the partial-discovery warning visible
/// together on ordinary terminal widths. The stable source tag is placed first
/// so path truncation cannot make two retained sources indistinguishable.
const SSH_STATUS_SOURCE_CHARS: usize = 40;

/// Build the bounded display label for an SSH target without copying or even
/// scanning the complete target. The renderer counts Unicode scalar values,
/// so this helper does the same and looks at one scalar beyond the untruncated
/// target budget solely to decide whether the ASCII marker is needed.
fn ssh_sidebar_label(target: &str) -> String {
    let inspected: Vec<char> = target
        .chars()
        .take(SSH_SIDEBAR_TARGET_CHARS.saturating_add(1))
        .collect();
    let truncated = inspected.len() > SSH_SIDEBAR_TARGET_CHARS;
    let visible_target_chars = if truncated {
        SSH_SIDEBAR_TRUNCATED_TARGET_CHARS
    } else {
        inspected.len()
    };
    let mut label = String::with_capacity(SSH_SIDEBAR_LABEL_CHARS.saturating_mul(4));
    label.push_str(SSH_SIDEBAR_LABEL_PREFIX);
    label.extend(inspected.into_iter().take(visible_target_chars));
    if truncated {
        label.push_str(SSH_SIDEBAR_TRUNCATION_MARKER);
    }
    label
}

fn ssh_status_source_label(label: &str) -> String {
    let (path, tag) = label
        .rsplit_once(' ')
        .filter(|(_, tag)| tag.starts_with('#'))
        .unwrap_or((label, "#?"));
    let prefix = format!("{tag} ");
    let path_budget = SSH_STATUS_SOURCE_CHARS.saturating_sub(prefix.chars().count());
    let inspected: Vec<char> = path.chars().take(path_budget.saturating_add(1)).collect();
    let truncated = inspected.len() > path_budget;
    let visible_chars = if truncated {
        path_budget.saturating_sub(SSH_SIDEBAR_TRUNCATION_MARKER_CHARS)
    } else {
        inspected.len()
    };
    let mut result = prefix;
    result.extend(inspected.into_iter().take(visible_chars));
    if truncated {
        result.push_str(SSH_SIDEBAR_TRUNCATION_MARKER);
    }
    result
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ConfiguredSshHost {
    /// Shared session vocabulary used for target identity; this is still only
    /// a configured fact and is never inserted into the live registry.
    kind: SessionKind,
    /// Bounded, root-relative provenance supplied by `SshConfig`.
    source_label: String,
}

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
    /// Configured SSH targets represented by the shared session vocabulary.
    /// They are sidebar facts only: no registry entry or connection exists.
    ssh_hosts: Vec<ConfiguredSshHost>,
    ssh_hosts_omitted: usize,
    selected_ssh_target: Option<String>,
    selected_ssh_source_label: Option<String>,
    /// Owned by the workspace; dispatched when the palette opens.
    palette: Palette<WorkspaceAction>,
    /// Where sidebar state is persisted. `None` in tests and when `HOME` is
    /// unset; in both cases the workspace is in-memory only and
    /// [`Self::persist`] is a no-op.
    state_path: Option<PathBuf>,
    /// Exact comparison baseline plus persistence diagnostics. Baseline
    /// validity is explicit so failures cannot reuse stale or assumed bytes.
    persistence: PersistenceState,
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
            ssh_hosts: Vec::new(),
            ssh_hosts_omitted: 0,
            selected_ssh_target: None,
            selected_ssh_source_label: None,
            palette: Palette::noren(
                WorkspaceAction::CreateSession,
                WorkspaceAction::SelectSession,
                WorkspaceAction::CloseSession,
                WorkspaceAction::FocusSidebar,
            ),
            state_path,
            persistence: PersistenceState::default(),
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
        match load_snapshot(path, &mut self.registry) {
            Ok(snapshot) => self.persistence.restore_succeeded(snapshot),
            Err(error) => {
                self.persistence.restore_failed();
                return Err(error);
            }
        }
        self.rebuild_sidebar();
        Ok(())
    }

    /// Persist the current sidebar state to
    /// [`state_path`](Self::state_path), if one is set.
    ///
    /// The write is atomic (temp-file rename) inside [`save_snapshot`]; this
    /// method never bypasses it. A failure is surfaced through stderr and
    /// swallowed so the app keeps running — losing a save is preferable to
    /// crashing the terminal.
    fn persist(&mut self) {
        let Some(path) = &self.state_path else {
            return;
        };
        let before = match snapshot(path) {
            Ok(current) => Observation::Observed(current),
            Err(error) => {
                eprintln!("Noren could not inspect sidebar state before saving: {error}");
                Observation::Unavailable
            }
        };
        let save = match save_snapshot(path, &self.registry) {
            Ok(intended) => {
                let observed = match snapshot(path) {
                    Ok(current) => Observation::Observed(current),
                    Err(error) => {
                        eprintln!("Noren could not verify saved sidebar state: {error}");
                        Observation::Unavailable
                    }
                };
                SaveOutcome::Written { intended, observed }
            }
            Err(error) => {
                eprintln!("Noren could not save sidebar state: {error}");
                SaveOutcome::Failed
            }
        };
        self.persistence
            .apply_attempt(AttemptOutcome::new(before, save));
    }

    /// Whether a save observed state written by another instance since the
    /// last verified restore or save.
    fn persistence_conflict(&self) -> bool {
        self.persistence.conflict()
    }

    /// Whether the latest persistence attempt completed without enough
    /// evidence to call the on-disk state safe.
    fn persistence_unverified(&self) -> bool {
        self.persistence.unverified()
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
        self.selected_ssh_target = None;
        self.selected_ssh_source_label = None;
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

    /// Replace the configured SSH host facts without creating sessions.
    /// Returns the number omitted by the bounded sidebar policy.
    fn load_ssh_config(&mut self, config: &SshConfig) -> usize {
        self.ssh_hosts = config
            .hosts()
            .iter()
            .take(MAX_SSH_SIDEBAR_HOSTS)
            .filter_map(|host| {
                let source = config.source(host.declared_source())?;
                Some(ConfiguredSshHost {
                    kind: SessionKind::Ssh {
                        target: host.alias().to_owned(),
                    },
                    source_label: source.label().to_owned(),
                })
            })
            .collect();
        self.ssh_hosts_omitted = config.hosts().len().saturating_sub(self.ssh_hosts.len());
        self.selected_ssh_target = None;
        self.selected_ssh_source_label = None;
        self.rebuild_sidebar();
        self.ssh_hosts_omitted
    }

    /// Select an SSH row as a pending UI choice, never as a live session.
    fn select_ssh_sidebar_row(&mut self, row_index: usize) -> bool {
        let session_rows = self.registry.len();
        let host_index = row_index.checked_sub(session_rows);
        let Some(Some(ConfiguredSshHost {
            kind: SessionKind::Ssh { target },
            source_label,
        })) = host_index.map(|index| self.ssh_hosts.get(index))
        else {
            return false;
        };
        self.selected_ssh_target = Some(target.clone());
        self.selected_ssh_source_label = Some(source_label.clone());
        self.rebuild_sidebar();
        true
    }

    /// Resolve the local session id at a stable sidebar position.
    ///
    /// Session rows precede SSH facts and are generated from the registry's
    /// deterministic id ordering. The application decides whether that model
    /// entry owns the one live PTY before changing selection.
    fn local_sidebar_session(&self, row_index: usize) -> Option<SessionId> {
        if row_index >= self.registry.len() {
            return None;
        }
        self.registry
            .sessions()
            .get(row_index)
            .map(|descriptor| descriptor.id())
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
        let mut entries = entries;
        let mut pending_marked = false;
        entries.extend(self.ssh_hosts.iter().filter_map(|host| {
            let SessionKind::Ssh { target } = &host.kind else {
                return None;
            };
            let selected =
                !pending_marked && self.selected_ssh_target.as_deref() == Some(target.as_str());
            pending_marked |= selected;
            Some(SidebarEntry::SshConnection {
                label: ssh_sidebar_label(target),
                host: SSH_SIDEBAR_DETAIL.to_string(),
                selected,
            })
        }));
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

    /// The configured host that was selected, if any. Selection is a pending
    /// UI choice and deliberately does not imply a connection or viewport.
    #[cfg(test)]
    fn selected_ssh_target(&self) -> Option<&str> {
        self.selected_ssh_target.as_deref()
    }

    fn selected_ssh_source_label(&self) -> Option<&str> {
        self.selected_ssh_source_label.as_deref()
    }

    #[cfg(test)]
    fn ssh_hosts_omitted(&self) -> usize {
        self.ssh_hosts_omitted
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

/// A live session that is not currently displayed.
///
/// The app holds exactly one *active* surface (`NorenApp::pty` plus
/// `NorenApp::terminal`, owned by [`NorenApp::active_session`]); every other
/// live session is parked here with its own PTY and terminal state. Parking
/// keeps the child running and its screen authoritative in Noren's terminal
/// state, so switching back shows current content. The renderer, input
/// routing, and mouse mapping never need to know which session is focused:
/// they operate on the active surface, and switching swaps surfaces rather
/// than re-deriving state.
struct ParkedSession {
    pty: PtySession,
    terminal: TerminalState,
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
    ssh_diagnostic: Option<String>,
    ssh_selection_status: Option<String>,
    redraw_needed: bool,
    // User-initiated selection state. The renderer does not highlight it yet;
    // copy still extracts it. Any PTY output or resize invalidates it because
    // grid coordinates only address the content they were captured on.
    selection: Option<Selection>,
    drag_origin: Option<GridPoint>,
    drag_mode: SelectionMode,
    cursor_position: Option<PhysicalPosition<f64>>,
    /// The currently-held mouse button when tracking is active, or `None`.
    /// Drives the `button` field of motion (drag/hover) reports.
    held_mouse_button: Option<MouseButton>,
    workspace: WorkspaceState,
    /// First workspace row currently visible in the bounded sidebar window.
    sidebar_scroll_offset: usize,
    active_session: Option<SessionId>,
    /// Live sessions that are not the active one, keyed by session id.
    parked_sessions: HashMap<SessionId, ParkedSession>,
    palette_open: bool,
    palette_selection: usize,
    passthrough_gate: PassthroughGate,
    passthrough_policy: PassthroughPolicy,
}

/// Which application-owned line, if any, occupies the renderer's status row.
///
/// Runtime statuses take precedence while `show_status` is set. A pending SSH
/// selection then exposes its bounded provenance; otherwise a readable config
/// keeps the partial-discovery notice (or a parse failure keeps its content-free
/// diagnostic). The runtime source is also the idle fallback, making the row a
/// permanent part of the application grid rather than dynamically hiding a PTY
/// row when a notice appears.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StatusRowSource {
    Runtime,
    SshSelection,
    SshDiagnostic,
}

impl StatusRowSource {
    fn text<'a>(
        self,
        runtime: &'a str,
        ssh_selection_status: Option<&'a str>,
        ssh_diagnostic: Option<&'a str>,
    ) -> &'a str {
        match self {
            Self::Runtime => runtime,
            Self::SshSelection => {
                ssh_selection_status.expect("SSH selection source requires a provenance status")
            }
            Self::SshDiagnostic => {
                ssh_diagnostic.expect("SSH diagnostic source requires diagnostic text")
            }
        }
    }
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
            ssh_diagnostic: None,
            ssh_selection_status: None,
            redraw_needed: true,
            selection: None,
            drag_origin: None,
            drag_mode: SelectionMode::Char,
            cursor_position: None,
            held_mouse_button: None,
            workspace: WorkspaceState::new(),
            sidebar_scroll_offset: 0,
            active_session: None,
            parked_sessions: HashMap::new(),
            palette_open: false,
            palette_selection: 0,
            passthrough_gate: PassthroughGate::new(),
            passthrough_policy: palette_policy(),
        }
    }

    /// Single status-row decision shared by rendering and pointer mapping.
    fn status_row(&self) -> StatusRowSource {
        if self.show_status {
            StatusRowSource::Runtime
        } else if self.ssh_selection_status.is_some() {
            StatusRowSource::SshSelection
        } else if self.ssh_diagnostic.is_some() {
            StatusRowSource::SshDiagnostic
        } else {
            StatusRowSource::Runtime
        }
    }

    /// Whether the permanent status chrome has enough room to own a row.
    fn status_row_present(window_rows: u16) -> bool {
        window_rows > 1
    }

    /// Terminal rows available after reserving permanent application chrome.
    ///
    /// The PTY, terminal state, renderer, and pointer mapper must all agree on
    /// this value. A one-row window cannot reserve its only row for chrome;
    /// keeping one terminal row is safer than constructing an invalid zero-row
    /// PTY, so the status line is temporarily suppressed there.
    fn content_terminal_rows(window_rows: u16) -> u16 {
        window_rows - u16::from(Self::status_row_present(window_rows))
    }

    fn rendered_status_row(&self, window_rows: u16) -> Option<StatusRowSource> {
        Self::status_row_present(window_rows).then(|| self.status_row())
    }

    /// Install the terminal state and return the exactly matching PTY size.
    ///
    /// Keeping this as the initialization seam prevents the two consumers from
    /// independently reinterpreting the application-owned status row.
    fn prepare_initial_terminal(&mut self, grid: GridSize) -> Option<PtySize> {
        let runtime = RuntimeGridSize::from_window(grid);
        let terminal = runtime.terminal_state()?;
        let pty = runtime.pty_size()?;
        self.terminal = Some(terminal);
        Some(pty)
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

    /// Load the conventional `~/.ssh/config` through the bounded parser.
    /// Missing/unreadable input is an empty host list; malformed readable
    /// input becomes a content-free diagnostics/status line and never stops
    /// startup. A readable config gets an explicit partial-discovery notice.
    fn load_ssh_hosts(&mut self) {
        match SshConfig::read_default() {
            Ok(config) => self.apply_ssh_config(&config),
            Err(error) => self.report_ssh_diagnostic(error.to_string()),
        }
    }

    /// Deterministic explicit-path seam used by tests and future reload UI.
    #[cfg(test)]
    fn load_ssh_hosts_from(&mut self, path: &std::path::Path) {
        match SshConfig::read(path) {
            Ok(config) => self.apply_ssh_config(&config),
            Err(error) => self.report_ssh_diagnostic(error.to_string()),
        }
    }

    fn apply_ssh_config(&mut self, config: &SshConfig) {
        let omitted = self.workspace.load_ssh_config(config);
        self.ssh_selection_status = None;
        if config.sources().is_empty() {
            self.ssh_diagnostic = None;
        } else {
            self.ssh_diagnostic = Some(match config.discovery_kind() {
                HostDiscoveryKind::PartialLiteralPatterns if config.hosts().is_empty() => {
                    "Noren SSH: partial literal aliases; none found".to_owned()
                }
                HostDiscoveryKind::PartialLiteralPatterns if omitted == 0 => {
                    "Noren SSH: partial literal aliases; select one for source".to_owned()
                }
                HostDiscoveryKind::PartialLiteralPatterns => format!(
                    "Noren SSH: partial literal aliases; showing first \
                     {MAX_SSH_SIDEBAR_HOSTS}; {omitted} omitted"
                ),
            });
        }
        self.redraw_needed = true;
    }

    fn report_ssh_diagnostic(&mut self, detail: String) {
        let line = format!("Noren diagnostics: {detail}");
        eprintln!("{line}");
        self.ssh_selection_status = None;
        self.ssh_diagnostic = Some(line);
        self.redraw_needed = true;
    }
}

impl NorenApp {
    fn record_pty_started(&mut self) {
        self.status = "Noren PoC ready";
        self.show_status = false;
        self.pty_child = PtyChildStatus::Running;
        let session_id = self.workspace.create_session(SessionKind::Local);
        self.workspace
            .select_session(session_id)
            .expect("freshly created session is live");
        self.ssh_selection_status = None;
        self.workspace
            .observe_session(session_id, SessionStatus::Running);
        self.active_session = Some(session_id);
    }

    /// Terminal state and PTY size for a new session at the current window
    /// grid, or at the default window grid when no window exists yet
    /// (headless start, application tests).
    fn session_surfaces(&mut self) -> Option<(TerminalState, PtySize)> {
        let changed = match self.window.as_ref() {
            Some(window) => {
                let size = window.inner_size();
                self.geometry.update(Resize::new(size.width, size.height))
            }
            None => self
                .geometry
                .update(Resize::new(WINDOW_WIDTH, WINDOW_HEIGHT)),
        };
        // `update` returns `None` for an unchanged grid; the current grid is
        // then exactly what a new session should use.
        let grid = changed.or_else(|| self.geometry.current())?;
        let runtime = RuntimeGridSize::from_window(grid);
        Some((runtime.terminal_state()?, runtime.pty_size()?))
    }

    /// Park the active session's surface without killing it.
    ///
    /// The PTY keeps running and its bytes are drained into its own terminal
    /// state by [`Self::drain_parked_sessions`], so a later switch back
    /// re-attaches to current content rather than a stale frame.
    fn park_active_session(&mut self) {
        if let (Some(id), Some(pty), Some(terminal)) = (
            self.active_session.take(),
            self.pty.take(),
            self.terminal.take(),
        ) {
            self.parked_sessions
                .insert(id, ParkedSession { pty, terminal });
            self.pty_child = PtyChildStatus::NotLaunched;
        }
    }

    /// Spawn a real local PTY session and give it the live view.
    ///
    /// This is the palette `session_create` runtime: the new sidebar row is
    /// backed by an actual `/bin/zsh` PTY. The registry observes `Running`
    /// when the spawn succeeds and `Failed` when it does not — creation never
    /// claims a session is running (the registry's `Starting` contract). The
    /// new session takes the live view and the previous one is parked, not
    /// killed, matching the new-tab-focuses-itself convention of terminal
    /// multiplexers.
    fn spawn_local_session(&mut self) -> Option<SessionId> {
        let id = self.workspace.create_session(SessionKind::Local);
        let Some((terminal, pty_size)) = self.session_surfaces() else {
            self.workspace.observe_session(
                id,
                SessionStatus::Failed {
                    reason: "terminal surface unavailable".to_owned(),
                },
            );
            return None;
        };
        match PtySession::spawn(pty_size) {
            Ok(pty) => {
                self.workspace.observe_session(id, SessionStatus::Running);
                self.park_active_session();
                self.pty = Some(pty);
                self.terminal = Some(terminal);
                self.active_session = Some(id);
                self.pty_child = PtyChildStatus::Running;
                self.workspace
                    .select_session(id)
                    .expect("freshly spawned session is live");
                self.ssh_selection_status = None;
                self.redraw_needed = true;
                Some(id)
            }
            Err(_) => {
                self.workspace.observe_session(
                    id,
                    SessionStatus::Failed {
                        reason: "PTY spawn failed".to_owned(),
                    },
                );
                None
            }
        }
    }

    /// Drain every parked session's PTY events into its own terminal state.
    ///
    /// A parked session keeps producing output; its bytes feed its own
    /// authoritative terminal state under the same per-turn parse budget as
    /// the active session, so nothing grows without bound and switching back
    /// shows current content. A parked child that exits or errors is observed
    /// through the registry (`Exited`/`Failed`), shut down and reaped, and
    /// dropped from the live bookkeeping — a dead row must never stay
    /// `Running`, in the background any more than in the foreground.
    fn drain_parked_sessions(&mut self) {
        let ids: Vec<SessionId> = self
            .workspace
            .registry()
            .sessions()
            .into_iter()
            .map(|descriptor| descriptor.id())
            .filter(|id| self.parked_sessions.contains_key(id))
            .collect();
        for id in ids {
            let terminal_status = self.drain_one_parked(id);
            if let Some(status) = terminal_status {
                self.workspace.observe_session(id, status);
                if let Some(mut parked) = self.parked_sessions.remove(&id)
                    && parked.pty.shutdown().is_err()
                {
                    self.workspace.observe_session(
                        id,
                        SessionStatus::Failed {
                            reason: "PTY shutdown failed".to_owned(),
                        },
                    );
                }
                // The sidebar detail for this row changed even though the
                // visible frame did not.
                self.redraw_needed = true;
            }
        }
    }

    /// Drain one parked session's ready events, returning the observed
    /// terminal status when the child exited or the channel failed.
    fn drain_one_parked(&mut self, id: SessionId) -> Option<SessionStatus> {
        let parked = self.parked_sessions.get_mut(&id)?;
        let mut remaining = PARSE_BUDGET_BYTES_PER_TURN;
        loop {
            if remaining < noren_pty::READ_CHUNK_BYTES {
                return None;
            }
            match parked.pty.try_recv() {
                Ok(None) => return None,
                Ok(Some(PtyEvent::Output(bytes))) => {
                    if bytes.len() > remaining {
                        // Over-budget output stays queued for a later turn;
                        // it is never dropped.
                        return None;
                    }
                    remaining -= bytes.len();
                    parked.terminal.feed_bytes(&bytes);
                }
                Ok(Some(PtyEvent::Eof)) => {
                    return Some(SessionStatus::Exited { code: None });
                }
                Ok(Some(PtyEvent::Exited { code })) => {
                    return Some(SessionStatus::Exited {
                        code: code.map(|code| code as i32),
                    });
                }
                Ok(Some(PtyEvent::Error(_))) => {
                    return Some(SessionStatus::Failed {
                        reason: "PTY operation failed".to_owned(),
                    });
                }
                Err(_) => {
                    return Some(SessionStatus::Failed {
                        reason: "PTY channel closed".to_owned(),
                    });
                }
            }
        }
    }

    /// Reap every parked session's child. Bounded and idempotent per session.
    fn shutdown_parked_sessions(&mut self) {
        for (_, mut parked) in self.parked_sessions.drain() {
            if parked.pty.shutdown().is_err() {
                eprintln!("Noren PTY shutdown reached its failure fallback");
            }
        }
    }

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

        let Some(pty_size) = self.prepare_initial_terminal(grid) else {
            eprintln!("Noren terminal state creation failed");
            event_loop.exit();
            return;
        };
        self.pty = match PtySession::spawn(pty_size) {
            Ok(session) => {
                self.record_pty_started();
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
                self.spawn_local_session();
            }
            WorkspaceAction::SelectSession => {
                let ids: Vec<SessionId> = self
                    .workspace
                    .registry()
                    .sessions()
                    .into_iter()
                    .map(|d| d.id())
                    .collect();
                let Some(active) = self.active_session else {
                    return;
                };
                if ids.contains(&active) && self.workspace.select_session(active).is_ok() {
                    self.ssh_selection_status = None;
                }
            }
            WorkspaceAction::CloseSession => {
                // A live session (the active one or a parked one) owns a real
                // child; until close learns to reap (the next slice), the
                // palette closes only rows without a live surface.
                let live = |app: &Self, id| {
                    app.active_session == Some(id) || app.parked_sessions.contains_key(&id)
                };
                if let Some(id) = self.workspace.registry().selected() {
                    if !live(self, id) {
                        let _ = self.workspace.close_session(id);
                    }
                } else {
                    let ids: Vec<SessionId> = self
                        .workspace
                        .registry()
                        .sessions()
                        .into_iter()
                        .map(|d| d.id())
                        .collect();
                    if let Some(id) = ids.into_iter().find(|id| !live(self, *id)) {
                        let _ = self.workspace.close_session(id);
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
        if self.handle_sidebar_click(state, button) {
            return;
        }
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

    /// Sidebar clicks are intentionally narrow until the sidebar owns a full
    /// selection model. SSH rows show a truthful pending-selection notice and
    /// never launch or select a terminal session.
    fn handle_sidebar_click(&mut self, state: ElementState, button: MouseButton) -> bool {
        let Some(frame_size) = self.window.as_ref().map(|window| window.inner_size()) else {
            return false;
        };
        self.handle_sidebar_click_in_frame(state, button, frame_size)
    }

    /// Window-independent seam for the sidebar event path. Production passes
    /// the live inner size; tests can supply a synthetic frame without creating
    /// a platform window.
    fn handle_sidebar_click_in_frame(
        &mut self,
        state: ElementState,
        button: MouseButton,
        frame_size: PhysicalSize<u32>,
    ) -> bool {
        if self.palette_open || button != MouseButton::Left || state != ElementState::Pressed {
            return false;
        }
        let Some(position) = self.cursor_position else {
            return false;
        };
        let Some(row_index) = self.sidebar_row_index(position, frame_size) else {
            return false;
        };
        if let Some(id) = self.workspace.local_sidebar_session(row_index) {
            if Some(id) != self.active_session {
                if let Some(active) = self.active_session
                    && self.workspace.select_session(active).is_ok()
                {
                    self.ssh_selection_status = None;
                    self.redraw_needed = true;
                }
                return true;
            }
            if self.workspace.select_session(id).is_ok() {
                self.ssh_selection_status = None;
                self.redraw_needed = true;
            }
            return true;
        }
        if self.workspace.select_ssh_sidebar_row(row_index) {
            let source = self
                .workspace
                .selected_ssh_source_label()
                .map(ssh_status_source_label)
                .unwrap_or_else(|| "#? source unavailable".to_owned());
            self.ssh_selection_status = Some(format!("SSH partial source {source}; offline"));
            self.redraw_needed = true;
            return true;
        }
        false
    }

    fn sidebar_row_index(
        &self,
        position: PhysicalPosition<f64>,
        frame_size: PhysicalSize<u32>,
    ) -> Option<usize> {
        if !position.x.is_finite()
            || !position.y.is_finite()
            || position.x < 0.0
            || position.y < 0.0
            || position.x >= f64::from(frame_size.width)
            || position.y >= f64::from(frame_size.height)
            || position.x >= sidebar_pixel_width(self.geometry.cell_width())
        {
            return None;
        }
        let row = pixel_row_index(position.y, self.geometry.cell_height())?;
        let fully_drawable_rows =
            renderer::fully_drawable_rows(frame_size.height, self.geometry.cell_metrics());
        let offset = self.clamped_sidebar_scroll_offset(fully_drawable_rows);
        let row_index = offset.checked_add(row)?;
        (row < fully_drawable_rows && row_index < self.workspace.sidebar().rows().len())
            .then_some(row_index)
    }

    /// Consume a wheel event in the sidebar and move its bounded row window.
    ///
    /// This local-chrome route runs before terminal mouse tracking, so even an
    /// application using DEC mouse modes receives no PTY bytes for sidebar
    /// scrolling.
    fn handle_sidebar_wheel_in_frame(
        &mut self,
        delta: MouseScrollDelta,
        frame_size: PhysicalSize<u32>,
    ) -> bool {
        let Some(position) = self.cursor_position else {
            return false;
        };
        if !position.x.is_finite()
            || !position.y.is_finite()
            || position.x < 0.0
            || position.y < 0.0
            || position.x >= sidebar_pixel_width(self.geometry.cell_width())
            || position.x >= f64::from(frame_size.width)
            || position.y >= f64::from(frame_size.height)
        {
            return false;
        }

        let visible_rows =
            renderer::fully_drawable_rows(frame_size.height, self.geometry.cell_metrics());
        self.clamp_sidebar_scroll(visible_rows);
        let max_offset = self
            .workspace
            .sidebar()
            .rows()
            .len()
            .saturating_sub(visible_rows);
        let lines = match delta {
            MouseScrollDelta::LineDelta(_, y) => f64::from(y),
            MouseScrollDelta::PixelDelta(position) => {
                position.y / f64::from(self.geometry.cell_height())
            }
        };
        let raw_steps = lines.abs().floor() as usize;
        let steps = if raw_steps == 0 && lines != 0.0 {
            1
        } else {
            raw_steps
        }
        .min(max_offset);
        let previous = self.sidebar_scroll_offset;
        if lines < 0.0 {
            self.sidebar_scroll_offset = self
                .sidebar_scroll_offset
                .saturating_add(steps)
                .min(max_offset);
        } else if lines > 0.0 {
            self.sidebar_scroll_offset = self.sidebar_scroll_offset.saturating_sub(steps);
        }
        if self.sidebar_scroll_offset != previous {
            self.redraw_needed = true;
        }
        true
    }

    fn clamped_sidebar_scroll_offset(&self, visible_rows: usize) -> usize {
        self.sidebar_scroll_offset.min(
            self.workspace
                .sidebar()
                .rows()
                .len()
                .saturating_sub(visible_rows),
        )
    }

    fn clamp_sidebar_scroll(&mut self, visible_rows: usize) {
        self.sidebar_scroll_offset = self.clamped_sidebar_scroll_offset(visible_rows);
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
        if let Some(frame_size) = self.window.as_ref().map(|window| window.inner_size())
            && self.handle_sidebar_wheel_in_frame(delta, frame_size)
        {
            return;
        }
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

    /// Map a window pixel position to grid coordinates using the renderer's
    /// shared top-aligned row layout. Returns `None` outside rendered terminal
    /// content, including status chrome and unused underfill rows.
    fn grid_point_at(&self, position: PhysicalPosition<f64>) -> Option<GridPoint> {
        let frame_size = self.window.as_ref()?.inner_size();
        self.grid_point_in_frame(position, frame_size)
    }

    /// Window-independent seam shared by selection and mouse reporting through
    /// [`grid_point_at`](Self::grid_point_at).
    fn grid_point_in_frame(
        &self,
        position: PhysicalPosition<f64>,
        frame_size: PhysicalSize<u32>,
    ) -> Option<GridPoint> {
        if !position.x.is_finite()
            || !position.y.is_finite()
            || position.x < 0.0
            || position.y < 0.0
            || position.x >= f64::from(frame_size.width)
            || position.y >= f64::from(frame_size.height)
        {
            return None;
        }
        let terminal = self.terminal.as_ref()?;
        let cell_width = self.geometry.cell_width();
        let cell_height = self.geometry.cell_height();
        // The sidebar occupies the leftmost SIDEBAR_COLS cell columns; clicks
        // inside it do not address the terminal grid.
        if position.x < sidebar_pixel_width(cell_width) {
            return None;
        }
        let content_rows = terminal.screen().display_row_count();
        let window_rows =
            renderer::fully_drawable_rows(frame_size.height, self.geometry.cell_metrics())
                .try_into()
                .unwrap_or(u16::MAX);
        let layout = renderer::FrameRowLayout::new(
            frame_size.height,
            self.geometry.cell_metrics(),
            content_rows,
            self.rendered_status_row(window_rows).is_some(),
        )?;
        let row = pixel_row_index(position.y, cell_height)?;
        let line_index = layout.content_line_at(row)?;
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
        self.current_mouse_modes().is_tracked() && !self.modifiers.is_shift()
    }

    /// Map a pixel position to 0-based `(col, row)` cell indices suitable for
    /// the mouse encoder. Uses the same frame mapper as local selection, then
    /// converts the absolute scrollback line to a 0-based visible row.
    fn mouse_cell_at(&self, position: PhysicalPosition<f64>) -> Option<(u32, u32)> {
        let frame_size = self.window.as_ref()?.inner_size();
        self.mouse_cell_in_frame(position, frame_size)
    }

    /// Window-independent seam for the mouse-reporting path.
    fn mouse_cell_in_frame(
        &self,
        position: PhysicalPosition<f64>,
        frame_size: PhysicalSize<u32>,
    ) -> Option<(u32, u32)> {
        let terminal = self.terminal.as_ref()?;
        let point = self.grid_point_in_frame(position, frame_size)?;
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

    /// Project the terminal's authoritative mode state into the mouse
    /// encoder's existing input type. The projection is deliberately computed
    /// for each event so there is no second mode cache to synchronize.
    fn current_mouse_modes(&self) -> MouseModes {
        let Some(modes) = self.terminal.as_ref().map(TerminalState::modes) else {
            return MouseModes::disabled();
        };
        MouseModes::disabled()
            .with_normal(modes.is_mouse_normal_tracking_enabled())
            .with_button_event(modes.is_mouse_button_event_tracking_enabled())
            .with_any_event(modes.is_mouse_any_event_tracking_enabled())
            .with_utf8(modes.is_mouse_utf8_encoding_enabled())
            .with_sgr(modes.is_mouse_sgr_encoding_enabled())
            .with_urxvt(modes.is_mouse_urxvt_encoding_enabled())
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

    /// Encode one pointer event from the live terminal authority without PTY
    /// side effects. A return of `None` means the event is not reportable.
    fn encode_mouse(&self, event: PointerEvent) -> Option<Vec<u8>> {
        MouseEncoder::encode(event, self.current_mouse_modes(), self.mouse_grid()?)
    }

    /// Encode one pointer event and write the report bytes to the PTY.
    fn encode_and_send_mouse(&mut self, event: PointerEvent) {
        if let Some(bytes) = self.encode_mouse(event) {
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
        let input = diagnostics::from_snapshot(snapshot.as_ref(), self.pty_child)
            .with_persistence_conflict(self.workspace.persistence_conflict())
            .with_persistence_unverified(self.workspace.persistence_unverified());
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
        let visible_rows =
            renderer::fully_drawable_rows(physical.height, self.geometry.cell_metrics());
        self.clamp_sidebar_scroll(visible_rows);
        self.redraw_needed = true;
    }

    fn apply_pending_resize(&mut self) {
        let Some(grid) = self.pending_grid.take() else {
            return;
        };
        // Resize re-addresses the grid, so captured coordinates expire.
        self.selection = None;
        self.drag_origin = None;
        let runtime = RuntimeGridSize::from_window(grid);
        if let Some(terminal) = &mut self.terminal {
            if runtime.resize_terminal(terminal).is_err() {
                self.status = "Noren terminal resize failed";
                self.show_status = true;
            }
        }
        if let (Some(session), Some(size)) = (&self.pty, runtime.pty_size()) {
            if session.resize(size).is_err() {
                self.status = "Noren PTY resize failed";
                self.show_status = true;
            }
        }
        // Parked sessions resize too, so switching back presents a terminal
        // state and PTY at the current geometry instead of a stale one.
        if let Some(size) = runtime.pty_size() {
            for parked in self.parked_sessions.values_mut() {
                if runtime.resize_terminal(&mut parked.terminal).is_err() {
                    self.status = "Noren terminal resize failed";
                    self.show_status = true;
                }
                if parked.pty.resize(size).is_err() {
                    self.status = "Noren PTY resize failed";
                    self.show_status = true;
                }
            }
        }
        self.redraw_needed = true;
    }

    /// Apply one PTY output chunk, in order, to the authoritative terminal
    /// parser. Production and application tests share this exact byte path.
    fn apply_pty_output(&mut self, bytes: &[u8]) {
        if let Some(terminal) = &mut self.terminal {
            terminal.feed_bytes(bytes);
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
                    self.apply_pty_output(&bytes);
                    output_consumed = true;
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
        let visible_rows = self
            .window
            .as_ref()
            .map(|window| {
                renderer::fully_drawable_rows(
                    window.inner_size().height,
                    self.geometry.cell_metrics(),
                )
            })
            .unwrap_or_default();
        let status_row = u16::try_from(visible_rows)
            .ok()
            .and_then(|rows| self.rendered_status_row(rows));
        self.clamp_sidebar_scroll(visible_rows);
        let sidebar_lines = visible_sidebar_text_lines(
            self.workspace.sidebar(),
            self.sidebar_scroll_offset,
            visible_rows,
        );
        let lines = if self.palette_open {
            let mut lines = palette_text_lines(self.workspace.palette(), self.palette_selection);
            lines.extend(sidebar_lines);
            lines
        } else {
            sidebar_lines
        };
        let status = status_row.map(|source| {
            source.text(
                self.status,
                self.ssh_selection_status.as_deref(),
                self.ssh_diagnostic.as_deref(),
            )
        });
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
        // Parked sessions die with the app too; their rows persist as
        // `Restored` entries for the next launch (quit is not close).
        self.shutdown_parked_sessions();
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
                // An Ime event means composition replaced the keyboard path:
                // the typed content is dropped unread because IME support
                // itself is deferred. The record is argument-free, so the
                // drop surfaces in diagnostics as a count that can never
                // carry the composed text.
                diagnostics::record_ime_drop();
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.apply_pending_resize();
        self.drain_pty();
        self.drain_parked_sessions();
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
        self.shutdown_parked_sessions();
    }
}

/// One interpretation of a window grid for every terminal-facing consumer.
///
/// This value owns the status-row reservation and sidebar-column reservation.
/// Initialization, resize, TerminalState, and PTY winsize all consume it so a
/// caller cannot accidentally apply application chrome to only one layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RuntimeGridSize {
    rows: u16,
    cols: u16,
}

impl RuntimeGridSize {
    fn from_window(grid: GridSize) -> Self {
        Self {
            rows: NorenApp::content_terminal_rows(grid.rows()),
            cols: terminal_cols(grid.cols()),
        }
    }

    fn terminal_state(self) -> Option<TerminalState> {
        TerminalState::new(self.rows, self.cols).ok()
    }

    fn resize_terminal(self, terminal: &mut TerminalState) -> Result<(), TerminalError> {
        terminal.resize(self.rows, self.cols)
    }

    fn pty_size(self) -> Option<PtySize> {
        PtySize::from_raw(self.rows, self.cols)
    }
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
#[cfg(test)]
fn sidebar_text_lines(sidebar: &SidebarView) -> Vec<String> {
    visible_sidebar_text_lines(sidebar, 0, usize::MAX)
}

/// Format only the visible slice of the sidebar. The scroll offset is clamped
/// to the last full page so redraw work stays proportional to frame rows, not
/// to hidden entries.
fn visible_sidebar_text_lines(
    sidebar: &SidebarView,
    offset: usize,
    max_rows: usize,
) -> Vec<String> {
    if max_rows == 0 {
        return Vec::new();
    }
    if sidebar.is_empty() {
        return sidebar
            .empty_state()
            .map(|state| vec![state.message().to_string()])
            .unwrap_or_default();
    }
    let offset = offset.min(sidebar.rows().len().saturating_sub(max_rows));
    sidebar.rows()[offset..]
        .iter()
        .take(max_rows)
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
    app.load_ssh_hosts();
    if event_loop.run_app(&mut app).is_err() {
        eprintln!("Noren event loop failed");
    }
}

#[cfg(test)]
#[path = "main/tests.rs"]
mod tests;
