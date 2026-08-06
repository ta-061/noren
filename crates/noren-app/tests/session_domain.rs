//! Domain tests for the session model (`src/session.rs`).
//!
//! The session module is not yet wired into `noren-app`'s `lib.rs` — that
//! happens in a later serial integration commit — so this target compiles it
//! standalone with `#[path]`. When the module is re-exported from the crate,
//! this line is replaced by `use noren_app::session;`.
//!
//! These tests pin the four invariants the model must hold, and assert that the
//! public types match the D-M3-001 contract shape:
//!
//! 1. at most one selected session, and closing it never dangles;
//! 2. the registry spawns no process (the tests run no children);
//! 3. status is only set from a reported observation, never inferred from create;
//! 4. repeated create/close does not grow live state.

#[path = "../src/session.rs"]
mod session;

use session::{
    SelectedSession, SessionAction, SessionDescriptor, SessionError, SessionEvent, SessionKind,
    SessionRegistry, SessionStatus,
};
use std::path::PathBuf;

/// Build a fresh local session and return its descriptor.
fn fresh(registry: &mut SessionRegistry) -> SessionDescriptor {
    let id = registry.create(SessionKind::Local);
    registry.get(id).expect("just-created session is live")
}

// ── Invariant 1: at most one selected session ───────────────────────────

#[test]
fn selecting_replaces_the_prior_selection() {
    let mut registry = SessionRegistry::new();
    let first = fresh(&mut registry);
    let second = fresh(&mut registry);

    registry.select(first.id()).unwrap();
    assert_eq!(registry.selected(), Some(first.id()));

    registry.select(second.id()).unwrap();
    assert_eq!(registry.selected(), Some(second.id()));
}

#[test]
fn selected_is_the_contract_type_alias() {
    // SelectedSession is `Option<SessionId>` per D-M3-001, so `selected()`
    // returns the id directly (not a wrapper struct).
    let mut registry = SessionRegistry::new();
    let session = fresh(&mut registry);
    registry.select(session.id()).unwrap();
    let selected: SelectedSession = registry.selected();
    assert_eq!(selected, Some(session.id()));
}

#[test]
fn selecting_an_unknown_session_errors() {
    let mut registry = SessionRegistry::new();
    let live = fresh(&mut registry);
    registry.close(live.id()).unwrap();

    let unknown = registry.create(SessionKind::Local);
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
    assert_eq!(registry.selected(), Some(session.id()));
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
    assert_eq!(registry.selected(), Some(selected.id()));
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
    let a = registry.create(SessionKind::Local);
    let b = registry.create(SessionKind::Local);

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
fn a_newly_created_session_is_starting_not_running() {
    let mut registry = SessionRegistry::new();
    let session = fresh(&mut registry);
    assert_eq!(
        session.status(),
        &SessionStatus::Starting,
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
        &SessionStatus::Running
    );
}

#[test]
fn observe_records_failure_and_exit_statuses_with_payloads() {
    let mut registry = SessionRegistry::new();
    let session = fresh(&mut registry);

    registry
        .observe(
            session.id(),
            SessionStatus::Failed {
                reason: "exit 1".to_owned(),
            },
        )
        .unwrap();
    assert_eq!(
        registry.get(session.id()).unwrap().status(),
        &SessionStatus::Failed {
            reason: "exit 1".to_owned()
        }
    );

    registry
        .observe(session.id(), SessionStatus::Exited { code: Some(0) })
        .unwrap();
    assert_eq!(
        registry.get(session.id()).unwrap().status(),
        &SessionStatus::Exited { code: Some(0) }
    );
}

#[test]
fn observing_the_current_status_is_a_no_op() {
    let mut registry = SessionRegistry::new();
    let session = fresh(&mut registry);
    registry
        .observe(session.id(), SessionStatus::Running)
        .unwrap();

    // Re-observing the same status returns no event and changes nothing.
    let event = registry
        .observe(session.id(), SessionStatus::Running)
        .unwrap();
    assert_eq!(event, None);
    assert_eq!(
        registry.get(session.id()).unwrap().status(),
        &SessionStatus::Running
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
    let id = registry.create(SessionKind::Local);

    // After create: Starting.
    assert_eq!(registry.get(id).unwrap().status(), &SessionStatus::Starting);
    // After observe: Running.
    registry.observe(id, SessionStatus::Running).unwrap();
    assert_eq!(registry.get(id).unwrap().status(), &SessionStatus::Running);
    // After a failed observation: Failed.
    registry
        .observe(
            id,
            SessionStatus::Failed {
                reason: "boom".to_owned(),
            },
        )
        .unwrap();
    assert_eq!(
        registry.get(id).unwrap().status(),
        &SessionStatus::Failed {
            reason: "boom".to_owned()
        }
    );
    // Close removes it; the status never gets "inferred back".
    registry.close(id).unwrap();
    assert_eq!(registry.get(id), None);
}

// ── Invariant 4: repeated create/close does not grow live state ─────────

#[test]
fn repeated_create_close_cycles_do_not_accumulate() {
    let mut registry = SessionRegistry::new();
    for _ in 0..1000 {
        let id = registry.create(SessionKind::Local);
        registry.close(id).unwrap();
    }
    assert_eq!(registry.len(), 0);
    assert!(registry.sessions().is_empty());
}

#[test]
fn a_recreated_session_gets_a_fresh_distinct_id() {
    let mut registry = SessionRegistry::new();
    let first = registry.create(SessionKind::Local);
    registry.close(first).unwrap();

    let second = registry.create(SessionKind::Local);
    assert_ne!(first, second, "ids must never collide");
    assert_eq!(registry.len(), 1);
}

#[test]
fn close_is_idempotent_in_state_only_second_close_errors() {
    let mut registry = SessionRegistry::new();
    let id = registry.create(SessionKind::Local);

    registry.close(id).unwrap();
    assert_eq!(registry.close(id), Err(SessionError::UnknownSession));
    assert_eq!(registry.len(), 0);
}

// ── Event correctness for the reducer API (D-M3-001 tuple shape) ────────

#[test]
fn apply_create_emits_a_created_event_with_the_id() {
    let mut registry = SessionRegistry::new();
    let events = registry
        .apply(SessionAction::Create {
            kind: SessionKind::Local,
        })
        .unwrap();
    // Contract event shape: Created(SessionId).
    let [SessionEvent::Created(id)] = events.as_slice() else {
        panic!("expected exactly one Created event, got {events:?}");
    };
    assert!(registry.get(*id).is_some());
}

#[test]
fn apply_close_of_selected_emits_closed_then_selected_none() {
    let mut registry = SessionRegistry::new();
    let id = registry.create(SessionKind::Local);
    registry.select(id).unwrap();

    let events = registry.apply(SessionAction::Close { id }).unwrap();
    assert_eq!(
        events.as_slice(),
        &[SessionEvent::Closed(id), SessionEvent::Selected(None)]
    );
}

#[test]
fn apply_close_of_non_selected_emits_only_closed() {
    let mut registry = SessionRegistry::new();
    let selected = registry.create(SessionKind::Local);
    let other = registry.create(SessionKind::Local);
    registry.select(selected).unwrap();

    let events = registry.apply(SessionAction::Close { id: other }).unwrap();
    assert_eq!(events.as_slice(), &[SessionEvent::Closed(other)]);
}

#[test]
fn apply_select_emits_selected_some() {
    let mut registry = SessionRegistry::new();
    let id = registry.create(SessionKind::Local);

    let events = registry.apply(SessionAction::Select { id }).unwrap();
    assert_eq!(events.as_slice(), &[SessionEvent::Selected(Some(id))]);
}

#[test]
fn observe_emits_status_changed_only_when_it_differs() {
    let mut registry = SessionRegistry::new();
    let id = registry.create(SessionKind::Local);

    // A real change yields the contract StatusChanged { id, status } event.
    let changed = registry.observe(id, SessionStatus::Running).unwrap();
    assert_eq!(
        changed,
        Some(SessionEvent::StatusChanged {
            id,
            status: SessionStatus::Running,
        })
    );

    // Re-observing the same status yields nothing.
    let unchanged = registry.observe(id, SessionStatus::Running).unwrap();
    assert_eq!(unchanged, None);
}

#[test]
fn apply_against_an_unknown_session_errors() {
    let mut registry = SessionRegistry::new();
    let unknown = {
        let id = registry.create(SessionKind::Local);
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
    // Observation is a registry method, not an action; it errors the same way.
    assert_eq!(
        registry.observe(unknown, SessionStatus::Running),
        Err(SessionError::UnknownSession)
    );
}

#[test]
fn session_action_has_exactly_the_three_contract_variants() {
    // D-M3-001 fixes SessionAction to {Create, Select, Close}. These
    // constructions compile only while that shape holds; if a variant is added
    // or renamed, this test fails to build.
    let mut registry = SessionRegistry::new();
    let id = registry.create(SessionKind::Local);
    let _create = SessionAction::Create {
        kind: SessionKind::Local,
    };
    let _select = SessionAction::Select { id };
    let _close = SessionAction::Close { id };
}

#[test]
fn session_event_matches_the_contract_variants() {
    // D-M3-001 fixes SessionEvent to Created/Selected/StatusChanged/Closed.
    // These constructors compile only while that shape holds.
    let mut registry = SessionRegistry::new();
    let id = registry.create(SessionKind::Local);
    let _ = [
        SessionEvent::Created(id),
        SessionEvent::Selected(Some(id)),
        SessionEvent::StatusChanged {
            id,
            status: SessionStatus::Running,
        },
        SessionEvent::Closed(id),
    ];
}

// ── Descriptor and query surface ────────────────────────────────────────

#[test]
fn sessions_are_listed_in_identifier_order() {
    let mut registry = SessionRegistry::new();
    let c = registry.create(SessionKind::Local);
    let a = registry.create(SessionKind::Local);
    let b = registry.create(SessionKind::Local);
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
fn descriptors_expose_kind_title_and_status() {
    let mut registry = SessionRegistry::new();
    let id = registry.create(SessionKind::Local);
    registry.observe(id, SessionStatus::Running).unwrap();

    let descriptor = registry.get(id).unwrap();
    assert_eq!(descriptor.id(), id);
    assert_eq!(descriptor.kind(), &SessionKind::Local);
    assert_eq!(descriptor.status(), &SessionStatus::Running);
    // Title is the generated stable display id ("session-1" for the first id).
    assert_eq!(descriptor.title(), "session-1");
}

#[test]
fn a_descriptor_has_a_generated_title_for_every_kind() {
    let mut registry = SessionRegistry::new();
    let local = registry.create(SessionKind::Local);
    let project = registry.create(SessionKind::Project {
        root: PathBuf::from("/code/noren"),
    });
    let worktree = registry.create(SessionKind::Worktree {
        path: PathBuf::from("/code/noren-wt"),
    });
    let ssh = registry.create(SessionKind::Ssh {
        target: "dev@example.com".to_owned(),
    });
    let agent = registry.create(SessionKind::Agent {
        name: "glm".to_owned(),
    });

    // Title is always a non-empty generated String, regardless of kind, since
    // the contract Create action carries no title.
    for id in [local, project, worktree, ssh, agent] {
        let descriptor = registry.get(id).expect("live session");
        assert!(
            !descriptor.title().is_empty(),
            "title must be generated for every kind"
        );
    }
}

// ── Reserved session kinds ──────────────────────────────────────────────

#[test]
fn reserved_kinds_can_be_bookkept_but_are_not_launchable() {
    let mut registry = SessionRegistry::new();
    let project = registry.create(SessionKind::Project {
        root: PathBuf::from("/p"),
    });
    let worktree = registry.create(SessionKind::Worktree {
        path: PathBuf::from("/w"),
    });
    let ssh = registry.create(SessionKind::Ssh {
        target: "host".to_owned(),
    });
    let agent = registry.create(SessionKind::Agent {
        name: "a".to_owned(),
    });

    // The registry accepts reserved shapes as entries (bookkeeping only); it
    // never launches anything, so this stays pure data.
    assert!(!registry.get(project).unwrap().kind().is_launchable());
    assert!(!registry.get(worktree).unwrap().kind().is_launchable());
    assert!(!registry.get(ssh).unwrap().kind().is_launchable());
    assert!(!registry.get(agent).unwrap().kind().is_launchable());
    let local = registry.create(SessionKind::Local);
    assert!(registry.get(local).unwrap().kind().is_launchable());

    // Reserved kinds still obey the same lifecycle and selection rules.
    registry.select(ssh).unwrap();
    assert_eq!(registry.selected(), Some(ssh));
    registry.close(ssh).unwrap();
    assert_eq!(registry.selected(), None);
    assert_eq!(registry.len(), 4);
}
