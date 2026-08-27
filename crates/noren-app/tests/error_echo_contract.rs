//! The file-echo contract for every file-derived error type (issue #150).
//!
//! Noren prints configuration and session-state errors straight to live
//! stderr (`main.rs`), which makes every error variant a disclosure surface
//! (threat TM-08). Issues #145 and #148 removed the raw echoes that crept
//! in; this suite is what keeps them out:
//!
//! 1. File-derived errors may name **keys** and **positions**, clipped to
//!    120 characters. They may not echo file **values** — with one
//!    allowlist: `[keys]` chord text. A chord is keybinding grammar, never
//!    a secret-bearing setting, and an error that cannot show the offending
//!    binding is not actionable (issue #150 argues this both ways and keeps
//!    the chord echo; the justifications live on the variants).
//! 2. Every variant of every file-derived error enum is classified below by
//!    an **exhaustive `match`**. Adding a variant anywhere without adding a
//!    classifier arm breaks this target's build — the failure mode for a
//!    silent echo is a compile error, not a code review hoping to notice.
//!    The per-enum variant counts and the total allowlist size are pinned,
//!    so admitting a new echo is a visible decision, and the classification
//!    itself is policed against the real rendering via the sample payload
//!    convention (`VALUE-sentinel`). The known residual: sample tables and
//!    classifiers are two lists kept in sync by these pins; a new variant
//!    cannot skip the classifier (compile error), and cannot misclassify a
//!    value-carrying sample (rendering assertion), so the only path to a
//!    silent echo runs through deliberately bypassing this file's rules.
//! 3. Sentinel probes verify the classification behaviorally through the
//!    real loaders, on both the `Display` and the derived `Debug` rendering
//!    of each error — `Debug` is a print surface too.
//!
//! The palette is covered by the same idea from the other side: its derived
//! `Debug` prints `label`, so the label type is pinned to `&'static str`,
//! which cannot hold file-derived text without an explicit leak.

use noren_app::config::{AppConfig, ChordParseError, ConfigError};
use noren_app::git_worktree::WorktreeListError;
use noren_app::palette::Palette;
use noren_app::passthrough::ChordError;
use noren_app::session::SessionRegistry;
use noren_app::session_persistence::{SESSION_STATE_VERSION, SessionPersistenceError, load_bytes};
use noren_app::ssh_config::{SshConfig, SshConfigErrorKind};
use noren_pty::PtyError;
use std::io::ErrorKind;

/// How a variant may treat file **values**. Key names, positions, and
/// app-generated canonical text are not values and are always permitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ValueEcho {
    /// The variant never carries file value text.
    Never,
    /// Echo allowlist: the variant may carry `[keys]` chord text, clipped
    /// to 120 characters, for the reason documented on the variant itself.
    ChordText,
    /// Echo allowlist: the variant may carry `[theme]` name text, clipped
    /// to 120 characters, for the reason documented on the variant itself.
    ThemeName,
}

/// Pinned variant count of [`ConfigError`]; a new variant must extend both
/// the classifier and the sample table.
const CONFIG_ERROR_VARIANTS: usize = 22;
/// Pinned variant count of [`SessionPersistenceError`].
const SESSION_ERROR_VARIANTS: usize = 14;
/// Pinned variant count of [`ChordParseError`].
const CHORD_PARSE_ERROR_VARIANTS: usize = 7;
/// Pinned variant count of [`SshConfigErrorKind`].
const SSH_ERROR_KINDS: usize = 13;
/// Pinned variant count of [`WorktreeListError`].
const WORKTREE_ERROR_VARIANTS: usize = 6;
/// Pinned variant count of [`PtyError`].
const PTY_ERROR_VARIANTS: usize = 18;
/// Pinned allowlist size across every file-derived enum. Raising this
/// number is the reviewed decision to admit a new value echo.
const CHORD_ALLOWLIST_SIZE: usize = 6;
/// Pinned size of the `[theme]` name echo allowlist; see
/// [`CHORD_ALLOWLIST_SIZE`] for the policy.
const THEME_NAME_ALLOWLIST_SIZE: usize = 1;
/// Sample payload convention (policed by the pin test): key-shaped sample
/// fields carry `sample_key` and may render; any **value-shaped** sample
/// field must carry this sentinel so the test can check the classification
/// against the actual rendering — a `Never` variant that renders it, on
/// either surface, is a contract violation on the spot.
const VALUE_SENTINEL: &str = "VALUE-sentinel";

/// Classify every [`ConfigError`] variant. Exhaustive by design: a new
/// variant fails this file's build until it is classified here.
fn classify_config(error: &ConfigError) -> ValueEcho {
    match error {
        ConfigError::NotFound => ValueEcho::Never,
        ConfigError::Io(_) => ValueEcho::Never,
        ConfigError::NotAFile => ValueEcho::Never,
        ConfigError::TooLarge => ValueEcho::Never,
        ConfigError::NotUtf8 => ValueEcho::Never,
        ConfigError::Parse { .. } => ValueEcho::Never,
        ConfigError::UnknownKey(_) => ValueEcho::Never,
        ConfigError::WrongType { .. } => ValueEcho::Never,
        ConfigError::OutOfRange { .. } => ValueEcho::Never,
        ConfigError::ChordNotAString { .. } => ValueEcho::Never,
        ConfigError::ThemeNotAString { .. } => ValueEcho::Never,
        // Allowlist: the theme name vocabulary — a closed set the schema
        // publishes — and naming the failed value is the point of the
        // error, exactly the `[keys]` chord argument.
        ConfigError::UnknownTheme { .. } => ValueEcho::ThemeName,
        // Allowlist: unparsed chord text plus its clipped parse reason —
        // naming the failed binding is the point of the error.
        ConfigError::InvalidChord { .. } => ValueEcho::ChordText,
        // Allowlist: the value already parsed, so only grammar-bounded
        // chord text can appear.
        ConfigError::UnclaimableChord { .. } => ValueEcho::ChordText,
        ConfigError::ReservedChord { .. } => ValueEcho::ChordText,
        // No file text: compile-time action names and canonical chord text
        // regenerated from the parsed chord.
        ConfigError::DuplicateChord { .. } => ValueEcho::Never,
        // `[[agents]]` variants carry only key names and array indices; the
        // name/command/args values never appear (an agent command can embed
        // a private path, and an args element anything at all).
        ConfigError::AgentTableNotAnArray { .. } => ValueEcho::Never,
        ConfigError::AgentFieldMissing { .. } => ValueEcho::Never,
        ConfigError::AgentFieldNotAString { .. } => ValueEcho::Never,
        ConfigError::AgentArgsNotAnArray { .. } => ValueEcho::Never,
        ConfigError::AgentArgNotAString { .. } => ValueEcho::Never,
        ConfigError::AgentCommandNotAbsolute { .. } => ValueEcho::Never,
    }
}

/// Classify every [`SessionPersistenceError`] variant. None is allowlisted:
/// a `sessions.toml` value has no grammar bounding what it can hold.
fn classify_session(error: &SessionPersistenceError) -> ValueEcho {
    match error {
        SessionPersistenceError::NotFound => ValueEcho::Never,
        SessionPersistenceError::Io(_) => ValueEcho::Never,
        SessionPersistenceError::NotAFile => ValueEcho::Never,
        SessionPersistenceError::TooLarge => ValueEcho::Never,
        SessionPersistenceError::NotUtf8 => ValueEcho::Never,
        SessionPersistenceError::Parse { .. } => ValueEcho::Never,
        SessionPersistenceError::MissingKey(_) => ValueEcho::Never,
        SessionPersistenceError::UnknownKey(_) => ValueEcho::Never,
        SessionPersistenceError::WrongType { .. } => ValueEcho::Never,
        SessionPersistenceError::OutOfRange { .. } => ValueEcho::Never,
        SessionPersistenceError::NonUtf8Path => ValueEcho::Never,
        SessionPersistenceError::UnknownKind => ValueEcho::Never,
        SessionPersistenceError::UnsupportedVersion(_) => ValueEcho::Never,
        SessionPersistenceError::TooManySessions => ValueEcho::Never,
    }
}

/// Classify every [`ChordParseError`] variant. The token-carrying three are
/// allowlisted as chord grammar text; their only stderr path is the clipped
/// `reason` of [`ConfigError::InvalidChord`].
fn classify_chord_parse(error: &ChordParseError) -> ValueEcho {
    match error {
        ChordParseError::Empty => ValueEcho::Never,
        ChordParseError::EmptyToken => ValueEcho::Never,
        ChordParseError::MissingKey => ValueEcho::Never,
        ChordParseError::NotAModifier(_) => ValueEcho::ChordText,
        ChordParseError::UnknownKey(_) => ValueEcho::ChordText,
        ChordParseError::RepeatedModifier(_) => ValueEcho::ChordText,
        ChordParseError::InvalidKey(_) => ValueEcho::Never,
    }
}

/// Classify every [`SshConfigErrorKind`] variant. All are unit variants;
/// the enclosing `SshConfigError` carries only a line number beside them.
fn classify_ssh_kind(kind: &SshConfigErrorKind) -> ValueEcho {
    match kind {
        SshConfigErrorKind::MissingArgument => ValueEcho::Never,
        SshConfigErrorKind::SurplusArgument => ValueEcho::Never,
        SshConfigErrorKind::MissingHostPattern => ValueEcho::Never,
        SshConfigErrorKind::InvalidPort => ValueEcho::Never,
        SshConfigErrorKind::InvalidUtf8 => ValueEcho::Never,
        SshConfigErrorKind::FileTooLarge => ValueEcho::Never,
        SshConfigErrorKind::TotalBytesExceeded => ValueEcho::Never,
        SshConfigErrorKind::IncludedFilesExceeded => ValueEcho::Never,
        SshConfigErrorKind::IncludeExpansionWorkExceeded => ValueEcho::Never,
        SshConfigErrorKind::ResolutionComplexityExceeded => ValueEcho::Never,
        SshConfigErrorKind::StructuralComplexityExceeded => ValueEcho::Never,
        SshConfigErrorKind::HostCountExceeded => ValueEcho::Never,
        SshConfigErrorKind::UnterminatedArgument => ValueEcho::Never,
    }
}

/// Classify every [`WorktreeListError`] variant. All are unit variants
/// with fixed, content-free Display strings — worktree discovery never
/// carries paths, branch names, or git output in its error messages.
fn classify_worktree_error(error: &WorktreeListError) -> ValueEcho {
    match error {
        WorktreeListError::GitUnavailable => ValueEcho::Never,
        WorktreeListError::NotARepository => ValueEcho::Never,
        WorktreeListError::LaunchDirectoryUnavailable => ValueEcho::Never,
        WorktreeListError::NotUtf8 => ValueEcho::Never,
        WorktreeListError::TooLarge => ValueEcho::Never,
        WorktreeListError::Malformed => ValueEcho::Never,
    }
}

/// Classify every [`PtyError`] variant. All carry only operation names and
/// `io::ErrorKind` discriminants — never terminal content, paths, or
/// environment values.
fn classify_pty_error(error: &PtyError) -> ValueEcho {
    match error {
        PtyError::MissingHome => ValueEcho::Never,
        PtyError::HomeNotAbsolute => ValueEcho::Never,
        PtyError::HomeNotDirectory => ValueEcho::Never,
        PtyError::CwdNotAbsolute => ValueEcho::Never,
        PtyError::CwdNotDirectory => ValueEcho::Never,
        PtyError::InvalidSize => ValueEcho::Never,
        PtyError::InputTooLarge => ValueEcho::Never,
        PtyError::ReplyTooLarge => ValueEcho::Never,
        PtyError::ReplyRateExceeded => ValueEcho::Never,
        PtyError::CommandQueueFull => ValueEcho::Never,
        PtyError::ChannelDisconnected => ValueEcho::Never,
        PtyError::SessionClosing => ValueEcho::Never,
        PtyError::ReaderJoinTimeout => ValueEcho::Never,
        PtyError::SupervisorJoinTimeout => ValueEcho::Never,
        PtyError::CommandEmpty => ValueEcho::Never,
        PtyError::CommandNotAbsolute => ValueEcho::Never,
        PtyError::Backend { .. } => ValueEcho::Never,
        PtyError::Io { .. } => ValueEcho::Never,
    }
}

/// One constructed variant per classifier arm, paired with the class the
/// contract promises for it. The counts are pinned so a new variant cannot
/// skip this table.
fn config_samples() -> Vec<(ConfigError, ValueEcho)> {
    let key = || "sample_key".to_owned();
    vec![
        (ConfigError::NotFound, ValueEcho::Never),
        (
            ConfigError::Io(ErrorKind::PermissionDenied),
            ValueEcho::Never,
        ),
        (ConfigError::NotAFile, ValueEcho::Never),
        (ConfigError::TooLarge, ValueEcho::Never),
        (ConfigError::NotUtf8, ValueEcho::Never),
        (ConfigError::Parse { line: 3, column: 4 }, ValueEcho::Never),
        (ConfigError::UnknownKey(key()), ValueEcho::Never),
        (ConfigError::WrongType { key: key() }, ValueEcho::Never),
        (ConfigError::OutOfRange { key: key() }, ValueEcho::Never),
        (
            ConfigError::ChordNotAString { key: key() },
            ValueEcho::Never,
        ),
        (
            ConfigError::ThemeNotAString { key: key() },
            ValueEcho::Never,
        ),
        (
            ConfigError::UnknownTheme {
                value: VALUE_SENTINEL.to_owned(),
            },
            ValueEcho::ThemeName,
        ),
        (
            ConfigError::InvalidChord {
                key: key(),
                value: VALUE_SENTINEL.to_owned(),
                reason: "sample reason".to_owned(),
            },
            ValueEcho::ChordText,
        ),
        (
            ConfigError::UnclaimableChord {
                key: key(),
                value: VALUE_SENTINEL.to_owned(),
            },
            ValueEcho::ChordText,
        ),
        (
            ConfigError::ReservedChord {
                key: key(),
                value: VALUE_SENTINEL.to_owned(),
            },
            ValueEcho::ChordText,
        ),
        (
            ConfigError::DuplicateChord {
                first: "a".to_owned(),
                second: "b".to_owned(),
                chord: "c".to_owned(),
            },
            ValueEcho::Never,
        ),
        (
            ConfigError::AgentTableNotAnArray { key: key() },
            ValueEcho::Never,
        ),
        (
            ConfigError::AgentFieldMissing { key: key() },
            ValueEcho::Never,
        ),
        (
            ConfigError::AgentFieldNotAString { key: key() },
            ValueEcho::Never,
        ),
        (
            ConfigError::AgentArgsNotAnArray { key: key() },
            ValueEcho::Never,
        ),
        (
            ConfigError::AgentArgNotAString { index: 0 },
            ValueEcho::Never,
        ),
        (
            ConfigError::AgentCommandNotAbsolute { key: key() },
            ValueEcho::Never,
        ),
    ]
}

fn session_samples() -> Vec<(SessionPersistenceError, ValueEcho)> {
    let key = || "sample_key".to_owned();
    vec![
        (SessionPersistenceError::NotFound, ValueEcho::Never),
        (
            SessionPersistenceError::Io(ErrorKind::PermissionDenied),
            ValueEcho::Never,
        ),
        (SessionPersistenceError::NotAFile, ValueEcho::Never),
        (SessionPersistenceError::TooLarge, ValueEcho::Never),
        (SessionPersistenceError::NotUtf8, ValueEcho::Never),
        (
            SessionPersistenceError::Parse { line: 3, column: 4 },
            ValueEcho::Never,
        ),
        (SessionPersistenceError::MissingKey(key()), ValueEcho::Never),
        (SessionPersistenceError::UnknownKey(key()), ValueEcho::Never),
        (
            SessionPersistenceError::WrongType { key: key() },
            ValueEcho::Never,
        ),
        (
            SessionPersistenceError::OutOfRange { key: key() },
            ValueEcho::Never,
        ),
        (SessionPersistenceError::NonUtf8Path, ValueEcho::Never),
        (SessionPersistenceError::UnknownKind, ValueEcho::Never),
        (
            SessionPersistenceError::UnsupportedVersion(2),
            ValueEcho::Never,
        ),
        (SessionPersistenceError::TooManySessions, ValueEcho::Never),
    ]
}

fn chord_parse_samples() -> Vec<(ChordParseError, ValueEcho)> {
    vec![
        (ChordParseError::Empty, ValueEcho::Never),
        (ChordParseError::EmptyToken, ValueEcho::Never),
        (ChordParseError::MissingKey, ValueEcho::Never),
        (
            ChordParseError::NotAModifier(VALUE_SENTINEL.to_owned()),
            ValueEcho::ChordText,
        ),
        (
            ChordParseError::UnknownKey(VALUE_SENTINEL.to_owned()),
            ValueEcho::ChordText,
        ),
        (
            ChordParseError::RepeatedModifier(VALUE_SENTINEL.to_owned()),
            ValueEcho::ChordText,
        ),
        (
            ChordParseError::InvalidKey(ChordError::FunctionKeyOutOfRange),
            ValueEcho::Never,
        ),
    ]
}

fn ssh_kind_samples() -> Vec<(SshConfigErrorKind, ValueEcho)> {
    use SshConfigErrorKind as K;
    vec![
        (K::MissingArgument, ValueEcho::Never),
        (K::SurplusArgument, ValueEcho::Never),
        (K::MissingHostPattern, ValueEcho::Never),
        (K::InvalidPort, ValueEcho::Never),
        (K::InvalidUtf8, ValueEcho::Never),
        (K::FileTooLarge, ValueEcho::Never),
        (K::TotalBytesExceeded, ValueEcho::Never),
        (K::IncludedFilesExceeded, ValueEcho::Never),
        (K::IncludeExpansionWorkExceeded, ValueEcho::Never),
        (K::ResolutionComplexityExceeded, ValueEcho::Never),
        (K::StructuralComplexityExceeded, ValueEcho::Never),
        (K::HostCountExceeded, ValueEcho::Never),
        (K::UnterminatedArgument, ValueEcho::Never),
    ]
}

fn worktree_error_samples() -> Vec<(WorktreeListError, ValueEcho)> {
    vec![
        (WorktreeListError::GitUnavailable, ValueEcho::Never),
        (WorktreeListError::NotARepository, ValueEcho::Never),
        (
            WorktreeListError::LaunchDirectoryUnavailable,
            ValueEcho::Never,
        ),
        (WorktreeListError::NotUtf8, ValueEcho::Never),
        (WorktreeListError::TooLarge, ValueEcho::Never),
        (WorktreeListError::Malformed, ValueEcho::Never),
    ]
}

fn pty_error_samples() -> Vec<(PtyError, ValueEcho)> {
    vec![
        (PtyError::MissingHome, ValueEcho::Never),
        (PtyError::HomeNotAbsolute, ValueEcho::Never),
        (PtyError::HomeNotDirectory, ValueEcho::Never),
        (PtyError::CwdNotAbsolute, ValueEcho::Never),
        (PtyError::CwdNotDirectory, ValueEcho::Never),
        (PtyError::InvalidSize, ValueEcho::Never),
        (PtyError::InputTooLarge, ValueEcho::Never),
        (PtyError::ReplyTooLarge, ValueEcho::Never),
        (PtyError::ReplyRateExceeded, ValueEcho::Never),
        (PtyError::CommandQueueFull, ValueEcho::Never),
        (PtyError::ChannelDisconnected, ValueEcho::Never),
        (PtyError::SessionClosing, ValueEcho::Never),
        (PtyError::ReaderJoinTimeout, ValueEcho::Never),
        (PtyError::SupervisorJoinTimeout, ValueEcho::Never),
        (PtyError::CommandEmpty, ValueEcho::Never),
        (PtyError::CommandNotAbsolute, ValueEcho::Never),
        (
            PtyError::Backend {
                operation: noren_pty::PtyOperation::Open,
            },
            ValueEcho::Never,
        ),
        (
            PtyError::Io {
                operation: noren_pty::PtyOperation::Read,
                kind: ErrorKind::BrokenPipe,
            },
            ValueEcho::Never,
        ),
    ]
}

/// The classifiers must cover every variant, the sample tables must match
/// the pinned variant counts, and the allowlist must be exactly the pinned
/// size. Together with the exhaustive matches this is the tripwire: a new
/// echoing variant stops the build, and a new allowlist entry stops this
/// assertion.
#[test]
fn every_file_derived_variant_is_classified_and_the_allowlist_is_pinned() {
    let config = config_samples();
    assert_eq!(
        config.len(),
        CONFIG_ERROR_VARIANTS,
        "a ConfigError variant changed; classify it and update the pin"
    );
    let session = session_samples();
    assert_eq!(
        session.len(),
        SESSION_ERROR_VARIANTS,
        "a SessionPersistenceError variant changed; classify it and update the pin"
    );
    let chords = chord_parse_samples();
    assert_eq!(
        chords.len(),
        CHORD_PARSE_ERROR_VARIANTS,
        "a ChordParseError variant changed; classify it and update the pin"
    );
    let ssh = ssh_kind_samples();
    assert_eq!(
        ssh.len(),
        SSH_ERROR_KINDS,
        "an SshConfigErrorKind variant changed; classify it and update the pin"
    );
    let worktree = worktree_error_samples();
    assert_eq!(
        worktree.len(),
        WORKTREE_ERROR_VARIANTS,
        "a WorktreeListError variant changed; classify it and update the pin"
    );
    let pty = pty_error_samples();
    assert_eq!(
        pty.len(),
        PTY_ERROR_VARIANTS,
        "a PtyError variant changed; classify it and update the pin"
    );

    for (sample, expected) in &config {
        assert_eq!(&classify_config(sample), expected, "{sample:?}");
    }
    for (sample, expected) in &session {
        assert_eq!(&classify_session(sample), expected, "{sample:?}");
    }
    for (sample, expected) in &chords {
        assert_eq!(&classify_chord_parse(sample), expected, "{sample:?}");
    }
    for (sample, expected) in &ssh {
        assert_eq!(&classify_ssh_kind(sample), expected, "{sample:?}");
    }
    for (sample, expected) in &worktree {
        assert_eq!(&classify_worktree_error(sample), expected, "{sample:?}");
    }
    for (sample, expected) in &pty {
        assert_eq!(&classify_pty_error(sample), expected, "{sample:?}");
    }

    // Police the classification against the actual rendering, on both
    // surfaces, using the sample payload convention: a Never-classified
    // variant must not render the value sentinel anywhere (derived Debug
    // included), and an allowlisted variant must actually carry it in
    // Display — an allowlist entry that never echoes is a classification
    // error, and usability probe below pins the real loader paths.
    let assert_rendering = |sample: String, debug: String, class: ValueEcho| match class {
        ValueEcho::Never => {
            assert!(
                !sample.contains(VALUE_SENTINEL),
                "a Never variant must not render file values: {sample}"
            );
            assert!(
                !debug.contains(VALUE_SENTINEL),
                "a Never variant must not carry file values in Debug: {debug}"
            );
        }
        ValueEcho::ChordText => {
            assert!(
                sample.contains(VALUE_SENTINEL),
                "an allowlisted variant must echo its chord text: {sample}"
            );
        }
        ValueEcho::ThemeName => {
            assert!(
                sample.contains(VALUE_SENTINEL),
                "an allowlisted variant must echo its theme name text: {sample}"
            );
        }
    };
    for (sample, class) in &config {
        assert_rendering(sample.to_string(), format!("{sample:?}"), *class);
    }
    for (sample, class) in &session {
        assert_rendering(sample.to_string(), format!("{sample:?}"), *class);
    }
    for (sample, class) in &chords {
        assert_rendering(sample.to_string(), format!("{sample:?}"), *class);
    }
    for (sample, class) in &ssh {
        // The kinds have no Display of their own; SshConfigError renders
        // them with `{:?}`, so Debug is the rendering surface here.
        assert_rendering(format!("{sample:?}"), format!("{sample:?}"), *class);
    }
    for (sample, class) in &worktree {
        assert_rendering(sample.to_string(), format!("{sample:?}"), *class);
    }
    for (sample, class) in &pty {
        assert_rendering(sample.to_string(), format!("{sample:?}"), *class);
    }

    let allowlist = config
        .iter()
        .map(|(_, class)| *class)
        .chain(session.iter().map(|(_, class)| *class))
        .chain(chords.iter().map(|(_, class)| *class))
        .chain(ssh.iter().map(|(_, class)| *class))
        .chain(worktree.iter().map(|(_, class)| *class))
        .chain(pty.iter().map(|(_, class)| *class))
        .filter(|class| *class == ValueEcho::ChordText)
        .count();
    assert_eq!(
        allowlist, CHORD_ALLOWLIST_SIZE,
        "the echo allowlist changed size; admitting a new value echo is a \
         reviewed decision (update this pin and the variant docs together)"
    );

    let theme_allowlist = config
        .iter()
        .map(|(_, class)| *class)
        .filter(|class| *class == ValueEcho::ThemeName)
        .count();
    assert_eq!(
        theme_allowlist, THEME_NAME_ALLOWLIST_SIZE,
        "the [theme] name echo allowlist changed size; admitting a new value \
         echo is a reviewed decision (update this pin and the variant docs \
         together)"
    );
}

/// Both renderings of a session-state error stay free of a value sentinel.
/// The `kind` value is arbitrary text — an SSH target pasted into the wrong
/// field lands here — so nothing but key names and positions may appear.
#[test]
fn session_state_values_never_reach_display_or_debug() {
    const SENTINEL: &str = "ops@VALUE-SENTINEL.bastion.example";
    let cases = [
        // Unknown kind value.
        format!("version = {SESSION_STATE_VERSION}\n\n[[sessions]]\nkind = \"{SENTINEL}\"\n"),
        // A payload and an unknown extra key: the payload value must not
        // echo; the unknown key name may (clipped), so it is kept short and
        // distinct from the value sentinel.
        format!(
            "version = {SESSION_STATE_VERSION}\n\n[[sessions]]\nkind = \"worktree\"\npath = \"{SENTINEL}\"\nsurplus = \"x\"\n"
        ),
        // Wrong-typed selected names the key only.
        format!(
            "version = {SESSION_STATE_VERSION}\nselected = \"{SENTINEL}\"\n\n[[sessions]]\nkind = \"local\"\n"
        ),
    ];
    for text in cases {
        let mut registry = SessionRegistry::new();
        let error = load_bytes(text.as_bytes(), &mut registry)
            .expect_err("every sentinel fixture must fail");
        assert!(registry.is_empty(), "a rejected document must load nothing");
        let display = error.to_string();
        assert!(
            !display.contains(SENTINEL),
            "Display must not echo session values: {display}"
        );
        let debug = format!("{error:?}");
        assert!(
            !debug.contains(SENTINEL),
            "Debug must not echo session values: {debug}"
        );
    }
}

/// Both renderings of a configuration error stay free of a value sentinel
/// outside the `[keys]` chord allowlist. Wrong-typed values and non-string
/// `[keys]` values name their key only.
#[test]
fn config_values_outside_keys_never_reach_display_or_debug() {
    const SENTINEL: &str = "VALUE-SENTINEL-not-a-setting";
    let cases = [
        format!("[font]\ncell_width = \"{SENTINEL}\"\n"),
        format!("[keys]\nsession_create = {SENTINEL}\n"),
        // `[[agents]]` values: the command is agent-launch text (may embed a
        // private path), so its rejections name the key, never the text.
        format!("[[agents]]\nname = \"x\"\ncommand = \"{SENTINEL}\"\n"),
        format!("[[agents]]\nname = \"{SENTINEL}\"\ncommand = 5\n"),
        format!("[[agents]]\nname = \"x\"\ncommand = \"/bin/true\"\nargs = [\"{SENTINEL}\", 1]\n"),
    ];
    for text in cases {
        let error = AppConfig::parse(&text).expect_err("every sentinel fixture must fail");
        let display = error.to_string();
        assert!(
            !display.contains(SENTINEL),
            "Display must not echo config values: {display}"
        );
        let debug = format!("{error:?}");
        assert!(
            !debug.contains(SENTINEL),
            "Debug must not echo config values: {debug}"
        );
    }
}

/// The chord allowlist is not vacuous: each allowlisted variant really does
/// echo the offending chord, which is the usability the contract protects.
/// Removing the echo must fail here, exactly as adding a new echo must
/// fail the classifier.
#[test]
fn allowlisted_chord_variants_still_echo_the_offending_chord() {
    // InvalidChord names the key, the value, and the failing token.
    let error = AppConfig::parse("[keys]\npalette_open = \"SENTINELTOKEN+p\"\n")
        .expect_err("an unparseable chord must fail");
    let display = error.to_string();
    assert!(
        display.contains("palette_open"),
        "the error must name the action key: {display}"
    );
    assert!(
        display.contains("SENTINELTOKEN+p"),
        "the error must show the offending value: {display}"
    );
    assert!(
        display.contains("SENTINELTOKEN"),
        "the reason must name the failing token: {display}"
    );

    // A hostile chord value stays bounded in the rendered message.
    let mut hostile = String::new();
    hostile.extend(std::iter::repeat_n('a', 10_000));
    let error = AppConfig::parse(&format!("[keys]\npalette_open = \"{hostile}\"\n"))
        .expect_err("a hostile chord value must fail");
    let display = error.to_string();
    assert!(
        display.chars().count() < 1024,
        "the rendered message must stay bounded: {} chars",
        display.chars().count()
    );

    // ReservedChord shows the dead binding.
    let error = AppConfig::parse("[keys]\nsession_create = \"escape\"\n")
        .expect_err("a palette UI key is a dead binding");
    assert!(
        error.to_string().contains("escape"),
        "ReservedChord must show the chord: {error}"
    );

    // UnclaimableChord shows the colliding chord.
    let error = AppConfig::parse("[keys]\npalette_open = \"super+escape\"\n")
        .expect_err("the frozen exit leader is unclaimable");
    assert!(
        error.to_string().contains("super+escape"),
        "UnclaimableChord must show the chord: {error}"
    );

    // DuplicateChord names both actions and the shared chord.
    let error = AppConfig::parse("[keys]\nsession_create = \"n\"\nsession_select = \"n\"\n")
        .expect_err("two actions on one chord");
    let display = error.to_string();
    assert!(
        display.contains("session_create") && display.contains("session_select"),
        "DuplicateChord must name both actions: {display}"
    );
    assert!(
        display.contains("chord n"),
        "DuplicateChord must show the shared chord: {display}"
    );
}

/// The theme-name allowlist is not vacuous either: an unknown theme echoes
/// the offending name (bounded, like every echo), while a wrong-typed value
/// names its key only.
#[test]
fn unknown_theme_echoes_the_offending_name_and_wrong_types_name_only_the_key() {
    let error =
        AppConfig::parse("[theme]\nname = \"sepia\"\n").expect_err("an unknown theme must fail");
    let display = error.to_string();
    assert!(
        display.contains("sepia"),
        "UnknownTheme must show the offending name: {display}"
    );
    assert!(
        display.contains("dark") && display.contains("light") && display.contains("high-contrast"),
        "UnknownTheme must name the accepted vocabulary: {display}"
    );

    // A hostile value stays bounded in the rendered message.
    let mut hostile = String::new();
    hostile.extend(std::iter::repeat_n('a', 10_000));
    let error = AppConfig::parse(&format!("[theme]\nname = \"{hostile}\"\n"))
        .expect_err("a hostile theme value must fail");
    assert!(
        error.to_string().chars().count() < 1024,
        "the rendered message must stay bounded: {} chars",
        error.to_string().chars().count()
    );

    // A wrong-typed value never echoes: only the key name appears.
    let error = AppConfig::parse("[theme]\nname = 12345\n")
        .expect_err("a non-string theme value must fail");
    let display = error.to_string();
    assert!(
        display.contains("name") && !display.contains("12345"),
        "ThemeNotAString must name the key and not the value: {display}"
    );
}

/// The ssh-config parser is under the same contract; one representative
/// probe here (its own suite covers the parser end to end).
#[test]
fn ssh_config_values_never_reach_display_or_debug() {
    const SENTINEL: &str = "VALUE-SENTINEL-not-a-port";
    let error = SshConfig::parse(&format!("Host prod\n  Port {SENTINEL}\n"))
        .expect_err("a non-numeric port must fail");
    let display = error.to_string();
    assert!(
        !display.contains(SENTINEL),
        "Display must not echo ssh config values: {display}"
    );
    let debug = format!("{error:?}");
    assert!(
        !debug.contains(SENTINEL),
        "Debug must not echo ssh config values: {debug}"
    );
}

/// The palette's derived `Debug` prints `label` and `CommandId`. Today that
/// is safe only because both are `&'static str`: a compile-time string
/// cannot hold file-derived text, so no config or session value can reach
/// a `Debug` line through the palette. That guarantee is load-bearing, so
/// this test fails to *compile* the moment `label` widens to a runtime
/// string — the leak then has to be added on top of a broken build, in the
/// open.
#[test]
fn palette_labels_and_ids_stay_compile_time_strings() {
    fn require_static_str(_: &'static str) {}

    let palette = Palette::noren((), (), (), ());
    assert_eq!(palette.len(), 4, "the canonical catalog is four commands");
    for command in palette.iter() {
        require_static_str(command.label());
        require_static_str(command.id().as_str());
    }

    // The derived Debug prints exactly the constant label set.
    let debug = format!("{palette:?}");
    for label in [
        "New Session",
        "Switch Session",
        "Close Session",
        "Focus Sidebar",
    ] {
        assert!(
            debug.contains(label),
            "palette Debug must carry the constant labels: {debug}"
        );
    }
}
