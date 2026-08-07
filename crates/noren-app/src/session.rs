//! Session domain model: the shared contract for Noren's session lifecycle.
//!
//! This module defines the in-memory bookkeeping for terminal sessions without
//! owning any process. A [`SessionRegistry`] is a pure state machine: it records
//! session entries, tracks a single selected session, and reflects *observed*
//! status. It never spawns, waits on, or reads from a child.
//!
//! The model respects the Noren/Zellij boundary (ADR 0003): it carries no pane,
//! tab, layout, or split notion. A session is an opaque identity plus a launch
//! shape, nothing more.
//!
//! # Contract conformance
//!
//! The public types conform to decision **D-M3-001**, recorded in
//! `docs/coordination/session-api.md`. They are fixed here once; a lane that
//! needs a different shape escalates rather than forking. [`SessionId`],
//! [`SessionKind`], [`SessionStatus`], [`SessionDescriptor`], [`SessionAction`],
//! [`SessionEvent`], [`SelectedSession`], and [`SessionRegistry`] match that
//! contract. [`SessionError`] is a local addition (D-M3-001 defines no error
//! type) and [`SessionRegistry::observe`] is deliberately a registry *method* —
//! not a user-facing [`SessionAction`] — by which the supervisor reports facts.
//!
//! # Invariants
//!
//! The registry preserves five invariants that the domain test suite checks:
//!
//! 1. **At most one selected session.** [`SessionAction::Select`] replaces any
//!    prior selection, and closing the selected session clears it rather than
//!    leaving a dangling id.
//! 2. **No process ownership.** The registry holds only data; domain tests need
//!    no child processes.
//! 3. **Status is observed, not inferred.** [`SessionAction::Create`] records a
//!    [`SessionStatus::Starting`] entry; only [`SessionRegistry::observe`]
//!    advances the status, so a successful create never claims a session is
//!    running.
//! 4. **Bounded live state.** [`SessionAction::Close`] removes an entry, so
//!    repeated create/close cycles do not accumulate dead sessions. The registry
//!    retains no event history.
//! 5. **Monotonic lifecycle.** [`SessionRegistry::observe`] rejects a lower
//!    lifecycle rank. A running session cannot return to starting, and an exited
//!    or failed session cannot be resurrected as running.
//!
//! No persistence format is chosen: the model is in-memory only and derives no
//! `serde` traits.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

/// Stable, opaque identifier for a session, minted by [`SessionRegistry`].
///
/// IDs are registry-local and not persistence keys. Independent registries can
/// mint equal counter values, so callers must never mix their ids. The inner
/// value is private so callers receive ids only from [`SessionEvent::Created`]
/// or [`SessionDescriptor`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(u64);

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "session-{}", self.0)
    }
}

/// The launch shape of a session.
///
/// Conforms to D-M3-001: five variants. [`Local`] is the only kind with an
/// implemented launch path; [`Project`], [`Worktree`], [`Ssh`], and [`Agent`]
/// are carried as data so the enum is stable for exhaustive matching — no code
/// in this module launches any of them.
///
/// D-M3-001 records the concrete payloads used here: `root`/`path` for the
/// local-rooted kinds and `target`/`name` for the remote/agent kinds.
///
/// [`Local`]: SessionKind::Local
/// [`Project`]: SessionKind::Project
/// [`Worktree`]: SessionKind::Worktree
/// [`Ssh`]: SessionKind::Ssh
/// [`Agent`]: SessionKind::Agent
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SessionKind {
    /// A local PTY session backed by a process on this machine.
    #[default]
    Local,
    /// A session scoped to a project root directory.
    Project {
        /// Absolute root directory of the project.
        root: PathBuf,
    },
    /// A session scoped to a git worktree.
    Worktree {
        /// Path to the worktree.
        path: PathBuf,
    },
    /// A remote session over SSH. Reserved: not launched by this model.
    Ssh {
        /// SSH target (`user@host` or `host`).
        target: String,
    },
    /// An AI-agent-backed session. Reserved: not launched by this model.
    Agent {
        /// Identifier or name of the backing agent.
        name: String,
    },
}

impl SessionKind {
    /// Whether this kind has an implemented launch path.
    ///
    /// Only [`Local`](SessionKind::Local) does today. The spawn layer gates on
    /// this rather than guessing; the other kinds are carried as reserved data.
    #[must_use]
    pub const fn is_launchable(&self) -> bool {
        matches!(self, Self::Local)
    }
}

/// Observed runtime status of a session.
///
/// Conforms to D-M3-001. The registry never infers a status from a successful
/// [`SessionAction::Create`]: a fresh entry is [`Starting`], and every other
/// status is set only by an explicit [`SessionRegistry::observe`] report.
///
/// [`Starting`]: SessionStatus::Starting
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SessionStatus {
    /// The entry exists but no runtime observation has been reported.
    #[default]
    Starting,
    /// Observed running.
    Running,
    /// Observed to have exited, carrying the exit code when known.
    Exited {
        /// The process exit code, or `None` when it could not be determined.
        code: Option<i32>,
    },
    /// Observed to have failed, carrying a short reason.
    Failed {
        /// A short, human-readable failure reason.
        reason: String,
    },
}

impl SessionStatus {
    /// Lifecycle rank used to reject backwards observations.
    ///
    /// `Starting` (0) < `Running` (1) < terminal `Exited`/`Failed` (2).
    /// Equal-rank terminal observations may refine their payload or variant,
    /// but a terminal session can never return to a live status.
    #[must_use]
    pub const fn rank(&self) -> u8 {
        match self {
            Self::Starting => 0,
            Self::Running => 1,
            Self::Exited { .. } | Self::Failed { .. } => 2,
        }
    }
}

/// A snapshot of a session's stable facts.
///
/// Conforms to D-M3-001. Returned by [`SessionRegistry::get`]. The status field
/// reflects the last reported observation, never an inference from creation.
/// The `title` is generated by the registry at create time (since the contract
/// [`SessionAction::Create`] carries no title); it defaults to the session's
/// stable display id (for example `"session-1"`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionDescriptor {
    id: SessionId,
    kind: SessionKind,
    status: SessionStatus,
    title: String,
}

impl SessionDescriptor {
    /// The session identifier.
    #[must_use]
    pub const fn id(&self) -> SessionId {
        self.id
    }

    /// The launch shape.
    #[must_use]
    pub fn kind(&self) -> &SessionKind {
        &self.kind
    }

    /// The last observed status.
    #[must_use]
    pub fn status(&self) -> &SessionStatus {
        &self.status
    }

    /// The human-facing title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }
}

/// A command the registry reduces into zero or more [`SessionEvent`]s.
///
/// Conforms to D-M3-001: exactly three actions. The registry owns no process,
/// so [`Create`](SessionAction::Create) only records an entry; a separate spawn
/// layer reports observed status back through [`SessionRegistry::observe`] to
/// advance a session past [`SessionStatus::Starting`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionAction {
    /// Record a new session entry.
    Create {
        /// Launch shape of the new session.
        kind: SessionKind,
    },
    /// Make `id` the single selected session.
    Select {
        /// An existing live session to select.
        id: SessionId,
    },
    /// Remove a session entry, clearing the selection if it was selected.
    Close {
        /// The session to remove.
        id: SessionId,
    },
}

/// The result of reducing a [`SessionAction`] or an observation.
///
/// Conforms to D-M3-001: `Created`, `Selected`, and `Closed` are tuple
/// variants; `StatusChanged` carries the id and new status as named fields.
/// Events describe what actually changed; a no-op (selecting the
/// already-selected session, observing the current status) emits nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    /// A new session entry was recorded.
    Created(SessionId),
    /// The selection changed: set, replaced, or cleared.
    Selected(Option<SessionId>),
    /// A session's observed status changed.
    StatusChanged {
        /// The session whose status changed.
        id: SessionId,
        /// The newly observed status.
        status: SessionStatus,
    },
    /// A session entry was removed.
    Closed(SessionId),
}

/// The currently selected session, or `None`.
///
/// Conforms to D-M3-001: a type alias over [`SessionId`]. Returned by
/// [`SessionRegistry::selected`]; it never dangles because closing the selected
/// session clears the selection.
pub type SelectedSession = Option<SessionId>;

/// Typed failure of a [`SessionAction`] (or observation) against the registry.
///
/// D-M3-001 defines no error type; this is a local addition so callers handle
/// unknown sessions and invalid lifecycle transitions without panicking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// The action named a session the registry does not know.
    ///
    /// A closed id is unknown because [`SessionAction::Close`] removes the
    /// entry entirely rather than retaining a tombstone.
    UnknownSession,
    /// The observation would move a session backwards or resurrect it.
    InvalidStatusTransition,
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSession => f.write_str("unknown session"),
            Self::InvalidStatusTransition => f.write_str("invalid status transition"),
        }
    }
}

impl std::error::Error for SessionError {}

/// Pure, in-memory bookkeeping for terminal sessions.
///
/// The registry reduces the three contract [`SessionAction`]s into
/// [`SessionEvent`]s while preserving the module invariants. It owns no child
/// process and keeps no event history, so repeated create/close cycles cannot
/// grow live state.
///
/// Created entries start at [`SessionStatus::Starting`]; a status advances only
/// when the spawn layer reports an observation through
/// [`observe`](Self::observe). That method is a registry operation, not a
/// contract [`SessionAction`]. D-M3-001 keeps supervisor observations separate
/// from user requests such as create, select, and close.
pub struct SessionRegistry {
    sessions: HashMap<SessionId, SessionDescriptor>,
    selected: Option<SessionId>,
    next_id: u64,
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionRegistry {
    /// Create an empty registry with no sessions and no selection.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            selected: None,
            next_id: 1,
        }
    }

    /// Reduce one contract action into the events it produced.
    ///
    /// A no-op action yields an empty vector; an action against an unknown
    /// session yields [`SessionError::UnknownSession`]. Observed status is not
    /// an action — feed it through [`Self::observe`].
    pub fn apply(&mut self, action: SessionAction) -> Result<Vec<SessionEvent>, SessionError> {
        match action {
            SessionAction::Create { kind } => {
                let id = self.create(kind);
                Ok(vec![SessionEvent::Created(id)])
            }
            SessionAction::Select { id } => self.select_events(id),
            SessionAction::Close { id } => self.close_events(id),
        }
    }

    /// Record a new session entry and return its identifier.
    ///
    /// Creation is infallible: the registry mints a fresh id and accepts every
    /// [`SessionKind`], including the reserved shapes. The new entry starts at
    /// [`SessionStatus::Starting`] with a generated title (its stable display
    /// id); it becomes [`Running`](SessionStatus::Running) only through
    /// [`Self::observe`].
    #[must_use]
    pub fn create(&mut self, kind: SessionKind) -> SessionId {
        let id = SessionId(self.next_id);
        // A u64 id space cannot realistically exhaust in an in-memory registry;
        // if it ever did, continuing would corrupt uniqueness, so panicking is
        // the only correct option.
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("session id space exhausted");
        let descriptor = SessionDescriptor {
            id,
            kind,
            status: SessionStatus::Starting,
            title: id.to_string(),
        };
        self.sessions.insert(id, descriptor);
        id
    }

    /// Remove a session, clearing the selection if it was selected.
    ///
    /// Returns [`SessionError::UnknownSession`] when `id` is not live.
    pub fn close(&mut self, id: SessionId) -> Result<(), SessionError> {
        self.close_events(id).map(drop)
    }

    /// Make `id` the single selected session, replacing any prior selection.
    ///
    /// Returns [`SessionError::UnknownSession`] when `id` is not live. Selecting
    /// the already-selected session is a no-op.
    pub fn select(&mut self, id: SessionId) -> Result<(), SessionError> {
        self.select_events(id).map(drop)
    }

    /// Report an observed status for `id`.
    ///
    /// This is the only path that advances a session past
    /// [`SessionStatus::Starting`]; creation never infers a running status.
    /// Observing the current status is a no-op and returns `Ok(None)`. Returns
    /// [`SessionError::UnknownSession`] when `id` is not live.
    ///
    /// Lifecycle transitions are monotonic. A lower-ranked report, including
    /// `Running -> Starting` or `Exited/Failed -> Running`, returns
    /// [`SessionError::InvalidStatusTransition`] without mutating the entry.
    /// Equal-rank terminal reports may refine the terminal detail.
    pub fn observe(
        &mut self,
        id: SessionId,
        status: SessionStatus,
    ) -> Result<Option<SessionEvent>, SessionError> {
        let descriptor = self
            .sessions
            .get_mut(&id)
            .ok_or(SessionError::UnknownSession)?;
        if descriptor.status == status {
            return Ok(None);
        }
        if status.rank() < descriptor.status.rank() {
            return Err(SessionError::InvalidStatusTransition);
        }
        descriptor.status = status;
        Ok(Some(SessionEvent::StatusChanged {
            id,
            status: descriptor.status.clone(),
        }))
    }

    /// The descriptor for `id`, if it is live.
    #[must_use]
    pub fn get(&self, id: SessionId) -> Option<SessionDescriptor> {
        self.sessions.get(&id).cloned()
    }

    /// All live sessions, ordered by identifier for determinism.
    #[must_use]
    pub fn sessions(&self) -> Vec<SessionDescriptor> {
        let mut all: Vec<SessionDescriptor> = self.sessions.values().cloned().collect();
        all.sort_by_key(SessionDescriptor::id);
        all
    }

    /// The currently selected session, if any.
    ///
    /// Never dangles: closing the selected session clears the selection, so a
    /// returned [`Some`] id always resolves to a live entry.
    #[must_use]
    pub fn selected(&self) -> SelectedSession {
        self.selected
    }

    /// Number of live sessions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Whether there are no live sessions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    fn select_events(&mut self, id: SessionId) -> Result<Vec<SessionEvent>, SessionError> {
        if !self.sessions.contains_key(&id) {
            return Err(SessionError::UnknownSession);
        }
        if self.selected == Some(id) {
            return Ok(Vec::new());
        }
        self.selected = Some(id);
        Ok(vec![SessionEvent::Selected(Some(id))])
    }

    fn close_events(&mut self, id: SessionId) -> Result<Vec<SessionEvent>, SessionError> {
        if self.sessions.remove(&id).is_none() {
            return Err(SessionError::UnknownSession);
        }
        let mut events = vec![SessionEvent::Closed(id)];
        if self.selected == Some(id) {
            self.selected = None;
            events.push(SessionEvent::Selected(None));
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    //! Module-level sanity checks for the domain model. The exhaustive
    //! invariant suite lives in the workspace integration test
    //! `tests/session_domain.rs`; these unit checks cover the pure accessors
    //! and display forms that are awkward to reach from outside the crate.

    use super::*;

    #[test]
    fn session_id_displays_with_a_stable_prefix() {
        let id = SessionId(7);
        assert_eq!(id.to_string(), "session-7");
    }

    #[test]
    fn only_local_kind_is_launchable() {
        assert!(SessionKind::Local.is_launchable());
        assert!(
            !SessionKind::Project {
                root: PathBuf::from("/p")
            }
            .is_launchable()
        );
        assert!(
            !SessionKind::Worktree {
                path: PathBuf::from("/w")
            }
            .is_launchable()
        );
        assert!(
            !SessionKind::Ssh {
                target: "h".to_owned()
            }
            .is_launchable()
        );
        assert!(
            !SessionKind::Agent {
                name: "a".to_owned()
            }
            .is_launchable()
        );
    }

    #[test]
    fn kinds_and_statuses_have_natural_defaults() {
        assert_eq!(SessionKind::default(), SessionKind::Local);
        assert_eq!(SessionStatus::default(), SessionStatus::Starting);
    }

    #[test]
    fn session_error_renders_a_message() {
        assert_eq!(SessionError::UnknownSession.to_string(), "unknown session");
        assert_eq!(
            SessionError::InvalidStatusTransition.to_string(),
            "invalid status transition"
        );
    }

    #[test]
    fn registry_starts_empty_with_no_selection() {
        let registry = SessionRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert!(registry.sessions().is_empty());
        assert_eq!(registry.selected(), None);
        assert_eq!(registry.selected(), SessionRegistry::default().selected());
    }
}
