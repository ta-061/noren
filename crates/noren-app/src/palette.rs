//! Renderer-independent command palette model.
//!
//! A command palette is a searchable catalog of commands identified by
//! **stable IDs** so that keybindings and future configuration can reference a
//! command without depending on its display text. This module defines only the
//! *model*: which commands exist, how a query matches them, and in what order
//! the matches are shown. It knows nothing about colors, geometry, or widget
//! types — a renderer reads [`Palette::search`] and decides what to paint, never
//! the reverse.
//!
//! # Boundary (ADR 0003)
//!
//! Noren manages the workspace *outside* the terminal; Zellij manages it
//! *inside*. The palette therefore offers **session and sidebar** commands
//! only — create, select, and close a session, and focus sidebar entries. It
//! never offers a pane, tab, split, or layout command: those are Zellij's, and
//! duplicating them is the exact failure this boundary exists to prevent. The
//! canonical command set reflects this — see [`CommandId`] and [`Palette::noren`].
//!
//! # Actions
//!
//! Each [`Command`] carries a dispatchable action of type `A`. The palette is
//! deliberately action-agnostic: it defines **no** session or sidebar action
//! enum of its own, so it cannot drift into a parallel vocabulary. At wire-up
//! (a separate, serial commit owned by another lane) `A` is bound to the shared
//! action type that reuses `SessionAction` from
//! `docs/coordination/decisions/D-M3-001-session-api.md`; this module supplies
//! only the stable IDs, labels, and matching policy.
//!
//! # Matching
//!
//! Search is **ASCII case-insensitive substring** over command labels, ranked by
//! the earliest match position with ties broken by catalog order. The query is
//! matched *literally*: it is never interpreted as regex or glob, so a query
//! made entirely of characters that are "special" elsewhere (such as
//! <code>\\(\\)\\[\\]</code>) is searched for verbatim and cannot panic. An
//! empty query matches every command in catalog order; a query that is a
//! substring of no label matches none. Non-ASCII case folding is intentionally
//! out of scope — only ASCII letters fold — so the matcher never implies a
//! capability it does not build.

use std::fmt;

/// A stable, opaque command identifier.
///
/// IDs are literal ASCII dotted paths (for example `"session.create"`). They are
/// compared and stored as opaque strings; they are never parsed as a regex or
/// glob, and the dotted shape carries no semantic meaning to this module. They
/// exist so keybindings and configuration can name a command without depending
/// on its mutable display text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CommandId(&'static str);

impl CommandId {
    /// Create a command ID from a static string.
    #[must_use]
    pub const fn new(id: &'static str) -> Self {
        Self(id)
    }

    /// The ID as a static string slice.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }

    /// **Create a new session.** Within the ADR 0003 boundary.
    pub const SESSION_CREATE: Self = Self::new("session.create");
    /// **Select (switch to) an existing session.** Within the ADR 0003 boundary.
    pub const SESSION_SELECT: Self = Self::new("session.select");
    /// **Close a session.** Within the ADR 0003 boundary.
    pub const SESSION_CLOSE: Self = Self::new("session.close");
    /// **Focus a sidebar entry.** Within the ADR 0003 boundary.
    pub const SIDEBAR_FOCUS: Self = Self::new("sidebar.focus");
}

impl AsRef<str> for CommandId {
    fn as_ref(&self) -> &str {
        self.0
    }
}

impl fmt::Display for CommandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// One palette entry: a stable [`CommandId`], a human-readable label, and the
/// action that dispatching the command performs.
///
/// The action type `A` is owned by the wiring layer (see the crate-level
/// docs); this module never instantiates a session or sidebar action itself.
#[derive(Clone, Debug)]
pub struct Command<A> {
    id: CommandId,
    label: &'static str,
    action: A,
}

impl<A> Command<A> {
    /// Create a command from its stable ID, display label, and action.
    #[must_use]
    pub const fn new(id: CommandId, label: &'static str, action: A) -> Self {
        Self { id, label, action }
    }

    /// The stable command ID.
    #[must_use]
    pub const fn id(&self) -> CommandId {
        self.id
    }

    /// The display label matched and shown by renderers.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        self.label
    }

    /// The action dispatching this command performs, by reference.
    #[must_use]
    pub const fn action(&self) -> &A {
        &self.action
    }

    /// Consume the command and return its action.
    #[must_use]
    pub fn into_action(self) -> A {
        self.action
    }
}

/// A ranked search hit: the matched command plus the char index in its label
/// where the match begins.
///
/// For an empty query, every command is returned as a hit with a match index of
/// zero. The match index counts [`char`]s, not bytes, so it is always valid
/// regardless of the label's UTF-8 width.
#[derive(Clone, Debug)]
pub struct SearchHit<'a, A> {
    command: &'a Command<A>,
    index: usize,
}

impl<'a, A> SearchHit<'a, A> {
    /// The matched command.
    #[must_use]
    pub const fn command(&self) -> &'a Command<A> {
        self.command
    }

    /// The char index in [`Command::label`] where the match begins.
    #[must_use]
    pub const fn match_index(&self) -> usize {
        self.index
    }
}

/// A searchable command catalog. Renderer-independent.
///
/// Build with [`Palette::from_commands`] or the canonical [`Palette::noren`]
/// set, then query with [`Palette::search`]. The catalog preserves insertion
/// order, which is the tie-breaker for search ranking.
#[derive(Clone, Debug)]
pub struct Palette<A> {
    commands: Vec<Command<A>>,
}

impl<A> Palette<A> {
    /// Create an empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    /// Create a catalog from an iterable of commands, preserving their order.
    #[must_use]
    pub fn from_commands(commands: impl IntoIterator<Item = Command<A>>) -> Self {
        Self {
            commands: commands.into_iter().collect(),
        }
    }

    /// Assemble Noren's canonical command catalog from the ADR 0003 boundary.
    ///
    /// The caller supplies the four dispatchable actions — one per stable
    /// command — so the palette introduces no action enum of its own. The four
    /// commands are exactly session create/select/close and sidebar focus;
    /// there is, by design, no pane, tab, split, or layout command.
    #[must_use]
    pub fn noren(session_create: A, session_select: A, session_close: A, sidebar_focus: A) -> Self {
        Self::from_commands([
            Command::new(CommandId::SESSION_CREATE, "New Session", session_create),
            Command::new(CommandId::SESSION_SELECT, "Switch Session", session_select),
            Command::new(CommandId::SESSION_CLOSE, "Close Session", session_close),
            Command::new(CommandId::SIDEBAR_FOCUS, "Focus Sidebar", sidebar_focus),
        ])
    }

    /// The number of commands in the catalog.
    #[must_use]
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Whether the catalog holds no commands.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Iterate over all commands in catalog order.
    pub fn iter(&self) -> impl Iterator<Item = &Command<A>> {
        self.commands.iter()
    }

    /// Find the first command with a given stable ID.
    ///
    /// Dispatch resolves an ID chosen by the user to its command and action;
    /// `None` means the ID is unknown to this catalog.
    #[must_use]
    pub fn get(&self, id: CommandId) -> Option<&Command<A>> {
        self.commands.iter().find(|command| command.id() == id)
    }

    /// Search the catalog with an ASCII case-insensitive substring query.
    ///
    /// See the crate-level matching policy: an empty query returns every
    /// command in catalog order (each hit's [`SearchHit::match_index`] is
    /// zero); a query that is a substring of no label returns an empty vector.
    /// The query is literal — never regex or glob — so any characters,
    /// including ones that look "escaped", are matched verbatim and cannot
    /// panic. Hits are ranked by the earliest match position in the label,
    /// with ties broken by catalog order.
    pub fn search<'a>(&'a self, query: &str) -> Vec<SearchHit<'a, A>> {
        let needle: Vec<char> = query.chars().map(|c| c.to_ascii_lowercase()).collect();
        if needle.is_empty() {
            return self
                .commands
                .iter()
                .map(|command| SearchHit { command, index: 0 })
                .collect();
        }
        let mut hits: Vec<SearchHit<'_, A>> = self
            .commands
            .iter()
            .filter_map(|command| {
                let label: Vec<char> = command
                    .label()
                    .chars()
                    .map(|c| c.to_ascii_lowercase())
                    .collect();
                substring_index(&label, &needle).map(|index| SearchHit { command, index })
            })
            .collect();
        // `hits` is already in catalog order (filter_map preserves it); a
        // stable sort by match index therefore keeps catalog order as the
        // tie-breaker for equal match positions.
        hits.sort_by_key(|hit| hit.index);
        hits
    }
}

impl<A> Default for Palette<A> {
    fn default() -> Self {
        Self::new()
    }
}

/// First index at which `needle` occurs as a contiguous slice of `haystack`, or
/// `None`. Both inputs are already ASCII-case-folded by the caller. An empty
/// needle matches at index zero.
fn substring_index(haystack: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
