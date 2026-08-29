//! Width-aware text projection for the sidebar view.
//!
//! Every entry row reserves a fixed cell for a kind shape and the same bounded
//! identity region. Rows carrying lifecycle reserve their final cell for the
//! #209 marker; rows without lifecycle leave that cell blank. Identity always
//! truncates to a visible ASCII ellipsis before the reserved suffix, so neither
//! a kind nor a lifecycle can be clipped by user-derived text. This projection
//! stays separate from [`crate::sidebar`] so the view model remains renderer-
//! and geometry-independent.

use crate::MAX_RENDER_COLS;
use crate::sidebar::{EntryKind, SessionLifecycle, SidebarRow, SidebarView};
use noren_terminal::AnsiColor;

/// Shipped sidebar width in cell columns.
pub const DEFAULT_SIDEBAR_COLUMNS: usize = 16;

/// Narrowest configurable sidebar: selection, kind, a complete ellipsis,
/// separator, and the reserved lifecycle cell all remain representable.
pub const MIN_SIDEBAR_COLUMNS: usize = 8;

/// Widest configurable sidebar while retaining one drawable terminal column.
pub const MAX_SIDEBAR_COLUMNS: usize = MAX_RENDER_COLS as usize - 1;

/// Marker glyphs in lifecycle order: starting, running, exited, failed.
///
/// These code points receive explicit, collision-checked 5x7 bitmaps in the
/// production renderer. They are shapes as well as colours, so remapped
/// palettes and colour-vision differences cannot collapse the four states.
pub const LIFECYCLE_MARKERS: [char; 4] = ['⌛', '▶', '■', '✕'];

/// Marker glyphs in [`EntryKind`] order: project, worktree, SSH, agent,
/// session.
///
/// Like [`LIFECYCLE_MARKERS`], these code points receive explicit,
/// collision-checked 5x7 bitmaps in the production renderer. Their shapes are
/// the primary signal and colour is reinforcement, so palette remapping and
/// colour-vision differences cannot collapse the five row kinds.
pub const KIND_MARKERS: [char; 5] = ['◆', '⑂', '⌁', '♟', '▣'];

const KIND_MARKER_COLUMN: usize = 1;
const IDENTITY_START_COLUMN: usize = 3;
const STATE_SUFFIX_COLUMNS: usize = 2;
const ELLIPSIS: &str = "...";

/// One projected sidebar row with its domain kind preserved for rendering.
///
/// Text alone cannot identify a lifecycle row: non-session labels are
/// user-derived and may place a marker-shaped character in the final visible
/// cell. `None` is reserved for chrome that has no workspace entry, such as
/// the empty-state notice or command palette rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarTextRow {
    text: String,
    kind: Option<EntryKind>,
    lifecycle: Option<SessionLifecycle>,
}

impl SidebarTextRow {
    /// Build a text-only chrome row that carries no workspace entry kind.
    #[must_use]
    pub fn chrome(text: String) -> Self {
        Self {
            text,
            kind: None,
            lifecycle: None,
        }
    }

    /// The text cells emitted for this row.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The domain entry kind, or `None` for non-entry chrome.
    #[must_use]
    pub const fn kind(&self) -> Option<EntryKind> {
        self.kind
    }

    /// Structured lifecycle carried by the final cell, or `None` when the
    /// row has no lifecycle signal.
    #[must_use]
    pub const fn lifecycle(&self) -> Option<SessionLifecycle> {
        self.lifecycle
    }

    fn entry(text: String, kind: EntryKind, lifecycle: Option<SessionLifecycle>) -> Self {
        Self {
            text,
            kind: Some(kind),
            lifecycle,
        }
    }

    fn into_text(self) -> String {
        self.text
    }
}

/// The marker glyph assigned to one lifecycle class.
#[must_use]
pub const fn lifecycle_marker(lifecycle: SessionLifecycle) -> char {
    match lifecycle {
        SessionLifecycle::Starting => LIFECYCLE_MARKERS[0],
        SessionLifecycle::Running => LIFECYCLE_MARKERS[1],
        SessionLifecycle::Exited => LIFECYCLE_MARKERS[2],
        SessionLifecycle::Failed => LIFECYCLE_MARKERS[3],
    }
}

/// Theme palette role reinforcing a lifecycle marker's shape.
#[must_use]
pub const fn lifecycle_marker_color(marker: char) -> Option<AnsiColor> {
    match marker {
        '⌛' => Some(AnsiColor::Yellow),
        '▶' => Some(AnsiColor::Green),
        '■' => Some(AnsiColor::BrightBlack),
        '✕' => Some(AnsiColor::Red),
        _ => None,
    }
}

/// The marker glyph assigned to one sidebar entry kind.
#[must_use]
pub const fn kind_marker(kind: EntryKind) -> char {
    match kind {
        EntryKind::Project => KIND_MARKERS[0],
        EntryKind::Worktree => KIND_MARKERS[1],
        EntryKind::SshConnection => KIND_MARKERS[2],
        EntryKind::Agent => KIND_MARKERS[3],
        EntryKind::Session => KIND_MARKERS[4],
    }
}

/// Theme palette role reinforcing a kind marker's shape.
#[must_use]
pub const fn kind_marker_color(kind: EntryKind) -> AnsiColor {
    match kind {
        EntryKind::Project => AnsiColor::BrightMagenta,
        EntryKind::Worktree => AnsiColor::Green,
        EntryKind::SshConnection => AnsiColor::Cyan,
        EntryKind::Agent => AnsiColor::Yellow,
        EntryKind::Session => AnsiColor::White,
    }
}

/// Format the visible sidebar slice at the shipped 16-column width.
#[must_use]
pub fn visible_sidebar_text_lines(
    sidebar: &SidebarView,
    offset: usize,
    max_rows: usize,
) -> Vec<String> {
    visible_sidebar_text_lines_at_width(sidebar, offset, max_rows, DEFAULT_SIDEBAR_COLUMNS)
}

/// Format the visible sidebar slice at an explicit cell width.
///
/// The scroll offset is clamped to the last full page so formatting work stays
/// proportional to visible rows. Every entry uses one grammar: selection,
/// kind shape, identity, a suffix separator, and an optional lifecycle marker.
/// Only the identity region truncates.
#[must_use]
pub fn visible_sidebar_text_lines_at_width(
    sidebar: &SidebarView,
    offset: usize,
    max_rows: usize,
    columns: usize,
) -> Vec<String> {
    visible_sidebar_text_rows_at_width(sidebar, offset, max_rows, columns)
        .into_iter()
        .map(SidebarTextRow::into_text)
        .collect()
}

/// Format visible rows while preserving the entry kind each line came from.
///
/// The renderer consumes this projection so kind and lifecycle colours are
/// gated by structured row facts, never by user-controlled text alone.
#[must_use]
pub fn visible_sidebar_text_rows_at_width(
    sidebar: &SidebarView,
    offset: usize,
    max_rows: usize,
    columns: usize,
) -> Vec<SidebarTextRow> {
    if max_rows == 0 || columns == 0 {
        return Vec::new();
    }
    if sidebar.is_empty() {
        return sidebar
            .empty_state()
            .map(|state| vec![SidebarTextRow::chrome(state.message().to_string())])
            .unwrap_or_default();
    }
    let offset = offset.min(sidebar.rows().len().saturating_sub(max_rows));
    sidebar.rows()[offset..]
        .iter()
        .take(max_rows)
        .map(|row| SidebarTextRow::entry(format_row(row, columns), row.kind(), row.lifecycle()))
        .collect()
}

fn format_row(row: &SidebarRow, columns: usize) -> String {
    // #209 already compresses a session's status into the reserved marker;
    // repeating its detail would force a fitting `session-N` identity to
    // ellipsize. Other kinds retain their secondary identity/status text and
    // pass through the same truncator as one string.
    let identity = match (row.kind(), row.detail()) {
        (EntryKind::Session, _) | (_, None) => row.label().to_owned(),
        (_, Some(detail)) => format!("{} {detail}", row.label()),
    };
    format_entry_row(
        row.is_selected(),
        row.kind(),
        &identity,
        row.lifecycle().map(lifecycle_marker),
        columns,
    )
}

fn format_entry_row(
    selected: bool,
    kind: EntryKind,
    identity: &str,
    state: Option<char>,
    columns: usize,
) -> String {
    let mut cells = vec![' '; columns];
    if columns > 0 {
        cells[0] = if selected { '>' } else { ' ' };
    }
    if columns > KIND_MARKER_COLUMN {
        cells[KIND_MARKER_COLUMN] = kind_marker(kind);
    }

    let identity_end = columns.saturating_sub(STATE_SUFFIX_COLUMNS);
    if identity_end > IDENTITY_START_COLUMN {
        let available = identity_end - IDENTITY_START_COLUMN;
        for (index, character) in truncated_label(identity, available).chars().enumerate() {
            cells[IDENTITY_START_COLUMN + index] = character;
        }
    }

    // Lifecycle owns the final cell even below the configurable width floor.
    // Writing it last preserves #209 if an internal caller supplies a tiny
    // synthetic width where the prefix cells overlap the suffix.
    if let Some(state) = state
        && let Some(last) = cells.last_mut()
    {
        *last = state;
    }

    cells.into_iter().collect()
}

fn truncated_label(label: &str, columns: usize) -> String {
    let count = label.chars().count();
    if count <= columns {
        return label.to_string();
    }
    let ellipsis_columns = ELLIPSIS.chars().count();
    if columns <= ellipsis_columns {
        return ELLIPSIS.chars().take(columns).collect();
    }
    label
        .chars()
        .take(columns - ellipsis_columns)
        .chain(ELLIPSIS.chars())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_kind_truncation(kind: EntryKind, state: Option<char>, expected: &str) {
        let line = format_entry_row(
            false,
            kind,
            "abcdefgh-identity-that-cannot-fit",
            state,
            DEFAULT_SIDEBAR_COLUMNS,
        );
        assert_eq!(line, expected);
        assert_eq!(line.chars().count(), DEFAULT_SIDEBAR_COLUMNS);
        assert_eq!(
            line.chars().nth(KIND_MARKER_COLUMN),
            Some(kind_marker(kind))
        );
        assert_eq!(&line.chars().skip(11).take(3).collect::<String>(), "...");
        assert_eq!(line.chars().nth(14), Some(' '));
        assert_eq!(line.chars().last(), state.or(Some(' ')));
    }

    #[test]
    fn long_session_identity_truncates_before_the_reserved_state_cell() {
        let line = format_entry_row(
            true,
            EntryKind::Session,
            "session-name-that-is-much-too-long",
            Some(lifecycle_marker(SessionLifecycle::Failed)),
            DEFAULT_SIDEBAR_COLUMNS,
        );

        assert_eq!(line, ">▣ session-... ✕");
        assert_eq!(line.chars().count(), DEFAULT_SIDEBAR_COLUMNS);
        assert_eq!(line.chars().last(), Some('✕'));
    }

    #[test]
    fn even_tiny_widths_keep_state_as_the_final_visible_cell() {
        for columns in 1..DEFAULT_SIDEBAR_COLUMNS {
            let line = format_entry_row(
                false,
                EntryKind::Session,
                "session-long",
                Some('▶'),
                columns,
            );
            assert_eq!(line.chars().count(), columns);
            assert_eq!(line.chars().last(), Some('▶'));
        }
    }

    #[test]
    fn every_kind_uses_its_own_shape_in_the_same_fixed_cell() {
        let cases = [
            (EntryKind::Project, '◆'),
            (EntryKind::Worktree, '⑂'),
            (EntryKind::SshConnection, '⌁'),
            (EntryKind::Agent, '♟'),
            (EntryKind::Session, '▣'),
        ];

        for (kind, marker) in cases {
            let line = format_entry_row(false, kind, "short", None, DEFAULT_SIDEBAR_COLUMNS);
            assert_eq!(line.chars().nth(KIND_MARKER_COLUMN), Some(marker));
            assert_eq!(line.chars().count(), DEFAULT_SIDEBAR_COLUMNS);
        }
    }

    #[test]
    fn every_kind_shows_the_complete_ellipsis_before_the_reserved_suffix() {
        for kind in [
            EntryKind::Project,
            EntryKind::Worktree,
            EntryKind::SshConnection,
            EntryKind::Agent,
            EntryKind::Session,
        ] {
            let state = (kind == EntryKind::Session).then_some('✕');
            let line = format_entry_row(
                false,
                kind,
                "identity-that-cannot-fit",
                state,
                DEFAULT_SIDEBAR_COLUMNS,
            );
            assert_eq!(&line.chars().skip(11).take(3).collect::<String>(), "...");
            assert_eq!(line.chars().count(), DEFAULT_SIDEBAR_COLUMNS);
            assert_eq!(line.chars().last(), state.or(Some(' ')));
        }
    }

    #[test]
    fn project_truncation_keeps_ellipsis_and_failed_lifecycle() {
        assert_kind_truncation(EntryKind::Project, Some('✕'), " ◆ abcdefgh... ✕");
    }

    #[test]
    fn worktree_truncation_keeps_ellipsis_and_blank_lifecycle_cell() {
        assert_kind_truncation(EntryKind::Worktree, None, " ⑂ abcdefgh...  ");
    }

    #[test]
    fn ssh_truncation_keeps_ellipsis_and_starting_lifecycle() {
        assert_kind_truncation(EntryKind::SshConnection, Some('⌛'), " ⌁ abcdefgh... ⌛");
    }

    #[test]
    fn agent_truncation_keeps_ellipsis_and_stopped_lifecycle() {
        assert_kind_truncation(EntryKind::Agent, Some('■'), " ♟ abcdefgh... ■");
    }

    #[test]
    fn session_truncation_keeps_ellipsis_and_running_lifecycle() {
        assert_kind_truncation(EntryKind::Session, Some('▶'), " ▣ abcdefgh... ▶");
    }

    #[test]
    fn every_supported_width_keeps_a_complete_ellipsis_before_the_final_cell() {
        for columns in MIN_SIDEBAR_COLUMNS..=32 {
            for kind in [
                EntryKind::Project,
                EntryKind::Worktree,
                EntryKind::SshConnection,
                EntryKind::Agent,
                EntryKind::Session,
            ] {
                let state = (kind != EntryKind::Worktree).then_some('✕');
                let line = format_entry_row(false, kind, &"x".repeat(columns + 20), state, columns);
                assert_eq!(line.chars().count(), columns);
                assert_eq!(
                    line.chars().skip(columns - 5).take(3).collect::<String>(),
                    "...",
                    "{kind:?} lost the ellipsis at {columns} columns: {line:?}"
                );
                assert_eq!(line.chars().nth(columns - 2), Some(' '));
                assert_eq!(line.chars().last(), state.or(Some(' ')));
            }
        }
    }
}
