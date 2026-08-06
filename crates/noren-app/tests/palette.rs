//! Independent verification of the M3-6 command palette model, written by the
//! GLM palette lane. These tests exercise the public surface the way a
//! renderer and the future wiring layer will: building the canonical catalog,
//! searching it, and resolving stable IDs to actions.
//!
//! The palette module is not yet wired into `lib.rs` (that export is a separate
//! serial commit owned by another lane), so the module is included here by
//! path. Once wired, the `#[path]` line is removed and these tests reach the
//! type through `noren_app::palette` instead.

#[path = "../src/palette.rs"]
mod palette;

use palette::{Command, CommandId, Palette};

/// A tiny stand-in for the shared action vocabulary the wiring layer will bind
/// to `A`. The palette is action-agnostic, so any owned type proves the action
/// is carried and recovered without the palette naming a session variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestAction {
    Do(&'static str),
}

fn catalog() -> Palette<TestAction> {
    Palette::noren(
        TestAction::Do("create"),
        TestAction::Do("select"),
        TestAction::Do("close"),
        TestAction::Do("sidebar"),
    )
}

// ── Boundary (ADR 0003): session + sidebar only ──────────────────────────

#[test]
fn canonical_catalog_has_exactly_four_commands() {
    let palette = catalog();
    assert_eq!(palette.len(), 4);
    assert!(!palette.is_empty());
    assert_eq!(
        palette.iter().map(|c| c.id().as_str()).collect::<Vec<_>>(),
        [
            "session.create",
            "session.select",
            "session.close",
            "sidebar.focus"
        ]
    );
}

#[test]
fn no_pane_tab_split_or_layout_command_is_offered() {
    let palette = catalog();
    for command in palette.iter() {
        let id = command.id().as_str();
        assert!(
            !id.contains("pane")
                && !id.contains("tab")
                && !id.contains("split")
                && !id.contains("layout"),
            "ADR 0003 boundary violated by command id {id:?}"
        );
        let label = command.label();
        let lower = label.to_ascii_lowercase();
        assert!(
            !lower.contains("pane")
                && !lower.contains(" tab")
                && !lower.contains("split")
                && !lower.contains("layout"),
            "ADR 0003 boundary violated by command label {label:?}"
        );
    }
}

// ── Stable IDs and labels ────────────────────────────────────────────────

#[test]
fn command_ids_are_stable_strings() {
    assert_eq!(CommandId::SESSION_CREATE.as_str(), "session.create");
    assert_eq!(CommandId::SESSION_SELECT.as_str(), "session.select");
    assert_eq!(CommandId::SESSION_CLOSE.as_str(), "session.close");
    assert_eq!(CommandId::SIDEBAR_FOCUS.as_str(), "sidebar.focus");
    assert_eq!(CommandId::SESSION_CREATE.to_string(), "session.create");
    assert_eq!(CommandId::SIDEBAR_FOCUS.as_ref(), "sidebar.focus");
}

#[test]
fn command_carries_and_releases_its_action() {
    let command = Command::new(CommandId::new("custom.x"), "Custom X", TestAction::Do("x"));
    assert_eq!(command.id(), CommandId::new("custom.x"));
    assert_eq!(command.label(), "Custom X");
    assert_eq!(command.action(), &TestAction::Do("x"));
    assert_eq!(command.into_action(), TestAction::Do("x"));
}

// ── Matching policy: ASCII case-insensitive substring ───────────────────

#[test]
fn empty_query_returns_every_command_in_catalog_order() {
    let palette = catalog();
    let hits = palette.search("");
    assert_eq!(hits.len(), 4);
    assert!(hits.iter().all(|hit| hit.match_index() == 0));
    assert_eq!(
        hits.iter().map(|h| h.command().id()).collect::<Vec<_>>(),
        [
            CommandId::SESSION_CREATE,
            CommandId::SESSION_SELECT,
            CommandId::SESSION_CLOSE,
            CommandId::SIDEBAR_FOCUS,
        ]
    );
}

#[test]
fn case_insensitive_substring_matches_a_subset_of_labels() {
    let palette = catalog();
    let hits = palette.search("SES");
    let ids: Vec<CommandId> = hits.iter().map(|h| h.command().id()).collect();
    // "ses" is a substring of "new session", "switch session", and
    // "close session" — but not "focus sidebar".
    assert_eq!(ids.len(), 3);
    assert!(ids.contains(&CommandId::SESSION_CREATE));
    assert!(ids.contains(&CommandId::SESSION_SELECT));
    assert!(ids.contains(&CommandId::SESSION_CLOSE));
    assert!(!ids.contains(&CommandId::SIDEBAR_FOCUS));
}

#[test]
fn no_match_query_returns_an_empty_result() {
    let palette = catalog();
    assert!(palette.search("xyzzy-no-such-command").is_empty());
}

#[test]
fn query_longer_than_every_label_returns_empty() {
    let palette = catalog();
    assert!(
        palette
            .search("an-absurdly-long-query-that-no-short-label-contains")
            .is_empty()
    );
}

#[test]
fn special_and_escaped_characters_are_matched_literally_without_panicking() {
    let palette = catalog();
    // A query made entirely of characters that are "special" to regex/glob is
    // just searched for verbatim: it matches nothing here and never panics.
    let hostile = r"\(\)\[\]\.\*\+\\";
    let result = std::panic::catch_unwind(|| palette.search(hostile));
    assert!(result.is_ok(), "literal special-char query must not panic");
    assert!(result.expect("search returned").is_empty());
}

// ── Ranking ─────────────────────────────────────────────────────────────

#[test]
fn ranking_prefers_earlier_match_position_then_catalog_order() {
    let palette = Palette::from_commands([
        Command::new(
            CommandId::new("a.late"),
            "Glyph Spotlight",
            TestAction::Do("late"),
        ),
        Command::new(
            CommandId::new("a.early"),
            "Spotlight Mode",
            TestAction::Do("early"),
        ),
        Command::new(
            CommandId::new("a.mid"),
            "the Spotlight here",
            TestAction::Do("mid"),
        ),
    ]);
    let hits = palette.search("spotlight");
    let ids: Vec<&str> = hits.iter().map(|h| h.command().id().as_str()).collect();
    // "Spotlight Mode" matches at 0, "the Spotlight here" at 4,
    // "Glyph Spotlight" at 6.
    assert_eq!(ids, ["a.early", "a.mid", "a.late"]);
    assert_eq!(hits[0].match_index(), 0);
    assert_eq!(hits[1].match_index(), 4);
    assert_eq!(hits[2].match_index(), 6);
}

#[test]
fn equal_match_positions_keep_catalog_order_under_stable_ranking() {
    let palette = Palette::from_commands([
        Command::new(CommandId::new("z.first"), "Session A", TestAction::Do("a")),
        Command::new(CommandId::new("z.second"), "Session B", TestAction::Do("b")),
    ]);
    let hits = palette.search("session");
    assert_eq!(
        hits.iter().map(|h| h.command().id()).collect::<Vec<_>>(),
        [CommandId::new("z.first"), CommandId::new("z.second")]
    );
    assert_eq!(hits[0].match_index(), 0);
}

// ── Dispatch lookup ─────────────────────────────────────────────────────

#[test]
fn get_resolves_a_known_id_to_its_command_and_action() {
    let palette = catalog();
    let command = palette.get(CommandId::SESSION_CLOSE).expect("present");
    assert_eq!(command.label(), "Close Session");
    assert_eq!(command.action(), &TestAction::Do("close"));
}

#[test]
fn get_returns_none_for_an_id_outside_the_boundary() {
    let palette = catalog();
    // A pane/tab/split/layout ID is deliberately absent: dispatch for any such
    // ID must report "unknown" rather than synthesize a command.
    assert!(
        palette
            .get(CommandId::new("pane.split.horizontal"))
            .is_none()
    );
    assert!(palette.get(CommandId::new("tab.new")).is_none());
}

#[test]
fn empty_palette_is_empty_and_searches_to_nothing() {
    let palette: Palette<TestAction> = Palette::default();
    assert!(palette.is_empty());
    assert_eq!(palette.len(), 0);
    assert!(palette.search("").is_empty());
    assert!(palette.search("session").is_empty());
    assert!(palette.get(CommandId::SESSION_CREATE).is_none());
}
