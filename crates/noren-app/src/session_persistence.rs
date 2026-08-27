//! Sidebar persistence: which sessions exist and which one is selected.
//!
//! This module saves and restores the Noren sidebar state defined by
//! [ADR 0003](../../../docs/adr/0003-noren-zellij-responsibility-boundary.md):
//! which projects, worktrees, SSH targets, agents, and terminal sessions
//! exist, and which of them is selected. It reuses the shared D-M3-001
//! vocabulary from [`crate::session`] ([`SessionKind`] and
//! [`SessionRegistry`]); it defines no parallel session model.
//!
//! # The boundary is enforced in the parser
//!
//! Noren manages the workspace OUTSIDE the terminal; Zellij manages it
//! INSIDE. Nothing that describes the interior of a session is ever written
//! or accepted: no tabs, no panes, no splits, no layout tree, no terminal
//! content. A session serializes as its [`SessionKind`] and nothing else,
//! and a session table carrying any other key (a `panes` count, a `tab`
//! name, anything) is rejected as unknown rather than parsed and ignored.
//!
//! # The format is versioned, but it is not final
//!
//! The on-disk document is TOML and carries `version = 1` from the first
//! write. Shipping a format that later changes breaks every existing user's
//! state, so this module treats version 1 as a starting point with a
//! migration path (`version` is checked before anything else) instead of a
//! public commitment. A document claiming any other version is rejected
//! whole — never partially parsed.
//!
//! # The file is untrusted input
//!
//! A truncated, malformed, wrong-version, non-UTF-8, oversized, or hostile
//! file produces a clean [`SessionPersistenceError`] and leaves the target
//! registry untouched: decoding validates the entire document before a
//! single entry is created, so a partial load can never look complete. A
//! **missing** file is normal — it is the first run — and loads as empty
//! state without error.
//!
//! # Bounded state
//!
//! Both directions are bounded, because a session list that grows without
//! limit on disk is the same defect class as an unbounded in-memory list:
//! reads stop at [`MAX_SESSION_STATE_BYTES`] and refuse more than
//! [`MAX_SESSIONS`] entries, and writes refuse to encode past either bound.
//!
//! # Identity and restoration
//!
//! Per D-M3-001, [`SessionId`]s are registry-local and are **not**
//! persistence keys, so no id is written. Entries are stored in registry
//! order and the selection is stored as a positional index into that list.
//! Restoration runs each loaded kind through [`SessionRegistry::restore`],
//! which records it as [`SessionStatus::Restored`] with a generated title;
//! runtime status is an observed fact and is never persisted. Titles are
//! derived facts today (a rename feature may change that in a later format
//! version), so they are not persisted either.
//!
//! Restoration re-populates the registry only. Whether restoring a session
//! re-spawns a shell or reattaches to an existing one (for example through
//! Zellij) is an open M3 question this module does not answer; it spawns
//! nothing.
//!
//! [`SessionId`]: crate::session::SessionId
//! [`SessionStatus::Restored`]: crate::session::SessionStatus::Restored

use crate::session::{SessionKind, SessionRegistry};
use std::fmt;
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::path::Path;
use toml_edit::{DocumentMut, Table};

/// Recommended file name for sidebar state inside the Noren data directory.
pub const SESSION_STATE_FILE_NAME: &str = "sessions.toml";

/// The only format version this build reads and writes.
///
/// The version is written on every save and checked before any other key is
/// interpreted, so a future format change has a migration path instead of a
/// guess. This is a stop condition, not a commitment: version 1 is
/// versioned-but-not-final.
pub const SESSION_STATE_VERSION: i64 = 1;

/// Maximum number of sidebar entries persisted in either direction.
///
/// Writes refuse to encode more than this; reads refuse to apply more. The
/// sidebar itself must stay within the same bound; persistence can neither
/// raise nor lower what the live registry may hold.
pub const MAX_SESSIONS: usize = 512;

/// Maximum accepted state file size in bytes.
///
/// Reads are streamed and bounded like the configuration loader, so a
/// pathological path cannot grow memory past this cap; oversized input is a
/// [`SessionPersistenceError::TooLarge`] error instead. Writes check the
/// same cap as the document is built, so a registry that would encode past
/// it is rejected with [`SessionPersistenceError::TooLarge`] rather than
/// written unreadable.
pub const MAX_SESSION_STATE_BYTES: u64 = 512 * 1024;

/// Suffix of the temporary file used for crash-safe replacement writes.
const TMP_SUFFIX: &str = ".tmp";

/// Maximum characters of hostile input echoed inside any error message.
const MAX_ERROR_DETAIL_CHARS: usize = 120;

/// Typed persistence failure without file contents.
///
/// Every variant renders a bounded message: hostile key names are clipped
/// by [`clip`] before they are stored, a TOML parse failure keeps only the
/// 1-based position computed by [`toml_error_position`], never the
/// third-party parser's text, and no variant carries a file *value* — the
/// `kind` value is reported by name and accepted set only (issue #150).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionPersistenceError {
    /// The state file does not exist. Internal to [`load`], which turns
    /// absence into empty state; it never reaches callers of the public API.
    NotFound,
    /// The file could not be read, written, or renamed; only the I/O error
    /// kind is retained.
    Io(ErrorKind),
    /// The path exists but does not resolve to a regular file.
    NotAFile,
    /// The state exceeds [`MAX_SESSION_STATE_BYTES`], in either direction:
    /// a file too large to read, or a registry that would encode to a
    /// document too large to read back.
    TooLarge,
    /// The file is not valid UTF-8.
    NotUtf8,
    /// The file is not valid TOML (including truncation).
    ///
    /// Only the 1-based position of the first offending token is retained.
    /// The third-party parser's message is never forwarded: it quotes the
    /// offending source line, and a source line is file content.
    Parse {
        /// The 1-based line where parsing stopped.
        line: usize,
        /// The 1-based column where parsing stopped.
        column: usize,
    },
    /// The file omits a required key.
    MissingKey(String),
    /// The file names a key this format does not define.
    ///
    /// This is also where the ADR 0003 boundary bites: session-interior
    /// keys such as `panes`, `tabs`, or `layout` land here.
    UnknownKey(String),
    /// A key holds the wrong TOML type.
    WrongType {
        /// The offending key.
        key: String,
    },
    /// A value is outside its accepted range: a `selected` index beyond the
    /// entry list, or an empty path/target/name payload.
    OutOfRange {
        /// The offending key.
        key: String,
    },
    /// A session path is not valid UTF-8. TOML strings are UTF-8, so the
    /// path could only be saved through a lossy conversion — persisting a
    /// different path than the live one is corruption, so the save refuses.
    NonUtf8Path,
    /// The `kind` value names no [`SessionKind`] variant.
    ///
    /// Only the key and the accepted kinds are reported, never the file's
    /// own spelling: a `kind` value is arbitrary text (an SSH target pasted
    /// into the wrong field would land here), so echoing it would put file
    /// content on stderr. Unlike a chord there is no usability loss — the
    /// accepted set is small enough to name in full (issue #150).
    UnknownKind,
    /// The file claims a format version this build does not speak, past or
    /// future. The document is rejected whole, never partially parsed.
    UnsupportedVersion(i64),
    /// The file, or the write request, carries more than [`MAX_SESSIONS`]
    /// entries.
    TooManySessions,
}

impl fmt::Display for SessionPersistenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("session state file not found"),
            Self::Io(kind) => write!(f, "session state I/O failed: {kind}"),
            Self::NotAFile => f.write_str("session state path does not resolve to a regular file"),
            Self::TooLarge => write!(f, "session state exceeds {MAX_SESSION_STATE_BYTES} bytes"),
            Self::NotUtf8 => f.write_str("session state is not valid UTF-8"),
            Self::Parse { line, column } => write!(
                f,
                "session state is not valid TOML at line {line}, column {column}"
            ),
            Self::MissingKey(key) => write!(f, "session state is missing key: {key}"),
            Self::UnknownKey(key) => write!(f, "unknown session state key: {key}"),
            Self::WrongType { key } => {
                write!(f, "session state key {key} has the wrong TOML type")
            }
            Self::OutOfRange { key } => {
                write!(f, "session state key {key} is outside its accepted range")
            }
            Self::NonUtf8Path => {
                f.write_str("session path is not valid UTF-8 and cannot be persisted without loss")
            }
            Self::UnknownKind => write!(
                f,
                "session kind is not one of: local, project, worktree, ssh, agent"
            ),
            Self::UnsupportedVersion(version) => write!(
                f,
                "session state version {version} is not supported; this build speaks version {SESSION_STATE_VERSION}"
            ),
            Self::TooManySessions => write!(f, "session state exceeds {MAX_SESSIONS} sessions"),
        }
    }
}

impl std::error::Error for SessionPersistenceError {}

/// Save the sidebar state to `path`, replacing any previous file.
///
/// The write is crash-safe in the format sense: the document is written to a
/// temporary sibling and renamed over the destination, so the file is either
/// the old state or the new state, never a truncation. Refuses to encode
/// more than [`MAX_SESSIONS`] entries or a document larger than
/// [`MAX_SESSION_STATE_BYTES`]. Parent directories are created when absent.
pub fn save(path: &Path, registry: &SessionRegistry) -> Result<(), SessionPersistenceError> {
    save_snapshot(path, registry).map(drop)
}

/// Save the sidebar state and return the exact bounded bytes handed to the
/// atomic writer.
///
/// This function is public only because this package's library and binary are
/// separate crates: the binary needs the exact write bytes for post-save
/// verification and cannot call a library-private item. The returned buffer
/// is no larger than [`MAX_SESSION_STATE_BYTES`], enforced by [`encode`].
///
/// Callers that verify a save must compare their post-save observation with
/// this value. Merely observing that some file exists after the rename is not
/// evidence that it contains this process's document: another process may
/// have replaced it between the write and the observation. The returned
/// buffer is therefore the intended document, not a claim about later disk
/// contents.
pub fn save_snapshot(
    path: &Path,
    registry: &SessionRegistry,
) -> Result<Vec<u8>, SessionPersistenceError> {
    let bytes = encode(registry)?.into_bytes();
    write_atomic(path, &bytes)?;
    Ok(bytes)
}

/// Load the sidebar state from `path` into `registry`, creating one entry
/// per saved kind (in saved order) and selecting the saved selection.
///
/// A missing file is the first run: it leaves `registry` untouched and
/// returns `Ok`. Any file that exists must read, decode, and validate or the
/// call errors — and because decoding validates before creating, an error
/// also leaves `registry` exactly as it was. Entries re-enter through
/// [`SessionRegistry::restore`], so they start at `Restored` with generated
/// titles; this module spawns nothing.
pub fn load(path: &Path, registry: &mut SessionRegistry) -> Result<(), SessionPersistenceError> {
    load_snapshot(path, registry).map(drop)
}

/// Load the sidebar state and return the exact bounded bytes that were read.
///
/// The returned snapshot is a caller-owned baseline for detecting a later
/// external replacement. It is captured from the same read that is decoded,
/// so the baseline does not require a second filesystem observation during
/// restore.
pub fn load_snapshot(
    path: &Path,
    registry: &mut SessionRegistry,
) -> Result<Option<Vec<u8>>, SessionPersistenceError> {
    let bytes = match read_bounded(path) {
        Ok(bytes) => bytes,
        Err(SessionPersistenceError::NotFound) => return Ok(None),
        Err(error) => return Err(error),
    };
    load_bytes(&bytes, registry)?;
    Ok(Some(bytes))
}

/// Read the current bounded state-file bytes without decoding them.
///
/// A missing file is represented as `Ok(None)`, matching [`load`]. This is
/// used only for the best-effort external-change check; the atomic save path
/// remains responsible for writing the document.
pub fn snapshot(path: &Path) -> Result<Option<Vec<u8>>, SessionPersistenceError> {
    match read_bounded(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(SessionPersistenceError::NotFound) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Decode raw state file bytes and apply them to `registry`.
///
/// Exposed so the decode contract is testable without touching the
/// filesystem. Validation is whole-document: a non-UTF-8, malformed,
/// wrong-version, or oversized document errors before `registry` is mutated.
pub fn load_bytes(
    bytes: &[u8],
    registry: &mut SessionRegistry,
) -> Result<(), SessionPersistenceError> {
    let text = std::str::from_utf8(bytes).map_err(|_| SessionPersistenceError::NotUtf8)?;
    let (kinds, selected) = decode(text)?;
    apply(kinds, selected, registry)
}

/// Serialize `registry` to the versioned TOML document.
///
/// Deterministic: the same registry state always produces the same text.
/// Entries are written in [`SessionRegistry::sessions`] order; the selection
/// is written as a positional index and omitted when nothing is selected.
/// Refuses more than [`MAX_SESSIONS`] entries, a document larger than
/// [`MAX_SESSION_STATE_BYTES`] (checked as it is built, so the encoded
/// [`String`] is bounded and anything that would be unreadable on load is
/// rejected before it is returned), and non-UTF-8 paths.
pub fn encode(registry: &SessionRegistry) -> Result<String, SessionPersistenceError> {
    let sessions = registry.sessions();
    if sessions.len() > MAX_SESSIONS {
        return Err(SessionPersistenceError::TooManySessions);
    }
    let byte_cap = usize::try_from(MAX_SESSION_STATE_BYTES).unwrap_or(usize::MAX);
    let selected_index: Option<usize> = registry.selected().map(|selected| {
        sessions
            .iter()
            .position(|descriptor| descriptor.id() == selected)
            .expect("registry selection always refers to a live session")
    });

    let mut text = String::new();
    text.push_str("# Noren sidebar state. Noren owns what exists outside the terminal;\n");
    text.push_str("# nothing in this file describes what is inside a session (ADR 0003).\n");
    text.push_str(&format!("version = {SESSION_STATE_VERSION}\n"));
    if let Some(index) = selected_index {
        text.push_str(&format!("selected = {index}\n"));
    }
    for descriptor in &sessions {
        text.push_str("\n[[sessions]]\n");
        match descriptor.kind() {
            SessionKind::Local => text.push_str("kind = \"local\"\n"),
            SessionKind::Project { root } => {
                text.push_str("kind = \"project\"\n");
                text.push_str(&format!("root = {}\n", toml_string(path_text(root)?)));
            }
            SessionKind::Worktree { path } => {
                text.push_str("kind = \"worktree\"\n");
                text.push_str(&format!("path = {}\n", toml_string(path_text(path)?)));
            }
            SessionKind::Ssh { target } => {
                text.push_str("kind = \"ssh\"\n");
                text.push_str(&format!("target = {}\n", toml_string(target)));
            }
            SessionKind::Agent { name } => {
                text.push_str("kind = \"agent\"\n");
                text.push_str(&format!("name = {}\n", toml_string(name)));
            }
        }
        if text.len() > byte_cap {
            return Err(SessionPersistenceError::TooLarge);
        }
    }
    Ok(text)
}

/// Borrow a persistable path, refusing the lossy alternative.
///
/// TOML strings are UTF-8; a path that is not valid UTF-8 could only be
/// written through a lossy conversion, which would silently persist a
/// different path. That is corruption, so it is an error instead.
fn path_text(path: &std::path::Path) -> Result<&str, SessionPersistenceError> {
    path.to_str().ok_or(SessionPersistenceError::NonUtf8Path)
}

/// Escape one value as a TOML basic (double-quoted) string.
///
/// TOML basic strings must escape quotes, backslashes, and every control
/// character; everything else passes through as UTF-8. The decode side is
/// the TOML parser, so the round-trip tests in
/// `tests/session_persistence.rs` are the witness that this escaping is
/// faithful.
fn toml_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\u{8}' => escaped.push_str("\\b"),
            '\u{c}' => escaped.push_str("\\f"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04X}", u32::from(character)));
            }
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}

/// Parse and validate the whole document before anything is applied.
///
/// The version check runs first: a document claiming another version is
/// rejected here, before sessions are even looked at, so a future file is
/// never partially parsed.
fn decode(text: &str) -> Result<(Vec<SessionKind>, Option<usize>), SessionPersistenceError> {
    let document: DocumentMut = text.parse().map_err(|error: toml_edit::TomlError| {
        let (line, column) = toml_error_position(text, &error);
        SessionPersistenceError::Parse { line, column }
    })?;
    let root = document.as_table();

    let version = root
        .get("version")
        .ok_or_else(|| SessionPersistenceError::MissingKey(clip("version")))?
        .as_integer()
        .ok_or_else(|| SessionPersistenceError::WrongType {
            key: clip("version"),
        })?;
    if version != SESSION_STATE_VERSION {
        return Err(SessionPersistenceError::UnsupportedVersion(version));
    }

    let mut kinds: Vec<SessionKind> = Vec::new();
    let mut selected: Option<usize> = None;
    for (key, item) in root.iter() {
        match key {
            "version" => {}
            "selected" => {
                let index =
                    item.as_integer()
                        .ok_or_else(|| SessionPersistenceError::WrongType {
                            key: clip("selected"),
                        })?;
                selected = Some(usize::try_from(index).map_err(|_| {
                    SessionPersistenceError::OutOfRange {
                        key: clip("selected"),
                    }
                })?);
            }
            "sessions" => {
                let array = item.as_array_of_tables().ok_or_else(|| {
                    SessionPersistenceError::WrongType {
                        key: clip("sessions"),
                    }
                })?;
                if array.len() > MAX_SESSIONS {
                    return Err(SessionPersistenceError::TooManySessions);
                }
                for table in array.iter() {
                    kinds.push(parse_session(table)?);
                }
            }
            other => return Err(SessionPersistenceError::UnknownKey(clip(other))),
        }
    }

    if let Some(index) = selected {
        if index >= kinds.len() {
            return Err(SessionPersistenceError::OutOfRange {
                key: clip("selected"),
            });
        }
    }
    Ok((kinds, selected))
}

/// Parse one `[[sessions]]` table into the D-M3-001 shape it names.
///
/// The payload keys are exact: `local` carries none, `project` carries
/// `root`, `worktree` carries `path`, `ssh` carries `target`, and `agent`
/// carries `name`. Anything else — including anything describing the
/// interior of a session — is an [`SessionPersistenceError::UnknownKey`].
fn parse_session(table: &Table) -> Result<SessionKind, SessionPersistenceError> {
    let kind = table
        .get("kind")
        .ok_or_else(|| SessionPersistenceError::MissingKey(clip("kind")))?
        .as_str()
        .ok_or_else(|| SessionPersistenceError::WrongType { key: clip("kind") })?;
    match kind {
        "local" => {
            require_exact_keys(table, &[])?;
            Ok(SessionKind::Local)
        }
        "project" => Ok(SessionKind::Project {
            root: payload(table, "root")?.into(),
        }),
        "worktree" => Ok(SessionKind::Worktree {
            path: payload(table, "path")?.into(),
        }),
        "ssh" => Ok(SessionKind::Ssh {
            target: payload(table, "target")?,
        }),
        "agent" => Ok(SessionKind::Agent {
            name: payload(table, "name")?,
        }),
        _other => Err(SessionPersistenceError::UnknownKind),
    }
}

/// The non-`kind` keys a session table may carry: none for `local`, exactly
/// one payload key for every other kind. Any other key is rejected.
fn require_exact_keys(table: &Table, expected: &[&str]) -> Result<(), SessionPersistenceError> {
    for (key, _) in table.iter() {
        if key != "kind" && !expected.contains(&key) {
            return Err(SessionPersistenceError::UnknownKey(clip(key)));
        }
    }
    Ok(())
}

/// Read the single required payload string of a session table.
///
/// Enforces the exact key set (`kind` plus `key`), requires the payload to
/// be a non-empty string, and rejects every other key.
fn payload(table: &Table, key: &str) -> Result<String, SessionPersistenceError> {
    require_exact_keys(table, &[key])?;
    let value = table
        .get(key)
        .ok_or_else(|| SessionPersistenceError::MissingKey(clip(key)))?
        .as_str()
        .ok_or_else(|| SessionPersistenceError::WrongType { key: clip(key) })?;
    if value.is_empty() {
        return Err(SessionPersistenceError::OutOfRange { key: clip(key) });
    }
    Ok(value.to_owned())
}

/// Apply a fully validated document to the registry.
///
/// Runs after [`decode`] accepted everything, so the only remaining check is
/// the combined bound: restoring into a populated registry must not push the
/// live state past [`MAX_SESSIONS`] either.
fn apply(
    kinds: Vec<SessionKind>,
    selected: Option<usize>,
    registry: &mut SessionRegistry,
) -> Result<(), SessionPersistenceError> {
    if registry.len() + kinds.len() > MAX_SESSIONS {
        return Err(SessionPersistenceError::TooManySessions);
    }
    let created: Vec<_> = kinds
        .into_iter()
        .map(|kind| registry.restore(kind))
        .collect();
    if let Some(index) = selected {
        registry
            .select(created[index])
            .expect("decoded selection index was validated against the entry list");
    }
    Ok(())
}

/// Read a state file with a hard byte cap.
///
/// Mirrors the configuration loader: symlinks are followed like any
/// user-owned file, but the target must be a regular file and the streamed
/// read stops with [`SessionPersistenceError::TooLarge`] at
/// [`MAX_SESSION_STATE_BYTES`], so a hostile target can neither panic the
/// app nor exhaust memory.
fn read_bounded(path: &Path) -> Result<Vec<u8>, SessionPersistenceError> {
    let metadata = fs::metadata(path).map_err(io_error)?;
    if !metadata.is_file() {
        return Err(SessionPersistenceError::NotAFile);
    }
    let mut file = fs::File::open(path).map_err(io_error)?;
    let cap = usize::try_from(MAX_SESSION_STATE_BYTES).unwrap_or(usize::MAX);
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let read = file
            .read(&mut chunk)
            .map_err(|error| SessionPersistenceError::Io(error.kind()))?;
        if read == 0 {
            return Ok(buffer);
        }
        if buffer.len() + read > cap {
            return Err(SessionPersistenceError::TooLarge);
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

/// Replace `path` atomically with `bytes` via a temporary sibling.
///
/// The temporary file is fully written and synced before the rename, so a
/// crash leaves either the previous document or the new one — never a
/// truncated file. (Directory fsync durability and cross-restart reattach
/// remain open questions recorded in the handoff.)
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), SessionPersistenceError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
    }
    let file_name = path
        .file_name()
        .ok_or(SessionPersistenceError::Io(ErrorKind::InvalidInput))?;
    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(TMP_SUFFIX);
    let tmp = path.with_file_name(tmp_name);

    if let Err(error) = write_tmp(&tmp, bytes) {
        let _ = fs::remove_file(&tmp);
        return Err(error);
    }
    fs::rename(&tmp, path).map_err(|error| {
        let _ = fs::remove_file(&tmp);
        SessionPersistenceError::Io(error.kind())
    })
}

fn write_tmp(path: &Path, bytes: &[u8]) -> Result<(), SessionPersistenceError> {
    let mut file = fs::File::create(path).map_err(io_error)?;
    file.write_all(bytes)
        .map_err(|error| SessionPersistenceError::Io(error.kind()))?;
    file.sync_all()
        .map_err(|error| SessionPersistenceError::Io(error.kind()))?;
    Ok(())
}

/// Surface a missing file as the distinct [`SessionPersistenceError::NotFound`].
fn io_error(error: std::io::Error) -> SessionPersistenceError {
    match error.kind() {
        ErrorKind::NotFound => SessionPersistenceError::NotFound,
        kind => SessionPersistenceError::Io(kind),
    }
}

/// Clip hostile input embedded in an error message to a bounded length.
fn clip(text: impl AsRef<str>) -> String {
    let text = text.as_ref();
    let cut = text
        .char_indices()
        .nth(MAX_ERROR_DETAIL_CHARS)
        .map_or(text.len(), |(index, _)| index);
    let mut clipped: String = text[..cut].chars().collect();
    if cut < text.len() {
        clipped.push('…');
    }
    clipped
}

/// Reduce a third-party TOML parse error to a content-free position.
///
/// `toml_edit`'s `Display` quotes the offending source line — file content —
/// so none of its text is forwarded here. Only its byte span is consumed:
/// the span's start is translated into a 1-based line and column counted
/// from the document text itself (mirroring the line/column arithmetic of
/// `toml_edit`'s own rendering), which keeps the error actionable for a
/// user fixing the file without echoing any of it. A spanless error falls
/// back to position 1, 1 rather than guessing.
fn toml_error_position(text: &str, error: &toml_edit::TomlError) -> (usize, usize) {
    let Some(span) = error.span() else {
        return (1, 1);
    };
    let bytes = text.as_bytes();
    // An eof span points one past the last byte; clamp onto it like
    // `toml_edit` does so the position names the final character.
    let offset = span.start.min(bytes.len().saturating_sub(1));
    let line_start = bytes[..offset]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |newline| newline + 1);
    let line = 1 + bytes[..line_start]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count();
    let column = 1 + std::str::from_utf8(&bytes[line_start..=offset])
        .map_or(offset - line_start, |tail| tail.chars().count() - 1);
    (line, column)
}

#[cfg(test)]
mod tests {
    //! Unit checks for the serialization helpers. The exhaustive behavioral
    //! suite lives in the workspace integration test
    //! `tests/session_persistence.rs`.

    use super::*;

    #[test]
    fn toml_string_escapes_every_reserved_character() {
        assert_eq!(toml_string(""), "\"\"");
        assert_eq!(toml_string("plain"), "\"plain\"");
        assert_eq!(toml_string("a\"b"), "\"a\\\"b\"");
        assert_eq!(toml_string("a\\b"), "\"a\\\\b\"");
        assert_eq!(toml_string("a\tb"), "\"a\\tb\"");
        assert_eq!(toml_string("a\nb"), "\"a\\nb\"");
        assert_eq!(toml_string("a\rb"), "\"a\\rb\"");
        assert_eq!(toml_string("\u{8}\u{c}"), "\"\\b\\f\"");
        assert_eq!(toml_string("界"), "\"界\"");
    }

    #[test]
    fn toml_string_escapes_control_characters_as_unicode() {
        assert_eq!(toml_string("\u{0}"), "\"\\u0000\"");
        assert_eq!(toml_string("\u{1b}"), "\"\\u001B\"");
        assert_eq!(toml_string("\u{7f}"), "\"\\u007F\"");
    }

    #[test]
    fn the_format_constants_are_the_shipped_stop_condition() {
        assert_eq!(SESSION_STATE_VERSION, 1);
        assert_eq!(SESSION_STATE_FILE_NAME, "sessions.toml");
    }

    #[test]
    fn the_byte_bound_is_usable_on_every_platform() {
        assert!(usize::try_from(MAX_SESSION_STATE_BYTES).is_ok());
    }

    #[test]
    fn error_messages_stay_bounded() {
        let hostile = "a".repeat(10_000);
        let error = SessionPersistenceError::UnknownKey(clip(&hostile));
        assert!(error.to_string().len() < 1024);
        assert!(
            matches!(error, SessionPersistenceError::UnknownKey(ref key) if key.ends_with('…'))
        );
    }
}
