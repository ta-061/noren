//! Independent verification of the session-supervisor lane (M3-1b).
//!
//! This target is written as an outside-in check of the public contract the
//! task describes, against a fake/mock process model. The module under review
//! is **not** wired into `noren-app/src/lib.rs` yet (wiring is a later serial
//! integration commit), so it is compiled into this test binary via `#[path]`.
//! `cfg(test)` is enabled for the test binary, which also brings the lane's
//! `mock` harness into scope.
//!
//! The load-bearing claim under test — the reason this lane exists — is that a
//! dead child surfaces as `Exited`/`Failed` and never stays `Running`, that
//! termination reaps and is idempotent under one deadline, and that the
//! supervisor (not a registry) owns the child handle.

#![forbid(unsafe_code)]

#[path = "../src/session_supervisor.rs"]
mod session_supervisor;

use session_supervisor::SessionSupervisor;
use session_supervisor::mock::{MockChild, MockSpawner};
use session_supervisor::{
    ChildError, SessionFailure, SessionOpError, SessionStatus, ShutdownError,
};
use std::time::{Duration, Instant};

/// Build a supervisor whose injected spawner dispenses `children` in order.
fn supervisor_with(children: Vec<MockChild>) -> SessionSupervisor {
    SessionSupervisor::new(Box::new(MockSpawner::from_children(children)))
}

// ── Claim 1: a dead child never remains Running ──────────────────────────

#[test]
fn unprompted_death_surfaces_as_exited_within_one_poll() {
    let (child, ctrl) = MockChild::running_with_control();
    let mut sup = supervisor_with(vec![child]);
    let id = sup.spawn().expect("spawn");

    // Before death: Running and selected.
    assert_eq!(sup.status(id), Some(SessionStatus::Running));
    assert_eq!(sup.selected(), Some(id));

    // The child exits on its own (stale/dead case). One poll must surface it.
    ctrl.exit(Some(42));
    let report = sup.poll();
    assert_eq!(report.exited(), &[(id, Some(42))]);
    assert_eq!(
        sup.status(id),
        Some(SessionStatus::Exited { code: Some(42) })
    );
    assert_ne!(sup.status(id), Some(SessionStatus::Running));
    // Selection over a dead session is dropped, not frozen on it.
    assert_eq!(sup.selected(), None);
}

#[test]
fn a_poll_error_kills_the_session_as_failed_not_running() {
    let (child, ctrl) = MockChild::running_with_control();
    let mut sup = supervisor_with(vec![child]);
    let id = sup.spawn().expect("spawn");
    ctrl.fail_poll();
    let _ = sup.poll();
    assert_eq!(
        sup.status(id),
        Some(SessionStatus::Failed {
            reason: SessionFailure::PollFailed
        })
    );
    assert_ne!(sup.status(id), Some(SessionStatus::Running));
}

// ── Claim 2: termination reaps and is idempotent under one deadline ───────

#[test]
fn terminate_reaps_and_second_call_is_a_no_op() {
    let (child, ctrl) = MockChild::running_with_control();
    let mut sup = supervisor_with(vec![child]);
    let id = sup.spawn().expect("spawn");

    let started = Instant::now();
    let status = sup.terminate(id, started + Duration::from_secs(1));
    assert!(matches!(status, SessionStatus::Exited { .. }));
    assert!(started.elapsed() <= Duration::from_secs(1));
    let shutdowns_after_first = ctrl.shutdown_count();
    assert_eq!(shutdowns_after_first, 1);
    let polls_after_first = ctrl.poll_count();

    // Idempotent: status stable, no further backend work or reaping polls.
    let again = sup.terminate_now(id);
    assert_eq!(again, status);
    assert_eq!(ctrl.shutdown_count(), shutdowns_after_first);
    assert_eq!(ctrl.poll_count(), polls_after_first);
}

#[test]
fn terminate_a_batch_under_one_shared_deadline() {
    let mut sup = supervisor_with(vec![
        MockChild::running(),
        MockChild::running(),
        MockChild::running(),
        MockChild::running(),
    ]);
    let ids: Vec<_> = (0..4).map(|_| sup.spawn().expect("spawn")).collect();
    assert_eq!(sup.running_count(), 4);

    let started = Instant::now();
    let results = sup.shutdown_all();
    let elapsed = started.elapsed();

    assert_eq!(results.len(), ids.len());
    assert_eq!(results.iter().map(|(id, _)| *id).collect::<Vec<_>>(), ids);
    for (_, status) in &results {
        assert!(matches!(
            status,
            SessionStatus::Exited { .. } | SessionStatus::Failed { .. }
        ));
        assert_ne!(*status, SessionStatus::Running);
    }
    // One shared deadline means the whole batch is bounded by it, not by
    // sessions * deadline. The mock backend is instant, so this is comfortably
    // under the 2s budget; the assertion guards against an n*deadline bug.
    assert!(elapsed <= session_supervisor::SHUTDOWN_DEADLINE);
    assert_eq!(sup.running_count(), 0);

    // Fully idempotent on a second pass.
    let again = sup.shutdown_all();
    assert_eq!(again.len(), ids.len());
    assert_eq!(sup.running_count(), 0);
}

// ── Claim 3: failure matrix — every fault path lands in a terminal state ──

#[test]
fn spawn_failure_records_nothing_and_reports_spawn_failed() {
    let mut sup = SessionSupervisor::new(Box::new(MockSpawner::failing()));
    assert_eq!(sup.spawn(), Err(SessionFailure::SpawnFailed));
    assert_eq!(sup.len(), 0);
    assert_eq!(sup.selected(), None);
}

#[test]
fn shutdown_hard_error_becomes_failed_shutdown_failed() {
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
fn shutdown_timeout_becomes_failed_reap_timeout_and_does_not_hang() {
    let (child, ctrl) = MockChild::running_with_control();
    let mut sup = supervisor_with(vec![child]);
    let id = sup.spawn().expect("spawn");
    ctrl.fail_shutdown(ShutdownError::TimedOut);
    let started = Instant::now();
    let status = sup.terminate_now(id);
    assert!(started.elapsed() <= session_supervisor::SHUTDOWN_DEADLINE);
    assert_eq!(
        status,
        SessionStatus::Failed {
            reason: SessionFailure::ReapTimeout
        }
    );
    // A reap-timeout child never becomes stuck Running.
    assert_ne!(sup.status(id), Some(SessionStatus::Running));
}

#[test]
fn an_already_passed_deadline_skips_the_backend_and_times_out() {
    let (child, ctrl) = MockChild::running_with_control();
    let mut sup = supervisor_with(vec![child]);
    let id = sup.spawn().expect("spawn");
    let status = sup.terminate(id, Instant::now());
    assert_eq!(
        status,
        SessionStatus::Failed {
            reason: SessionFailure::ReapTimeout
        }
    );
    assert_eq!(ctrl.shutdown_count(), 0);
}

// ── Claim 4: selection refuses a dead session (ownership observable by id) ─

#[test]
fn selection_cannot_focus_a_dead_session() {
    let (child, ctrl) = MockChild::running_with_control();
    let extra = MockChild::running();
    let mut sup = supervisor_with(vec![child, extra]);
    let live = sup.spawn().expect("spawn");
    let other = sup.spawn().expect("spawn other");

    // Live is focusable.
    sup.select(live).expect("select live");

    // Terminate + forget `other` so its id is genuinely unknown to the
    // supervisor (this avoids depending on the STUB id's private constructor).
    sup.terminate_now(other);
    sup.forget(other).expect("retire other");
    assert_eq!(sup.select(other), Err(SessionOpError::Unknown));

    // Once `live` dies, the same id is rejected as NotRunning — the death is
    // visible through the selection path, not hidden behind a stale focus.
    ctrl.exit(Some(0));
    let _ = sup.poll();
    assert_eq!(sup.select(live), Err(SessionOpError::NotRunning));
}

#[test]
fn clear_selection_is_independent_of_termination() {
    let (child, _ctrl) = MockChild::running_with_control();
    let mut sup = supervisor_with(vec![child]);
    let id = sup.spawn().expect("spawn");
    assert_eq!(sup.selected(), Some(id));
    sup.clear_selection();
    assert_eq!(sup.selected(), None);
    // The session is unaffected and still Running.
    assert_eq!(sup.status(id), Some(SessionStatus::Running));
}

// ── Claim 5: forget retires only terminal records ─────────────────────────

#[test]
fn forget_refuses_running_and_removes_terminal() {
    let (child, ctrl) = MockChild::running_with_control();
    let mut sup = supervisor_with(vec![child]);
    let id = sup.spawn().expect("spawn");
    assert_eq!(sup.forget(id), Err(SessionOpError::StillRunning));

    ctrl.exit(Some(9));
    let _ = sup.poll();
    sup.forget(id).expect("retire terminal");
    assert_eq!(sup.status(id), None);
    assert_eq!(sup.len(), 0);
    // Retiring an already-forgotten id is Unknown.
    assert_eq!(sup.forget(id), Err(SessionOpError::Unknown));
}

// ── Claim 6: error types are control-plane only and carry no child content ─

#[test]
fn error_and_status_displays_redact_all_child_content() {
    let cases: Vec<(String, &str)> = vec![
        (
            format!(
                "{}",
                SessionStatus::Failed {
                    reason: SessionFailure::ReapTimeout
                }
            ),
            "failed(reap-timeout)",
        ),
        (
            format!("{}", SessionStatus::Exited { code: None }),
            "exited",
        ),
        (
            format!("{}", ChildError::Shutdown(ShutdownError::Failed)),
            "child shutdown failed",
        ),
    ];
    for (rendered, expected) in cases {
        // No secret/child bytes leak into control-plane strings.
        assert!(rendered.len() < 64);
        assert_eq!(rendered, expected);
    }
}
