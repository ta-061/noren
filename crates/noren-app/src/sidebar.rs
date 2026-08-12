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
//! module imports them and never redefines them. Sidebar entries that are not
//! live sessions (projects, worktrees, configured SSH targets, and reserved
//! agent entries) are plain text facts. Constructing them, including through
//! the bundled fixtures, never launches a process, opens an SSH connection, or
//! launches an agent.

use crate::session::{SessionDescriptor, SessionId, SessionKind, SessionStatus};

/// The entry classes the sidebar can list.
///
/// This view-level taxonomy is deliberately wider than [`SessionKind`]:
/// projects and worktrees are workspace anchors rather than launch shapes, a
/// configured SSH target may appear without a running session, and agent
/// entries remain reserved fixtures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntryKind {
    /// A project anchored at a directory.
    Project,
    /// A git worktree anchored at a branch checkout.
    Worktree,
    /// A configured SSH target. It is not a live connection or PTY.
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
/// Construction is data-only: no variant opens a connection or launches a
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
    /// A configured SSH target displayed in the sidebar.
    SshConnection {
        /// Display name of the target.
        label: String,
        /// Secondary display text, such as a host or connection status.
        host: String,
        /// Whether this disconnected target is the pending UI choice.
        ///
        /// This is display state only. It never denotes a live session,
        /// connection, process, or viewport.
        selected: bool,
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

    /// The text a renderer is expected to use as the viewport title.
    ///
    /// Delegates to the descriptor's auto-generated title (the session's
    /// stable display id, e.g. `"session-1"`); the session contract carries
    /// no caller-supplied label.
    #[must_use]
    pub fn title(&self) -> &str {
        self.session.title()
    }
}

/// Immutable snapshot of the workspace skeleton: the left sidebar and, at
/// most, the single right-hand viewport.
///
/// Invariants enforced by [`SidebarView::build`]:
///
/// 1. `empty_state` is `Some` exactly when there are no rows.
/// 2. At most one row has [`SidebarRow::is_selected`] set. A pending SSH row
///    suppresses the live-session row's visual marker.
/// 3. `viewport` is `Some` exactly when one non-restored session is active,
///    and it names that same session. A pending SSH marker does not replace
///    this actual viewport. A restored entry has no live process to attach to,
///    so selecting it does not create a viewport.
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
    /// Entries render in order. `selected` resolves the actual session
    /// viewport. If an SSH entry carries pending display selection, its first
    /// occurrence gets the sole visible marker and the actual session marker
    /// is suppressed without changing that viewport. Duplicate descriptions
    /// render without a second marker, so the one-selection invariant holds
    /// for every input.
    #[must_use]
    pub fn build(entries: &[SidebarEntry], selected: Option<SessionId>) -> Self {
        let pending_ssh = entries
            .iter()
            .any(|entry| matches!(entry, SidebarEntry::SshConnection { selected: true, .. }));
        let mut viewport: Option<SessionViewport> = None;
        let mut selected_row = false;
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
                SidebarEntry::SshConnection {
                    label,
                    host,
                    selected,
                } => {
                    let is_selected = *selected && !selected_row;
                    if is_selected {
                        selected_row = true;
                    }
                    SidebarRow {
                        kind: EntryKind::SshConnection,
                        label: label.clone(),
                        detail: Some(host.clone()),
                        selected: is_selected,
                    }
                }
                SidebarEntry::Agent { label } => SidebarRow {
                    kind: EntryKind::Agent,
                    label: label.clone(),
                    detail: None,
                    selected: false,
                },
                SidebarEntry::Session(descriptor) => {
                    let is_active = selected == Some(descriptor.id());
                    if is_active && !matches!(descriptor.status(), SessionStatus::Restored) {
                        viewport = Some(SessionViewport {
                            session: descriptor.clone(),
                        });
                    }
                    let is_selected = is_active && !pending_ssh && !selected_row;
                    if is_selected {
                        selected_row = true;
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

/// The text label for a session row: the descriptor's auto-generated title
/// (its stable display id, e.g. `"session-1"`).
fn session_label(descriptor: &SessionDescriptor) -> String {
    descriptor.title().to_string()
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

fn session_kind_text(kind: &SessionKind) -> &'static str {
    match kind {
        SessionKind::Local => "local",
        SessionKind::Project { .. } => "project",
        SessionKind::Worktree { .. } => "worktree",
        SessionKind::Ssh { .. } => "ssh",
        SessionKind::Agent { .. } => "agent",
    }
}

fn session_status_text(status: &SessionStatus) -> &'static str {
    match status {
        SessionStatus::Starting => "starting",
        SessionStatus::Restored => "restored (not running)",
        SessionStatus::Running => "running",
        SessionStatus::Exited { .. } => "exited",
        SessionStatus::Failed { .. } => "failed",
    }
}

/// Deterministic, process-free fixtures for the sidebar view.
///
/// Sessions are constructed through the shared `SessionRegistry`, which is
/// pure state and spawns nothing; SSH and agent entries are text facts only.
#[cfg(feature = "test-support")]
pub mod fixtures {
    use super::SidebarEntry;
    use crate::session::{SessionId, SessionKind, SessionRegistry, SessionStatus};

    /// A deterministic registry with three sessions and no selection: two
    /// observed-running local shells and one reserved SSH-shaped session
    /// still at its starting status because no observation was reported for
    /// it. Titles are auto-generated from the session id by the registry.
    #[must_use]
    pub fn session_registry() -> SessionRegistry {
        let mut registry = SessionRegistry::new();
        let first = registry.create(SessionKind::Local);
        let second = registry.create(SessionKind::Local);
        let _ = registry.create(SessionKind::Ssh {
            target: "web1.internal".to_string(),
        });
        observe_running(&mut registry, first);
        observe_running(&mut registry, second);
        debug_assert_eq!(registry.len(), 3);
        registry
    }

    fn observe_running(registry: &mut SessionRegistry, id: SessionId) {
        registry
            .observe(id, SessionStatus::Running)
            .expect("fixture observes ids the same fixture registry created");
    }

    /// One entry of each kind, in sidebar order: a project, a git worktree, an
    /// SSH-target fixture, a reserved agent, and every session in `registry`.
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
                selected: false,
            },
            SidebarEntry::Agent {
                label: "claude-code".to_string(),
            },
        ];
        entries.extend(registry.sessions().into_iter().map(SidebarEntry::Session));
        entries
    }
}
