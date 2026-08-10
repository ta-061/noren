//! Bounded, read-only parsing of the user's OpenSSH client configuration.
//!
//! This is an app-owned filesystem adapter rather than a session or transport
//! implementation. It produces host facts for a later sidebar slice; it never
//! opens an SSH connection and never opens a path named by `IdentityFile` (or
//! any other non-`Include` directive).

use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Maximum include nesting. Cycles are also stopped by the visited-file set.
pub const MAX_INCLUDE_DEPTH: usize = 16;

const MAX_FILE_BYTES: usize = 1024 * 1024;
const MAX_INCLUDED_FILES: usize = 256;

/// Maximum conservative work estimate for resolving aliases against blocks
/// that cannot use the literal index.
///
/// Before resolving any host, every alias/pattern pair is charged
/// `(alias_bytes + 1) * (pattern_bytes + 1)` units. This covers conversion to
/// characters and the iterative glob matcher's polynomial worst case. Every
/// fallback block and setting visit is charged separately. Sixteen mebi-units
/// leaves ample room for realistic configurations while rejecting hostile
/// alias x wildcard-block products before that cross-product is evaluated.
const MAX_RESOLUTION_WORK: u128 = 16 * 1024 * 1024;

/// Parsed SSH host facts. Values are the first values obtained for the alias,
/// following OpenSSH's per-keyword precedence rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshHost {
    alias: String,
    host_name: Option<String>,
    user: Option<String>,
    port: Option<u16>,
}

impl SshHost {
    /// The alias named by a literal `Host` pattern.
    #[must_use]
    pub fn alias(&self) -> &str {
        &self.alias
    }

    /// The configured `HostName`, if one was provided.
    #[must_use]
    pub fn host_name(&self) -> Option<&str> {
        self.host_name.as_deref()
    }

    /// The configured `User`, if one was provided.
    #[must_use]
    pub fn user(&self) -> Option<&str> {
        self.user.as_deref()
    }

    /// The configured `Port`, if one was provided.
    #[must_use]
    pub const fn port(&self) -> Option<u16> {
        self.port
    }
}

/// Parsed SSH configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SshConfig {
    hosts: Vec<SshHost>,
}

impl SshConfig {
    /// Read an explicit OpenSSH config path.
    ///
    /// A missing or unreadable top-level file is treated as an empty config.
    /// Errors from a file that can be read are limited to malformed input and
    /// bounded-input failures; no error includes source text.
    pub fn read(path: &Path) -> Result<Self, SshConfigError> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(_) => return Ok(Self::default()),
        };
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
        let text = decode_file(bytes, 0)?;
        Self::from_text_with_includes(&text, &source, &root)
    }

    /// Read the conventional per-user configuration at `~/.ssh/config`.
    ///
    /// If `HOME` is not available, or the file cannot be read, the result is
    /// an empty host list.
    pub fn read_default() -> Result<Self, SshConfigError> {
        let Some(home) = env::var_os("HOME") else {
            return Ok(Self::default());
        };
        Self::read(&PathBuf::from(home).join(".ssh/config"))
    }

    /// Parse configuration text without resolving `Include` directives.
    ///
    /// This is useful for deterministic callers that already expanded their
    /// source, and keeps the parser independently testable. Use [`Self::read`]
    /// for normal file loading.
    pub fn parse(text: &str) -> Result<Self, SshConfigError> {
        let blocks = parse_text(text, 0)?;
        Self::from_blocks(&blocks)
    }

    /// The concrete hosts discovered from literal `Host` patterns.
    #[must_use]
    pub fn hosts(&self) -> &[SshHost] {
        &self.hosts
    }

    fn from_text_with_includes(
        text: &str,
        source: &Path,
        root: &Path,
    ) -> Result<Self, SshConfigError> {
        let mut blocks = Vec::new();
        let mut current = None;
        let mut visited = HashSet::new();
        let mut included_files = 0;
        let mut state = ParseState {
            visited: &mut visited,
            included_files: &mut included_files,
            blocks: &mut blocks,
            current: &mut current,
        };
        parse_file(text, source, root, 0, &mut state)?;
        Self::from_blocks(&blocks)
    }

    fn from_blocks(blocks: &[Block]) -> Result<Self, SshConfigError> {
        let mut aliases = Vec::new();
        let mut seen_aliases = HashSet::new();
        let mut literal_blocks = HashMap::<String, Vec<usize>>::new();
        let mut fallback_blocks = Vec::new();
        let mut fallback_pattern_work = 0_u128;
        let mut fallback_settings = 0_u128;

        // Literal-only blocks can be looked up by alias. Blocks with a wildcard
        // (and global blocks) remain in their original order for every alias.
        // All of this indexing is linear in the parsed input; no alias is
        // matched against a fallback block until the budget check below passes.
        for (block_index, block) in blocks.iter().enumerate() {
            let Some(patterns) = &block.patterns else {
                fallback_blocks.push(block_index);
                fallback_settings = fallback_settings.saturating_add(block.settings.len() as u128);
                continue;
            };

            for pattern in patterns {
                if !pattern.starts_with('!') && !has_wildcard(pattern) {
                    let key = pattern.to_ascii_lowercase();
                    if seen_aliases.insert(key) {
                        aliases.push(pattern.clone());
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
                    }
                }
            } else {
                fallback_blocks.push(block_index);
                fallback_settings = fallback_settings.saturating_add(block.settings.len() as u128);
                for pattern in patterns {
                    fallback_pattern_work =
                        fallback_pattern_work.saturating_add(pattern.len() as u128 + 1);
                }
            }
        }

        let alias_count = aliases.len() as u128;
        let alias_work = aliases.iter().fold(0_u128, |work, alias| {
            work.saturating_add(alias.len() as u128 + 1)
        });
        let fallback_visits = (fallback_blocks.len() as u128)
            .saturating_add(fallback_settings)
            .saturating_mul(alias_count);
        let resolution_work = alias_work
            .saturating_mul(fallback_pattern_work)
            .saturating_add(fallback_visits);
        if resolution_work > MAX_RESOLUTION_WORK {
            return Err(error(0, SshConfigErrorKind::ResolutionComplexityExceeded));
        }

        let hosts = aliases
            .into_iter()
            .map(|alias| {
                let mut host = SshHost {
                    alias: alias.clone(),
                    host_name: None,
                    user: None,
                    port: None,
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
        Ok(Self { hosts })
    }
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
    /// A `Host` directive has no patterns.
    MissingHostPattern,
    /// A `Port` argument is not a valid TCP port.
    InvalidPort,
    /// The file is not valid UTF-8.
    InvalidUtf8,
    /// The bounded input size was exceeded.
    FileTooLarge,
    /// Resolving all literal aliases against fallback patterns would exceed
    /// the parser's deterministic preflight work budget.
    ResolutionComplexityExceeded,
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Setting {
    HostName(String),
    User(String),
    Port(u16),
}

struct ParseState<'a> {
    visited: &'a mut HashSet<PathBuf>,
    included_files: &'a mut usize,
    blocks: &'a mut Vec<Block>,
    current: &'a mut Option<usize>,
}

impl Block {
    fn applies_to(&self, alias: &str) -> bool {
        match &self.patterns {
            None => true,
            Some(patterns) if patterns.is_empty() => false,
            Some(patterns) => {
                let positive = patterns
                    .iter()
                    .filter(|pattern| !pattern.starts_with('!'))
                    .any(|pattern| wildcard_match(pattern, alias));
                let negated = patterns
                    .iter()
                    .filter_map(|pattern| pattern.strip_prefix('!'))
                    .any(|pattern| wildcard_match(pattern, alias));
                positive && !negated
            }
        }
    }
}

fn parse_file(
    text: &str,
    source: &Path,
    root: &Path,
    depth: usize,
    state: &mut ParseState<'_>,
) -> Result<(), SshConfigError> {
    if depth > MAX_INCLUDE_DEPTH || !state.visited.insert(source.to_path_buf()) {
        return Ok(());
    }
    for (line_index, line) in text.lines().enumerate() {
        let line_number = line_index + 1;
        let tokens = tokenize(line, line_number)?;
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
                });
                *state.current = Some(state.blocks.len() - 1);
            }
            "match" => {
                state.blocks.push(Block {
                    patterns: Some(Vec::new()),
                    settings: Vec::new(),
                });
                *state.current = Some(state.blocks.len() - 1);
            }
            "include" => {
                if tokens.len() < 2 {
                    return Err(error(line_number, SshConfigErrorKind::MissingArgument));
                }
                for pattern in &tokens[1..] {
                    for included in expand_include(pattern, source.parent().unwrap_or(root), root) {
                        if *state.included_files >= MAX_INCLUDED_FILES {
                            break;
                        }
                        *state.included_files += 1;
                        let Some(included_source) = canonicalize_within(&included, root) else {
                            continue;
                        };
                        let Ok(bytes) = fs::read(&included_source) else {
                            continue;
                        };
                        let included_text = decode_file(bytes, line_number)?;
                        parse_file(&included_text, &included_source, root, depth + 1, state)?;
                    }
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
                if let Some(index) = *state.current {
                    state.blocks[index].settings.push(setting);
                } else {
                    state.blocks.push(Block {
                        patterns: None,
                        settings: vec![setting],
                    });
                    *state.current = Some(state.blocks.len() - 1);
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

fn parse_text(text: &str, line_offset: usize) -> Result<Vec<Block>, SshConfigError> {
    let mut blocks = Vec::new();
    let mut current = None;
    for (line_index, line) in text.lines().enumerate() {
        let line_number = line_index + 1 + line_offset;
        let tokens = tokenize(line, line_number)?;
        if tokens.is_empty() {
            continue;
        }
        let keyword = tokens[0].to_ascii_lowercase();
        match keyword.as_str() {
            "host" => {
                if tokens.len() < 2 {
                    return Err(error(line_number, SshConfigErrorKind::MissingHostPattern));
                }
                blocks.push(Block {
                    patterns: Some(tokens[1..].to_vec()),
                    settings: Vec::new(),
                });
                current = Some(blocks.len() - 1);
            }
            "match" => {
                blocks.push(Block {
                    patterns: Some(Vec::new()),
                    settings: Vec::new(),
                });
                current = Some(blocks.len() - 1);
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
                if let Some(index) = current {
                    blocks[index].settings.push(setting);
                } else {
                    blocks.push(Block {
                        patterns: None,
                        settings: vec![setting],
                    });
                    current = Some(blocks.len() - 1);
                }
            }
            "include" => {}
            _ => {}
        }
    }
    Ok(blocks)
}

fn decode_file(bytes: Vec<u8>, line: usize) -> Result<String, SshConfigError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(error(line, SshConfigErrorKind::FileTooLarge));
    }
    String::from_utf8(bytes).map_err(|_| error(line, SshConfigErrorKind::InvalidUtf8))
}

fn tokenize(line: &str, line_number: usize) -> Result<Vec<String>, SshConfigError> {
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
                    tokens.push(std::mem::take(&mut token));
                }
                token_started = false;
                boundary = true;
            }
            character if character.is_whitespace() && !quoted => {
                if token_started {
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
        tokens.push(token);
    }
    Ok(tokens)
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

fn canonicalize_within(path: &Path, root: &Path) -> Option<PathBuf> {
    let canonical = fs::canonicalize(path).ok()?;
    canonical.starts_with(root).then_some(canonical)
}

fn expand_include(pattern: &str, including_dir: &Path, root: &Path) -> Vec<PathBuf> {
    let pattern = expand_home(pattern).unwrap_or_else(|| PathBuf::from(pattern));
    let path = if pattern.is_absolute() {
        pattern
    } else {
        including_dir.join(pattern)
    };
    let mut matches = Vec::new();
    expand_components_owned(&path, root, &mut matches);
    matches.sort();
    matches
}

fn expand_home(pattern: &str) -> Option<PathBuf> {
    let suffix = pattern.strip_prefix("~/")?;
    Some(PathBuf::from(env::var_os("HOME")?).join(suffix))
}

fn expand_components_owned(path: &Path, root: &Path, matches: &mut Vec<PathBuf>) {
    let components: Vec<_> = path.components().collect();
    let current = if path.is_absolute() {
        PathBuf::from("/")
    } else {
        PathBuf::new()
    };
    let start = usize::from(path.is_absolute());
    expand_owned_tail(&components[start..], current, root, matches);
}

fn expand_owned_tail(
    components: &[Component<'_>],
    mut current: PathBuf,
    root: &Path,
    matches: &mut Vec<PathBuf>,
) {
    let Some((component, rest)) = components.split_first() else {
        if let Some(canonical) = canonicalize_within(&current, root) {
            if canonical.is_file() {
                matches.push(canonical);
            }
        }
        return;
    };
    match component {
        Component::CurDir => expand_owned_tail(rest, current, root, matches),
        Component::ParentDir => {
            current.push("..");
            let Some(canonical) = canonicalize_within(&current, root) else {
                return;
            };
            expand_owned_tail(rest, canonical, root, matches);
        }
        Component::Normal(name) if has_wildcard(&name.to_string_lossy()) => {
            let Some(directory) = canonicalize_within(&current, root) else {
                return;
            };
            let Ok(entries) = fs::read_dir(directory) else {
                return;
            };
            let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let file_name = entry.file_name();
                let Some(file_name) = file_name.to_str() else {
                    continue;
                };
                if wildcard_match(&name.to_string_lossy(), file_name) {
                    expand_owned_tail(rest, entry.path(), root, matches);
                }
            }
        }
        Component::Normal(name) => {
            current.push(name);
            expand_owned_tail(rest, current, root, matches);
        }
        Component::RootDir | Component::Prefix(_) => {}
    }
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
                },
                SshHost {
                    alias: "staging".to_owned(),
                    host_name: Some("web.internal.example".to_owned()),
                    user: Some("deploy".to_owned()),
                    port: Some(2222),
                },
                SshHost {
                    alias: "database".to_owned(),
                    host_name: Some("db.internal.example".to_owned()),
                    user: Some("postgres".to_owned()),
                    port: Some(5432),
                },
            ]
        );
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
                },
                SshHost {
                    alias: "api.example".to_owned(),
                    host_name: Some("wildcard.example".to_owned()),
                    user: Some("wildcard".to_owned()),
                    port: None,
                },
                SshHost {
                    alias: "db.example".to_owned(),
                    host_name: Some("wildcard.example".to_owned()),
                    user: Some("wildcard".to_owned()),
                    port: Some(2200),
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
                },
                SshHost {
                    alias: "prod-Web".to_owned(),
                    host_name: Some("wildcard.example".to_owned()),
                    user: Some("wildcard-user".to_owned()),
                    port: Some(2022),
                },
                SshHost {
                    alias: "PROD-ADMIN".to_owned(),
                    host_name: Some("admin.example".to_owned()),
                    user: Some("default-user".to_owned()),
                    port: Some(2022),
                },
                SshHost {
                    alias: "skip".to_owned(),
                    host_name: Some("skip.example".to_owned()),
                    user: Some("skip-user".to_owned()),
                    port: Some(2022),
                },
            ]
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
        let config = SshConfig::parse("Host empty\nHostName \"\"\nUser \"\" realuser\n")
            .expect("empty quoted values parse");

        assert_eq!(config.hosts()[0].host_name(), Some(""));
        assert_eq!(config.hosts()[0].user(), Some(""));
    }

    #[test]
    fn relative_include_is_resolved_from_the_including_file_and_cycles_stop() {
        let fixture = Fixture::new("include-cycle");
        let config_path = fixture.write("config", "Include parts/one.conf\n");
        fixture.write(
            "parts/one.conf",
            "Host included\n  HostName included.example\nInclude two.conf\n",
        );
        fixture.write(
            "parts/two.conf",
            "Host second\n  User included-user\nInclude one.conf\n",
        );

        let config = SshConfig::read(&config_path).expect("include fixture parses");
        assert_eq!(config.hosts().len(), 2);
        assert_eq!(config.hosts()[0].host_name(), Some("included.example"));
        assert_eq!(config.hosts()[1].user(), Some("included-user"));
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
    fn missing_top_level_file_is_an_empty_success() {
        let fixture = Fixture::new("missing");
        let config = SshConfig::read(&fixture.root.join("does-not-exist"))
            .expect("missing SSH config is not an error");
        assert!(config.hosts().is_empty());
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
