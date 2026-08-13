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
#[path = "ssh_config/tests.rs"]
mod tests;
