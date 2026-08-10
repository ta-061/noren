//! Bounded, read-only parsing of the user's OpenSSH client configuration.
//!
//! This is an app-owned filesystem adapter rather than a session or transport
//! implementation. It produces host facts for a later sidebar slice; it never
//! opens an SSH connection and never opens a path named by `IdentityFile` (or
//! any other non-`Include` directive).

use std::collections::HashSet;
use std::env;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Maximum include nesting. Cycles are also stopped by the visited-file set.
pub const MAX_INCLUDE_DEPTH: usize = 16;

const MAX_FILE_BYTES: usize = 1024 * 1024;
const MAX_INCLUDED_FILES: usize = 256;

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
        Ok(Self::from_blocks(&blocks))
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
        Ok(Self::from_blocks(&blocks))
    }

    fn from_blocks(blocks: &[Block]) -> Self {
        let mut aliases = Vec::new();
        for block in blocks {
            let Some(patterns) = &block.patterns else {
                continue;
            };
            for pattern in patterns {
                if !pattern.starts_with('!') && !has_wildcard(pattern) && !aliases.contains(pattern)
                {
                    aliases.push(pattern.clone());
                }
            }
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
                for block in blocks {
                    if !block.applies_to(&alias) {
                        continue;
                    }
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
                host
            })
            .collect();
        Self { hosts }
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
    // keeps matching iterative and linear in the pattern/candidate lengths.
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
    use std::io::Write;

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
            elapsed < std::time::Duration::from_secs(1),
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
