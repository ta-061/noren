//! Width-aware text projection for the sidebar view.
//!
//! Session rows reserve their final cell for a lifecycle marker. Identity may
//! truncate to a visible ASCII ellipsis, but the marker never competes with
//! the name for space. This projection stays separate from [`crate::sidebar`]
//! so the view model remains renderer- and geometry-independent.

use crate::sidebar::{SessionLifecycle, SidebarRow, SidebarView};
use noren_terminal::AnsiColor;

/// Shipped sidebar width in cell columns.
pub const DEFAULT_SIDEBAR_COLUMNS: usize = 16;

/// Marker glyphs in lifecycle order: starting, running, exited, failed.
///
/// These code points receive explicit, collision-checked 5x7 bitmaps in the
/// production renderer. They are shapes as well as colours, so remapped
/// palettes and colour-vision differences cannot collapse the four states.
pub const LIFECYCLE_MARKERS: [char; 4] = ['◌', '▶', '■', '✕'];

const ROW_PREFIX_COLUMNS: usize = 2;
const STATE_SUFFIX_COLUMNS: usize = 2;
const ELLIPSIS: &str = "...";

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
        '◌' => Some(AnsiColor::Yellow),
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
    if max_rows == 0 || columns == 0 {
        return Vec::new();
    }
    if sidebar.is_empty() {
        return sidebar
            .empty_state()
            .map(|state| vec![state.message().to_string()])
            .unwrap_or_default();
    }
    let offset = offset.min(sidebar.rows().len().saturating_sub(max_rows));
    sidebar.rows()[offset..]
        .iter()
        .take(max_rows)
        .map(|row| format_row(row, columns))
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
