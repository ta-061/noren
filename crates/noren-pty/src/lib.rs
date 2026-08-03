//! Noren-owned process/PTY boundary contracts for the macOS local-PTY PoC.
//!
//! This crate owns structured spawn, master I/O, resize, status, and teardown
//! behind the `PtyBackend` contract defined by the
//! [minimum architecture](https://github.com/ta-061/noren/blob/main/docs/architecture/minimal-local-pty-poc.md).
//!
//! This first baseline defines only the typed seams: a validated non-zero
//! [`PtySize`] and the placeholder [`PtyCommand`], [`PtyEvent`], and
//! [`PtyError`] shapes. Spawning, the supervisor/reader threads, resize
//! propagation, and child reaping land in a later step behind the same
//! contract. `portable-pty` 0.9.0 is declared as the exact trial candidate and
//! is compiled on the target, but no PTY is opened here.

use std::ffi::OsString;
use std::fmt;
use std::num::NonZeroU16;
use std::path::PathBuf;

/// Validated, non-zero terminal grid dimensions.
///
/// Rows and columns are guaranteed non-zero at construction. A zero-sized
/// window must never reach a PTY; [`PtySize::new`] and [`PtySize::from_raw`]
/// reject zero so the coalescing rule in the architecture cannot produce a zero
/// resize on the spawn or resize paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PtySize {
    rows: NonZeroU16,
    cols: NonZeroU16,
}

impl PtySize {
    /// Create a size from already-validated non-zero rows and columns.
    #[must_use]
    pub const fn new(rows: NonZeroU16, cols: NonZeroU16) -> Self {
        Self { rows, cols }
    }

    /// Create a size from raw dimensions, rejecting any zero side.
    ///
    /// Returns `None` when either `rows` or `cols` is zero. The coalescing
    /// layer is expected to retain the last valid grid instead of sending a
    /// zero dimension; this is the defensive boundary that enforces it.
    #[must_use]
    pub const fn from_raw(rows: u16, cols: u16) -> Option<Self> {
        match (NonZeroU16::new(rows), NonZeroU16::new(cols)) {
            (Some(rows), Some(cols)) => Some(Self { rows, cols }),
            _ => None,
        }
    }

    /// Number of rows, always non-zero.
    #[must_use]
    pub const fn rows(self) -> u16 {
        self.rows.get()
    }

    /// Number of columns, always non-zero.
    #[must_use]
    pub const fn cols(self) -> u16 {
        self.cols.get()
    }

    /// Raw `(rows, cols)` pair.
    #[must_use]
    pub const fn into_raw(self) -> (u16, u16) {
        (self.rows.get(), self.cols.get())
    }
}

/// Structured spawn request.
///
/// PoC policy fixes the executable to `/bin/zsh` with no `-c`; the full spawn
/// policy (validated absolute `$HOME` cwd, inherited environment with
/// `TERM=xterm-256color` / `TERM_PROGRAM=Noren-PoC` overrides, and
/// `COLUMNS` / `LINES` removal) is enforced by the future `PtyBackend`. This
/// baseline only carries the typed shape so later wiring has a stable target.
/// No process is spawned by this type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PtyCommand {
    program: PathBuf,
    args: Vec<OsString>,
    cwd: PathBuf,
}

impl PtyCommand {
    /// Start a structured command for `program` with `cwd`.
    ///
    /// Arguments stay structured; the future supervisor never concatenates a
    /// caller-supplied command string.
    #[must_use]
    pub fn new(program: impl Into<PathBuf>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: cwd.into(),
        }
    }

    /// Append one structured argument.
    #[must_use]
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Program path that will be executed.
    #[must_use]
    pub fn program(&self) -> &std::path::Path {
        &self.program
    }

    /// Structured argv after the program.
    #[must_use]
    pub fn args(&self) -> &[OsString] {
        &self.args
    }

    /// Working directory for the child.
    #[must_use]
    pub fn cwd(&self) -> &std::path::Path {
        &self.cwd
    }
}

/// Events emitted by a PTY supervisor to the application main loop.
///
/// Output bytes are opaque and non-authoritative: the app forwards them to a
/// terminal engine and never interprets them as commands. This baseline defines
/// the contract; the supervisor and reader thread land in a later step. The
/// enum intentionally does not implement `Clone`/`PartialEq` because the error
/// variant carries an `std::io::Error`.
#[derive(Debug)]
pub enum PtyEvent {
    /// A bounded chunk of PTY output bytes. Non-authoritative.
    Output(Vec<u8>),
    /// The PTY reader observed EOF on the master.
    Eof,
    /// The child process exited. `code` is the raw wait status when known.
    Exited { code: Option<i32> },
    /// A typed PTY error.
    Error(PtyError),
}

/// Typed PTY errors.
///
/// Component errors carry a component, operation, and safe status; they never
/// embed terminal contents, environment values, or input bytes.
#[derive(Debug)]
pub enum PtyError {
    /// A non-zero terminal size was required but a side was zero.
    InvalidSize,
    /// Spawn failed before a child existed.
    Spawn,
    /// Blocking PTY I/O failed.
    Io(std::io::Error),
}

impl fmt::Display for PtyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSize => f.write_str("terminal size must be non-zero"),
            Self::Spawn => f.write_str("PTY spawn failed"),
            Self::Io(err) => write!(f, "PTY I/O failed: {err}"),
        }
    }
}

impl std::error::Error for PtyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;
    use std::num::NonZeroU16;

    fn nz(value: u16) -> NonZeroU16 {
        NonZeroU16::new(value).expect("non-zero")
    }

    #[test]
    fn nonzero_size_round_trips() {
        let size = PtySize::new(nz(24), nz(80));
        assert_eq!(size.rows(), 24);
        assert_eq!(size.cols(), 80);
        assert_eq!(size.into_raw(), (24, 80));
    }

    #[test]
    fn from_raw_rejects_any_zero_side() {
        assert!(PtySize::from_raw(0, 0).is_none());
        assert!(PtySize::from_raw(0, 80).is_none());
        assert!(PtySize::from_raw(24, 0).is_none());
        assert_eq!(
            PtySize::from_raw(24, 80).map(PtySize::into_raw),
            Some((24, 80))
        );
    }

    #[test]
    fn command_keeps_arguments_structured() {
        let home = std::env::temp_dir();
        let command = PtyCommand::new("/bin/zsh", home.as_path())
            .arg("-l")
            .arg("--login");
        assert_eq!(command.program(), std::path::Path::new("/bin/zsh"));
        assert_eq!(command.cwd(), home.as_path());
        assert_eq!(
            command.args(),
            [OsString::from("-l"), OsString::from("--login")]
        );
    }

    #[test]
    fn events_carry_payloads_without_interpreting_them() {
        let output = PtyEvent::Output(vec![b'a', b'b']);
        if let PtyEvent::Output(bytes) = &output {
            assert_eq!(bytes, &vec![b'a', b'b']);
        } else {
            panic!("expected Output event");
        }

        assert!(matches!(PtyEvent::Eof, PtyEvent::Eof));
        assert!(matches!(
            PtyEvent::Exited { code: Some(0) },
            PtyEvent::Exited { code: Some(0) }
        ));
        assert!(matches!(
            PtyEvent::Exited { code: None },
            PtyEvent::Exited { code: None }
        ));
        assert!(matches!(
            PtyEvent::Error(PtyError::Spawn),
            PtyEvent::Error(_)
        ));
    }

    #[test]
    fn invalid_size_error_displays_safely() {
        let error = PtyError::InvalidSize;
        assert_eq!(error.to_string(), "terminal size must be non-zero");
        assert!(error.source().is_none());
    }

    #[test]
    fn io_error_surfaces_source_without_payload() {
        let error = PtyError::Io(std::io::Error::other("boom"));
        assert!(error.source().is_some());
        assert!(error.to_string().contains("PTY I/O failed"));
    }

    /// Candidate PTY library seam: the validated non-zero size maps to the
    /// `portable-pty` size type with zero pixel dimensions. The PoC has no font
    /// yet, so pixel size stays zero. This proves the exact candidate links and
    /// matches the documented spawn-path mapping without opening a PTY.
    #[test]
    fn portable_pty_size_mapping_is_nonzero() {
        let size = PtySize::new(nz(24), nz(80));
        let raw = portable_pty::PtySize {
            rows: size.rows(),
            cols: size.cols(),
            pixel_width: 0,
            pixel_height: 0,
        };
        assert_eq!(raw.rows, 24);
        assert_eq!(raw.cols, 80);
        assert_eq!(raw.pixel_width, 0);
        assert_eq!(raw.pixel_height, 0);
    }
}
