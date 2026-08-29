//! macOS entry point for the bounded local-zsh PTY PoC.

mod frame_geometry;
mod input_translation;
mod persistence_state;
mod renderer;

use frame_geometry::{
    pixel_row_index, sidebar_pixel_width_at_width, terminal_cols_at_width, terminal_column_at_width,
};
#[cfg(test)]
use frame_geometry::{sidebar_pixel_width, terminal_cols, terminal_column_at};
#[cfg(test)]
use input_translation::{
    app_modifiers_from_gate, gate_key_to_app, keypad_key, translate_logical_key,
};
use input_translation::{
    chord_from_event, chord_from_logical, diagnostics_chord_pressed, encode_button, encode_chord,
    palette_command_for, translate_key, translate_keypad_key, wheel_clicks,
};
use persistence_state::{AttemptOutcome, Observation, PersistenceState, SaveOutcome};

#[cfg(test)]
use noren_app::sidebar_text::{visible_sidebar_text_lines, visible_sidebar_text_lines_at_width};
#[cfg(test)]
use noren_app::{
    Arrow, CellMetrics, FunctionKey, Key, KeyDropReason, KeyInput, KeyPhase, KeypadInput,
    KeypadKey, MAX_RENDER_COLS,
    mouse::{MouseButton as EncoderButton, WheelDirection},
    passthrough::Modifiers as GateModifiers,
};
use noren_app::{
    CursorKeyMode, GridGeometry, GridSize, InputMode, KeyEncoder, KeypadMode, Modifiers,
    PARSE_BUDGET_BYTES_PER_TURN, PRODUCT_NAME, PasteReject, Resize, SystemClipboard,
    config::{AppConfig, KeymapConfig, UiConfig},
    diagnostics::{self, PtyChildStatus},
    encode_paste,
    git_worktree::{self, DiscoveredWorktree, WorktreeDiscovery, WorktreeListError},
    mouse::{MouseEncoder, MouseGrid, MouseModes, PointerEvent, PointerModifiers},
    palette::{CommandId, Palette},
    passthrough::{
        CLAIM_ID_PALETTE, Chord, ChordSeq, GateKind, KeyCode as GateKeyCode, PassthroughAction,
        PassthroughClaim, PassthroughGate, PassthroughPolicy, default_exit_claim,
    },
    session::{
        SessionAction, SessionError, SessionEvent, SessionId, SessionKind, SessionRegistry,
        SessionStatus,
    },
    session_persistence::{
        SESSION_STATE_FILE_NAME, SessionPersistenceError, load_snapshot, save_snapshot, snapshot,
    },
    sidebar::{SessionLifecycle, SidebarEntry, SidebarView},
    sidebar_text::{SidebarTextRow, visible_sidebar_text_rows_at_width},
    ssh_config::{HostDiscoveryKind, SshConfig},
    theme::Theme,
    wheel_routing::{TerminalWheelOwner, terminal_wheel_owner},
};
use noren_pty::{
    PtyEvent, PtySession, PtySize, SshDestination, SshDestinationError, SshLaunchPolicy,
};
use noren_terminal::{
    GridPoint, Selection, SelectionMode, TerminalEngine, TerminalError, TerminalState,
};
use renderer::{FrameChrome, RenderOutcome, Renderer};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};
#[cfg(test)]
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

const WINDOW_WIDTH: u32 = 900;
const WINDOW_HEIGHT: u32 = 600;
const POLL_INTERVAL: Duration = Duration::from_millis(16);
/// Keep configured-host memory and identity work bounded independently of the
/// frame height, with the same named-constant discipline as the worktree,
/// project, and agent row caps. The sidebar exposes a scroll window over this
/// bounded list. 64 rows of already-bounded labels retain a few KiB and keep
/// rebuild identity scans O(64) — deliberately above the largest
/// human-maintained alias lists (issue #175's example is 60 hosts) so the
/// bound exists for safety, not as the effective size of a real config.
/// Anything past it is reported with a reason, never dropped silently. The
/// parse-side DoS budgets in `ssh_config.rs` (#137) bound untrusted input
/// independently of this view cap and are unchanged by it.
const MAX_SSH_SIDEBAR_HOSTS: usize = 64;
const SSH_SIDEBAR_TRUNCATION_MARKER: &str = "...";
const SSH_SIDEBAR_TRUNCATION_MARKER_CHARS: usize = 3;
/// Bound the copy held by the view model independently of the SSH parser's
/// much larger accepted input. Visible clipping is still owned solely by the
/// shared width-aware projection, which adds its one ellipsis for every kind.
const MAX_SIDEBAR_IDENTITY_CHARS: usize = 1024;
const SSH_SIDEBAR_DETAIL: &str = "not connected";
/// How long an ssh session may sit between EOF and its reaped exit event
/// before it is classified as an immediate disconnect.
const SSH_EOF_REAP_GRACE: Duration = Duration::from_secs(2);
/// Status-row text for each SSH connection phase. Fixed strings only: a
/// destination may embed a secret-shaped value, so no phase message may ever
/// name it.
const SSH_STATUS_CONNECTING: &str = "Noren ssh connecting";
const SSH_STATUS_CLOSED: &str = "Noren ssh session closed";
const SSH_STATUS_CONNECT_FAILED: &str = "Noren ssh connection failed";
const SSH_STATUS_DISCONNECTED: &str = "Noren ssh connection lost";
const SSH_STATUS_LAUNCH_FAILED: &str = "Noren ssh launch failed";
const SSH_STATUS_REFUSED: &str = "Noren ssh connect refused";
/// Status-row text when a worktree row whose directory no longer exists is
/// selected: the launch is refused before any session or child exists.
const WORKTREE_STATUS_MISSING: &str = "Noren worktree directory missing";
/// Status-row text when a worktree session's zsh child could not be spawned.
const WORKTREE_STATUS_LAUNCH_FAILED: &str = "Noren worktree launch failed";
/// Keep configured-agent memory and identity work bounded independently of
/// the frame height, exactly like the SSH host bound. Agent rows share the
/// sidebar's scroll window over this bounded list.
const MAX_AGENT_SIDEBAR_ROWS: usize = 24;
const AGENT_SIDEBAR_DETAIL_IDLE: &str = "not running";
const AGENT_SIDEBAR_DETAIL_FAILED: &str = "launch failed";
/// Status-row text when an agent row's configured command could not be
/// spawned (missing or non-executable program). Fixed text: the command is
/// configuration content and never appears on the status row.
const AGENT_STATUS_LAUNCH_FAILED: &str = "Noren agent launch failed";
/// Keep configured-project memory and identity work bounded independently of
/// the frame height, exactly like the agent and SSH bounds. Project rows
/// share the sidebar's scroll window over this bounded list.
const MAX_PROJECT_SIDEBAR_ROWS: usize = 24;
const PROJECT_SIDEBAR_DETAIL_IDLE: &str = "not running";
const PROJECT_SIDEBAR_DETAIL_FAILED: &str = "launch failed";
/// Status-row text when a project row whose root directory no longer exists
/// is selected: the launch is refused before any session or child exists.
const PROJECT_STATUS_MISSING: &str = "Noren project directory missing";
/// Status-row text when a project session's zsh child could not be spawned.
const PROJECT_STATUS_LAUNCH_FAILED: &str = "Noren project launch failed";

/// The window title: [`PRODUCT_NAME`] plus the crate version the binary was
/// built as, so the first surface a user sees states the product and the
/// version they actually launched — and moves with `Cargo.toml` automatically
/// instead of restating a frozen framing like "PoC" (issue #185).
fn window_title() -> String {
    format!("{PRODUCT_NAME} {}", env!("CARGO_PKG_VERSION"))
}

/// Status-row text while the first local PTY is starting. Both startup
/// statuses are prefixed with [`PRODUCT_NAME`]; the issue-185 pin test
/// `window_title_and_startup_status_read_the_product_name_and_built_version`
/// asserts the prefix so the status row and the window title cannot drift
/// apart again.
const STATUS_STARTING: &str = "Noren starting";
/// Status-row text once the first local PTY's child is running.
const STATUS_READY: &str = "Noren ready";

/// Observed state of the one live SSH launch. This is application-local
/// runtime state, deliberately outside the session registry: the registry
/// persists to `sessions.toml`, and a destination — which may embed a
/// secret-shaped value — must never be persisted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SshConnectionPhase {
    /// The system ssh client was spawned and owns the terminal; no output
    /// has been observed yet.
    Connecting,
    /// The remote side has produced output through the normal PTY path, so
    /// an interactive session is under way.
    Connected,
    /// ssh exited cleanly (exit code 0); the terminal is back offline.
    Closed,
    /// The ssh child could not be spawned at all.
    LaunchFailed,
    /// ssh exited non-zero: unreachable host, authentication failure, or a
    /// refused destination.
    ConnectFailed,
    /// The PTY closed without a reaped exit code: an immediate disconnect.
    Disconnected,
}

impl SshConnectionPhase {
    /// Whether this phase owns the terminal with a live ssh child.
    const fn is_live(self) -> bool {
        matches!(self, Self::Connecting | Self::Connected)
    }

    /// The shared #209 lifecycle shape for this connection phase.
    const fn sidebar_lifecycle(self) -> SessionLifecycle {
        match self {
            Self::Connecting => SessionLifecycle::Starting,
            Self::Connected => SessionLifecycle::Running,
            Self::Closed => SessionLifecycle::Exited,
            Self::LaunchFailed | Self::ConnectFailed | Self::Disconnected => {
                SessionLifecycle::Failed
            }
        }
    }

    /// The bounded ASCII sidebar detail text.
    const fn sidebar_detail(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Closed => SSH_SIDEBAR_DETAIL,
            Self::LaunchFailed => "launch failed",
            Self::ConnectFailed => "connection failed",
            Self::Disconnected => "disconnected",
        }
    }

    /// The status-row text for this phase.
    const fn status_text(self) -> &'static str {
        match self {
            Self::Connecting => SSH_STATUS_CONNECTING,
            Self::Connected => SSH_STATUS_CONNECTING,
            Self::Closed => SSH_STATUS_CLOSED,
            Self::LaunchFailed => SSH_STATUS_LAUNCH_FAILED,
            Self::ConnectFailed => SSH_STATUS_CONNECT_FAILED,
            Self::Disconnected => SSH_STATUS_DISCONNECTED,
        }
    }
}

/// Classify the terminal observation of an ended ssh child. This is the
/// failure-mapping seam: every non-clean outcome is a visible failure phase,
/// never a silent hang or a fake success.
fn ssh_exit_observation(code: Option<u32>) -> SshConnectionPhase {
    match code {
        Some(0) => SshConnectionPhase::Closed,
        Some(_) => SshConnectionPhase::ConnectFailed,
        None => SshConnectionPhase::Disconnected,
    }
}

/// Classify a spawn attempt for the fixed system ssh client.
fn ssh_launch_observation(spawned: bool) -> SshConnectionPhase {
    if spawned {
        SshConnectionPhase::Connecting
    } else {
        SshConnectionPhase::LaunchFailed
    }
}
/// Keep the complete source identity and the partial-discovery warning visible
/// together on ordinary terminal widths. The stable source tag is placed first
/// so path truncation cannot make two retained sources indistinguishable.
const SSH_STATUS_SOURCE_CHARS: usize = 40;

/// Build the bounded view-model identity for an SSH target without copying or
/// scanning the complete target. This is a memory bound, not visible
/// truncation: the shared row projection owns the only displayed ellipsis.
fn ssh_sidebar_label(target: &str) -> String {
    target.chars().take(MAX_SIDEBAR_IDENTITY_CHARS).collect()
}

/// Fixed-text, count-only clause explaining why wildcard pattern groups are
/// absent from the sidebar: a pattern is a matching rule, not a connectable
/// destination, so it never becomes a row. The count says the configuration
/// was still read. Only the number varies — pattern text is file content and
/// must never reach the status row (TM-08, issue #155).
fn ssh_unlisted_wildcard_clause(count: usize) -> String {
    match count {
        0 => String::new(),
        1 => "; 1 wildcard pattern not listed".to_owned(),
        count => format!("; {count} wildcard patterns not listed"),
    }
}

/// The fixed detail text for a configured agent row.
fn agent_sidebar_detail(failed: bool) -> &'static str {
    if failed {
        AGENT_SIDEBAR_DETAIL_FAILED
    } else {
        AGENT_SIDEBAR_DETAIL_IDLE
    }
}

/// Configured launch targets use #209's stopped/failed shapes rather than a
/// second family of text prefixes.
fn configured_target_lifecycle(failed: bool) -> SessionLifecycle {
    if failed {
        SessionLifecycle::Failed
    } else {
        SessionLifecycle::Exited
    }
}

/// The fixed detail text for a configured project row.
fn project_sidebar_detail(failed: bool) -> &'static str {
    if failed {
        PROJECT_SIDEBAR_DETAIL_FAILED
    } else {
        PROJECT_SIDEBAR_DETAIL_IDLE
    }
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

#[derive(Clone, PartialEq, Eq)]
struct ConfiguredSshHost {
    /// Shared session vocabulary used for target identity; this is still only
    /// a configured fact and is never inserted into the live registry.
    kind: SessionKind,
    /// Bounded, root-relative provenance supplied by `SshConfig`.
    source_label: String,
}

/// Shape-only [`Debug`] (issue #146 triage): the launch-shape discriminant
/// and a provenance length, never the target or the config path text.
///
/// This is the configured-target leaf held directly by `WorkspaceState`
/// next to the fields its Debug used to redact; with this impl the vec can
/// be handed to Debug without a container-side guard.
impl fmt::Debug for ConfiguredSshHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match &self.kind {
            SessionKind::Local => "local",
            SessionKind::Project { .. } => "project",
            SessionKind::Worktree { .. } => "worktree",
            SessionKind::Ssh { .. } => "ssh",
            SessionKind::Agent { .. } => "agent",
        };
        f.debug_struct("ConfiguredSshHost")
            .field("kind", &kind)
            .field("source_label_chars", &self.source_label.chars().count())
            .finish_non_exhaustive()
    }
}

/// One configured agent held by the workspace: the display name shown on its
/// sidebar row plus the argv vector launched when the row is selected.
///
/// Sidebar fact until a row is selected, exactly like the configured SSH
/// hosts and discovered worktrees. The command was validated at
/// configuration load (absolute, bounded, shell-free argv); copying it here
/// preserves it verbatim, and it is never inserted into the live registry or
/// persisted — only the [`SessionKind::Agent`] name reaches `sessions.toml`.
#[derive(Clone, PartialEq, Eq)]
struct ConfiguredAgent {
    /// Display name from configuration; also the `SessionKind::Agent` name
    /// of a launched session, which is what persists.
    name: String,
    /// Absolute program path; `argv[0]` of the launch. Never displayed.
    command: String,
    /// argv words after the program, verbatim from configuration.
    args: Vec<String>,
}

/// Shape-only [`Debug`] (issue #146): the name and command are
/// user-authored configuration text, and a command can embed a private
/// path, so neither is printed — only the argv element count.
impl fmt::Debug for ConfiguredAgent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfiguredAgent")
            .field("name_chars", &self.name.chars().count())
            .field("argv", &(self.args.len() + 1))
            .finish_non_exhaustive()
    }
}

/// One configured project held by the workspace: the display name shown on
/// its sidebar row plus the absolute root directory a session starts in when
/// the row is selected.
///
/// Sidebar fact until a row is selected, exactly like the configured agents
/// and SSH hosts and the discovered worktrees. The root was validated at
/// configuration load (absolute, bounded); copying it here preserves it
/// verbatim, and it is never displayed — the row shows the configured name
/// and a fixed state detail — nor debug-printed. Only the launched session's
/// [`SessionKind::Project`] root reaches `sessions.toml`.
#[derive(Clone, PartialEq, Eq)]
struct ConfiguredProject {
    /// Display name from configuration.
    name: String,
    /// Absolute root directory of the project; the working directory of a
    /// launched session's child.
    root: PathBuf,
}

/// Shape-only [`Debug`] (issue #146): the name is user-authored
/// configuration text, and a root can embed a username or a private
/// directory name, so neither is printed — only their lengths.
impl fmt::Debug for ConfiguredProject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfiguredProject")
            .field("name_chars", &self.name.chars().count())
            .field("root_chars", &self.root.as_os_str().len())
            .finish_non_exhaustive()
    }
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
/// the palette opener from the configured keymap.
///
/// The exit leader stays the frozen [`default_exit_claim`]. The palette claim
/// is the configured [`KeymapConfig::palette_open`] chord, which
/// configuration has already validated against the pinned Zellij v0.44.3
/// corpus and the exit leader, so this claim steals no chord from Zellij or
/// its panes. This is the smallest set that works: one chord to open the
/// palette, one to exit to the workspace. No bare modifier chords, no
/// chords the corpus binds.
fn palette_policy(keys: KeymapConfig) -> PassthroughPolicy {
    let palette_claim = PassthroughClaim {
        id: CLAIM_ID_PALETTE,
        action: PassthroughAction::OpenCommandPalette,
        seq: ChordSeq::single(keys.palette_open()),
        justification: "the configured palette chord is validated at load time against the \
                        pinned Zellij v0.44.3 default corpus and the exit leader, so claiming \
                        it steals no chord from Zellij or its panes",
    };
    PassthroughPolicy::try_new(vec![default_exit_claim(), palette_claim])
        .expect("configuration validates the palette chord; the claim set is collision-free")
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
    /// Configured projects (`[[projects]]`), bounded by
    /// [`MAX_PROJECT_SIDEBAR_ROWS`]. Sidebar facts until a row is selected:
    /// no registry entry or child exists for one. Selecting a row creates a
    /// `SessionKind::Project` session, which is the shape that persists.
    projects: Vec<ConfiguredProject>,
    /// How many configured projects the bounded sidebar policy omitted.
    projects_omitted: usize,
    /// Names of configured projects whose most recent launch FAILED.
    /// Best-effort row marker bounded by the configured project count; the
    /// session rows the registry owns remain the authority for a launch's
    /// observed status.
    project_launch_failures: std::collections::HashSet<String>,
    /// Worktrees discovered in the launch repository at startup: sidebar
    /// facts only until a row is selected, exactly like the configured SSH
    /// hosts. Selecting a row creates a `SessionKind::Worktree` session,
    /// which is the shape that persists.
    worktrees: Vec<DiscoveredWorktree>,
    /// How many discovered worktrees the bounded sidebar policy omitted.
    worktrees_omitted: usize,
    /// Configured SSH targets represented by the shared session vocabulary.
    /// They are sidebar facts only: no registry entry or connection exists.
    ssh_hosts: Vec<ConfiguredSshHost>,
    ssh_hosts_omitted: usize,
    selected_ssh_target: Option<String>,
    selected_ssh_source_label: Option<String>,
    /// Configured agents (`[[agents]]`), bounded by [`MAX_AGENT_SIDEBAR_ROWS`].
    /// Sidebar facts until a row is selected: no registry entry or child
    /// exists for one. The command never enters the registry, so it is never
    /// persisted to `sessions.toml` — only a launched session's
    /// [`SessionKind::Agent`] name is.
    agents: Vec<ConfiguredAgent>,
    /// How many configured agents the bounded sidebar policy omitted.
    agents_omitted: usize,
    /// Names of configured agents whose most recent launch FAILED. Best-effort
    /// row marker bounded by the configured agent count; the session rows the
    /// registry owns remain the authority for a launch's observed status.
    agent_launch_failures: std::collections::HashSet<String>,
    /// The one live SSH launch's target and phase, mirrored here only so
    /// [`Self::rebuild_sidebar`] can mark the matching configured row. The
    /// target never enters the registry, so it is never persisted.
    ssh_connection: Option<(String, SshConnectionPhase)>,
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

impl fmt::Debug for WorkspaceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Every nested leaf this struct reaches (SidebarView,
        // PersistenceState) is shape-only by construction, so no per-field
        // redaction is needed for them. The remaining direct content fields
        // — the selected and connected SSH targets, their provenance label,
        // and the state-file path — print presence only; the connection
        // phase is a fixed enum and safe to name.
        f.debug_struct("WorkspaceState")
            .field("registry", &self.registry.len())
            .field("registry_selection", &self.registry.selected())
            .field("sidebar", &self.sidebar)
            .field("projects", &self.projects.len())
            .field("projects_omitted", &self.projects_omitted)
            .field(
                "project_launch_failures",
                &self.project_launch_failures.len(),
            )
            .field("worktrees", &self.worktrees.len())
            .field("worktrees_omitted", &self.worktrees_omitted)
            .field("ssh_hosts", &self.ssh_hosts.len())
            .field("ssh_hosts_omitted", &self.ssh_hosts_omitted)
            .field("agents", &self.agents.len())
            .field("agents_omitted", &self.agents_omitted)
            .field("agent_launch_failures", &self.agent_launch_failures.len())
            .field("selected_ssh_target", &self.selected_ssh_target.is_some())
            .field(
                "selected_ssh_source_label",
                &self.selected_ssh_source_label.is_some(),
            )
            .field(
                "ssh_connection",
                &self.ssh_connection.as_ref().map(|(_, phase)| *phase),
            )
            .field("palette", &self.palette)
            .field("state_path", &self.state_path.is_some())
            .field("persistence", &self.persistence)
            .finish()
    }
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
            projects: Vec::new(),
            projects_omitted: 0,
            project_launch_failures: std::collections::HashSet::new(),
            worktrees: Vec::new(),
            worktrees_omitted: 0,
            ssh_hosts: Vec::new(),
            ssh_hosts_omitted: 0,
            selected_ssh_target: None,
            selected_ssh_source_label: None,
            agents: Vec::new(),
            agents_omitted: 0,
            agent_launch_failures: std::collections::HashSet::new(),
            ssh_connection: None,
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

    /// Wire where sidebar state persists, ahead of [`Self::restore`].
    ///
    /// The seam `load_sidebar_state` uses: it keeps the path field private to
    /// this impl, so the binary cannot rewrite persistence mid-run.
    fn set_state_path(&mut self, state_path: Option<PathBuf>) {
        self.state_path = state_path;
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

    /// Replace the configured project facts without creating sessions. Rows
    /// are bounded by [`MAX_PROJECT_SIDEBAR_ROWS`] and the omitted count is
    /// retained for the status notice, exactly like the agent, SSH host, and
    /// worktree bounds. Returns the number omitted.
    fn load_projects(&mut self, projects: &[noren_app::config::ProjectConfig]) -> usize {
        self.projects = projects
            .iter()
            .take(MAX_PROJECT_SIDEBAR_ROWS)
            .map(|project| ConfiguredProject {
                name: project.name().to_owned(),
                root: PathBuf::from(project.root()),
            })
            .collect();
        self.projects_omitted = projects.len().saturating_sub(self.projects.len());
        self.project_launch_failures.clear();
        self.rebuild_sidebar();
        self.projects_omitted
    }

    /// The configured project fact at a stable sidebar position, if that
    /// position is a project row. Session rows precede project rows; worktree
    /// rows follow them.
    fn project_sidebar_row(&self, row_index: usize) -> Option<&ConfiguredProject> {
        let index = row_index.checked_sub(self.registry.len())?;
        self.projects.get(index)
    }

    /// Mark a configured project's most recent launch as failed, so its row
    /// carries the visible `PRJ-ERR` state.
    fn record_project_launch_failure(&mut self, name: &str) {
        self.project_launch_failures.insert(name.to_owned());
    }

    /// Clear a configured project's failure marker after a successful launch.
    fn clear_project_launch_failure(&mut self, name: &str) {
        self.project_launch_failures.remove(name);
    }

    /// Replace the discovered worktree facts without creating sessions.
    /// Rows are bounded by [`git_worktree::MAX_WORKTREE_SIDEBAR_ROWS`] and
    /// the omitted count is retained for the status notice.
    fn load_worktrees(&mut self, discovery: WorktreeDiscovery) {
        self.worktrees_omitted = discovery.omitted();
        self.worktrees = discovery.rows().to_vec();
        self.rebuild_sidebar();
    }

    /// The worktree fact at a stable sidebar position, if that position is
    /// a worktree row. Session rows and project rows precede worktree rows;
    /// SSH host rows follow them.
    fn worktree_sidebar_row(&self, row_index: usize) -> Option<&DiscoveredWorktree> {
        let index = row_index.checked_sub(self.registry.len() + self.projects.len())?;
        self.worktrees.get(index)
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
        let project_rows = self.projects.len();
        let worktree_rows = self.worktrees.len();
        let host_index = row_index.checked_sub(session_rows + project_rows + worktree_rows);
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

    /// Replace the configured agent facts without creating sessions. Rows
    /// are bounded by [`MAX_AGENT_SIDEBAR_ROWS`] and the omitted count is
    /// retained for the status notice, exactly like the worktree and SSH
    /// host bounds. Returns the number omitted.
    fn load_agents(&mut self, agents: &[noren_app::config::AgentConfig]) -> usize {
        self.agents = agents
            .iter()
            .take(MAX_AGENT_SIDEBAR_ROWS)
            .map(|agent| ConfiguredAgent {
                name: agent.name().to_owned(),
                command: agent.command().to_owned(),
                args: agent.args().to_vec(),
            })
            .collect();
        self.agents_omitted = agents.len().saturating_sub(self.agents.len());
        self.agent_launch_failures.clear();
        self.rebuild_sidebar();
        self.agents_omitted
    }

    /// The configured agent fact at a stable sidebar position, if that
    /// position is an agent row. Session rows precede project rows, project
    /// rows precede worktree rows, SSH host rows follow them, and agent rows
    /// follow the SSH hosts; the subtraction chain mirrors the order the
    /// sidebar renders.
    fn agent_sidebar_row(&self, row_index: usize) -> Option<&ConfiguredAgent> {
        let index = row_index.checked_sub(
            self.registry.len() + self.projects.len() + self.worktrees.len() + self.ssh_hosts.len(),
        )?;
        self.agents.get(index)
    }

    /// Mark a configured agent's most recent launch as failed, so its row
    /// carries the visible `AG-ERR` state.
    fn record_agent_launch_failure(&mut self, name: &str) {
        self.agent_launch_failures.insert(name.to_owned());
    }

    /// Clear a configured agent's failure marker after a successful launch.
    fn clear_agent_launch_failure(&mut self, name: &str) {
        self.agent_launch_failures.remove(name);
    }

    /// Record the phase of the one live SSH launch and refresh the sidebar.
    ///
    /// The target is compared against the configured host rows to mark the
    /// matching row's state. It is held in memory only: the registry is the
    /// only persisted state and it never sees an SSH launch, so the target
    /// cannot reach `sessions.toml`.
    fn set_ssh_connection(&mut self, target: &str, phase: SshConnectionPhase) {
        self.ssh_connection = Some((target.to_owned(), phase));
        self.rebuild_sidebar();
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
    /// Called after every mutation so the view never lags the model. Row
    /// order is stable: session rows first, then configured project facts,
    /// then discovered worktree facts, then configured SSH host facts, then
    /// configured agent facts.
    fn rebuild_sidebar(&mut self) {
        let entries: Vec<SidebarEntry> = self
            .registry
            .sessions()
            .into_iter()
            .map(SidebarEntry::Session)
            .collect();
        let mut entries = entries;
        // Project identity is the configured display name, never the root
        // path. Kind and launch state occupy their fixed shape cells, leaving
        // the complete 16-column identity budget free of textual prefixes.
        entries.extend(self.projects.iter().map(|project| {
            let failed = self.project_launch_failures.contains(&project.name);
            SidebarEntry::Project {
                name: project.name.clone(),
                root: project_sidebar_detail(failed).to_owned(),
                lifecycle: configured_target_lifecycle(failed),
            }
        }));
        entries.extend(
            self.worktrees
                .iter()
                .map(|worktree| SidebarEntry::Worktree {
                    name: worktree.name_display(),
                    branch: worktree.branch_display(),
                }),
        );
        let mut pending_marked = false;
        entries.extend(self.ssh_hosts.iter().filter_map(|host| {
            let SessionKind::Ssh { target } = &host.kind else {
                return None;
            };
            let selected =
                !pending_marked && self.selected_ssh_target.as_deref() == Some(target.as_str());
            pending_marked |= selected;
            // The one live connection, if any, is matched by exact target so
            // colliding truncated labels cannot mark the wrong row.
            let phase = self
                .ssh_connection
                .as_ref()
                .filter(|(connected, _)| connected == target)
                .map(|(_, phase)| *phase);
            let (detail, lifecycle) = match phase {
                Some(phase) => (phase.sidebar_detail(), phase.sidebar_lifecycle()),
                None => (SSH_SIDEBAR_DETAIL, SessionLifecycle::Exited),
            };
            Some(SidebarEntry::SshConnection {
                label: ssh_sidebar_label(target),
                host: detail.to_owned(),
                selected,
                lifecycle,
            })
        }));
        entries.extend(self.agents.iter().map(|agent| {
            let failed = self.agent_launch_failures.contains(&agent.name);
            SidebarEntry::Agent {
                label: agent.name.clone(),
                status: agent_sidebar_detail(failed).to_owned(),
                lifecycle: configured_target_lifecycle(failed),
            }
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
    /// UI choice; when a launch follows, the click path also drives
    /// [`Self::set_ssh_connection`] for the same target.
    fn selected_ssh_target(&self) -> Option<&str> {
        self.selected_ssh_target.as_deref()
    }

    fn selected_ssh_source_label(&self) -> Option<&str> {
        self.selected_ssh_source_label.as_deref()
    }

    /// The one live SSH launch's target and phase, if one is recorded.
    ///
    /// Read seam for the application layer: the phase predicates and the
    /// target of the next [`Self::set_ssh_connection`] update resolve through
    /// here, so the field never has to leave this impl.
    fn ssh_connection(&self) -> Option<&(String, SshConnectionPhase)> {
        self.ssh_connection.as_ref()
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
    scroll_offset: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScrollDirection {
    Older,
    Newer,
}

/// Boundary decision for one wheel delta over the terminal surface.
///
/// The variant is intentionally test-visible inside this binary: the central
/// correctness property is ownership, not merely whether some state changed.
#[derive(Debug, PartialEq, Eq)]
enum TerminalWheelRoute {
    ConsumedLocally { before: usize, after: usize },
    ForwardedToApplication(Vec<Vec<u8>>),
}

struct NorenApp {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    geometry: GridGeometry,
    /// Validated `[sidebar] columns`, shared by layout, drawing, formatting,
    /// and hit testing so configured control cannot create split geometry.
    sidebar_columns: usize,
    pending_grid: Option<GridSize>,
    terminal: Option<TerminalState>,
    /// Logical rows above the active terminal's live tail. Zero follows new
    /// output automatically; a non-zero value is deliberate user history and
    /// stays put until the user scrolls down. Rendering clamps defensively too.
    scroll_offset: usize,
    pty: Option<PtySession>,
    pty_child: PtyChildStatus,
    modifiers: Modifiers,
    status: &'static str,
    show_status: bool,
    diagnostics_visible: bool,
    diagnostics_line: String,
    ssh_diagnostic: Option<String>,
    ssh_selection_status: Option<String>,
    /// Bounded, content-free worktree-discovery notice (cap or failure).
    worktree_diagnostic: Option<String>,
    /// Bounded, content-free configured-projects notice (cap only; project
    /// launch failures surface as runtime statuses and row markers instead).
    project_diagnostic: Option<String>,
    /// Bounded, content-free configured-agents notice (cap only; agent
    /// launch failures surface as runtime statuses instead).
    agent_diagnostic: Option<String>,
    /// When EOF was observed on a live ssh child whose reaped exit event has
    /// not arrived yet; drives the immediate-disconnect classification.
    ssh_eof_since: Option<Instant>,
    /// Production spawns the real system ssh client. The binary test seam can
    /// disable the spawn so the click path is exercised without launching any
    /// process, keeping the unit suite deterministic.
    ssh_spawn_enabled: bool,
    /// Test-only override that makes the ssh spawn attempt itself fail (the
    /// `Err` arm of `PtySession::spawn_ssh`), which cannot be forced from a
    /// valid destination on a healthy machine. Production never sets it.
    #[cfg(test)]
    ssh_spawn_force_failure: bool,
    redraw_needed: bool,
    // User-initiated selection state. The renderer paints this exact range and
    // copy extracts the same model. Any PTY output or resize invalidates it
    // because grid coordinates only address the content they were captured on.
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
    /// The session whose final frame is still displayed after its child
    /// exited. `finish_pty` clears `active_session` (input ownership dies
    /// with the child) but keeps the terminal for the last frame; this field
    /// remembers whose frame it is so closing that row runs the same
    /// detach-and-fallback path an active close does, instead of leaving a
    /// frozen frame behind a vanished row.
    exited_surface_session: Option<SessionId>,
    /// Live sessions that are not the active one, keyed by session id.
    parked_sessions: HashMap<SessionId, ParkedSession>,
    /// Test-only override for the PTY child's home directory. Production
    /// always spawns in the inherited `$HOME`; tests that drive a shell by
    /// typing set an isolated empty directory so the shell's startup cannot
    /// depend on the developer's personal configuration (which may take
    /// arbitrarily long or read the terminal).
    #[cfg(test)]
    test_pty_home: Option<PathBuf>,
    palette_open: bool,
    palette_selection: usize,
    passthrough_gate: PassthroughGate,
    passthrough_policy: PassthroughPolicy,
    /// Configured key chords for the palette opener and the four palette
    /// commands. The single source of truth for every workspace chord the
    /// binary honors; no hard-coded chord remains on the key paths.
    keys: KeymapConfig,
    /// Optional application chrome. Its default keeps the palette affordance
    /// visible; the validated config can explicitly remove that one hint.
    ui: UiConfig,
    /// The configured colour theme, handed to the renderer at creation so
    /// every palette-derived draw colour — default foreground, ANSI
    /// resolution, clear colour — follows the `[theme]` selection. The
    /// default (`dark`) is exactly the pre-theme palette.
    theme: Theme,
    /// The configured cursor appearance (`[cursor]` shape and preferred
    /// colour), handed to the renderer at creation. Final ink is resolved
    /// against the actual cursor cell; visibility is not part of this style:
    /// the caret ships drawn and only DECTCEM hides it (issues #197/#200).
    cursor_style: renderer::CursorStyle,
}

/// Which application-owned line, if any, occupies the renderer's status row.
///
/// Runtime statuses take precedence while `show_status` is set. A pending SSH
/// selection then exposes its bounded provenance; a worktree-discovery notice
/// (cap or failure) follows; an agents-cap notice follows that; otherwise a
/// readable config keeps the partial-discovery notice (or a parse failure
/// keeps its content-free diagnostic). The runtime source is also the idle
/// fallback, making the row a permanent part of the application grid rather
/// than dynamically hiding a PTY row when a notice appears.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StatusRowSource {
    Runtime,
    SshSelection,
    WorktreeDiagnostic,
    ProjectDiagnostic,
    AgentDiagnostic,
    SshDiagnostic,
}

impl StatusRowSource {
    fn text<'a>(
        self,
        runtime: &'a str,
        ssh_selection_status: Option<&'a str>,
        worktree_diagnostic: Option<&'a str>,
        project_diagnostic: Option<&'a str>,
        agent_diagnostic: Option<&'a str>,
        ssh_diagnostic: Option<&'a str>,
    ) -> &'a str {
        match self {
            Self::Runtime => runtime,
            Self::SshSelection => {
                ssh_selection_status.expect("SSH selection source requires a provenance status")
            }
            Self::WorktreeDiagnostic => {
                worktree_diagnostic.expect("worktree diagnostic source requires diagnostic text")
            }
            Self::ProjectDiagnostic => {
                project_diagnostic.expect("project diagnostic source requires diagnostic text")
            }
            Self::AgentDiagnostic => {
                agent_diagnostic.expect("agent diagnostic source requires diagnostic text")
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
        // Configured agents and projects are sidebar facts from the same
        // validated file: the names come from configuration (already
        // bounded), the sidebar bound applies on top, and the omitted count
        // reaches the status row.
        let mut workspace = WorkspaceState::new();
        let projects_omitted = workspace.load_projects(config.projects());
        let project_diagnostic = (projects_omitted > 0).then(|| {
            format!(
                "Noren projects: showing first {MAX_PROJECT_SIDEBAR_ROWS}; {projects_omitted} omitted"
            )
        });
        let agents_omitted = workspace.load_agents(config.agents());
        let agent_diagnostic = (agents_omitted > 0).then(|| {
            format!(
                "Noren agents: showing first {MAX_AGENT_SIDEBAR_ROWS}; {agents_omitted} omitted"
            )
        });
        Self {
            window: None,
            renderer: None,
            geometry,
            sidebar_columns: config.sidebar().columns(),
            pending_grid: None,
            terminal: None,
            scroll_offset: 0,
            pty: None,
            pty_child: PtyChildStatus::NotLaunched,
            modifiers: Modifiers::empty(),
            status: STATUS_STARTING,
            show_status: true,
            diagnostics_visible: false,
            diagnostics_line: String::new(),
            ssh_diagnostic: None,
            ssh_selection_status: None,
            worktree_diagnostic: None,
            project_diagnostic,
            agent_diagnostic,
            ssh_eof_since: None,
            ssh_spawn_enabled: true,
            #[cfg(test)]
            ssh_spawn_force_failure: false,
            redraw_needed: true,
            selection: None,
            drag_origin: None,
            drag_mode: SelectionMode::Char,
            cursor_position: None,
            held_mouse_button: None,
            workspace,
            sidebar_scroll_offset: 0,
            active_session: None,
            exited_surface_session: None,
            parked_sessions: HashMap::new(),
            #[cfg(test)]
            test_pty_home: None,
            palette_open: false,
            palette_selection: 0,
            passthrough_gate: PassthroughGate::new(),
            passthrough_policy: palette_policy(config.keys()),
            keys: config.keys(),
            ui: config.ui(),
            theme: config.theme().palette(),
            cursor_style: renderer::CursorStyle::theme_default(&config.theme().palette())
                .with_shape(config.cursor().shape())
                .with_color_override(config.cursor().color().map(|[r, g, b]| {
                    [
                        f32::from(r) / 255.0,
                        f32::from(g) / 255.0,
                        f32::from(b) / 255.0,
                    ]
                })),
        }
    }

    /// Single status-row decision shared by rendering and pointer mapping.
    fn status_row(&self) -> StatusRowSource {
        if self.show_status {
            StatusRowSource::Runtime
        } else if self.ssh_selection_status.is_some() {
            StatusRowSource::SshSelection
        } else if self.worktree_diagnostic.is_some() {
            StatusRowSource::WorktreeDiagnostic
        } else if self.project_diagnostic.is_some() {
            StatusRowSource::ProjectDiagnostic
        } else if self.agent_diagnostic.is_some() {
            StatusRowSource::AgentDiagnostic
        } else if self.ssh_diagnostic.is_some() {
            StatusRowSource::SshDiagnostic
        } else {
            StatusRowSource::Runtime
        }
    }

    fn rendered_status_row(&self, window_rows: u16) -> Option<StatusRowSource> {
        Self::status_row_present(window_rows).then(|| self.status_row())
    }

    /// Install the terminal state and return the exactly matching PTY size.
    ///
    /// Keeping this as the initialization seam prevents the two consumers from
    /// independently reinterpreting the application-owned status row.
    fn prepare_initial_terminal(&mut self, grid: GridSize) -> Option<PtySize> {
        let runtime = RuntimeGridSize::from_window(grid, self.sidebar_columns);
        let terminal = runtime.terminal_state()?;
        let pty = runtime.pty_size()?;
        self.terminal = Some(terminal);
        self.scroll_offset = 0;
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
        self.workspace.set_state_path(path);
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

    /// Discover the git worktrees of the repository Noren was launched in.
    ///
    /// A launch directory outside any git repository is the common case and
    /// is silent (like a missing SSH config): no rows, no notice. A missing
    /// or non-directory launch path is reported as `LaunchDirectoryUnavailable`
    /// (not `GitUnavailable`): git may be installed, the directory is not.
    /// Every other failure — git unavailable, unreadable or malformed
    /// output — is a bounded, content-free status line that never stops
    /// startup. A repository with more worktrees than the sidebar cap
    /// reports the cap and the omitted count.
    fn load_git_worktrees(&mut self) {
        let launch_dir = std::env::current_dir()
            .map_err(|_| WorktreeListError::LaunchDirectoryUnavailable)
            .and_then(|dir| git_worktree::discover_worktrees(&dir));
        self.apply_worktree_discovery(launch_dir);
    }

    /// Deterministic explicit-directory seam used by tests.
    #[cfg(test)]
    fn load_git_worktrees_from(&mut self, launch_dir: &std::path::Path) {
        let discovery = git_worktree::discover_worktrees(launch_dir);
        self.apply_worktree_discovery(discovery);
    }

    fn apply_worktree_discovery(
        &mut self,
        discovery: Result<WorktreeDiscovery, WorktreeListError>,
    ) {
        match discovery {
            Ok(discovered) => {
                let omitted = discovered.omitted();
                self.workspace.load_worktrees(discovered);
                self.worktree_diagnostic = (omitted > 0).then(|| {
                    format!(
                        "Noren worktrees: showing first {}; {omitted} omitted",
                        git_worktree::MAX_WORKTREE_SIDEBAR_ROWS
                    )
                });
            }
            Err(WorktreeListError::NotARepository) => {
                self.workspace.load_worktrees(WorktreeDiscovery::empty());
                self.worktree_diagnostic = None;
            }
            Err(error) => {
                // The error Display is a fixed, content-free string.
                self.workspace.load_worktrees(WorktreeDiscovery::empty());
                let line = format!("Noren worktrees: {error}");
                eprintln!("{line}");
                self.worktree_diagnostic = Some(line);
            }
        }
        self.redraw_needed = true;
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
        let total = config.hosts().len();
        let shown = total.saturating_sub(omitted);
        let unlisted = ssh_unlisted_wildcard_clause(config.unlisted_wildcard_patterns());
        self.ssh_selection_status = None;
        if config.sources().is_empty() {
            self.ssh_diagnostic = None;
        } else {
            self.ssh_diagnostic = Some(match config.discovery_kind() {
                HostDiscoveryKind::PartialLiteralPatterns if total == 0 => {
                    let mut line = "Noren SSH: partial literal aliases; none found".to_owned();
                    line.push_str(&unlisted);
                    line
                }
                HostDiscoveryKind::PartialLiteralPatterns if omitted == 0 => {
                    let mut line =
                        "Noren SSH: partial literal aliases; select one for source".to_owned();
                    line.push_str(&unlisted);
                    line
                }
                HostDiscoveryKind::PartialLiteralPatterns => format!(
                    "Noren SSH: partial literal aliases; showing first {shown} of {total}; \
                     {omitted} past sidebar bound{unlisted}"
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

    /// Whether the live PTY is owned by an ssh child of a live launch.
    fn ssh_live(&self) -> bool {
        self.workspace
            .ssh_connection()
            .is_some_and(|(_, phase)| phase.is_live())
    }

    /// The PTY size for a new launch: the authoritative terminal grid, or a
    /// safe default before a terminal exists.
    fn live_pty_size(&self) -> PtySize {
        self.terminal
            .as_ref()
            .and_then(|terminal| {
                let (rows, cols) = terminal.size();
                PtySize::from_raw(rows, cols)
            })
            .unwrap_or_else(|| PtySize::from_raw(24, 80).expect("24x80 is a valid size"))
    }

    /// Retire the current terminal owner: the local session is observed as
    /// exited (its process is terminated) and its PTY is shut down bounded.
    fn retire_live_terminal(&mut self) {
        if let Some(id) = self.active_session.take() {
            self.workspace
                .observe_session(id, SessionStatus::Exited { code: None });
        }
        if let Some(mut previous) = self.pty.take() {
            let _ = previous.shutdown();
        }
    }

    /// Surface a typed destination refusal. The typed error names the OpenSSH
    /// keyword and token for token-bearing destinations; it never carries the
    /// destination itself, so a secret-shaped target cannot leak here.
    fn report_ssh_connect_refusal(&mut self, error: SshDestinationError) {
        self.ssh_selection_status = Some(format!("SSH connect refused: {error}"));
        // Show the typed refusal in preference to the static runtime text.
        self.status = SSH_STATUS_REFUSED;
        self.show_status = false;
        self.redraw_needed = true;
    }

    /// Record an observed phase of the live SSH launch and make it visible.
    fn apply_ssh_phase(&mut self, phase: SshConnectionPhase) {
        let Some(target) = self
            .workspace
            .ssh_connection()
            .map(|(target, _)| target.clone())
        else {
            return;
        };
        self.workspace.set_ssh_connection(&target, phase);
        if !phase.is_live() {
            self.status = phase.status_text();
            self.show_status = true;
        } else if phase == SshConnectionPhase::Connected {
            // The remote side is producing output through the normal PTY
            // path; the terminal content is the interface now.
            self.show_status = false;
        }
        self.redraw_needed = true;
    }

    /// Launch the system ssh client for `target` in the terminal's PTY.
    ///
    /// The destination is validated first: a refused destination never tears
    /// down the running terminal and never spawns a child. A successful spawn
    /// replaces the current terminal owner; a failed spawn leaves the current
    /// terminal untouched and surfaces [`SshConnectionPhase::LaunchFailed`].
    fn connect_ssh_target(&mut self, target: &str) -> SshConnectOutcome {
        if self
            .workspace
            .ssh_connection()
            .is_some_and(|(connected, phase)| connected == target && phase.is_live())
        {
            return SshConnectOutcome::AlreadyLive;
        }
        let destination = match SshDestination::new(target) {
            Ok(destination) => destination,
            Err(error) => {
                self.report_ssh_connect_refusal(error);
                return SshConnectOutcome::Refused;
            }
        };
        let spawned = self.ssh_spawn_enabled && {
            #[cfg(test)]
            let attempt = if self.ssh_spawn_force_failure {
                Err(noren_pty::PtyError::Backend {
                    operation: noren_pty::PtyOperation::SpawnChild,
                })
            } else {
                PtySession::spawn_ssh(SshLaunchPolicy::inherit(destination), self.live_pty_size())
            };
            #[cfg(not(test))]
            let attempt =
                PtySession::spawn_ssh(SshLaunchPolicy::inherit(destination), self.live_pty_size());
            match attempt {
                Ok(session) => {
                    self.retire_live_terminal();
                    self.pty = Some(session);
                    self.scroll_offset = 0;
                    self.pty_child = PtyChildStatus::Running;
                    true
                }
                Err(_) => false,
            }
        };
        let phase = ssh_launch_observation(spawned);
        self.ssh_eof_since = None;
        self.workspace.set_ssh_connection(target, phase);
        self.status = phase.status_text();
        self.show_status = true;
        self.redraw_needed = true;
        SshConnectOutcome::Launched(phase)
    }
}

/// Result of a sidebar click's connect attempt, for status-row wording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SshConnectOutcome {
    /// A spawn was attempted; the phase says whether it took.
    Launched(SshConnectionPhase),
    /// The destination was refused; the typed refusal is already visible.
    Refused,
    /// The same target already owns the terminal with a live child.
    AlreadyLive,
}

impl NorenApp {
    fn record_pty_started(&mut self) {
        self.status = STATUS_READY;
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
        let runtime = RuntimeGridSize::from_window(grid, self.sidebar_columns);
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
            let scroll_offset = std::mem::take(&mut self.scroll_offset);
            self.parked_sessions.insert(
                id,
                ParkedSession {
                    pty,
                    terminal,
                    scroll_offset,
                },
            );
            self.pty_child = PtyChildStatus::NotLaunched;
        }
    }

    /// Spawn the PTY for a new session.
    ///
    /// Production always runs the fixed `/bin/zsh` policy in the inherited
    /// `$HOME`. Tests that drive a shell by typing redirect the child into an
    /// isolated empty home (see [`NorenApp::test_pty_home`]) so a developer's
    /// startup files cannot make the test wait minutes for a prompt — the
    /// spawn itself, the policy, and the reaping contract are identical.
    fn spawn_pty_session(&self, size: PtySize) -> Result<PtySession, noren_pty::PtyError> {
        #[cfg(test)]
        if let Some(home) = &self.test_pty_home {
            return PtySession::spawn_in_home(home, size);
        }
        PtySession::spawn(size)
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
        match self.spawn_pty_session(pty_size) {
            Ok(pty) => {
                self.workspace.observe_session(id, SessionStatus::Running);
                self.park_active_session();
                self.pty = Some(pty);
                self.terminal = Some(terminal);
                self.scroll_offset = 0;
                self.active_session = Some(id);
                self.pty_child = PtyChildStatus::Running;
                // Grid coordinates captured on the previous session's screen
                // can only address the wrong content; the selection model
                // expires them, exactly as an explicit switch does.
                self.selection = None;
                self.drag_origin = None;
                self.exited_surface_session = None;
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

    /// Spawn the PTY for a directory-scoped session (a worktree or a
    /// configured project root), whose working directory is that directory.
    ///
    /// Production inherits `HOME` unchanged so the user's own shell
    /// configuration applies inside the directory. Tests that drive the
    /// shell by typing redirect the child's `HOME` to the isolated empty
    /// home (see [`NorenApp::test_pty_home`]) for the same reason as
    /// [`Self::spawn_pty_session`]: a developer's real `$HOME` may carry
    /// startup files that take arbitrarily long or read the terminal, which
    /// would make every shell-driving test depend on personal configuration.
    /// The working directory is the supplied directory in both shapes, so
    /// the child's actual cwd stays observable through its own `pwd` answer.
    fn spawn_directory_pty_session(
        &self,
        dir: &std::path::Path,
        size: PtySize,
    ) -> Result<PtySession, noren_pty::PtyError> {
        #[cfg(test)]
        if let Some(home) = &self.test_pty_home {
            return PtySession::spawn_in_dir_with_home(dir, home, size);
        }
        PtySession::spawn_in_dir(dir, size)
    }

    /// Spawn a real PTY session whose working directory is the worktree at
    /// `path`, and give it the live view.
    ///
    /// This is the worktree-row runtime: the new sidebar row is a
    /// `SessionKind::Worktree` registry session backed by an actual `/bin/zsh`
    /// PTY whose child starts *in* the worktree checkout (HOME inherited, so
    /// the user's shell configuration still applies). The registry observes
    /// `Running` when the spawn succeeds and `Failed` when it does not. The
    /// new session takes the live view and the previous one is parked, not
    /// killed — the same convention as [`Self::spawn_local_session`].
    fn spawn_worktree_session(&mut self, path: &std::path::Path) -> Option<SessionId> {
        let id = self.workspace.create_session(SessionKind::Worktree {
            path: path.to_owned(),
        });
        let Some((terminal, pty_size)) = self.session_surfaces() else {
            self.workspace.observe_session(
                id,
                SessionStatus::Failed {
                    reason: "terminal surface unavailable".to_owned(),
                },
            );
            return None;
        };
        match self.spawn_directory_pty_session(path, pty_size) {
            Ok(pty) => {
                self.workspace.observe_session(id, SessionStatus::Running);
                self.park_active_session();
                self.pty = Some(pty);
                self.terminal = Some(terminal);
                self.scroll_offset = 0;
                self.active_session = Some(id);
                self.pty_child = PtyChildStatus::Running;
                self.selection = None;
                self.drag_origin = None;
                self.exited_surface_session = None;
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
                self.status = WORKTREE_STATUS_LAUNCH_FAILED;
                self.show_status = true;
                self.redraw_needed = true;
                None
            }
        }
    }

    /// Spawn a real PTY session whose working directory is the configured
    /// project root, and give it the live view.
    ///
    /// This is the project-row runtime: the new sidebar row is a
    /// `SessionKind::Project` registry session backed by an actual `/bin/zsh`
    /// PTY whose child starts *in* the project root (HOME inherited, so the
    /// user's shell configuration still applies) — the same launch shape and
    /// conventions as a worktree session, distinguished by the kind that
    /// persists. The registry observes `Running` when the spawn succeeds and
    /// `Failed` when it does not. A configured root whose directory no longer
    /// exists is refused before this method runs (the caller checks), exactly
    /// like a registered-but-deleted worktree.
    fn spawn_project_session(&mut self, project: &ConfiguredProject) -> Option<SessionId> {
        let id = self.workspace.create_session(SessionKind::Project {
            root: project.root.clone(),
        });
        let Some((terminal, pty_size)) = self.session_surfaces() else {
            self.workspace.observe_session(
                id,
                SessionStatus::Failed {
                    reason: "terminal surface unavailable".to_owned(),
                },
            );
            self.workspace.record_project_launch_failure(&project.name);
            self.workspace.rebuild_sidebar();
            self.redraw_needed = true;
            return None;
        };
        match self.spawn_directory_pty_session(&project.root, pty_size) {
            Ok(pty) => {
                self.workspace.observe_session(id, SessionStatus::Running);
                self.workspace.clear_project_launch_failure(&project.name);
                self.park_active_session();
                self.pty = Some(pty);
                self.terminal = Some(terminal);
                self.scroll_offset = 0;
                self.active_session = Some(id);
                self.pty_child = PtyChildStatus::Running;
                self.selection = None;
                self.drag_origin = None;
                self.exited_surface_session = None;
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
                // observe_session already rebuilt once; recording the row
                // marker after it needs a second rebuild or the configured
                // row keeps its idle label.
                self.workspace.record_project_launch_failure(&project.name);
                self.workspace.rebuild_sidebar();
                self.status = PROJECT_STATUS_LAUNCH_FAILED;
                self.show_status = true;
                self.redraw_needed = true;
                None
            }
        }
    }

    /// Spawn a real PTY session running a configured agent's command, and
    /// give it the live view.
    ///
    /// This is the agent-row runtime: the new sidebar row is a
    /// `SessionKind::Agent` registry session backed by a PTY whose child is
    /// the configured argv vector — a shell-free launch (see
    /// [`noren_pty::AgentLaunchPolicy`]), so configuration text can never
    /// become shell syntax. The registry observes `Running` when the spawn
    /// succeeds and `Failed` when it does not (a missing or non-executable
    /// command lands here as a visible failure: the configured row shows the
    /// `AG-ERR` state, the session row shows `failed`, and the status row
    /// carries the fixed failure line — never a hang, never a silent
    /// no-op). A successful launch takes the live view and parks the
    /// previous session, the same convention as every other launch; a
    /// failed launch leaves the current live session untouched.
    fn spawn_agent_session(&mut self, agent: &ConfiguredAgent) -> Option<SessionId> {
        let id = self.workspace.create_session(SessionKind::Agent {
            name: agent.name.clone(),
        });
        let Some((terminal, pty_size)) = self.session_surfaces() else {
            self.workspace.observe_session(
                id,
                SessionStatus::Failed {
                    reason: "terminal surface unavailable".to_owned(),
                },
            );
            self.workspace.record_agent_launch_failure(&agent.name);
            self.workspace.rebuild_sidebar();
            self.redraw_needed = true;
            return None;
        };
        // Configuration validated the command at load; the policy validation
        // is defense in depth and folds into the same visible failure path
        // rather than panicking on a value this process did not re-validate.
        let launch = noren_pty::AgentLaunchPolicy::new(&agent.command, &agent.args)
            .and_then(|policy| noren_pty::PtySession::spawn_agent(policy, pty_size));
        match launch {
            Ok(pty) => {
                self.workspace.observe_session(id, SessionStatus::Running);
                self.workspace.clear_agent_launch_failure(&agent.name);
                self.park_active_session();
                self.pty = Some(pty);
                self.terminal = Some(terminal);
                self.scroll_offset = 0;
                self.active_session = Some(id);
                self.pty_child = PtyChildStatus::Running;
                self.selection = None;
                self.drag_origin = None;
                self.exited_surface_session = None;
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
                // observe_session already rebuilt once; recording the row
                // marker after it needs a second rebuild or the configured
                // row keeps its idle label.
                self.workspace.record_agent_launch_failure(&agent.name);
                self.workspace.rebuild_sidebar();
                self.status = AGENT_STATUS_LAUNCH_FAILED;
                self.show_status = true;
                self.redraw_needed = true;
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
                    let was_alternate = parked.terminal.modes().is_alternate_screen_active();
                    let was_tracking_mouse = Self::application_tracks_mouse(&parked.terminal);
                    parked.terminal.feed_bytes(&bytes);
                    let entered_alternate =
                        !was_alternate && parked.terminal.modes().is_alternate_screen_active();
                    let claimed_mouse =
                        !was_tracking_mouse && Self::application_tracks_mouse(&parked.terminal);
                    parked.scroll_offset = if entered_alternate || claimed_mouse {
                        0
                    } else {
                        Self::clamped_scroll_offset(&parked.terminal, parked.scroll_offset)
                    };
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

    /// Switch the live view to `id`.
    ///
    /// The active surface is parked (its PTY keeps running) and `id`'s parked
    /// surface is re-attached, so from the next event onward the renderer,
    /// input routing, and mouse mapping operate on the selected session —
    /// they read the same active fields, which now belong to `id`. Switching
    /// back re-attaches the same terminal state, so the session shows its own
    /// current screen, not a stale or foreign one.
    ///
    /// Returns `false` when `id` has no live surface (a model-only, restored,
    /// or already exited row): nothing is detached and the current live view
    /// keeps input ownership.
    fn switch_live_session(&mut self, id: SessionId) -> bool {
        if self.workspace.registry().get(id).is_none() {
            return false;
        }
        if self.active_session == Some(id) {
            // Already the live view; re-affirm the selection only.
            return self.workspace.select_session(id).is_ok();
        }
        let Some(parked) = self.parked_sessions.remove(&id) else {
            return false;
        };
        self.park_active_session();
        self.pty = Some(parked.pty);
        self.terminal = Some(parked.terminal);
        self.scroll_offset = self.terminal.as_ref().map_or(0, |terminal| {
            Self::clamped_scroll_offset(terminal, parked.scroll_offset)
        });
        self.active_session = Some(id);
        self.pty_child = PtyChildStatus::Running;
        // Grid coordinates captured on the previous session's screen can
        // only address the wrong content; the selection model expires them.
        self.selection = None;
        self.drag_origin = None;
        self.exited_surface_session = None;
        self.redraw_needed = true;
        self.workspace.select_session(id).is_ok()
    }

    /// Close session `id` for real: reap its child, remove its row, and
    /// repair the live view.
    ///
    /// This is the palette `session_close` runtime. A row with a live surface
    /// (the active one or a parked one) owns a real child; closing it runs
    /// that child's bounded kill-and-reap shutdown *before* the row is
    /// removed, so a closed session can never keep a process running behind a
    /// vanished row. A row without a live surface (model-only or restored) is
    /// closed in the registry alone, exactly as before.
    ///
    /// # Fallback when the active session is closed
    ///
    /// The live view moves to the remaining live session with the lowest id —
    /// the topmost sidebar row — which is deterministic and matches the
    /// row-order the user sees. When no live session remains the live view is
    /// cleared entirely: no terminal surface, no input owner, a truthful
    /// status line, and the sidebar's empty state. The palette can create a
    /// new session from there; an empty workspace never shows a closed
    /// session's frozen frame as if it were alive.
    fn close_session(&mut self, id: SessionId) -> bool {
        if self.workspace.registry().get(id).is_none() {
            return false;
        }
        let was_active = self.active_session == Some(id);
        // The displayed frame can also belong to a session whose child has
        // already exited: `finish_pty` keeps its final frame on screen with
        // input ownership already gone. Closing that row must clear the
        // surface and run the fallback too — otherwise a frozen frame stays
        // behind a vanished row in an empty workspace.
        let owns_displayed_surface = was_active || self.exited_surface_session == Some(id);
        // Reap a parked child first; its surface never touched the live view.
        if let Some(mut parked) = self.parked_sessions.remove(&id)
            && parked.pty.shutdown().is_err()
        {
            eprintln!("Noren closed-session PTY shutdown reached its failure fallback");
        }
        // Detach the displayed surface before removing the row so the renderer
        // and input routing can never observe a closed session.
        if owns_displayed_surface {
            self.active_session = None;
            self.exited_surface_session = None;
            self.terminal = None;
            self.scroll_offset = 0;
            self.pty_child = PtyChildStatus::NotLaunched;
            self.selection = None;
            self.drag_origin = None;
            if let Some(mut session) = self.pty.take()
                && session.shutdown().is_err()
            {
                eprintln!("Noren closed-session PTY shutdown reached its failure fallback");
            }
        }
        // The registry removes the row (and clears the selection if it pointed
        // at the closed session) and persists the structural change.
        let closed = self.workspace.close_session(id).is_ok();
        if owns_displayed_surface {
            // Fall back to the topmost remaining live session, if any exists.
            let fallback = self.parked_sessions.keys().min().copied();
            match fallback {
                Some(next) => {
                    self.switch_live_session(next);
                }
                None => {
                    self.status = "Noren last session closed";
                    self.show_status = true;
                }
            }
        }
        self.redraw_needed = true;
        closed
    }

    /// Cycle the live view to the next live session in sidebar order.
    ///
    /// This is the palette `session_select` runtime. The live view moves from
    /// the active session to the next live row in registry order (the order
    /// the sidebar shows), wrapping around, through the same
    /// [`switch_live_session`] path a sidebar click takes. With fewer than
    /// two live sessions there is nothing to cycle to: the current live view
    /// is re-affirmed, and input ownership never moves to a row without a
    /// live surface.
    fn select_next_live_session(&mut self) {
        let live: Vec<SessionId> = self
            .workspace
            .registry()
            .sessions()
            .into_iter()
            .map(|descriptor| descriptor.id())
            .filter(|id| self.active_session == Some(*id) || self.parked_sessions.contains_key(id))
            .collect();
        let Some(next) = live
            .iter()
            .position(|id| self.active_session == Some(*id))
            .and_then(|position| live.get((position + 1) % live.len()).copied())
            .or_else(|| live.first().copied())
        else {
            return;
        };
        if self.switch_live_session(next) {
            self.ssh_selection_status = None;
            self.redraw_needed = true;
        }
    }

    fn initialize(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title(window_title())
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
        self.renderer = match Renderer::new(
            Arc::clone(&window),
            self.geometry.cell_metrics(),
            self.sidebar_columns,
            self.theme,
            self.cursor_style,
        ) {
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

    /// Maximum locally addressable history for one terminal surface.
    /// Alternate-screen applications own their complete view and expose no
    /// Noren scrollback, even though primary history remains retained.
    fn max_scroll_offset(terminal: &TerminalState) -> usize {
        if terminal.modes().is_alternate_screen_active() {
            0
        } else {
            terminal.scrollback_len()
        }
    }

    fn application_tracks_mouse(terminal: &TerminalState) -> bool {
        terminal_wheel_owner(terminal.modes()) == TerminalWheelOwner::Application
    }

    fn clamped_scroll_offset(terminal: &TerminalState, requested: usize) -> usize {
        requested.min(Self::max_scroll_offset(terminal))
    }

    fn clamp_active_scroll_offset(&mut self) {
        self.scroll_offset = self.terminal.as_ref().map_or(0, |terminal| {
            Self::clamped_scroll_offset(terminal, self.scroll_offset)
        });
    }

    /// Move the active primary-screen viewport by a bounded number of rows.
    /// Returns the before/after offsets so the wheel ownership test can prove
    /// local consumption without relying on a frame merely changing.
    fn scroll_view(&mut self, direction: ScrollDirection, rows: usize) -> (usize, usize) {
        self.clamp_active_scroll_offset();
        let before = self.scroll_offset;
        let max_offset = self.terminal.as_ref().map_or(0, Self::max_scroll_offset);
        self.scroll_offset = match direction {
            ScrollDirection::Older => self.scroll_offset.saturating_add(rows).min(max_offset),
            ScrollDirection::Newer => self.scroll_offset.saturating_sub(rows),
        };
        if self.scroll_offset != before {
            self.selection = None;
            self.drag_origin = None;
            self.redraw_needed = true;
        }
        (before, self.scroll_offset)
    }

    fn handle_scrollback_chord(&mut self, chord: Chord) -> bool {
        let Some(terminal) = self.terminal.as_ref() else {
            return false;
        };
        if terminal.modes().is_alternate_screen_active() {
            return false;
        }
        let direction = if chord == self.keys.scroll_page_up() {
            ScrollDirection::Older
        } else if chord == self.keys.scroll_page_down() {
            ScrollDirection::Newer
        } else {
            return false;
        };
        let page_rows = usize::from(terminal.size().0).max(1);
        self.scroll_view(direction, page_rows);
        true
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
        if event.state == ElementState::Pressed
            && let Some(chord) = chord_from_event(event, self.modifiers)
            && self.handle_scrollback_chord(chord)
        {
            return;
        }
        self.handle_passthrough_key(event);
    }

    /// Route one key event through the pass-through gate.
    ///
    /// The gate claims the exit leader (Super+Escape) and the configured
    /// palette chord. Everything else is forwarded byte-for-byte through the
    /// same encoder path as before the gate existed, so a closed-palette key
    /// press is byte-identical to the pre-gate behaviour.
    fn handle_passthrough_key(&mut self, event: &KeyEvent) {
        if event.state == ElementState::Pressed
            && !event.repeat
            && let Some(chord) = chord_from_event(event, self.modifiers)
            && self.recover_empty_workspace_with_chord(chord)
        {
            return;
        }
        let input_mode = self.current_input_mode();
        let encoded = if let Some(input) = translate_keypad_key(event) {
            KeyEncoder::encode_keypad_with(input.with_modifiers(self.modifiers), input_mode)
        } else {
            translate_key(event, self.modifiers)
                .and_then(|input| KeyEncoder::encode_with(input, input_mode))
        };
        if event.state == ElementState::Pressed
            && let Some(chord) = chord_from_event(event, self.modifiers)
            && self.gate_pressed_chord(chord, input_mode)
        {
            return;
        }
        let Ok(bytes) = encoded else {
            return;
        };
        self.send_input(&bytes);
    }

    /// Honor the configured create-session chord directly at the dead end.
    ///
    /// Outside an empty workspace the chord keeps its existing scope inside
    /// the open palette, so ordinary terminal input is unchanged. When the UI
    /// says `No sessions`, no PTY can receive the key; consuming the exact
    /// chord shown by [`noren_app::ui::empty_workspace_recovery`] turns that
    /// inert state into a real recovery action.
    fn recover_empty_workspace_with_chord(&mut self, chord: Chord) -> bool {
        if !self.workspace.sidebar().is_empty() || chord != self.keys.session_create() {
            return false;
        }
        self.run_workspace_action(WorkspaceAction::CreateSession);
        true
    }

    /// Route one pressed chord through the gate.
    ///
    /// This is the seam the key-event path delegates to; it owns the palette
    /// claim, so the configured opener chord — and nothing else — opens the
    /// palette. Returns whether the chord was consumed (intercepted or held
    /// pending); a forwarded chord replays any held leader bytes and falls
    /// through to normal encoding.
    fn gate_pressed_chord(&mut self, chord: Chord, input_mode: InputMode) -> bool {
        let decision = self.passthrough_gate.press(&self.passthrough_policy, chord);
        match decision.kind {
            GateKind::Intercepted(PassthroughAction::OpenCommandPalette) => {
                self.open_palette();
                true
            }
            GateKind::Intercepted(PassthroughAction::ExitToWorkspace) => true,
            GateKind::Pending => true,
            GateKind::Forwarded => {
                for replayed in &decision.replayed {
                    if let Some(bytes) = encode_chord(replayed, input_mode) {
                        self.send_input(&bytes);
                    }
                }
                false
            }
        }
    }

    /// Handle a key event while the palette is open.
    ///
    /// The four configured command chords dispatch their commands; Escape
    /// dismisses without running; Arrow Up/Down and Enter navigate and
    /// confirm the selection. A single character that matches no command
    /// chord dismisses the palette, as before configuration existed.
    fn handle_palette_key(&mut self, event: &KeyEvent) {
        self.handle_palette_key_impl(&event.logical_key, event.state, event.repeat);
    }

    /// Window-independent seam for the palette key path: the real handler
    /// with the winit event reduced to the parts it reads.
    fn handle_palette_key_impl(
        &mut self,
        logical_key: &WinitKey,
        state: ElementState,
        repeat: bool,
    ) {
        if state != ElementState::Pressed || repeat {
            return;
        }
        match logical_key {
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
                let pressed = chord_from_logical(logical_key, self.modifiers);
                let bare = GateKeyCode::Char(ch.to_ascii_lowercase());
                match palette_command_for(&self.keys, pressed, Some(bare)) {
                    Some(id) => self.dispatch_palette_command(id),
                    None => self.close_palette(),
                }
            }
            _ => {
                let pressed = chord_from_logical(logical_key, self.modifiers);
                if let Some(id) = palette_command_for(&self.keys, pressed, None) {
                    self.dispatch_palette_command(id);
                }
            }
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
                // The palette cycles the live view through live sessions in
                // sidebar order — the same switch a sidebar click performs.
                self.select_next_live_session();
            }
            WorkspaceAction::CloseSession => {
                // The palette closes the selected row — live or not. A live
                // row owns a real child; `close_session` reaps it before
                // removing the row and repairs the live view (fallback to the
                // topmost remaining live session, or an honest empty view).
                let target = self.workspace.registry().selected().or_else(|| {
                    self.workspace
                        .registry()
                        .sessions()
                        .first()
                        .map(|descriptor| descriptor.id())
                });
                if let Some(id) = target {
                    self.close_session(id);
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
        if let Some(selection) = self.terminal.as_ref().map(Selection::entire_grid) {
            self.show_selection(selection);
        }
    }

    /// Install a user-visible selection and schedule the frame that paints it.
    ///
    /// Selection input can arrive while the PTY is idle, so relying on output
    /// to request a redraw would leave the new range invisible indefinitely.
    fn show_selection(&mut self, selection: Selection) {
        self.selection = Some(selection);
        self.redraw_needed = true;
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
        if let Some(selection) = self
            .terminal
            .as_ref()
            .map(|terminal| Selection::new(terminal, self.drag_mode, origin, point))
        {
            self.show_selection(selection);
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
                let selection = Selection::new(terminal, mode, point, point);
                self.show_selection(selection);
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
            // A row with a live surface takes the live view: the terminal
            // surface, input routing, and the renderer follow the selection.
            if self.switch_live_session(id) {
                self.ssh_selection_status = None;
                self.redraw_needed = true;
                return true;
            }
            // The row has no live surface (model-only, restored, or exited):
            // input ownership stays with the current live session, but the
            // CLICK selects the clicked row — the palette's close command
            // operates on the selected row, and re-selecting the live one
            // here would redirect a close onto a shell the user did not
            // point at.
            if self.workspace.select_session(id).is_ok() {
                self.ssh_selection_status = None;
                self.redraw_needed = true;
            }
            return true;
        }
        if let Some(project) = self.workspace.project_sidebar_row(row_index) {
            // A project row whose configured root still exists launches a
            // real session rooted at that directory; a configured-but-gone
            // root is refused before any session or child exists — exactly
            // like a registered-but-deleted worktree. Both are first-class,
            // visible outcomes.
            let project = project.clone();
            if project.root.is_dir() {
                self.spawn_project_session(&project);
            } else {
                self.workspace.record_project_launch_failure(&project.name);
                self.workspace.rebuild_sidebar();
                self.status = PROJECT_STATUS_MISSING;
                self.show_status = true;
            }
            self.redraw_needed = true;
            return true;
        }
        if let Some(worktree) = self.workspace.worktree_sidebar_row(row_index) {
            // A present worktree row launches a real session rooted at the
            // worktree directory; a registered-but-deleted one is refused
            // before any session or child exists. Both are first-class,
            // visible outcomes.
            let path = worktree.path().to_owned();
            if worktree.directory_present() {
                self.spawn_worktree_session(&path);
            } else {
                self.status = WORKTREE_STATUS_MISSING;
                self.show_status = true;
            }
            self.redraw_needed = true;
            return true;
        }
        if self.workspace.select_ssh_sidebar_row(row_index) {
            let target = self.workspace.selected_ssh_target().map(str::to_owned);
            let source = self
                .workspace
                .selected_ssh_source_label()
                .map(ssh_status_source_label)
                .unwrap_or_else(|| "#? source unavailable".to_owned());
            self.ssh_selection_status = Some(format!("SSH partial source {source}; offline"));
            let outcome = match target.as_deref() {
                Some(target) => self.connect_ssh_target(target),
                None => return true,
            };
            // A refusal keeps its typed message; otherwise refresh the
            // provenance line with the launch's observed phase word.
            if let SshConnectOutcome::Launched(phase) = outcome {
                self.ssh_selection_status = Some(format!(
                    "SSH partial source {source}; {}",
                    phase.sidebar_detail()
                ));
            }
            self.redraw_needed = true;
            return true;
        }
        if let Some(agent) = self.workspace.agent_sidebar_row(row_index) {
            // A configured agent row launches its validated argv in a PTY;
            // the outcome (a running session or a visible launch failure) is
            // first-class, exactly like a worktree row.
            let agent = agent.clone();
            self.spawn_agent_session(&agent);
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
            || position.x
                >= sidebar_pixel_width_at_width(self.geometry.cell_width(), self.sidebar_columns)
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
            || position.x
                >= sidebar_pixel_width_at_width(self.geometry.cell_width(), self.sidebar_columns)
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

    /// Resolve and apply the terminal-side ownership of one wheel delta.
    ///
    /// Any authoritative tracking mode (9/1000/1002/1003) gives the application
    /// the wheel, including when Shift is held. Without tracking, Noren
    /// consumes the delta as local primary-screen history navigation. This is
    /// the ADR-0003 boundary in one branch: inside belongs to Zellij/vim when
    /// they ask for it; otherwise the outer terminal supplies its own view.
    fn route_terminal_wheel(
        &mut self,
        delta: MouseScrollDelta,
        cell: Option<(u32, u32)>,
    ) -> TerminalWheelRoute {
        let clicks = wheel_clicks(delta, self.geometry.cell_metrics());
        let owner = self
            .terminal
            .as_ref()
            .map_or(TerminalWheelOwner::LocalHistory, |terminal| {
                terminal_wheel_owner(terminal.modes())
            });
        if owner == TerminalWheelOwner::Application {
            let mut reports = Vec::new();
            if let Some((col, row)) = cell {
                let modifiers = self.pointer_modifiers();
                for direction in clicks {
                    if let Some(report) =
                        self.encode_mouse(PointerEvent::wheel(direction, col, row, modifiers))
                    {
                        reports.push(report);
                    }
                }
            }
            return TerminalWheelRoute::ForwardedToApplication(reports);
        }

        let before = self.scroll_offset;
        for direction in clicks {
            let direction = match direction {
                noren_app::mouse::WheelDirection::Up => ScrollDirection::Older,
                noren_app::mouse::WheelDirection::Down => ScrollDirection::Newer,
            };
            self.scroll_view(direction, 1);
        }
        TerminalWheelRoute::ConsumedLocally {
            before,
            after: self.scroll_offset,
        }
    }

    /// Handle a scroll-wheel event. Sidebar chrome keeps its existing outside-
    /// terminal ownership; over the terminal, [`Self::route_terminal_wheel`]
    /// makes the application/local decision before any state is changed.
    fn handle_mouse_wheel(&mut self, delta: MouseScrollDelta) {
        if let Some(frame_size) = self.window.as_ref().map(|window| window.inner_size())
            && self.handle_sidebar_wheel_in_frame(delta, frame_size)
        {
            return;
        }
        let cell = self
            .cursor_position
            .and_then(|position| self.mouse_cell_at(position));
        if let TerminalWheelRoute::ForwardedToApplication(reports) =
            self.route_terminal_wheel(delta, cell)
        {
            for report in reports {
                self.send_input(&report);
            }
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
        // The sidebar occupies the configured leftmost cell columns; clicks
        // inside it do not address the terminal grid.
        if position.x < sidebar_pixel_width_at_width(cell_width, self.sidebar_columns) {
            return None;
        }
        let scroll_offset = Self::clamped_scroll_offset(terminal, self.scroll_offset);
        let content_rows = if scroll_offset == 0 {
            terminal.screen().display_row_count()
        } else {
            usize::from(terminal.size().0)
        };
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
        let column = terminal_column_at_width(position.x, cols, cell_width, self.sidebar_columns)?;
        let logical_start = terminal.scrollback_len().saturating_sub(scroll_offset);
        Some(GridPoint::new(logical_start + line_index, column))
    }

    /// Whether pointer events should be reported to the PTY instead of driving
    /// local text selection. Active when a tracking mode (9/1000/1002/1003) is
    /// on
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
        if Self::application_tracks_mouse(terminal) {
            if !position.x.is_finite()
                || !position.y.is_finite()
                || position.x < 0.0
                || position.y < 0.0
                || position.x >= f64::from(frame_size.width)
                || position.y >= f64::from(frame_size.height)
                || position.x
                    < sidebar_pixel_width_at_width(self.geometry.cell_width(), self.sidebar_columns)
            {
                return None;
            }
            let (rows, cols) = terminal.size();
            let row = pixel_row_index(position.y, self.geometry.cell_height())?;
            if row >= usize::from(rows) {
                return None;
            }
            let column = terminal_column_at_width(
                position.x,
                cols,
                self.geometry.cell_width(),
                self.sidebar_columns,
            )?;
            return Some((u32::try_from(column).ok()?, u32::try_from(row).ok()?));
        }
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
            .with_x10(modes.is_mouse_x10_tracking_enabled())
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
                window.set_title(&window_title());
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
        let runtime = RuntimeGridSize::from_window(grid, self.sidebar_columns);
        if let Some(terminal) = &mut self.terminal {
            if runtime.resize_terminal(terminal).is_err() {
                self.status = "Noren terminal resize failed";
                self.show_status = true;
            }
            self.scroll_offset = Self::clamped_scroll_offset(terminal, self.scroll_offset);
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
                parked.scroll_offset =
                    Self::clamped_scroll_offset(&parked.terminal, parked.scroll_offset);
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
            let was_alternate = terminal.modes().is_alternate_screen_active();
            let was_tracking_mouse = Self::application_tracks_mouse(terminal);
            terminal.feed_bytes(bytes);
            // A program entering an alternate screen or claiming the mouse
            // owns the visible terminal surface. Rejoin its live screen before
            // routing subsequent input; ordinary output otherwise preserves a
            // deliberate non-zero history offset and follows automatically at
            // offset zero.
            let entered_alternate = !was_alternate && terminal.modes().is_alternate_screen_active();
            let claimed_mouse = !was_tracking_mouse && Self::application_tracks_mouse(terminal);
            if entered_alternate || claimed_mouse {
                self.scroll_offset = 0;
            } else {
                self.scroll_offset = Self::clamped_scroll_offset(terminal, self.scroll_offset);
            }
        }
        // First output from an ssh child means the remote side is talking
        // through the normal I/O path: the launch is interactive now.
        if self.ssh_live()
            && self
                .workspace
                .ssh_connection()
                .is_some_and(|(_, phase)| matches!(phase, SshConnectionPhase::Connecting))
        {
            self.apply_ssh_phase(SshConnectionPhase::Connected);
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
                    if self.ssh_live() {
                        // The reader hit EOF; the supervisor's reaped exit
                        // event follows within one poll. Wait for it (and
                        // its honest code) instead of guessing now.
                        self.ssh_eof_since.get_or_insert(Instant::now());
                    } else {
                        terminal_status = Some("Noren shell reached EOF");
                    }
                }
                PtyEvent::Exited { code } => {
                    self.pty_child = PtyChildStatus::Exited { code };
                    terminal_status = Some(if self.ssh_live() {
                        let phase = ssh_exit_observation(code);
                        self.apply_ssh_phase(phase);
                        phase.status_text()
                    } else if code == Some(0) {
                        "Noren shell exited"
                    } else {
                        "Noren shell exited with failure"
                    });
                    break;
                }
                PtyEvent::Error(_) => {
                    terminal_status = Some(if self.ssh_live() {
                        self.apply_ssh_phase(SshConnectionPhase::Disconnected);
                        SSH_STATUS_DISCONNECTED
                    } else {
                        "Noren PTY operation failed"
                    });
                    break;
                }
            }
        }
        // An ssh child whose reaped exit never arrived after EOF is an
        // immediate disconnect; surface it rather than waiting forever.
        if terminal_status.is_none()
            && let Some(eof_since) = self.ssh_eof_since
            && eof_since.elapsed() >= SSH_EOF_REAP_GRACE
        {
            self.ssh_eof_since = None;
            if self.ssh_live() {
                self.apply_ssh_phase(SshConnectionPhase::Disconnected);
                terminal_status = Some(SSH_STATUS_DISCONNECTED);
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
        self.ssh_eof_since = None;
        if let Some(id) = self.active_session.take() {
            let code = match self.pty_child {
                PtyChildStatus::Exited { code } => code.map(|c| c as i32),
                _ => None,
            };
            self.workspace
                .observe_session(id, SessionStatus::Exited { code });
            // The final frame stays displayed below; remember whose it is so
            // closing that row detaches the surface honestly.
            self.exited_surface_session = Some(id);
        }
        if let Some(mut session) = self.pty.take()
            && session.shutdown().is_err()
        {
            self.status = "Noren PTY shutdown failed";
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        self.clamp_active_scroll_offset();
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
        let sidebar_rows = visible_sidebar_text_rows_at_width(
            self.workspace.sidebar(),
            self.sidebar_scroll_offset,
            visible_rows,
            self.sidebar_columns,
        );
        let rows = if self.palette_open {
            let mut rows: Vec<_> =
                palette_text_lines(self.workspace.palette(), self.palette_selection, &self.keys)
                    .into_iter()
                    .map(SidebarTextRow::chrome)
                    .collect();
            rows.extend(sidebar_rows);
            rows
        } else {
            sidebar_rows
        };
        let status = status_row.map(|source| {
            source.text(
                self.status,
                self.ssh_selection_status.as_deref(),
                self.worktree_diagnostic.as_deref(),
                self.project_diagnostic.as_deref(),
                self.agent_diagnostic.as_deref(),
                self.ssh_diagnostic.as_deref(),
            )
        });
        // A one-row window deliberately gives its only row to the PTY; keep
        // the hint on the same status-row presence decision so rendering,
        // terminal sizing, and pointer mapping cannot disagree there.
        let palette_hint = status
            .as_ref()
            .and_then(|_| noren_app::ui::palette_hint(self.keys, self.ui));
        let viewport_indicator = status
            .as_ref()
            .and_then(|_| noren_app::ui::scrollback_indicator(self.scroll_offset, self.keys));
        let workspace_notice = (self.workspace.sidebar().is_empty() && self.terminal.is_none())
            .then(|| noren_app::ui::empty_workspace_recovery(self.keys));
        let chrome = FrameChrome::new(None, status)
            .with_sidebar_rows(Some(&rows))
            .with_viewport_indicator(viewport_indicator.as_deref())
            .with_palette_hint(palette_hint.as_deref())
            .with_workspace_notice(workspace_notice.as_deref())
            .with_scroll_offset(self.scroll_offset)
            .with_selection(self.selection.as_ref());
        let outcome = self
            .renderer
            .as_mut()
            .map(|renderer| renderer.render(snapshot.as_ref(), chrome));
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
            WindowEvent::Focused(focused) => {
                // Focus loss must be visible in the caret (issue #200): the
                // renderer switches between the focused mark and the
                // unfocused hollow outline.
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.set_focused(focused);
                }
                self.redraw_needed = true;
            }
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
    fn from_window(grid: GridSize, sidebar_columns: usize) -> Self {
        Self {
            rows: NorenApp::content_terminal_rows(grid.rows()),
            cols: terminal_cols_at_width(grid.cols(), sidebar_columns),
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

/// Convert the sidebar view into text lines at the shipped width.
///
/// Production and the integration frame oracle share the width-aware
/// implementation in `sidebar_text`; this test seam keeps existing binary
/// tests on the exact same projection.
#[cfg(test)]
fn sidebar_text_lines(sidebar: &SidebarView) -> Vec<String> {
    visible_sidebar_text_lines(sidebar, 0, usize::MAX)
}

/// Build text lines for the palette display, drawn at the top of the sidebar
/// column when the palette is open.
///
/// Each command is one line: `]` marks the selected command, space otherwise,
/// followed by a single-key shortcut and the label. The lines are uppercase
/// to match the bitmap font's case-folding.
fn palette_text_lines(
    palette: &Palette<WorkspaceAction>,
    selection: usize,
    keys: &KeymapConfig,
) -> Vec<String> {
    palette
        .iter()
        .enumerate()
        .map(|(idx, cmd)| {
            let marker = if idx == selection { ']' } else { ' ' };
            let key = command_shortcut(cmd.id(), keys);
            format!("{marker}{key} {label}", label = cmd.label())
        })
        .collect()
}

/// The one-character palette display shortcut for a command's configured
/// chord.
///
/// A character binding shows its (upper-cased) character; any other chord —
/// modifiers or a named key — has no single-glyph representation in the
/// bitmap font and shows `?`. The label therefore never claims a shortcut
/// the chord does not carry.
fn command_shortcut(id: CommandId, keys: &KeymapConfig) -> char {
    let binding = match id {
        CommandId::SESSION_CREATE => keys.session_create(),
        CommandId::SESSION_SELECT => keys.session_select(),
        CommandId::SESSION_CLOSE => keys.session_close(),
        CommandId::SIDEBAR_FOCUS => keys.sidebar_focus(),
        _ => return '?',
    };
    match binding.code() {
        GateKeyCode::Char(character) => character.to_ascii_uppercase(),
        _ => '?',
    }
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
    app.load_git_worktrees();
    app.load_ssh_hosts();
    if event_loop.run_app(&mut app).is_err() {
        eprintln!("Noren event loop failed");
    }
}

#[cfg(test)]
#[path = "main/tests.rs"]
mod tests;
