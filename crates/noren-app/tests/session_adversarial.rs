//! Adversarial session-lifecycle tests — lane `kimi-a` (engine GLM 5.2).
//!
//! # What this attacks
//!
//! The session lifecycle: the pure domain registry (the D-M3-001 "shared
//! session API contract", lane `glm-a`, `crates/noren-app/src/session.rs`) and
//! the lifecycle supervisor (lane `glm-b`,
//! `crates/noren-app/src/session_supervisor.rs`). The attack surface, per the
//! lane brief: mass create/close cycles, rapid selection switching, child
//! crash, a stale selected session, duplicate ids, invalid actions, shutdown
//! races, resource cleanup, unbounded list growth, a malformed future
//! persistence fixture, and repeated attach/detach simulation. Boundedness is
//! checked explicitly — a prior sweep found a cell growing without limit while
//! every documented bound still held, and the brief warns the same class is
//! possible here with a session list.
//!
//! # Why a local fake (read this before trusting a finding)
//!
//! At branch point `origin/main` = `1d329a5` the domain module, the supervisor
//! module, **and** decision D-M3-001 are all absent from `main` — they live on
//! the unmerged branches `agent/m3-session-domain` and `agent/m3-session-
//! supervisor`. The lane brief says: if the domain module is not on `main`
//! yet, build against a local fake that matches D-M3-001's shape and say so —
//! do not block. That is what the two `fake_*` modules below are: faithful,
//! behaviour-for-behaviour mirrors of the published branch code (types, public
//! API, and the reduction/reaping algorithms copied line-for-line where the
//! algorithm is load-bearing), compiled standalone as a test target.
//!
//! Consequences a reviewer must weigh:
//!
//! - I did **not** author the code under review (the branch modules); I did
//!   author the fakes. A failing reproducer is valid to the extent the fake
//!   mirrors the branch, which I copied carefully but could not compile-link
//!   against because neither module is wired into `lib.rs` on `main` yet.
//! - A test that probes an *algorithmic* property (boundedness, status
//!   monotonicity, unknown-id handling) reflects the shared algorithm, so a
//!   failure reproduces in the real code regardless of who typed the fake.
//! - Tests marked `#[ignore = "reproduces <id>"]` are reported defects: they
//!   **fail** when run with `--ignored` and document the behaviour. Everything
//!   else passes against the fake and confirms the design holds under that
//!   attack (conditional on the real code matching the mirror).
//!
//! The Noren/Zellij boundary (ADR 0003) is respected: no pane, tab, layout, or
//! split notion appears anywhere.
//!
//! # Status after the fixes (lane `glm-advfix`)
//!
//! The three reported defects — ADV-S1, ADV-S2, ADV-S3 — are fixed in the merged
//! modules (`src/session.rs`, `src/session_supervisor.rs`). The original
//! `#[ignore]` reproducers mirrored the buggy algorithm through the fakes; the
//! fix lane replaces them with normal `#[test]` regression guards that compile
//! and run against the **real merged modules** (included below via `#[path]`),
//! not the fakes. The fakes and their 20 defensive tests remain as the original
//! attack record; the three regression guards live in the final section.

// The two `fake_*` modules intentionally mirror the *full* published API
// surface of the branch modules line-for-line, including methods and enum
// variants that no test here exercises. The dead-code lint would otherwise
// flag those faithful copies, so it is silenced at the file level. Test
// functions themselves are never "dead" (they are test entry points), so this
// cannot mask a missing `#[test]`.
#![allow(dead_code)]

// ─────────────────────────────────────────────────────────────────────────────
// The real merged modules. The three regression guards at the end of this file
// compile against these (not the fakes), so they guard the actual fixed code.
// `cfg(test)` is enabled for an integration binary, which also brings the
// supervisor's `mock` harness and each module's own unit tests into scope.
// ─────────────────────────────────────────────────────────────────────────────

#[path = "../src/session.rs"]
mod session;

#[path = "../src/session_supervisor.rs"]
mod session_supervisor;

// ─────────────────────────────────────────────────────────────────────────────
// fake_domain — mirror of `session.rs` (lane glm-a / D-M3-001 shape).
// ─────────────────────────────────────────────────────────────────────────────

mod fake_domain {
    use std::collections::HashMap;
    use std::fmt;

    /// Opaque session identifier, minted only by [`SessionRegistry`].
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct SessionId(u64);

    impl fmt::Display for SessionId {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "session-{}", self.0)
        }
    }

    /// Launch shape of a session. Only `Local` is launchable; the rest are
    /// reserved for exhaustive matching.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub enum SessionKind {
        #[default]
        Local,
        Ssh,
        Agent,
    }

    impl SessionKind {
        #[must_use]
        pub const fn is_launchable(self) -> bool {
            matches!(self, Self::Local)
        }
    }

    /// Observed runtime status. `Created` is the initial state; every other
    /// value is set only by an explicit [`SessionAction::Observe`].
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub enum SessionStatus {
        #[default]
        Created,
        Running,
        Failed,
        Exited,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct SessionDescriptor {
        id: SessionId,
        kind: SessionKind,
        status: SessionStatus,
        label: Option<String>,
    }

    impl SessionDescriptor {
        #[must_use]
        pub const fn id(&self) -> SessionId {
            self.id
        }

        #[must_use]
        pub const fn kind(&self) -> SessionKind {
            self.kind
        }

        #[must_use]
        pub const fn status(&self) -> SessionStatus {
            self.status
        }

        #[must_use]
        pub fn label(&self) -> Option<&str> {
            self.label.as_deref()
        }
    }

    /// A command the registry reduces into zero or more events.
    #[derive(Clone, Debug)]
    pub enum SessionAction {
        Create {
            kind: SessionKind,
            label: Option<String>,
        },
        Close {
            id: SessionId,
        },
        Select {
            id: SessionId,
        },
        Observe {
            id: SessionId,
            status: SessionStatus,
        },
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum SessionEvent {
        Created {
            id: SessionId,
            descriptor: SessionDescriptor,
        },
        Closed {
            id: SessionId,
        },
        StatusChanged {
            id: SessionId,
            status: SessionStatus,
        },
        SelectionChanged {
            selected: Option<SessionId>,
        },
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct SelectedSession {
        id: SessionId,
        descriptor: SessionDescriptor,
    }

    impl SelectedSession {
        #[must_use]
        pub const fn id(&self) -> SessionId {
            self.id
        }

        #[must_use]
        pub fn descriptor(&self) -> &SessionDescriptor {
            &self.descriptor
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum SessionError {
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

    /// Pure, in-memory bookkeeping for terminal sessions. Mirrors the branch
    /// `SessionRegistry` exactly, including the invariant that only `Close`
    /// clears a selection and that `Observe` may set any status.
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
        #[must_use]
        pub fn new() -> Self {
            Self {
                sessions: HashMap::new(),
                selected: None,
                next_id: 1,
            }
        }

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

        #[must_use]
        pub fn create(&mut self, kind: SessionKind, label: Option<String>) -> SessionId {
            self.create_entry(kind, label).0
        }

        pub fn close(&mut self, id: SessionId) -> Result<(), SessionError> {
            self.close_entry(id).map(drop)
        }

        pub fn select(&mut self, id: SessionId) -> Result<(), SessionError> {
            self.select_entry(id).map(drop)
        }

        pub fn observe(
            &mut self,
            id: SessionId,
            status: SessionStatus,
        ) -> Result<(), SessionError> {
            self.observe_entry(id, status).map(drop)
        }

        #[must_use]
        pub fn get(&self, id: SessionId) -> Option<SessionDescriptor> {
            self.sessions.get(&id).cloned()
        }

        #[must_use]
        pub fn sessions(&self) -> Vec<SessionDescriptor> {
            let mut all: Vec<SessionDescriptor> = self.sessions.values().cloned().collect();
            all.sort_by_key(SessionDescriptor::id);
            all
        }

        #[must_use]
        pub fn selected(&self) -> Option<SelectedSession> {
            self.selected.and_then(|id| {
                self.sessions.get(&id).map(|descriptor| SelectedSession {
                    id,
                    descriptor: descriptor.clone(),
                })
            })
        }

        #[must_use]
        pub fn len(&self) -> usize {
            self.sessions.len()
        }

        #[must_use]
        pub fn is_empty(&self) -> bool {
            self.sessions.is_empty()
        }

        fn create_entry(
            &mut self,
            kind: SessionKind,
            label: Option<String>,
        ) -> (SessionId, SessionDescriptor) {
            let id = SessionId(self.next_id);
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

        // NOTE (load-bearing for ADV-S1): the only guard is equality with the
        // current status. There is no transition-direction check, so a status
        // can move backwards (Running -> Created, Exited -> Running).
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
}

// ─────────────────────────────────────────────────────────────────────────────
// fake_supervisor — mirror of `session_supervisor.rs` (lane glm-b).
// ─────────────────────────────────────────────────────────────────────────────

mod fake_supervisor {
    use std::collections::HashMap;
    use std::collections::VecDeque;
    use std::fmt;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    pub const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(2);

    /// Opaque session identifier (STUB per D-M3-001).
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct SessionId(u64);

    impl fmt::Display for SessionId {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "session#{}", self.0)
        }
    }

    /// Observed lifecycle status. `Running` is the only non-terminal variant;
    /// a dead child is always `Exited` or `Failed`, never a frozen `Running`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum SessionStatus {
        Running,
        Exited { code: Option<u32> },
        Failed { reason: SessionFailure },
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum SessionFailure {
        SpawnFailed,
        PollFailed,
        ShutdownFailed,
        ReapTimeout,
    }

    impl fmt::Display for SessionStatus {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Running => f.write_str("running"),
                Self::Exited { code: Some(code) } => write!(f, "exited(code={code})"),
                Self::Exited { code: None } => f.write_str("exited"),
                Self::Failed { reason } => write!(f, "failed({reason})"),
            }
        }
    }

    impl fmt::Display for SessionFailure {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::SpawnFailed => f.write_str("spawn-failed"),
                Self::PollFailed => f.write_str("poll-failed"),
                Self::ShutdownFailed => f.write_str("shutdown-failed"),
                Self::ReapTimeout => f.write_str("reap-timeout"),
            }
        }
    }

    fn is_terminal(status: SessionStatus) -> bool {
        !matches!(status, SessionStatus::Running)
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ChildError {
        Poll,
        Shutdown(ShutdownError),
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ShutdownError {
        Failed,
        TimedOut,
    }

    impl fmt::Display for ChildError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Poll => f.write_str("child poll failed"),
                Self::Shutdown(ShutdownError::Failed) => f.write_str("child shutdown failed"),
                Self::Shutdown(ShutdownError::TimedOut) => f.write_str("child shutdown timed out"),
            }
        }
    }

    impl fmt::Display for ShutdownError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Failed => f.write_str("failed"),
                Self::TimedOut => f.write_str("timed-out"),
            }
        }
    }

    impl std::error::Error for ChildError {}

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum PollOutcome {
        StillRunning,
        Exited { code: Option<u32> },
    }

    /// A supervised child process handle.
    pub trait Child: Send {
        fn poll_exit(&mut self) -> Result<PollOutcome, ChildError>;
        fn shutdown(&mut self, deadline: Instant) -> Result<(), ChildError>;
    }

    /// Factory that creates a [`Child`] for a new session.
    pub trait Spawner: Send {
        fn spawn(&mut self) -> Result<Box<dyn Child + Send>, SessionFailure>;
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum SessionOpError {
        Unknown,
        NotRunning,
        StillRunning,
    }

    impl fmt::Display for SessionOpError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Unknown => f.write_str("unknown session"),
                Self::NotRunning => f.write_str("session is not running"),
                Self::StillRunning => f.write_str("session is still running"),
            }
        }
    }

    impl std::error::Error for SessionOpError {}

    #[derive(Debug, Default)]
    pub struct ReapReport {
        exited: Vec<(SessionId, Option<u32>)>,
        failed: Vec<(SessionId, SessionFailure)>,
    }

    impl ReapReport {
        #[must_use]
        pub fn exited(&self) -> &[(SessionId, Option<u32>)] {
            &self.exited
        }

        #[must_use]
        pub fn failed(&self) -> &[(SessionId, SessionFailure)] {
            &self.failed
        }

        #[must_use]
        pub fn transitioned(&self) -> usize {
            self.exited.len() + self.failed.len()
        }
    }

    struct SupervisedSession {
        child: Option<Box<dyn Child + Send>>,
        status: SessionStatus,
    }

    /// Lifecycle supervisor owning every live child handle. Mirrors the branch
    /// `SessionSupervisor` exactly, including the `wrapping_add` id minting
    /// (ADV note) and the unknown-id arm of `terminate` (ADV-S3).
    pub struct SessionSupervisor {
        sessions: HashMap<SessionId, SupervisedSession>,
        order: Vec<SessionId>,
        selected: Option<SessionId>,
        spawner: Box<dyn Spawner>,
        next_id: u64,
    }

    impl fmt::Debug for SessionSupervisor {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("SessionSupervisor")
                .field("sessions", &self.sessions.len())
                .field("selected", &self.selected)
                .finish_non_exhaustive()
        }
    }

    impl SessionSupervisor {
        pub fn new(spawner: Box<dyn Spawner>) -> Self {
            Self {
                sessions: HashMap::new(),
                order: Vec::new(),
                selected: None,
                spawner,
                next_id: 0,
            }
        }

        pub fn spawn(&mut self) -> Result<SessionId, SessionFailure> {
            let child = self.spawner.spawn()?;
            let id = self.fresh_id();
            self.sessions.insert(
                id,
                SupervisedSession {
                    child: Some(child),
                    status: SessionStatus::Running,
                },
            );
            self.order.push(id);
            self.selected = Some(id);
            Ok(id)
        }

        #[must_use]
        pub fn status(&self, id: SessionId) -> Option<SessionStatus> {
            self.sessions.get(&id).map(|session| session.status)
        }

        #[must_use]
        pub fn len(&self) -> usize {
            self.sessions.len()
        }

        #[must_use]
        pub fn is_empty(&self) -> bool {
            self.sessions.is_empty()
        }

        #[must_use]
        pub fn running_count(&self) -> usize {
            self.sessions
                .values()
                .filter(|session| session.status == SessionStatus::Running)
                .count()
        }

        #[must_use]
        pub fn selected(&self) -> Option<SessionId> {
            self.selected
        }

        pub fn select(&mut self, id: SessionId) -> Result<(), SessionOpError> {
            let session = self.sessions.get(&id).ok_or(SessionOpError::Unknown)?;
            if session.status != SessionStatus::Running {
                return Err(SessionOpError::NotRunning);
            }
            self.selected = Some(id);
            Ok(())
        }

        pub fn clear_selection(&mut self) {
            self.selected = None;
        }

        pub fn poll(&mut self) -> ReapReport {
            let mut report = ReapReport::default();
            let running: Vec<SessionId> = self
                .sessions
                .iter()
                .filter(|(_, session)| session.status == SessionStatus::Running)
                .map(|(id, _)| *id)
                .collect();

            for id in running {
                let outcome = match self.sessions.get_mut(&id) {
                    Some(session) => match session.child.as_mut() {
                        Some(child) => child.poll_exit(),
                        None => Err(ChildError::Poll),
                    },
                    None => continue,
                };
                match outcome {
                    Ok(PollOutcome::StillRunning) => {}
                    Ok(PollOutcome::Exited { code }) => self.mark_exited(&mut report, id, code),
                    Err(ChildError::Poll) => {
                        self.mark_failed(&mut report, id, SessionFailure::PollFailed);
                    }
                    Err(ChildError::Shutdown(reason)) => {
                        self.mark_failed(&mut report, id, shutdown_to_failure(reason));
                    }
                }
            }
            report
        }

        pub fn terminate(&mut self, id: SessionId, deadline: Instant) -> SessionStatus {
            let already = self.sessions.get(&id).map(|session| session.status);
            match already {
                Some(status) if is_terminal(status) => return status,
                // NOTE (ADV-S3): an unknown id fabricates Failed(PollFailed),
                // a control-plane status for a session that was never polled.
                None => {
                    return SessionStatus::Failed {
                        reason: SessionFailure::PollFailed,
                    };
                }
                _ => {}
            }

            if Instant::now() >= deadline {
                return self.finalize_failed(id, SessionFailure::ReapTimeout);
            }

            let result = match self
                .sessions
                .get_mut(&id)
                .and_then(|session| session.child.as_mut())
            {
                Some(child) => child.shutdown(deadline),
                None => Err(ChildError::Poll),
            };

            match result {
                Ok(()) => {
                    let code = match self.poll_after_shutdown(id) {
                        Ok(PollOutcome::Exited { code }) => code,
                        Ok(PollOutcome::StillRunning) => None,
                        Err(_) => None,
                    };
                    self.finalize_exited(id, code)
                }
                Err(ChildError::Shutdown(reason)) => {
                    self.finalize_failed(id, shutdown_to_failure(reason))
                }
                Err(ChildError::Poll) => self.finalize_failed(id, SessionFailure::ShutdownFailed),
            }
        }

        fn poll_after_shutdown(&mut self, id: SessionId) -> Result<PollOutcome, ChildError> {
            match self.sessions.get_mut(&id) {
                Some(session) => match session.child.as_mut() {
                    Some(child) => child.poll_exit(),
                    None => Ok(PollOutcome::Exited { code: None }),
                },
                None => Ok(PollOutcome::Exited { code: None }),
            }
        }

        pub fn terminate_now(&mut self, id: SessionId) -> SessionStatus {
            let deadline = Instant::now() + SHUTDOWN_DEADLINE;
            self.terminate(id, deadline)
        }

        pub fn shutdown_all(&mut self) -> Vec<(SessionId, SessionStatus)> {
            let deadline = Instant::now() + SHUTDOWN_DEADLINE;
            let ids = self.order.clone();
            let mut results = Vec::with_capacity(ids.len());
            for id in ids {
                let status = self.terminate(id, deadline);
                results.push((id, status));
            }
            self.selected = None;
            results
        }

        pub fn forget(&mut self, id: SessionId) -> Result<(), SessionOpError> {
            let status = self
                .sessions
                .get(&id)
                .ok_or(SessionOpError::Unknown)?
                .status;
            if status == SessionStatus::Running {
                return Err(SessionOpError::StillRunning);
            }
            self.sessions.remove(&id);
            self.order.retain(|existing| *existing != id);
            if self.selected == Some(id) {
                self.selected = None;
            }
            Ok(())
        }

        // NOTE (ADV divergence vs domain): the domain mints ids with
        // `checked_add` and panics on the (unreachable) overflow; the
        // supervisor uses `wrapping_add`, which silently wraps and could
        // collide with a live id after 2^64 spawns. Not reachable through the
        // public API in any realistic run, so it has no reproducer below.
        fn fresh_id(&mut self) -> SessionId {
            let id = SessionId(self.next_id);
            self.next_id = self.next_id.wrapping_add(1);
            id
        }

        fn mark_exited(&mut self, report: &mut ReapReport, id: SessionId, code: Option<u32>) {
            if let Some(session) = self.sessions.get_mut(&id) {
                session.status = SessionStatus::Exited { code };
                session.child = None;
            }
            report.exited.push((id, code));
            if self.selected == Some(id) {
                self.selected = None;
            }
        }

        fn mark_failed(&mut self, report: &mut ReapReport, id: SessionId, reason: SessionFailure) {
            if let Some(session) = self.sessions.get_mut(&id) {
                session.status = SessionStatus::Failed { reason };
                session.child = None;
            }
            report.failed.push((id, reason));
            if self.selected == Some(id) {
                self.selected = None;
            }
        }

        fn finalize_exited(&mut self, id: SessionId, code: Option<u32>) -> SessionStatus {
            if let Some(session) = self.sessions.get_mut(&id) {
                session.status = SessionStatus::Exited { code };
                session.child = None;
            }
            if self.selected == Some(id) {
                self.selected = None;
            }
            SessionStatus::Exited { code }
        }

        fn finalize_failed(&mut self, id: SessionId, reason: SessionFailure) -> SessionStatus {
            if let Some(session) = self.sessions.get_mut(&id) {
                session.status = SessionStatus::Failed { reason };
                session.child = None;
            }
            if self.selected == Some(id) {
                self.selected = None;
            }
            SessionStatus::Failed { reason }
        }
    }

    fn shutdown_to_failure(reason: ShutdownError) -> SessionFailure {
        match reason {
            ShutdownError::TimedOut => SessionFailure::ReapTimeout,
            ShutdownError::Failed => SessionFailure::ShutdownFailed,
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Mock process model. Shared-state machine a test drives through a
    // controller: flip the child to exited, toggle poll/shutdown failures, and
    // record the deadlines passed to `shutdown`.
    // ─────────────────────────────────────────────────────────────────────────

    #[derive(Debug)]
    struct MockState {
        alive: bool,
        exit_code: Option<u32>,
        poll_error: bool,
        shutdown_error: Option<ShutdownError>,
        shutdown_calls: u32,
        poll_calls: u32,
        deadlines: Vec<Instant>,
    }

    impl Default for MockState {
        fn default() -> Self {
            Self {
                alive: true,
                exit_code: None,
                poll_error: false,
                shutdown_error: None,
                shutdown_calls: 0,
                poll_calls: 0,
                deadlines: Vec::new(),
            }
        }
    }

    pub struct MockChild {
        state: Arc<Mutex<MockState>>,
    }

    pub struct MockController {
        state: Arc<Mutex<MockState>>,
    }

    impl MockChild {
        /// A child that is already dead and reports `Exited` on the first poll.
        #[must_use]
        pub fn exited() -> Self {
            let state = Arc::new(Mutex::new(MockState {
                alive: false,
                ..MockState::default()
            }));
            Self { state }
        }

        /// A live child with no scripted failure.
        #[must_use]
        pub fn running() -> Self {
            Self::running_with_control().0
        }

        /// A live child plus the controller that scripts it.
        #[must_use]
        pub fn running_with_control() -> (Self, MockController) {
            let state = Arc::new(Mutex::new(MockState::default()));
            (
                Self {
                    state: Arc::clone(&state),
                },
                MockController { state },
            )
        }
    }

    impl Child for MockChild {
        fn poll_exit(&mut self) -> Result<PollOutcome, ChildError> {
            let mut state = self.state.lock().expect("mock lock");
            state.poll_calls += 1;
            if state.poll_error {
                return Err(ChildError::Poll);
            }
            if state.alive {
                Ok(PollOutcome::StillRunning)
            } else {
                Ok(PollOutcome::Exited {
                    code: state.exit_code,
                })
            }
        }

        fn shutdown(&mut self, deadline: Instant) -> Result<(), ChildError> {
            let mut state = self.state.lock().expect("mock lock");
            state.shutdown_calls += 1;
            state.deadlines.push(deadline);
            if let Some(error) = state.shutdown_error {
                return Err(ChildError::Shutdown(error));
            }
            state.alive = false;
            Ok(())
        }
    }

    impl MockController {
        /// Make the child exit on its own with `code` (`None` = signal-style).
        pub fn exit(&self, code: Option<u32>) {
            let mut state = self.state.lock().expect("mock lock");
            state.alive = false;
            state.exit_code = code;
        }

        /// Cause subsequent `poll_exit` calls to error.
        pub fn fail_poll(&self) {
            self.state.lock().expect("mock lock").poll_error = true;
        }

        /// Cause subsequent `shutdown` calls to error with `reason`.
        pub fn fail_shutdown(&self, reason: ShutdownError) {
            self.state.lock().expect("mock lock").shutdown_error = Some(reason);
        }

        #[must_use]
        pub fn shutdown_count(&self) -> u32 {
            self.state.lock().expect("mock lock").shutdown_calls
        }

        #[must_use]
        pub fn deadlines(&self) -> Vec<Instant> {
            self.state.lock().expect("mock lock").deadlines.clone()
        }
    }

    /// Spawner that dispenses pre-built children in order.
    pub struct MockSpawner {
        children: VecDeque<MockChild>,
        fail_next: bool,
    }

    impl MockSpawner {
        #[must_use]
        pub fn from_children(children: Vec<MockChild>) -> Self {
            Self {
                children: children.into(),
                fail_next: false,
            }
        }

        #[must_use]
        pub fn failing() -> Self {
            Self {
                children: VecDeque::new(),
                fail_next: true,
            }
        }
    }

    impl Spawner for MockSpawner {
        fn spawn(&mut self) -> Result<Box<dyn Child + Send>, SessionFailure> {
            if self.fail_next {
                return Err(SessionFailure::SpawnFailed);
            }
            self.children
                .pop_front()
                .map(|child| Box::new(child) as Box<dyn Child + Send>)
                .ok_or(SessionFailure::SpawnFailed)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Test scope. Short module aliases avoid the name clash (both fakes define
// `SessionId` / `SessionStatus` because the supervisor's are STUB stand-ins).
// ─────────────────────────────────────────────────────────────────────────────

use fake_domain as dom;
use fake_supervisor as sup;
use fake_supervisor::{MockChild, MockController, MockSpawner};
use fake_supervisor::{SessionFailure, SessionOpError, SessionStatus};

fn supervisor_with(children: Vec<MockChild>) -> sup::SessionSupervisor {
    sup::SessionSupervisor::new(Box::new(MockSpawner::from_children(children)))
}

/// True when the supervisor's selection is `None` or points at a `Running`
/// session — the "selection never dangles / never points at a corpse" property.
fn selection_is_valid(s: &sup::SessionSupervisor) -> bool {
    match s.selected() {
        None => true,
        Some(id) => s.status(id) == Some(SessionStatus::Running),
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Domain registry — defensive (passing) tests.
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn domain_mass_create_close_cycle_stays_bounded() {
    // The documented invariant #4: repeated create/close does not accumulate
    // dead sessions, because Close removes the entry entirely.
    let mut reg = dom::SessionRegistry::new();
    for _ in 0..2000 {
        let id = reg.create(dom::SessionKind::Local, None);
        assert!(reg.close(id).is_ok());
    }
    assert_eq!(reg.len(), 0);
    assert!(reg.is_empty());
    assert!(reg.sessions().is_empty());
}

#[test]
fn domain_rapid_selection_switching_keeps_single_selection() {
    let mut reg = dom::SessionRegistry::new();
    let a = reg.create(dom::SessionKind::Local, None);
    let b = reg.create(dom::SessionKind::Local, None);
    let c = reg.create(dom::SessionKind::Local, None);

    for (target, _) in [(a, 'a'), (b, 'b'), (c, 'c'), (a, 'a'), (c, 'c'), (b, 'b')] {
        reg.select(target).expect("select live");
        assert_eq!(reg.selected().map(|s| s.id()), Some(target));
    }
    // Exactly one session (the last selected) is reflected; selection is a
    // scalar, never a set.
    assert!(reg.selected().is_some());
}

#[test]
fn domain_actions_against_closed_id_are_unknown_session() {
    let mut reg = dom::SessionRegistry::new();
    let id = reg.create(dom::SessionKind::Local, None);
    reg.close(id).expect("close live");

    // Every later action naming a closed id is rejected — close is a full
    // removal, not a tombstone, so the id is fully unknown afterwards.
    assert_eq!(reg.close(id), Err(dom::SessionError::UnknownSession));
    assert_eq!(reg.select(id), Err(dom::SessionError::UnknownSession));
    assert_eq!(
        reg.observe(id, dom::SessionStatus::Running),
        Err(dom::SessionError::UnknownSession)
    );
    assert!(reg.get(id).is_none());
}

#[test]
fn domain_close_selected_clears_selection_no_dangle() {
    let mut reg = dom::SessionRegistry::new();
    let a = reg.create(dom::SessionKind::Local, None);
    let _b = reg.create(dom::SessionKind::Local, None);
    reg.select(a).expect("select");
    assert_eq!(reg.selected().map(|s| s.id()), Some(a));

    reg.close(a).expect("close selected");
    // Selection cleared, not left pointing at a removed id.
    assert_eq!(reg.selected(), None);
    // selected() must never return a descriptor whose id is absent.
    assert_eq!(reg.len(), 1);
}

#[test]
fn domain_close_selected_emits_closed_then_selection_cleared() {
    let mut reg = dom::SessionRegistry::new();
    let id = reg.create(dom::SessionKind::Local, None);
    let events = reg.apply(dom::SessionAction::Select { id }).unwrap();
    assert_eq!(
        events,
        vec![dom::SessionEvent::SelectionChanged { selected: Some(id) }]
    );
    let events = reg.apply(dom::SessionAction::Close { id }).unwrap();
    assert_eq!(
        events,
        vec![
            dom::SessionEvent::Closed { id },
            dom::SessionEvent::SelectionChanged { selected: None }
        ]
    );
}

#[test]
fn domain_create_mints_unique_monotonic_ids() {
    let mut reg = dom::SessionRegistry::new();
    let ids: Vec<_> = (0..50)
        .map(|_| reg.create(dom::SessionKind::Local, None))
        .collect();
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "duplicate id minted");
    assert_eq!(reg.len(), 50);
}

#[test]
fn domain_malformed_persistence_replay_rejects_dangling_refs() {
    // Simulate replaying a corrupted persisted action log: a record references
    // an id that was already closed (a dangling reference). The registry must
    // reject it rather than resurrect state. This is the "malformed future
    // persistence fixture" attack.
    let mut reg = dom::SessionRegistry::new();
    let id = reg.create(dom::SessionKind::Local, None);
    reg.close(id).expect("close");

    // A stale log entry tries to select/observe a closed id.
    assert_eq!(
        reg.apply(dom::SessionAction::Select { id }),
        Err(dom::SessionError::UnknownSession)
    );
    assert_eq!(
        reg.apply(dom::SessionAction::Observe {
            id,
            status: dom::SessionStatus::Running
        }),
        Err(dom::SessionError::UnknownSession)
    );
    // Two creates never collide on id (ids are internally minted, never
    // injected), so a duplicate-id fixture cannot be introduced via actions.
    let other = reg.create(dom::SessionKind::Local, None);
    assert_ne!(other, id);
}

#[test]
fn domain_observe_same_status_is_noop_and_advances_to_running() {
    let mut reg = dom::SessionRegistry::new();
    let id = reg.create(dom::SessionKind::Local, None);
    assert_eq!(reg.get(id).unwrap().status(), dom::SessionStatus::Created);

    // Observe to the current status is a no-op (no event).
    let none = reg
        .apply(dom::SessionAction::Observe {
            id,
            status: dom::SessionStatus::Created,
        })
        .unwrap();
    assert!(none.is_empty());

    reg.observe(id, dom::SessionStatus::Running)
        .expect("observe");
    assert_eq!(reg.get(id).unwrap().status(), dom::SessionStatus::Running);
}

// ═════════════════════════════════════════════════════════════════════════════
// Domain registry — reported defects.
//
// ADV-S1 is fixed in `src/session.rs`; its regression guard now compiles
// against the merged module and lives in the final section of this file. The
// divergence test below stays (it documents a real, pre-existing contract
// divergence between the domain and supervisor selection rules).
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn domain_selected_session_observed_terminal_remains_selected_documenting_divergence() {
    // This PASSES and documents a contract divergence, not a defect on the
    // domain's own terms: the domain clears selection only on Close (invariant
    // #1), so a selected session observed to a terminal status stays selected.
    // The supervisor enforces the stronger "selection implies Running". The two
    // must be reconciled when the supervisor is pointed at the domain types.
    let mut reg = dom::SessionRegistry::new();
    let id = reg.create(dom::SessionKind::Local, None);
    reg.select(id).expect("select");
    reg.observe(id, dom::SessionStatus::Exited)
        .expect("observe");
    assert_eq!(reg.selected().map(|s| s.id()), Some(id));
    assert_eq!(reg.get(id).unwrap().status(), dom::SessionStatus::Exited);
}

// ═════════════════════════════════════════════════════════════════════════════
// Supervisor — defensive (passing) tests.
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn supervisor_child_crash_via_poll_is_exited_not_stuck_running() {
    let (child, ctrl) = MockChild::running_with_control();
    let mut s = supervisor_with(vec![child]);
    let id = s.spawn().expect("spawn");
    assert_eq!(s.status(id), Some(SessionStatus::Running));
    assert_eq!(s.selected(), Some(id));

    ctrl.exit(Some(0));
    let report = s.poll();
    assert_eq!(report.transitioned(), 1);
    assert_eq!(report.exited(), &[(id, Some(0))]);
    assert_eq!(s.status(id), Some(SessionStatus::Exited { code: Some(0) }));
    // The load-bearing invariant: not a frozen Running, and selection cleared.
    assert_ne!(s.status(id), Some(SessionStatus::Running));
    assert_eq!(s.selected(), None);
}

#[test]
fn supervisor_signal_like_exit_reports_no_code_but_still_exited() {
    let (child, ctrl) = MockChild::running_with_control();
    let mut s = supervisor_with(vec![child]);
    let id = s.spawn().expect("spawn");
    ctrl.exit(None);
    let _ = s.poll();
    assert_eq!(s.status(id), Some(SessionStatus::Exited { code: None }));
    assert_ne!(s.status(id), Some(SessionStatus::Running));
}

#[test]
fn supervisor_poll_error_surfaces_as_failed_not_running() {
    let (child, ctrl) = MockChild::running_with_control();
    let mut s = supervisor_with(vec![child]);
    let id = s.spawn().expect("spawn");
    ctrl.fail_poll();
    let report = s.poll();
    assert_eq!(report.failed(), &[(id, SessionFailure::PollFailed)]);
    assert_eq!(
        s.status(id),
        Some(SessionStatus::Failed {
            reason: SessionFailure::PollFailed
        })
    );
}

#[test]
fn supervisor_terminate_is_idempotent_and_skips_backend_for_terminal() {
    let (child, ctrl) = MockChild::running_with_control();
    let mut s = supervisor_with(vec![child]);
    let id = s.spawn().expect("spawn");

    let first = s.terminate_now(id);
    assert_eq!(first, SessionStatus::Exited { code: None });
    assert_eq!(ctrl.shutdown_count(), 1);

    // Idempotent: same status, no second backend shutdown.
    let again = s.terminate_now(id);
    assert_eq!(again, first);
    assert_eq!(ctrl.shutdown_count(), 1);
}

#[test]
fn supervisor_terminate_shutdown_failure_surfaces_as_failed() {
    let (child, ctrl) = MockChild::running_with_control();
    let mut s = supervisor_with(vec![child]);
    let id = s.spawn().expect("spawn");
    ctrl.fail_shutdown(sup::ShutdownError::Failed);
    let status = s.terminate_now(id);
    assert_eq!(
        status,
        SessionStatus::Failed {
            reason: SessionFailure::ShutdownFailed
        }
    );
}

#[test]
fn supervisor_terminate_elapsed_deadline_is_reap_timeout_without_backend_call() {
    let (child, ctrl) = MockChild::running_with_control();
    let mut s = supervisor_with(vec![child]);
    let id = s.spawn().expect("spawn");
    let past = std::time::Instant::now();
    let status = s.terminate(id, past);
    assert_eq!(
        status,
        SessionStatus::Failed {
            reason: SessionFailure::ReapTimeout
        }
    );
    assert_eq!(ctrl.shutdown_count(), 0);
}

#[test]
fn supervisor_shutdown_all_uses_one_shared_deadline_and_is_idempotent() {
    let (children, ctrls) = controlled_children(3);
    let mut s = supervisor_with(children);
    let ids: Vec<_> = (0..3).map(|_| s.spawn().expect("spawn")).collect();

    let results = s.shutdown_all();
    assert_eq!(results.iter().map(|(id, _)| *id).collect::<Vec<_>>(), ids);
    for (_, status) in &results {
        assert_ne!(*status, SessionStatus::Running);
    }
    assert_eq!(s.running_count(), 0);
    assert_eq!(s.selected(), None);

    // Each child shut down exactly once, and every child received the SAME
    // deadline Instant (one shared budget, not n * deadline).
    let first_deadlines = ctrls[0].deadlines();
    assert_eq!(first_deadlines.len(), 1);
    for ctrl in &ctrls {
        let d = ctrl.deadlines();
        assert_eq!(d.len(), 1, "shutdown called more than once");
        assert_eq!(d[0], first_deadlines[0], "deadlines differ across sessions");
    }

    // Idempotent: a second pass keeps everything terminal, no extra backend work.
    let again = s.shutdown_all();
    assert_eq!(again.len(), 3);
    assert_eq!(s.running_count(), 0);
    for ctrl in &ctrls {
        assert_eq!(ctrl.shutdown_count(), 1);
    }
}

#[test]
fn supervisor_select_refuses_dead_and_unknown_and_forget_requires_terminal() {
    let (child_a, ctrl_a) = MockChild::running_with_control();
    let (child_c, _ctrl_c) = MockChild::running_with_control();
    let mut s = supervisor_with(vec![child_a, child_c]);
    let a = s.spawn().expect("spawn");
    let c = s.spawn().expect("spawn");

    // Live sessions are selectable; selection is the scalar last-written.
    s.select(c).expect("select c");
    s.select(a).expect("select a");
    assert_eq!(s.selected(), Some(a));

    // Crash the selected session: poll must clear the selection.
    ctrl_a.exit(Some(0));
    let _ = s.poll();
    assert_eq!(s.selected(), None);
    assert_eq!(
        s.select(a),
        Err(SessionOpError::NotRunning),
        "a dead session must not be selectable"
    );

    // c is still live.
    s.select(c).expect("select c live");

    // forget requires terminal: c is running -> StillRunning.
    assert_eq!(s.forget(c), Err(SessionOpError::StillRunning));

    // Terminate then forget c: it becomes unknown afterwards.
    s.terminate_now(c);
    s.forget(c).expect("forget terminal c");
    assert_eq!(s.status(c), None);
    assert_eq!(s.select(c), Err(SessionOpError::Unknown));
    assert_eq!(s.forget(c), Err(SessionOpError::Unknown));

    // forget of the already-terminal a succeeds and empties the supervisor.
    s.forget(a).expect("forget terminal a");
    assert!(s.is_empty());
}

#[test]
fn supervisor_spawn_failure_records_no_session() {
    let mut s = sup::SessionSupervisor::new(Box::new(MockSpawner::failing()));
    assert_eq!(s.spawn(), Err(SessionFailure::SpawnFailed));
    assert!(s.is_empty());
    assert_eq!(s.selected(), None);
    assert_eq!(s.len(), 0);
}

#[test]
fn supervisor_rapid_selection_amid_crashes_never_dangles() {
    // Chaotic mix of selection switching and crashes. After every step the
    // invariant holds: selection is None or points at a Running session.
    let (children, ctrls) = controlled_children(6);
    let mut s = supervisor_with(children);
    let ids: Vec<_> = (0..6).map(|_| s.spawn().expect("spawn")).collect();

    for round in 0..30 {
        let target = ids[round % ids.len()];
        let _ = s.select(target); // ok or NotRunning if it died earlier
        assert!(selection_is_valid(&s), "selection dangled at round {round}");

        // Crash whichever session this round's controller owns.
        ctrls[round % ctrls.len()].exit(Some(0));
        let _ = s.poll();
        assert!(
            selection_is_valid(&s),
            "selection dangled after poll at round {round}"
        );
    }
    // No session is stuck Running after every controller signalled exit + poll.
    let _ = s.shutdown_all();
    assert_eq!(s.running_count(), 0);
}

#[test]
fn supervisor_repeated_spawn_terminate_forget_cycle_stays_bounded() {
    // Contrast with ADV-S2: WITH forget, the attach/detach loop is bounded.
    // Each iteration retires the record, so len returns to zero.
    for _ in 0..500 {
        let mut s = supervisor_with(vec![MockChild::running()]);
        let id = s.spawn().expect("spawn");
        s.terminate_now(id);
        s.forget(id).expect("forget");
        assert_eq!(s.len(), 0, "record not retired by forget");
    }
}

fn controlled_children(n: usize) -> (Vec<MockChild>, Vec<MockController>) {
    let mut children = Vec::with_capacity(n);
    let mut ctrls = Vec::with_capacity(n);
    for _ in 0..n {
        let (child, ctrl) = MockChild::running_with_control();
        children.push(child);
        ctrls.push(ctrl);
    }
    (children, ctrls)
}

// ═════════════════════════════════════════════════════════════════════════════
// Regression guards — ADV-S1 / ADV-S2 / ADV-S3, ported to the MERGED modules.
//
// These are NOT the original `#[ignore]` reproducers (which mirrored the buggy
// algorithm through the `fake_*` modules above). The defects are fixed in
// `src/session.rs` and `src/session_supervisor.rs`, so each guard below
// compiles and runs against the real merged code via the `#[path]` includes at
// the top of this file. They assert the fixed property holds; if any fix
// regresses they fail. The property each guards is the same the original
// reproducer attacked; the assertions are adapted to the fixed contracts
// (regressions now error / unknown ids now surface honestly) rather than
// weakened.
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn adv_s1_observe_rejects_non_monotonic_status_regression() {
    // ADV-S1 (fixed in src/session.rs): observe() now enforces a monotonic
    // status rank (Starting < Running < terminal). A regression and a
    // resurrection of a dead session are both rejected with
    // InvalidStatusTransition; a dead session stays dead.
    use session::{SessionError, SessionKind, SessionRegistry, SessionStatus};

    let mut reg = SessionRegistry::new();
    let id = reg.create(SessionKind::Local);

    // Forward: Starting -> Running.
    reg.observe(id, SessionStatus::Running)
        .expect("advance to Running");
    assert_eq!(reg.get(id).unwrap().status(), &SessionStatus::Running);

    // Regression 1: Running -> Starting must be rejected (no un-observing).
    assert_eq!(
        reg.observe(id, SessionStatus::Starting),
        Err(SessionError::InvalidStatusTransition),
        "observe(Starting) must not regress a Running session"
    );
    assert_eq!(reg.get(id).unwrap().status(), &SessionStatus::Running);

    // Forward: Running -> Exited (terminal).
    reg.observe(id, SessionStatus::Exited { code: Some(0) })
        .expect("advance to terminal");

    // Regression 2: Exited -> Running must be rejected (no resurrection). A
    // dead session stays dead — the most alarming form of the original defect.
    assert_eq!(
        reg.observe(id, SessionStatus::Running),
        Err(SessionError::InvalidStatusTransition),
        "observe(Running) must not resurrect an Exited session"
    );
    assert_eq!(
        reg.get(id).unwrap().status(),
        &SessionStatus::Exited { code: Some(0) }
    );

    // Equal-rank refinement is still permitted (not a resurrection): Failed ->
    // Exited once a real exit code arrives. Both are terminal.
    let id2 = reg.create(SessionKind::Local);
    reg.observe(
        id2,
        SessionStatus::Failed {
            reason: "boom".to_owned(),
        },
    )
    .expect("advance to Failed");
    reg.observe(id2, SessionStatus::Exited { code: Some(1) })
        .expect("refine Failed -> Exited");
    assert_eq!(
        reg.get(id2).unwrap().status(),
        &SessionStatus::Exited { code: Some(1) }
    );
}

#[test]
fn adv_s2_dead_records_do_not_accumulate_without_bound() {
    // ADV-S2 (fixed in src/session_supervisor.rs): the supervisor auto-retires
    // terminal records past RETAIN_TERMINAL_RECORDS, so repeated spawn/die
    // cycles cannot grow memory without bound. Stated bound: total retained
    // records <= running_count() + RETAIN_TERMINAL_RECORDS.
    use session_supervisor::mock::{MockChild, MockSpawner};
    use session_supervisor::{RETAIN_TERMINAL_RECORDS, SessionSupervisor};

    let children: Vec<_> = (0..500).map(|_| MockChild::exited()).collect();
    let mut s = SessionSupervisor::new(Box::new(MockSpawner::from_children(children)));
    for _ in 0..500 {
        let _id = s.spawn().expect("spawn");
        let _ = s.poll(); // reaps immediately -> terminal; oldest retired past cap
    }
    assert_eq!(s.running_count(), 0, "no session should still be running");
    // Boundedness: dead records do not accumulate to 500; they are capped at a
    // constant independent of how many sessions ever existed.
    assert!(
        s.len() <= RETAIN_TERMINAL_RECORDS,
        "supervisor retained {} records after 500 spawn/crash cycles; \
         bounded by RETAIN_TERMINAL_RECORDS ({})",
        s.len(),
        RETAIN_TERMINAL_RECORDS
    );
    assert!(
        s.len() < 500,
        "the unbounded growth the defect allowed is gone"
    );
}

#[test]
fn adv_s3_terminate_unknown_id_signals_unknown_not_fabricated_status() {
    // ADV-S3 (fixed in src/session_supervisor.rs): terminate() now returns
    // Result<SessionStatus, SessionOpError>; an unknown id surfaces honestly as
    // Err(Unknown) instead of fabricating Failed(PollFailed) for a poll that
    // never happened on a session that does not exist.
    use session_supervisor::mock::{MockChild, MockSpawner};
    use session_supervisor::{SessionOpError, SessionSupervisor};

    let mut s = SessionSupervisor::new(Box::new(MockSpawner::from_children(vec![
        MockChild::running(),
    ])));
    let id = s.spawn().expect("spawn");
    s.terminate_now(id).expect("terminate live"); // -> terminal
    s.forget(id).expect("forget terminal");
    assert_eq!(s.status(id), None); // genuinely unknown now

    // The honest representation: Err(Unknown), not a fabricated Failed status.
    assert_eq!(
        s.terminate_now(id),
        Err(SessionOpError::Unknown),
        "terminate on an unknown id must signal Unknown, not fabricate a status"
    );
}
