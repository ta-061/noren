//! Optional, bounded TOML configuration for the PoC.
//!
//! A missing or empty file behaves exactly as the built-in defaults: every
//! setting is optional and defaults to the constant the app already uses. A
//! file that exists but is malformed, non-UTF-8, oversized, or holds an
//! out-of-range value is a hard error, never a silent fallback to defaults,
//! because configuration is untrusted input ([`docs/security/threat-model.md`]
//! (../../../docs/security/threat-model.md)).
//!
//! Hard ceilings from the terminal foundation can never be raised by
//! configuration: the grid derived from font cell sizes stays clamped by
//! [`crate::MAX_RENDER_ROWS`] by [`crate::MAX_RENDER_COLS`], far inside
//! `MAX_SCREEN_CELLS`, and only values at least as large as the renderer's
//! built-in cell constants are accepted, so a configured grid can never
//! exceed the grid the renderer draws. Keys this schema cannot apply are
//! rejected as unknown rather than parsed and silently ignored; see
//! `docs/configuration.md` for the reserved settings and the lane work each
//! one is waiting on.
//!
//! The `[keys]` table configures the workspace key chords (the palette
//! opener and the four palette command shortcuts). It follows the same
//! discipline: an absent table keeps the compiled-in defaults, an
//! unparseable chord is a typed [`ConfigError::InvalidChord`] naming the
//! offending key and value, an unknown action name is
//! [`ConfigError::UnknownKey`], two actions on one chord is
//! [`ConfigError::DuplicateChord`], and a palette chord the pass-through
//! policy could never claim (a pinned Zellij default or the frozen
//! `Super+Escape` exit leader) is [`ConfigError::UnclaimableChord`] rather
//! than a silently dead binding.
//!
//! The `[theme]` table selects the built-in colour palette. It follows the
//! same discipline: an absent table keeps `dark` — exactly the colours the
//! app shipped with before themes existed — a non-string value is
//! [`ConfigError::ThemeNotAString`], and a name outside the closed
//! vocabulary (`dark`, `light`, `high-contrast`) is
//! [`ConfigError::UnknownTheme`] naming the offending value, never a
//! silent fallback to the default.
//!
//! The `[[agents]]` array configures AI-agent sidebar entries: a display
//! `name` and the `command` (plus optional `args`) to launch when the row is
//! selected. It follows the same discipline — unknown keys are rejected,
//! every value is type- and range-checked, and the agent launch is an **argv
//! vector, never a shell invocation**: a value containing `;`, `$(...)`, or a
//! backtick is passed to the agent program as literal data, because no shell
//! ever interprets it. The command must be an absolute path; `PATH` lookup is
//! deliberately not performed, so a writable `PATH` entry cannot substitute a
//! different binary.
//!
//! The `[[projects]]` array configures project sidebar entries: a display
//! `name` and the absolute `root` directory a session starts in when the row
//! is selected. Projects are configured, not discovered, because a project
//! has no authoritative machine-readable source (unlike a worktree, whose
//! source of truth is `git worktree list --porcelain`): scanning a directory
//! tree for `.git` folders would be slow, unbounded, and would guess at the
//! user's intent. The same discipline applies — unknown keys are rejected,
//! every value is type- and range-checked, the root must be an absolute path
//! (leading `/`; neither `~` expansion nor relative resolution against the
//! launch directory is performed, so the configured text and the launched
//! directory can never silently diverge), and the root text is never echoed
//! in any error message or `Debug` output.
//!
//! # Privacy
//!
//! Configuration carries no secrets: no supported key names a credential,
//! key, or other sensitive value. The shell program is **not** configurable:
//! the threat model (TM-01) fixes the spawn at `/bin/zsh` with no configured
//! additions. The two path-shaped surfaces — an agent `command` and a project
//! `root` — are different in kind from a credential: each names a program or
//! directory the user explicitly asked Noren to launch or open, and each is
//! validated (absolute, bounded) rather than forbidden. A path can still
//! embed a username or a private directory name, so neither surface is ever
//! echoed through an error `Display` or a `Debug` rendering (issue #146).

use crate::passthrough::{
    CLAIM_ID_PALETTE, Chord, ChordError, ChordSeq, KeyCode, Modifiers, PassthroughAction,
    PassthroughClaim, PassthroughPolicy, default_exit_claim,
};
use crate::theme::{Theme, ThemeName};
use crate::{POC_CELL_HEIGHT, POC_CELL_WIDTH};
use std::env;
use std::fmt;
use std::fs;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, TableLike};

/// Maximum accepted configuration file size in bytes.
///
/// The read is streamed and bounded, so even a pathological path (for example
/// a symlink to a device node) cannot grow memory past this cap; oversized
/// input is a [`ConfigError::TooLarge`] error instead.
pub const MAX_CONFIG_BYTES: u64 = 64 * 1024;

/// Largest accepted font cell edge in physical pixels.
///
/// Zero is rejected outright because grid division would fault; the ceiling
/// is policy, keeping absurdly large values an explicit error rather than a
/// silently degenerate one-cell grid.
pub const MAX_CELL_EDGE: u32 = 1024;

/// Environment variable naming an explicit configuration file path.
///
/// When set and non-empty, the file must exist and parse: absence is a
/// [`ConfigError::NotFound`] error rather than a silent default, because the
/// override expresses explicit intent.
pub const CONFIG_ENV_VAR: &str = "NOREN_CONFIG";

/// Configuration file name inside the macOS application-support directory.
pub const CONFIG_FILE_NAME: &str = "config.toml";

/// Maximum characters of hostile input echoed inside any error message.
const MAX_ERROR_DETAIL_CHARS: usize = 120;

/// Maximum accepted bytes for one `[agents]` string field (`name`,
/// `command`, or one `args` element).
///
/// Policy, like [`MAX_CELL_EDGE`]: keeps a single field from monopolizing
/// the bounded 64 KiB read, and gives the sidebar label arithmetic a
/// reasonable worst case. Longer values are
/// [`ConfigError::OutOfRange`] errors, never truncated.
///
/// This equals [`noren_pty::MAX_AGENT_ARGV_ELEMENT_BYTES`] — the launch
/// policy re-enforces the same cap at its own layer — and a pin test
/// below holds the two constants equal so the layers can never diverge.
pub const MAX_AGENT_FIELD_BYTES: usize = 1024;

/// Maximum accepted bytes for one `[[projects]]` string field (`name` or
/// `root`).
///
/// Policy, like [`MAX_AGENT_FIELD_BYTES`] and [`MAX_CELL_EDGE`]: keeps a
/// single field from monopolizing the bounded 64 KiB read, and gives the
/// sidebar label arithmetic the same reasonable worst case as an agent name.
/// Longer values are [`ConfigError::OutOfRange`] errors, never truncated.
/// A pin test below holds this equal to [`MAX_AGENT_FIELD_BYTES`] so the
/// two array-of-tables sections cannot drift apart.
pub const MAX_PROJECT_FIELD_BYTES: usize = 1024;

/// Font geometry settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontConfig {
    cell_width: u32,
    cell_height: u32,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            cell_width: POC_CELL_WIDTH,
            cell_height: POC_CELL_HEIGHT,
        }
    }
}

impl FontConfig {
    /// Cell width in physical pixels; defaults to [`POC_CELL_WIDTH`].
    ///
    /// Validation rejects values below the renderer's built-in
    /// [`POC_CELL_WIDTH`] or above [`MAX_CELL_EDGE`], keeping the configured
    /// grid inside what the renderer draws.
    #[must_use]
    pub const fn cell_width(self) -> u32 {
        self.cell_width
    }

    /// Cell height in physical pixels; defaults to [`POC_CELL_HEIGHT`].
    ///
    /// Validation rejects values below the renderer's built-in
    /// [`POC_CELL_HEIGHT`] or above [`MAX_CELL_EDGE`], keeping the configured
    /// grid inside what the renderer draws.
    #[must_use]
    pub const fn cell_height(self) -> u32 {
        self.cell_height
    }
}

/// Configurable workspace key chords.
///
/// [`KeymapConfig::default`] is exactly the chord set the app shipped with
/// before configuration existed: `super+p` opens the palette, and the bare
/// characters `c`/`s`/`x`/`f` dispatch the four palette commands. Every
/// chord is a normalized pass-through [`Chord`], so the binary compares
/// configured bindings against live key events without re-parsing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeymapConfig {
    palette_open: Chord,
    session_create: Chord,
    session_select: Chord,
    session_close: Chord,
    sidebar_focus: Chord,
}

impl Default for KeymapConfig {
    fn default() -> Self {
        Self {
            palette_open: default_chord(KeyCode::Char('p'), Modifiers::empty().super_key()),
            session_create: default_chord(KeyCode::Char('c'), Modifiers::empty()),
            session_select: default_chord(KeyCode::Char('s'), Modifiers::empty()),
            session_close: default_chord(KeyCode::Char('x'), Modifiers::empty()),
            sidebar_focus: default_chord(KeyCode::Char('f'), Modifiers::empty()),
        }
    }
}

impl KeymapConfig {
    /// The chord opening the command palette; defaults to `super+p`.
    #[must_use]
    pub const fn palette_open(self) -> Chord {
        self.palette_open
    }

    /// The palette command chord dispatching `session.create`; defaults to `c`.
    #[must_use]
    pub const fn session_create(self) -> Chord {
        self.session_create
    }

    /// The palette command chord dispatching `session.select`; defaults to `s`.
    #[must_use]
    pub const fn session_select(self) -> Chord {
        self.session_select
    }

    /// The palette command chord dispatching `session.close`; defaults to `x`.
    #[must_use]
    pub const fn session_close(self) -> Chord {
        self.session_close
    }

    /// The palette command chord dispatching `sidebar.focus`; defaults to `f`.
    #[must_use]
    pub const fn sidebar_focus(self) -> Chord {
        self.sidebar_focus
    }
}

/// A default keymap chord constant; the values are printable characters and
/// the Super modifier, which always normalize.
fn default_chord(code: KeyCode, modifiers: Modifiers) -> Chord {
    Chord::new(code, modifiers).expect("default keymap chords are normalized constants")
}

/// Colour theme selection.
///
/// [`ThemeConfig::default`] is `dark`: exactly the palette the app shipped
/// with before themes existed, so an absent `[theme]` table preserves the
/// pre-theme rendering bit-for-bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ThemeConfig {
    name: ThemeName,
}

impl ThemeConfig {
    /// The selected built-in theme name; defaults to `dark`.
    #[must_use]
    pub const fn name(self) -> ThemeName {
        self.name
    }

    /// The concrete palette the selected name resolves to.
    #[must_use]
    pub const fn palette(self) -> Theme {
        self.name.palette()
    }
}

/// One configured agent: a display name plus the argv vector of its launch.
///
/// The command is executed **without a shell** (see [`crate::config`]'s
/// module documentation): `command` is `argv[0]` and each `args` element is
/// one argv word, so shell metacharacters in any field are literal data to
/// the agent program, never an injection. The command must be an absolute
/// path; no `PATH` lookup is performed.
///
/// # Debug discipline
///
/// [`fmt::Debug`] is shape-only (issue #146): the name and command are
/// user-authored file text, and a command can embed a private path, so
/// neither is printed. Use the accessors for real values.
#[derive(Clone, PartialEq, Eq)]
pub struct AgentConfig {
    name: String,
    command: String,
    args: Vec<String>,
}

impl fmt::Debug for AgentConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentConfig")
            .field("name_chars", &self.name.chars().count())
            .field("argv", &self.argv().len())
            .finish_non_exhaustive()
    }
}

impl AgentConfig {
    /// The display name shown on the agent's sidebar row.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The absolute program path; `argv[0]` of the launch.
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }

    /// The argv words after the program, in configured order.
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// The full argv vector: `command` followed by `args`.
    #[must_use]
    pub fn argv(&self) -> Vec<String> {
        let mut argv = Vec::with_capacity(self.args.len() + 1);
        argv.push(self.command.clone());
        argv.extend(self.args.iter().cloned());
        argv
    }
}

/// One configured project: a display name plus the absolute root directory a
/// session starts in when the project's sidebar row is selected.
///
/// The root is a directory path the user explicitly asked Noren to open, so
/// it is validated (absolute, bounded) rather than forbidden — the same
/// reasoning that admits an agent `command`. It is never checked for
/// existence at load time: a configured-but-missing directory is a runtime
/// fact, refused visibly when the row is selected (exactly like a
/// registered-but-deleted worktree), not a load-time guess.
///
/// # Debug discipline
///
/// [`fmt::Debug`] is shape-only (issue #146): the name is user-authored file
/// text, and a root can embed a username or a private directory name, so
/// neither is printed. Use the accessors for real values.
#[derive(Clone, PartialEq, Eq)]
pub struct ProjectConfig {
    name: String,
    root: String,
}

impl fmt::Debug for ProjectConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProjectConfig")
            .field("name_chars", &self.name.chars().count())
            .field("root_chars", &self.root.chars().count())
            .finish_non_exhaustive()
    }
}

impl ProjectConfig {
    /// The display name shown on the project's sidebar row.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The absolute root directory, as the configuration spelled it. The
    /// launch layer validates it (absolute, existing directory) again.
    #[must_use]
    pub fn root(&self) -> &str {
        &self.root
    }
}

/// Validated application configuration.
///
/// [`AppConfig::default`] is byte-for-byte the behavior the app had before
/// configuration existed; [`AppConfig::load`] returns it for a missing file.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AppConfig {
    font: FontConfig,
    keys: KeymapConfig,
    theme: ThemeConfig,
    agents: Vec<AgentConfig>,
    projects: Vec<ProjectConfig>,
}

impl AppConfig {
    /// Font geometry settings.
    #[must_use]
    pub const fn font(&self) -> FontConfig {
        self.font
    }

    /// Workspace key chord settings.
    #[must_use]
    pub const fn keys(&self) -> KeymapConfig {
        self.keys
    }

    /// Colour theme settings.
    #[must_use]
    pub const fn theme(&self) -> ThemeConfig {
        self.theme
    }

    /// Configured agents, in file order. Empty when `[agents]` is absent.
    #[must_use]
    pub fn agents(&self) -> &[AgentConfig] {
        &self.agents
    }

    /// Configured projects, in file order. Empty when `[projects]` is absent.
    #[must_use]
    pub fn projects(&self) -> &[ProjectConfig] {
        &self.projects
    }

    /// Load configuration from the standard path or the [`CONFIG_ENV_VAR`]
    /// override.
    ///
    /// A missing file at the standard path means defaults. A path named by
    /// the override must exist. Any file that exists must read, decode, and
    /// validate, or the call errors.
    pub fn load() -> Result<Self, ConfigError> {
        Self::resolve(env::var_os(CONFIG_ENV_VAR).as_deref())
    }

    /// Resolve configuration from an optional explicit path override.
    pub fn resolve(override_path: Option<&std::ffi::OsStr>) -> Result<Self, ConfigError> {
        match override_path.filter(|value| !value.is_empty()) {
            Some(value) => Self::load_from(Path::new(value)),
            None => match default_path() {
                Some(path) if path.is_file() => Self::load_from(&path),
                _ => Ok(Self::default()),
            },
        }
    }

    /// Load and validate configuration from an explicit path.
    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        let bytes = read_bounded(path)?;
        Self::from_bytes(&bytes)
    }

    /// Validate configuration from raw file bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ConfigError> {
        let text = std::str::from_utf8(bytes).map_err(|_| ConfigError::NotUtf8)?;
        Self::parse(text)
    }

    /// Validate configuration from its TOML text.
    ///
    /// Unknown keys, wrong value types, and out-of-range values are errors so
    /// a typo or hostile value can never masquerade as a working setting.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let document: DocumentMut = text.parse().map_err(|error: toml_edit::TomlError| {
            let (line, column) = toml_error_position(text, &error);
            ConfigError::Parse { line, column }
        })?;
        let mut config = Self::default();
        for (key, item) in document.as_table().iter() {
            match key {
                // `[[agents]]` and `[[projects]]` are arrays of tables, not
                // tables, so they are matched before the table-like
                // conversion below.
                "agents" => parse_agents(item, &mut config.agents)?,
                "projects" => parse_projects(item, &mut config.projects)?,
                _ => {
                    let table = item
                        .as_table_like()
                        .ok_or_else(|| ConfigError::WrongType { key: clip(key) })?;
                    match key {
                        "font" => parse_font(table, &mut config.font)?,
                        "keys" => parse_keys(table, &mut config.keys)?,
                        "theme" => parse_theme(table, &mut config.theme)?,
                        // `[terminal]` historically attracted a `scrollback_lines` key.
                        // The terminal foundation has no configurable retention cap yet,
                        // so the table is rejected instead of parsed-and-ignored.
                        _ => return Err(ConfigError::UnknownKey(clip(key))),
                    }
                }
            }
        }
        Ok(config)
    }
}

/// Apply the `[[agents]]` array to the configured agent list.
///
/// The array spelling is the only accepted form (`agents = [...]` with
/// inline tables is rejected, not silently reinterpreted), every entry must
/// carry a string `name` and a string absolute `command`, and the optional
/// `args` array must hold only strings. Unknown entry keys are rejected like
/// every other unknown key in this schema.
fn parse_agents(item: &Item, agents: &mut Vec<AgentConfig>) -> Result<(), ConfigError> {
    let entries = item
        .as_array_of_tables()
        .ok_or_else(|| ConfigError::AgentTableNotAnArray {
            key: clip("agents"),
        })?;
    for entry in entries.iter() {
        let mut name: Option<String> = None;
        let mut command: Option<String> = None;
        let mut args: Vec<String> = Vec::new();
        for (key, value) in entry.iter() {
            match key {
                "name" => name = Some(agent_string_field(key, value)?),
                "command" => {
                    let program = agent_string_field(key, value)?;
                    if !program.starts_with('/') {
                        return Err(ConfigError::AgentCommandNotAbsolute { key: clip(key) });
                    }
                    command = Some(program);
                }
                "args" => {
                    let array = value
                        .as_array()
                        .ok_or_else(|| ConfigError::AgentArgsNotAnArray { key: clip(key) })?;
                    for (index, argument) in array.iter().enumerate() {
                        let text = argument
                            .as_str()
                            .ok_or(ConfigError::AgentArgNotAString { index })?;
                        if text.len() > MAX_AGENT_FIELD_BYTES {
                            return Err(ConfigError::OutOfRange { key: clip(key) });
                        }
                        args.push(text.to_owned());
                    }
                }
                _ => return Err(ConfigError::UnknownKey(clip(key))),
            }
        }
        let name = name.ok_or_else(|| ConfigError::AgentFieldMissing { key: clip("name") })?;
        let command = command.ok_or_else(|| ConfigError::AgentFieldMissing {
            key: clip("command"),
        })?;
        agents.push(AgentConfig {
            name,
            command,
            args,
        });
    }
    Ok(())
}

/// Read one required `[agents]` string field (`name` or `command`), enforcing
/// the shared type and length rules.
fn agent_string_field(key: &str, item: &Item) -> Result<String, ConfigError> {
    let text = item
        .as_str()
        .ok_or_else(|| ConfigError::AgentFieldNotAString { key: clip(key) })?;
    if text.is_empty() || text.len() > MAX_AGENT_FIELD_BYTES {
        return Err(ConfigError::OutOfRange { key: clip(key) });
    }
    Ok(text.to_owned())
}

/// Apply the `[[projects]]` array to the configured project list.
///
/// The array spelling is the only accepted form (`projects = [...]` with
/// inline tables is rejected, not silently reinterpreted), every entry must
/// carry a string `name` and a string absolute `root`, and unknown entry keys
/// are rejected like every other unknown key in this schema. The root must
/// start with `/`: neither `~` expansion nor resolution against the launch
/// directory is performed, so the configured text and the directory the
/// session starts in can never silently diverge.
fn parse_projects(item: &Item, projects: &mut Vec<ProjectConfig>) -> Result<(), ConfigError> {
    let entries = item
        .as_array_of_tables()
        .ok_or_else(|| ConfigError::ProjectTableNotAnArray {
            key: clip("projects"),
        })?;
    for entry in entries.iter() {
        let mut name: Option<String> = None;
        let mut root: Option<String> = None;
        for (key, value) in entry.iter() {
            match key {
                "name" => name = Some(project_string_field(key, value)?),
                "root" => {
                    let root_text = project_string_field(key, value)?;
                    if !root_text.starts_with('/') {
                        return Err(ConfigError::ProjectRootNotAbsolute { key: clip(key) });
                    }
                    root = Some(root_text);
                }
                _ => return Err(ConfigError::UnknownKey(clip(key))),
            }
        }
        let name = name.ok_or_else(|| ConfigError::ProjectFieldMissing { key: clip("name") })?;
        let root = root.ok_or_else(|| ConfigError::ProjectFieldMissing { key: clip("root") })?;
        projects.push(ProjectConfig { name, root });
    }
    Ok(())
}

/// Read one required `[[projects]]` string field (`name` or `root`),
/// enforcing the shared type and length rules.
fn project_string_field(key: &str, item: &Item) -> Result<String, ConfigError> {
    let text = item
        .as_str()
        .ok_or_else(|| ConfigError::ProjectFieldNotAString { key: clip(key) })?;
    if text.is_empty() || text.len() > MAX_PROJECT_FIELD_BYTES {
        return Err(ConfigError::OutOfRange { key: clip(key) });
    }
    Ok(text.to_owned())
}

fn parse_font(table: &dyn TableLike, font: &mut FontConfig) -> Result<(), ConfigError> {
    let max_edge = usize::try_from(MAX_CELL_EDGE).unwrap_or(usize::MAX);
    let min_width = usize::try_from(POC_CELL_WIDTH).expect("POC cell width fits in usize");
    let min_height = usize::try_from(POC_CELL_HEIGHT).expect("POC cell height fits in usize");
    for (key, item) in table.iter() {
        match key {
            // The PoC renderer draws with the built-in cell constants, so a
            // configured grid must never exceed the grid the renderer can
            // show: values below the renderer floor are rejected rather than
            // silently hide terminal content.
            "cell_width" => {
                font.cell_width = integer_in_range(key, item, min_width, max_edge)?
                    .try_into()
                    .expect("range-checked value fits u32");
            }
            "cell_height" => {
                font.cell_height = integer_in_range(key, item, min_height, max_edge)?
                    .try_into()
                    .expect("range-checked value fits u32");
            }
            _ => return Err(ConfigError::UnknownKey(clip(key))),
        }
    }
    Ok(())
}

fn integer_in_range(key: &str, item: &Item, min: usize, max: usize) -> Result<usize, ConfigError> {
    let value = item
        .as_integer()
        .ok_or_else(|| ConfigError::WrongType { key: clip(key) })?;
    let value = usize::try_from(value)
        .ok()
        .filter(|value| (min..=max).contains(value))
        .ok_or_else(|| ConfigError::OutOfRange { key: clip(key) })?;
    Ok(value)
}

/// Apply the `[theme]` table to the theme selection.
///
/// Every value must be a TOML string naming one of the built-in themes,
/// matched exactly (the vocabulary is closed and case-sensitive). An
/// unknown name is [`ConfigError::UnknownTheme`] naming the offending
/// value — never a silent fallback to `dark`, because a typo would
/// otherwise masquerade as a working setting.
fn parse_theme(table: &dyn TableLike, theme: &mut ThemeConfig) -> Result<(), ConfigError> {
    for (key, item) in table.iter() {
        match key {
            "name" => {
                let value = item
                    .as_str()
                    .ok_or_else(|| ConfigError::ThemeNotAString { key: clip(key) })?;
                theme.name = ThemeName::parse(value)
                    .ok_or_else(|| ConfigError::UnknownTheme { value: clip(value) })?;
            }
            _ => return Err(ConfigError::UnknownKey(clip(key))),
        }
    }
    Ok(())
}

/// Apply the `[keys]` table to the keymap.
/// Every TOML key must name a known action and every value must be a string
/// holding one parseable chord. Palette-command chords must avoid the keys
/// the open palette always interprets structurally (Escape, Enter, and the
/// vertical arrows), the palette opener must stay claimable by the
/// pass-through policy, and after applying the table all five chords must
/// stay pairwise distinct — including the chords of actions the table left
/// at their defaults.
fn parse_keys(table: &dyn TableLike, keys: &mut KeymapConfig) -> Result<(), ConfigError> {
    for (key, item) in table.iter() {
        // Unknown action names are rejected before the value is examined,
        // so a sub-table named by a typo reports the unknown action, not a
        // value-type complaint.
        let value = match key {
            "palette_open" | "session_create" | "session_select" | "session_close"
            | "sidebar_focus" => item
                .as_str()
                .ok_or_else(|| ConfigError::ChordNotAString { key: clip(key) })?,
            _ => return Err(ConfigError::UnknownKey(clip(key))),
        };
        match key {
            "palette_open" => {
                let chord = parse_configured_chord(key, value)?;
                keys.palette_open = validate_palette_claim(chord, key, value)?;
            }
            "session_create" => keys.session_create = parse_command_chord(key, value)?,
            "session_select" => keys.session_select = parse_command_chord(key, value)?,
            "session_close" => keys.session_close = parse_command_chord(key, value)?,
            "sidebar_focus" => keys.sidebar_focus = parse_command_chord(key, value)?,
            // Unreachable in practice: the value match above rejected every
            // other name as unknown.
            _ => return Err(ConfigError::UnknownKey(clip(key))),
        }
    }
    ensure_distinct_chords(keys)
}

/// Parse one `[keys]` value into a chord, blaming the key and value on
/// failure.
///
/// The binary claims Super+A/C/V (clipboard) and Super+D (diagnostics)
/// ahead of every configured path — palette commands included, because
/// `handle_key` consults them first even while the palette is open — and
/// matches them with any combination of further modifiers held. A chord
/// there would be dead configuration, so it is rejected here like every
/// other unclaimable binding.
fn parse_configured_chord(key: &str, value: &str) -> Result<Chord, ConfigError> {
    let chord = parse_chord(value).map_err(|error| ConfigError::InvalidChord {
        key: clip(key),
        value: clip(value),
        reason: clip(error.to_string()),
    })?;
    if chord.modifiers().is_super()
        && matches!(
            chord.code(),
            KeyCode::Char('a') | KeyCode::Char('c') | KeyCode::Char('v') | KeyCode::Char('d')
        )
    {
        return Err(ConfigError::ReservedChord {
            key: clip(key),
            value: clip(value),
        });
    }
    Ok(chord)
}

/// Parse and validate one palette-command chord. The four command chords are
/// matched only while the palette is open, so they cannot steal a chord from
/// Zellij or the child, but the open palette interprets Escape, Enter, and
/// the vertical arrows structurally before chord dispatch — a command bound
/// there would be dead configuration, so it is rejected.
fn parse_command_chord(key: &str, value: &str) -> Result<Chord, ConfigError> {
    let chord = parse_configured_chord(key, value)?;
    if matches!(
        chord.code(),
        KeyCode::Escape | KeyCode::Enter | KeyCode::Up | KeyCode::Down
    ) {
        return Err(ConfigError::ReservedChord {
            key: clip(key),
            value: clip(value),
        });
    }
    Ok(chord)
}

/// Validate that the pass-through policy could actually claim the configured
/// palette chord, using the same enforcement point the live policy uses.
///
/// A chord colliding with a pinned Zellij default, or shadowed by / shadowing
/// the frozen `Super+Escape` exit leader, could never open the palette;
/// accepting it would be a silently dead binding.
fn validate_palette_claim(chord: Chord, key: &str, value: &str) -> Result<Chord, ConfigError> {
    let probe = PassthroughClaim {
        id: CLAIM_ID_PALETTE,
        action: PassthroughAction::OpenCommandPalette,
        seq: ChordSeq::single(chord),
        justification: "configuration validation probe",
    };
    match PassthroughPolicy::try_new(vec![default_exit_claim(), probe]) {
        Ok(_) => Ok(chord),
        Err(_) => Err(ConfigError::UnclaimableChord {
            key: clip(key),
            value: clip(value),
        }),
    }
}

/// Reject any chord bound to two of the five actions.
///
/// The check runs on the final keymap, so it also catches a configured value
/// that silently collides with an action the table left at its default.
fn ensure_distinct_chords(keys: &KeymapConfig) -> Result<(), ConfigError> {
    let bindings = [
        ("palette_open", keys.palette_open),
        ("session_create", keys.session_create),
        ("session_select", keys.session_select),
        ("session_close", keys.session_close),
        ("sidebar_focus", keys.sidebar_focus),
    ];
    for (index, (first_name, first)) in bindings.iter().enumerate() {
        for (second_name, second) in bindings.iter().skip(index + 1) {
            if first == second {
                return Err(ConfigError::DuplicateChord {
                    first: (*first_name).to_owned(),
                    second: (*second_name).to_owned(),
                    chord: chord_text(*first),
                });
            }
        }
    }
    Ok(())
}

/// One chord modifier recognized by the [`parse_chord`] grammar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Modifier {
    Ctrl,
    Alt,
    Shift,
    Super,
}

/// Parse a modifier token, case-insensitively.
fn parse_modifier(token: &str) -> Option<Modifier> {
    match token.to_ascii_lowercase().as_str() {
        "ctrl" => Some(Modifier::Ctrl),
        "alt" => Some(Modifier::Alt),
        "shift" => Some(Modifier::Shift),
        "super" => Some(Modifier::Super),
        _ => None,
    }
}

/// Parse a final key token: one character (case-folded by [`Chord::new`]) or
/// a named key, case-insensitively.
fn parse_key_code(token: &str) -> Result<KeyCode, ChordParseError> {
    let mut characters = token.chars();
    let Some(first) = characters.next() else {
        return Err(ChordParseError::EmptyToken);
    };
    if characters.next().is_none() {
        return Ok(KeyCode::Char(first));
    }
    let lowered = token.to_ascii_lowercase();
    let code = match lowered.as_str() {
        "enter" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        "escape" => KeyCode::Escape,
        "space" => KeyCode::Space,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "insert" => KeyCode::Insert,
        "delete" => KeyCode::Delete,
        other => {
            let Some(number) = other.strip_prefix('f') else {
                return Err(ChordParseError::UnknownKey(token.to_owned()));
            };
            let Ok(number) = number.parse::<u8>() else {
                return Err(ChordParseError::UnknownKey(token.to_owned()));
            };
            KeyCode::Function(number)
        }
    };
    Ok(code)
}

/// Parse chord text such as `"super+p"` or `"ctrl+shift+t"` into a normalized
/// [`Chord`].
///
/// The grammar is zero or more of the modifiers `super`, `ctrl`, `alt`, and
/// `shift` (each at most once, case-insensitive) followed by exactly one key,
/// all joined with `+`. The key is a single character or a named key
/// (`enter`, `tab`, `backspace`, `escape`, `space`, arrows, `home`, `end`,
/// `pageup`, `pagedown`, `insert`, `delete`, `f1`–`f24`). Characters fold to
/// lowercase and whitespace or control characters are rejected, exactly as
/// [`Chord::new`] normalizes. Every failure is a typed
/// [`ChordParseError`]; there is no forgiving or default chord.
///
/// # Errors
///
/// Returns the first violation in left-to-right token order: an empty text,
/// an empty `+`-separated token, a missing final key, a non-modifier token
/// before the key, an unknown key name, a repeated modifier, or a key that
/// [`Chord::new`] cannot normalize.
pub fn parse_chord(text: &str) -> Result<Chord, ChordParseError> {
    if text.is_empty() {
        return Err(ChordParseError::Empty);
    }
    let tokens: Vec<&str> = text.split('+').collect();
    if tokens.iter().any(|token| token.is_empty()) {
        return Err(ChordParseError::EmptyToken);
    }
    let Some((key_token, modifier_tokens)) = tokens.split_last() else {
        return Err(ChordParseError::Empty);
    };
    if parse_modifier(key_token).is_some() {
        return Err(ChordParseError::MissingKey);
    }
    let mut modifiers = Modifiers::empty();
    for token in modifier_tokens {
        let token = *token;
        let Some(modifier) = parse_modifier(token) else {
            return Err(ChordParseError::NotAModifier(token.to_owned()));
        };
        let already = match modifier {
            Modifier::Ctrl => modifiers.is_ctrl(),
            Modifier::Alt => modifiers.is_alt(),
            Modifier::Shift => modifiers.is_shift(),
            Modifier::Super => modifiers.is_super(),
        };
        if already {
            return Err(ChordParseError::RepeatedModifier(token.to_owned()));
        }
        modifiers = match modifier {
            Modifier::Ctrl => modifiers.ctrl(),
            Modifier::Alt => modifiers.alt(),
            Modifier::Shift => modifiers.shift(),
            Modifier::Super => modifiers.super_key(),
        };
    }
    let code = parse_key_code(key_token)?;
    Chord::new(code, modifiers).map_err(ChordParseError::InvalidKey)
}

/// Render a chord back to its canonical `parse_chord` text.
///
/// Used for error messages, so the text names exactly what two actions share.
fn chord_text(chord: Chord) -> String {
    let modifiers = chord.modifiers();
    let mut parts: Vec<String> = Vec::new();
    if modifiers.is_ctrl() {
        parts.push("ctrl".to_owned());
    }
    if modifiers.is_alt() {
        parts.push("alt".to_owned());
    }
    if modifiers.is_shift() {
        parts.push("shift".to_owned());
    }
    if modifiers.is_super() {
        parts.push("super".to_owned());
    }
    parts.push(key_code_text(chord.code()));
    parts.join("+")
}

/// Canonical text of one key identity.
fn key_code_text(code: KeyCode) -> String {
    match code {
        KeyCode::Char(character) => character.to_string(),
        KeyCode::Function(number) => format!("f{number}"),
        KeyCode::Enter => "enter".to_owned(),
        KeyCode::Tab => "tab".to_owned(),
        KeyCode::Backspace => "backspace".to_owned(),
        KeyCode::Escape => "escape".to_owned(),
        KeyCode::Space => "space".to_owned(),
        KeyCode::Up => "up".to_owned(),
        KeyCode::Down => "down".to_owned(),
        KeyCode::Left => "left".to_owned(),
        KeyCode::Right => "right".to_owned(),
        KeyCode::Home => "home".to_owned(),
        KeyCode::End => "end".to_owned(),
        KeyCode::PageUp => "pageup".to_owned(),
        KeyCode::PageDown => "pagedown".to_owned(),
        KeyCode::Insert => "insert".to_owned(),
        KeyCode::Delete => "delete".to_owned(),
    }
}

/// Why chord text could not be parsed.
///
/// Every failure is typed; none falls back to a default chord.
///
/// **Echo allowlist** (issue #150): `NotAModifier`, `UnknownKey`, and
/// `RepeatedModifier` carry tokens of the `[keys]` value — chord grammar
/// text, not secret-bearing data (see the contract on [`ConfigError`]).
/// The only path to stderr is the clipped `reason` of
/// [`ConfigError::InvalidChord`]; this type's own display is unbounded and
/// must never be printed to a live stream unclipped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChordParseError {
    /// The chord text is empty.
    Empty,
    /// A `+`-separated token is empty (leading, trailing, or doubled `+`).
    EmptyToken,
    /// The text ends in a modifier, leaving no final key.
    MissingKey,
    /// A token before the final key is not one of the four modifiers.
    NotAModifier(String),
    /// The final token is not a character or known key name.
    UnknownKey(String),
    /// The same modifier appears more than once.
    RepeatedModifier(String),
    /// The key is valid grammar but not a normalizable [`Chord`] (a control
    /// or whitespace character, or a function key outside F1–F24).
    InvalidKey(ChordError),
}

impl fmt::Display for ChordParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("the chord is empty"),
            Self::EmptyToken => f.write_str("a '+'-separated part of the chord is empty"),
            Self::MissingKey => f.write_str("the chord has modifiers but no final key"),
            Self::NotAModifier(token) => write!(
                f,
                "chord part {token} precedes the key but is not one of super, ctrl, alt, shift"
            ),
            Self::UnknownKey(token) => {
                write!(f, "chord key {token} is not a recognized single key")
            }
            Self::RepeatedModifier(token) => write!(f, "modifier {token} appears more than once"),
            Self::InvalidKey(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ChordParseError {}

/// Standard per-user configuration path:
/// `~/Library/Application Support/Noren/config.toml`.
///
/// Returns `None` when `HOME` is unset or empty; callers then behave as if no
/// configuration exists.
#[must_use]
pub fn default_path() -> Option<PathBuf> {
    let home = env::var_os("HOME").filter(|value| !value.is_empty())?;
    let mut path = PathBuf::from(home);
    path.push("Library");
    path.push("Application Support");
    path.push("Noren");
    path.push(CONFIG_FILE_NAME);
    Some(path)
}

/// Read a file with a hard byte cap.
///
/// Symlinks are followed like any user-owned file, but the target must be a
/// regular file and the streamed read stops with [`ConfigError::TooLarge`] at
/// [`MAX_CONFIG_BYTES`], so a hostile target can neither panic the app nor
/// exhaust memory.
fn read_bounded(path: &Path) -> Result<Vec<u8>, ConfigError> {
    let metadata = fs::metadata(path).map_err(io_error)?;
    if !metadata.is_file() {
        return Err(ConfigError::NotAFile);
    }
    let mut file = fs::File::open(path).map_err(io_error)?;
    let cap = usize::try_from(MAX_CONFIG_BYTES).unwrap_or(usize::MAX);
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let read = file
            .read(&mut chunk)
            .map_err(|error| ConfigError::Io(error.kind()))?;
        if read == 0 {
            return Ok(buffer);
        }
        if buffer.len() + read > cap {
            return Err(ConfigError::TooLarge);
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

/// Surface a missing file as the distinct [`ConfigError::NotFound`].
fn io_error(error: std::io::Error) -> ConfigError {
    match error.kind() {
        ErrorKind::NotFound => ConfigError::NotFound,
        kind => ConfigError::Io(kind),
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

/// Typed configuration failure under the file-echo contract (TM-08,
/// issue #150).
///
/// `main` prints this error straight to live stderr, so every variant is a
/// disclosure surface. The contract:
///
/// * file **key names** and parse **positions** may appear, clipped to
///   120 characters by [`clip`] — a key name is where the user must look,
///   and is the most actionable thing an error can say;
/// * file **values** are never echoed, with one allowlist: the `[keys]`
///   chord variants marked **Echo allowlist** below. A chord is keybinding
///   grammar, and the schema deliberately exposes no credential or path
///   key, so no legitimate `[keys]` value is secret material — while an
///   error that cannot show the offending binding is not actionable.
///
/// `tests/error_echo_contract.rs` classifies every variant against this
/// contract: a new variant cannot compile without the classifier
/// acknowledging it, and the allowlist size is pinned there so admitting a
/// new echo is a reviewed decision, not a silent one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// An explicitly requested configuration file does not exist.
    NotFound,
    /// The file could not be read; only the I/O error kind is retained.
    Io(ErrorKind),
    /// The path exists but does not resolve to a regular file.
    NotAFile,
    /// The file exceeds [`MAX_CONFIG_BYTES`].
    TooLarge,
    /// The file is not valid UTF-8.
    NotUtf8,
    /// The file is not valid TOML.
    ///
    /// Only the 1-based position of the first offending token is retained;
    /// the third-party parser's message quotes the offending source line —
    /// file content — so none of its text is forwarded.
    Parse {
        /// The 1-based line where parsing stopped.
        line: usize,
        /// The 1-based column where parsing stopped.
        column: usize,
    },
    /// The file names a key this schema does not define.
    UnknownKey(String),
    /// A key holds the wrong TOML type.
    WrongType { key: String },
    /// A value is outside its accepted range.
    OutOfRange { key: String },
    /// A `[keys]` action is bound to a value that is not a TOML string.
    ChordNotAString { key: String },
    /// A `[theme]` value is not a TOML string.
    ThemeNotAString { key: String },
    /// A `[theme]` name is not one of the built-in themes.
    ///
    /// **Echo allowlist** (issue #150): `value` carries the `[theme]` name
    /// text, clipped to 120 characters by [`clip`]. A theme name is drawn
    /// from a closed, published vocabulary (`dark`, `light`,
    /// `high-contrast`) — keybinding-grammar text, never a credential,
    /// key, or path — and the schema deliberately exposes no setting where
    /// a secret is plausible, so the residual risk of a secret pasted into
    /// `[theme]` by mistake is accepted for the same reason the `[keys]`
    /// chord echo is: an error that cannot say which name failed is not
    /// actionable.
    UnknownTheme { value: String },
    /// A `[keys]` value does not parse as a chord.
    ///
    /// **Echo allowlist** (issue #150): `value` and `reason` carry `[keys]`
    /// chord text. `value` is unparsed, so it is arbitrary text clipped to
    /// 120 characters — the residual risk of a secret pasted into `[keys]`
    /// by mistake is accepted because naming the binding that failed is
    /// the entire point of this error. `reason` is the clipped
    /// [`ChordParseError`] display, which names the exact failing token.
    InvalidChord {
        key: String,
        value: String,
        reason: String,
    },
    /// A `[keys]` palette chord parses but could never be claimed: it
    /// collides with a pinned Zellij default or the frozen `Super+Escape`
    /// exit leader.
    ///
    /// **Echo allowlist** (issue #150): `value` has already parsed as a
    /// chord, so the echoed text is bounded by the chord grammar itself —
    /// only the four modifier names and known key names, case-insensitive,
    /// can appear. Arbitrary file text cannot reach this variant, and
    /// without the value the user could not tell which binding collides.
    UnclaimableChord { key: String, value: String },
    /// A `[keys]` palette command chord uses a key the open palette always
    /// interprets structurally, so the binding could never fire.
    ///
    /// **Echo allowlist** (issue #150): same bound as
    /// [`ConfigError::UnclaimableChord`] — the value has already parsed,
    /// so only grammar-bounded chord text can appear.
    ReservedChord { key: String, value: String },
    /// Two `[keys]` actions are bound to the same chord.
    ///
    /// No file text appears: `first` and `second` are compile-time action
    /// names, and `chord` is canonical text regenerated from the parsed
    /// chord by `chord_text`, never the file's own spelling.
    DuplicateChord {
        first: String,
        second: String,
        chord: String,
    },
    /// `agents` is present but is not an array of tables spelled `[[agents]]`.
    ///
    /// No file text appears beyond the fixed key name.
    AgentTableNotAnArray { key: String },
    /// An `[[agents]]` entry omits a required key (`name` or `command`).
    AgentFieldMissing { key: String },
    /// An `[[agents]]` string field holds the wrong TOML type.
    AgentFieldNotAString { key: String },
    /// The `args` key of an `[[agents]]` entry is not a TOML array.
    AgentArgsNotAnArray { key: String },
    /// One `args` element of an `[[agents]]` entry is not a TOML string.
    /// `index` is the element's 0-based position.
    AgentArgNotAString { index: usize },
    /// The `command` of an `[[agents]]` entry is not an absolute path.
    ///
    /// The rejected text never appears: the position is the entry itself,
    /// and `PATH` lookup is deliberately not performed, so the only accepted
    /// shape is a leading `/`.
    AgentCommandNotAbsolute { key: String },
    /// `projects` is present but is not an array of tables spelled
    /// `[[projects]]`.
    ///
    /// No file text appears beyond the fixed key name.
    ProjectTableNotAnArray { key: String },
    /// A `[[projects]]` entry omits a required key (`name` or `root`).
    ProjectFieldMissing { key: String },
    /// A `[[projects]]` string field holds the wrong TOML type.
    ProjectFieldNotAString { key: String },
    /// The `root` of a `[[projects]]` entry is not an absolute path.
    ///
    /// The rejected text never appears: neither `~` expansion nor resolution
    /// against the launch directory is performed, so the only accepted shape
    /// is a leading `/`. A root can embed a username or a private directory
    /// name, so echoing it would leak exactly what this schema keeps out of
    /// diagnostics.
    ProjectRootNotAbsolute { key: String },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("configuration file not found"),
            Self::Io(kind) => write!(f, "configuration could not be read: {kind}"),
            Self::NotAFile => f.write_str("configuration path does not resolve to a regular file"),
            Self::TooLarge => write!(f, "configuration exceeds {MAX_CONFIG_BYTES} bytes"),
            Self::NotUtf8 => f.write_str("configuration is not valid UTF-8"),
            Self::Parse { line, column } => write!(
                f,
                "configuration is not valid TOML at line {line}, column {column}"
            ),
            Self::UnknownKey(key) => write!(f, "unknown configuration key: {key}"),
            Self::WrongType { key } => {
                write!(f, "configuration key {key} must be an integer")
            }
            Self::OutOfRange { key } => {
                write!(f, "configuration key {key} is outside its accepted range")
            }
            Self::ChordNotAString { key } => write!(
                f,
                "configuration key {key} must be a chord string like \"super+p\""
            ),
            Self::ThemeNotAString { key } => write!(
                f,
                "configuration key {key} must be a theme name string like \"light\""
            ),
            Self::UnknownTheme { value } => write!(
                f,
                "configuration theme {value} is not a built-in theme; expected one of \
                 dark, light, high-contrast"
            ),
            Self::InvalidChord { key, value, reason } => write!(
                f,
                "configuration key {key} has an unparseable chord {value}: {reason}"
            ),
            Self::UnclaimableChord { key, value } => write!(
                f,
                "configuration key {key} binds chord {value}, which collides with a pinned \
                 Zellij default or the frozen Super+Escape exit leader"
            ),
            Self::ReservedChord { key, value } => write!(
                f,
                "configuration key {key} binds chord {value} on a key the application \
                 always claims first: the open palette reads it as navigation or dismissal, \
                 and Super+A/C/V/D are fixed clipboard and diagnostics shortcuts"
            ),
            Self::DuplicateChord {
                first,
                second,
                chord,
            } => write!(
                f,
                "configuration keys {first} and {second} are both bound to the chord {chord}"
            ),
            Self::AgentTableNotAnArray { key } => write!(
                f,
                "configuration key {key} must be an array of tables spelled [[agents]]"
            ),
            Self::AgentFieldMissing { key } => {
                write!(f, "an [[agents]] entry is missing required key: {key}")
            }
            Self::AgentFieldNotAString { key } => {
                write!(
                    f,
                    "configuration key {key} inside [[agents]] must be a string"
                )
            }
            Self::AgentArgsNotAnArray { key } => write!(
                f,
                "configuration key {key} inside [[agents]] must be an array of strings"
            ),
            Self::AgentArgNotAString { index } => {
                write!(f, "args element {index} inside [[agents]] must be a string")
            }
            Self::AgentCommandNotAbsolute { key } => write!(
                f,
                "configuration key {key} inside [[agents]] must be an absolute path with a \
                 leading '/'; PATH lookup is deliberately not performed"
            ),
            Self::ProjectTableNotAnArray { key } => write!(
                f,
                "configuration key {key} must be an array of tables spelled [[projects]]"
            ),
            Self::ProjectFieldMissing { key } => {
                write!(f, "a [[projects]] entry is missing required key: {key}")
            }
            Self::ProjectFieldNotAString { key } => {
                write!(
                    f,
                    "configuration key {key} inside [[projects]] must be a string"
                )
            }
            Self::ProjectRootNotAbsolute { key } => write!(
                f,
                "configuration key {key} inside [[projects]] must be an absolute path with a \
                 leading '/'; neither '~' expansion nor resolution against the launch \
                 directory is performed"
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GridGeometry, MAX_RENDER_COLS, MAX_RENDER_ROWS, Resize};
    use std::io::Write;

    fn temp_path(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("noren-config-test-{}-{name}", std::process::id()));
        path
    }

    fn write_test_file(name: &str, bytes: &[u8]) -> PathBuf {
        let path = temp_path(name);
        let mut file = fs::File::create(&path).expect("create temporary config fixture");
        file.write_all(bytes)
            .expect("write temporary config fixture");
        path
    }

    #[test]
    fn default_config_matches_the_pre_configuration_constants() {
        let config = AppConfig::default();
        assert_eq!(config.font().cell_width(), POC_CELL_WIDTH);
        assert_eq!(config.font().cell_height(), POC_CELL_HEIGHT);
        assert_eq!(config, AppConfig::default());
    }

    #[test]
    fn missing_default_path_means_defaults() {
        let mut path = temp_path("missing-dir");
        path.push("does-not-exist");
        path.push(CONFIG_FILE_NAME);
        assert!(matches!(
            AppConfig::load_from(&path),
            Err(ConfigError::NotFound)
        ));
    }

    #[test]
    fn empty_and_comment_only_files_keep_every_default() {
        for text in ["", "# only a comment\n"] {
            assert_eq!(AppConfig::parse(text), Ok(AppConfig::default()));
        }
    }

    #[test]
    fn cell_width_key_overrides_only_its_own_default() {
        let config = AppConfig::parse("[font]\ncell_width = 12\n").expect("valid configuration");
        assert_eq!(config.font().cell_width(), 12);
        assert_eq!(config.font().cell_height(), POC_CELL_HEIGHT);
    }

    #[test]
    fn cell_height_key_overrides_only_its_own_default() {
        let config = AppConfig::parse("[font]\ncell_height = 24\n").expect("valid configuration");
        assert_eq!(config.font().cell_height(), 24);
        assert_eq!(config.font().cell_width(), POC_CELL_WIDTH);
    }

    #[test]
    fn the_terminal_table_is_rejected_until_the_cap_is_enforceable() {
        // `scrollback_lines` cannot be honored yet (the terminal foundation
        // retains a fixed hard cap), so accepting it would be a silent no-op.
        for text in [
            "[terminal]\nscrollback_lines = 250\n",
            "[terminal]\nscrollback_lines = 10000\n",
        ] {
            assert_eq!(
                AppConfig::parse(text),
                Err(ConfigError::UnknownKey("terminal".to_owned())),
                "{text:?} must not parse as a working setting"
            );
        }
    }

    #[test]
    fn every_supported_key_applies_together() {
        let text = "[font]\ncell_width = 11\ncell_height = 22\n";
        let config = AppConfig::parse(text).expect("valid configuration");
        assert_eq!(config.font().cell_width(), 11);
        assert_eq!(config.font().cell_height(), 22);
    }

    #[test]
    fn the_cell_edge_cap_is_accepted_and_cannot_be_raised() {
        let text = format!("[font]\ncell_width = {MAX_CELL_EDGE}\ncell_height = {MAX_CELL_EDGE}\n");
        let config = AppConfig::parse(&text).expect("the cap itself is valid");
        assert_eq!(config.font().cell_width(), MAX_CELL_EDGE);
        assert_eq!(config.font().cell_height(), MAX_CELL_EDGE);
        for hostile in [
            format!("cell_width = {}", u64::from(MAX_CELL_EDGE) + 1),
            "cell_height = 9223372036854775807".to_owned(),
            "cell_width = 0".to_owned(),
            "cell_height = -1".to_owned(),
        ] {
            let result = AppConfig::parse(&format!("[font]\n{hostile}\n"));
            assert!(
                matches!(result, Err(ConfigError::OutOfRange { .. })),
                "{hostile} must not bypass the cell-edge bounds, got {result:?}"
            );
        }
    }

    #[test]
    fn cell_edges_reject_below_renderer_floor_negative_and_enormous_values() {
        for hostile in [
            // Below the renderer's built-in cell constants the grid would
            // exceed what the renderer draws, so these are rejected.
            &format!("cell_width = {}", POC_CELL_WIDTH - 1),
            &format!("cell_height = {}", POC_CELL_HEIGHT - 1),
            "cell_width = 0",
            "cell_height = 0",
            "cell_width = -5",
            "cell_width = 999999999999",
            "cell_height = 9223372036854775807",
        ] {
            let result = AppConfig::parse(&format!("[font]\n{hostile}\n"));
            assert!(
                matches!(result, Err(ConfigError::OutOfRange { .. })),
                "{hostile} must be rejected, got {result:?}"
            );
        }
    }

    #[test]
    fn malformed_toml_is_an_error_not_a_panic() {
        for text in [
            "[font\n",
            "cell_width = ",
            "====",
            "[font]\ncell_width = 12\ncell_width = 13\n",
        ] {
            let result = std::panic::catch_unwind(|| AppConfig::parse(text));
            let parsed = result.expect("parsing must never panic");
            assert!(
                matches!(parsed, Err(ConfigError::Parse { .. })),
                "{text:?} must fail parsing, got {parsed:?}"
            );
        }
    }

    #[test]
    fn non_utf8_bytes_are_an_error_not_a_panic() {
        let result = AppConfig::from_bytes(&[0xff, 0xfe, 0x00, b'[', b'f']);
        assert_eq!(result, Err(ConfigError::NotUtf8));
    }

    #[test]
    fn wrong_types_and_unknown_keys_are_rejected() {
        let cases = [
            (
                "[font]\ncell_width = \"wide\"\n",
                ConfigError::WrongType {
                    key: "cell_width".to_owned(),
                },
            ),
            (
                "[font]\ncell_width = 12.5\n",
                ConfigError::WrongType {
                    key: "cell_width".to_owned(),
                },
            ),
            (
                "font = 3\n",
                ConfigError::WrongType {
                    key: "font".to_owned(),
                },
            ),
            (
                "[shell]\nprogram = \"/bin/sh\"\n",
                ConfigError::UnknownKey("shell".to_owned()),
            ),
            (
                "[font]\nsize = 12\n",
                ConfigError::UnknownKey("size".to_owned()),
            ),
            (
                "[terminal]\nscrollback_lines = 250\n",
                ConfigError::UnknownKey("terminal".to_owned()),
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(AppConfig::parse(text), Err(expected), "{text:?}");
        }
    }

    #[test]
    fn hostile_key_names_are_clipped_in_errors() {
        let mut key = String::from("\"");
        key.extend(std::iter::repeat_n('a', 10_000));
        key.push_str("\" = 1\n");
        let error = AppConfig::parse(&key).expect_err("huge key must fail");
        let echoed = match &error {
            ConfigError::UnknownKey(name) | ConfigError::WrongType { key: name } => name,
            other => panic!("expected a keyed error, got {other:?}"),
        };
        assert!(
            echoed.chars().count() <= MAX_ERROR_DETAIL_CHARS + 1,
            "hostile key must be clipped: {echoed}"
        );
    }

    #[test]
    fn oversized_files_are_rejected_without_unbounded_reads() {
        let path = write_test_file("oversized.toml", &vec![b'a'; MAX_CONFIG_BYTES as usize + 1]);
        assert_eq!(AppConfig::load_from(&path), Err(ConfigError::TooLarge));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn directory_and_symlink_targets_are_rejected_cleanly() {
        let dir = temp_path("a-directory");
        fs::create_dir_all(&dir).expect("create directory fixture");
        assert_eq!(AppConfig::load_from(&dir), Err(ConfigError::NotAFile));

        let target = write_test_file("symlink-target.toml", b"[font]\ncell_width = 11\n");
        let link = temp_path("symlink-config.toml");
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink(&target, &link).expect("create symlink fixture");
        let via_symlink = AppConfig::load_from(&link).expect("symlinked file still parses");
        assert_eq!(via_symlink.font().cell_width(), 11);

        let link_to_dir = temp_path("symlink-to-dir");
        let _ = fs::remove_file(&link_to_dir);
        std::os::unix::fs::symlink(&dir, &link_to_dir).expect("create symlink fixture");
        assert_eq!(
            AppConfig::load_from(&link_to_dir),
            Err(ConfigError::NotAFile)
        );

        let _ = fs::remove_file(link);
        let _ = fs::remove_file(link_to_dir);
        let _ = fs::remove_file(target);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_from_reads_a_well_formed_file() {
        let path = write_test_file(
            "well-formed.toml",
            b"[font]\ncell_width = 12\ncell_height = 24\n",
        );
        let config = AppConfig::load_from(&path).expect("valid file loads");
        assert_eq!(config.font().cell_width(), 12);
        assert_eq!(config.font().cell_height(), 24);
        let _ = fs::remove_file(path);
    }

    /// Required regression: with no readable configuration file, the app
    /// resolves the exact pre-configuration constants and nothing else.
    #[test]
    fn no_config_file_behaves_identically_to_today() {
        let missing = temp_path("no-such-dir/config.toml");
        let resolved = AppConfig::resolve(Some(missing.as_os_str()));
        assert_eq!(resolved, Err(ConfigError::NotFound));
        // The standard-path fallback for an absent file is the default config,
        // and the default config is exactly the built-in constants.
        assert_eq!(AppConfig::default().font().cell_width(), POC_CELL_WIDTH);
        assert_eq!(AppConfig::default().font().cell_height(), POC_CELL_HEIGHT);
    }

    #[test]
    fn default_cell_sizes_produce_the_same_geometry_as_the_poc_constructor() {
        let configured = GridGeometry::with_cells(POC_CELL_WIDTH, POC_CELL_HEIGHT)
            .expect("PoC cell sizes are valid");
        let mut configured = configured;
        let mut poc = GridGeometry::poc();
        let sizes = [(900, 600), (1, 1), (0, 480), (u32::MAX, u32::MAX)];
        for (width, height) in sizes {
            let resize = Resize::new(width, height);
            assert_eq!(configured.update(resize), poc.update(resize));
        }
        assert_eq!(configured.current(), poc.current());
    }

    #[test]
    fn any_valid_cell_size_keeps_the_grid_within_every_hard_cap() {
        for width in [1, POC_CELL_WIDTH, MAX_CELL_EDGE] {
            for height in [1, POC_CELL_HEIGHT, MAX_CELL_EDGE] {
                let mut geometry = GridGeometry::with_cells(width, height)
                    .expect("range-checked cell sizes are valid");
                let grid = geometry
                    .update(Resize::new(u32::MAX, u32::MAX))
                    .expect("non-zero resize yields a grid");
                assert!(u32::from(grid.rows()) <= u32::from(MAX_RENDER_ROWS));
                assert!(u32::from(grid.cols()) <= u32::from(MAX_RENDER_COLS));
                let cells = usize::from(grid.rows()) * usize::from(grid.cols());
                assert!(cells <= noren_terminal::MAX_SCREEN_CELLS);
            }
        }
        assert!(GridGeometry::with_cells(0, POC_CELL_HEIGHT).is_none());
        assert!(GridGeometry::with_cells(POC_CELL_WIDTH, 0).is_none());
    }

    #[test]
    fn explicit_override_must_exist_and_parse() {
        let path = write_test_file("env-override.toml", b"[font]\ncell_height = 21\n");
        let loaded = AppConfig::resolve(Some(path.as_os_str())).expect("override file loads");
        assert_eq!(loaded.font().cell_height(), 21);

        let missing = temp_path("env-override-missing.toml");
        assert_eq!(
            AppConfig::resolve(Some(missing.as_os_str())),
            Err(ConfigError::NotFound)
        );

        // An empty override behaves like no override: standard path rules.
        assert!(AppConfig::resolve(Some(std::ffi::OsStr::new(""))) != Err(ConfigError::NotFound));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn resolve_without_override_never_errors_on_missing_defaults() {
        // The standard path may or may not exist on the host; either way the
        // resolution succeeds: existing files must parse, absent ones default.
        let result = AppConfig::resolve(None);
        match default_path() {
            Some(path) if path.is_file() => assert_eq!(result, AppConfig::load_from(&path)),
            _ => assert_eq!(result, Ok(AppConfig::default())),
        }
    }

    #[test]
    fn standard_path_points_inside_the_home_directory() {
        let Some(path) = default_path() else {
            return; // HOME unset: absence means defaults, covered elsewhere.
        };
        assert!(path.is_absolute());
        assert!(path.ends_with(Path::new("Library/Application Support/Noren/config.toml")));
    }

    #[test]
    fn error_messages_never_embed_full_file_contents() {
        let mut hostile = String::from("not toml ");
        hostile.extend(std::iter::repeat_n('x', 100_000));
        let error = AppConfig::parse(&hostile).expect_err("malformed input fails");
        let message = error.to_string();
        assert!(message.len() < 1024, "message must stay bounded: {message}");
    }

    // ── [keys] keymap tests ────────────────────────────────────────────

    fn chord(code: KeyCode, modifiers: Modifiers) -> Chord {
        Chord::new(code, modifiers).expect("test chords are normalized constants")
    }

    /// With no `[keys]` surface at all, the keymap is exactly the chord set
    /// the app shipped with before configuration existed.
    #[test]
    fn default_keymap_matches_the_pre_configuration_chords() {
        let keys = KeymapConfig::default();
        assert_eq!(
            keys.palette_open(),
            chord(KeyCode::Char('p'), Modifiers::empty().super_key()),
            "palette opener defaults to super+p"
        );
        assert_eq!(
            keys.session_create(),
            chord(KeyCode::Char('c'), Modifiers::empty())
        );
        assert_eq!(
            keys.session_select(),
            chord(KeyCode::Char('s'), Modifiers::empty())
        );
        assert_eq!(
            keys.session_close(),
            chord(KeyCode::Char('x'), Modifiers::empty())
        );
        assert_eq!(
            keys.sidebar_focus(),
            chord(KeyCode::Char('f'), Modifiers::empty())
        );
        assert_eq!(AppConfig::default().keys(), keys);
        assert_eq!(AppConfig::parse("# nothing\n").expect("valid").keys(), keys);
    }

    /// An empty `[keys]` table is presence without content: all defaults.
    #[test]
    fn empty_keys_table_keeps_every_default_chord() {
        let config = AppConfig::parse("[keys]\n").expect("an empty table is valid");
        assert_eq!(config.keys(), KeymapConfig::default());
    }

    #[test]
    fn custom_palette_chord_overrides_only_the_opener() {
        let config = AppConfig::parse("[keys]\npalette_open = \"super+k\"\n")
            .expect("super+k is claimable and distinct");
        assert_eq!(
            config.keys().palette_open(),
            chord(KeyCode::Char('k'), Modifiers::empty().super_key())
        );
        assert_eq!(
            config.keys(),
            KeymapConfig {
                palette_open: config.keys().palette_open(),
                ..KeymapConfig::default()
            }
        );
    }

    #[test]
    fn custom_command_chords_apply_to_their_own_actions() {
        let config = AppConfig::parse(
            "[keys]\nsession_create = \"ctrl+shift+t\"\nsession_select = \"n\"\n\
             session_close = \"f2\"\nsidebar_focus = \"tab\"\n",
        )
        .expect("all four command chords are valid and distinct");
        assert_eq!(
            config.keys().session_create(),
            chord(KeyCode::Char('t'), Modifiers::empty().ctrl().shift())
        );
        assert_eq!(
            config.keys().session_select(),
            chord(KeyCode::Char('n'), Modifiers::empty())
        );
        assert_eq!(
            config.keys().session_close(),
            chord(KeyCode::Function(2), Modifiers::empty())
        );
        assert_eq!(
            config.keys().sidebar_focus(),
            chord(KeyCode::Tab, Modifiers::empty())
        );
        assert_eq!(
            config.keys().palette_open(),
            KeymapConfig::default().palette_open()
        );
    }

    #[test]
    fn keys_apply_alongside_font_in_one_file() {
        let config =
            AppConfig::parse("[font]\ncell_width = 12\n[keys]\npalette_open = \"super+b\"\n")
                .expect("both tables are valid");
        assert_eq!(config.font().cell_width(), 12);
        assert_eq!(
            config.keys().palette_open(),
            chord(KeyCode::Char('b'), Modifiers::empty().super_key())
        );
    }

    /// Mutation proof (a): an action name the schema does not define must be
    /// an error, never a silently ignored key. If unknown actions were
    /// ignored, these files would parse as defaults and this test would fail.
    #[test]
    fn unknown_keys_actions_are_rejected_not_ignored() {
        let cases = [
            ("[keys]\npalette = \"super+p\"\n", "palette"),
            ("[keys]\nsession_creat = \"c\"\n", "session_creat"),
            ("[keys]\nzoom = \"super+z\"\n", "zoom"),
            // A dotted action name becomes a sub-table, still unknown.
            ("[keys.session]\ncreate = \"c\"\n", "session"),
        ];
        for (text, name) in cases {
            assert_eq!(
                AppConfig::parse(text),
                Err(ConfigError::UnknownKey(name.to_owned())),
                "{text:?} must not parse as a working setting"
            );
        }
    }

    /// Super+A/C/V (clipboard) and Super+D (diagnostics) are handled ahead of
    /// every configured path, for every action, with further modifiers still
    /// matching — a configured binding there would be dead, so it is a typed
    /// `ReservedChord` error naming the key, never a silently shadowed accept.
    #[test]
    fn fixed_global_shortcuts_are_rejected_for_every_action() {
        let actions = [
            "palette_open",
            "session_create",
            "session_select",
            "session_close",
            "sidebar_focus",
        ];
        let reserved = [
            ("super+a", 'a'),
            ("super+c", 'c'),
            ("super+v", 'v'),
            ("super+d", 'd'),
            // Shift (and any further modifier) still routes into the fixed
            // handlers, so these are dead too.
            ("super+shift+a", 'a'),
            ("super+ctrl+d", 'd'),
        ];
        for action in actions {
            for (chord, character) in reserved {
                let text = format!("[keys]\n{action} = \"{chord}\"\n");
                assert_eq!(
                    AppConfig::parse(&text),
                    Err(ConfigError::ReservedChord {
                        key: action.to_owned(),
                        value: chord.to_owned(),
                    }),
                    "{chord} must not be accepted for {action}"
                );
                let _ = character;
            }
        }
        // The unmodified command keys and the default palette chord stay
        // acceptable; only the fixed globals are refused.
        for (action, chord) in [
            ("session_create", "c"),
            ("palette_open", "super+p"),
            ("sidebar_focus", "f"),
        ] {
            let text = format!("[keys]\n{action} = \"{chord}\"\n");
            assert!(
                AppConfig::parse(&text).is_ok(),
                "{chord} for {action} must stay accepted"
            );
        }
    }

    /// Every unparseable chord is a typed error naming the offending key and
    /// its value; none falls back to a default chord.
    #[test]
    fn unparseable_chords_are_typed_errors_naming_key_and_value() {
        let cases = [
            ("", ChordParseError::Empty),
            ("+p", ChordParseError::EmptyToken),
            ("super+", ChordParseError::EmptyToken),
            ("super++p", ChordParseError::EmptyToken),
            ("super", ChordParseError::MissingKey),
            ("ctrl+shift", ChordParseError::MissingKey),
            ("hyper+p", ChordParseError::NotAModifier("hyper".to_owned())),
            ("p+q", ChordParseError::NotAModifier("p".to_owned())),
            (
                "ctrl+ctrl+t",
                ChordParseError::RepeatedModifier("ctrl".to_owned()),
            ),
            ("foo", ChordParseError::UnknownKey("foo".to_owned())),
            ("super+foo", ChordParseError::UnknownKey("foo".to_owned())),
            ("fxy", ChordParseError::UnknownKey("fxy".to_owned())),
            (
                " ",
                ChordParseError::InvalidKey(ChordError::ControlOrWhitespaceChar),
            ),
            (
                "f25",
                ChordParseError::InvalidKey(ChordError::FunctionKeyOutOfRange),
            ),
        ];
        for (value, reason) in cases {
            assert_eq!(
                AppConfig::parse(&format!("[keys]\nsession_create = \"{value}\"\n")),
                Err(ConfigError::InvalidChord {
                    key: "session_create".to_owned(),
                    value: value.to_owned(),
                    reason: reason.to_string(),
                }),
                "chord {value:?} must be rejected with its key and value"
            );
        }
    }

    /// Hostile chord values are clipped in the error, like hostile keys.
    #[test]
    fn unparseable_chord_values_are_clipped_in_errors() {
        let mut value = String::new();
        value.extend(std::iter::repeat_n('a', 10_000));
        let error = AppConfig::parse(&format!("[keys]\npalette_open = \"{value}\"\n"))
            .expect_err("hostile chord value fails");
        match error {
            ConfigError::InvalidChord { value, .. } => {
                assert!(
                    value.chars().count() <= MAX_ERROR_DETAIL_CHARS + 1,
                    "chord value must be clipped: {value}"
                );
            }
            other => panic!("expected InvalidChord, got {other:?}"),
        }
    }

    /// Two actions on one chord are rejected — including a configured value
    /// that collides with an action the table left at its default.
    #[test]
    fn duplicate_chords_are_rejected_including_against_defaults() {
        let explicit = AppConfig::parse("[keys]\nsession_create = \"n\"\nsession_select = \"n\"\n")
            .expect_err("two actions on one chord");
        assert_eq!(
            explicit,
            ConfigError::DuplicateChord {
                first: "session_create".to_owned(),
                second: "session_select".to_owned(),
                chord: "n".to_owned(),
            }
        );

        let against_default = AppConfig::parse("[keys]\nsession_create = \"f\"\n")
            .expect_err("f is the sidebar_focus default");
        assert_eq!(
            against_default,
            ConfigError::DuplicateChord {
                first: "session_create".to_owned(),
                second: "sidebar_focus".to_owned(),
                chord: "f".to_owned(),
            }
        );

        // Distinct-modifier chords on the same character are fine.
        assert!(AppConfig::parse("[keys]\nsession_create = \"ctrl+c\"\n").is_ok());
    }

    /// A palette chord the pass-through policy could never claim is an
    /// error, because the app could never honor it.
    #[test]
    fn unclaimable_palette_chords_are_rejected() {
        for value in [
            "p",            // bare p: Zellij pane mode binds it
            "ctrl+t",       // Zellij shared_except_locked binds Ctrl t
            "super+escape", // the frozen exit leader
        ] {
            assert_eq!(
                AppConfig::parse(&format!("[keys]\npalette_open = \"{value}\"\n")),
                Err(ConfigError::UnclaimableChord {
                    key: "palette_open".to_owned(),
                    value: value.to_owned(),
                }),
                "chord {value:?} must not parse as a claimable opener"
            );
        }
        // Super-space chords stay outside the corpus and are accepted.
        assert!(AppConfig::parse("[keys]\npalette_open = \"super+shift+p\"\n").is_ok());
    }

    /// Command chords on keys the open palette always interprets structurally
    /// would be dead bindings, so they are rejected; the opener may use them.
    #[test]
    fn command_chords_on_palette_ui_keys_are_rejected() {
        for value in ["escape", "enter", "up", "down"] {
            assert_eq!(
                AppConfig::parse(&format!("[keys]\nsession_create = \"{value}\"\n")),
                Err(ConfigError::ReservedChord {
                    key: "session_create".to_owned(),
                    value: value.to_owned(),
                }),
                "command chord {value:?} could never fire"
            );
        }
        assert!(AppConfig::parse("[keys]\nsession_create = \"left\"\n").is_ok());
        assert!(AppConfig::parse("[keys]\npalette_open = \"super+enter\"\n").is_ok());
    }

    /// `[keys]` values must be TOML strings.
    #[test]
    fn keys_values_must_be_chord_strings() {
        for text in [
            "[keys]\nsession_create = 3\n",
            "[keys]\npalette_open = true\n",
            "[keys]\nsession_select = ['c']\n",
        ] {
            let error = AppConfig::parse(text).expect_err("non-string chord fails");
            assert!(
                matches!(error, ConfigError::ChordNotAString { .. }),
                "{text:?} must be ChordNotAString, got {error:?}"
            );
        }
    }

    // ── chord parser unit tests ────────────────────────────────────────

    #[test]
    fn parse_chord_builds_modifiers_named_keys_and_folds_case() {
        let cases = [
            (
                "super+p",
                chord(KeyCode::Char('p'), Modifiers::empty().super_key()),
            ),
            (
                "SUPER+P",
                chord(KeyCode::Char('p'), Modifiers::empty().super_key()),
            ),
            (
                "ctrl+shift+t",
                chord(KeyCode::Char('t'), Modifiers::empty().ctrl().shift()),
            ),
            (
                "alt+ctrl+shift+super+x",
                chord(
                    KeyCode::Char('x'),
                    Modifiers::empty().alt().ctrl().shift().super_key(),
                ),
            ),
            ("c", chord(KeyCode::Char('c'), Modifiers::empty())),
            ("P", chord(KeyCode::Char('p'), Modifiers::empty())),
            (
                "super+f5",
                chord(KeyCode::Function(5), Modifiers::empty().super_key()),
            ),
            ("enter", chord(KeyCode::Enter, Modifiers::empty())),
            (
                "alt+pageup",
                chord(KeyCode::PageUp, Modifiers::empty().alt()),
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(parse_chord(text), Ok(expected), "{text:?}");
        }
    }

    #[test]
    fn parse_chord_rejects_each_grammar_violation_directly() {
        let cases = [
            ("", ChordParseError::Empty),
            ("+", ChordParseError::EmptyToken),
            ("p+", ChordParseError::EmptyToken),
            ("shift", ChordParseError::MissingKey),
            (
                "alt+hyper+p",
                ChordParseError::NotAModifier("hyper".to_owned()),
            ),
            (
                "super+super+p",
                ChordParseError::RepeatedModifier("super".to_owned()),
            ),
            (
                "f0",
                ChordParseError::InvalidKey(ChordError::FunctionKeyOutOfRange),
            ),
            (
                "\t",
                ChordParseError::InvalidKey(ChordError::ControlOrWhitespaceChar),
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(parse_chord(text), Err(expected), "{text:?}");
        }
    }

    /// Canonical text renders every chord back to its own parse input.
    #[test]
    fn chord_text_round_trips_through_parse_chord() {
        let chords = [
            chord(KeyCode::Char('p'), Modifiers::empty().super_key()),
            chord(KeyCode::Char('t'), Modifiers::empty().ctrl().shift()),
            chord(KeyCode::Char('c'), Modifiers::empty()),
            chord(KeyCode::Function(12), Modifiers::empty().super_key()),
            chord(KeyCode::Enter, Modifiers::empty()),
            chord(KeyCode::PageUp, Modifiers::empty().alt()),
        ];
        for parsed in chords {
            let text = chord_text(parsed);
            assert_eq!(parse_chord(&text), Ok(parsed), "{text:?} must round-trip");
        }
        assert_eq!(
            chord_text(KeymapConfig::default().palette_open()),
            "super+p"
        );
    }

    // ── [theme] theme tests ───────────────────────────────────────────

    /// With no `[theme]` surface at all, the selection is `dark` — exactly
    /// the palette the app shipped with before themes existed.
    #[test]
    fn default_theme_is_dark_the_pre_theme_palette() {
        assert_eq!(ThemeConfig::default().name(), ThemeName::Dark);
        assert_eq!(AppConfig::default().theme().name(), ThemeName::Dark);
        assert_eq!(
            AppConfig::parse("# nothing\n")
                .expect("valid")
                .theme()
                .name(),
            ThemeName::Dark
        );
        // The default config's palette is the dark theme constant itself.
        assert_eq!(
            AppConfig::default().theme().palette(),
            ThemeName::Dark.palette()
        );
    }

    /// An empty `[theme]` table is presence without content: the default.
    #[test]
    fn empty_theme_table_keeps_the_default_selection() {
        let config = AppConfig::parse("[theme]\n").expect("an empty table is valid");
        assert_eq!(config.theme(), ThemeConfig::default());
    }

    /// Each documented name selects its own palette.
    #[test]
    fn every_documented_theme_name_selects_its_own_palette() {
        for (text, name) in [
            ("dark", ThemeName::Dark),
            ("light", ThemeName::Light),
            ("high-contrast", ThemeName::HighContrast),
        ] {
            let config = AppConfig::parse(&format!("[theme]\nname = \"{text}\"\n")).expect("valid");
            assert_eq!(config.theme().name(), name, "{text:?}");
            assert_eq!(config.theme().palette(), name.palette());
        }
    }

    /// An unknown theme name is a typed error naming the offending value —
    /// never a silent fallback to the default. Near-misses included.
    #[test]
    fn unknown_theme_names_are_rejected_naming_the_offending_value() {
        for text in [
            "[theme]\nname = \"sepia\"\n",
            "[theme]\nname = \"solarized-dark\"\n",
            // Case and spelling near-misses: the vocabulary is closed and
            // case-sensitive, so these must not quietly select a theme.
            "[theme]\nname = \"Dark\"\n",
            "[theme]\nname = \"highcontrast\"\n",
            "[theme]\nname = \"\"\n",
        ] {
            let error = AppConfig::parse(text).expect_err("must not parse");
            assert!(
                matches!(&error, ConfigError::UnknownTheme { .. }),
                "{text:?} must be UnknownTheme, got {error:?}"
            );
        }
        let error = AppConfig::parse("[theme]\nname = \"sepia\"\n").expect_err("rejected");
        assert_eq!(
            error,
            ConfigError::UnknownTheme {
                value: "sepia".to_owned()
            }
        );
    }

    /// A hostile theme value is clipped in the error, like every echo.
    #[test]
    fn hostile_theme_values_are_clipped_in_errors() {
        let mut hostile = String::new();
        hostile.extend(std::iter::repeat_n('a', 10_000));
        let text = format!("[theme]\nname = \"{hostile}\"\n");
        let error = AppConfig::parse(&text).expect_err("a huge name must fail");
        let ConfigError::UnknownTheme { value } = &error else {
            panic!("expected UnknownTheme, got {error:?}");
        };
        assert!(
            value.chars().count() <= MAX_ERROR_DETAIL_CHARS + 1,
            "hostile theme name must be clipped: {value}"
        );
    }

    /// Wrong value types and unknown keys are rejected with the section's
    /// own typed errors.
    #[test]
    fn theme_table_rejects_wrong_types_and_unknown_keys() {
        let cases = [
            (
                "[theme]\nname = 3\n",
                ConfigError::ThemeNotAString {
                    key: "name".to_owned(),
                },
            ),
            (
                "[theme]\nname = 12.5\n",
                ConfigError::ThemeNotAString {
                    key: "name".to_owned(),
                },
            ),
            (
                "[theme]\nname = true\n",
                ConfigError::ThemeNotAString {
                    key: "name".to_owned(),
                },
            ),
            (
                "[theme]\npalette = \"light\"\n",
                ConfigError::UnknownKey("palette".to_owned()),
            ),
            (
                "theme = \"light\"\n",
                ConfigError::WrongType {
                    key: "theme".to_owned(),
                },
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(AppConfig::parse(text), Err(expected), "{text:?}");
        }
    }

    /// `[theme]` composes with the other sections without interference.
    #[test]
    fn theme_applies_alongside_font_and_keys() {
        let text = "[font]\ncell_width = 12\n\n[theme]\nname = \"light\"\n\n[keys]\nsession_create = \"t\"\n";
        let config = AppConfig::parse(text).expect("all sections are valid together");
        assert_eq!(config.font().cell_width(), 12);
        assert_eq!(config.theme().name(), ThemeName::Light);
        assert_eq!(
            config.keys().session_create(),
            chord(KeyCode::Char('t'), Modifiers::empty())
        );
    }

    // ── [[agents]] tests ───────────────────────────────────────────────

    /// With no `[[agents]]` entries the configured list is empty — exactly
    /// the pre-agents behavior.
    #[test]
    fn default_config_has_no_agents() {
        assert!(AppConfig::default().agents().is_empty());
        assert!(
            AppConfig::parse("# nothing\n")
                .expect("valid")
                .agents()
                .is_empty()
        );
        // A bare `[[agents]]` header is one empty entry, and an entry without
        // its required keys is a typed error, not a silently skipped row.
        assert!(
            AppConfig::parse("[[agents]]\n")
                .expect_err("an empty entry lacks required keys")
                .to_string()
                .contains("missing required key")
        );
    }

    #[test]
    fn agents_parse_in_file_order_with_name_command_and_args() {
        let config = AppConfig::parse(
            "[[agents]]\nname = \"claude\"\ncommand = \"/usr/local/bin/claude\"\n\
             args = [\"--login\", \"--theme\", \"dark\"]\n\
             [[agents]]\nname = \"aider\"\ncommand = \"/opt/homebrew/bin/aider\"\n",
        )
        .expect("valid agents configuration");
        let agents = config.agents();
        assert_eq!(agents.len(), 2, "file order is preserved");
        assert_eq!(agents[0].name(), "claude");
        assert_eq!(agents[0].command(), "/usr/local/bin/claude");
        assert_eq!(
            agents[0].args(),
            [
                "--login".to_owned(),
                "--theme".to_owned(),
                "dark".to_owned()
            ]
            .as_slice()
        );
        assert_eq!(
            agents[0].argv(),
            vec![
                "/usr/local/bin/claude".to_owned(),
                "--login".to_owned(),
                "--theme".to_owned(),
                "dark".to_owned(),
            ],
            "argv is the command followed by the configured args"
        );
        assert!(agents[1].args().is_empty());
        assert_eq!(agents[1].argv().len(), 1);
        // The rest of the configuration is untouched.
        assert_eq!(config.font(), FontConfig::default());
        assert_eq!(config.keys(), KeymapConfig::default());
    }

    /// Shell metacharacters are data, not syntax: an agent field carrying
    /// `;`, `$(...)`, or a backtick parses fine, because nothing here ever
    /// hands it to a shell.
    #[test]
    fn agent_fields_may_contain_shell_metacharacters_as_literal_data() {
        let config = AppConfig::parse(
            "[[agents]]\nname = \"rm -rf; $(evil) `x`\"\n\
             command = \"/bin/echo\"\nargs = [\"a;rm\", \"$(b)\", \"`c`\"]\n",
        )
        .expect("metacharacters are ordinary string content");
        assert_eq!(config.agents()[0].name(), "rm -rf; $(evil) `x`");
        assert_eq!(config.agents()[0].args().len(), 3);
    }

    #[test]
    fn agent_entries_reject_each_malformed_shape_with_a_typed_error() {
        let cases = [
            (
                "agents = 3\n",
                ConfigError::AgentTableNotAnArray {
                    key: "agents".to_owned(),
                },
            ),
            (
                "[agents]\nname = \"x\"\n",
                ConfigError::AgentTableNotAnArray {
                    key: "agents".to_owned(),
                },
            ),
            (
                "agents = [{ name = \"x\", command = \"/bin/true\" }]\n",
                ConfigError::AgentTableNotAnArray {
                    key: "agents".to_owned(),
                },
            ),
            (
                "[[agents]]\ncommand = \"/bin/true\"\n",
                ConfigError::AgentFieldMissing {
                    key: "name".to_owned(),
                },
            ),
            (
                "[[agents]]\nname = \"x\"\n",
                ConfigError::AgentFieldMissing {
                    key: "command".to_owned(),
                },
            ),
            (
                "[[agents]]\nname = 5\ncommand = \"/bin/true\"\n",
                ConfigError::AgentFieldNotAString {
                    key: "name".to_owned(),
                },
            ),
            (
                "[[agents]]\nname = \"x\"\ncommand = \"/bin/true\"\nargs = \"y\"\n",
                ConfigError::AgentArgsNotAnArray {
                    key: "args".to_owned(),
                },
            ),
            (
                "[[agents]]\nname = \"x\"\ncommand = \"/bin/true\"\nargs = [\"a\", 2]\n",
                ConfigError::AgentArgNotAString { index: 1 },
            ),
            (
                "[[agents]]\nname = \"x\"\ncommand = \"claude\"\n",
                ConfigError::AgentCommandNotAbsolute {
                    key: "command".to_owned(),
                },
            ),
            (
                "[[agents]]\nname = \"x\"\ncommand = \"/bin/true\"\nextra = 1\n",
                ConfigError::UnknownKey("extra".to_owned()),
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(AppConfig::parse(text), Err(expected), "{text:?}");
        }
    }

    #[test]
    fn agent_string_fields_are_range_checked_like_every_other_value() {
        let oversize = "a".repeat(MAX_AGENT_FIELD_BYTES + 1);
        for hostile in [
            "[[agents]]\nname = \"\"\ncommand = \"/bin/true\"\n".to_owned(),
            "[[agents]]\nname = \"x\"\ncommand = \"\"\n".to_owned(),
            format!("[[agents]]\nname = \"{oversize}\"\ncommand = \"/bin/true\"\n"),
            format!("[[agents]]\nname = \"x\"\ncommand = \"/{oversize}\"\n"),
            format!("[[agents]]\nname = \"x\"\ncommand = \"/bin/true\"\nargs = [\"{oversize}\"]\n"),
        ] {
            let result = AppConfig::parse(&hostile);
            assert!(
                matches!(result, Err(ConfigError::OutOfRange { .. })),
                "{hostile:.60?}… must be rejected as out of range, got {result:?}"
            );
        }
        // The cap itself is accepted.
        let at_cap = "a".repeat(MAX_AGENT_FIELD_BYTES);
        assert!(
            AppConfig::parse(&format!(
                "[[agents]]\nname = \"{at_cap}\"\ncommand = \"/bin/true\"\n"
            ))
            .is_ok()
        );
    }

    /// Agent fields are file values: no error surface may echo them, and the
    /// configuration's own Debug is shape-only (issue #146).
    #[test]
    fn agent_values_never_reach_errors_or_config_debug() {
        const SENTINEL: &str = "NOREN-AGENT-VALUE-hunter2";
        let cases = [
            // Relative command: the typed refusal names the key, not the text.
            format!("[[agents]]\nname = \"x\"\ncommand = \"{SENTINEL}\"\n"),
            // Wrong-typed fields name the key only.
            format!("[[agents]]\nname = \"{SENTINEL}\"\ncommand = 5\n"),
            format!("[[agents]]\nname = 5\ncommand = \"/{SENTINEL}\"\n"),
            format!(
                "[[agents]]\nname = \"x\"\ncommand = \"/bin/true\"\nargs = [\"{SENTINEL}\", 1]\n"
            ),
        ];
        for text in cases {
            let error = AppConfig::parse(&text).expect_err("every sentinel fixture fails");
            let display = error.to_string();
            assert!(
                !display.contains(SENTINEL),
                "Display must not echo agent values: {display}"
            );
            let debug = format!("{error:?}");
            assert!(
                !debug.contains(SENTINEL),
                "Debug must not echo agent values: {debug}"
            );
        }
        // A VALID configuration never prints its fields through Debug either.
        let config = AppConfig::parse(&format!(
            "[[agents]]\nname = \"{SENTINEL}\"\ncommand = \"/bin/{SENTINEL}\"\nargs = [\"{SENTINEL}\"]\n"
        ))
        .expect("valid");
        let rendered = format!("{config:?}");
        assert!(
            !rendered.contains(SENTINEL),
            "AgentConfig Debug leaked file text: {rendered}"
        );
    }

    /// The config-layer per-field cap and the policy-layer per-element cap
    /// are the same number: a configuration the schema accepted can never be
    /// refused by [`noren_pty::AgentLaunchPolicy`], while the policy still
    /// enforces the bound against callers that skipped this schema. Diverging
    /// the constants would create a silent dead range in one direction and a
    /// launch-time surprise in the other, so the pin is a test.
    #[test]
    fn agent_field_cap_matches_the_launch_policy_element_cap() {
        assert_eq!(
            MAX_AGENT_FIELD_BYTES,
            noren_pty::MAX_AGENT_ARGV_ELEMENT_BYTES,
            "the config and policy layers must enforce the same argv element cap"
        );
        // The caps compose, not just match: the largest schema-accepted
        // command and arg construct a valid policy. The command's cap
        // includes its leading `/`.
        let command = format!("/{}", "a".repeat(MAX_AGENT_FIELD_BYTES - 1));
        let arg = "a".repeat(MAX_AGENT_FIELD_BYTES);
        let policy = noren_pty::AgentLaunchPolicy::new(&command, &[arg])
            .expect("a schema-accepted element always fits the policy");
        assert_eq!(policy.args().len(), 1);
    }

    // ── [[projects]] tests ─────────────────────────────────────────────

    /// With no `[[projects]]` entries the configured list is empty — exactly
    /// the pre-projects behavior.
    #[test]
    fn default_config_has_no_projects() {
        assert!(AppConfig::default().projects().is_empty());
        assert!(
            AppConfig::parse("# nothing\n")
                .expect("valid")
                .projects()
                .is_empty()
        );
        // A bare `[[projects]]` header is one empty entry, and an entry
        // without its required keys is a typed error, not a skipped row.
        assert!(
            AppConfig::parse("[[projects]]\n")
                .expect_err("an empty entry lacks required keys")
                .to_string()
                .contains("missing required key")
        );
    }

    #[test]
    fn projects_parse_in_file_order_with_name_and_root() {
        let config = AppConfig::parse(
            "[[projects]]\nname = \"noren\"\nroot = \"/Users/dev/noren\"\n\
             [[projects]]\nname = \"zellij\"\nroot = \"/Users/dev/tooling/zellij\"\n",
        )
        .expect("valid projects configuration");
        let projects = config.projects();
        assert_eq!(projects.len(), 2, "file order is preserved");
        assert_eq!(projects[0].name(), "noren");
        assert_eq!(projects[0].root(), "/Users/dev/noren");
        assert_eq!(projects[1].name(), "zellij");
        assert_eq!(projects[1].root(), "/Users/dev/tooling/zellij");
        // The rest of the configuration is untouched.
        assert_eq!(config.font(), FontConfig::default());
        assert_eq!(config.keys(), KeymapConfig::default());
        // Projects compose with agents in one file.
        let combined = AppConfig::parse(
            "[[agents]]\nname = \"a\"\ncommand = \"/bin/true\"\n\
             [[projects]]\nname = \"p\"\nroot = \"/srv/p\"\n",
        )
        .expect("both arrays are valid together");
        assert_eq!(combined.agents().len(), 1);
        assert_eq!(combined.projects().len(), 1);
    }

    #[test]
    fn project_entries_reject_each_malformed_shape_with_a_typed_error() {
        let cases = [
            (
                "projects = 3\n",
                ConfigError::ProjectTableNotAnArray {
                    key: "projects".to_owned(),
                },
            ),
            (
                "[projects]\nname = \"x\"\n",
                ConfigError::ProjectTableNotAnArray {
                    key: "projects".to_owned(),
                },
            ),
            (
                "projects = [{ name = \"x\", root = \"/srv/x\" }]\n",
                ConfigError::ProjectTableNotAnArray {
                    key: "projects".to_owned(),
                },
            ),
            (
                "[[projects]]\nroot = \"/srv/x\"\n",
                ConfigError::ProjectFieldMissing {
                    key: "name".to_owned(),
                },
            ),
            (
                "[[projects]]\nname = \"x\"\n",
                ConfigError::ProjectFieldMissing {
                    key: "root".to_owned(),
                },
            ),
            (
                "[[projects]]\nname = 5\nroot = \"/srv/x\"\n",
                ConfigError::ProjectFieldNotAString {
                    key: "name".to_owned(),
                },
            ),
            (
                "[[projects]]\nname = \"x\"\nroot = \"srv/x\"\n",
                ConfigError::ProjectRootNotAbsolute {
                    key: "root".to_owned(),
                },
            ),
            (
                "[[projects]]\nname = \"x\"\nroot = \"~/dev/x\"\n",
                ConfigError::ProjectRootNotAbsolute {
                    key: "root".to_owned(),
                },
            ),
            (
                "[[projects]]\nname = \"x\"\nroot = \"/srv/x\"\nextra = 1\n",
                ConfigError::UnknownKey("extra".to_owned()),
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(AppConfig::parse(text), Err(expected), "{text:?}");
        }
    }

    #[test]
    fn project_string_fields_are_range_checked_like_every_other_value() {
        let oversize = "a".repeat(MAX_PROJECT_FIELD_BYTES + 1);
        for hostile in [
            "[[projects]]\nname = \"\"\nroot = \"/srv/x\"\n".to_owned(),
            "[[projects]]\nname = \"x\"\nroot = \"\"\n".to_owned(),
            format!("[[projects]]\nname = \"{oversize}\"\nroot = \"/srv/x\"\n"),
            format!("[[projects]]\nname = \"x\"\nroot = \"/{oversize}\"\n"),
        ] {
            let result = AppConfig::parse(&hostile);
            assert!(
                matches!(result, Err(ConfigError::OutOfRange { .. })),
                "{hostile:.60?}… must be rejected as out of range, got {result:?}"
            );
        }
        // The cap itself is accepted; a root's cap includes its leading `/`,
        // like an agent command's.
        let at_cap = "a".repeat(MAX_PROJECT_FIELD_BYTES);
        let root_at_cap = "a".repeat(MAX_PROJECT_FIELD_BYTES - 1);
        assert!(
            AppConfig::parse(&format!(
                "[[projects]]\nname = \"{at_cap}\"\nroot = \"/{root_at_cap}\"\n"
            ))
            .is_ok()
        );
    }

    /// Project fields are file values: no error surface may echo them, and
    /// the configuration's own Debug is shape-only (issue #146).
    #[test]
    fn project_values_never_reach_errors_or_config_debug() {
        const SENTINEL: &str = "NOREN-PROJECT-VALUE-hunter2";
        let cases = [
            // Relative root: the typed refusal names the key, not the text.
            format!("[[projects]]\nname = \"x\"\nroot = \"{SENTINEL}\"\n"),
            // Tilde root: still not absolute, still key-only.
            format!("[[projects]]\nname = \"x\"\nroot = \"~/{SENTINEL}\"\n"),
            // Wrong-typed fields name the key only.
            format!("[[projects]]\nname = \"{SENTINEL}\"\nroot = 5\n"),
            format!("[[projects]]\nname = 5\nroot = \"/{SENTINEL}\"\n"),
        ];
        for text in cases {
            let error = AppConfig::parse(&text).expect_err("every sentinel fixture fails");
            let display = error.to_string();
            assert!(
                !display.contains(SENTINEL),
                "Display must not echo project values: {display}"
            );
            let debug = format!("{error:?}");
            assert!(
                !debug.contains(SENTINEL),
                "Debug must not echo project values: {debug}"
            );
        }
        // A VALID configuration never prints its fields through Debug either.
        let config = AppConfig::parse(&format!(
            "[[projects]]\nname = \"{SENTINEL}\"\nroot = \"/Users/{SENTINEL}/p\"\n"
        ))
        .expect("valid");
        let rendered = format!("{config:?}");
        assert!(
            !rendered.contains(SENTINEL),
            "ProjectConfig Debug leaked file text: {rendered}"
        );
    }

    /// The two array-of-tables field caps are one policy: a project field can
    /// never outgrow the bound an agent field is held to, and vice versa.
    #[test]
    fn project_field_cap_matches_the_agent_field_cap() {
        assert_eq!(
            MAX_PROJECT_FIELD_BYTES, MAX_AGENT_FIELD_BYTES,
            "the [[projects]] and [[agents]] field caps must not drift apart"
        );
    }
}
