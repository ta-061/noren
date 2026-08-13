//! Bounded, read-only parsing of the user's OpenSSH client configuration.
//!
//! This is an app-owned filesystem adapter rather than a session or transport
//! implementation. It produces bounded host facts consumed by the sidebar; it
//! never opens an SSH connection and never opens a path named by `IdentityFile`
//! (or any other non-`Include` directive).
//!
//! `HostName` and `User` values are retained as discovery metadata, not as
//! connection-ready values. In particular, percent tokens such as `%h`, `%p`,
//! and `%r` remain unexpanded. A future connection path must resolve them with
//! OpenSSH-equivalent semantics or reject them before use.
//!
//! File-backed parsing intentionally confines every `Include` target to the
//! canonical parent directory of the top-level config. This is stricter than
//! OpenSSH: absolute, `~`, `..`, and symlinked targets are ignored when their
//! canonical destination escapes that root.

use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
use std::fs::OpenOptions;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

/// Maximum include nesting. Cycles are also stopped by the active recursion
/// stack, so a file may be included again once its prior invocation returns.
pub const MAX_INCLUDE_DEPTH: usize = 16;

/// Maximum bytes accepted from any one config source, including [`SshConfig::parse`].
const MAX_FILE_BYTES: usize = 1024 * 1024;

/// Maximum number of include matches followed during one top-level parse.
const MAX_INCLUDED_FILES: usize = 256;

/// Maximum aggregate source bytes retained across the top-level file and all
/// includes. Eight MiB is far beyond ordinary OpenSSH configs while keeping
/// the parser's retained source-derived data to a sane application-sized cap.
const MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;

/// Maximum include-expansion units across one parse.
///
/// Each pattern, directory entry, path component, wildcard match, and
/// frontier-wide literal push is charged in content-sized units before work or
/// allocation. A match costs `(pattern_bytes + 1) * (candidate_bytes + 1)` and
/// a frontier-wide push costs `(component_bytes + 1) * frontier_width`.
/// Sixty-four ki-units permits large real directories while bounding
/// adversarial directory and branching-glob products.
const MAX_INCLUDE_EXPANSION_WORK: usize = 64 * 1024;

/// Maximum emitted tokens (directive keywords and arguments) accumulated across
/// the top-level source and every Include occurrence. The eight-MiB source cap
/// does not bound `Vec`/`String`/`HashMap` item overhead, so each completed
/// token is charged before it is pushed, covering ignored directives too.
const MAX_TOKEN_ITEMS: usize = 262_144;

/// Maximum distinct discovered hosts. A new alias beyond this cap is rejected
/// before any host, index, or output state is added for it.
const MAX_HOSTS: usize = 65_536;

/// Maximum retained bytes in a user-facing SSH source label, including its
/// stable ordinal tag. Canonical paths are used only transiently for parser
/// identity; the retained label is root-relative and bounded by this value.
pub const MAX_SSH_SOURCE_LABEL_BYTES: usize = 64;

/// Maximum conservative work estimate for discovering and resolving aliases.
///
/// Before either cross-product runs, every alias/pattern pair is charged
/// `(alias_bytes + 1) * (pattern_bytes + 1)` units. This covers conversion to
/// characters and the iterative glob matcher. Every indexed and fallback
/// block visit is charged, and setting work includes both the visit and bytes
/// that may be cloned into a host. Sixteen mebi-units leaves ample room for
/// realistic configurations while rejecting hostile products before they are
/// evaluated.
const MAX_RESOLUTION_WORK: u128 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
struct ParserLimits {
    file_bytes: usize,
    included_files: usize,
    total_bytes: usize,
    include_expansion_work: usize,
    resolution_work: u128,
    token_items: usize,
    hosts: usize,
}

const DEFAULT_LIMITS: ParserLimits = ParserLimits {
    file_bytes: MAX_FILE_BYTES,
    included_files: MAX_INCLUDED_FILES,
    total_bytes: MAX_TOTAL_BYTES,
    include_expansion_work: MAX_INCLUDE_EXPANSION_WORK,
    resolution_work: MAX_RESOLUTION_WORK,
    token_items: MAX_TOKEN_ITEMS,
    hosts: MAX_HOSTS,
};

/// Scope of host discovery performed by this parser.
///
/// Noren does not ask OpenSSH to evaluate destinations here. Wildcard hosts,
/// `Match` conditions, token expansion, system configuration, and other
/// dynamic behavior therefore cannot produce a complete destination list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HostDiscoveryKind {
    /// Only positive literal aliases written in `Host` directives are listed.
    #[default]
    PartialLiteralPatterns,
}

/// Stable, parse-local identity of an SSH configuration source.
///
/// Zero is the top-level source (or the synthetic inline source used by
/// [`SshConfig::parse`]); successfully read included files receive increasing
/// ordinals on first encounter. Repeated includes reuse the same identity.
/// IDs are ordinal tokens, not globally unique: an ID must only be resolved
/// through the [`SshConfig`] that produced it. The same ordinal from a different
/// parse may identify a different source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SshSourceId(usize);

impl SshSourceId {
    /// Encounter-order ordinal within this [`SshConfig`].
    #[must_use]
    pub const fn ordinal(self) -> usize {
        self.0
    }
}

/// Bounded, user-facing provenance for one parsed configuration source.
///
/// The label base is either `inline` or an ASCII-escaped/lossy path relative to
/// the top-level configuration directory. It never contains the canonical root
/// or home-directory prefix. The ordinal tag is appended to the label so two
/// paths with the same bounded prefix remain distinguishable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshSource {
    id: SshSourceId,
    tag: String,
    label: String,
}

impl SshSource {
    /// Parse-local source identity.
    #[must_use]
    pub const fn id(&self) -> SshSourceId {
        self.id
    }

    /// Compact stable tag, such as `#0` or `#12`.
    #[must_use]
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// Bounded root-relative display label with the stable tag appended.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
}

fn inline_source(id: SshSourceId) -> SshSource {
    let tag = format!("#{}", id.ordinal());
    let label = bounded_source_label("inline", &tag);
    SshSource { id, tag, label }
}

fn file_source(id: SshSourceId, source: &Path, root: &Path) -> SshSource {
    let tag = format!("#{}", id.ordinal());
    let relative = source
        .strip_prefix(root)
        .ok()
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config".to_owned());
    let label = bounded_source_label(&relative, &tag);
    SshSource { id, tag, label }
}

fn bounded_source_label(raw: &str, tag: &str) -> String {
    const TRUNCATION_MARKER: &str = "...";

    let suffix = format!(" {tag}");
    let content_limit = MAX_SSH_SOURCE_LABEL_BYTES.saturating_sub(suffix.len());
    let mut escaped = String::new();
    let mut fragment_ends = Vec::new();
    for character in raw.chars() {
        if (character.is_ascii_graphic() && character != '\\') || character == ' ' {
            escaped.push(character);
        } else {
            escaped.extend(character.escape_default());
        }
        fragment_ends.push(escaped.len());
    }

    let content = if escaped.len() <= content_limit {
        escaped
    } else {
        let truncated_limit = content_limit.saturating_sub(TRUNCATION_MARKER.len());
        let boundary = fragment_ends
            .into_iter()
            .take_while(|end| *end <= truncated_limit)
            .last()
            .unwrap_or(0);
        escaped.truncate(boundary);
        escaped.push_str(TRUNCATION_MARKER);
        escaped
    };

    format!("{content}{suffix}")
}

/// Parsed SSH host-discovery facts.
///
/// Values are the first values obtained for the alias, following OpenSSH's
/// per-keyword precedence rule, but they are not an effective connection
/// configuration. Percent tokens such as `%h`, `%p`, and `%r` remain literal;
/// callers must not pass unresolved values to a future connection path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshHost {
    alias: String,
    host_name: Option<String>,
    user: Option<String>,
    port: Option<u16>,
    declared_source: SshSourceId,
}

impl SshHost {
    /// The alias named by a literal `Host` pattern.
    #[must_use]
    pub fn alias(&self) -> &str {
        &self.alias
    }

    /// The configured `HostName`, if one was provided.
    ///
    /// This is the literal, unexpanded value and may still contain percent
    /// tokens. It is discovery metadata, not a connection-ready hostname.
    #[must_use]
    pub fn host_name(&self) -> Option<&str> {
        self.host_name.as_deref()
    }

    /// The configured `User`, if one was provided.
    ///
    /// This is the literal, unexpanded value and may still contain percent
    /// tokens. It is discovery metadata, not a connection-ready login name.
    #[must_use]
    pub fn user(&self) -> Option<&str> {
        self.user.as_deref()
    }

    /// The configured `Port`, if one was provided.
    #[must_use]
    pub const fn port(&self) -> Option<u16> {
        self.port
    }

    /// Source of the first qualifying positive literal `Host` declaration.
    ///
    /// Later blocks may supply effective values under OpenSSH's first-value
    /// rules, but they do not replace this declaration provenance.
    #[must_use]
    pub const fn declared_source(&self) -> SshSourceId {
        self.declared_source
    }
}

/// Parsed SSH configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SshConfig {
    hosts: Vec<SshHost>,
    sources: Vec<SshSource>,
    discovery_kind: HostDiscoveryKind,
}

impl SshConfig {
    /// Read an explicit OpenSSH config path.
    ///
    /// A missing or unreadable top-level file is treated as an empty config.
    /// Errors from a file that can be read are limited to malformed input and
    /// bounded-input failures; no error includes source text. Each source is
    /// capped at one MiB, aggregate source text at eight MiB, include matches
    /// at 256, include expansion at 65,536 charged units, emitted tokens at
    /// 262,144, and distinct discovered hosts at 65,536. Relative `Include`
    /// paths are resolved from the top-level path's parent, including in files
    /// reached through nested includes. Every candidate must canonicalize
    /// beneath that canonical parent. This confinement is intentionally
    /// stricter than OpenSSH: an absolute, `~`, `..`, or symlinked target whose
    /// canonical destination escapes the root is ignored.
    pub fn read(path: &Path) -> Result<Self, SshConfigError> {
        Self::read_with_limits(path, DEFAULT_LIMITS)
    }

    fn read_with_limits(path: &Path, limits: ParserLimits) -> Result<Self, SshConfigError> {
        let root = path
            .parent()
            .and_then(|parent| fs::canonicalize(parent).ok());
        let Some(root) = root else {
            return Ok(Self::default());
        };
        let source = canonicalize_within(path, &root);
        let Some(source) = source else {
            return Ok(Self::default());
        };
        let Some(text) = read_text_file(&source, 0, limits.file_bytes)? else {
            return Ok(Self::default());
        };
        if text.len() > limits.total_bytes {
            return Err(error(0, SshConfigErrorKind::TotalBytesExceeded));
        }
        Self::from_text_with_includes(&text, &source, &root, limits)
    }

    /// Read the conventional per-user configuration at `~/.ssh/config`.
    ///
    /// If `HOME` is not available, or the file cannot be read, the result is
    /// an empty host list.
    pub fn read_default() -> Result<Self, SshConfigError> {
        let Some(home) = home_directory(env::var_os("HOME")) else {
            return Ok(Self::default());
        };
        Self::read(&home.join(".ssh/config"))
    }

    /// Parse configuration text without resolving `Include` directives.
    ///
    /// This is useful for deterministic callers that already expanded their
    /// source, and keeps the parser independently testable. Use [`Self::read`]
    /// for normal file loading. Inputs larger than one MiB are rejected before
    /// tokenization. `HostName` and `User` percent tokens remain literal; the
    /// returned facts must not be treated as connection-ready values.
    pub fn parse(text: &str) -> Result<Self, SshConfigError> {
        Self::parse_with_limits(text, DEFAULT_LIMITS)
    }

    fn parse_with_limits(text: &str, limits: ParserLimits) -> Result<Self, SshConfigError> {
        if text.len() > limits.file_bytes {
            return Err(error(0, SshConfigErrorKind::FileTooLarge));
        }
        let inline_id = SshSourceId(0);
        let mut state = ParseState::new(text.len(), limits);
        state.sources.push(inline_source(inline_id));
        parse_source(text, 0, inline_id, IncludeHandling::Ignore, &mut state)?;
        let ParseState {
            blocks, sources, ..
        } = state;
        Self::from_blocks_with_limit(&blocks, sources, limits)
    }

    /// The concrete hosts discovered from literal `Host` patterns.
    #[must_use]
    pub fn hosts(&self) -> &[SshHost] {
        &self.hosts
    }

    /// Explicitly partial scope used to discover [`Self::hosts`].
    #[must_use]
    pub const fn discovery_kind(&self) -> HostDiscoveryKind {
        self.discovery_kind
    }

    /// Bounded provenance for a parse-local `id` produced by this config.
    ///
    /// IDs are ordinal tokens, so an ID from another parse with the same
    /// ordinal may resolve to an unrelated source here. Callers must pair IDs
    /// with the [`SshConfig`] that produced them. Returns `None` only when this
    /// config has no source at the ID's ordinal.
    #[must_use]
    pub fn source(&self, id: SshSourceId) -> Option<&SshSource> {
        self.sources
            .get(id.ordinal())
            .filter(|source| source.id == id)
    }

    /// Bounded sources that contributed to this parse, in first-encounter
    /// order. A missing or unreadable top-level file has no sources.
    #[must_use]
    pub fn sources(&self) -> &[SshSource] {
        &self.sources
    }

    fn from_text_with_includes(
        text: &str,
        source: &Path,
        root: &Path,
        limits: ParserLimits,
    ) -> Result<Self, SshConfigError> {
        let mut state = ParseState::new(text.len(), limits);
        let source_id = state.intern_source(source, root);
        parse_file(text, source, source_id, root, 0, &mut state)?;
        let ParseState {
            blocks, sources, ..
        } = state;
        Self::from_blocks_with_limit(&blocks, sources, limits)
    }

    fn from_blocks_with_limit(
        blocks: &[Block],
        sources: Vec<SshSource>,
        limits: ParserLimits,
    ) -> Result<Self, SshConfigError> {
        let discovery_match_work = blocks.iter().fold(0_u128, |total, block| {
            let Some(patterns) = &block.patterns else {
                return total;
            };
            let literal_work = patterns
                .iter()
                .filter(|pattern| !pattern.starts_with('!') && !has_wildcard(pattern))
                .fold(0_u128, |work, pattern| {
                    work.saturating_add(pattern.len() as u128 + 1)
                });
            let negative_work = patterns
                .iter()
                .filter_map(|pattern| pattern.strip_prefix('!'))
                .fold(0_u128, |work, pattern| {
                    work.saturating_add(pattern.len() as u128 + 1)
                });
            total.saturating_add(literal_work.saturating_mul(negative_work))
        });
        let mut resolution_work = ResolutionWork {
            discovery_match_work,
            ..ResolutionWork::default()
        };
        ensure_resolution_work(resolution_work, limits.resolution_work)?;

        let mut aliases = Vec::new();
        let mut seen_aliases = HashSet::new();
        let mut literal_blocks = HashMap::<String, Vec<usize>>::new();
        let mut fallback_blocks = Vec::new();
        let mut fallback_pattern_work = 0_u128;
        let mut fallback_setting_work = 0_u128;

        // Literal-only blocks can be looked up by alias. Blocks with a wildcard
        // (and global blocks) remain in their original order for every alias.
        // All of this indexing is linear in the parsed input; no alias is
        // matched against a fallback block until the budget check below passes.
        for (block_index, block) in blocks.iter().enumerate() {
            let Some(patterns) = &block.patterns else {
                fallback_blocks.push(block_index);
                fallback_setting_work = fallback_setting_work.saturating_add(block.settings_work());
                continue;
            };

            let negative_patterns: Vec<_> = patterns
                .iter()
                .filter_map(|candidate| candidate.strip_prefix('!'))
                .collect();
            for pattern in patterns
                .iter()
                .filter(|pattern| !pattern.starts_with('!') && !has_wildcard(pattern))
            {
                let cancelled = negative_patterns
                    .iter()
                    .any(|negative| wildcard_match(negative, pattern));
                if !cancelled {
                    let key = pattern.to_ascii_lowercase();
                    if !seen_aliases.contains(&key) {
                        // Reject a new distinct alias beyond the cap before any
                        // host, index, or output state is added for it.
                        // Case-insensitive duplicates do not consume a slot.
                        if aliases.len() >= limits.hosts {
                            return Err(error(0, SshConfigErrorKind::HostCountExceeded));
                        }
                        seen_aliases.insert(key);
                        aliases.push((pattern.clone(), block.source));
                    }
                }
            }

            if patterns.iter().all(|pattern| !has_wildcard(pattern)) {
                // An all-literal block is known to have a positive match when
                // reached through this index. Remove entries cancelled by a
                // case-insensitive negation now, so resolving one alias never
                // scans all patterns in a multi-alias literal block.
                let negated: HashSet<String> = patterns
                    .iter()
                    .filter_map(|pattern| pattern.strip_prefix('!'))
                    .map(str::to_ascii_lowercase)
                    .collect();
                let mut indexed_in_block = HashSet::new();
                for pattern in patterns.iter().filter(|pattern| !pattern.starts_with('!')) {
                    let key = pattern.to_ascii_lowercase();
                    if !negated.contains(&key) && indexed_in_block.insert(key.clone()) {
                        literal_blocks.entry(key).or_default().push(block_index);
                        resolution_work.indexed_block_visits =
                            resolution_work.indexed_block_visits.saturating_add(1);
                        resolution_work.indexed_setting_work = resolution_work
                            .indexed_setting_work
                            .saturating_add(block.settings_work());
                    }
                }
            } else {
                fallback_blocks.push(block_index);
                fallback_setting_work = fallback_setting_work.saturating_add(block.settings_work());
                for pattern in patterns {
                    let match_pattern = pattern.strip_prefix('!').unwrap_or(pattern);
                    fallback_pattern_work =
                        fallback_pattern_work.saturating_add(match_pattern.len() as u128 + 1);
                }
            }
        }

        let alias_count = aliases.len() as u128;
        let alias_work = aliases.iter().fold(0_u128, |work, (alias, _)| {
            work.saturating_add(alias.len() as u128 + 1)
        });
        resolution_work.alias_index_work = alias_work;
        resolution_work.fallback_match_work = alias_work.saturating_mul(fallback_pattern_work);
        resolution_work.fallback_block_visits =
            (fallback_blocks.len() as u128).saturating_mul(alias_count);
        resolution_work.fallback_setting_work = fallback_setting_work.saturating_mul(alias_count);
        ensure_resolution_work(resolution_work, limits.resolution_work)?;

        let hosts = aliases
            .into_iter()
            .map(|(alias, declared_source)| {
                let mut host = SshHost {
                    alias: alias.clone(),
                    host_name: None,
                    user: None,
                    port: None,
                    declared_source,
                };

                let indexed = literal_blocks
                    .get(&alias.to_ascii_lowercase())
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                let mut indexed_position = 0;
                let mut fallback_position = 0;
                // Merge both ordered views so per-keyword first-value-wins
                // precedence remains identical to scanning all blocks.
                while indexed_position < indexed.len() || fallback_position < fallback_blocks.len()
                {
                    let (block_index, needs_match) = match (
                        indexed.get(indexed_position),
                        fallback_blocks.get(fallback_position),
                    ) {
                        (Some(indexed), Some(fallback)) if indexed < fallback => {
                            indexed_position += 1;
                            (*indexed, false)
                        }
                        (Some(_), Some(fallback)) => {
                            fallback_position += 1;
                            (*fallback, true)
                        }
                        (Some(indexed), None) => {
                            indexed_position += 1;
                            (*indexed, false)
                        }
                        (None, Some(fallback)) => {
                            fallback_position += 1;
                            (*fallback, true)
                        }
                        (None, None) => unreachable!(),
                    };
                    let block = &blocks[block_index];
                    if needs_match && !block.applies_to(&alias) {
                        continue;
                    }
                    apply_settings(&mut host, block);
                }
                host
            })
            .collect();
        Ok(Self {
            hosts,
            sources,
            discovery_kind: HostDiscoveryKind::PartialLiteralPatterns,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ResolutionWork {
    discovery_match_work: u128,
    alias_index_work: u128,
    indexed_block_visits: u128,
    indexed_setting_work: u128,
    fallback_match_work: u128,
    fallback_block_visits: u128,
    fallback_setting_work: u128,
}

impl ResolutionWork {
    fn total(self) -> u128 {
        [
            self.discovery_match_work,
            self.alias_index_work,
            self.indexed_block_visits,
            self.indexed_setting_work,
            self.fallback_match_work,
            self.fallback_block_visits,
            self.fallback_setting_work,
        ]
        .into_iter()
        .fold(0_u128, u128::saturating_add)
    }
}

fn ensure_resolution_work(work: ResolutionWork, limit: u128) -> Result<(), SshConfigError> {
    if work.total() > limit {
        return Err(error(0, SshConfigErrorKind::ResolutionComplexityExceeded));
    }
    Ok(())
}

fn apply_settings(host: &mut SshHost, block: &Block) {
    for setting in &block.settings {
        match setting {
            Setting::HostName(value) if host.host_name.is_none() => {
                host.host_name = Some(value.clone());
            }
            Setting::User(value) if host.user.is_none() => {
                host.user = Some(value.clone());
            }
            Setting::Port(value) if host.port.is_none() => {
                host.port = Some(*value);
            }
            _ => {}
        }
    }
}

/// Safe, content-free parser failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshConfigError {
    line: usize,
    kind: SshConfigErrorKind,
}

impl SshConfigError {
    /// The one-based line number when the error came from a text directive.
    /// Zero denotes a file-level error such as invalid UTF-8.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }

    /// The safe category of the failure.
    #[must_use]
    pub const fn kind(&self) -> &SshConfigErrorKind {
        &self.kind
    }
}

/// Categories exposed by [`SshConfigError`]. None carries source content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SshConfigErrorKind {
    /// A directive needs an argument that was not present.
    MissingArgument,
    /// A supported single-argument directive has extra arguments.
    SurplusArgument,
    /// A `Host` directive has no patterns.
    MissingHostPattern,
    /// A `Port` argument is not a valid TCP port.
    InvalidPort,
    /// The file is not valid UTF-8.
    InvalidUtf8,
    /// The bounded input size was exceeded.
    FileTooLarge,
    /// The aggregate bytes across the top-level source and includes exceeded
    /// the parser-wide retained-input bound.
    TotalBytesExceeded,
    /// More include matches were found than the parser will follow.
    IncludedFilesExceeded,
    /// Directory inspection or path expansion exceeded the global Include
    /// work bound.
    IncludeExpansionWorkExceeded,
    /// Discovering or resolving literal aliases would exceed the parser's
    /// deterministic preflight work budget.
    ResolutionComplexityExceeded,
    /// The number of emitted tokens (directive keywords and arguments) across
    /// the top-level source and all Include occurrences exceeded the parser's
    /// structural cap.
    StructuralComplexityExceeded,
    /// The number of distinct discovered hosts exceeded the parser's cap.
    HostCountExceeded,
    /// A quoted or escaped argument was not terminated.
    UnterminatedArgument,
}

impl fmt::Display for SshConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "SSH configuration error: {:?}", self.kind)
        } else {
            write!(
                f,
                "SSH configuration error at line {}: {:?}",
                self.line, self.kind
            )
        }
    }
}

impl std::error::Error for SshConfigError {}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Block {
    /// `None` is the global pre-`Host` section. An empty list is an inactive
    /// `Match` section, which this configuration-only slice does not resolve.
    patterns: Option<Vec<String>>,
    settings: Vec<Setting>,
    source: SshSourceId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Setting {
    HostName(String),
    User(String),
    Port(u16),
}

struct ParseState {
    /// Active recursion stack of canonical source paths. A file is skipped only
    /// while it is still being parsed (direct and indirect cycles); once its
    /// invocation returns it may be included again in a different caller
    /// context, consuming the global limits each time.
    active: HashSet<PathBuf>,
    included_files: usize,
    total_bytes: usize,
    token_items: usize,
    include_expansion: IncludeExpansionBudget,
    blocks: Vec<Block>,
    current: Option<usize>,
    source_ids: HashMap<PathBuf, SshSourceId>,
    sources: Vec<SshSource>,
    limits: ParserLimits,
}

impl ParseState {
    fn new(top_level_bytes: usize, limits: ParserLimits) -> Self {
        Self {
            active: HashSet::new(),
            included_files: 0,
            total_bytes: top_level_bytes,
            token_items: 0,
            include_expansion: IncludeExpansionBudget::new(limits.include_expansion_work),
            blocks: Vec::new(),
            current: None,
            source_ids: HashMap::new(),
            sources: Vec::new(),
            limits,
        }
    }

    fn intern_source(&mut self, source: &Path, root: &Path) -> SshSourceId {
        if let Some(id) = self.source_ids.get(source) {
            return *id;
        }

        let id = SshSourceId(self.sources.len());
        self.source_ids.insert(source.to_path_buf(), id);
        self.sources.push(file_source(id, source, root));
        id
    }

    fn charge_source_bytes(&mut self, bytes: usize, line: usize) -> Result<(), SshConfigError> {
        let Some(total) = self.total_bytes.checked_add(bytes) else {
            return Err(error(line, SshConfigErrorKind::TotalBytesExceeded));
        };
        if total > self.limits.total_bytes {
            return Err(error(line, SshConfigErrorKind::TotalBytesExceeded));
        }
        self.total_bytes = total;
        Ok(())
    }
}

#[derive(Debug)]
struct IncludeExpansionBudget {
    used: usize,
    limit: usize,
}

impl IncludeExpansionBudget {
    const fn new(limit: usize) -> Self {
        Self { used: 0, limit }
    }

    fn charge(&mut self, amount: usize, line: usize) -> Result<(), SshConfigError> {
        let Some(used) = self.used.checked_add(amount) else {
            return Err(error(
                line,
                SshConfigErrorKind::IncludeExpansionWorkExceeded,
            ));
        };
        if used > self.limit {
            return Err(error(
                line,
                SshConfigErrorKind::IncludeExpansionWorkExceeded,
            ));
        }
        self.used = used;
        Ok(())
    }
}

impl Block {
    fn push_setting(&mut self, setting: Setting) {
        // If an earlier block already supplied this keyword, neither value in
        // this block can win. Otherwise this block's first value wins. A later
        // same-keyword value in the same block is therefore always inert,
        // including when Include directives assemble the block in pieces.
        if self
            .settings
            .iter()
            .any(|existing| existing.same_keyword(&setting))
        {
            return;
        }
        self.settings.push(setting);
    }

    fn settings_work(&self) -> u128 {
        self.settings.iter().fold(0_u128, |work, setting| {
            work.saturating_add(setting.resolution_work())
        })
    }

    fn applies_to(&self, alias: &str) -> bool {
        match &self.patterns {
            None => true,
            Some(patterns) if patterns.is_empty() => false,
            Some(patterns) => {
                let mut positive = false;
                for pattern in patterns {
                    if let Some(pattern) = pattern.strip_prefix('!') {
                        if wildcard_match(pattern, alias) {
                            return false;
                        }
                    } else if wildcard_match(pattern, alias) {
                        positive = true;
                    }
                }
                positive
            }
        }
    }
}

impl Setting {
    fn same_keyword(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::HostName(_), Self::HostName(_))
                | (Self::User(_), Self::User(_))
                | (Self::Port(_), Self::Port(_))
        )
    }

    fn resolution_work(&self) -> u128 {
        match self {
            Self::HostName(value) | Self::User(value) => value.len() as u128 + 1,
            Self::Port(_) => 1,
        }
    }
}

/// `Include` is the only directive whose behavior differs between the two
/// public parsing paths. Inline parsing keeps it opaque and performs no
/// filesystem access. File parsing follows it through the dedicated confined
/// handler below. All other directive semantics live in [`parse_source`].
#[derive(Clone, Copy, Debug)]
enum IncludeHandling<'a> {
    Ignore,
    FollowConfined { root: &'a Path, depth: usize },
}

fn parse_file(
    text: &str,
    source: &Path,
    source_id: SshSourceId,
    root: &Path,
    depth: usize,
    state: &mut ParseState,
) -> Result<(), SshConfigError> {
    // The active recursion stack stops direct and indirect cycles (A->B->A)
    // while a file is still being parsed. It does not prevent a file from being
    // included again once its prior invocation returns, so the same file may be
    // parsed in two different caller Host contexts. The entry is removed on both
    // the success and error paths so the stack always reflects active calls.
    if depth > MAX_INCLUDE_DEPTH || !state.active.insert(source.to_path_buf()) {
        return Ok(());
    }
    let result = parse_source(
        text,
        0,
        source_id,
        IncludeHandling::FollowConfined { root, depth },
        state,
    );
    state.active.remove(source);
    result
}

/// Behavior-preserving parsing core shared by inline text and every file,
/// including recursively followed sources. Keeping the line/token loop here
/// makes directive validation, block construction, source provenance, and
/// token charging identical on both paths; only `Include` delegates to the
/// explicitly selected policy.
fn parse_source(
    text: &str,
    line_offset: usize,
    source_id: SshSourceId,
    include_handling: IncludeHandling<'_>,
    state: &mut ParseState,
) -> Result<(), SshConfigError> {
    for (line_index, line) in text.lines().enumerate() {
        let line_number = line_index + 1 + line_offset;
        let tokens = tokenize(
            line,
            line_number,
            &mut state.token_items,
            state.limits.token_items,
        )?;
        if tokens.is_empty() {
            continue;
        }
        let keyword = tokens[0].to_ascii_lowercase();
        match keyword.as_str() {
            "host" => {
                if tokens.len() < 2 {
                    return Err(error(line_number, SshConfigErrorKind::MissingHostPattern));
                }
                state.blocks.push(Block {
                    patterns: Some(tokens[1..].to_vec()),
                    settings: Vec::new(),
                    source: source_id,
                });
                state.current = Some(state.blocks.len() - 1);
            }
            "match" => {
                state.blocks.push(Block {
                    patterns: Some(Vec::new()),
                    settings: Vec::new(),
                    source: source_id,
                });
                state.current = Some(state.blocks.len() - 1);
            }
            "include" => {
                if let IncludeHandling::FollowConfined { root, depth } = include_handling {
                    if tokens.len() < 2 {
                        return Err(error(line_number, SshConfigErrorKind::MissingArgument));
                    }
                    follow_confined_includes(&tokens[1..], root, depth, line_number, state)?;
                }
            }
            "hostname" | "user" | "port" => {
                let value = tokens
                    .get(1)
                    .ok_or_else(|| error(line_number, SshConfigErrorKind::MissingArgument))?;
                let setting = match keyword.as_str() {
                    "hostname" => Setting::HostName(value.clone()),
                    "user" => Setting::User(value.clone()),
                    "port" => Setting::Port(parse_port(value, line_number)?),
                    _ => unreachable!("keyword was matched above"),
                };
                // OpenSSH validates the first argument before reporting trailing
                // garbage (for example, `Port invalid extra` is InvalidPort).
                // Preserve that diagnostic ordering while rejecting every
                // extra argument for the supported single-value directives.
                if tokens.len() > 2 {
                    return Err(error(line_number, SshConfigErrorKind::SurplusArgument));
                }
                if let Some(index) = state.current {
                    state.blocks[index].push_setting(setting);
                } else {
                    let mut block = Block {
                        patterns: None,
                        settings: Vec::new(),
                        source: source_id,
                    };
                    block.push_setting(setting);
                    state.blocks.push(block);
                    state.current = Some(state.blocks.len() - 1);
                }
            }
            _ => {
                // OpenSSH has many keywords outside this slice. They remain
                // syntactically opaque and, importantly, are never opened.
            }
        }
    }
    Ok(())
}

/// Follow already-tokenized Include patterns without weakening any of the
/// filesystem boundary. The order below is security-significant: expansion is
/// globally budgeted before filesystem work, the include count is charged
/// before canonicalization/read, regular files are opened through the no-follow
/// seam, bytes are charged before parsing, and caller context is restored on
/// both success and error.
fn follow_confined_includes(
    patterns: &[String],
    root: &Path,
    depth: usize,
    line_number: usize,
    state: &mut ParseState,
) -> Result<(), SshConfigError> {
    // OpenSSH roots every relative Include in a user config at the user
    // configuration directory, represented here by `root`. Noren additionally
    // requires every canonical target to remain under that root.
    for pattern in patterns {
        let remaining = state
            .limits
            .included_files
            .saturating_sub(state.included_files);
        let included_paths = expand_include(
            pattern,
            root,
            remaining,
            &mut state.include_expansion,
            line_number,
        )?;
        for included in included_paths {
            if state.included_files >= state.limits.included_files {
                return Err(error(
                    line_number,
                    SshConfigErrorKind::IncludedFilesExceeded,
                ));
            }
            state.included_files += 1;
            let Some(included_source) = canonicalize_within(&included, root) else {
                continue;
            };
            if depth >= MAX_INCLUDE_DEPTH || state.active.contains(&included_source) {
                continue;
            }
            let Some(included_text) =
                read_text_file(&included_source, line_number, state.limits.file_bytes)?
            else {
                continue;
            };
            state.charge_source_bytes(included_text.len(), line_number)?;
            let included_source_id = state.intern_source(&included_source, root);
            // OpenSSH restores the caller's current Host context after an
            // included file returns, and settings at the start of an included
            // file attach to that caller context. Save and restore `current`
            // around the recursive parse, including on the error path.
            let saved_current = state.current;
            let result = parse_file(
                &included_text,
                &included_source,
                included_source_id,
                root,
                depth + 1,
                state,
            );
            state.current = saved_current;
            result?;
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BoundedReadError {
    Io,
    TooLarge,
}

fn read_bounded<R: Read>(reader: R, limit: usize) -> Result<Vec<u8>, BoundedReadError> {
    let byte_limit = limit.checked_add(1).ok_or(BoundedReadError::TooLarge)?;
    let byte_limit = u64::try_from(byte_limit).map_err(|_| BoundedReadError::TooLarge)?;
    let mut reader = reader.take(byte_limit);
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|_| BoundedReadError::Io)?;
    if bytes.len() > limit {
        return Err(BoundedReadError::TooLarge);
    }
    Ok(bytes)
}

fn read_text_file(
    path: &Path,
    line: usize,
    limit: usize,
) -> Result<Option<String>, SshConfigError> {
    let Some(file) = open_regular_file(path) else {
        return Ok(None);
    };
    let bytes = match read_bounded(file, limit) {
        Ok(bytes) => bytes,
        Err(BoundedReadError::Io) => return Ok(None),
        Err(BoundedReadError::TooLarge) => {
            return Err(error(line, SshConfigErrorKind::FileTooLarge));
        }
    };
    decode_file(bytes, line, limit).map(Some)
}

fn open_regular_file(path: &Path) -> Option<File> {
    #[cfg(unix)]
    {
        // Callers pass a path that fs::canonicalize already resolved to a
        // symlink-free in-root location, so legitimate in-root symlinks have
        // collapsed to their regular-file targets before this point. The
        // confinement flag below participates in that same open(2), closing
        // the post-canonicalization final-component race on every Unix target.
        // Apple also rejects symlinks in ancestor components atomically; other
        // Unix targets continue to rely on the canonical root checks for
        // ancestor confinement. See [`open_confinement_flags`] for details.
        //
        // O_NONBLOCK prevents a FIFO or similar special source from stalling
        // before any byte budget can apply. Inspect the opened descriptor, not
        // the path, and read from that same descriptor only when it is a
        // regular file.
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(open_confinement_flags() | libc::O_NONBLOCK)
            .open(path)
            .ok()?;
        file.metadata().ok()?.is_file().then_some(file)
    }

    #[cfg(not(unix))]
    {
        let file = File::open(path).ok()?;
        file.metadata().ok()?.is_file().then_some(file)
    }
}

/// Symlink-confinement flags ORed into the single `open(2)` call. Apple rejects
/// symlinks in any path component atomically. Other Unix targets reject only a
/// final-component symlink and rely on the canonical root checks for ancestors.
#[cfg(unix)]
fn open_confinement_flags() -> libc::c_int {
    // Apple (Noren's supported macOS target): `O_NOFOLLOW_ANY` makes the kernel
    // reject, atomically during one `open(2)`, a symlink in ANY component,
    // including ancestors. This closes the post-canonicalization race where an
    // ancestor directory is swapped for a symlink between `canonicalize()` and
    // `open()`; `O_NOFOLLOW` alone would protect only the final component. It
    // is a strict superset of `O_NOFOLLOW`'s final-component guarantee, so the
    // two flags are not combined here.
    #[cfg(target_vendor = "apple")]
    {
        libc::O_NOFOLLOW_ANY
    }
    // Non-Apple Unix: no portable single-syscall primitive rejects ancestor
    // symlinks atomically, so `O_NOFOLLOW` protects only the final component.
    // Ancestor confinement continues to rely on the caller's canonical root
    // checks (`canonicalize_within`). This keeps compilation valid on these
    // platforms without claiming the unsupported ancestor confinement that the
    // Apple primitive provides.
    #[cfg(not(target_vendor = "apple"))]
    {
        libc::O_NOFOLLOW
    }
}

fn decode_file(bytes: Vec<u8>, line: usize, limit: usize) -> Result<String, SshConfigError> {
    if bytes.len() > limit {
        return Err(error(line, SshConfigErrorKind::FileTooLarge));
    }
    String::from_utf8(bytes).map_err(|_| error(line, SshConfigErrorKind::InvalidUtf8))
}

fn tokenize(
    line: &str,
    line_number: usize,
    token_items: &mut usize,
    token_limit: usize,
) -> Result<Vec<String>, SshConfigError> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    let mut escaped = false;
    let mut token_started = false;
    let mut boundary = true;
    for character in line.chars() {
        if escaped {
            token.push(character);
            escaped = false;
            token_started = true;
            boundary = false;
            continue;
        }
        match character {
            '\\' => {
                escaped = true;
                token_started = true;
            }
            '"' => {
                quoted = !quoted;
                token_started = true;
                boundary = false;
            }
            '#' if !quoted && boundary => break,
            '=' if !quoted => {
                if token_started {
                    charge_token_item(token_items, token_limit, line_number)?;
                    tokens.push(std::mem::take(&mut token));
                }
                token_started = false;
                boundary = true;
            }
            character if character.is_whitespace() && !quoted => {
                if token_started {
                    charge_token_item(token_items, token_limit, line_number)?;
                    tokens.push(std::mem::take(&mut token));
                }
                token_started = false;
                boundary = true;
            }
            character => {
                token.push(character);
                token_started = true;
                boundary = false;
            }
        }
    }
    if escaped || quoted {
        return Err(error(line_number, SshConfigErrorKind::UnterminatedArgument));
    }
    if token_started {
        charge_token_item(token_items, token_limit, line_number)?;
        tokens.push(token);
    }
    Ok(tokens)
}

/// Charge one structural token item immediately before a completed token is
/// pushed into its `Vec`. The counter is shared across the top-level source and
/// every Include occurrence, so it bounds aggregate `Vec`/`String` item growth
/// even when eight MiB of source parses to far more collection entries.
fn charge_token_item(used: &mut usize, limit: usize, line: usize) -> Result<(), SshConfigError> {
    let Some(next) = used.checked_add(1) else {
        return Err(error(
            line,
            SshConfigErrorKind::StructuralComplexityExceeded,
        ));
    };
    if next > limit {
        return Err(error(
            line,
            SshConfigErrorKind::StructuralComplexityExceeded,
        ));
    }
    *used = next;
    Ok(())
}

fn error(line: usize, kind: SshConfigErrorKind) -> SshConfigError {
    SshConfigError { line, kind }
}

fn parse_port(value: &str, line: usize) -> Result<u16, SshConfigError> {
    let port = value
        .parse::<u16>()
        .map_err(|_| error(line, SshConfigErrorKind::InvalidPort))?;
    (port != 0)
        .then_some(port)
        .ok_or_else(|| error(line, SshConfigErrorKind::InvalidPort))
}

fn has_wildcard(pattern: &str) -> bool {
    pattern.contains(['*', '?'])
}

fn wildcard_match(pattern: &str, candidate: &str) -> bool {
    // Greedily consume characters and retry only from the latest star. This
    // keeps matching iterative and non-exponential. The resolver's preflight
    // budget charges the possible pattern-by-candidate polynomial work.
    let pattern: Vec<char> = pattern.chars().collect();
    let candidate: Vec<char> = candidate.chars().collect();
    let mut pattern_index = 0;
    let mut candidate_index = 0;
    let mut star_index = None;
    let mut star_candidate_index = 0;

    while candidate_index < candidate.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == '?'
                || pattern[pattern_index].eq_ignore_ascii_case(&candidate[candidate_index]))
        {
            pattern_index += 1;
            candidate_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == '*' {
            star_index = Some(pattern_index);
            pattern_index += 1;
            star_candidate_index = candidate_index;
        } else if let Some(star_index) = star_index {
            pattern_index = star_index + 1;
            star_candidate_index += 1;
            candidate_index = star_candidate_index;
        } else {
            return false;
        }
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == '*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

fn include_component_matches(pattern: &str, candidate: &str) -> bool {
    // OpenSSH pathname expansion does not let a wildcard implicitly consume a
    // leading dot. Keep this separate from Host matching, where `Host *` must
    // continue to match aliases that begin with `.`.
    (!candidate.starts_with('.') || pattern.starts_with('.')) && wildcard_match(pattern, candidate)
}

fn canonicalize_within(path: &Path, root: &Path) -> Option<PathBuf> {
    let canonical = fs::canonicalize(path).ok()?;
    canonical.starts_with(root).then_some(canonical)
}

fn expand_include(
    pattern: &str,
    root: &Path,
    max_matches: usize,
    budget: &mut IncludeExpansionBudget,
    line: usize,
) -> Result<Vec<PathBuf>, SshConfigError> {
    // Charge the attacker-controlled raw pattern bytes before expand_home,
    // PathBuf construction, the root-relative join, or component scanning can run,
    // so a nearly one-MiB literal Include path cannot be built and scanned for a
    // handful of units. Overflow rejects content-free with
    // IncludeExpansionWorkExceeded.
    budget.charge(pattern.len().saturating_add(1), line)?;
    let pattern = expand_home(pattern).unwrap_or_else(|| PathBuf::from(pattern));
    let path = if pattern.is_absolute() {
        pattern
    } else {
        root.join(pattern)
    };
    let mut wildcard_index = None;
    for (index, component) in path.components().enumerate() {
        budget.charge(1, line)?;
        if matches!(
            component,
            Component::Normal(name) if has_wildcard(&name.to_string_lossy())
        ) {
            wildcard_index = Some(index);
            break;
        }
    }

    let Some(wildcard_index) = wildcard_index else {
        // Charge the upcoming canonicalize and stat before they run, not only
        // after a successful file check.
        budget.charge(1, line)?;
        let Some(canonical) = canonicalize_within(&path, root) else {
            return Ok(Vec::new());
        };
        if !canonical.is_file() {
            return Ok(Vec::new());
        }
        if max_matches == 0 {
            return Err(error(line, SshConfigErrorKind::IncludedFilesExceeded));
        }
        return Ok(vec![canonical]);
    };

    let mut prefix = PathBuf::new();
    for component in path.components().take(wildcard_index) {
        prefix.push(component.as_os_str());
    }
    // Charge the prefix canonicalize before it runs. Canonicalize only for
    // confinement; the frontier keeps the logical matched pathname so that
    // symlink globs sort by matched path, not by canonical target.
    budget.charge(1, line)?;
    let Some(_prefix_canonical) = canonicalize_within(&prefix, root) else {
        return Ok(Vec::new());
    };
    let mut frontier = vec![prefix];

    for component in path.components().skip(wildcard_index) {
        let mut next = Vec::new();
        match component {
            Component::CurDir => {
                for current in frontier {
                    budget.charge(1, line)?;
                    next.push(current);
                }
            }
            Component::ParentDir => {
                // Pushing ".." copies its bytes into every frontier path. Charge
                // the component-by-frontier product before any push so a wide
                // frontier cannot amplify the copies past the bound; overflow
                // rejects instead of bypassing the limit.
                let component_bytes = "..".len();
                let cost = component_bytes
                    .saturating_add(1)
                    .saturating_mul(frontier.len());
                budget.charge(cost, line)?;
                for mut current in frontier {
                    // Canonicalizing the rewritten path is a filesystem step;
                    // charge it before it runs. Validate confinement canonically
                    // but keep the logical path.
                    budget.charge(1, line)?;
                    current.push("..");
                    let Some(_canonical) = canonicalize_within(&current, root) else {
                        continue;
                    };
                    next.push(current);
                }
            }
            Component::Normal(name) if has_wildcard(&name.to_string_lossy()) => {
                let pattern = name.to_string_lossy();
                for current in frontier {
                    let current_bytes = current.as_os_str().len();
                    // Canonicalizing the directory and opening it are repeated
                    // filesystem steps; charge before either runs so a wide
                    // frontier cannot brute-force the filesystem for free. The
                    // directory is canonicalized only for traversal; the matched
                    // name is appended to the logical current below.
                    budget.charge(1, line)?;
                    let Some(directory) = canonicalize_within(&current, root) else {
                        continue;
                    };
                    let Ok(mut directory_entries) = fs::read_dir(&directory) else {
                        continue;
                    };
                    // Stream entries one at a time, charging strictly before each
                    // iterator advance: the obtain unit is paid before every
                    // `next()` call, including the terminal `None` probe that ends
                    // the directory, so no `DirEntry` is ever constructed before a
                    // successful charge and no batch can be collected before the
                    // budget bites. No intermediate vector is collected or sorted
                    // here; the final bounded sort of logical matched paths
                    // preserves lexical Include order.
                    loop {
                        budget.charge(1, line)?;
                        let Some(entry_result) = directory_entries.next() else {
                            break;
                        };
                        let Ok(entry) = entry_result else {
                            continue;
                        };
                        let file_name = entry.file_name();
                        let Some(file_name) = file_name.to_str() else {
                            continue;
                        };
                        // The matcher scales with pattern by candidate bytes.
                        // Charge that product before attempting the match so a
                        // directory of long names cannot do uncounted work; an
                        // over-budget name rejects content-free before matching.
                        let match_cost = pattern
                            .len()
                            .saturating_add(1)
                            .saturating_mul(file_name.len().saturating_add(1));
                        budget.charge(match_cost, line)?;
                        if include_component_matches(&pattern, file_name) {
                            // The logical path copy is `current` plus the matched
                            // name; charge those bytes before constructing the
                            // PathBuf. Overflow rejects, never bypasses.
                            let path_cost = current_bytes
                                .saturating_add(file_name.len())
                                .saturating_add(1);
                            budget.charge(path_cost, line)?;
                            let mut logical = current.clone();
                            logical.push(file_name);
                            next.push(logical);
                        }
                    }
                }
            }
            Component::Normal(name) => {
                // Pushing this literal component copies its bytes into every
                // frontier path. Charge the component-by-frontier product before
                // any push so a wide frontier cannot amplify the copies past the
                // bound; overflow rejects instead of bypassing the limit.
                let component_bytes = name.to_string_lossy().len();
                let cost = component_bytes
                    .saturating_add(1)
                    .saturating_mul(frontier.len());
                budget.charge(cost, line)?;
                for mut current in frontier {
                    current.push(name);
                    next.push(current);
                }
            }
            Component::RootDir | Component::Prefix(_) => {}
        }
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }

    // Collect bounded (logical matched path, canonical target) pairs. The
    // logical path drives ordering so symlink globs follow matched-path order
    // rather than canonical-target order; the canonical target drives confinement
    // and the file check. Reject the over-limit match before parsing anything.
    let mut matches: Vec<(PathBuf, PathBuf)> = Vec::new();
    for candidate in frontier {
        // Canonicalizing and stat-ing each candidate are filesystem steps;
        // charge before they run so a large non-matching frontier cannot do
        // uncounted stat work.
        budget.charge(1, line)?;
        let Some(canonical) = canonicalize_within(&candidate, root) else {
            continue;
        };
        if !canonical.is_file() {
            continue;
        }
        if matches.len() >= max_matches {
            return Err(error(line, SshConfigErrorKind::IncludedFilesExceeded));
        }
        matches.push((candidate, canonical));
    }
    matches.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(matches
        .into_iter()
        .map(|(_, canonical)| canonical)
        .collect())
}

fn home_directory(home: Option<OsString>) -> Option<PathBuf> {
    home.filter(|value| !value.is_empty()).map(PathBuf::from)
}

fn expand_home(pattern: &str) -> Option<PathBuf> {
    let suffix = pattern.strip_prefix("~/")?;
    Some(home_directory(env::var_os("HOME"))?.join(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;
    use std::io::Write as _;

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root = env::temp_dir().join(format!(
                "noren-ssh-config-test-{}-{name}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).expect("create SSH config fixture directory");
            Self { root }
        }

        fn write(&self, relative: &str, contents: &str) -> PathBuf {
            let path = self.root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create SSH config fixture parent");
            }
            let mut file = fs::File::create(&path).expect("create SSH config fixture");
            file.write_all(contents.as_bytes())
                .expect("write SSH config fixture");
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[cfg(unix)]
    const FIFO_HELPER_TEST: &str = "ssh_config::tests::fifo_subprocess_helper";
    #[cfg(unix)]
    const FIFO_HELPER_MODE_ENV: &str = "NOREN_SSH_CONFIG_FIFO_HELPER_MODE";
    #[cfg(unix)]
    const FIFO_HELPER_PATH_ENV: &str = "NOREN_SSH_CONFIG_FIFO_HELPER_PATH";
    #[cfg(unix)]
    const FIFO_HELPER_EXPECTED_ENV: &str = "NOREN_SSH_CONFIG_FIFO_HELPER_EXPECTED";
    #[cfg(unix)]
    const FIFO_HELPER_NONCE_ENV: &str = "NOREN_SSH_CONFIG_FIFO_HELPER_NONCE";
    #[cfg(unix)]
    const FIFO_HELPER_ACK_ENV: &str = "NOREN_SSH_CONFIG_FIFO_HELPER_ACK";
    #[cfg(unix)]
    const FIFO_HELPER_MODE_OPEN_REGULAR_FILE: &str = "open-regular-file";
    #[cfg(unix)]
    const FIFO_HELPER_MODE_READ_CONFIG: &str = "read-config";
    #[cfg(unix)]
    const FIFO_HELPER_EXPECTED_NONE: &str = "none";
    #[cfg(unix)]
    const FIFO_HELPER_EXPECTED_EMPTY: &str = "empty";
    #[cfg(unix)]
    const FIFO_HELPER_EXPECTED_AFTER_FIFOS: &str = "after-fifos";

    #[cfg(unix)]
    fn create_fifo(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create FIFO fixture parent");
        }
        let status = std::process::Command::new("/usr/bin/mkfifo")
            .arg(path)
            .status()
            .expect("run mkfifo for regression fixture");
        assert!(status.success(), "mkfifo failed with {status}");
    }

    #[cfg(unix)]
    fn fifo_helper_nonce() -> String {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("{}-{timestamp}-{sequence}", std::process::id())
    }

    #[cfg(unix)]
    fn fifo_helper_acknowledgement(nonce: &str, mode: &str, path: &Path, expected: &str) -> String {
        format!(
            "noren-fifo-helper-v1\nnonce={nonce:?}\nmode={mode:?}\npath={path:?}\nexpected={expected:?}\n"
        )
    }

    #[cfg(unix)]
    fn remove_fifo_helper_acknowledgement(path: &Path) -> std::io::Result<()> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    #[cfg(unix)]
    fn assert_fifo_helper_completes(
        mode: &str,
        path: &Path,
        expected: &str,
        acknowledgement_path: &Path,
    ) {
        assert!(
            !acknowledgement_path.exists(),
            "FIFO helper acknowledgement must start absent"
        );
        let nonce = fifo_helper_nonce();
        let expected_acknowledgement = fifo_helper_acknowledgement(&nonce, mode, path, expected);
        let mut child = std::process::Command::new(
            env::current_exe().expect("locate the current unit-test executable"),
        )
        .arg("--exact")
        .arg(FIFO_HELPER_TEST)
        .arg("--ignored")
        .arg("--nocapture")
        .env_remove("NOREN_SSH_CONFIG_FIFO_TEST_CHILD")
        .env_remove("NOREN_SSH_CONFIG_FIFO_TEST_PATH")
        .env(FIFO_HELPER_MODE_ENV, mode)
        .env(FIFO_HELPER_PATH_ENV, path)
        .env(FIFO_HELPER_EXPECTED_ENV, expected)
        .env(FIFO_HELPER_NONCE_ENV, &nonce)
        .env(FIFO_HELPER_ACK_ENV, acknowledgement_path)
        .spawn()
        .expect("spawn bounded FIFO regression subprocess");
        let timeout = std::time::Duration::from_secs(2);
        let deadline = std::time::Instant::now() + timeout;

        let status = loop {
            match child.try_wait() {
                Ok(Some(_)) => {
                    break child
                        .wait()
                        .expect("reap completed FIFO regression subprocess");
                }
                Ok(None) if std::time::Instant::now() >= deadline => {
                    let kill_result = child.kill();
                    let reap_result = child.wait();
                    let cleanup_result = remove_fifo_helper_acknowledgement(acknowledgement_path);
                    panic!(
                        "FIFO regression subprocess exceeded {timeout:?}; \
                         kill={kill_result:?}, reap={reap_result:?}, \
                         acknowledgement_cleanup={cleanup_result:?}"
                    );
                }
                Ok(None) => {}
                Err(error) => {
                    let kill_result = child.kill();
                    let reap_result = child.wait();
                    let cleanup_result = remove_fifo_helper_acknowledgement(acknowledgement_path);
                    panic!(
                        "could not poll FIFO regression subprocess: {error}; \
                         kill={kill_result:?}, reap={reap_result:?}, \
                         acknowledgement_cleanup={cleanup_result:?}"
                    );
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };

        let acknowledgement = fs::read_to_string(acknowledgement_path);
        let cleanup_result = remove_fifo_helper_acknowledgement(acknowledgement_path);
        assert!(
            status.success(),
            "FIFO regression subprocess failed with {status}; \
             acknowledgement={acknowledgement:?}, cleanup={cleanup_result:?}"
        );
        cleanup_result.expect("remove FIFO helper acknowledgement");
        let acknowledgement = acknowledgement
            .expect("FIFO helper must acknowledge the exact invoked test and protocol");
        assert_eq!(
            acknowledgement, expected_acknowledgement,
            "FIFO helper acknowledgement must match its nonce and invocation"
        );
        assert!(
            !acknowledgement_path.exists(),
            "FIFO helper acknowledgement must be removed"
        );
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "invoked only by bounded FIFO parent tests"]
    fn fifo_subprocess_helper() {
        let mode = env::var(FIFO_HELPER_MODE_ENV).expect("FIFO helper mode is supplied by parent");
        let path = PathBuf::from(
            env::var_os(FIFO_HELPER_PATH_ENV).expect("FIFO helper path is supplied by parent"),
        );
        let expected = env::var(FIFO_HELPER_EXPECTED_ENV)
            .expect("FIFO helper expected result is supplied by parent");
        let nonce =
            env::var(FIFO_HELPER_NONCE_ENV).expect("FIFO helper nonce is supplied by parent");
        let acknowledgement_path = PathBuf::from(
            env::var_os(FIFO_HELPER_ACK_ENV)
                .expect("FIFO helper acknowledgement path is supplied by parent"),
        );

        match (mode.as_str(), expected.as_str()) {
            (FIFO_HELPER_MODE_OPEN_REGULAR_FILE, FIFO_HELPER_EXPECTED_NONE) => {
                assert!(
                    open_regular_file(&path).is_none(),
                    "FIFO descriptor must be rejected by opened-descriptor metadata"
                );
            }
            (FIFO_HELPER_MODE_READ_CONFIG, FIFO_HELPER_EXPECTED_EMPTY) => {
                let config =
                    SshConfig::read(&path).expect("FIFO source is ignored without an error");
                assert!(config.hosts().is_empty());
            }
            (FIFO_HELPER_MODE_READ_CONFIG, FIFO_HELPER_EXPECTED_AFTER_FIFOS) => {
                let config =
                    SshConfig::read(&path).expect("included FIFO sources are ignored promptly");
                assert_eq!(config.hosts().len(), 1);
                assert_eq!(config.hosts()[0].alias(), "after-fifos");
                assert_eq!(config.hosts()[0].user(), Some("parsed"));
            }
            _ => panic!("unsupported FIFO helper protocol: mode={mode:?}, expected={expected:?}"),
        }

        let acknowledgement = fifo_helper_acknowledgement(&nonce, &mode, &path, &expected);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&acknowledgement_path)
            .expect("create fresh FIFO helper acknowledgement");
        file.write_all(acknowledgement.as_bytes())
            .expect("write FIFO helper acknowledgement");
    }

    fn components_through_first_wildcard(path: &Path) -> usize {
        path.components()
            .position(|component| {
                matches!(
                    component,
                    Component::Normal(name) if has_wildcard(&name.to_string_lossy())
                )
            })
            .map(|index| index + 1)
            .expect("test path contains a wildcard")
    }

    /// Total component count of a path, used for literal (no-wildcard) Include
    /// paths whose component scan runs to the end.
    fn total_components(path: &Path) -> usize {
        path.components().count()
    }

    /// Byte length of the logical frontier directory in these non-symlink test
    /// fixtures. It matches the `current.as_os_str().len()` path-copy base used
    /// by `expand_include`, while keeping exact-work formulas independent of the
    /// temp directory's absolute path length.
    fn canonical_directory_bytes(root: &Path, relative: &str) -> usize {
        fs::canonicalize(root.join(relative))
            .expect("canonical fixture directory exists")
            .as_os_str()
            .len()
    }

    fn parse_blocks_for_test(
        text: &str,
        line_offset: usize,
        source_id: SshSourceId,
        token_items: &mut usize,
        token_limit: usize,
    ) -> Result<Vec<Block>, SshConfigError> {
        let mut state = ParseState::new(
            text.len(),
            ParserLimits {
                token_items: token_limit,
                ..DEFAULT_LIMITS
            },
        );
        state.token_items = *token_items;
        let result = parse_source(
            text,
            line_offset,
            source_id,
            IncludeHandling::Ignore,
            &mut state,
        );
        *token_items = state.token_items;
        result?;
        Ok(state.blocks)
    }

    #[test]
    fn missing_home_has_no_home_directory() {
        assert_eq!(home_directory(None), None);
    }

    #[test]
    fn empty_home_has_no_home_directory() {
        assert_eq!(home_directory(Some(OsString::new())), None);
    }

    #[test]
    fn nonempty_home_resolves_to_its_path() {
        let home = OsString::from("/deterministic/home");

        assert_eq!(
            home_directory(Some(home)),
            Some(PathBuf::from("/deterministic/home"))
        );
    }

    #[test]
    fn realistic_multi_host_config_produces_concrete_hosts() {
        let config = SshConfig::parse(
            r#"
            # Work hosts
            Host web staging
                HostName web.internal.example
                User deploy
                Port 2222

            Host database
                HostName db.internal.example
                User postgres
                Port 5432

            Host *
                User nobody
            "#,
        )
        .expect("realistic config parses");

        assert_eq!(
            config.hosts(),
            &[
                SshHost {
                    alias: "web".to_owned(),
                    host_name: Some("web.internal.example".to_owned()),
                    user: Some("deploy".to_owned()),
                    port: Some(2222),
                    declared_source: SshSourceId(0),
                },
                SshHost {
                    alias: "staging".to_owned(),
                    host_name: Some("web.internal.example".to_owned()),
                    user: Some("deploy".to_owned()),
                    port: Some(2222),
                    declared_source: SshSourceId(0),
                },
                SshHost {
                    alias: "database".to_owned(),
                    host_name: Some("db.internal.example".to_owned()),
                    user: Some("postgres".to_owned()),
                    port: Some(5432),
                    declared_source: SshSourceId(0),
                },
            ]
        );
    }

    #[test]
    fn inline_parse_declares_partial_discovery_and_inline_provenance() {
        let config = SshConfig::parse("Host workstation\n  User deploy\n")
            .expect("inline provenance fixture parses");
        let host = &config.hosts()[0];
        let source = config
            .source(host.declared_source())
            .expect("host source belongs to config");

        assert_eq!(
            config.discovery_kind(),
            HostDiscoveryKind::PartialLiteralPatterns
        );
        assert_eq!(host.declared_source().ordinal(), 0);
        assert_eq!(source.id(), host.declared_source());
        assert_eq!(source.tag(), "#0");
        assert_eq!(source.label(), "inline #0");
    }

    #[test]
    fn source_labels_are_root_relative_escaped_utf8_and_bounded() {
        let root = Path::new("/private/user-home/.ssh");
        let relative = format!("parts/line\n{}-host.conf", "界".repeat(40));
        let source_path = root.join(relative);
        let source = file_source(SshSourceId(7), &source_path, root);

        assert_eq!(source.tag(), "#7");
        assert!(source.label().contains("line\\n"));
        assert!(source.label().contains("\\u{754c}"));
        assert!(source.label().is_ascii());
        assert!(source.label().ends_with(" #7"));
        assert!(source.label().len() <= MAX_SSH_SOURCE_LABEL_BYTES);
        assert!(!source.label().contains("/private/user-home"));
    }

    #[test]
    fn a_source_outside_the_root_never_leaks_its_absolute_path() {
        let source = file_source(
            SshSourceId(3),
            Path::new("/private/secret/config"),
            Path::new("/safe/root"),
        );

        assert_eq!(source.label(), "config #3");
        assert!(!source.label().contains("private"));
    }

    #[test]
    fn first_value_wins_per_keyword_against_later_wildcard_defaults() {
        let config = SshConfig::parse(
            "Host web\n  HostName web-specific\n  User deploy\n  Port 2200\nHost *\n  HostName general\n  User nobody\n  Port 22\n",
        )
        .expect("precedence fixture parses");
        let host = &config.hosts()[0];

        assert_eq!(host.alias(), "web");
        assert_eq!(host.host_name(), Some("web-specific"));
        assert_eq!(host.user(), Some("deploy"));
        assert_eq!(host.port(), Some(2200));
    }

    #[test]
    fn wildcard_question_mark_and_negation_patterns_are_applied() {
        let config = SshConfig::parse(
            "Host app1\n  HostName app-one\nHost app? !app2\n  User wildcard\nHost *.example\n  HostName example-host\nHost app2\n  User specific\nHost api.example\n  User api\n",
        )
        .expect("pattern fixture parses");

        let app1 = config
            .hosts()
            .iter()
            .find(|host| host.alias() == "app1")
            .expect("app1 host");
        assert_eq!(app1.user(), Some("wildcard"));
        let app2 = config
            .hosts()
            .iter()
            .find(|host| host.alias() == "app2")
            .expect("app2 host");
        assert_eq!(app2.user(), Some("specific"));
        let api = config
            .hosts()
            .iter()
            .find(|host| host.alias() == "api.example")
            .expect("api host");
        assert_eq!(api.host_name(), Some("example-host"));
    }

    #[test]
    fn exact_and_wildcard_self_negated_literals_are_not_discovered() {
        for text in [
            "Host foo !foo\n  HostName phantom\nHost *\n  User default\n",
            "Host foo !f*\n  HostName phantom\nHost *\n  User default\n",
        ] {
            let config = SshConfig::parse(text).expect("self-negated config parses");
            assert!(
                config.hosts().is_empty(),
                "a later fallback block must not create a phantom literal host"
            );
        }
    }

    #[test]
    fn later_genuinely_positive_literal_restores_self_negated_alias_discovery() {
        let config = SshConfig::parse(
            "Host foo !FOO\n  HostName phantom\nHost BAR\n  User bar\nHost FOO\n  User real\nHost *\n  HostName default\n",
        )
        .expect("later-positive config parses");

        assert_eq!(config.hosts().len(), 2);
        assert_eq!(config.hosts()[0].alias(), "BAR");
        assert_eq!(config.hosts()[1].alias(), "FOO");
        assert_eq!(config.hosts()[1].host_name(), Some("default"));
        assert_eq!(config.hosts()[1].user(), Some("real"));
    }

    #[test]
    fn negation_only_block_does_not_match_other_hosts() {
        let config = SshConfig::parse(
            "Host blocked other\nHost !blocked\n  User negation-only\nHost *\n  User default\n",
        )
        .expect("negation-only fixture parses");

        assert_eq!(config.hosts().len(), 2);
        assert!(
            config
                .hosts()
                .iter()
                .all(|host| host.user() == Some("default"))
        );
    }

    #[test]
    fn mixed_literal_and_wildcard_blocks_preserve_resolution_order() {
        let config = SshConfig::parse(
            "Host exact\n  HostName exact.example\n  User exact\nHost *.example\n  HostName wildcard.example\n  User wildcard\nHost api.example\n  User api\nHost db.example !api.example\n  Port 2200\nHost *\n  HostName general.example\n  User nobody\n",
        )
        .expect("mixed config parses");

        assert_eq!(
            config.hosts(),
            &[
                SshHost {
                    alias: "exact".to_owned(),
                    host_name: Some("exact.example".to_owned()),
                    user: Some("exact".to_owned()),
                    port: None,
                    declared_source: SshSourceId(0),
                },
                SshHost {
                    alias: "api.example".to_owned(),
                    host_name: Some("wildcard.example".to_owned()),
                    user: Some("wildcard".to_owned()),
                    port: None,
                    declared_source: SshSourceId(0),
                },
                SshHost {
                    alias: "db.example".to_owned(),
                    host_name: Some("wildcard.example".to_owned()),
                    user: Some("wildcard".to_owned()),
                    port: Some(2200),
                    declared_source: SshSourceId(0),
                },
            ]
        );
    }

    #[test]
    fn accepted_mixed_config_preserves_precedence_defaults_negation_and_case() {
        let config = SshConfig::parse(
            "Port 2022\n\
             Host Alpha ALPHA alpha\n\
               HostName alpha-first.example\n\
               User alpha-user\n\
             Host PROD-* !prod-admin\n\
               HostName wildcard.example\n\
               User wildcard-user\n\
             Host prod-Web\n\
               HostName literal-web.example\n\
               User literal-web\n\
             Host PROD-ADMIN\n\
               HostName admin.example\n\
             Host * !SKIP\n\
               HostName default.example\n\
               User default-user\n\
             Host skip\n\
               HostName skip.example\n\
               User skip-user\n\
             Host alpha\n\
               HostName alpha-late.example\n\
               User alpha-late\n",
        )
        .expect("bounded mixed config parses");

        assert_eq!(
            config.hosts(),
            &[
                SshHost {
                    alias: "Alpha".to_owned(),
                    host_name: Some("alpha-first.example".to_owned()),
                    user: Some("alpha-user".to_owned()),
                    port: Some(2022),
                    declared_source: SshSourceId(0),
                },
                SshHost {
                    alias: "prod-Web".to_owned(),
                    host_name: Some("wildcard.example".to_owned()),
                    user: Some("wildcard-user".to_owned()),
                    port: Some(2022),
                    declared_source: SshSourceId(0),
                },
                SshHost {
                    alias: "PROD-ADMIN".to_owned(),
                    host_name: Some("admin.example".to_owned()),
                    user: Some("default-user".to_owned()),
                    port: Some(2022),
                    declared_source: SshSourceId(0),
                },
                SshHost {
                    alias: "skip".to_owned(),
                    host_name: Some("skip.example".to_owned()),
                    user: Some("skip-user".to_owned()),
                    port: Some(2022),
                    declared_source: SshSourceId(0),
                },
            ]
        );
    }

    #[test]
    fn exact_reviewer_indexed_literal_shape_is_complete_with_collapsed_settings() {
        let mut text = String::from("Host");
        for index in 0..60_000 {
            write!(text, " a{index}").expect("write alias to string");
        }
        text.push('\n');
        for _ in 0..58_152 {
            text.push_str("HostName x\n");
        }
        assert_eq!(text.len(), 1_048_567);

        let mut token_items = 0usize;
        let blocks =
            parse_blocks_for_test(&text, 0, SshSourceId(0), &mut token_items, MAX_TOKEN_ITEMS)
                .expect("reviewer shape tokenizes");
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].settings.len(),
            1,
            "later same-block HostName values can never affect first-value-wins"
        );
        assert_eq!(
            blocks[0].settings.first(),
            Some(&Setting::HostName("x".to_owned()))
        );

        let config = SshConfig::parse(&text).expect("bounded indexed shape parses completely");
        assert_eq!(config.hosts().len(), 60_000);
        assert_eq!(config.hosts()[0].alias(), "a0");
        assert_eq!(config.hosts()[0].host_name(), Some("x"));
        assert_eq!(config.hosts()[59_999].alias(), "a59999");
        assert_eq!(config.hosts()[59_999].host_name(), Some("x"));
    }

    #[test]
    fn indexed_resolution_work_bites_at_boundary_and_saturates_on_overflow() {
        let mut token_items = 0usize;
        let blocks = parse_blocks_for_test(
            "Host a\nHostName x\n",
            0,
            SshSourceId(0),
            &mut token_items,
            MAX_TOKEN_ITEMS,
        )
        .expect("indexed fixture parses");

        SshConfig::from_blocks_with_limit(
            &blocks,
            vec![inline_source(SshSourceId(0))],
            ParserLimits {
                resolution_work: 5,
                ..DEFAULT_LIMITS
            },
        )
        .expect("alias lookup plus indexed block, setting visit, and clone byte cost five");
        let error = SshConfig::from_blocks_with_limit(
            &blocks,
            vec![inline_source(SshSourceId(0))],
            ParserLimits {
                resolution_work: 4,
                ..DEFAULT_LIMITS
            },
        )
        .expect_err("every indexed block and setting visit must be charged");
        assert_eq!(
            error.kind(),
            &SshConfigErrorKind::ResolutionComplexityExceeded
        );

        ensure_resolution_work(
            ResolutionWork {
                indexed_block_visits: MAX_RESOLUTION_WORK,
                ..ResolutionWork::default()
            },
            MAX_RESOLUTION_WORK,
        )
        .expect("the exact production boundary is accepted");
        let over = ensure_resolution_work(
            ResolutionWork {
                indexed_block_visits: MAX_RESOLUTION_WORK + 1,
                ..ResolutionWork::default()
            },
            MAX_RESOLUTION_WORK,
        )
        .expect_err("one indexed visit over the production boundary is rejected");
        assert_eq!(
            over.kind(),
            &SshConfigErrorKind::ResolutionComplexityExceeded
        );

        let overflow = ensure_resolution_work(
            ResolutionWork {
                indexed_block_visits: u128::MAX,
                indexed_setting_work: 1,
                ..ResolutionWork::default()
            },
            MAX_RESOLUTION_WORK,
        )
        .expect_err("saturated accounting cannot wrap back under the limit");
        assert_eq!(
            overflow.kind(),
            &SshConfigErrorKind::ResolutionComplexityExceeded
        );
    }

    #[test]
    fn indexed_setting_clone_bytes_are_part_of_resolution_work() {
        let mut text = String::from("Host");
        for index in 0..100 {
            write!(text, " a{index}").expect("write alias to string");
        }
        write!(text, "\nHostName {}\n", "x".repeat(200_000)).expect("write large setting value");
        assert!(text.len() < MAX_FILE_BYTES);

        let error = match SshConfig::parse(&text) {
            Ok(_) => panic!("cloning the large value for every indexed alias is over budget"),
            Err(error) => error,
        };
        assert_eq!(
            error.kind(),
            &SshConfigErrorKind::ResolutionComplexityExceeded
        );
    }

    #[test]
    fn self_negation_discovery_work_is_preflight_bounded() {
        let mut token_items = 0usize;
        let blocks = parse_blocks_for_test(
            "Host abcdef !*\n",
            0,
            SshSourceId(0),
            &mut token_items,
            MAX_TOKEN_ITEMS,
        )
        .expect("discovery fixture parses");

        SshConfig::from_blocks_with_limit(
            &blocks,
            vec![inline_source(SshSourceId(0))],
            ParserLimits {
                resolution_work: 14,
                ..DEFAULT_LIMITS
            },
        )
        .expect("seven alias units times two negative-pattern units fits");
        let error = SshConfig::from_blocks_with_limit(
            &blocks,
            vec![inline_source(SshSourceId(0))],
            ParserLimits {
                resolution_work: 13,
                ..DEFAULT_LIMITS
            },
        )
        .expect_err("discovery matching must be included in the preflight");
        assert_eq!(
            error.kind(),
            &SshConfigErrorKind::ResolutionComplexityExceeded
        );
    }

    #[test]
    fn near_one_mib_mixed_alias_wildcard_product_is_fast_reject() {
        let mut text = String::new();
        for index in 0..14_189 {
            writeln!(text, "Host literal-alias-{index:05}").expect("write to string");
            text.push_str("HostName x\n");
            writeln!(text, "Host impossible-{index:05}*").expect("write to string");
            text.push_str("HostName y\n");
        }
        assert!((900 * 1024..=1024 * 1024).contains(&text.len()));

        let started = std::time::Instant::now();
        let error = SshConfig::parse(&text).expect_err("hostile product must be rejected");
        let elapsed = started.elapsed();

        assert_eq!(error.line(), 0);
        assert_eq!(
            error.kind(),
            &SshConfigErrorKind::ResolutionComplexityExceeded
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "mixed preflight rejection took {elapsed:?}"
        );
        eprintln!("mixed 1 MiB preflight rejection: {elapsed:?}");
    }

    #[test]
    fn mixed_budget_does_not_assume_literal_blocks_prefill_fallback_keyword() {
        let mut text = String::new();
        for index in 0..3_000 {
            writeln!(text, "Host unset-alias-{index:04}").expect("write to string");
            writeln!(text, "Host impossible-{index:04}*").expect("write to string");
            text.push_str("User fallback\n");
        }

        let error = SshConfig::parse(&text)
            .expect_err("unset User must not expose alias by fallback-block work");

        assert_eq!(error.line(), 0);
        assert_eq!(
            error.kind(),
            &SshConfigErrorKind::ResolutionComplexityExceeded
        );
    }

    #[test]
    fn near_one_mib_all_literal_config_remains_fast_and_complete() {
        let mut text = String::new();
        for index in 0..20_000 {
            writeln!(text, "Host literal-alias-{index:05}").expect("write to string");
            text.push_str("HostName target.example\n");
        }
        assert!((900 * 1024..=1024 * 1024).contains(&text.len()));

        let started = std::time::Instant::now();
        let config = SshConfig::parse(&text).expect("large literal config parses");
        let elapsed = started.elapsed();

        assert_eq!(config.hosts().len(), 20_000);
        assert_eq!(config.hosts()[0].alias(), "literal-alias-00000");
        assert_eq!(config.hosts()[19_999].host_name(), Some("target.example"));
        assert!(
            elapsed < std::time::Duration::from_secs(15),
            "one MiB literal config parsing took {elapsed:?}"
        );
        eprintln!("literal 1 MiB parsing: {elapsed:?}");
    }

    #[test]
    fn all_literal_multi_alias_block_uses_the_index_without_cross_scanning() {
        let mut text = String::from("Host");
        for index in 0..8_000 {
            write!(text, " alias-{index:04}").expect("write to string");
        }
        text.push_str("\nHostName shared.example\n");

        let config = SshConfig::parse(&text).expect("multi-alias literal block parses");

        assert_eq!(config.hosts().len(), 8_000);
        assert!(
            config
                .hosts()
                .iter()
                .all(|host| host.host_name() == Some("shared.example"))
        );
    }

    #[test]
    fn thousands_of_literal_aliases_parse_within_a_generous_bound() {
        let mut text = String::new();
        for index in 0..8_000 {
            text.push_str("Host alias-");
            text.push_str(&index.to_string());
            text.push_str("\nHostName target.example\n");
        }
        assert!(text.len() < 1024 * 1024);

        let started = std::time::Instant::now();
        let config = SshConfig::parse(&text).expect("large literal config parses");
        let elapsed = started.elapsed();

        assert_eq!(config.hosts().len(), 8_000);
        assert!(
            elapsed < std::time::Duration::from_secs(15),
            "large literal config parsing took {elapsed:?}"
        );
    }

    #[test]
    fn pathological_nonmatching_wildcard_completes_quickly() {
        let pattern = "*a*a*a*a*a*a*a*a*a*a*b";
        let alias = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let text = format!("Host {alias}\nHostName target\nHost {pattern}\n");
        let started = std::time::Instant::now();

        for _ in 0..3 {
            let config = SshConfig::parse(&text).expect("pathological config parses");
            assert_eq!(config.hosts().len(), 1);
            assert_eq!(config.hosts()[0].host_name(), Some("target"));
        }

        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "pathological wildcard matching took {elapsed:?}"
        );
    }

    #[test]
    fn very_long_host_pattern_parses_without_recursion() {
        let pattern = "a".repeat(300_000);
        let text = format!("Host {pattern}\nHostName long.example\n");

        let config = SshConfig::parse(&text).expect("long Host pattern parses");

        assert_eq!(config.hosts().len(), 1);
        assert_eq!(config.hosts()[0].alias(), pattern);
        assert_eq!(config.hosts()[0].host_name(), Some("long.example"));
    }

    #[test]
    fn empty_quoted_values_are_preserved() {
        let config = SshConfig::parse("Host empty\nHostName \"\"\nUser \"\"\n")
            .expect("empty quoted values parse");

        assert_eq!(config.hosts()[0].host_name(), Some(""));
        assert_eq!(config.hosts()[0].user(), Some(""));
    }

    #[test]
    fn percent_tokens_remain_literal_discovery_metadata() {
        let config = SshConfig::parse("Host tokenized\n  HostName %h-via-%p.example\n  User %r\n")
            .expect("token-bearing discovery facts parse");

        assert_eq!(config.hosts().len(), 1);
        assert_eq!(config.hosts()[0].host_name(), Some("%h-via-%p.example"));
        assert_eq!(config.hosts()[0].user(), Some("%r"));
    }

    #[test]
    fn direct_single_value_directives_reject_surplus_arguments() {
        let secret = "surplus-secret-value";
        for directive in [
            format!("HostName target.example {secret}-hostname"),
            format!("User deploy {secret}-user"),
            format!("Port 2200 {secret}-port"),
        ] {
            let text = format!("# direct source\nHost direct\n{directive}\nHost after\n");
            let error = SshConfig::parse(&text)
                .expect_err("every supported single-value directive rejects a surplus token");

            assert_eq!(error.line(), 3);
            assert_eq!(error.kind(), &SshConfigErrorKind::SurplusArgument);
            assert!(!error.to_string().contains(secret));
            assert!(!format!("{error:?}").contains(secret));
        }
    }

    #[test]
    fn invalid_port_precedes_surplus_argument_like_openssh() {
        let error = SshConfig::parse("Host direct\nPort invalid extra\n")
            .expect_err("the invalid first Port argument is diagnosed first");

        assert_eq!(error.line(), 2);
        assert_eq!(error.kind(), &SshConfigErrorKind::InvalidPort);
    }

    #[test]
    fn direct_and_included_sources_share_supported_directive_semantics() {
        let body = "Host parity\n  HostName %h-via-%p.example\n  User %r\n  Port 2200\n";
        let direct = SshConfig::parse(body).expect("direct source parses");

        let fixture = Fixture::new("shared-directive-core");
        let config_path = fixture.write("config", "Include parts/body.conf\n");
        fixture.write("parts/body.conf", body);
        let included = SshConfig::read(&config_path).expect("included source parses");

        assert_eq!(direct.hosts().len(), 1);
        assert_eq!(included.hosts().len(), 1);
        let direct = &direct.hosts()[0];
        let included = &included.hosts()[0];
        assert_eq!(direct.alias(), included.alias());
        assert_eq!(direct.host_name(), included.host_name());
        assert_eq!(direct.user(), included.user());
        assert_eq!(direct.port(), included.port());
        assert_eq!(included.declared_source().ordinal(), 1);
    }

    #[test]
    fn included_single_value_directives_reject_surplus_arguments() {
        let fixture = Fixture::new("included-surplus-argument");
        let config_path = fixture.write("config", "Include parts/body.conf\n");
        let secret = "included-surplus-secret";

        for directive in [
            format!("HostName target.example {secret}-hostname"),
            format!("User deploy {secret}-user"),
            format!("Port 2200 {secret}-port"),
        ] {
            fixture.write(
                "parts/body.conf",
                &format!("Host included\n{directive}\nHost after\n"),
            );
            let error = SshConfig::read(&config_path)
                .expect_err("included directives use the same strict single-value parsing core");

            assert_eq!(error.line(), 2);
            assert_eq!(error.kind(), &SshConfigErrorKind::SurplusArgument);
            assert!(!error.to_string().contains(secret));
            assert!(!format!("{error:?}").contains(secret));
        }
    }

    #[test]
    fn inline_include_stays_opaque_while_file_include_is_validated() {
        let inline = SshConfig::parse("Include\nHost inline\n  User direct\n")
            .expect("inline parsing keeps Include opaque");
        assert_eq!(inline.hosts().len(), 1);
        assert_eq!(inline.hosts()[0].alias(), "inline");

        let fixture = Fixture::new("explicit-include-policy");
        let config_path = fixture.write("config", "Include\nHost file-backed\n");
        let error = SshConfig::read(&config_path)
            .expect_err("file parsing explicitly validates Include arguments");
        assert_eq!(error.line(), 1);
        assert_eq!(error.kind(), &SshConfigErrorKind::MissingArgument);
    }

    #[test]
    fn relative_includes_are_resolved_from_root_and_cycles_stop() {
        let fixture = Fixture::new("include-cycle");
        let config_path = fixture.write("config", "Include parts/one.conf\n");
        // The nested bare path must select root-level two.conf; parts/two.conf
        // is a same-name decoy that exposes including-file-relative behavior.
        fixture.write(
            "parts/one.conf",
            "Host included\n  HostName included.example\nInclude two.conf\n",
        );
        fixture.write(
            "two.conf",
            "Host second\n  User included-user\nInclude parts/one.conf\n",
        );
        fixture.write(
            "parts/two.conf",
            "Host decoy\n  HostName must-not-load.example\n",
        );

        // The cap admits the two real files and the cycle back-edge only. If
        // active-stack cycle detection misses the back-edge, parsing fails.
        let limits = ParserLimits {
            included_files: 3,
            ..DEFAULT_LIMITS
        };
        let config = SshConfig::read_with_limits(&config_path, limits)
            .expect("root-relative include fixture parses and stops its cycle");
        assert_eq!(config.hosts().len(), 2);
        assert_eq!(config.hosts()[0].alias(), "included");
        assert_eq!(config.hosts()[0].host_name(), Some("included.example"));
        assert_eq!(config.hosts()[1].alias(), "second");
        assert_eq!(config.hosts()[1].user(), Some("included-user"));
    }

    #[test]
    fn repeated_include_reuses_source_id_and_first_declaration_provenance() {
        let fixture = Fixture::new("include-provenance");
        let config_path = fixture.write(
            "config",
            "Include parts/hosts.conf\n\
             Host inherited\n\
               User parent-value\n\
             Include parts/hosts.conf\n\
             Host root-only\n",
        );
        fixture.write("parts/hosts.conf", "Host Inherited\n");

        let config = SshConfig::read(&config_path).expect("provenance fixture parses");
        assert_eq!(
            config.sources.len(),
            2,
            "the repeated file is interned once"
        );

        let inherited = &config.hosts()[0];
        assert_eq!(inherited.alias(), "Inherited");
        assert_eq!(inherited.user(), Some("parent-value"));
        assert_eq!(inherited.declared_source().ordinal(), 1);
        assert_eq!(
            config
                .source(inherited.declared_source())
                .expect("included source is retained")
                .label(),
            "parts/hosts.conf #1"
        );

        let root = config
            .hosts()
            .iter()
            .find(|host| host.alias() == "root-only")
            .expect("root host is discovered");
        assert_eq!(root.declared_source().ordinal(), 0);
        assert_eq!(
            config
                .source(root.declared_source())
                .expect("top-level source is retained")
                .label(),
            "config #0"
        );
    }

    #[test]
    fn glob_includes_apply_in_lexical_order() {
        let fixture = Fixture::new("include-lexical-order");
        let config_path = fixture.write("config", "Include parts/*.conf\n");
        fixture.write(
            "parts/z-last.conf",
            "Host ordered\n  HostName last.example\n",
        );
        fixture.write(
            "parts/a-first.conf",
            "Host ordered\n  HostName first.example\n",
        );

        let config = SshConfig::read(&config_path).expect("lexical include fixture parses");
        assert_eq!(config.hosts().len(), 1);
        assert_eq!(config.hosts()[0].alias(), "ordered");
        assert_eq!(config.hosts()[0].host_name(), Some("first.example"));
    }

    #[test]
    fn include_wildcard_ignores_dotfiles_and_keeps_visible_lexical_order() {
        let fixture = Fixture::new("include-dotfile");
        let config_path = fixture.write("config", "Include parts/*.conf\n");
        fixture.write(
            "parts/.secret.conf",
            "Host secret\n  HostName must-not-load.example\n",
        );
        fixture.write(
            "parts/z-last.conf",
            "Host ordered\n  HostName last.example\n",
        );
        fixture.write(
            "parts/a-first.conf",
            "Host ordered\n  HostName first.example\n",
        );

        let config = SshConfig::read(&config_path).expect("visible Include glob parses");
        assert_eq!(config.hosts().len(), 1);
        assert_eq!(config.hosts()[0].alias(), "ordered");
        assert_eq!(config.hosts()[0].host_name(), Some("first.example"));
    }

    #[test]
    fn include_wildcard_does_not_traverse_an_implicit_hidden_directory() {
        let fixture = Fixture::new("include-hidden-directory");
        let config_path = fixture.write("config", "Include parts/*/*.conf\n");
        fixture.write(
            "parts/.hidden/secret.conf",
            "Host secret\n  HostName must-not-load.example\n",
        );
        fixture.write(
            "parts/visible/public.conf",
            "Host public\n  HostName public.example\n",
        );

        let config = SshConfig::read(&config_path).expect("nested visible Include glob parses");
        assert_eq!(config.hosts().len(), 1);
        assert_eq!(config.hosts()[0].alias(), "public");
        assert_eq!(config.hosts()[0].host_name(), Some("public.example"));
    }

    #[test]
    fn explicitly_dot_prefixed_include_components_match_hidden_paths() {
        let fixture = Fixture::new("include-explicit-dot");
        let config_path = fixture.write(
            "config",
            "Include parts/.secret*.conf parts/.hidden*/*.conf\n",
        );
        fixture.write(
            "parts/.secret-one.conf",
            "Host hidden-file\n  HostName hidden-file.example\n",
        );
        fixture.write(
            "parts/.hidden-dir/nested.conf",
            "Host hidden-directory\n  HostName hidden-directory.example\n",
        );

        let config = SshConfig::read(&config_path).expect("explicit hidden Includes parse");
        let aliases: Vec<&str> = config.hosts().iter().map(SshHost::alias).collect();
        assert_eq!(aliases, vec!["hidden-file", "hidden-directory"]);
    }

    #[test]
    fn host_wildcard_still_matches_a_dot_prefixed_alias() {
        let config = SshConfig::parse(
            "Host .dot-alias\n  HostName dot.example\nHost *\n  User wildcard-user\n",
        )
        .expect("Host wildcard fixture parses");

        assert_eq!(config.hosts().len(), 1);
        assert_eq!(config.hosts()[0].alias(), ".dot-alias");
        assert_eq!(config.hosts()[0].user(), Some("wildcard-user"));
    }

    #[test]
    fn missing_included_file_remains_ignored() {
        let fixture = Fixture::new("missing-include");
        let config_path = fixture.write(
            "config",
            "Include parts/does-not-exist.conf\nHost local\n  User deploy\n",
        );

        let config = SshConfig::read(&config_path).expect("missing include is ignored");
        assert_eq!(config.hosts().len(), 1);
        assert_eq!(config.hosts()[0].alias(), "local");
        assert_eq!(config.hosts()[0].user(), Some("deploy"));
    }

    #[test]
    fn include_match_limit_accepts_boundary_then_fails_before_partial_parse() {
        let fixture = Fixture::new("include-match-limit");
        let config_path = fixture.write("config", "Include parts/*.conf\n");
        for index in 0..MAX_INCLUDED_FILES {
            fixture.write(&format!("parts/{index:03}.conf"), "");
        }

        let config = SshConfig::read(&config_path).expect("exact include-file boundary works");
        assert!(config.hosts().is_empty());

        fixture.write("parts/000.conf", "Port must-not-be-parsed\n");
        fixture.write(&format!("parts/{MAX_INCLUDED_FILES:03}.conf"), "");
        let error = SshConfig::read(&config_path)
            .expect_err("the over-limit expansion fails before parsing its first match");
        assert_eq!(error.line(), 1);
        assert_eq!(error.kind(), &SshConfigErrorKind::IncludedFilesExceeded);
        assert!(!error.to_string().contains("must-not-be-parsed"));
        assert!(!format!("{error:?}").contains("must-not-be-parsed"));
    }

    #[test]
    fn directory_enumeration_is_charged_before_unbounded_collection() {
        let fixture = Fixture::new("include-directory-work");
        let config_path = fixture.write("config", "Include parts/no-match-*.conf\n");
        for index in 0..4 {
            fixture.write(&format!("parts/unrelated-{index}.txt"), "");
        }
        let canonical_root = fs::canonicalize(&fixture.root).expect("canonical fixture root");
        let expanded_path = canonical_root.join("parts/no-match-*.conf");
        // (raw pattern bytes + 1) initial charge + component scan + 1 prefix
        // canonicalize + 1 directory canonicalize/open, then (entries + 1)
        // obtain units -- one per streamed entry plus the terminal None probe
        // that ends the directory -- and a byte-weighted `(pattern+1)*(name+1)`
        // match charge per entry. Entries are streamed one at a time under the
        // budget, with no intermediate collection or sort.
        let include_pattern = "parts/no-match-*.conf";
        let pattern_segment = "no-match-*.conf";
        let file_name = "unrelated-0.txt";
        let entries = 4;
        let match_cost = (pattern_segment.len() + 1) * (file_name.len() + 1);
        let required_work = include_pattern.len().saturating_add(1)
            + components_through_first_wildcard(&expanded_path)
            + 1
            + 1
            + (entries + 1)
            + entries * match_cost;
        let limits = ParserLimits {
            include_expansion_work: required_work - 1,
            ..DEFAULT_LIMITS
        };

        let error = SshConfig::read_with_limits(&config_path, limits)
            .expect_err("the terminal iterator probe crosses the one-unit-short work boundary");
        assert_eq!(error.line(), 1);
        assert_eq!(
            error.kind(),
            &SshConfigErrorKind::IncludeExpansionWorkExceeded
        );
    }

    #[test]
    fn include_expansion_work_is_global_across_directives() {
        let fixture = Fixture::new("include-global-work");
        let config_path = fixture.write("config", "Include parts/a*.conf\nInclude parts/b*.conf\n");
        fixture.write("parts/a.conf", "");
        fixture.write("parts/b.conf", "");
        let canonical_root = fs::canonicalize(&fixture.root).expect("canonical fixture root");
        let first_path = canonical_root.join("parts/a*.conf");
        // (raw pattern bytes + 1) initial charge + component scan + 1 prefix
        // canonicalize + 1 directory canonicalize/open, then (considered + 1)
        // obtain units -- one per streamed entry plus the terminal None probe --
        // plus a byte-weighted match charge per entry, plus a byte-weighted
        // path-copy for the single match, plus 1 final-candidate canonicalize
        // unit.
        let include_pattern = "parts/a*.conf";
        let pattern_segment = "a*.conf";
        let candidate = "a.conf";
        let considered = 2;
        let match_cost = (pattern_segment.len() + 1) * (candidate.len() + 1);
        let dir_bytes = canonical_directory_bytes(&canonical_root, "parts");
        let path_copy = dir_bytes + candidate.len() + 1;
        let one_pattern_work = include_pattern.len().saturating_add(1)
            + components_through_first_wildcard(&first_path)
            + 1
            + 1
            + (considered + 1)
            + considered * match_cost
            + path_copy
            + 1;
        let limits = ParserLimits {
            include_expansion_work: one_pattern_work,
            ..DEFAULT_LIMITS
        };

        let error = SshConfig::read_with_limits(&config_path, limits)
            .expect_err("the second directive must share the first directive's budget");
        assert_eq!(error.line(), 2);
        assert_eq!(
            error.kind(),
            &SshConfigErrorKind::IncludeExpansionWorkExceeded
        );
    }

    #[test]
    fn nested_branching_glob_paths_are_bounded_during_expansion() {
        let fixture = Fixture::new("include-branching-work");
        let config_path = fixture.write("config", "Include parts/*/*.conf\n");
        for directory in ["one", "two"] {
            for file in ["a.conf", "b.conf"] {
                fixture.write(&format!("parts/{directory}/{file}"), "");
            }
        }
        let canonical_root = fs::canonicalize(&fixture.root).expect("canonical fixture root");
        let expanded_path = canonical_root.join("parts/*/*.conf");
        // (raw pattern bytes + 1) initial charge + component scan + 1 prefix
        // canonicalize. Each wildcard level charges 1 directory canonicalize/open
        // per frontier directory, then (entries + dirs) obtain units -- one per
        // streamed entry plus a terminal None probe per directory -- plus a
        // byte-weighted match charge per entry, plus a byte-weighted path-copy
        // per match. Finally each final candidate costs 1 canonicalize unit.
        let include_pattern = "parts/*/*.conf";
        let first_pattern = "*";
        let dir_name = "one";
        let first_entries = 2; // one, two (1 directory enumerated)
        let first_match_cost = (first_pattern.len() + 1) * (dir_name.len() + 1);
        let dir_bytes_l1 = canonical_directory_bytes(&canonical_root, "parts");
        let path_copy_l1 = dir_bytes_l1 + dir_name.len() + 1;

        let second_pattern = "*.conf";
        let file_name = "a.conf";
        let second_dirs = first_entries; // 2 directories enumerated at level 2
        let second_entries = 4; // 2 directories x 2 files
        let second_match_cost = (second_pattern.len() + 1) * (file_name.len() + 1);
        let dir_bytes_l2 = canonical_directory_bytes(&canonical_root, "parts/one");
        let path_copy_l2 = dir_bytes_l2 + file_name.len() + 1;

        let required_work = include_pattern.len().saturating_add(1)
            + components_through_first_wildcard(&expanded_path)
            + 1
            + 1
            + (first_entries + 1)
            + first_entries * first_match_cost
            + first_entries * path_copy_l1
            + second_dirs
            + (second_entries + second_dirs)
            + second_entries * second_match_cost
            + second_entries * path_copy_l2
            + 4;
        let limits = ParserLimits {
            include_expansion_work: required_work - 1,
            ..DEFAULT_LIMITS
        };

        let error = SshConfig::read_with_limits(&config_path, limits)
            .expect_err("the fourth final candidate's canonicalize charge crosses the budget");
        assert_eq!(error.line(), 1);
        assert_eq!(
            error.kind(),
            &SshConfigErrorKind::IncludeExpansionWorkExceeded
        );
    }

    #[test]
    fn include_match_charge_scales_with_pattern_and_candidate_bytes() {
        // The wildcard match is charged `(pattern_bytes + 1) * (candidate_bytes
        // + 1)` before it is attempted, so a directory of long non-matching
        // names cannot force uncounted matcher work. The budget below is one
        // unit short of the byte-weighted total, so the parse must reject. If
        // the charge were weakened to a flat `1` per candidate the same parse
        // would succeed; this test guards against that mutation.
        let fixture = Fixture::new("include-match-byte-charge");
        let pattern_segment = "x*.conf";
        let config_path = fixture.write("config", &format!("Include parts/{pattern_segment}\n"));
        let copies = 4;
        for index in 0..copies {
            fixture.write(&format!("parts/y00{index}.conf"), "");
        }
        let canonical_root = fs::canonicalize(&fixture.root).expect("canonical fixture root");
        let expanded_path = canonical_root.join(format!("parts/{pattern_segment}"));
        let candidate = "y000.conf";
        // (raw pattern bytes + 1) initial charge + component scan + 1 prefix
        // canonicalize + 1 directory canonicalize/open, then (copies + 1)
        // obtain units -- one per streamed entry plus the terminal None probe --
        // plus the byte-weighted match charge per entry.
        let include_pattern = format!("parts/{pattern_segment}");
        let match_cost = (pattern_segment.len() + 1) * (candidate.len() + 1);
        let required_work = include_pattern.len().saturating_add(1)
            + components_through_first_wildcard(&expanded_path)
            + 1
            + 1
            + (copies + 1)
            + copies * match_cost;
        let limits = ParserLimits {
            include_expansion_work: required_work - 1,
            ..DEFAULT_LIMITS
        };

        let error = SshConfig::read_with_limits(&config_path, limits).expect_err(
            "the byte-weighted match work plus terminal probe crosses the one-unit-short boundary",
        );
        assert_eq!(error.line(), 1);
        assert_eq!(
            error.kind(),
            &SshConfigErrorKind::IncludeExpansionWorkExceeded
        );
    }

    #[test]
    fn include_literal_push_charge_scales_with_component_and_frontier() {
        // Appending a literal component to every frontier path is charged
        // `(component_bytes + 1) * frontier_width` before any push, so a wide
        // frontier cannot amplify the copy. The budget below is one unit short
        // of the work up to and including that push, so the parse rejects at the
        // literal push and the final non-existent candidates are never reached.
        // If the charge were weakened to a flat `1` or to `frontier_width` alone
        // the same parse would succeed; this test guards against both mutations.
        let fixture = Fixture::new("include-literal-push-charge");
        let leaf = "missing-leaf-name";
        let config_path = fixture.write("config", &format!("Include parts/*/{leaf}\n"));
        let copies = 4;
        for index in 0..copies {
            fixture.write(&format!("parts/d{index}/.keep"), "");
        }
        let canonical_root = fs::canonicalize(&fixture.root).expect("canonical fixture root");
        let expanded_path = canonical_root.join(format!("parts/*/{leaf}"));
        let include_pattern = format!("parts/*/{leaf}");
        let wildcard_pattern = "*";
        let dir_name = "d0";
        let match_cost = (wildcard_pattern.len() + 1) * (dir_name.len() + 1);
        let dir_bytes = canonical_directory_bytes(&canonical_root, "parts");
        let path_copy = dir_bytes + dir_name.len() + 1;
        let push_work = (leaf.len() + 1) * copies;
        let required_work = include_pattern.len().saturating_add(1)
            + components_through_first_wildcard(&expanded_path)
            + 1
            + 1
            + (copies + 1)
            + copies * match_cost
            + copies * path_copy
            + push_work;
        let limits = ParserLimits {
            include_expansion_work: required_work - 1,
            ..DEFAULT_LIMITS
        };

        let error = SshConfig::read_with_limits(&config_path, limits)
            .expect_err("the byte-weighted literal push must reject the wide frontier");
        assert_eq!(error.line(), 1);
        assert_eq!(
            error.kind(),
            &SshConfigErrorKind::IncludeExpansionWorkExceeded
        );
    }

    #[test]
    fn deep_wildcard_path_expands_iteratively_without_recursion() {
        // expand_include loops over path components without ever calling
        // itself, so a deep Include path cannot grow the call stack. A wildcard
        // followed by many literal components is resolved entirely by that
        // iterative loop; this test exercises it at depth and asserts a clean
        // result with no panic.
        let fixture = Fixture::new("include-deep-iterative-path");
        let depth = 32;
        let mut nested = fixture.root.join("parts").join("branch");
        let mut literal_path = String::new();
        for level in 0..depth {
            let segment = format!("lvl{level:02}");
            nested.push(&segment);
            literal_path.push_str(&segment);
            literal_path.push('/');
        }
        let leaf = "target.conf";
        fs::create_dir_all(&nested).expect("create deep directory tree");
        fs::write(
            nested.join(leaf),
            "Host deep-target\n  HostName deep.example\n",
        )
        .expect("write deep leaf config");
        let config_path =
            fixture.write("config", &format!("Include parts/*/{literal_path}{leaf}\n"));

        let config = SshConfig::read(&config_path).expect("deep iterative include parses");
        assert_eq!(config.hosts().len(), 1);
        assert_eq!(config.hosts()[0].alias(), "deep-target");
        assert_eq!(config.hosts()[0].host_name(), Some("deep.example"));
    }

    #[test]
    fn logical_path_copy_is_charged_before_allocation() {
        // Cloning the logical frontier path and appending the matched name
        // allocates a PathBuf. That copy is charged `(current_bytes +
        // name_bytes + 1)` before the allocation. The budget below is one unit
        // short of that charge plus all preceding work, so the parse rejects at
        // the path copy; if the charge were removed the same parse would
        // succeed. This test guards against that mutation.
        let fixture = Fixture::new("include-path-copy-charge");
        let pattern_segment = "a*.conf";
        let config_path = fixture.write("config", &format!("Include parts/{pattern_segment}\n"));
        let candidate = "a.conf";
        fixture.write(&format!("parts/{candidate}"), "");
        let canonical_root = fs::canonicalize(&fixture.root).expect("canonical fixture root");
        let expanded_path = canonical_root.join(format!("parts/{pattern_segment}"));
        let include_pattern = format!("parts/{pattern_segment}");
        let match_cost = (pattern_segment.len() + 1) * (candidate.len() + 1);
        let dir_bytes = canonical_directory_bytes(&canonical_root, "parts");
        let path_copy = dir_bytes + candidate.len() + 1;
        // (raw pattern bytes + 1) initial charge + component scan + prefix
        // canonicalize (1) + directory canonicalize/open (1) + entry obtain (1)
        // + byte-weighted match.
        let before_path_copy = include_pattern.len().saturating_add(1)
            + components_through_first_wildcard(&expanded_path)
            + 1
            + 1
            + 1
            + match_cost;
        let limits = ParserLimits {
            include_expansion_work: before_path_copy + path_copy - 1,
            ..DEFAULT_LIMITS
        };

        let error = SshConfig::read_with_limits(&config_path, limits)
            .expect_err("the path-copy charge must reject before the logical path allocates");
        assert_eq!(error.line(), 1);
        assert_eq!(
            error.kind(),
            &SshConfigErrorKind::IncludeExpansionWorkExceeded
        );
    }

    #[test]
    fn final_candidates_are_charged_before_their_file_check() {
        // Each frontier candidate's canonicalize and stat are charged before
        // they run, so a frontier of non-files cannot do uncounted stat work.
        // The wildcard below matches four directories (none are files); the
        // budget is one unit short of the fourth candidate's pre-canonicalize
        // charge, so the parse rejects there. If the charge were moved back to
        // after the file check, zero final charges would occur (no matches) and
        // the parse would succeed; this test guards against that mutation.
        let fixture = Fixture::new("include-final-canonicalize-charge");
        let config_path = fixture.write("config", "Include parts/*\n");
        let copies = 4;
        for index in 0..copies {
            fixture.write(&format!("parts/d{index}/.keep"), "");
        }
        let canonical_root = fs::canonicalize(&fixture.root).expect("canonical fixture root");
        let expanded_path = canonical_root.join("parts/*");
        let include_pattern = "parts/*";
        let wildcard_pattern = "*";
        let dir_name = "d0";
        let match_cost = (wildcard_pattern.len() + 1) * (dir_name.len() + 1);
        let dir_bytes = canonical_directory_bytes(&canonical_root, "parts");
        let path_copy = dir_bytes + dir_name.len() + 1;
        let required_work = include_pattern.len().saturating_add(1)
            + components_through_first_wildcard(&expanded_path)
            + 1
            + 1
            + (copies + 1)
            + copies * match_cost
            + copies * path_copy
            + copies;
        let limits = ParserLimits {
            include_expansion_work: required_work - 1,
            ..DEFAULT_LIMITS
        };

        let error = SshConfig::read_with_limits(&config_path, limits)
            .expect_err("the fourth candidate's pre-canonicalize charge must reject");
        assert_eq!(error.line(), 1);
        assert_eq!(
            error.kind(),
            &SshConfigErrorKind::IncludeExpansionWorkExceeded
        );
    }

    #[test]
    fn many_long_nonmatches_under_a_small_budget_reject_during_streaming() {
        // A directory of many long non-matching names must reject under a small
        // budget while the iterator is still streaming: each entry is obtained,
        // byte-weighted-matched, and (on no match) discarded before the next is
        // pulled, with no intermediate collection or sort. The shrunk work bound
        // allows only a handful of these long entries to be touched out of the
        // thousand present. (The per-entry streaming accounting itself -- one
        // obtain unit plus the byte-weighted match -- is pinned by the exact-
        // work formulas in the sibling tests above.)
        let fixture = Fixture::new("include-streaming-load");
        let pattern_segment = "x*.conf";
        let config_path = fixture.write("config", &format!("Include parts/{pattern_segment}\n"));
        let copies = 1_000;
        let stem = format!("{}-payload", "y".repeat(40));
        for index in 0..copies {
            fixture.write(&format!("parts/{stem}-{index:04}"), "");
        }
        let limits = ParserLimits {
            include_expansion_work: 2_000,
            ..DEFAULT_LIMITS
        };

        let error = SshConfig::read_with_limits(&config_path, limits)
            .expect_err("streaming iteration must reject long non-matches under the small budget");
        assert_eq!(error.line(), 1);
        assert_eq!(
            error.kind(),
            &SshConfigErrorKind::IncludeExpansionWorkExceeded
        );
    }

    #[test]
    fn initial_raw_pattern_charge_has_exact_boundary_and_one_under() {
        // The very first charge in expand_include is the attacker-controlled
        // raw pattern bytes (+1), paid before expand_home, PathBuf construction,
        // the relative join, or component scanning. A literal (no-wildcard)
        // missing Include path charges that initial unit, then one unit per
        // scanned component, then one unit for the canonicalize/stat; the path
        // does not exist so the routine yields no matches.
        let fixture = Fixture::new("include-initial-charge");
        let include_pattern = "a/literal/missing/include/path.conf";
        let config_path = fixture.write("config", &format!("Include {include_pattern}\n"));
        let canonical_root = fs::canonicalize(&fixture.root).expect("canonical fixture root");
        let expanded_path = canonical_root.join(include_pattern);
        let initial = include_pattern.len().saturating_add(1);
        let scan = total_components(&expanded_path);
        let required_work = initial + scan + 1;

        // Exact boundary: every unit of the literal-path work fits, so the
        // missing path is silently ignored and the parse succeeds.
        let boundary = ParserLimits {
            include_expansion_work: required_work,
            ..DEFAULT_LIMITS
        };
        let config = SshConfig::read_with_limits(&config_path, boundary)
            .expect("exact initial-charge boundary parses");
        assert!(config.hosts().is_empty());

        // One unit under: the final canonicalize/stat charge crosses the budget
        // and rejects content-free. If the initial charge were weakened to a flat
        // 1 the actual work would be lower and this parse would succeed.
        let one_under = ParserLimits {
            include_expansion_work: required_work - 1,
            ..DEFAULT_LIMITS
        };
        let error = SshConfig::read_with_limits(&config_path, one_under)
            .expect_err("one unit under the initial-charge boundary rejects");
        assert_eq!(error.line(), 1);
        assert_eq!(
            error.kind(),
            &SshConfigErrorKind::IncludeExpansionWorkExceeded
        );
    }

    #[test]
    fn nearly_one_mib_literal_include_path_rejects_under_production_budget() {
        // A literal (no-wildcard) Include path sized so the top-level config is
        // exactly MAX_FILE_BYTES. Its raw pattern bytes alone (~1 MiB) far
        // exceed the 64 KiB Include-expansion work budget, so it rejects before
        // any path construction or canonicalization, with safe error text.
        let fixture = Fixture::new("include-huge-literal");
        let secret = "distinctive-secret-fragment";
        let mut pattern = String::from("missing/");
        pattern.push_str(secret);
        let padding = MAX_FILE_BYTES - "Include ".len() - "\n".len() - pattern.len();
        pattern.push_str(&"x".repeat(padding));
        assert_eq!(
            "Include ".len() + pattern.len() + "\n".len(),
            MAX_FILE_BYTES
        );
        let config_path = fixture.write("config", &format!("Include {pattern}\n"));

        let started = std::time::Instant::now();
        let error = SshConfig::read(&config_path)
            .expect_err("a ~1 MiB literal Include path rejects under the work budget");
        let elapsed = started.elapsed();

        assert_eq!(error.line(), 1);
        assert_eq!(
            error.kind(),
            &SshConfigErrorKind::IncludeExpansionWorkExceeded
        );
        assert!(!error.to_string().contains(secret));
        assert!(!format!("{error:?}").contains(secret));
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "~1 MiB literal Include rejection took {elapsed:?}"
        );
        eprintln!("~1 MiB literal Include rejection: {elapsed:?}");
    }

    #[test]
    fn cumulative_source_bytes_accept_boundary_and_reject_one_over() {
        let fixture = Fixture::new("include-total-bytes");
        let top = "Include parts/a.conf\nInclude parts/b.conf\n";
        let first = "Host alpha\n  HostName alpha.example\n";
        let second = "Host beta\n  User deploy\n";
        let config_path = fixture.write("config", top);
        fixture.write("parts/a.conf", first);
        fixture.write("parts/b.conf", second);
        let exact_total = top.len() + first.len() + second.len();

        let exact_limits = ParserLimits {
            total_bytes: exact_total,
            ..DEFAULT_LIMITS
        };
        let config = SshConfig::read_with_limits(&config_path, exact_limits)
            .expect("exact aggregate byte boundary parses");
        assert_eq!(config.hosts().len(), 2);

        let over_limits = ParserLimits {
            total_bytes: exact_total - 1,
            ..DEFAULT_LIMITS
        };
        let error = SshConfig::read_with_limits(&config_path, over_limits)
            .expect_err("one aggregate byte over the boundary is rejected");
        assert_eq!(error.line(), 2);
        assert_eq!(error.kind(), &SshConfigErrorKind::TotalBytesExceeded);
    }

    #[test]
    fn include_assembled_alias_wildcard_product_is_rejected_by_same_preflight() {
        let fixture = Fixture::new("include-complexity");
        let config_path = fixture.write(
            "config",
            "Include parts/aliases.conf\nInclude parts/wildcards.conf\n",
        );
        let mut aliases = String::new();
        let mut wildcards = String::new();
        for index in 0..1_000 {
            writeln!(aliases, "Host included-alias-{index:04}").expect("write to string");
            writeln!(wildcards, "Host no-include-match-{index:04}*").expect("write to string");
            wildcards.push_str("User included-fallback\n");
        }
        fixture.write("parts/aliases.conf", &aliases);
        fixture.write("parts/wildcards.conf", &wildcards);

        let error = SshConfig::read(&config_path)
            .expect_err("included hostile product must use the same complexity budget");

        assert_eq!(error.line(), 0);
        assert_eq!(
            error.kind(),
            &SshConfigErrorKind::ResolutionComplexityExceeded
        );
        assert!(!error.to_string().contains("included-alias"));
        assert!(!format!("{error:?}").contains("included-alias"));
    }

    #[test]
    fn direct_self_include_terminates_instead_of_hanging() {
        let fixture = Fixture::new("self-include");
        let config_path = fixture.write("config", "Include loop.conf\n");
        fixture.write(
            "loop.conf",
            "Host loopback\n  HostName loop.example\nInclude loop.conf\n",
        );

        let config = SshConfig::read(&config_path).expect("self-including file must not hang");
        assert_eq!(config.hosts().len(), 1);
        assert_eq!(config.hosts()[0].alias(), "loopback");
        assert_eq!(config.hosts()[0].host_name(), Some("loop.example"));
    }

    #[test]
    fn include_depth_boundary_still_stops_deeper_files() {
        let fixture = Fixture::new("include-depth");
        let config_path = fixture.write("config", "Include levels/0.conf\n");
        for index in 0..=MAX_INCLUDE_DEPTH {
            let include = if index < MAX_INCLUDE_DEPTH {
                let next = index + 1;
                format!("Include levels/{next}.conf\n")
            } else {
                String::new()
            };
            fixture.write(
                &format!("levels/{index}.conf"),
                &format!("Host depth-{index}\n{include}"),
            );
        }

        let config = SshConfig::read(&config_path).expect("bounded include chain parses");
        assert_eq!(config.hosts().len(), MAX_INCLUDE_DEPTH);
        assert_eq!(config.hosts()[0].alias(), "depth-0");
        assert_eq!(
            config.hosts()[MAX_INCLUDE_DEPTH - 1].alias(),
            format!("depth-{}", MAX_INCLUDE_DEPTH - 1)
        );
        assert!(config.hosts().iter().all(|host| host.alias() != "depth-16"));
    }

    #[test]
    fn include_cannot_escape_the_top_level_config_directory() {
        let fixture = Fixture::new("include-boundary");
        let config_path = fixture.write("config", "Include ../outside.conf\n");
        let outside = fixture
            .root
            .parent()
            .expect("fixture has parent")
            .join("outside.conf");
        fs::write(&outside, "Host should-not-be-read\n").expect("write outside fixture");

        let config = SshConfig::read(&config_path).expect("boundary fixture loads");
        assert!(config.hosts().is_empty());
        let _ = fs::remove_file(outside);
    }

    #[test]
    fn include_returns_to_the_caller_context_after_expanding() {
        // OpenSSH restores the caller's active Host context after an Include.
        // The Include below runs at the global (pre-Host) context, so a setting
        // after it is global and applies to both aliases defined by the included
        // files. Without restoration, the last included Host would steal it.
        let fixture = Fixture::new("include-context-restore");
        let config_path = fixture.write("config", "Include parts/*.conf\nUser chosen\n");
        fixture.write("parts/a.conf", "Host foo\n");
        fixture.write("parts/b.conf", "Host bar\n");

        let config = SshConfig::read(&config_path).expect("include context restores");
        let foo = config
            .hosts()
            .iter()
            .find(|host| host.alias() == "foo")
            .expect("foo present");
        let bar = config
            .hosts()
            .iter()
            .find(|host| host.alias() == "bar")
            .expect("bar present");
        assert_eq!(foo.user(), Some("chosen"));
        assert_eq!(bar.user(), Some("chosen"));
    }

    #[test]
    fn host_inside_an_included_file_does_not_steal_following_parent_settings() {
        // A Host directive inside an included file must not capture settings that
        // follow the Include in the caller's Host block; those belong to the
        // caller context that is restored after the included file returns.
        let fixture = Fixture::new("include-parent-context");
        let config_path = fixture.write(
            "config",
            "Host parent\n  Include child.conf\n  User parent-user\n",
        );
        fixture.write("child.conf", "Host child\n  HostName child.example\n");

        let config = SshConfig::read(&config_path).expect("parent context restores after include");
        let parent = config
            .hosts()
            .iter()
            .find(|host| host.alias() == "parent")
            .expect("parent present");
        let child = config
            .hosts()
            .iter()
            .find(|host| host.alias() == "child")
            .expect("child present");
        assert_eq!(parent.user(), Some("parent-user"));
        assert_eq!(child.host_name(), Some("child.example"));
        assert_eq!(child.user(), None);
    }

    #[test]
    fn the_same_file_included_under_two_host_sections_applies_to_both() {
        // OpenSSH parses the same file again once its prior invocation has
        // returned, so a shared include applies to each caller Host context.
        // Cycles are still stopped while a file is active on the recursion stack.
        let fixture = Fixture::new("include-repeated");
        let config_path = fixture.write(
            "config",
            "Host foo\n  Include shared.conf\nHost bar\n  Include shared.conf\n",
        );
        fixture.write("shared.conf", "User chosen\n");

        let config = SshConfig::read(&config_path).expect("repeated include parses");
        let foo = config
            .hosts()
            .iter()
            .find(|host| host.alias() == "foo")
            .expect("foo present");
        let bar = config
            .hosts()
            .iter()
            .find(|host| host.alias() == "bar")
            .expect("bar present");
        assert_eq!(foo.user(), Some("chosen"));
        assert_eq!(bar.user(), Some("chosen"));
    }

    #[test]
    fn indirect_cycle_through_repeated_include_still_terminates() {
        // shared.conf pulls loop.conf which pulls shared.conf again; the active
        // recursion stack stops the second shared.conf while the first is still
        // open, so the parse terminates.
        let fixture = Fixture::new("include-indirect-cycle");
        let config_path = fixture.write("config", "Host outer\n  Include shared.conf\n");
        fixture.write("shared.conf", "User chosen\nInclude loop.conf\n");
        fixture.write("loop.conf", "Host looper\nInclude shared.conf\n");

        let config = SshConfig::read(&config_path).expect("indirect cycle terminates");
        let outer = config
            .hosts()
            .iter()
            .find(|host| host.alias() == "outer")
            .expect("outer present");
        assert_eq!(outer.user(), Some("chosen"));
        assert!(config.hosts().iter().any(|host| host.alias() == "looper"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_glob_follows_logical_matched_path_order() {
        use std::os::unix::fs::symlink;
        // parts/a.conf links to targets/z.conf and parts/z.conf links to
        // targets/a.conf. Canonical-target order (a < z) is the inverse of the
        // matched-path order (parts/a.conf < parts/z.conf); OpenSSH follows the
        // matched path, so the a-link's target (z) wins.
        let fixture = Fixture::new("include-symlink-order");
        let config_path = fixture.write("config", "Include parts/*.conf\n");
        fixture.write("targets/z.conf", "Host ordered\n  HostName from-z-target\n");
        fixture.write("targets/a.conf", "Host ordered\n  HostName from-a-target\n");
        fs::create_dir_all(fixture.root.join("parts")).expect("create parts directory");
        symlink("../targets/z.conf", fixture.root.join("parts/a.conf")).expect("symlink a");
        symlink("../targets/a.conf", fixture.root.join("parts/z.conf")).expect("symlink z");

        let config = SshConfig::read(&config_path).expect("symlink glob parses");
        assert_eq!(config.hosts().len(), 1);
        assert_eq!(config.hosts()[0].host_name(), Some("from-z-target"));
    }

    #[cfg(unix)]
    #[test]
    fn nested_wildcard_through_a_directory_symlink_keeps_logical_identity() {
        use std::os::unix::fs::symlink;
        // dirs/alpha -> targets/zzz and dirs/beta -> targets/aaa. Canonical
        // target order (aaa < zzz) is the inverse of matched-path order
        // (alpha < beta); the wildcard must keep the logical path across the
        // directory symlink so inclusion order follows the matched path.
        let fixture = Fixture::new("include-nested-symlink");
        let config_path = fixture.write("config", "Include dirs/*/x.conf\n");
        fixture.write(
            "targets/zzz/x.conf",
            "Host alpha-host\n  HostName alpha.example\n",
        );
        fixture.write(
            "targets/aaa/x.conf",
            "Host beta-host\n  HostName beta.example\n",
        );
        fs::create_dir_all(fixture.root.join("dirs")).expect("create dirs");
        symlink("../targets/zzz", fixture.root.join("dirs/alpha")).expect("symlink alpha");
        symlink("../targets/aaa", fixture.root.join("dirs/beta")).expect("symlink beta");

        let config = SshConfig::read(&config_path).expect("nested symlink glob parses");
        let names: Vec<&str> = config.hosts().iter().map(SshHost::alias).collect();
        assert_eq!(names, vec!["alpha-host", "beta-host"]);
    }

    #[cfg(unix)]
    #[test]
    fn regular_config_and_in_root_symlink_still_parse() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new("regular-and-symlink");
        let config_path =
            fixture.write("regular.conf", "Host regular\n  HostName regular.example\n");
        let symlink_path = fixture.root.join("symlink.conf");
        symlink("regular.conf", &symlink_path).expect("create in-root config symlink");

        for source in [&config_path, &symlink_path] {
            let config = SshConfig::read(source).expect("regular config source parses");
            assert_eq!(config.hosts().len(), 1);
            assert_eq!(config.hosts()[0].alias(), "regular");
            assert_eq!(config.hosts()[0].host_name(), Some("regular.example"));
        }
    }

    /// Production-seam coverage for [`open_regular_file`], exercising each
    /// independent confinement backstop in isolation. The paths are built from
    /// a `canonicalize`-resolved fixture root so the only symlink on each path
    /// is the one the test controls, keeping the assertions deterministic
    /// regardless of the host temp directory's own ancestor symlinks. The FIFO
    /// seam is invoked by the bounded subprocess helper so losing `O_NONBLOCK`
    /// cannot hang the main test process.
    #[cfg(target_vendor = "apple")]
    #[test]
    fn open_regular_file_rejects_ancestor_symlink_atomically() {
        use std::os::unix::fs::symlink;

        // Models the exact post-canonicalization race from the review: a path
        // that canonicalize() resolved is now reached through an ancestor
        // component that an attacker swapped for a symlink. A single open(2)
        // with O_NOFOLLOW_ANY must reject any ancestor symlink atomically.
        // Removing O_NOFOLLOW_ANY (leaving only final-component O_NOFOLLOW)
        // lets the open follow the ancestor symlink and succeed, so this test
        // fails under that mutation.
        let fixture = Fixture::new("ancestor-symlink-race");
        fixture.write("realdir/target.conf", "Host outside-race\n");
        symlink("realdir", fixture.root.join("linkdir")).expect("create ancestor symlink");
        let canonical_root = fs::canonicalize(&fixture.root).expect("canonical fixture root");
        let path = canonical_root.join("linkdir/target.conf");
        assert!(
            open_regular_file(&path).is_none(),
            "ancestor symlink must be rejected atomically by one open(2)"
        );
    }

    #[cfg(unix)]
    #[test]
    fn open_regular_file_rejects_final_component_symlink() {
        use std::os::unix::fs::symlink;

        // A final-component symlink to a regular file must be rejected. On
        // Apple this is covered by O_NOFOLLOW_ANY (a strict superset); on
        // other Unix it is the O_NOFOLLOW contract. Removing that final-
        // component protection lets the open follow the symlink and succeed,
        // so this test fails under that mutation on any Unix that lacks the
        // Apple primitive.
        let fixture = Fixture::new("final-symlink");
        fixture.write("target.conf", "Host via-symlink\n");
        symlink("target.conf", fixture.root.join("link.conf")).expect("create final symlink");
        let canonical_root = fs::canonicalize(&fixture.root).expect("canonical fixture root");
        let path = canonical_root.join("link.conf");
        assert!(
            open_regular_file(&path).is_none(),
            "final-component symlink must be rejected"
        );
    }

    #[cfg(unix)]
    #[test]
    fn open_regular_file_rejects_fifo_descriptor_via_metadata() {
        // The helper invokes open_regular_file directly. O_NONBLOCK lets
        // open(2) return a FIFO descriptor promptly, and the opened-descriptor
        // metadata check must then reject it. Bypassing is_file() fails in the
        // child; removing O_NONBLOCK times out, after which the parent kills
        // and reaps the child. A directory-only assertion would be vacuous
        // because read_to_end on a directory errors regardless.
        let fixture = Fixture::new("fifo-descriptor");
        create_fifo(&fixture.root.join("source.fifo"));
        let canonical_root = fs::canonicalize(&fixture.root).expect("canonical fixture root");
        let path = canonical_root.join("source.fifo");
        assert_fifo_helper_completes(
            FIFO_HELPER_MODE_OPEN_REGULAR_FILE,
            &path,
            FIFO_HELPER_EXPECTED_NONE,
            &fixture.root.join("helper.ack"),
        );
    }

    #[cfg(unix)]
    #[test]
    fn top_level_fifo_returns_promptly() {
        let fixture = Fixture::new("top-level-fifo");
        let fifo_path = fixture.root.join("config.fifo");
        create_fifo(&fifo_path);
        assert_fifo_helper_completes(
            FIFO_HELPER_MODE_READ_CONFIG,
            &fifo_path,
            FIFO_HELPER_EXPECTED_EMPTY,
            &fixture.root.join("helper.ack"),
        );
    }

    #[cfg(unix)]
    #[test]
    fn top_level_symlink_to_fifo_returns_promptly() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new("top-level-fifo-symlink");
        let fifo_path = fixture.root.join("target.fifo");
        let symlink_path = fixture.root.join("config");
        create_fifo(&fifo_path);
        symlink("target.fifo", &symlink_path).expect("create top-level FIFO symlink");
        assert_fifo_helper_completes(
            FIFO_HELPER_MODE_READ_CONFIG,
            &symlink_path,
            FIFO_HELPER_EXPECTED_EMPTY,
            &fixture.root.join("helper.ack"),
        );
    }

    #[cfg(unix)]
    #[test]
    fn included_fifo_sources_return_promptly() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new("included-fifos");
        let config_path = fixture.write(
            "config",
            "Include parts/literal.fifo parts/link.fifo\nHost after-fifos\n  User parsed\n",
        );
        let fifo_path = fixture.root.join("parts/literal.fifo");
        create_fifo(&fifo_path);
        symlink("literal.fifo", fixture.root.join("parts/link.fifo"))
            .expect("create included FIFO symlink");
        assert_fifo_helper_completes(
            FIFO_HELPER_MODE_READ_CONFIG,
            &config_path,
            FIFO_HELPER_EXPECTED_AFTER_FIFOS,
            &fixture.root.join("helper.ack"),
        );
    }

    #[test]
    fn token_item_cap_accepts_boundary_then_rejects_one_over() {
        // Each "Host h" line is two tokens. A cap of four accepts exactly two
        // lines; the fifth token (the keyword of the third line) is rejected
        // before it is pushed.
        let limits = ParserLimits {
            token_items: 4,
            ..DEFAULT_LIMITS
        };
        let ok = SshConfig::parse_with_limits("Host a\nHost b\n", limits)
            .expect("exactly four tokens parse");
        assert_eq!(ok.hosts().len(), 2);

        let error = SshConfig::parse_with_limits("Host a\nHost b\nHost secret-value\n", limits)
            .expect_err("the fifth token is rejected before it is pushed");
        assert_eq!(
            error.kind(),
            &SshConfigErrorKind::StructuralComplexityExceeded
        );
        assert!(!error.to_string().contains("secret-value"));
        assert!(!format!("{error:?}").contains("secret-value"));
    }

    #[test]
    fn token_item_cap_aggregates_across_include_files() {
        let fixture = Fixture::new("include-token-cap");
        let config_path = fixture.write("config", "Include parts/a.conf\nInclude parts/b.conf\n");
        fixture.write("parts/a.conf", "Host a\n");
        fixture.write("parts/b.conf", "Host b\n");
        // config contributes four tokens, a.conf two, b.conf two (eight total);
        // a shared cap of seven rejects inside the second include.
        let limits = ParserLimits {
            token_items: 7,
            ..DEFAULT_LIMITS
        };
        let error = SshConfig::read_with_limits(&config_path, limits)
            .expect_err("the shared token cap rejects inside the second include");
        assert_eq!(
            error.kind(),
            &SshConfigErrorKind::StructuralComplexityExceeded
        );
    }

    #[test]
    fn ignored_directives_still_consume_the_token_cap() {
        // Unknown keywords are ignored semantically but their tokens are still
        // charged, so a hostile mass of ignored directives cannot exhaust memory.
        let limits = ParserLimits {
            token_items: 2,
            ..DEFAULT_LIMITS
        };
        let error = SshConfig::parse_with_limits("Host a\nProxyCommand secret-cmd\n", limits)
            .expect_err("ignored directive tokens are charged");
        assert_eq!(
            error.kind(),
            &SshConfigErrorKind::StructuralComplexityExceeded
        );
        assert!(!error.to_string().contains("secret-cmd"));
        assert!(!format!("{error:?}").contains("secret-cmd"));
    }

    #[test]
    fn host_cap_accepts_boundary_then_rejects_one_over() {
        let limits = ParserLimits {
            hosts: 3,
            ..DEFAULT_LIMITS
        };
        let ok = SshConfig::parse_with_limits("Host a\nHost b\nHost c\n", limits)
            .expect("exactly three distinct hosts parse");
        assert_eq!(ok.hosts().len(), 3);

        let error = SshConfig::parse_with_limits("Host a\nHost b\nHost c\nHost secret-d\n", limits)
            .expect_err("the fourth distinct host is rejected before state is added");
        assert_eq!(error.kind(), &SshConfigErrorKind::HostCountExceeded);
        assert!(!error.to_string().contains("secret-d"));
        assert!(!format!("{error:?}").contains("secret-d"));
    }

    #[test]
    fn case_insensitive_duplicate_hosts_do_not_consume_extra_slots() {
        let limits = ParserLimits {
            hosts: 1,
            ..DEFAULT_LIMITS
        };
        let config = SshConfig::parse_with_limits("Host web\nHost WEB\n", limits)
            .expect("case-insensitive duplicate fits one slot");
        assert_eq!(config.hosts().len(), 1);
        assert_eq!(config.hosts()[0].alias(), "web");
    }

    #[test]
    fn missing_top_level_file_is_an_empty_success() {
        let fixture = Fixture::new("missing");
        let config = SshConfig::read(&fixture.root.join("does-not-exist"))
            .expect("missing SSH config is not an error");
        assert!(config.hosts().is_empty());
    }

    #[test]
    fn parse_rejects_oversized_input_before_malformed_content() {
        let secret = "oversized-secret-host";
        let mut text = format!("Host \"{secret}");
        text.push_str(&"x".repeat(MAX_FILE_BYTES + 1 - text.len()));
        assert_eq!(text.len(), MAX_FILE_BYTES + 1);

        let error = SshConfig::parse(&text).expect_err("oversized input is rejected first");
        assert_eq!(error.line(), 0);
        assert_eq!(error.kind(), &SshConfigErrorKind::FileTooLarge);
        assert!(!error.to_string().contains(secret));
        assert!(!format!("{error:?}").contains(secret));
    }

    #[test]
    fn exactly_at_per_input_limit_still_parses() {
        let text = format!("#{}", "x".repeat(MAX_FILE_BYTES - 1));
        assert_eq!(text.len(), MAX_FILE_BYTES);

        let config = SshConfig::parse(&text).expect("exactly bounded input parses");
        assert!(config.hosts().is_empty());
    }

    #[test]
    fn bounded_reader_consumes_at_most_limit_plus_one() {
        struct CountingReader {
            remaining: usize,
            consumed: usize,
        }

        impl std::io::Read for CountingReader {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                let amount = buffer.len().min(self.remaining);
                buffer[..amount].fill(b'x');
                self.remaining -= amount;
                self.consumed += amount;
                Ok(amount)
            }
        }

        let mut oversized = CountingReader {
            remaining: 10_000,
            consumed: 0,
        };
        let error = read_bounded(&mut oversized, 32)
            .expect_err("the limit-plus-one byte proves oversized input");
        assert_eq!(error, BoundedReadError::TooLarge);
        assert_eq!(oversized.consumed, 33);

        let mut exact = CountingReader {
            remaining: 32,
            consumed: 0,
        };
        let bytes = read_bounded(&mut exact, 32).expect("exactly bounded reader succeeds");
        assert_eq!(bytes.len(), 32);
        assert_eq!(exact.consumed, 32);
    }

    #[test]
    fn exactly_at_file_limit_is_loaded_by_bounded_reader() {
        let fixture = Fixture::new("exact-file-limit");
        let contents = format!("#{}", "x".repeat(MAX_FILE_BYTES - 1));
        let config_path = fixture.write("config", &contents);

        let config = SshConfig::read(&config_path).expect("exactly bounded file loads");
        assert!(config.hosts().is_empty());
    }

    #[test]
    fn oversized_top_level_file_is_typed_and_content_free() {
        let fixture = Fixture::new("oversized-top-level");
        let config_path = fixture.root.join("secret-config-name");
        let file = File::create(&config_path).expect("create sparse top-level config");
        file.set_len(MAX_FILE_BYTES as u64 + 1)
            .expect("extend sparse top-level config");

        let error = SshConfig::read(&config_path).expect_err("oversized file is rejected");
        assert_eq!(error.line(), 0);
        assert_eq!(error.kind(), &SshConfigErrorKind::FileTooLarge);
        assert!(!error.to_string().contains("secret-config-name"));
        assert!(!format!("{error:?}").contains("secret-config-name"));
    }

    #[test]
    fn oversized_included_file_uses_same_bounded_reader_and_safe_error() {
        let fixture = Fixture::new("oversized-include");
        let config_path = fixture.write("config", "Include parts/secret-name.conf\n");
        let included = fixture.root.join("parts/secret-name.conf");
        fs::create_dir_all(included.parent().expect("include has parent"))
            .expect("create include parent");
        let file = File::create(&included).expect("create sparse include");
        file.set_len(MAX_FILE_BYTES as u64 + 1)
            .expect("extend sparse include");

        let error = SshConfig::read(&config_path).expect_err("oversized include is rejected");
        assert_eq!(error.line(), 1);
        assert_eq!(error.kind(), &SshConfigErrorKind::FileTooLarge);
        assert!(!error.to_string().contains("secret-name"));
        assert!(!format!("{error:?}").contains("secret-name"));
    }

    #[test]
    fn malformed_input_is_typed_and_never_echoes_config_content() {
        let secret = "internal-secret-hostname";
        let error = SshConfig::parse(&format!("Host web\nPort {secret}\n"))
            .expect_err("invalid port must be rejected");

        assert_eq!(error.line(), 2);
        assert_eq!(error.kind(), &SshConfigErrorKind::InvalidPort);
        assert!(!error.to_string().contains(secret));
        assert!(!format!("{error:?}").contains(secret));
    }

    #[test]
    fn host_keyword_without_patterns_yields_missing_host_pattern_without_content() {
        let secret = "leaked-hostname";
        let error = SshConfig::parse(&format!("Host\nHostName {secret}\n"))
            .expect_err("Host without patterns must be rejected");

        assert_eq!(error.line(), 1);
        assert_eq!(error.kind(), &SshConfigErrorKind::MissingHostPattern);
        assert!(!error.to_string().contains(secret));
        assert!(!format!("{error:?}").contains(secret));
    }

    #[test]
    fn setting_without_value_yields_missing_argument_without_content() {
        let secret = "leaked-port";
        let error = SshConfig::parse(&format!("Host web\nHostName\nPort {secret}\n"))
            .expect_err("setting without value must be rejected");

        assert_eq!(error.line(), 2);
        assert_eq!(error.kind(), &SshConfigErrorKind::MissingArgument);
        assert!(!error.to_string().contains(secret));
        assert!(!format!("{error:?}").contains(secret));
    }

    #[test]
    fn unterminated_quote_yields_unterminated_argument_without_content() {
        let secret = "leaked-hostname";
        let error = SshConfig::parse(&format!("Host \"web\nHostName {secret}\n"))
            .expect_err("unterminated quote must be rejected");

        assert_eq!(error.line(), 1);
        assert_eq!(error.kind(), &SshConfigErrorKind::UnterminatedArgument);
        assert!(!error.to_string().contains(secret));
        assert!(!format!("{error:?}").contains(secret));
    }

    #[test]
    fn identity_file_is_opaque_and_never_required_for_config_parsing() {
        let fixture = Fixture::new("identity-file");
        let config_path = fixture.write(
            "config",
            "Host deploy\n  HostName deploy.example\n  IdentityFile ~/.ssh/id_ed25519\n",
        );

        let config = SshConfig::read(&config_path).expect("opaque IdentityFile parses");
        assert_eq!(config.hosts().len(), 1);
        assert_eq!(config.hosts()[0].host_name(), Some("deploy.example"));
    }

    #[test]
    fn keywords_are_case_insensitive_and_values_are_preserved() {
        let config =
            SshConfig::parse("hOsT MyAlias\nhOsTnAmE MiXeD.Internal\nuSeR DeployUser\npOrT 2201\n")
                .expect("case-insensitive keywords parse");
        let host = &config.hosts()[0];
        assert_eq!(host.alias(), "MyAlias");
        assert_eq!(host.host_name(), Some("MiXeD.Internal"));
        assert_eq!(host.user(), Some("DeployUser"));
        assert_eq!(host.port(), Some(2201));
    }
}
