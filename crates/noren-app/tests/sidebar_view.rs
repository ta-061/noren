//! Task M3-3: sidebar view model and visual skeleton.
//!
//! These tests speak the leased public API the way the task describes. They
//! import the session contract types from the crate's `session` module, owned
//! by task M3-1a (see `docs/coordination/session-api.md`); the sidebar never
//! redefines them.

use std::collections::BTreeSet;

use noren_app::session::{
    SessionDescriptor, SessionId, SessionKind, SessionRegistry, SessionStatus,
};
use noren_app::sidebar::{
    EMPTY_SIDEBAR_MESSAGE, EmptyState, EntryKind, SessionLifecycle, SessionViewport, SidebarEntry,
    SidebarRow, SidebarView, fixtures,
};

fn fixture_ids(registry: &SessionRegistry) -> Vec<SessionId> {
    registry
        .sessions()
        .iter()
        .map(SessionDescriptor::id)
        .collect()
}

#[test]
fn each_entry_kind_maps_to_a_distinct_view_row() {
    let registry = fixtures::session_registry();
    let ids = fixture_ids(&registry);
    let view = SidebarView::build(&fixtures::entries(&registry), None);

    let kinds: Vec<EntryKind> = view.rows().iter().map(SidebarRow::kind).collect();
    assert_eq!(
        kinds,
        [
            EntryKind::Project,
            EntryKind::Worktree,
            EntryKind::SshConnection,
            EntryKind::Agent,
            EntryKind::Session,
            EntryKind::Session,
            EntryKind::Session,
        ]
    );

    let distinct: BTreeSet<EntryKind> = kinds.iter().copied().collect();
    assert_eq!(distinct.len(), 5, "every entry kind stays distinguishable");

    let labels: Vec<String> = view
        .rows()
        .iter()
        .map(|row| row.label().to_string())
        .collect();
    let mut expected: Vec<String> = ["noren", "pool-m3c", "web-1", "claude-code"]
        .iter()
        .map(|label| (*label).to_string())
        .collect();
    expected.extend(ids.iter().map(SessionId::to_string));
    assert_eq!(labels, expected);
    let distinct_labels: BTreeSet<String> = labels.iter().cloned().collect();
    assert_eq!(
        distinct_labels.len(),
        labels.len(),
        "rows never share a label"
    );

    let expected_rows = [
        (EntryKind::Project, Some("~/dev/noren")),
        (EntryKind::Worktree, Some("agent/m3-sidebar-view")),
        (EntryKind::SshConnection, Some("web1.internal:22")),
        (EntryKind::Agent, Some("not running")),
        (EntryKind::Session, Some("local · running")),
        (EntryKind::Session, Some("local · running")),
        (EntryKind::Session, Some("ssh · starting")),
    ];
    for (row, (kind, detail)) in view.rows().iter().zip(expected_rows) {
        assert_eq!(row.kind(), kind);
        assert_eq!(row.detail(), detail);
        assert!(
            !row.is_selected(),
            "nothing is selected without a selection"
        );
    }
}

#[test]
fn empty_sidebar_yields_an_empty_state_view_not_a_panic() {
    let view = SidebarView::build(&[], None);
    assert!(view.is_empty());
    assert!(view.rows().is_empty());
    assert_eq!(view.selected_row_count(), 0);
    assert_eq!(view.viewport(), None);
    let empty = view
        .empty_state()
        .expect("empty sidebar carries its notice");
    // Pin the literal text, not the constant. Comparing `message()` to
    // `EMPTY_SIDEBAR_MESSAGE` is a tautology: `build()` constructs the
    // `EmptyState` from that same constant (`sidebar.rs:256`), so the assertion
    // holds for any value the constant takes. A reviewer mutated the constant to
    // "MUTATED" and all ten tests still passed. The user-visible string is a
    // documented guarantee and needs a test that fails when it changes.
    assert_eq!(empty.message(), "No sessions");
    assert_eq!(
        EMPTY_SIDEBAR_MESSAGE, "No sessions",
        "the exported constant is part of the public contract"
    );
    assert_eq!(empty, &EmptyState::new("No sessions".to_string()));
}

#[test]
fn one_selected_session_among_many_describes_exactly_one_viewport() {
    let registry = fixtures::session_registry();
    let ids = fixture_ids(&registry);
    assert_eq!(ids.len(), 3);

    let view = SidebarView::build(&fixtures::entries(&registry), Some(ids[1]));

    let viewport: &SessionViewport = view.viewport().expect("the selected session is visible");
    assert_eq!(viewport.session_id(), ids[1]);
    assert_eq!(
        viewport.descriptor(),
        &registry.get(ids[1]).expect("fixture id is live")
    );
    assert_eq!(viewport.title(), ids[1].to_string());

    assert_eq!(view.selected_row_count(), 1, "exactly one row is selected");
    let selected_row = view
        .rows()
        .iter()
        .find(|row| row.is_selected())
        .expect("one selected row");
    assert_eq!(selected_row.kind(), EntryKind::Session);
    assert_eq!(selected_row.label(), ids[1].to_string());
}

#[test]
fn unselected_sessions_produce_no_viewport() {
    let registry = fixtures::session_registry();
    let ids = fixture_ids(&registry);

    let none = SidebarView::build(&fixtures::entries(&registry), None);
    assert_eq!(none.viewport(), None, "no selection describes no viewport");
    assert_eq!(none.selected_row_count(), 0);

    let selected = SidebarView::build(&fixtures::entries(&registry), Some(ids[1]));
    let visible = selected.viewport().map(SessionViewport::session_id);
    assert_eq!(visible, Some(ids[1]));
    assert_eq!(selected.selected_row_count(), 1);
    for unselected in [ids[0], ids[2]] {
        assert_ne!(visible, Some(unselected), "unselected sessions stay hidden");
    }
}

#[test]
fn a_selection_matching_no_entry_is_dropped_not_rendered() {
    let registry = fixtures::session_registry();
    let ids = fixture_ids(&registry);
    let project_only = [SidebarEntry::Project {
        name: "noren".to_string(),
        root: "~/dev/noren".to_string(),
    }];

    let view = SidebarView::build(&project_only, Some(ids[0]));
    assert_eq!(
        view.viewport(),
        None,
        "a dangling selection renders nothing"
    );
    assert_eq!(view.selected_row_count(), 0);
    assert_eq!(view.rows().len(), 1, "other rows still render");
    assert!(!view.is_empty());
    assert_eq!(view.empty_state(), None);
}

#[test]
fn duplicate_session_descriptions_keep_exactly_one_selection() {
    let registry = fixtures::session_registry();
    let ids = fixture_ids(&registry);
    let descriptor = registry.get(ids[0]).expect("fixture id is live");
    let entries = vec![
        SidebarEntry::Session(descriptor.clone()),
        SidebarEntry::Session(descriptor),
    ];

    let view = SidebarView::build(&entries, Some(ids[0]));
    assert_eq!(view.selected_row_count(), 1);
    assert!(view.rows()[0].is_selected());
    assert!(!view.rows()[1].is_selected());
    assert_eq!(
        view.viewport().map(SessionViewport::session_id),
        Some(ids[0])
    );
}

#[test]
fn session_rows_use_the_descriptor_title() {
    let registry = fixtures::session_registry();
    let ids = fixture_ids(&registry);
    let session = registry.get(ids[2]).expect("fixture id is live");
    assert_eq!(session.title(), ids[2].to_string());

    let entries = [SidebarEntry::Session(session)];
    let view = SidebarView::build(&entries, Some(ids[2]));
    assert_eq!(view.rows()[0].label(), ids[2].to_string());
    let viewport = view.viewport().expect("selected session is visible");
    assert_eq!(viewport.title(), ids[2].to_string());
}

#[test]
fn session_rows_report_observed_status() {
    let mut registry = fixtures::session_registry();
    let ids = fixture_ids(&registry);
    registry
        .observe(
            ids[0],
            SessionStatus::Failed {
                reason: "exit 1".to_string(),
            },
        )
        .expect("fixture id is live");

    let view = SidebarView::build(&fixtures::entries(&registry), None);
    let session_rows: Vec<&SidebarRow> = view
        .rows()
        .iter()
        .filter(|row| row.kind() == EntryKind::Session)
        .collect();
    assert_eq!(
        session_rows
            .iter()
            .map(|row| row.detail())
            .collect::<Vec<_>>(),
        [
            Some("local · failed"),
            Some("local · running"),
            Some("ssh · starting")
        ]
    );
    assert_eq!(
        session_rows
            .iter()
            .map(|row| row.lifecycle())
            .collect::<Vec<_>>(),
        [
            Some(SessionLifecycle::Failed),
            Some(SessionLifecycle::Running),
            Some(SessionLifecycle::Starting),
        ]
    );
}

#[test]
fn exited_and_restored_rows_share_the_truthful_non_running_marker_class() {
    let mut registry = SessionRegistry::new();
    let exited = registry.create(SessionKind::Local);
    registry
        .observe(exited, SessionStatus::Exited { code: Some(0) })
        .expect("starting may advance to exited");
    let _restored = registry.restore(SessionKind::Local);
    let view = SidebarView::build(
        &registry
            .sessions()
            .into_iter()
            .map(SidebarEntry::Session)
            .collect::<Vec<_>>(),
        None,
    );

    assert_eq!(view.rows().len(), 2);
    assert!(
        view.rows()
            .iter()
            .all(|row| row.lifecycle() == Some(SessionLifecycle::Exited))
    );
}

#[test]
fn sidebar_views_are_immutable_values() {
    let registry = fixtures::session_registry();
    let ids = fixture_ids(&registry);
    let entries = fixtures::entries(&registry);

    let first = SidebarView::build(&entries, Some(ids[1]));
    let second = SidebarView::build(&entries, Some(ids[1]));
    assert_eq!(first, second, "equal inputs describe equal views");
    assert_eq!(first.clone(), first, "snapshots clone to identical values");

    let changed = SidebarView::build(&entries, Some(ids[0]));
    assert_ne!(first, changed, "a different selection changes the view");
}

#[test]
fn non_session_entries_render_without_any_sessions() {
    let registry = SessionRegistry::new();
    assert!(registry.is_empty());
    let view = SidebarView::build(&fixtures::entries(&registry), None);
    assert_eq!(view.rows().len(), 4, "non-session entries still render");
    assert_eq!(view.viewport(), None);
    assert_eq!(view.empty_state(), None);
    assert!(!view.is_empty());
}

// ── Debug is shape-only (issue #146) ──
//
// The sidebar's payloads are user- or environment-derived text (project
// names and roots, branch names, SSH labels that embed the target, host
// details) plus shared session descriptors. These guards assert directly on
// `format!("{:?}")` — per PR #142, the guard must not depend on a
// production formatter existing — so the sentinel must not appear even
// while no production code path formats these types yet.

/// Unique content stand-in: any debug surface that prints it is a leak.
fn leak_sentinel() -> String {
    format!("NOREN-SIDEBAR-SENTINEL-{}", std::process::id())
}

/// One entry of every kind, each carrying the sentinel in every payload
/// field the type has, plus the id of the session entry.
fn sentinel_entries(sentinel: &str) -> (Vec<SidebarEntry>, SessionId) {
    let mut registry = SessionRegistry::new();
    let id = registry.create(SessionKind::Project {
        root: sentinel.parse().expect("sentinel parses as a path"),
    });
    registry
        .observe(
            id,
            SessionStatus::Failed {
                reason: sentinel.to_string(),
            },
        )
        .expect("a fresh session may be observed failed");
    let descriptor: SessionDescriptor = registry.get(id).expect("created id is live").clone();
    (
        vec![
            SidebarEntry::Project {
                name: sentinel.to_string(),
                root: sentinel.to_string(),
            },
            SidebarEntry::Worktree {
                name: sentinel.to_string(),
                branch: sentinel.to_string(),
            },
            SidebarEntry::SshConnection {
                label: sentinel.to_string(),
                host: sentinel.to_string(),
                selected: true,
            },
            SidebarEntry::Agent {
                label: sentinel.to_string(),
                status: sentinel.to_string(),
            },
            SidebarEntry::Session(descriptor),
        ],
        id,
    )
}

#[test]
fn entry_debug_reports_shape_without_content() {
    let sentinel = leak_sentinel();
    for entry in sentinel_entries(&sentinel).0 {
        let rendered = format!("{entry:?}");
        assert!(
            !rendered.contains(&sentinel),
            "entry debug leaked content: {rendered}"
        );
    }
}

#[test]
fn entry_debug_keeps_useful_shape() {
    let sentinel = leak_sentinel();
    let entries = sentinel_entries(&sentinel).0;
    let ssh = format!("{:?}", &entries[2]);
    assert!(ssh.contains("SshConnection"), "variant name: {ssh}");
    assert!(ssh.contains("selected: true"), "selection state: {ssh}");

    let mut registry = SessionRegistry::new();
    let id = registry.create(SessionKind::Local);
    let session = format!(
        "{:?}",
        SidebarEntry::Session(registry.get(id).expect("id live").clone())
    );
    assert!(session.contains("Session"), "variant name: {session}");
    assert!(
        session.contains(&format!("{id:?}")),
        "the registry-generated id is safe shape: {session}"
    );
    assert!(
        !session.contains("Local"),
        "the launch shape may carry paths or targets: {session}"
    );
}

#[test]
fn view_debug_reports_shape_without_content() {
    let sentinel = leak_sentinel();
    let (entries, session_id) = sentinel_entries(&sentinel);
    let view = SidebarView::build(&entries, Some(session_id));

    let rendered = format!("{view:?}");
    assert!(
        !rendered.contains(&sentinel),
        "view debug leaked content: {rendered}"
    );
    // Not vacuous: the shape a renderer bug report needs stays visible.
    assert!(rendered.contains("row_count: 5"), "{rendered}");
    assert!(rendered.contains("row_kinds: ["), "{rendered}");
    assert!(rendered.contains("\"ssh\""), "kind names: {rendered}");
    assert!(rendered.contains("selected_row: Some(2)"), "{rendered}");
    assert!(rendered.contains("empty_state: false"), "{rendered}");
    assert!(
        rendered.contains(&format!("viewport: Some({session_id:?})")),
        "the viewport id is safe shape: {rendered}"
    );
}
