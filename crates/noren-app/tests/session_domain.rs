//! Domain tests for the session model (`src/session.rs`).
//!
//! The session module is not yet wired into `noren-app`'s `lib.rs` — that
//! happens in a later serial integration commit — so this target compiles it
//! standalone with `#[path]`. When the module is re-exported from the crate,
//! this line is replaced by `use noren_app::session;`.
//!
//! These tests pin the four invariants the model must hold:
//!
//! 1. at most one selected session, and closing it never dangles;
//! 2. the registry spawns no process (the tests run no children);
//! 3. status is only set from a reported observation, never inferred from create;
//! 4. repeated create/close does not grow live state.

#[path = "../src/session.rs"]
mod session;

use session::{
    SessionAction, SessionDescriptor, SessionError, SessionEvent, SessionKind, SessionRegistry,
    SessionStatus,
};

/// Build a fresh local session and return its id.
fn fresh(registry: &mut SessionRegistry) -> SessionDescriptor {
    let id = registry.create(SessionKind::Local, None);
    registry.get(id).expect("just-created session is live")
}

// ── Invariant 1: at most one selected session ───────────────────────────

#[test]
fn selecting_replaces_the_prior_selection() {
    let mut registry = SessionRegistry::new();
    let first = fresh(&mut registry);
    let second = fresh(&mut registry);

    registry.select(first.id()).unwrap();
    assert_eq!(registry.selected().map(|s| s.id()), Some(first.id()));

    registry.select(second.id()).unwrap();
    assert_eq!(registry.selected().map(|s| s.id()), Some(second.id()));
    // The prior selection is gone, not retained alongside.
    assert_eq!(registry.selected().unwrap().id(), second.id());
}

#[test]
fn selecting_an_unknown_session_errors() {
    let mut registry = SessionRegistry::new();
    let live = fresh(&mut registry);
    registry.close(live.id()).unwrap();

    let unknown = registry.create(SessionKind::Local, None);
    registry.close(unknown).unwrap();
    assert_eq!(registry.select(unknown), Err(SessionError::UnknownSession));
    assert_eq!(registry.selected(), None);
}

#[test]
fn selecting_the_already_selected_session_is_a_no_op() {
    let mut registry = SessionRegistry::new();
    let session = fresh(&mut registry);
    registry.select(session.id()).unwrap();

    let events = registry
        .apply(SessionAction::Select { id: session.id() })
        .expect("re-selecting a live session is valid");
    assert!(events.is_empty(), "no-op select emits no events");
    assert_eq!(registry.selected().map(|s| s.id()), Some(session.id()));
}

// ── Invariant 1 (continued): closing the selected never dangles ─────────

#[test]
fn closing_the_selected_session_clears_the_selection() {
    let mut registry = SessionRegistry::new();
    let selected = fresh(&mut registry);
    let _other = fresh(&mut registry);
    registry.select(selected.id()).unwrap();

    registry.close(selected.id()).unwrap();
    // Selection is empty (never a dangling id).
    assert_eq!(registry.selected(), None);
}

#[test]
fn closing_a_non_selected_session_keeps_the_selection() {
    let mut registry = SessionRegistry::new();
    let selected = fresh(&mut registry);
    let other = fresh(&mut registry);
    registry.select(selected.id()).unwrap();

    registry.close(other.id()).unwrap();
    assert_eq!(registry.selected().map(|s| s.id()), Some(selected.id()));
    // The selected id still resolves to a live entry.
    assert!(registry.get(selected.id()).is_some());
}

#[test]
fn closing_the_only_session_leaves_no_selection() {
    let mut registry = SessionRegistry::new();
    let only = fresh(&mut registry);
    registry.select(only.id()).unwrap();

    registry.close(only.id()).unwrap();
    assert!(registry.is_empty());
    assert_eq!(registry.selected(), None);
}

#[test]
fn selected_descriptor_matches_get_descriptor() {
    let mut registry = SessionRegistry::new();
    let session = fresh(&mut registry);
    registry
        .observe(session.id(), SessionStatus::Running)
        .unwrap();
    registry.select(session.id()).unwrap();

    let selected = registry.selected().expect("a session is selected");
    assert_eq!(selected.descriptor(), &registry.get(session.id()).unwrap());
}

// ── Invariant 2: the registry spawns no process ─────────────────────────
//
// There is no assertion to make because there is nothing to observe: creating,
// observing, selecting, and closing sessions never touches the process table.
// This test exists to document the contract and to exercise a full lifecycle
// purely in memory — if any step had to spawn or wait on a child, it would fail
// in an environment with no PTY machinery.

#[test]
fn a_full_session_lifecycle_runs_without_any_child_process() {
    let mut registry = SessionRegistry::new();
    let a = registry.create(SessionKind::Local, Some("edit".to_owned()));
    let b = registry.create(SessionKind::Local, None);

    registry.observe(a, SessionStatus::Running).unwrap();
    registry.observe(b, SessionStatus::Running).unwrap();
    registry.select(a).unwrap();

    assert_eq!(registry.len(), 2);
    registry.close(b).unwrap();
    registry.close(a).unwrap();
    assert!(registry.is_empty());
    assert_eq!(registry.selected(), None);
}

// ── Invariant 3: status is observed, not inferred from create ───────────

#[test]
fn a_newly_created_session_is_created_not_running() {
    let mut registry = SessionRegistry::new();
    let session = fresh(&mut registry);
    assert_eq!(
        session.status(),
        SessionStatus::Created,
        "create must not infer a running status"
    );
}

#[test]
fn observe_advances_status_to_running() {
    let mut registry = SessionRegistry::new();
    let session = fresh(&mut registry);

    registry
        .observe(session.id(), SessionStatus::Running)
        .unwrap();
    assert_eq!(
        registry.get(session.id()).unwrap().status(),
        SessionStatus::Running
    );
}

#[test]
fn observe_records_failure_and_exit_statuses() {
    let mut registry = SessionRegistry::new();
    let session = fresh(&mut registry);

    registry
        .observe(session.id(), SessionStatus::Failed)
        .unwrap();
    assert_eq!(
        registry.get(session.id()).unwrap().status(),
        SessionStatus::Failed
    );

    registry
        .observe(session.id(), SessionStatus::Exited)
        .unwrap();
    assert_eq!(
        registry.get(session.id()).unwrap().status(),
        SessionStatus::Exited
    );
}

#[test]
fn observing_the_current_status_is_a_no_op() {
    let mut registry = SessionRegistry::new();
    let session = fresh(&mut registry);
    registry
        .observe(session.id(), SessionStatus::Running)
        .unwrap();

    let events = registry
        .apply(SessionAction::Observe {
            id: session.id(),
            status: SessionStatus::Running,
        })
        .unwrap();
    assert!(events.is_empty());
    assert_eq!(
        registry.get(session.id()).unwrap().status(),
        SessionStatus::Running
    );
}

#[test]
fn observe_on_an_unknown_session_errors() {
    let mut registry = SessionRegistry::new();
    let gone = {
        let session = fresh(&mut registry);
        let id = session.id();
        registry.close(id).unwrap();
        id
    };
    assert_eq!(
        registry.observe(gone, SessionStatus::Running),
        Err(SessionError::UnknownSession)
    );
}

#[test]
fn create_then_observe_then_close_keeps_status_observed_only() {
    let mut registry = SessionRegistry::new();
    let id = registry.create(SessionKind::Local, None);

    // After create: Created.
    assert_eq!(registry.get(id).unwrap().status(), SessionStatus::Created);
    // After observe: Running.
    registry.observe(id, SessionStatus::Running).unwrap();
    assert_eq!(registry.get(id).unwrap().status(), SessionStatus::Running);
    // After a failed observation: Failed.
    registry.observe(id, SessionStatus::Failed).unwrap();
    assert_eq!(registry.get(id).unwrap().status(), SessionStatus::Failed);
    // Close removes it; the status never gets "inferred back".
    registry.close(id).unwrap();
    assert_eq!(registry.get(id), None);
}

// ── Invariant 4: repeated create/close does not grow live state ─────────

#[test]
fn repeated_create_close_cycles_do_not_accumulate() {
    let mut registry = SessionRegistry::new();
    for _ in 0..1000 {
        let id = registry.create(SessionKind::Local, None);
        registry.close(id).unwrap();
    }
    assert_eq!(registry.len(), 0);
    assert!(registry.sessions().is_empty());
}

#[test]
fn a_recreated_session_gets_a_fresh_distinct_id() {
    let mut registry = SessionRegistry::new();
    let first = registry.create(SessionKind::Local, None);
    registry.close(first).unwrap();

    let second = registry.create(SessionKind::Local, None);
    assert_ne!(first, second, "ids must never collide");
    assert_eq!(registry.len(), 1);
}

#[test]
fn close_is_idempotent_in_state_only_second_close_errors() {
    let mut registry = SessionRegistry::new();
    let id = registry.create(SessionKind::Local, None);

    registry.close(id).unwrap();
    assert_eq!(registry.close(id), Err(SessionError::UnknownSession));
    assert_eq!(registry.len(), 0);
}

// ── Event correctness for the reducer API ───────────────────────────────

#[test]
fn apply_create_emits_a_created_event_with_the_descriptor() {
    let mut registry = SessionRegistry::new();
    let events = registry
        .apply(SessionAction::Create {
            kind: SessionKind::Local,
            label: Some("shell".to_owned()),
        })
        .unwrap();
    let [SessionEvent::Created { id, descriptor }] = events.as_slice() else {
        panic!("expected exactly one Created event, got {events:?}");
    };
    assert_eq!(descriptor.id(), *id);
    assert_eq!(descriptor.kind(), SessionKind::Local);
    assert_eq!(descriptor.status(), SessionStatus::Created);
    assert_eq!(descriptor.label(), Some("shell"));
    assert_eq!(registry.get(*id), Some(descriptor.clone()));
}

#[test]
fn apply_close_of_selected_emits_closed_then_selection_cleared() {
    let mut registry = SessionRegistry::new();
    let id = registry.create(SessionKind::Local, None);
    registry.select(id).unwrap();

    let events = registry.apply(SessionAction::Close { id }).unwrap();
    assert_eq!(
        events.as_slice(),
        &[
            SessionEvent::Closed { id },
            SessionEvent::SelectionChanged { selected: None },
        ]
    );
}

#[test]
fn apply_close_of_non_selected_emits_only_closed() {
    let mut registry = SessionRegistry::new();
    let selected = registry.create(SessionKind::Local, None);
    let other = registry.create(SessionKind::Local, None);
    registry.select(selected).unwrap();

    let events = registry.apply(SessionAction::Close { id: other }).unwrap();
    assert_eq!(events.as_slice(), &[SessionEvent::Closed { id: other }]);
}

#[test]
fn apply_observe_emits_status_changed_only_when_it_differs() {
    let mut registry = SessionRegistry::new();
    let id = registry.create(SessionKind::Local, None);

    let changed = registry
        .apply(SessionAction::Observe {
            id,
            status: SessionStatus::Running,
        })
        .unwrap();
    assert_eq!(
        changed.as_slice(),
        &[SessionEvent::StatusChanged {
            id,
            status: SessionStatus::Running,
        }]
    );

    let unchanged = registry
        .apply(SessionAction::Observe {
            id,
            status: SessionStatus::Running,
        })
        .unwrap();
    assert!(unchanged.is_empty());
}

#[test]
fn apply_select_emits_selection_changed() {
    let mut registry = SessionRegistry::new();
    let id = registry.create(SessionKind::Local, None);

    let events = registry.apply(SessionAction::Select { id }).unwrap();
    assert_eq!(
        events.as_slice(),
        &[SessionEvent::SelectionChanged { selected: Some(id) }]
    );
}

#[test]
fn apply_against_an_unknown_session_errors() {
    let mut registry = SessionRegistry::new();
    let unknown = {
        let id = registry.create(SessionKind::Local, None);
        registry.close(id).unwrap();
        id
    };
    assert_eq!(
        registry.apply(SessionAction::Close { id: unknown }),
        Err(SessionError::UnknownSession)
    );
    assert_eq!(
        registry.apply(SessionAction::Select { id: unknown }),
        Err(SessionError::UnknownSession)
    );
    assert_eq!(
        registry.apply(SessionAction::Observe {
            id: unknown,
            status: SessionStatus::Running,
        }),
        Err(SessionError::UnknownSession)
    );
}

// ── Descriptor and query surface ────────────────────────────────────────

#[test]
fn sessions_are_listed_in_identifier_order() {
    let mut registry = SessionRegistry::new();
    let c = registry.create(SessionKind::Local, None);
    let a = registry.create(SessionKind::Local, None);
    let b = registry.create(SessionKind::Local, None);
    // Creation order is c < a < b by minted id; listing is by id.
    assert!(a < b);
    assert!(c < a);

    let ids: Vec<_> = registry
        .sessions()
        .iter()
        .map(SessionDescriptor::id)
        .collect();
    assert_eq!(ids, vec![c, a, b]);
}

#[test]
fn descriptors_expose_kind_label_and_status() {
    let mut registry = SessionRegistry::new();
    let id = registry.create(SessionKind::Local, Some("main".to_owned()));
    registry.observe(id, SessionStatus::Running).unwrap();

    let descriptor = registry.get(id).unwrap();
    assert_eq!(descriptor.id(), id);
    assert_eq!(descriptor.kind(), SessionKind::Local);
    assert_eq!(descriptor.label(), Some("main"));
    assert_eq!(descriptor.status(), SessionStatus::Running);
}

#[test]
fn a_labelless_session_descriptor_reports_none() {
    let mut registry = SessionRegistry::new();
    let id = registry.create(SessionKind::Local, None);
    assert_eq!(registry.get(id).unwrap().label(), None);
}

// ── Reserved session kinds ──────────────────────────────────────────────

#[test]
fn reserved_kinds_can_be_bookkept_but_are_not_launchable() {
    let mut registry = SessionRegistry::new();
    let ssh = registry.create(SessionKind::Ssh, None);
    let agent = registry.create(SessionKind::Agent, None);

    // The registry accepts reserved shapes as entries (bookkeeping only); it
    // never launches anything, so this stays pure data.
    assert!(!registry.get(ssh).unwrap().kind().is_launchable());
    assert!(!registry.get(agent).unwrap().kind().is_launchable());

    // They still obey the same lifecycle and selection rules.
    registry.select(ssh).unwrap();
    assert_eq!(registry.selected().map(|s| s.id()), Some(ssh));
    registry.close(ssh).unwrap();
    assert_eq!(registry.selected(), None);
    assert_eq!(registry.len(), 1);
}
