//! Behavioral tests for sidebar persistence (`src/session_persistence.rs`).
//!
//! The persistence module is not yet wired into `noren-app`'s `lib.rs` —
//! that one-line declaration belongs to the M3 integration lane, keeping
//! this lane's file lease intact. Until then this target compiles the
//! module through a `#[path]` shim, exactly as `tests/session_domain.rs`
//! did for `session` before PR #75 wired it. The `session` shim module
//! below mirrors the crate's public session surface so the persistence
//! module's `crate::session` references resolve identically in both
//! contexts; once `lib.rs` declares `pub mod session_persistence;`, replace
//! the shim with crate imports as the session-domain lane did.
//!
//! Pinned behavior:
//!
//! 1. an absent file is the first run: empty state, no error;
//! 2. round-trip: write then read yields the same entries and selection;
//! 3. truncated, malformed, non-UTF-8, and wrong-version files each error
//!    cleanly and never leave a partially-loaded registry;
//! 4. a file claiming a future version is rejected, not partially parsed;
//! 5. session lists are bounded in both directions;
//! 6. the ADR 0003 boundary is enforced: session-interior keys never parse.

mod session {
    //! Mirror of `noren_app::session` for the still-unwired module below.
    pub use noren_app::session::*;
}

#[path = "../src/session_persistence.rs"]
mod session_persistence;

use noren_app::session::{SessionDescriptor, SessionKind, SessionRegistry, SessionStatus};
use session_persistence::{
    MAX_SESSION_STATE_BYTES, MAX_SESSIONS, SESSION_STATE_VERSION, SessionPersistenceError, encode,
    load, load_bytes, save,
};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Per-test uniqueness: tests run concurrently and share the temp dir.
static CASE: AtomicUsize = AtomicUsize::new(0);

fn temp_path(name: &str) -> PathBuf {
    let unique = CASE.fetch_add(1, Ordering::Relaxed);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "noren-persist-test-{}-{unique}-{name}",
        std::process::id()
    ));
    path
}

fn write_file(path: &Path, bytes: &[u8]) {
    let mut file = std::fs::File::create(path).expect("create state fixture");
    file.write_all(bytes).expect("write state fixture");
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// A sidebar spanning every D-M3-001 kind, in a stable order.
fn sidebar_registry() -> SessionRegistry {
    let mut registry = SessionRegistry::new();
    let _ = registry.create(SessionKind::Local);
    let _ = registry.create(SessionKind::Project {
        root: PathBuf::from("/srv/noren"),
    });
    let _ = registry.create(SessionKind::Worktree {
        path: PathBuf::from("/srv/noren-worktrees/pool-persist"),
    });
    let _ = registry.create(SessionKind::Ssh {
        target: "ops@bastion.example".to_owned(),
    });
    let _ = registry.create(SessionKind::Agent {
        name: "opencode".to_owned(),
    });
    registry
}

fn kinds_of(registry: &SessionRegistry) -> Vec<SessionKind> {
    registry
        .sessions()
        .iter()
        .map(SessionDescriptor::kind)
        .cloned()
        .collect()
}

/// The selection as a positional index, the form the file persists it in.
fn selected_index(registry: &SessionRegistry) -> Option<usize> {
    let sessions = registry.sessions();
    let selected = registry.selected()?;
    sessions
        .iter()
        .position(|descriptor| descriptor.id() == selected)
}

// ── Required: absent file → empty state, no error ──────────────────────

#[test]
fn missing_file_loads_as_empty_state_without_error() {
    let mut path = temp_path("missing-dir");
    path.push("does-not-exist");
    path.push("sessions.toml");
    assert!(!path.exists());

    let mut registry = SessionRegistry::new();
    assert_eq!(load(&path, &mut registry), Ok(()));
    assert!(registry.is_empty());
    assert_eq!(registry.selected(), None);
}

#[test]
fn missing_file_behaves_exactly_as_today() {
    // First run: the registry a missing file produces is indistinguishable
    // from a fresh one.
    let mut loaded = SessionRegistry::new();
    let fresh = SessionRegistry::new();
    assert_eq!(load(&temp_path("never-written.toml"), &mut loaded), Ok(()));
    assert_eq!(loaded.len(), fresh.len());
    assert_eq!(loaded.selected(), fresh.selected());
    assert_eq!(kinds_of(&loaded), kinds_of(&fresh));
}

// ── Required: round-trip yields the same entries and selection ─────────

#[test]
fn round_trip_preserves_every_entry_kind_and_the_selection() {
    let mut source = sidebar_registry();
    source
        .select(source.sessions()[2].id())
        .expect("the worktree entry is live");
    let path = temp_path("round-trip.toml");
    save(&path, &source).expect("sidebar state saves");

    let mut restored = SessionRegistry::new();
    load(&path, &mut restored).expect("sidebar state loads");

    assert_eq!(restored.len(), source.len());
    assert_eq!(kinds_of(&restored), kinds_of(&source));
    assert_eq!(selected_index(&restored), Some(2));
    assert_eq!(
        selected_index(&source).map(|index| kinds_of(&source)[index].clone()),
        selected_index(&restored).map(|index| kinds_of(&restored)[index].clone())
    );
    cleanup(&path);
}

#[test]
fn restored_entries_reenter_as_starting_with_generated_titles() {
    // Restoration re-spawns domain entries, not runtime facts: every entry
    // comes back through SessionRegistry::create, so it is Starting with a
    // generated display title, and the selected entry resolves live.
    let mut source = sidebar_registry();
    source
        .select(source.sessions()[4].id())
        .expect("the agent entry is live");
    let path = temp_path("restored-status.toml");
    save(&path, &source).expect("state saves");

    let mut restored = SessionRegistry::new();
    load(&path, &mut restored).expect("state loads");
    for (index, descriptor) in restored.sessions().iter().enumerate() {
        assert_eq!(descriptor.status(), &SessionStatus::Starting);
        assert_eq!(descriptor.title(), format!("session-{}", index + 1));
    }
    let selected = registry_selected_descriptor(&restored);
    assert_eq!(
        selected.kind(),
        &SessionKind::Agent {
            name: "opencode".to_owned()
        }
    );
    cleanup(&path);
}

fn registry_selected_descriptor(registry: &SessionRegistry) -> SessionDescriptor {
    let selected = registry.selected().expect("selection persisted");
    registry.get(selected).expect("selection resolves live")
}

#[test]
fn round_trip_with_no_selection_omits_and_restores_none() {
    let source = sidebar_registry();
    let path = temp_path("unselected.toml");
    save(&path, &source).expect("state saves");

    let text = std::fs::read_to_string(&path).expect("state file readable");
    assert!(
        text.contains(&format!("version = {SESSION_STATE_VERSION}\n")),
        "the pinned version is written first"
    );
    assert!(
        !text.contains("selected"),
        "no selection key when unselected"
    );

    let mut restored = SessionRegistry::new();
    load(&path, &mut restored).expect("state loads");
    assert_eq!(restored.len(), source.len());
    assert_eq!(restored.selected(), None);
    cleanup(&path);
}

#[test]
fn empty_registry_round_trips_to_empty_state() {
    let source = SessionRegistry::new();
    let path = temp_path("empty.toml");
    save(&path, &source).expect("empty state saves");

    let mut restored = SessionRegistry::new();
    load(&path, &mut restored).expect("empty state loads");
    assert!(restored.is_empty());
    assert_eq!(restored.selected(), None);
    cleanup(&path);
}

#[test]
fn encoding_is_deterministic_across_saves() {
    let mut source = sidebar_registry();
    source
        .select(source.sessions()[3].id())
        .expect("the ssh entry is live");
    let first = temp_path("determinism-1.toml");
    let second = temp_path("determinism-2.toml");
    save(&first, &source).expect("first save");
    save(&second, &source).expect("second save");
    assert_eq!(
        std::fs::read(&first).expect("read first"),
        std::fs::read(&second).expect("read second")
    );

    // Re-encoding a restored registry reproduces the document byte for byte.
    let mut restored = SessionRegistry::new();
    load(&first, &mut restored).expect("state loads");
    assert_eq!(
        encode(&restored).expect("re-encode"),
        encode(&source).expect("encode")
    );
    cleanup(&first);
    cleanup(&second);
}

#[test]
fn hostile_strings_round_trip_through_the_escaper() {
    let target = "weird\"quote\\back\nline\ttab \u{1b} escape 界";
    let mut source = SessionRegistry::new();
    let _ = source.create(SessionKind::Ssh {
        target: target.to_owned(),
    });
    let _ = source.create(SessionKind::Project {
        root: PathBuf::from("/srv/space name/ünïcode"),
    });
    source
        .select(source.sessions()[0].id())
        .expect("entry is live");

    let path = temp_path("hostile-strings.toml");
    save(&path, &source).expect("hostile strings save");
    let mut restored = SessionRegistry::new();
    load(&path, &mut restored).expect("hostile strings load");
    assert_eq!(kinds_of(&restored), kinds_of(&source));
    assert_eq!(selected_index(&restored), Some(0));
    cleanup(&path);
}

#[test]
fn save_replaces_previous_state_never_partially() {
    let path = temp_path("replace.toml");
    let mut first = SessionRegistry::new();
    let _ = first.create(SessionKind::Local);
    save(&path, &first).expect("first save");

    let second = sidebar_registry();
    save(&path, &second).expect("replacement save");

    let mut restored = SessionRegistry::new();
    load(&path, &mut restored).expect("replacement loads");
    assert_eq!(kinds_of(&restored), kinds_of(&second));
    cleanup(&path);
}

#[test]
fn save_creates_missing_parent_directories() {
    let mut path = temp_path("nested-parent");
    path.push("deeper");
    path.push("sessions.toml");
    let source = sidebar_registry();
    save(&path, &source).expect("parent directories are created");

    let mut restored = SessionRegistry::new();
    load(&path, &mut restored).expect("state loads");
    assert_eq!(kinds_of(&restored), kinds_of(&source));
    let _ = std::fs::remove_dir_all(temp_path_root(&path));
}

fn temp_path_root(nested: &Path) -> PathBuf {
    // The temp_path name is the first component after the system temp dir;
    // strip "deeper/sessions.toml" to reach the per-case root.
    nested
        .ancestors()
        .nth(2)
        .expect("two levels deep")
        .to_path_buf()
}

// ── Required: truncated, malformed, non-UTF-8, wrong-version error ─────

#[test]
fn truncated_files_error_cleanly() {
    let source = sidebar_registry();
    let text = encode(&source).expect("encoding succeeds");
    let mut bytes = text.into_bytes();
    bytes.truncate(bytes.len() - 7); // cut inside the final payload string

    let mut registry = SessionRegistry::new();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        load_bytes(&bytes, &mut registry)
    }));
    let loaded = result.expect("loading must never panic");
    assert!(
        matches!(loaded, Err(SessionPersistenceError::Parse(_))),
        "a truncated document must fail parsing, got {loaded:?}"
    );
    assert!(registry.is_empty());
    assert_eq!(registry.selected(), None);
}

#[test]
fn malformed_documents_error_cleanly() {
    let cases = [
        "",                                            // no version at all
        "# only a comment\n",                          // still no version
        "not toml at all\n",                           // no assignment
        "version = \n",                                // dangling value
        "version = 1\nkind = \"local\"\n",             // session keys at the root
        "version = 1\nsessions = 3\n",                 // wrong shape for sessions
        "version = 1\n[sessions]\nkind = \"local\"\n", // table, not array
    ];
    for text in cases {
        let mut registry = SessionRegistry::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            load_bytes(text.as_bytes(), &mut registry)
        }));
        let loaded = result.expect("loading must never panic");
        assert!(
            matches!(
                loaded,
                Err(SessionPersistenceError::Parse(_)
                    | SessionPersistenceError::MissingKey(_)
                    | SessionPersistenceError::UnknownKey(_)
                    | SessionPersistenceError::WrongType { .. })
            ),
            "{text:?} must fail cleanly, got {loaded:?}"
        );
        assert!(registry.is_empty(), "{text:?} must load nothing");
    }
}

#[test]
fn the_version_key_must_be_an_integer() {
    let mut registry = SessionRegistry::new();
    assert_eq!(
        load_bytes(b"version = \"one\"\n", &mut registry),
        Err(SessionPersistenceError::WrongType {
            key: "version".to_owned()
        })
    );
    assert!(registry.is_empty());
}

#[test]
fn non_utf8_files_error_cleanly() {
    let bytes = [0xff_u8, 0xfe, 0x00, b'v', b'e', b'r'];
    let mut registry = SessionRegistry::new();
    let loaded = load_bytes(&bytes, &mut registry);
    assert_eq!(loaded, Err(SessionPersistenceError::NotUtf8));
    assert!(registry.is_empty());

    let path = temp_path("non-utf8.toml");
    write_file(&path, &bytes);
    assert_eq!(
        load(&path, &mut registry),
        Err(SessionPersistenceError::NotUtf8)
    );
    assert!(registry.is_empty());
    cleanup(&path);
}

#[test]
fn wrong_versions_are_rejected_whole() {
    for version in [0_i64, -1, 2, 99] {
        let text = format!("version = {version}\nselected = 0\n\n[[sessions]]\nkind = \"local\"\n");
        let mut registry = SessionRegistry::new();
        assert_eq!(
            load_bytes(text.as_bytes(), &mut registry),
            Err(SessionPersistenceError::UnsupportedVersion(version)),
            "version {version} must be rejected"
        );
        assert!(registry.is_empty(), "version {version} must load nothing");
    }
}

/// Required: a file claiming a future version is rejected, not partially
/// parsed — even into a registry that already holds live sessions.
#[test]
fn a_future_version_is_rejected_without_partial_state() {
    let future =
        "version = 2\nselected = 0\n\n[[sessions]]\nkind = \"ssh\"\ntarget = \"new@host\"\n";

    let mut empty = SessionRegistry::new();
    assert_eq!(
        load_bytes(future.as_bytes(), &mut empty),
        Err(SessionPersistenceError::UnsupportedVersion(2))
    );
    assert!(empty.is_empty());

    let mut populated = sidebar_registry();
    populated
        .select(populated.sessions()[1].id())
        .expect("the project entry is live");
    let before_kinds = kinds_of(&populated);
    let before_selection = selected_index(&populated);
    assert_eq!(
        load_bytes(future.as_bytes(), &mut populated),
        Err(SessionPersistenceError::UnsupportedVersion(2))
    );
    assert_eq!(kinds_of(&populated), before_kinds, "entries untouched");
    assert_eq!(
        selected_index(&populated),
        before_selection,
        "selection untouched"
    );
}

// ── Boundary and strictness: untrusted details never slip through ──────

#[test]
fn session_interior_keys_are_rejected_by_the_boundary() {
    // ADR 0003: tabs, panes, splits, and layout belong to Zellij. Any key
    // describing the interior of a session must be rejected, and a session
    // table accepts only the exact payload its kind defines.
    let cases = [
        "[[sessions]]\nkind = \"local\"\npanes = 2\n",
        "[[sessions]]\nkind = \"local\"\ntabs = [\"main\"]\n",
        "[[sessions]]\nkind = \"project\"\nroot = \"/p\"\nlayout = \"split\"\n",
        "[[sessions]]\nkind = \"ssh\"\ntarget = \"h\"\ncwd = \"/tmp\"\n",
        "[[sessions]]\nkind = \"local\"\ncwd = \"/tmp\"\n",
    ];
    for body in cases {
        let text = format!("version = 1\n\n{body}");
        let mut registry = SessionRegistry::new();
        assert_eq!(
            load_bytes(text.as_bytes(), &mut registry),
            Err(SessionPersistenceError::UnknownKey(
                body.lines()
                    .last()
                    .expect("case has a trailing key")
                    .split_once(" = ")
                    .expect("case names a key")
                    .0
                    .to_owned()
            )),
            "{body:?} describes the interior of a session and must be rejected"
        );
        assert!(registry.is_empty(), "{body:?} must load nothing");
    }
}

#[test]
fn unknown_top_level_and_session_keys_are_rejected() {
    let mut registry = SessionRegistry::new();
    assert_eq!(
        load_bytes(b"version = 1\n[theme]\ndark = true\n", &mut registry),
        Err(SessionPersistenceError::UnknownKey("theme".to_owned()))
    );
    let mut registry = SessionRegistry::new();
    assert_eq!(
        load_bytes(
            b"version = 1\n\n[[sessions]]\nkind = \"agent\"\nname = \"a\"\nmodel = \"x\"\n",
            &mut registry
        ),
        Err(SessionPersistenceError::UnknownKey("model".to_owned()))
    );
}

#[test]
fn malformed_session_tables_error_cleanly() {
    let cases = [
        // Missing or wrong-typed `kind`.
        (
            "version = 1\n\n[[sessions]]\n",
            SessionPersistenceError::MissingKey("kind".to_owned()),
        ),
        (
            "version = 1\n\n[[sessions]]\nkind = 3\n",
            SessionPersistenceError::WrongType {
                key: "kind".to_owned(),
            },
        ),
        // Unknown kind string.
        (
            "version = 1\n\n[[sessions]]\nkind = \"mystery\"\n",
            SessionPersistenceError::UnknownKind("mystery".to_owned()),
        ),
        // Missing or wrong-typed payloads.
        (
            "version = 1\n\n[[sessions]]\nkind = \"project\"\n",
            SessionPersistenceError::MissingKey("root".to_owned()),
        ),
        (
            "version = 1\n\n[[sessions]]\nkind = \"worktree\"\npath = 9\n",
            SessionPersistenceError::WrongType {
                key: "path".to_owned(),
            },
        ),
        // Empty payloads are out of range.
        (
            "version = 1\n\n[[sessions]]\nkind = \"ssh\"\ntarget = \"\"\n",
            SessionPersistenceError::OutOfRange {
                key: "target".to_owned(),
            },
        ),
        // Selection problems.
        (
            "version = 1\nselected = 3\n\n[[sessions]]\nkind = \"local\"\n",
            SessionPersistenceError::OutOfRange {
                key: "selected".to_owned(),
            },
        ),
        (
            "version = 1\nselected = -1\n\n[[sessions]]\nkind = \"local\"\n",
            SessionPersistenceError::OutOfRange {
                key: "selected".to_owned(),
            },
        ),
        (
            "version = 1\nselected = 0\n",
            SessionPersistenceError::OutOfRange {
                key: "selected".to_owned(),
            },
        ),
        (
            "version = 1\nselected = \"0\"\n\n[[sessions]]\nkind = \"local\"\n",
            SessionPersistenceError::WrongType {
                key: "selected".to_owned(),
            },
        ),
    ];
    for (text, expected) in cases {
        let mut registry = SessionRegistry::new();
        let loaded = load_bytes(text.as_bytes(), &mut registry);
        assert_eq!(loaded, Err(expected), "{text:?}");
        assert!(registry.is_empty(), "{text:?} must load nothing");
    }
}

#[test]
fn a_non_utf8_path_refuses_to_save_lossily() {
    use std::os::unix::ffi::OsStringExt;
    let mut registry = SessionRegistry::new();
    let _ = registry.create(SessionKind::Worktree {
        path: PathBuf::from(std::ffi::OsString::from_vec(vec![b'/', 0xff, b'w'])),
    });
    let path = temp_path("non-utf8-path.toml");
    assert_eq!(
        save(&path, &registry),
        Err(SessionPersistenceError::NonUtf8Path)
    );
    assert!(!path.exists(), "a refused save must leave nothing behind");
}

#[test]
fn load_errors_never_mutate_a_populated_registry() {
    for bytes in [
        b"garbage".as_slice(),
        b"version = 7\n",
        &[0xff_u8, 0xfe][..],
        b"version = 1\nselected = 999\n\n[[sessions]]\nkind = \"local\"\n",
    ] {
        let mut registry = sidebar_registry();
        registry
            .select(registry.sessions()[3].id())
            .expect("the ssh entry is live");
        let before_kinds = kinds_of(&registry);
        let before_selection = selected_index(&registry);

        assert!(load_bytes(bytes, &mut registry).is_err());
        assert_eq!(registry.len(), before_kinds.len());
        assert_eq!(kinds_of(&registry), before_kinds);
        assert_eq!(selected_index(&registry), before_selection);
    }
}

#[test]
fn directory_and_oversized_paths_are_rejected_cleanly() {
    let dir = temp_path("state-is-a-directory");
    std::fs::create_dir_all(&dir).expect("create directory fixture");
    assert_eq!(
        load(&dir, &mut SessionRegistry::new()),
        Err(SessionPersistenceError::NotAFile)
    );

    let oversized = temp_path("oversized.toml");
    write_file(
        &oversized,
        &vec![b'#'; MAX_SESSION_STATE_BYTES as usize + 1],
    );
    assert_eq!(
        load(&oversized, &mut SessionRegistry::new()),
        Err(SessionPersistenceError::TooLarge)
    );
    cleanup(&oversized);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn hostile_key_names_are_clipped_in_errors() {
    let mut key = String::from("version = 1\n\"");
    key.extend(std::iter::repeat_n('a', 10_000));
    key.push_str("\" = 1\n");
    let mut registry = SessionRegistry::new();
    let error =
        load_bytes(key.as_bytes(), &mut registry).expect_err("a hostile unknown key must fail");
    let SessionPersistenceError::UnknownKey(echoed) = error else {
        panic!("expected UnknownKey, got {error:?}")
    };
    assert!(
        echoed.chars().count() <= 121,
        "hostile key must be clipped: {echoed}"
    );
}

#[test]
fn error_messages_never_embed_full_file_contents() {
    let mut hostile = String::from("not toml ");
    hostile.extend(std::iter::repeat_n('x', 100_000));
    let mut registry = SessionRegistry::new();
    let error = load_bytes(hostile.as_bytes(), &mut registry).expect_err("malformed input fails");
    let message = error.to_string();
    assert!(message.len() < 1024, "message must stay bounded: {message}");
}

// ── Required: session lists stay bounded in both directions ─────────────

#[test]
fn oversized_registries_are_rejected_before_anything_is_written() {
    let mut registry = SessionRegistry::new();
    for _ in 0..=MAX_SESSIONS {
        let _ = registry.create(SessionKind::Local);
    }
    let path = temp_path("too-many.toml");
    assert_eq!(
        save(&path, &registry),
        Err(SessionPersistenceError::TooManySessions)
    );
    assert!(!path.exists(), "nothing may be written beyond the bound");
}

#[test]
fn the_maximum_session_list_stays_bounded_on_disk() {
    let mut registry = SessionRegistry::new();
    for index in 0..MAX_SESSIONS {
        let _ = registry.create(SessionKind::Project {
            root: PathBuf::from(format!("/srv/project-{index:04}")),
        });
    }
    registry
        .select(registry.sessions()[MAX_SESSIONS - 1].id())
        .expect("the last entry is live");

    let path = temp_path("max-list.toml");
    save(&path, &registry).expect("the maximum list saves");
    let size = std::fs::metadata(&path).expect("state file exists").len();
    assert!(
        size <= MAX_SESSION_STATE_BYTES,
        "a maximal list must stay inside the byte bound"
    );

    let mut restored = SessionRegistry::new();
    load(&path, &mut restored).expect("the maximum list loads");
    assert_eq!(restored.len(), MAX_SESSIONS);
    assert_eq!(selected_index(&restored), Some(MAX_SESSIONS - 1));
    cleanup(&path);
}

#[test]
fn oversized_documents_are_rejected_on_read() {
    let mut text = String::from("version = 1\n");
    for _ in 0..=MAX_SESSIONS {
        text.push_str("\n[[sessions]]\nkind = \"local\"\n");
    }
    let mut registry = SessionRegistry::new();
    assert_eq!(
        load_bytes(text.as_bytes(), &mut registry),
        Err(SessionPersistenceError::TooManySessions)
    );
    assert!(registry.is_empty());
}

#[test]
fn restoring_into_a_populated_registry_keeps_the_bound() {
    let mut registry = SessionRegistry::new();
    for _ in 0..MAX_SESSIONS - 1 {
        let _ = registry.create(SessionKind::Local);
    }
    let before = registry.len();
    let text = "version = 1\n\n[[sessions]]\nkind = \"local\"\n\n[[sessions]]\nkind = \"local\"\n";
    assert_eq!(
        load_bytes(text.as_bytes(), &mut registry),
        Err(SessionPersistenceError::TooManySessions)
    );
    assert_eq!(registry.len(), before, "the registry is untouched on error");
}

#[test]
fn encode_rejects_an_oversized_registry_before_serializing() {
    let mut registry = SessionRegistry::new();
    for _ in 0..=MAX_SESSIONS {
        let _ = registry.create(SessionKind::Ssh {
            target: "host".to_owned(),
        });
    }
    assert_eq!(
        encode(&registry),
        Err(SessionPersistenceError::TooManySessions)
    );
}
