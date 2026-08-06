//! Sidebar view model: a renderer-independent description of the left sidebar
//! and the single-viewport workspace skeleton.
//!
//! This module describes *what* the workspace shows, never *how* to paint it:
//! the public types carry text, entry kinds, selection, and visibility only,
//! with no colors, geometry, or widget types. The renderer consumes immutable
//! snapshots (compare `noren_terminal::TerminalSnapshot`), so the builder
//! returns a new [`SidebarView`] value and never mutates a handed-out one.
//!
//! The model respects the Noren/Zellij boundary (ADR 0003): Noren manages the
//! workspace outside the terminal, Zellij manages it inside. There is no pane,
//! tab, layout, or split type anywhere in this module. The right-hand side
//! shows exactly one session through [`SessionViewport`], which names the
//! visible session and nothing about what is displayed inside it.
//!
//! Sessions are described by the shared contract types (`SessionDescriptor`
//! and friends, owned by the sibling `session` module per D-M3-001); this
//! module imports them and never redefines them. Sidebar entries that are
//! not live sessions (projects, worktrees, reserved SSH and agent
//! connections) are plain text facts, and the bundled fixtures construct
//! everything without launching a process, an SSH connection, or an agent.

use crate::session::{SessionDescriptor, SessionId, SessionKind, SessionStatus};

/// The entry classes the sidebar can list.
///
/// This view-level taxonomy is deliberately wider than [`SessionKind`]:
/// projects and worktrees are workspace anchors rather than launch shapes,
/// and an SSH connection or agent entry is a reserved fixture that may exist
/// in the sidebar before any session of that shape runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntryKind {
    /// A project anchored at a directory.
    Project,
    /// A git worktree anchored at a branch checkout.
    Worktree,
    /// A reserved SSH connection. Fixture only: no connection is opened.
    SshConnection,
    /// A reserved agent. Fixture only: no agent is launched.
    Agent,
    /// A live terminal session described by a `SessionDescriptor`.
    Session,
}

/// One sidebar row: what to show, not how to draw it.
///
/// A renderer maps [`SidebarRow::kind`] to a distinguishable treatment and
/// shows [`SidebarRow::label`] plus the optional [`SidebarRow::detail`].
/// Selection is carried as a boolean so the row type stays free of geometry
/// and color.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarRow {
    kind: EntryKind,
    label: String,
    detail: Option<String>,
    selected: bool,
}

impl SidebarRow {
    /// The entry class of this row.
    #[must_use]
    pub const fn kind(&self) -> EntryKind {
        self.kind
    }

    /// The primary text of this row.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Optional secondary text, such as a path, branch, host, or status.
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// Whether this row carries the single selection.
    #[must_use]
    pub const fn is_selected(&self) -> bool {
        self.selected
    }
}

/// A sidebar entry before projection into view rows.
///
/// Session entries carry the shared contract descriptor; the other kinds are
/// plain text facts describing workspace items that are not live sessions.
/// Construction is fixture-only: no variant opens a connection or launches a
/// process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarEntry {
    /// A project anchored at `root`.
    Project {
        /// Display name of the project.
        name: String,
        /// The directory the project is rooted at, as display text.
        root: String,
    },
    /// A git worktree checked out at `branch`.
    Worktree {
        /// Display name of the worktree.
        name: String,
        /// The branch checked out in this worktree.
        branch: String,
    },
    /// A reserved SSH connection to `host`.
    SshConnection {
        /// Display name of the connection.
        label: String,
        /// The host the connection targets, as display text.
        host: String,
    },
    /// A reserved agent entry.
    Agent {
        /// Display name of the agent.
        label: String,
    },
    /// A live terminal session, described by the shared contract descriptor.
    Session(SessionDescriptor),
}

/// The view shown when the sidebar has no entries.
///
/// A value in its own right so the empty state is representable and testable
/// without inventing placeholder rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmptyState {
    message: String,
}

/// Message the builder shows when no entries were given.
pub const EMPTY_SIDEBAR_MESSAGE: &str = "No sessions";

impl EmptyState {
    /// An empty-state notice carrying `message`.
    #[must_use]
    pub fn new(message: String) -> Self {
        Self { message }
    }

    /// The notice text a renderer is expected to show.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// The single viewport: exactly one session visible on the right-hand side.
///
/// Per ADR 0003, what happens inside the session is Zellij's business, so
/// this type carries the session's identity only. It has no pane, tab,
/// layout, geometry, or terminal-content field, and none may be added here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionViewport {
    session: SessionDescriptor,
}

impl SessionViewport {
    /// The descriptor of the visible session.
    #[must_use]
    pub const fn descriptor(&self) -> &SessionDescriptor {
        &self.session
    }

    /// The identifier of the visible session.
    #[must_use]
    pub fn session_id(&self) -> SessionId {
        self.session.id()
    }

    /// The visible session's own label, if it has one.
    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.session.label()
    }

    /// The text a renderer is expected to use as the viewport title: the
    /// session label when present, otherwise the session identifier.
    #[must_use]
    pub fn title(&self) -> String {
        self.label()
            .map_or_else(|| self.session.id().to_string(), str::to_string)
    }
}

/// Immutable snapshot of the workspace skeleton: the left sidebar and, at
/// most, the single right-hand viewport.
///
/// Invariants enforced by [`SidebarView::build`]:
///
/// 1. `empty_state` is `Some` exactly when there are no rows.
/// 2. At most one row has [`SidebarRow::is_selected`] set.
/// 3. `viewport` is `Some` exactly when one session row is selected, and it
///    names that same session. Unselected sessions describe no viewport.
/// 4. A selection that matches no entry is dropped, never rendered dangling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarView {
    rows: Vec<SidebarRow>,
    empty_state: Option<EmptyState>,
    viewport: Option<SessionViewport>,
}

impl SidebarView {
    /// Project `entries` into an immutable view with `selected` applied.
    ///
    /// Entries render in order. The selection applies to session entries
    /// only; the first session entry matching `selected` becomes the single
    /// selected row and the single [`SessionViewport`]. Duplicate
    /// descriptions of the same session render without selection, so the
    /// one-selection invariant holds for every input.
    #[must_use]
    pub fn build(entries: &[SidebarEntry], selected: Option<SessionId>) -> Self {
        let mut viewport: Option<SessionViewport> = None;
        let mut rows: Vec<SidebarRow> = Vec::with_capacity(entries.len());
        for entry in entries {
            rows.push(match entry {
                SidebarEntry::Project { name, root } => SidebarRow {
                    kind: EntryKind::Project,
                    label: name.clone(),
                    detail: Some(root.clone()),
                    selected: false,
                },
                SidebarEntry::Worktree { name, branch } => SidebarRow {
                    kind: EntryKind::Worktree,
                    label: name.clone(),
                    detail: Some(branch.clone()),
                    selected: false,
                },
                SidebarEntry::SshConnection { label, host } => SidebarRow {
                    kind: EntryKind::SshConnection,
                    label: label.clone(),
                    detail: Some(host.clone()),
                    selected: false,
                },
                SidebarEntry::Agent { label } => SidebarRow {
                    kind: EntryKind::Agent,
                    label: label.clone(),
                    detail: None,
                    selected: false,
                },
                SidebarEntry::Session(descriptor) => {
                    let is_selected = viewport.is_none() && selected == Some(descriptor.id());
                    if is_selected {
                        viewport = Some(SessionViewport {
                            session: descriptor.clone(),
                        });
                    }
                    SidebarRow {
                        kind: EntryKind::Session,
                        label: session_label(descriptor),
                        detail: Some(session_detail(descriptor)),
                        selected: is_selected,
                    }
                }
            });
        }
        let empty_state = rows
            .is_empty()
            .then(|| EmptyState::new(EMPTY_SIDEBAR_MESSAGE.to_string()));
        Self {
            rows,
            empty_state,
            viewport,
        }
    }

    /// The sidebar rows, empty when the sidebar is in its empty state.
    #[must_use]
    pub fn rows(&self) -> &[SidebarRow] {
        &self.rows
    }

    /// Whether the sidebar has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The empty-state notice, present exactly when [`Self::is_empty`].
    #[must_use]
    pub const fn empty_state(&self) -> Option<&EmptyState> {
        self.empty_state.as_ref()
    }

    /// The single viewport, present exactly when one session is selected.
    #[must_use]
    pub const fn viewport(&self) -> Option<&SessionViewport> {
        self.viewport.as_ref()
    }

    /// The number of rows carrying the selection; at most one by construction.
    #[must_use]
    pub fn selected_row_count(&self) -> usize {
        self.rows.iter().filter(|row| row.is_selected()).count()
    }
}

/// The text label for a session row: the descriptor's label when present,
/// otherwise the stable session identifier.
fn session_label(descriptor: &SessionDescriptor) -> String {
    descriptor
        .label()
        .map_or_else(|| descriptor.id().to_string(), str::to_string)
}

/// The secondary text for a session row: launch shape and last observed
/// status, such as `local · running`.
fn session_detail(descriptor: &SessionDescriptor) -> String {
    format!(
        "{} · {}",
        session_kind_text(descriptor.kind()),
        session_status_text(descriptor.status())
    )
}

fn session_kind_text(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Local => "local",
        SessionKind::Ssh => "ssh",
        SessionKind::Agent => "agent",
    }
}

fn session_status_text(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Created => "created",
        SessionStatus::Running => "running",
        SessionStatus::Failed => "failed",
        SessionStatus::Exited => "exited",
    }
}

/// Deterministic, process-free fixtures for the sidebar view.
///
/// Sessions are constructed through the shared `SessionRegistry`, which is
/// pure state and spawns nothing; SSH and agent entries are text facts only.
pub mod fixtures {
    use super::SidebarEntry;
    use crate::session::{SessionId, SessionKind, SessionRegistry, SessionStatus};

    /// A deterministic registry with three sessions and no selection: two
    /// observed-running local shells with labels and one reserved SSH-shaped
    /// session without a label, still at its created status because no
    /// observation was reported for it.
    #[must_use]
    pub fn session_registry() -> SessionRegistry {
        let mut registry = SessionRegistry::new();
        let build = registry.create(SessionKind::Local, Some("build".to_string()));
        let tests = registry.create(SessionKind::Local, Some("tests".to_string()));
        let _ = registry.create(SessionKind::Ssh, None);
        observe_running(&mut registry, build);
        observe_running(&mut registry, tests);
        debug_assert_eq!(registry.len(), 3);
        registry
    }

    fn observe_running(registry: &mut SessionRegistry, id: SessionId) {
        registry
            .observe(id, SessionStatus::Running)
            .expect("fixture observes ids the same fixture registry created");
    }

    /// One entry of each kind, in sidebar order: a project, a git worktree, a
    /// reserved SSH connection, a reserved agent, and every session in
    /// `registry`.
    #[must_use]
    pub fn entries(registry: &SessionRegistry) -> Vec<SidebarEntry> {
        let mut entries = vec![
            SidebarEntry::Project {
                name: "noren".to_string(),
                root: "~/dev/noren".to_string(),
            },
            SidebarEntry::Worktree {
                name: "pool-m3c".to_string(),
                branch: "agent/m3-sidebar-view".to_string(),
            },
            SidebarEntry::SshConnection {
                label: "web-1".to_string(),
                host: "web1.internal:22".to_string(),
            },
            SidebarEntry::Agent {
                label: "claude-code".to_string(),
            },
        ];
        entries.extend(registry.sessions().into_iter().map(SidebarEntry::Session));
        entries
    }
}
