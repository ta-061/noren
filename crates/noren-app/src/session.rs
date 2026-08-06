//! Session domain model: the shared contract for Noren's session lifecycle.
//!
//! This module defines the in-memory bookkeeping for terminal sessions without
//! owning any process. A [`SessionRegistry`] is a pure state machine: it records
//! session entries, tracks a single selected session, and reflects *observed*
//! status. It never spawns, waits on, or reads from a child; the spawn layer
//! lives in another lane and reports back through [`SessionAction::Observe`].
//!
//! The model respects the Noren/Zellij boundary (ADR 0003): it carries no pane,
//! tab, layout, or split notion. A session is an opaque identity plus a launch
//! shape, nothing more.
//!
//! # Invariants
//!
//! The registry preserves four invariants that the domain test suite checks:
//!
//! 1. **At most one selected session.** [`SessionAction::Select`] replaces any
//!    prior selection, and closing the selected session clears it rather than
//!    leaving a dangling id.
//! 2. **No process ownership.** The registry holds only data; domain tests need
//!    no child processes.
//! 3. **Status is observed, not inferred.** [`SessionAction::Create`] records a
//!    [`SessionStatus::Created`] entry; only [`SessionAction::Observe`] advances
//!    the status, so a successful create never claims a session is running.
//! 4. **Bounded live state.** [`SessionAction::Close`] removes an entry, so
//!    repeated create/close cycles do not accumulate dead sessions. The registry
//!    retains no event history.
//!
//! [`SessionKind::Ssh`] and [`SessionKind::Agent`] exist as reserved shapes so
//! other lanes can pattern-match exhaustively today; their launch path is not
//! implemented here. No persistence format is chosen: the model is in-memory
//! only and derives no `serde` traits.

use std::collections::HashMap;
use std::fmt;

/// Stable, opaque identifier for a session, minted by [`SessionRegistry`].
///
/// Two ids compare equal only when they were minted from the same registry
/// counter value. The inner value is private so callers cannot fabricate ids;
/// they receive them from [`SessionEvent::Created`] or [`SessionDescriptor`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(u64);

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "session-{}", self.0)
    }
}

/// The launch shape of a session.
///
/// [`SessionKind::Local`] is the only kind with an implemented launch path.
/// [`SessionKind::Ssh`] and [`SessionKind::Agent`] are reserved so the enum is
/// stable for exhaustive matching; no code in this module launches them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SessionKind {
    /// A local PTY session backed by a process on this machine.
    #[default]
    Local,
    /// A remote session over SSH. Reserved: not launched by this model.
    Ssh,
    /// An AI-agent-backed session. Reserved: not launched by this model.
    Agent,
}

impl SessionKind {
    /// Whether this kind has an implemented launch path.
    ///
    /// Only [`Local`] does; [`Ssh`](SessionKind::Ssh) and
    /// [`Agent`](SessionKind::Agent) are reserved shapes, so the future spawn
    /// layer gates on this rather than guessing.
    #[must_use]
    pub const fn is_launchable(self) -> bool {
        matches!(self, Self::Local)
    }
}

/// Observed runtime status of a session.
///
/// The registry never infers a status from a successful
/// [`SessionAction::Create`]: a fresh entry is [`SessionStatus::Created`], and
/// every other status is set only by an explicit [`SessionAction::Observe`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SessionStatus {
    /// The entry exists but no runtime observation has been reported.
    #[default]
    Created,
    /// Observed running.
    Running,
    /// Observed to have failed (non-zero exit or launch error).
    Failed,
    /// Observed to have exited cleanly.
    Exited,
}

/// A snapshot of a session's stable facts.
///
/// Returned by [`SessionRegistry::get`] and carried on [`SessionEvent::Created`].
/// The status field reflects the last reported observation, never an inference
/// from creation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionDescriptor {
    id: SessionId,
    kind: SessionKind,
    status: SessionStatus,
    label: Option<String>,
}

impl SessionDescriptor {
    /// The session identifier.
    #[must_use]
    pub const fn id(&self) -> SessionId {
        self.id
    }

    /// The launch shape.
    #[must_use]
    pub const fn kind(&self) -> SessionKind {
        self.kind
    }

    /// The last observed status.
    #[must_use]
    pub const fn status(&self) -> SessionStatus {
        self.status
    }

    /// The optional human-facing label, if any.
    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }
}

/// A command the registry reduces into zero or more [`SessionEvent`]s.
///
/// The registry owns no process, so [`SessionAction::Create`] only records an
/// entry; a separate spawn layer reports back through [`SessionAction::Observe`]
/// to advance status beyond [`SessionStatus::Created`].
#[derive(Clone, Debug)]
pub enum SessionAction {
    /// Record a new session entry.
    Create {
        /// Launch shape of the new session.
        kind: SessionKind,
        /// Optional human-facing label.
        label: Option<String>,
    },
    /// Remove a session entry, clearing the selection if it was selected.
    Close {
        /// The session to remove.
        id: SessionId,
    },
    /// Make `id` the single selected session.
    Select {
        /// An existing live session to select.
        id: SessionId,
    },
    /// Report an observed status for `id`.
    Observe {
        /// The session being reported on.
        id: SessionId,
        /// The newly observed status.
        status: SessionStatus,
    },
}

/// The result of reducing a [`SessionAction`].
///
/// Events describe what actually changed; a no-op action (selecting the already
/// selected session, observing the current status) emits nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    /// A new session entry was recorded.
    Created {
        /// The new identifier.
        id: SessionId,
        /// The new descriptor.
        descriptor: SessionDescriptor,
    },
    /// A session entry was removed.
    Closed {
        /// The removed identifier.
        id: SessionId,
    },
    /// A session's observed status changed.
    StatusChanged {
        /// The session that changed.
        id: SessionId,
        /// The new status.
        status: SessionStatus,
    },
    /// The selection changed (set, replaced, or cleared).
    SelectionChanged {
        /// The now-selected session, or `None` when cleared.
        selected: Option<SessionId>,
    },
}

/// A view of the currently selected session.
///
/// Returned by [`SessionRegistry::selected`]; always consistent with live state
/// because closing the selected session clears the selection, so the id and
/// descriptor never dangle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedSession {
    id: SessionId,
    descriptor: SessionDescriptor,
}

impl SelectedSession {
    /// The selected session identifier.
    #[must_use]
    pub const fn id(&self) -> SessionId {
        self.id
    }

    /// The selected session descriptor.
    #[must_use]
    pub fn descriptor(&self) -> &SessionDescriptor {
        &self.descriptor
    }
}

/// Typed failure of a [`SessionAction`] against the registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// The action named a session the registry does not know.
    ///
    /// A closed id is unknown because [`SessionAction::Close`] removes the
    /// entry entirely rather than retaining a tombstone.
    UnknownSession,
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSession => f.write_str("unknown session"),
        }
    }
}

impl std::error::Error for SessionError {}

/// Pure, in-memory bookkeeping for terminal sessions.
///
/// The registry reduces [`SessionAction`]s into [`SessionEvent`]s while
/// preserving the module invariants. It owns no child process and keeps no
/// event history, so repeated create/close cycles cannot grow live state.
///
/// Created entries start at [`SessionStatus::Created`]; a status advances only
/// when the spawn layer reports an observation through
/// [`SessionRegistry::observe`].
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

    /// Reduce one action into the events it produced.
    ///
    /// A no-op action yields an empty vector; an action against an unknown
    /// session yields [`SessionError::UnknownSession`].
    pub fn apply(&mut self, action: SessionAction) -> Result<Vec<SessionEvent>, SessionError> {
        match action {
            SessionAction::Create { kind, label } => {
                let (id, descriptor) = self.create_entry(kind, label);
                Ok(vec![SessionEvent::Created { id, descriptor }])
            }
            SessionAction::Close { id } => self.close_entry(id),
            SessionAction::Select { id } => self.select_entry(id),
            SessionAction::Observe { id, status } => self.observe_entry(id, status),
        }
    }

    /// Record a new session entry and return its identifier.
    ///
    /// Creation is infallible: the registry mints a fresh id and accepts every
    /// [`SessionKind`], including the reserved [`Ssh`](SessionKind::Ssh) and
    /// [`Agent`](SessionKind::Agent) shapes. The new entry starts at
    /// [`SessionStatus::Created`]; it becomes [`Running`](SessionStatus::Running)
    /// only through [`Self::observe`].
    #[must_use]
    pub fn create(&mut self, kind: SessionKind, label: Option<String>) -> SessionId {
        self.create_entry(kind, label).0
    }

    /// Remove a session, clearing the selection if it was selected.
    ///
    /// Returns [`SessionError::UnknownSession`] when `id` is not live.
    pub fn close(&mut self, id: SessionId) -> Result<(), SessionError> {
        self.close_entry(id).map(drop)
    }

    /// Make `id` the single selected session, replacing any prior selection.
    ///
    /// Returns [`SessionError::UnknownSession`] when `id` is not live. Selecting
    /// the already-selected session is a no-op.
    pub fn select(&mut self, id: SessionId) -> Result<(), SessionError> {
        self.select_entry(id).map(drop)
    }

    /// Report an observed status for `id`.
    ///
    /// This is the only path that advances a session past
    /// [`SessionStatus::Created`]; creation never infers a running status.
    /// Observing the current status is a no-op. Returns
    /// [`SessionError::UnknownSession`] when `id` is not live.
    pub fn observe(&mut self, id: SessionId, status: SessionStatus) -> Result<(), SessionError> {
        self.observe_entry(id, status).map(drop)
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

    /// A view of the selected session, if any.
    ///
    /// Never dangles: closing the selected session clears the selection, so the
    /// returned id always resolves to a live entry.
    #[must_use]
    pub fn selected(&self) -> Option<SelectedSession> {
        self.selected.and_then(|id| {
            self.sessions.get(&id).map(|descriptor| SelectedSession {
                id,
                descriptor: descriptor.clone(),
            })
        })
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

    /// Mint and insert a new entry, returning its id and descriptor.
    fn create_entry(
        &mut self,
        kind: SessionKind,
        label: Option<String>,
    ) -> (SessionId, SessionDescriptor) {
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
            status: SessionStatus::Created,
            label,
        };
        self.sessions.insert(id, descriptor.clone());
        (id, descriptor)
    }

    fn close_entry(&mut self, id: SessionId) -> Result<Vec<SessionEvent>, SessionError> {
        if self.sessions.remove(&id).is_none() {
            return Err(SessionError::UnknownSession);
        }
        let mut events = vec![SessionEvent::Closed { id }];
        if self.selected == Some(id) {
            self.selected = None;
            events.push(SessionEvent::SelectionChanged { selected: None });
        }
        Ok(events)
    }

    fn select_entry(&mut self, id: SessionId) -> Result<Vec<SessionEvent>, SessionError> {
        if !self.sessions.contains_key(&id) {
            return Err(SessionError::UnknownSession);
        }
        if self.selected == Some(id) {
            return Ok(Vec::new());
        }
        self.selected = Some(id);
        Ok(vec![SessionEvent::SelectionChanged { selected: Some(id) }])
    }

    fn observe_entry(
        &mut self,
        id: SessionId,
        status: SessionStatus,
    ) -> Result<Vec<SessionEvent>, SessionError> {
        let descriptor = self
            .sessions
            .get_mut(&id)
            .ok_or(SessionError::UnknownSession)?;
        if descriptor.status == status {
            return Ok(Vec::new());
        }
        descriptor.status = status;
        Ok(vec![SessionEvent::StatusChanged { id, status }])
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
        assert!(!SessionKind::Ssh.is_launchable());
        assert!(!SessionKind::Agent.is_launchable());
    }

    #[test]
    fn kinds_and_statuses_have_natural_defaults() {
        assert_eq!(SessionKind::default(), SessionKind::Local);
        assert_eq!(SessionStatus::default(), SessionStatus::Created);
    }

    #[test]
    fn session_error_renders_a_message() {
        assert_eq!(SessionError::UnknownSession.to_string(), "unknown session");
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
