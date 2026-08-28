//! Width-aware text projection for the sidebar view.
//!
//! Session rows reserve their final cell for a lifecycle marker. Identity may
//! truncate to a visible ASCII ellipsis, but the marker never competes with
//! the name for space. This projection stays separate from [`crate::sidebar`]
//! so the view model remains renderer- and geometry-independent.

use crate::MAX_RENDER_COLS;
use crate::sidebar::{EntryKind, SessionLifecycle, SidebarRow, SidebarView};
use noren_terminal::AnsiColor;

/// Shipped sidebar width in cell columns.
pub const DEFAULT_SIDEBAR_COLUMNS: usize = 16;

/// Narrowest configurable sidebar: selection, one identity cell, ellipsis,
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

const ROW_PREFIX_COLUMNS: usize = 2;
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
}

impl SidebarTextRow {
    /// Build a text-only chrome row that carries no workspace entry kind.
    #[must_use]
    pub fn chrome(text: String) -> Self {
        Self { text, kind: None }
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

    fn entry(text: String, kind: EntryKind) -> Self {
        Self {
            text,
            kind: Some(kind),
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
/// proportional to visible rows. Non-session rows preserve their established
/// text. Session rows reserve the last cell for lifecycle and truncate only
/// the identity region.
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
/// The renderer consumes this projection so semantic lifecycle colour is
/// gated by `EntryKind::Session`, never by user-controlled text alone.
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
        .map(|row| SidebarTextRow::entry(format_row(row, columns), row.kind()))
        .collect()
}

fn format_row(row: &SidebarRow, columns: usize) -> String {
    match row.lifecycle() {
        Some(lifecycle) => format_session_row(
            row.is_selected(),
            row.label(),
            lifecycle_marker(lifecycle),
            columns,
        ),
        None => {
            let selection = if row.is_selected() { '>' } else { ' ' };
            match row.detail() {
                Some(detail) => format!("{selection} {} {detail}", row.label()),
                None => format!("{selection} {}", row.label()),
            }
        }
    }
}

fn format_session_row(selected: bool, label: &str, state: char, columns: usize) -> String {
    let mut cells = vec![' '; columns];
    let last = columns - 1;
    cells[last] = state;

    if columns > 1 {
        cells[0] = if selected { '>' } else { ' ' };
    }

    let name_end = columns.saturating_sub(STATE_SUFFIX_COLUMNS);
    if name_end > ROW_PREFIX_COLUMNS {
        let available = name_end - ROW_PREFIX_COLUMNS;
        for (index, character) in truncated_label(label, available).chars().enumerate() {
            cells[ROW_PREFIX_COLUMNS + index] = character;
        }
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
        return label.chars().take(columns).collect();
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

    #[test]
    fn long_session_identity_truncates_before_the_reserved_state_cell() {
        let line = format_session_row(
            true,
            "session-name-that-is-much-too-long",
            lifecycle_marker(SessionLifecycle::Failed),
            DEFAULT_SIDEBAR_COLUMNS,
        );

        assert_eq!(line, "> session-n... ✕");
        assert_eq!(line.chars().count(), DEFAULT_SIDEBAR_COLUMNS);
        assert_eq!(line.chars().last(), Some('✕'));
    }

    #[test]
    fn even_tiny_widths_keep_state_as_the_final_visible_cell() {
        for columns in 1..DEFAULT_SIDEBAR_COLUMNS {
            let line = format_session_row(false, "session-long", '▶', columns);
            assert_eq!(line.chars().count(), columns);
            assert_eq!(line.chars().last(), Some('▶'));
        }
    }
}
