//! Session lifecycle supervisor: spawn, terminate, select, process ownership,
//! child reaping, and stale/dead session handling.
//!
//! # What this lane owns
//!
//! The supervisor is the sole owner of live child handles. It:
//!
//! - **spawns** sessions through an injected [`Spawner`] (the seam where the
//!   real PTY wiring drops in),
//! - **reaps** them on a non-blocking [`SessionSupervisor::poll`] so a child
//!   that died is surfaced as [`SessionStatus::Exited`] or
//!   [`SessionStatus::Failed`] — never a stuck [`SessionStatus::Running`],
//! - **terminates** them with [`SessionSupervisor::terminate`], which delegates
//!   to the child's own kill+reap under one shared deadline and is idempotent,
//! - **selects** the focused session, refusing to focus a dead one, and
//! - **forgets** terminal sessions so their records can be retired.
//!
//! The registry (the parallel session-domain lane) never owns a child handle;
//! it observes status through this module by id. That ownership split is the
//! point of the lane: a dead child must become `Exited`/`Failed`, not a frozen
//! `Running`, and only the supervisor can move that needle because only it holds
//! the handle.
//!
//! # Termination contract
//!
//! Termination matches the existing PTY behaviour (`crates/noren-pty/src/lib.rs`
//! `PtySession::shutdown`): it is bounded by a single deadline, reaps the child,
//! and is idempotent. [`SessionSupervisor::shutdown_all`] computes one absolute
//! deadline and feeds it to every session so the whole batch finishes within
//! that budget rather than `n * deadline`. A second call on an already-terminal
//! session returns the recorded status without redoing work and without
//! re-invoking the backend.
//!
//! # Integration with D-M3-001 (stub boundary)
//!
//! The session-domain lane (`agent/m3-session-domain`) is defining the shared
//! session API contract in parallel; it is **not** on `main` yet. The types
//! marked `STUB` below — [`SessionId`], [`SessionStatus`], and
//! [`SessionFailure`] — are the minimal local stand-ins needed to compile and
//! test. They reference decision **D-M3-001**. Integration is a **deletion, not
//! a merge**: delete the `STUB` block and re-export the domain's types, then
//! point [`SessionSupervisor`] at them. Everything else here (the [`Child`]
//! trait, the [`Spawner`] seam, [`SessionSupervisor`] itself, the reaping state
//! machine, the mock) is this lane's real, non-stub deliverable.
//!
//! # Not wired yet
//!
//! This module is intentionally **not** declared in `crates/noren-app/src/lib.rs`
//! (wiring is a later serial integration commit). The integration test includes
//! it via `#[path = "../src/session_supervisor.rs"]` so it compiles and runs as
//! a standalone test target against a fake/mock process model plus a failure
//! matrix. See `tests/session_supervisor.rs`.

use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

/// Deadline for orderly shutdown of one session or the whole batch.
///
/// Mirrors `noren-pty::SHUTDOWN_DEADLINE` so termination budgets agree across
/// the two layers. Termination never blocks longer than this.
pub const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(2);

// ─────────────────────────────────────────────────────────────────────────────
// STUB block — delete on integration with D-M3-001 (session-domain lane).
// These stand in for the domain's `SessionId` / status / failure types only so
// this lane compiles and tests in isolation. Do not extend them.
// ─────────────────────────────────────────────────────────────────────────────

/// STUB (D-M3-001): opaque session identifier.
///
/// Locally an incrementing `u64`. The domain's id type replaces this verbatim;
/// the supervisor only stores and compares ids and never inspects their bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SessionId(u64);

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "session#{}", self.0)
    }
}

/// STUB (D-M3-001): observed lifecycle status of a supervised session.
///
/// The two terminal variants are the load-bearing distinction: once the child
/// is dead it is [`SessionStatus::Exited`] (it ran and stopped, with an exit
/// code when the backend reported one) or [`SessionStatus::Failed`] (the
/// supervisor could not cleanly reap it). It is never left as
/// [`SessionStatus::Running`] after death.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionStatus {
    /// Spawned and not yet observed to have exited.
    Running,
    /// Reaped after exiting. `code` is `Some(n)` when the backend reported an
    /// exit code and `None` for a signal-style death with no code.
    Exited {
        /// Backend-reported exit code, if any.
        code: Option<u32>,
    },
    /// Reaped or abandoned after an operational fault: spawn/kill/reap error or
    /// a reap that overshot the deadline.
    Failed {
        /// Typed reason, control-plane only (no child output).
        reason: SessionFailure,
    },
}

/// STUB (D-M3-001): why a session became [`SessionStatus::Failed`].
///
/// Control-plane only: it never carries PTY bytes, screen text, or commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionFailure {
    /// The spawner could not produce a child handle.
    SpawnFailed,
    /// A non-blocking liveness poll reported a backend error.
    PollFailed,
    /// The backend kill/reap returned a hard error (not a timeout).
    ShutdownFailed,
    /// The kill/reap did not complete before the deadline; the child was
    /// abandoned as failed rather than left `Running`.
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

/// True when the status is one of the terminal (`Exited`/`Failed`) variants.
fn is_terminal(status: SessionStatus) -> bool {
    !matches!(status, SessionStatus::Running)
}

// ─────────────────────────────────────────────────────────────────────────────
// Process handle trait — the supervisor owns this; the registry never does.
// ─────────────────────────────────────────────────────────────────────────────

/// Failure reported by a [`Child`] operation. Control-plane only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildError {
    /// A non-blocking liveness poll failed.
    Poll,
    /// The bounded kill/reap failed. `TimedOut` means the deadline elapsed
    /// before the child was reaped; `Failed` is any other backend error.
    Shutdown(ShutdownError),
}

/// Outcome of a non-blocking [`Child::poll_exit`] probe.
///
/// This exists specifically to keep "still alive" distinct from "exited with no
/// code" — the ambiguity that would otherwise let a dead child read as `Running`.
/// A reaped child is always [`PollOutcome::Exited`], with `code: None` for a
/// signal-style death; only a genuinely live child is [`PollOutcome::StillRunning`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PollOutcome {
    /// The child is still alive; no exit has been reaped.
    StillRunning,
    /// The child has exited and been reaped. `code` is `Some(n)` when the
    /// backend reported a code and `None` for a signal-style death.
    Exited {
        /// Backend-reported exit code, if any.
        code: Option<u32>,
    },
}

/// Sub-reason for a [`ChildError::Shutdown`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShutdownError {
    /// The backend returned a hard error from kill or reap.
    Failed,
    /// The deadline elapsed before reap completed.
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

/// A supervised child process handle.
///
/// The supervisor holds the only strong reference to a live child; the registry
/// never does. The trait deliberately exposes only the two operations the
/// supervisor needs:
///
/// - [`Child::poll_exit`] is the non-blocking reaping probe. It returns
///   [`PollOutcome::StillRunning`] while the child is alive and
///   [`PollOutcome::Exited`] once the backend has observed and reaped the exit.
///   The explicit outcome is what turns a dead child into `Exited` instead of a
///   stuck `Running` — "exited with no code" must not be confusable with
///   "still alive".
/// - [`Child::shutdown`] performs kill + reap + join under the supplied
///   deadline and is idempotent at the backend level (a repeat call after exit
///   is a no-op), mirroring `noren-pty::PtySession::shutdown`.
///
/// Production wiring is a thin adapter over `noren-pty::PtySession`:
/// `poll_exit` drains `try_recv` for `PtyEvent::Exited` (mapping it to
/// [`PollOutcome::Exited`], and absence to [`PollOutcome::StillRunning`]), and
/// `shutdown` forwards to `PtySession::shutdown`. That adapter lives in the
/// serial integration commit, not here, so this module stays free of PTY types.
pub trait Child: Send {
    /// Non-blocking liveness probe.
    fn poll_exit(&mut self) -> Result<PollOutcome, ChildError>;

    /// Kill, reap, and join before `deadline`. Idempotent.
    fn shutdown(&mut self, deadline: Instant) -> Result<(), ChildError>;
}

/// Factory that creates a [`Child`] for a new session.
///
/// This is the seam where real PTY spawn wiring drops in. The supervisor owns
/// child *handles*; it does not own spawn policy (program, argv, cwd, size),
/// which is the domain lane's concern and arrives through this trait.
pub trait Spawner: Send {
    /// Create one child handle. The returned box is owned solely by the
    /// supervisor from this point on.
    fn spawn(&mut self) -> Result<Box<dyn Child + Send>, SessionFailure>;
}

// ─────────────────────────────────────────────────────────────────────────────
// Selection and record-management errors.
// ─────────────────────────────────────────────────────────────────────────────

/// Why a [`SessionSupervisor::select`] or [`SessionSupervisor::forget`] failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionOpError {
    /// The id is unknown to the supervisor.
    Unknown,
    /// A select targeted a session that is no longer running.
    NotRunning,
    /// A forget targeted a session that must be terminated first.
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

/// Transitions observed during one [`SessionSupervisor::poll`] pass.
///
/// Lists exactly the sessions that left `Running` this pass, in insertion order.
/// `Exited` and `Failed` are reported separately so a caller can distinguish a
/// natural exit from an operational fault without re-reading status.
#[derive(Debug, Default)]
pub struct ReapReport {
    exited: Vec<(SessionId, Option<u32>)>,
    failed: Vec<(SessionId, SessionFailure)>,
}

impl ReapReport {
    /// Sessions that exited this pass with their backend exit code.
    #[must_use]
    pub fn exited(&self) -> &[(SessionId, Option<u32>)] {
        &self.exited
    }

    /// Sessions that failed this pass with their typed reason.
    #[must_use]
    pub fn failed(&self) -> &[(SessionId, SessionFailure)] {
        &self.failed
    }

    /// Total sessions that left `Running` this pass.
    #[must_use]
    pub fn transitioned(&self) -> usize {
        self.exited.len() + self.failed.len()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The supervisor.
// ─────────────────────────────────────────────────────────────────────────────

/// One supervised session: an optional live child handle plus its status.
///
/// `child` is `None` once the session is terminal — the supervisor has reaped
/// and released the handle. The status remains so callers can observe the
/// outcome by id.
struct SupervisedSession {
    child: Option<Box<dyn Child + Send>>,
    status: SessionStatus,
}

/// Lifecycle supervisor owning every live child handle.
///
/// Construct with [`SessionSupervisor::new`] and an injected [`Spawner`]. The
/// supervisor is single-threaded by design (it runs on the app's main thread or
/// a dedicated worker); it does not need to be `Send` because child handles need
/// not be `Sync`.
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
    /// Create an empty supervisor that spawns children through `spawner`.
    pub fn new(spawner: Box<dyn Spawner>) -> Self {
        Self {
            sessions: HashMap::new(),
            order: Vec::new(),
            selected: None,
            spawner,
            next_id: 0,
        }
    }

    /// Spawn a new session.
    ///
    /// On success the session is `Running` and becomes the selected one. On
    /// failure no session is recorded and the typed reason is returned; the
    /// spawner retains ownership of any partial child it created.
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

    /// Current status of `id`, or `None` if unknown.
    #[must_use]
    pub fn status(&self, id: SessionId) -> Option<SessionStatus> {
        self.sessions.get(&id).map(|session| session.status)
    }

    /// Number of tracked sessions (running or terminal).
    #[must_use]
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Whether any session is tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Number of sessions still `Running`.
    #[must_use]
    pub fn running_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|session| session.status == SessionStatus::Running)
            .count()
    }

    /// The currently selected session id, if any.
    #[must_use]
    pub fn selected(&self) -> Option<SessionId> {
        self.selected
    }

    /// Focus `id`. Fails if `id` is unknown or not `Running`: a dead session
    /// surfaces as [`SessionOpError::NotRunning`], never as a silently-stuck
    /// selection.
    pub fn select(&mut self, id: SessionId) -> Result<(), SessionOpError> {
        let session = self.sessions.get(&id).ok_or(SessionOpError::Unknown)?;
        if session.status != SessionStatus::Running {
            return Err(SessionOpError::NotRunning);
        }
        self.selected = Some(id);
        Ok(())
    }

    /// Clear the current selection without terminating anything.
    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    /// Non-blocking reap pass over every `Running` session.
    ///
    /// For each still-running session this probes [`Child::poll_exit`]; a child
    /// that has exited is moved to `Exited` and one whose poll errored is moved
    /// to `Failed`. If the selected session transitioned, the selection is
    /// cleared so a caller does not address a dead session as if it were live.
    /// The transitioned sessions are returned in a [`ReapReport`].
    pub fn poll(&mut self) -> ReapReport {
        let mut report = ReapReport::default();
        // Collect ids first to avoid borrowing `sessions` across child calls.
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
                    // No handle and still Running is an internal inconsistency;
                    // surface it as a failure rather than freezing.
                    None => Err(ChildError::Poll),
                },
                None => continue,
            };
            match outcome {
                Ok(PollOutcome::StillRunning) => {}
                Ok(PollOutcome::Exited { code }) => self.mark_exited(&mut report, id, code),
                Err(ChildError::Poll) => {
                    self.mark_failed(&mut report, id, SessionFailure::PollFailed)
                }
                Err(ChildError::Shutdown(reason)) => {
                    self.mark_failed(&mut report, id, shutdown_to_failure(reason));
                }
            }
        }
        report
    }

    /// Terminate `id`: kill + reap under `deadline`, idempotently.
    ///
    /// - If `id` is already terminal, returns its status without redoing work.
    /// - If `deadline` has already passed, marks `Failed(ReapTimeout)`.
    /// - Otherwise delegates to [`Child::shutdown`], then reads the exit code:
    ///   a code → `Exited`, no observable code → `Exited { code: None }`, a
    ///   backend error → `Failed`.
    ///
    /// The child handle is released once terminal.
    pub fn terminate(&mut self, id: SessionId, deadline: Instant) -> SessionStatus {
        // Fast path: already terminal — idempotent, no backend call.
        let already = self.sessions.get(&id).map(|session| session.status);
        match already {
            Some(status) if is_terminal(status) => return status,
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

        let backend = self
            .sessions
            .get_mut(&id)
            .and_then(|session| session.child.as_mut());
        let result = match backend {
            Some(child) => child.shutdown(deadline),
            None => Err(ChildError::Poll),
        };

        match result {
            Ok(()) => {
                // After a successful shutdown the backend has reaped the child;
                // read the recorded code. `shutdown` returning `Ok` is itself the
                // terminal signal, so even `StillRunning` (a backend
                // inconsistency) is recorded as `Exited { code: None }` rather
                // than leaving the session `Running`.
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

    /// Read the post-shutdown outcome without borrowing `sessions` across calls.
    fn poll_after_shutdown(&mut self, id: SessionId) -> Result<PollOutcome, ChildError> {
        match self.sessions.get_mut(&id) {
            Some(session) => match session.child.as_mut() {
                Some(child) => child.poll_exit(),
                None => Ok(PollOutcome::Exited { code: None }),
            },
            None => Ok(PollOutcome::Exited { code: None }),
        }
    }

    /// Terminate `id` under the standard [`SHUTDOWN_DEADLINE`] from now.
    pub fn terminate_now(&mut self, id: SessionId) -> SessionStatus {
        let deadline = Instant::now() + SHUTDOWN_DEADLINE;
        self.terminate(id, deadline)
    }

    /// Terminate every session under one shared deadline.
    ///
    /// Computes a single `now + SHUTDOWN_DEADLINE` and feeds it to each
    /// [`Self::terminate`], so the whole batch is bounded by one deadline
    /// rather than `n * deadline`. Already-terminal sessions are skipped.
    /// Idempotent: a second call performs no backend work. Returns the final
    /// status of every session in insertion order, and clears the selection.
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

    /// Retire a terminal session's record.
    ///
    /// Returns [`SessionOpError::StillRunning`] for a live session (terminate it
    /// first) and [`SessionOpError::Unknown`] for an unknown id.
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

// ─────────────────────────────────────────────────────────────────────────────
// Mock process model + scripted failure harness (test-only).
// ─────────────────────────────────────────────────────────────────────────────
//
// The mock is a tiny shared-state machine a test drives through a
// [`mock::MockController`]: flip the child to exited, or toggle poll/shutdown
// failures. This is what makes the failure matrix deterministic without
// spawning real processes. It is `pub` under `cfg(test)` so both this module's
// unit tests and the `tests/session_supervisor.rs` integration target (which
// includes this file via `#[path]`, and for which `cfg(test)` is also enabled)
// share one definition. Production never references it.

#[cfg(test)]
pub mod mock {
    use super::{Child, ChildError, PollOutcome, SessionFailure, ShutdownError};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    #[derive(Debug)]
    struct MockState {
        alive: bool,
        exit_code: Option<u32>,
        poll_error: bool,
        shutdown_error: Option<ShutdownError>,
        shutdown_calls: u32,
        poll_calls: u32,
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
            }
        }
    }

    /// Scripted child for the failure matrix. Shares its state with a
    /// [`MockController`] the test holds.
    pub struct MockChild {
        state: Arc<Mutex<MockState>>,
    }

    /// Test-side handle that scripts a [`MockChild`] the supervisor owns.
    pub struct MockController {
        state: Arc<Mutex<MockState>>,
    }

    impl MockChild {
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
                // The recorded code verbatim: `Some(n)` for a self-exit with a
                // code, `None` for a signal-style death or a supervisor kill
                // (kill produces no clean exit code in this model). The
                // outcome stays `Exited`, never `StillRunning`, once dead.
                Ok(PollOutcome::Exited {
                    code: state.exit_code,
                })
            }
        }

        fn shutdown(&mut self, _deadline: Instant) -> Result<(), ChildError> {
            let mut state = self.state.lock().expect("mock lock");
            state.shutdown_calls += 1;
            if let Some(error) = state.shutdown_error {
                return Err(ChildError::Shutdown(error));
            }
            // Kill takes effect: the child is no longer alive. The exit code is
            // whatever the controller scripted (None for a signal-style kill,
            // which is the default).
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

        /// Cause the next and subsequent `poll_exit` calls to error.
        pub fn fail_poll(&self) {
            self.state.lock().expect("mock lock").poll_error = true;
        }

        /// Cause the next and subsequent `shutdown` calls to error with `reason`.
        pub fn fail_shutdown(&self, reason: ShutdownError) {
            self.state.lock().expect("mock lock").shutdown_error = Some(reason);
        }

        /// Number of times the supervisor called `shutdown`.
        #[must_use]
        pub fn shutdown_count(&self) -> u32 {
            self.state.lock().expect("mock lock").shutdown_calls
        }

        /// Number of times the supervisor called `poll_exit`.
        #[must_use]
        pub fn poll_count(&self) -> u32 {
            self.state.lock().expect("mock lock").poll_calls
        }
    }

    /// Spawner that dispenses pre-built children in order.
    pub struct MockSpawner {
        children: std::collections::VecDeque<MockChild>,
        fail_next: bool,
    }

    impl MockSpawner {
        /// Dispense `children` in order.
        #[must_use]
        pub fn from_children(children: Vec<MockChild>) -> Self {
            Self {
                children: children.into(),
                fail_next: false,
            }
        }

        /// A spawner whose next `spawn` fails with [`SessionFailure::SpawnFailed`].
        #[must_use]
        pub fn failing() -> Self {
            Self {
                children: std::collections::VecDeque::new(),
                fail_next: true,
            }
        }
    }

    impl super::Spawner for MockSpawner {
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

#[cfg(test)]
mod tests {
    //! The supervisor's own failure-matrix unit tests. The independent
    //! integration verification lives in `tests/session_supervisor.rs`.

    use super::mock::{MockChild, MockSpawner};
    use super::{
        ChildError, SessionFailure, SessionOpError, SessionStatus, SessionSupervisor, ShutdownError,
    };

    fn supervisor_with(children: Vec<MockChild>) -> SessionSupervisor {
        SessionSupervisor::new(Box::new(MockSpawner::from_children(children)))
    }

    #[test]
    fn spawn_assigns_unique_ids_and_selects_newest() {
        let mut sup = supervisor_with(vec![MockChild::running(), MockChild::running()]);
        let a = sup.spawn().expect("first spawn");
        let b = sup.spawn().expect("second spawn");
        assert_ne!(a, b);
        assert_eq!(sup.status(a), Some(SessionStatus::Running));
        assert_eq!(sup.status(b), Some(SessionStatus::Running));
        assert_eq!(sup.selected(), Some(b));
        assert_eq!(sup.running_count(), 2);
        assert_eq!(sup.len(), 2);
        assert!(!sup.is_empty());
    }

    #[test]
    fn poll_surfaces_unprompted_exit_as_exited_not_running() {
        let (child, ctrl) = MockChild::running_with_control();
        let mut sup = supervisor_with(vec![child]);
        let id = sup.spawn().expect("spawn");
        assert_eq!(sup.status(id), Some(SessionStatus::Running));

        // The child exits on its own with code 0; poll must discover it.
        ctrl.exit(Some(0));
        let report = sup.poll();
        assert_eq!(report.transitioned(), 1);
        assert_eq!(report.exited(), &[(id, Some(0))]);
        assert_eq!(
            sup.status(id),
            Some(SessionStatus::Exited { code: Some(0) })
        );
        // The load-bearing invariant: not Running, and selection cleared.
        assert_ne!(sup.status(id), Some(SessionStatus::Running));
        assert_eq!(sup.selected(), None);
    }

    #[test]
    fn non_zero_and_signal_like_exits_round_trip_through_poll() {
        let (child, ctrl) = MockChild::running_with_control();
        let mut sup = supervisor_with(vec![child]);
        let id = sup.spawn().expect("spawn");
        ctrl.exit(Some(7));
        let _ = sup.poll();
        assert_eq!(
            sup.status(id),
            Some(SessionStatus::Exited { code: Some(7) })
        );

        // A signal-style death reports no code and is still Exited.
        let (child2, ctrl2) = MockChild::running_with_control();
        let mut sup2 = supervisor_with(vec![child2]);
        let id2 = sup2.spawn().expect("spawn");
        ctrl2.exit(None);
        let _ = sup2.poll();
        assert_eq!(sup2.status(id2), Some(SessionStatus::Exited { code: None }));
    }

    #[test]
    fn poll_error_surfaces_as_failed_not_running() {
        let (child, ctrl) = MockChild::running_with_control();
        let mut sup = supervisor_with(vec![child]);
        let id = sup.spawn().expect("spawn");
        ctrl.fail_poll();
        let report = sup.poll();
        assert_eq!(report.failed(), &[(id, SessionFailure::PollFailed)]);
        assert_eq!(
            sup.status(id),
            Some(SessionStatus::Failed {
                reason: SessionFailure::PollFailed
            })
        );
        assert_ne!(sup.status(id), Some(SessionStatus::Running));
    }

    #[test]
    fn terminate_reaps_a_running_child_and_is_idempotent() {
        let (child, ctrl) = MockChild::running_with_control();
        let mut sup = supervisor_with(vec![child]);
        let id = sup.spawn().expect("spawn");

        let status = sup.terminate_now(id);
        // The mock's shutdown kills the child; a kill yields no clean exit code,
        // so the recorded status is `Exited { code: None }`.
        assert_eq!(status, SessionStatus::Exited { code: None });
        assert_eq!(ctrl.shutdown_count(), 1);

        // Idempotent: no second backend shutdown, same status.
        let again = sup.terminate_now(id);
        assert_eq!(again, status);
        assert_eq!(ctrl.shutdown_count(), 1);
    }

    #[test]
    fn terminate_already_terminal_does_no_backend_work() {
        let (child, ctrl) = MockChild::running_with_control();
        let mut sup = supervisor_with(vec![child]);
        let id = sup.spawn().expect("spawn");
        ctrl.exit(Some(3));
        let _ = sup.poll();
        let before = ctrl.shutdown_count();
        let status = sup.terminate_now(id);
        assert_eq!(status, SessionStatus::Exited { code: Some(3) });
        assert_eq!(ctrl.shutdown_count(), before);
    }

    #[test]
    fn terminate_shutdown_failure_surfaces_as_failed() {
        let (child, ctrl) = MockChild::running_with_control();
        let mut sup = supervisor_with(vec![child]);
        let id = sup.spawn().expect("spawn");
        ctrl.fail_shutdown(ShutdownError::Failed);
        let status = sup.terminate_now(id);
        assert_eq!(
            status,
            SessionStatus::Failed {
                reason: SessionFailure::ShutdownFailed
            }
        );
    }

    #[test]
    fn terminate_deadline_timeout_surfaces_as_reap_timeout() {
        let (child, ctrl) = MockChild::running_with_control();
        let mut sup = supervisor_with(vec![child]);
        let id = sup.spawn().expect("spawn");
        ctrl.fail_shutdown(ShutdownError::TimedOut);
        let status = sup.terminate_now(id);
        assert_eq!(
            status,
            SessionStatus::Failed {
                reason: SessionFailure::ReapTimeout
            }
        );
    }

    #[test]
    fn terminate_with_elapsed_deadline_is_reap_timeout_without_backend_call() {
        let (child, ctrl) = MockChild::running_with_control();
        let mut sup = supervisor_with(vec![child]);
        let id = sup.spawn().expect("spawn");
        let past = std::time::Instant::now();
        let status = sup.terminate(id, past);
        assert_eq!(
            status,
            SessionStatus::Failed {
                reason: SessionFailure::ReapTimeout
            }
        );
        assert_eq!(ctrl.shutdown_count(), 0);
    }

    #[test]
    fn shutdown_all_terminates_every_session_under_one_deadline_and_is_idempotent() {
        let mut sup = supervisor_with(vec![
            MockChild::running(),
            MockChild::running(),
            MockChild::running(),
        ]);
        let ids: Vec<_> = (0..3).map(|_| sup.spawn().expect("spawn")).collect();
        assert_eq!(sup.running_count(), 3);

        let results = sup.shutdown_all();
        assert_eq!(results.len(), 3);
        assert_eq!(results.iter().map(|(id, _)| *id).collect::<Vec<_>>(), ids);
        for (_, status) in &results {
            assert_ne!(*status, SessionStatus::Running);
        }
        assert_eq!(sup.running_count(), 0);
        assert_eq!(sup.selected(), None);

        // Idempotent: a second pass keeps everything terminal.
        let again = sup.shutdown_all();
        assert_eq!(again.len(), 3);
        assert_eq!(sup.running_count(), 0);
    }

    #[test]
    fn select_refuses_dead_and_unknown() {
        let (child, ctrl) = MockChild::running_with_control();
        let mut sup = supervisor_with(vec![child]);
        let id = sup.spawn().expect("spawn");
        // Unknown id.
        assert_eq!(
            sup.select(super::SessionId(999)),
            Err(SessionOpError::Unknown)
        );
        // Live session is selectable.
        sup.select(id).expect("select live");
        // Once dead, selecting it again surfaces NotRunning, not a stuck focus.
        ctrl.exit(Some(0));
        let _ = sup.poll();
        assert_eq!(sup.select(id), Err(SessionOpError::NotRunning));
    }

    #[test]
    fn forget_requires_terminal_and_removes_record() {
        let (child, ctrl) = MockChild::running_with_control();
        let mut sup = supervisor_with(vec![child]);
        let id = sup.spawn().expect("spawn");
        assert_eq!(sup.forget(id), Err(SessionOpError::StillRunning));
        ctrl.exit(Some(0));
        let _ = sup.poll();
        sup.forget(id).expect("forget terminal");
        assert_eq!(sup.status(id), None);
        assert!(sup.is_empty());
        assert_eq!(sup.forget(id), Err(SessionOpError::Unknown));
    }

    #[test]
    fn spawn_failure_records_no_session() {
        let mut sup = supervisor_with(vec![]); // empty dispenser -> SpawnFailed
        let result = sup.spawn();
        assert_eq!(result, Err(SessionFailure::SpawnFailed));
        assert!(sup.is_empty());
        assert_eq!(sup.selected(), None);
    }

    #[test]
    fn displays_carry_no_child_content() {
        assert_eq!(
            format!(
                "{}",
                SessionStatus::Failed {
                    reason: SessionFailure::SpawnFailed
                }
            ),
            "failed(spawn-failed)"
        );
        assert_eq!(format!("{}", super::SessionId(4)), "session#4");
        let err = ChildError::Shutdown(ShutdownError::TimedOut);
        assert_eq!(format!("{err}"), "child shutdown timed out");
    }
}
