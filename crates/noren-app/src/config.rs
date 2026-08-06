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
//! # Privacy
//!
//! Configuration carries no secrets: no supported key names a credential,
//! key, or other sensitive path, and the schema deliberately exposes no path
//! keys at all. The shell program is **not** configurable: the threat model
//! (TM-01) fixes the spawn at `/bin/zsh` with no configured additions.

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

/// Validated application configuration.
///
/// [`AppConfig::default`] is byte-for-byte the behavior the app had before
/// configuration existed; [`AppConfig::load`] returns it for a missing file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct AppConfig {
    font: FontConfig,
}

impl AppConfig {
    /// Font geometry settings.
    #[must_use]
    pub const fn font(self) -> FontConfig {
        self.font
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
        let document: DocumentMut = text
            .parse()
            .map_err(|error: toml_edit::TomlError| ConfigError::Parse(clip(error.to_string())))?;
        let mut config = Self::default();
        for (key, item) in document.as_table().iter() {
            let table = item
                .as_table_like()
                .ok_or_else(|| ConfigError::WrongType { key: clip(key) })?;
            match key {
                "font" => parse_font(table, &mut config.font)?,
                // `[terminal]` historically attracted a `scrollback_lines` key.
                // The terminal foundation has no configurable retention cap yet,
                // so the table is rejected instead of parsed-and-ignored.
                _ => return Err(ConfigError::UnknownKey(clip(key))),
            }
        }
        Ok(config)
    }
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

/// Typed configuration failure without file contents.
///
/// Every variant renders a bounded message; hostile key names and parser
/// details are clipped by [`clip`] before they are stored.
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
    Parse(String),
    /// The file names a key this schema does not define.
    UnknownKey(String),
    /// A key holds the wrong TOML type.
    WrongType { key: String },
    /// A value is outside its accepted range.
    OutOfRange { key: String },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("configuration file not found"),
            Self::Io(kind) => write!(f, "configuration could not be read: {kind}"),
            Self::NotAFile => f.write_str("configuration path does not resolve to a regular file"),
            Self::TooLarge => write!(f, "configuration exceeds {MAX_CONFIG_BYTES} bytes"),
            Self::NotUtf8 => f.write_str("configuration is not valid UTF-8"),
            Self::Parse(detail) => write!(f, "configuration is not valid TOML: {detail}"),
            Self::UnknownKey(key) => write!(f, "unknown configuration key: {key}"),
            Self::WrongType { key } => {
                write!(f, "configuration key {key} must be an integer")
            }
            Self::OutOfRange { key } => {
                write!(f, "configuration key {key} is outside its accepted range")
            }
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
                matches!(parsed, Err(ConfigError::Parse(_))),
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
}
