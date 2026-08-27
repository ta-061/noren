//! Noren-owned process and PTY boundary for the macOS local-shell PoC.
//!
//! The public API deliberately exposes no `portable-pty` types. A session
//! launches the fixed `/bin/zsh` shell, the fixed system SSH client
//! (`/usr/bin/ssh`), or a configured agent command validated by
//! [`AgentLaunchPolicy`] — in every case without caller-controlled shell
//! interpretation or `-c`, moves blocking I/O off the UI thread, bounds
//! every queue and payload, and owns child termination and reaping in one
//! supervisor thread.
//!
//! The agent launch path is an argv vector, never a shell: the configured
//! program becomes `argv[0]`, each configured argument becomes exactly one
//! argv word, and the program must be an absolute path so no `PATH` lookup
//! can substitute a different binary. A value containing `;`, `$(...)`, or
//! a backtick is literal data to the agent program, because no shell ever
//! interprets it.
//!
//! The SSH launch path drives the system `ssh` binary only. Noren never
//! reimplements the SSH protocol and never passes a credential, key, or
//! password on the command line: argv is exactly `ssh -- <destination>`, and
//! authentication relies on ssh's own agent and configuration resolution. A
//! destination is accepted only through [`SshDestination`], whose typed
//! refusals carry no destination content.

use portable_pty::{CommandBuilder, PtySize as PortablePtySize, native_pty_system};
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read, Write};
use std::num::NonZeroU16;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Maximum bytes in one reader chunk or input command.
pub const READ_CHUNK_BYTES: usize = 16 * 1024;
/// Maximum buffered output chunks (one MiB at the maximum chunk size).
pub const OUTPUT_CHANNEL_CAPACITY: usize = 64;
/// Maximum buffered input, reply, resize, and close commands.
pub const COMMAND_CHANNEL_CAPACITY: usize = 256;
/// Maximum bytes in one terminal-generated reply.
pub const REPLY_BYTES_PER_MESSAGE: usize = 4 * 1024;
/// Maximum terminal-generated reply bytes accepted in one second.
pub const REPLY_BYTES_PER_SECOND: usize = 64 * 1024;
/// Total orderly-shutdown deadline.
pub const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(2);

const ZSH_PROGRAM: &str = "/bin/zsh";
/// Fixed system SSH client. An absolute path deliberately bypasses `PATH`
/// so a writable `PATH` entry cannot substitute a different binary.
const SSH_PROGRAM: &str = "/usr/bin/ssh";
/// End-of-options marker. Combined with the destination validation this makes
/// option injection through a hostile alias impossible, not just unlikely.
const SSH_END_OF_OPTIONS: &str = "--";
/// Maximum accepted SSH destination bytes before the launch is refused.
const MAX_SSH_DESTINATION_BYTES: usize = 1024;
const TERM_VALUE: &str = "xterm-256color";
const TERM_PROGRAM_VALUE: &str = "Noren-PoC";
const SUPERVISOR_POLL: Duration = Duration::from_millis(10);
const READER_JOIN_BUDGET: Duration = Duration::from_millis(1_750);
const LIFECYCLE_SEND_BUDGET: Duration = Duration::from_millis(100);
/// Maximum wait for the reader thread to report itself armed before the
/// child is forked. The report is a channel send executed as the reader's
/// first statement, so only total thread-startup failure can exceed it.
const READER_ARMED_BUDGET: Duration = Duration::from_secs(5);

/// Validated, non-zero terminal grid dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PtySize {
    rows: NonZeroU16,
    cols: NonZeroU16,
}

impl PtySize {
    /// Construct a size from already validated values.
    #[must_use]
    pub const fn new(rows: NonZeroU16, cols: NonZeroU16) -> Self {
        Self { rows, cols }
    }

    /// Construct a size while rejecting a zero row or column.
    #[must_use]
    pub const fn from_raw(rows: u16, cols: u16) -> Option<Self> {
        match (NonZeroU16::new(rows), NonZeroU16::new(cols)) {
            (Some(rows), Some(cols)) => Some(Self { rows, cols }),
            _ => None,
        }
    }

    /// Row count.
    #[must_use]
    pub const fn rows(self) -> u16 {
        self.rows.get()
    }

    /// Column count.
    #[must_use]
    pub const fn cols(self) -> u16 {
        self.cols.get()
    }

    /// Raw `(rows, columns)` pair.
    #[must_use]
    pub const fn into_raw(self) -> (u16, u16) {
        (self.rows.get(), self.cols.get())
    }

    fn portable(self) -> PortablePtySize {
        PortablePtySize {
            rows: self.rows(),
            cols: self.cols(),
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

/// Fixed local-zsh launch policy.
///
/// The home path is intentionally private and its `Debug` representation is
/// redacted. Callers can choose neither an executable nor arguments.
#[derive(Clone, PartialEq, Eq)]
pub struct ZshLaunchPolicy {
    home: PathBuf,
}

impl ZshLaunchPolicy {
    /// Read and validate the inherited `HOME` value.
    pub fn from_environment() -> Result<Self, PtyError> {
        validate_home(std::env::var_os("HOME"))
    }

    /// Return only non-sensitive, constant launch metadata.
    #[must_use]
    pub const fn metadata(&self) -> LaunchMetadata {
        LaunchMetadata {
            program: ZSH_PROGRAM,
            term: TERM_VALUE,
            term_program: TERM_PROGRAM_VALUE,
            removes_columns: true,
            removes_lines: true,
        }
    }
}

impl fmt::Debug for ZshLaunchPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ZshLaunchPolicy")
            .field("home", &"<redacted>")
            .finish()
    }
}

/// Fixed zsh-in-a-directory launch policy: like [`ZshLaunchPolicy`] but the
/// child's working directory is an arbitrary validated directory while
/// `HOME` is inherited unchanged from this process.
///
/// This is the launch shape a git-worktree session uses: the shell starts
/// *in* the worktree checkout (so `pwd`, build tools, and the prompt report
/// it) while the user's own shell configuration still applies. The directory
/// is intentionally private and its `Debug` representation is redacted: a
/// worktree path can embed a username or a private directory name.
#[derive(Clone, PartialEq, Eq)]
pub struct DirLaunchPolicy {
    dir: PathBuf,
}

impl DirLaunchPolicy {
    /// Validate `dir` as the child's working directory.
    ///
    /// Refuses a relative path ([`PtyError::CwdNotAbsolute`]) and a path
    /// that is not an existing directory ([`PtyError::CwdNotDirectory`]) —
    /// the latter is exactly the registered-but-deleted worktree case, so a
    /// stale worktree is refused with a typed error instead of a child that
    /// fails to spawn for an unstated reason.
    pub fn new(dir: &Path) -> Result<Self, PtyError> {
        if !dir.is_absolute() {
            return Err(PtyError::CwdNotAbsolute);
        }
        if !dir.is_dir() {
            return Err(PtyError::CwdNotDirectory);
        }
        Ok(Self {
            dir: dir.to_owned(),
        })
    }

    /// The validated working directory.
    ///
    /// This accessor exists to build diagnostics-free equality checks, not
    /// for display: [`fmt::Debug`] is redacted so a worktree path that
    /// embeds a username can never reach a log through a debug print.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl fmt::Debug for DirLaunchPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirLaunchPolicy")
            .field("dir", &"<redacted>")
            .finish()
    }
}

/// Validate an optional `HOME` without mutating process-global environment.
fn validate_home(home: Option<OsString>) -> Result<ZshLaunchPolicy, PtyError> {
    let home = PathBuf::from(home.ok_or(PtyError::MissingHome)?);
    if !home.is_absolute() {
        return Err(PtyError::HomeNotAbsolute);
    }
    if !home.is_dir() {
        return Err(PtyError::HomeNotDirectory);
    }
    Ok(ZshLaunchPolicy { home })
}

/// Safe inspection data for the fixed launch policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchMetadata {
    /// Fixed executable.
    pub program: &'static str,
    /// Fixed `TERM` override.
    pub term: &'static str,
    /// Fixed `TERM_PROGRAM` override.
    pub term_program: &'static str,
    /// Whether inherited `COLUMNS` is removed.
    pub removes_columns: bool,
    /// Whether inherited `LINES` is removed.
    pub removes_lines: bool,
}

fn build_zsh_command(policy: &ZshLaunchPolicy) -> CommandBuilder {
    let mut command = CommandBuilder::new(ZSH_PROGRAM);
    command.cwd(&policy.home);
    // Normal construction reads this exact value from the inherited HOME.
    // Re-applying it keeps cwd and child HOME consistent while allowing the
    // crate-local test harness to use an isolated directory without mutating
    // process-global environment.
    command.env("HOME", &policy.home);
    command.env("TERM", TERM_VALUE);
    command.env("TERM_PROGRAM", TERM_PROGRAM_VALUE);
    command.env_remove("COLUMNS");
    command.env_remove("LINES");
    command
}

/// Build the fixed zsh command for a directory-scoped launch. Security and
/// environment invariants, pinned by tests:
///
/// - argv is exactly `[ZSH_PROGRAM]` — no caller-controlled arguments, so no
///   `-c` command can ever appear in argv (`ps`-visible).
/// - the child's working directory is the policy's validated directory and
///   `HOME` is inherited unchanged: the user's own shell configuration still
///   applies, exactly like [`build_zsh_command`] when `HOME` and cwd agree.
fn build_dir_zsh_command(policy: &DirLaunchPolicy) -> CommandBuilder {
    let mut command = CommandBuilder::new(ZSH_PROGRAM);
    command.cwd(&policy.dir);
    command.env("TERM", TERM_VALUE);
    command.env("TERM_PROGRAM", TERM_PROGRAM_VALUE);
    command.env_remove("COLUMNS");
    command.env_remove("LINES");
    command
}

/// Build the fixed zsh command for a directory-scoped launch whose child
/// `HOME` is an explicitly supplied, validated home.
///
/// Identical to [`build_dir_zsh_command`] except `HOME` is set explicitly:
/// the test-harness sibling of the inherited-`HOME` production shape, so a
/// higher-level suite can point the child at an isolated empty home while
/// the working directory stays the policy's validated directory.
fn build_dir_zsh_command_with_home(
    policy: &DirLaunchPolicy,
    home: &ZshLaunchPolicy,
) -> CommandBuilder {
    let mut command = build_dir_zsh_command(policy);
    command.env("HOME", &home.home);
    command
}

/// An unexpanded OpenSSH percent token found in a destination.
///
/// OpenSSH expands these inside configuration keywords (never in the
/// destination argument), so a token that survived into the destination
/// string is by definition unexpanded. Each token names the keyword whose
/// value could have carried it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SshPercentToken {
    /// `%h`: the remote hostname, as written in a `HostName` value.
    Host,
    /// `%p`: the remote port, as written in a `Port`-derived value.
    Port,
    /// `%r`: the remote username, as written in a `User` value.
    RemoteUser,
}

impl SshPercentToken {
    /// The literal OpenSSH token spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Host => "%h",
            Self::Port => "%p",
            Self::RemoteUser => "%r",
        }
    }

    /// The configuration keyword whose value could have carried this token.
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Host => "HostName",
            Self::Port => "Port",
            Self::RemoteUser => "User",
        }
    }

    /// The first token `destination` contains, if any.
    ///
    /// The scan is deliberately conservative: it does not honour OpenSSH's
    /// `%%` escaping, because a destination that merely contains a token
    /// spelling has no legitimate origin in this application.
    fn first_in(destination: &str) -> Option<Self> {
        [Self::Host, Self::Port, Self::RemoteUser]
            .into_iter()
            .find(|token| destination.contains(token.as_str()))
    }
}

impl fmt::Display for SshPercentToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Typed refusal of a destination string, carrying no destination content.
///
/// Every message is a fixed string: the rejected bytes never appear in an
/// error, so a destination that happens to embed a secret cannot leak through
/// `Display`, `Debug`, or a log line that prints either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SshDestinationError {
    /// The destination was empty.
    Empty,
    /// The destination exceeded [`MAX_SSH_DESTINATION_BYTES`].
    Oversize,
    /// The destination began with `-`, so ssh could parse it as an option.
    LeadingHyphen,
    /// The destination contained an ASCII control character or whitespace.
    ControlOrWhitespace,
    /// The destination contained an unexpanded OpenSSH percent token. The
    /// connect must not proceed.
    RawToken {
        /// Which token was found.
        token: SshPercentToken,
    },
}

impl fmt::Display for SshDestinationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("SSH destination must not be empty"),
            Self::Oversize => f.write_str("SSH destination exceeds its byte limit"),
            Self::LeadingHyphen => f.write_str(
                "SSH destination must not begin with a hyphen; ssh would parse it as an option",
            ),
            Self::ControlOrWhitespace => {
                f.write_str("SSH destination must not contain control characters or whitespace")
            }
            Self::RawToken { token } => write!(
                f,
                "SSH destination contains the unexpanded OpenSSH token {} ({} keyword); \
                 the connect must not proceed",
                token.as_str(),
                token.keyword()
            ),
        }
    }
}

impl std::error::Error for SshDestinationError {}

/// A validated SSH destination (`ssh -- <destination>` final argument).
///
/// Validation is the argv-injection boundary, so it is deliberately stricter
/// than what OpenSSH would accept: the destination must be non-empty, at most
/// [`MAX_SSH_DESTINATION_BYTES`] bytes, free of ASCII control characters and
/// whitespace, must not begin with `-` (an alias like `-oProxyCommand=…`
/// parsed out of a hostile `Host` directive must never reach argv as an
/// option), and must not contain an unexpanded `%h`/`%p`/`%r` token (see
/// [`SshPercentToken`]). Combined with the fixed `--` end-of-options marker,
/// no accepted destination can be interpreted as an ssh option.
#[derive(Clone, PartialEq, Eq)]
pub struct SshDestination {
    destination: String,
}

impl SshDestination {
    /// Validate `destination` for the fixed ssh argv, or reject it with a
    /// typed, content-free error.
    pub fn new(destination: &str) -> Result<Self, SshDestinationError> {
        if destination.is_empty() {
            return Err(SshDestinationError::Empty);
        }
        if destination.len() > MAX_SSH_DESTINATION_BYTES {
            return Err(SshDestinationError::Oversize);
        }
        if let Some(token) = SshPercentToken::first_in(destination) {
            return Err(SshDestinationError::RawToken { token });
        }
        if destination.starts_with('-') {
            return Err(SshDestinationError::LeadingHyphen);
        }
        if destination
            .chars()
            .any(|c| c.is_ascii_control() || c.is_whitespace())
        {
            return Err(SshDestinationError::ControlOrWhitespace);
        }
        Ok(Self {
            destination: destination.to_owned(),
        })
    }

    /// The validated destination string.
    ///
    /// This accessor exists to build the child argv, not for display: the
    /// [`fmt::Debug`] implementation is redacted so a destination that embeds
    /// a secret can never reach a log through a debug print.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.destination
    }
}

impl fmt::Debug for SshDestination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The destination may embed a secret-shaped value and has no business
        // in any debug surface, so it is redacted exactly like the zsh
        // policy's home directory.
        f.debug_struct("SshDestination")
            .field("destination", &"<redacted>")
            .finish()
    }
}

/// Fixed system-ssh launch policy.
///
/// The child inherits the caller's environment unchanged (except the fixed
/// `TERM`/`TERM_PROGRAM` overrides below) so ssh performs its own agent, key,
/// and config resolution. There is deliberately no home-override seam like
/// [`ZshLaunchPolicy`]'s: OpenSSH resolves its per-user configuration through
/// the passwd database rather than `$HOME`, so an env override would not
/// isolate anything while suggesting it does.
#[derive(Clone, PartialEq, Eq)]
pub struct SshLaunchPolicy {
    destination: SshDestination,
}

impl SshLaunchPolicy {
    /// Launch the system ssh client to `destination`, inheriting HOME and the
    /// agent environment from this process.
    #[must_use]
    pub fn inherit(destination: SshDestination) -> Self {
        Self { destination }
    }

    /// The validated destination this policy connects to.
    #[must_use]
    pub fn destination(&self) -> &SshDestination {
        &self.destination
    }
}

impl fmt::Debug for SshLaunchPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshLaunchPolicy")
            .field("destination", &self.destination)
            .finish()
    }
}

/// Build the fixed ssh command. Security invariants, pinned by tests:
///
/// - argv is exactly `[SSH_PROGRAM, "--", destination]` — no options, so no
///   credential, identity, or command can ever appear in argv (`ps`-visible).
/// - `HOME` is not overridden (ssh needs the real agent/config resolution);
///   `COLUMNS`/`LINES` are dropped exactly like the zsh policy.
fn build_ssh_command(policy: &SshLaunchPolicy) -> CommandBuilder {
    let mut command = CommandBuilder::new(SSH_PROGRAM);
    command.arg(SSH_END_OF_OPTIONS);
    command.arg(policy.destination.as_str());
    command.env("TERM", TERM_VALUE);
    command.env("TERM_PROGRAM", TERM_PROGRAM_VALUE);
    command.env_remove("COLUMNS");
    command.env_remove("LINES");
    command
}

/// Validated argv launch policy for a configured agent command.
///
/// This is the third launch shape (after the fixed zsh and ssh policies):
/// the program comes from the user's own configuration instead of a
/// compile-time constant, so the validation that the fixed policies got for
/// free is explicit here:
///
/// - the command must be non-empty ([`PtyError::CommandEmpty`]) and
///   absolute with a leading `/` ([`PtyError::CommandNotAbsolute`]); no
///   `PATH` lookup is performed, so a writable `PATH` entry cannot
///   substitute a different binary;
/// - the launch is an **argv vector, never a shell invocation**: there is no
///   `sh -c` and no `-c` anywhere in the build, so a configured value
///   containing `;`, `$(...)`, or a backtick reaches the agent program as
///   literal data. Shell metacharacters cannot inject because no shell ever
///   interprets them.
///
/// The child runs in the inherited `HOME` (as cwd and `HOME`) like a local
/// session, with the same fixed `TERM`/`TERM_PROGRAM` surgery and
/// `COLUMNS`/`LINES` removal.
///
/// [`fmt::Debug`] is shape-only: a program path can embed a username or a
/// private directory name, so neither the program nor any argument is
/// printed (issue #146 discipline).
#[derive(Clone, PartialEq, Eq)]
pub struct AgentLaunchPolicy {
    program: String,
    args: Vec<String>,
}

impl AgentLaunchPolicy {
    /// Validate `program` and `args` as a shell-free argv vector.
    ///
    /// `program` must be non-empty and absolute; `args` are taken verbatim
    /// (each element is exactly one argv word; no quoting, splitting, or
    /// expansion is ever applied).
    pub fn new(program: &str, args: &[String]) -> Result<Self, PtyError> {
        if program.is_empty() {
            return Err(PtyError::CommandEmpty);
        }
        if !program.starts_with('/') {
            return Err(PtyError::CommandNotAbsolute);
        }
        Ok(Self {
            program: program.to_owned(),
            args: args.to_vec(),
        })
    }

    /// The validated program; `argv[0]` of the launch.
    ///
    /// This accessor exists to build the child argv, not for display: the
    /// [`fmt::Debug`] implementation is redacted so a private path can never
    /// reach a log through a debug print.
    #[must_use]
    pub fn program(&self) -> &str {
        &self.program
    }

    /// The validated argv words after the program.
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }
}

impl fmt::Debug for AgentLaunchPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentLaunchPolicy")
            .field("argv", &(self.args.len() + 1))
            .finish()
    }
}

/// Build the agent command from a validated policy. Security invariants,
/// pinned by tests:
///
/// - argv is exactly `[program, args...]` as supplied — no shell, no `-c`,
///   no extra words — so shell metacharacters in any configured value are
///   literal data to the agent program;
/// - the working directory and `HOME` are the inherited home (the same
///   treatment a local session gets), and the fixed `TERM`/`TERM_PROGRAM`
///   overrides plus `COLUMNS`/`LINES` removal apply like every policy.
fn build_agent_command(policy: &AgentLaunchPolicy, home: &ZshLaunchPolicy) -> CommandBuilder {
    let mut command = CommandBuilder::new(policy.program());
    command.cwd(&home.home);
    command.env("HOME", &home.home);
    for argument in policy.args() {
        command.arg(argument);
    }
    command.env("TERM", TERM_VALUE);
    command.env("TERM_PROGRAM", TERM_PROGRAM_VALUE);
    command.env_remove("COLUMNS");
    command.env_remove("LINES");
    command
}

/// PTY operations named by payload-free errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PtyOperation {
    Open,
    CloneReader,
    TakeWriter,
    SpawnChild,
    SpawnThread,
    Read,
    Write,
    Flush,
    Resize,
    ChildStatus,
    Kill,
    Reap,
}

/// Typed PTY failure without terminal, input, cwd, or environment contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PtyError {
    MissingHome,
    HomeNotAbsolute,
    HomeNotDirectory,
    CwdNotAbsolute,
    CwdNotDirectory,
    InvalidSize,
    InputTooLarge,
    ReplyTooLarge,
    ReplyRateExceeded,
    CommandQueueFull,
    ChannelDisconnected,
    SessionClosing,
    ReaderJoinTimeout,
    SupervisorJoinTimeout,
    /// An [`AgentLaunchPolicy`](crate::AgentLaunchPolicy) program was empty.
    CommandEmpty,
    /// An [`AgentLaunchPolicy`](crate::AgentLaunchPolicy) program was not an
    /// absolute path; `PATH` lookup is deliberately not performed.
    CommandNotAbsolute,
    Backend {
        operation: PtyOperation,
    },
    Io {
        operation: PtyOperation,
        kind: io::ErrorKind,
    },
}

impl PtyError {
    fn io(operation: PtyOperation, error: &io::Error) -> Self {
        Self::Io {
            operation,
            kind: error.kind(),
        }
    }
}

impl fmt::Display for PtyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHome => f.write_str("HOME is required"),
            Self::HomeNotAbsolute => f.write_str("HOME must be absolute"),
            Self::HomeNotDirectory => f.write_str("HOME must name an existing directory"),
            Self::CwdNotAbsolute => f.write_str("working directory must be absolute"),
            Self::CwdNotDirectory => {
                f.write_str("working directory must name an existing directory")
            }
            Self::InvalidSize => f.write_str("terminal size must be non-zero"),
            Self::InputTooLarge => f.write_str("PTY input message exceeds its byte limit"),
            Self::ReplyTooLarge => f.write_str("terminal reply exceeds its message byte limit"),
            Self::ReplyRateExceeded => f.write_str("terminal reply rate exceeds its byte limit"),
            Self::CommandQueueFull => f.write_str("PTY command queue is full"),
            Self::ChannelDisconnected => f.write_str("PTY channel disconnected"),
            Self::SessionClosing => f.write_str("PTY session is closing"),
            Self::ReaderJoinTimeout => f.write_str("PTY reader did not stop before the deadline"),
            Self::SupervisorJoinTimeout => {
                f.write_str("PTY supervisor did not stop before the deadline")
            }
            Self::CommandEmpty => f.write_str("agent command must not be empty"),
            Self::CommandNotAbsolute => {
                f.write_str("agent command must be an absolute path; PATH lookup is not performed")
            }
            Self::Backend { operation } => {
                write!(f, "PTY backend operation {operation:?} failed")
            }
            Self::Io { operation, kind } => {
                write!(f, "PTY operation {operation:?} failed with {kind:?}")
            }
        }
    }
}

impl std::error::Error for PtyError {}

/// Events delivered from the PTY workers to the application.
pub enum PtyEvent {
    /// Opaque, bounded PTY bytes.
    Output(Vec<u8>),
    /// The cloned reader observed EOF.
    Eof,
    /// The child was reaped.
    Exited { code: Option<u32> },
    /// A safe typed error.
    Error(PtyError),
}

impl fmt::Debug for PtyEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Output(bytes) => f
                .debug_struct("Output")
                .field("byte_count", &bytes.len())
                .finish(),
            Self::Eof => f.write_str("Eof"),
            Self::Exited { code } => f.debug_struct("Exited").field("code", code).finish(),
            Self::Error(error) => f.debug_tuple("Error").field(error).finish(),
        }
    }
}

enum SupervisorCommand {
    Input(Vec<u8>),
    Reply(Vec<u8>),
    Resize(PtySize),
    Close,
}

/// Running local-zsh PTY session.
///
/// The UI side is nonblocking: command methods use `try_send`, while
/// [`PtySession::try_recv`] drains ready output. [`PtySession::shutdown`] is
/// the only bounded wait and is idempotent.
pub struct PtySession {
    command_tx: SyncSender<SupervisorCommand>,
    event_rx: Receiver<PtyEvent>,
    done_rx: Receiver<Result<(), PtyError>>,
    closing: Arc<AtomicBool>,
    supervisor: Option<JoinHandle<()>>,
    finished: bool,
}

impl fmt::Debug for PtySession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PtySession")
            .field("closing", &self.closing.load(Ordering::Acquire))
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

impl PtySession {
    /// Spawn `/bin/zsh` at the supplied initial non-zero size.
    pub fn spawn(size: PtySize) -> Result<Self, PtyError> {
        Self::spawn_with_policy(ZshLaunchPolicy::from_environment()?, size)
    }

    /// Spawn the fixed system ssh client for `destination` at the supplied
    /// initial non-zero size, so the remote shell is interactive.
    ///
    /// argv is exactly `ssh -- <destination>` (see [`build_ssh_command`]);
    /// no credential is ever passed on the command line and authentication is
    /// left to ssh's own agent and configuration resolution.
    pub fn spawn_ssh(policy: SshLaunchPolicy, size: PtySize) -> Result<Self, PtyError> {
        Self::spawn_session(build_ssh_command(&policy), size)
    }

    /// Spawn a configured agent command at the supplied initial non-zero
    /// size, as a shell-free argv vector (see [`build_agent_command`]).
    ///
    /// argv is exactly `[program, args...]` from the validated policy — no
    /// `sh -c`, so metacharacters in configured values are literal data. A
    /// missing or non-executable program surfaces as the spawn error from
    /// the PTY backend ([`PtyError::Backend`] with
    /// [`PtyOperation::SpawnChild`]); the caller is expected to make that a
    /// visible failure, never a silent no-op. The child runs in the
    /// inherited `HOME` with the same environment surgery as a local
    /// session.
    pub fn spawn_agent(policy: AgentLaunchPolicy, size: PtySize) -> Result<Self, PtyError> {
        let home = ZshLaunchPolicy::from_environment()?;
        Self::spawn_session(build_agent_command(&policy, &home), size)
    }

    /// Spawn `/bin/zsh` with `home` as the child's `HOME` and working
    /// directory instead of the inherited one.
    ///
    /// The same fixed launch policy as [`PtySession::spawn`]: identical
    /// program, `TERM`, and environment surgery; only the home differs, and it
    /// is validated exactly like an inherited `HOME` (absolute, existing
    /// directory). This is the seam higher-level test suites use to run the
    /// child in an isolated directory: a developer's real `$HOME` may carry
    /// startup files that take arbitrarily long or read the terminal, which
    /// would make every shell-driving test depend on personal configuration.
    pub fn spawn_in_home(home: &Path, size: PtySize) -> Result<Self, PtyError> {
        let policy = validate_home(Some(home.as_os_str().to_owned()))?;
        Self::spawn_with_policy(policy, size)
    }

    /// Spawn `/bin/zsh` with `dir` as the child's working directory, leaving
    /// `HOME` inherited.
    ///
    /// The same fixed launch policy as [`PtySession::spawn`] — identical
    /// program, `TERM`, and environment surgery — except the working
    /// directory is the supplied directory (validated absolute and existing
    /// by [`DirLaunchPolicy::new`], which refuses a registered-but-deleted
    /// worktree with a typed error) and `HOME` is inherited unchanged so the
    /// user's own shell configuration applies. This is the launch shape a
    /// git-worktree session uses: the shell starts inside the worktree
    /// checkout.
    pub fn spawn_in_dir(dir: &Path, size: PtySize) -> Result<Self, PtyError> {
        let policy = DirLaunchPolicy::new(dir)?;
        Self::spawn_session(build_dir_zsh_command(&policy), size)
    }

    /// Spawn `/bin/zsh` with `dir` as the child's working directory and
    /// `home` as the child's `HOME`.
    ///
    /// Directory-scoped sibling of [`PtySession::spawn_in_home`]: production
    /// worktree launches use [`PtySession::spawn_in_dir`], which inherits
    /// `HOME` unchanged so the user's own shell configuration applies.
    /// Higher-level test suites that drive the child by typing use this seam
    /// to point `HOME` at an isolated empty directory — a developer's real
    /// `$HOME` may carry startup files that take arbitrarily long or read the
    /// terminal, which would make every shell-driving test depend on
    /// personal configuration — while the working directory remains the
    /// validated directory, so the child's actual cwd stays observable
    /// through its own `pwd` answer. Both paths are validated exactly like
    /// their standalone policies, and the fixed program, `TERM`, and
    /// environment surgery are identical to [`PtySession::spawn_in_dir`].
    pub fn spawn_in_dir_with_home(
        dir: &Path,
        home: &Path,
        size: PtySize,
    ) -> Result<Self, PtyError> {
        let dir_policy = DirLaunchPolicy::new(dir)?;
        let home_policy = validate_home(Some(home.as_os_str().to_owned()))?;
        Self::spawn_session(
            build_dir_zsh_command_with_home(&dir_policy, &home_policy),
            size,
        )
    }

    /// Spawn using an already validated fixed-zsh policy.
    fn spawn_with_policy(policy: ZshLaunchPolicy, size: PtySize) -> Result<Self, PtyError> {
        Self::spawn_session(build_zsh_command(&policy), size)
    }

    /// Common bounded supervisor wiring for every fixed launch policy.
    fn spawn_session(command: CommandBuilder, size: PtySize) -> Result<Self, PtyError> {
        let (command_tx, command_rx) = mpsc::sync_channel(COMMAND_CHANNEL_CAPACITY);
        let (event_tx, event_rx) = mpsc::sync_channel(OUTPUT_CHANNEL_CAPACITY);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        let closing = Arc::new(AtomicBool::new(false));
        let supervisor_closing = Arc::clone(&closing);

        let supervisor = thread::Builder::new()
            .name("noren-pty-supervisor".to_owned())
            .spawn(move || {
                supervisor_main(
                    command,
                    size,
                    command_rx,
                    event_tx,
                    ready_tx,
                    done_tx,
                    supervisor_closing,
                );
            })
            .map_err(|error| PtyError::io(PtyOperation::SpawnThread, &error))?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                command_tx,
                event_rx,
                done_rx,
                closing,
                supervisor: Some(supervisor),
                finished: false,
            }),
            Ok(Err(error)) => {
                let _ = supervisor.join();
                Err(error)
            }
            Err(_) => {
                let _ = supervisor.join();
                Err(PtyError::ChannelDisconnected)
            }
        }
    }

    /// Queue user input without blocking the UI thread.
    pub fn send_input(&self, bytes: &[u8]) -> Result<(), PtyError> {
        validate_input(bytes)?;
        self.send_command(SupervisorCommand::Input(bytes.to_vec()))
    }

    /// Queue an opaque terminal-generated reply without blocking the UI.
    pub fn send_reply(&self, bytes: &[u8]) -> Result<(), PtyError> {
        validate_reply(bytes)?;
        self.send_command(SupervisorCommand::Reply(bytes.to_vec()))
    }

    /// Queue one coalesced, non-zero PTY resize.
    pub fn resize(&self, size: PtySize) -> Result<(), PtyError> {
        self.send_command(SupervisorCommand::Resize(size))
    }

    /// Receive one ready event without blocking.
    pub fn try_recv(&self) -> Result<Option<PtyEvent>, PtyError> {
        match self.event_rx.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) if self.finished => Ok(None),
            Err(TryRecvError::Disconnected) => Err(PtyError::ChannelDisconnected),
        }
    }

    /// Stop accepting input and request an idempotent close.
    pub fn request_close(&self) {
        if !self.closing.swap(true, Ordering::AcqRel) {
            let _ = self.command_tx.try_send(SupervisorCommand::Close);
        }
    }

    /// Close, reap, and join within the documented deadline.
    pub fn shutdown(&mut self) -> Result<(), PtyError> {
        if self.finished {
            return Ok(());
        }
        self.request_close();
        let result = match self.done_rx.recv_timeout(SHUTDOWN_DEADLINE) {
            Ok(result) => {
                if let Some(supervisor) = self.supervisor.take() {
                    if supervisor.join().is_err() {
                        self.finished = true;
                        return Err(PtyError::ChannelDisconnected);
                    }
                }
                result
            }
            Err(RecvTimeoutError::Timeout) => {
                // Dropping the handle detaches the stuck supervisor. Mark the
                // session finished so `Drop` cannot wait for a second deadline.
                drop(self.supervisor.take());
                Err(PtyError::SupervisorJoinTimeout)
            }
            Err(RecvTimeoutError::Disconnected) => {
                if let Some(supervisor) = self.supervisor.take() {
                    let _ = supervisor.join();
                }
                Err(PtyError::ChannelDisconnected)
            }
        };
        self.finished = true;
        result
    }

    fn send_command(&self, command: SupervisorCommand) -> Result<(), PtyError> {
        if self.closing.load(Ordering::Acquire) {
            return Err(PtyError::SessionClosing);
        }
        match self.command_tx.try_send(command) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(PtyError::CommandQueueFull),
            Err(TrySendError::Disconnected(_)) => Err(PtyError::ChannelDisconnected),
        }
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn validate_input(bytes: &[u8]) -> Result<(), PtyError> {
    if bytes.len() > READ_CHUNK_BYTES {
        Err(PtyError::InputTooLarge)
    } else {
        Ok(())
    }
}

fn validate_reply(bytes: &[u8]) -> Result<(), PtyError> {
    if bytes.len() > REPLY_BYTES_PER_MESSAGE {
        Err(PtyError::ReplyTooLarge)
    } else {
        Ok(())
    }
}

fn supervisor_main(
    command: CommandBuilder,
    size: PtySize,
    command_rx: Receiver<SupervisorCommand>,
    event_tx: SyncSender<PtyEvent>,
    ready_tx: SyncSender<Result<(), PtyError>>,
    done_tx: SyncSender<Result<(), PtyError>>,
    closing: Arc<AtomicBool>,
) {
    let setup = setup_pty(command, size, &event_tx, Arc::clone(&closing));
    let (master, writer, mut child, reader, reader_done) = match setup {
        Ok(parts) => {
            let _ = ready_tx.send(Ok(()));
            parts
        }
        Err(error) => {
            let _ = ready_tx.send(Err(error));
            let _ = done_tx.send(Err(error));
            return;
        }
    };

    let mut writer = Some(writer);
    let mut child_exited = false;
    let mut first_error = None;
    let mut replies = ReplyWindow::new();

    while !closing.load(Ordering::Acquire) {
        match command_rx.recv_timeout(SUPERVISOR_POLL) {
            Ok(SupervisorCommand::Input(bytes)) => {
                if let Err(error) = write_bytes(writer.as_mut(), &bytes) {
                    send_lifecycle(&event_tx, PtyEvent::Error(error));
                    first_error.get_or_insert(error);
                    break;
                }
            }
            Ok(SupervisorCommand::Reply(bytes)) => {
                if let Err(error) = replies.accept(bytes.len()) {
                    send_lifecycle(&event_tx, PtyEvent::Error(error));
                } else if let Err(error) = write_bytes(writer.as_mut(), &bytes) {
                    send_lifecycle(&event_tx, PtyEvent::Error(error));
                    first_error.get_or_insert(error);
                    break;
                }
            }
            Ok(SupervisorCommand::Resize(size)) => {
                if master.resize(size.portable()).is_err() {
                    let error = PtyError::Backend {
                        operation: PtyOperation::Resize,
                    };
                    send_lifecycle(&event_tx, PtyEvent::Error(error));
                    first_error.get_or_insert(error);
                    break;
                }
            }
            Ok(SupervisorCommand::Close) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                child_exited = true;
                send_lifecycle(
                    &event_tx,
                    PtyEvent::Exited {
                        code: Some(status.exit_code()),
                    },
                );
                break;
            }
            Ok(None) => {}
            Err(error) => {
                let error = PtyError::io(PtyOperation::ChildStatus, &error);
                send_lifecycle(&event_tx, PtyEvent::Error(error));
                first_error.get_or_insert(error);
                break;
            }
        }
    }

    closing.store(true, Ordering::Release);
    drop(writer.take());

    if !child_exited {
        match child.try_wait() {
            Ok(Some(status)) => {
                send_lifecycle(
                    &event_tx,
                    PtyEvent::Exited {
                        code: Some(status.exit_code()),
                    },
                );
            }
            Ok(None) => {
                if let Err(error) = child.kill() {
                    let error = PtyError::io(PtyOperation::Kill, &error);
                    send_lifecycle(&event_tx, PtyEvent::Error(error));
                    first_error.get_or_insert(error);
                }
                match child.wait() {
                    Ok(status) => send_lifecycle(
                        &event_tx,
                        PtyEvent::Exited {
                            code: Some(status.exit_code()),
                        },
                    ),
                    Err(error) => {
                        let error = PtyError::io(PtyOperation::Reap, &error);
                        send_lifecycle(&event_tx, PtyEvent::Error(error));
                        first_error.get_or_insert(error);
                    }
                }
            }
            Err(error) => {
                let error = PtyError::io(PtyOperation::ChildStatus, &error);
                send_lifecycle(&event_tx, PtyEvent::Error(error));
                first_error.get_or_insert(error);
            }
        }
    }

    drop(master);
    let reader_result = match reader_done.recv_timeout(READER_JOIN_BUDGET) {
        Ok(()) => {
            if reader.join().is_err() {
                Err(PtyError::ChannelDisconnected)
            } else {
                Ok(())
            }
        }
        Err(_) => {
            drop(reader);
            Err(PtyError::ReaderJoinTimeout)
        }
    };

    let result = match reader_result {
        Err(error) => Err(error),
        Ok(()) => first_error.map_or(Ok(()), Err),
    };
    if let Err(error) = result {
        send_lifecycle(&event_tx, PtyEvent::Error(error));
    }
    let _ = done_tx.send(result);
}

type PtyParts = (
    Box<dyn portable_pty::MasterPty + Send>,
    Box<dyn Write + Send>,
    Box<dyn portable_pty::Child + Send + Sync>,
    JoinHandle<()>,
    Receiver<()>,
);

fn setup_pty(
    command: CommandBuilder,
    size: PtySize,
    event_tx: &SyncSender<PtyEvent>,
    closing: Arc<AtomicBool>,
) -> Result<PtyParts, PtyError> {
    let pair = native_pty_system()
        .openpty(size.portable())
        .map_err(|_| PtyError::Backend {
            operation: PtyOperation::Open,
        })?;
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|_| PtyError::Backend {
            operation: PtyOperation::CloneReader,
        })?;
    let writer = pair.master.take_writer().map_err(|_| PtyError::Backend {
        operation: PtyOperation::TakeWriter,
    })?;

    // The reader must be parked in `read` BEFORE the child is forked.
    // macOS discards any unread slave-to-master output at the moment the
    // last slave-side descriptor closes: if a short-lived child writes and
    // exits before the reader's first `read` is pending, that output is
    // flushed and only EOF is observed (established with a raw openpty
    // probe: a first read starting after the child was reaped lost the
    // marker in 200/200 rounds). The rendezvous below waits for the
    // reader's armed report, so the child's writes always land in a
    // pending read and the flush at child exit can no longer destroy
    // undelivered output. A failure to arm is a typed spawn error; the
    // master and writer drop here, which unblocks and ends the reader.
    let (reader_done_tx, reader_done_rx) = mpsc::sync_channel(1);
    let (armed_tx, armed_rx) = mpsc::sync_channel(1);
    let reader_events = event_tx.clone();
    let reader_thread = thread::Builder::new()
        .name("noren-pty-reader".to_owned())
        .spawn(move || reader_main(reader, reader_events, reader_done_tx, armed_tx, closing))
        .map_err(|error| PtyError::io(PtyOperation::SpawnThread, &error))?;
    armed_rx
        .recv_timeout(READER_ARMED_BUDGET)
        .map_err(|_| PtyError::Backend {
            operation: PtyOperation::SpawnThread,
        })?;

    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|_| PtyError::Backend {
            operation: PtyOperation::SpawnChild,
        })?;
    // Not the last slave-side descriptor: the child holds its stdio, so
    // this close cannot flush anything the parked reader has not read.
    drop(pair.slave);

    Ok((pair.master, writer, child, reader_thread, reader_done_rx))
}

fn write_bytes(writer: Option<&mut Box<dyn Write + Send>>, bytes: &[u8]) -> Result<(), PtyError> {
    let writer = writer.ok_or(PtyError::SessionClosing)?;
    writer
        .write_all(bytes)
        .map_err(|error| PtyError::io(PtyOperation::Write, &error))?;
    writer
        .flush()
        .map_err(|error| PtyError::io(PtyOperation::Flush, &error))
}

fn reader_main(
    mut reader: Box<dyn Read + Send>,
    event_tx: SyncSender<PtyEvent>,
    done_tx: SyncSender<()>,
    armed_tx: SyncSender<()>,
    closing: Arc<AtomicBool>,
) {
    // Armed report: the reader is scheduled and entering its first `read`.
    // setup_pty waits for this before forking the child, so a pending read
    // always exists by the time the child can write (see setup_pty).
    let _ = armed_tx.send(());
    let mut buffer = vec![0_u8; READ_CHUNK_BYTES];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => {
                send_lifecycle(&event_tx, PtyEvent::Eof);
                break;
            }
            Ok(count) => {
                if !send_output(&event_tx, buffer[..count].to_vec(), &closing) {
                    break;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) if closing.load(Ordering::Acquire) => {
                send_lifecycle(&event_tx, PtyEvent::Eof);
                break;
            }
            Err(error) => {
                send_lifecycle(
                    &event_tx,
                    PtyEvent::Error(PtyError::io(PtyOperation::Read, &error)),
                );
                break;
            }
        }
    }
    let _ = done_tx.send(());
}

fn send_output(event_tx: &SyncSender<PtyEvent>, bytes: Vec<u8>, closing: &AtomicBool) -> bool {
    let mut event = PtyEvent::Output(bytes);
    loop {
        match event_tx.try_send(event) {
            Ok(()) => return true,
            Err(TrySendError::Full(returned)) => {
                if closing.load(Ordering::Acquire) {
                    return false;
                }
                event = returned;
                thread::sleep(Duration::from_millis(2));
            }
            Err(TrySendError::Disconnected(_)) => return false,
        }
    }
}

fn send_lifecycle(event_tx: &SyncSender<PtyEvent>, event: PtyEvent) {
    let deadline = Instant::now() + LIFECYCLE_SEND_BUDGET;
    let mut event = event;
    loop {
        match event_tx.try_send(event) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => return,
            Err(TrySendError::Full(returned)) => {
                if Instant::now() >= deadline {
                    return;
                }
                event = returned;
                thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

struct ReplyWindow {
    started: Instant,
    bytes: usize,
}

impl ReplyWindow {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            bytes: 0,
        }
    }

    fn accept(&mut self, count: usize) -> Result<(), PtyError> {
        if self.started.elapsed() >= Duration::from_secs(1) {
            self.started = Instant::now();
            self.bytes = 0;
        }
        let next = self.bytes.saturating_add(count);
        if next > REPLY_BYTES_PER_SECOND {
            return Err(PtyError::ReplyRateExceeded);
        }
        self.bytes = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn temp_directory() -> PathBuf {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("noren-pty-test-{}-{sequence}", std::process::id()));
        fs::create_dir(&path).expect("create test directory");
        path
    }

    /// A uniquely named directory whose name embeds `fragment`, for redaction
    /// checks: any debug surface that prints the path prints the fragment.
    fn temp_directory_with(fragment: &str) -> PathBuf {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "noren-pty-test-{}-{sequence}-{fragment}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create test directory");
        path
    }

    #[cfg(target_os = "macos")]
    struct TestHome(PathBuf);

    #[cfg(target_os = "macos")]
    impl TestHome {
        fn new() -> Self {
            Self(temp_directory())
        }

        fn policy(&self) -> ZshLaunchPolicy {
            validate_home(Some(self.0.clone().into_os_string())).expect("valid isolated home")
        }
    }

    #[cfg(target_os = "macos")]
    impl Drop for TestHome {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove isolated home");
        }
    }

    #[cfg(target_os = "macos")]
    fn test_session(home: &TestHome) -> PtySession {
        let size = PtySize::from_raw(24, 80).expect("valid initial size");
        PtySession::spawn_with_policy(home.policy(), size).expect("spawn fixed zsh")
    }

    #[cfg(target_os = "macos")]
    fn poll_events(
        session: &PtySession,
        deadline: Instant,
        output: &mut Vec<u8>,
        lifecycle: &mut bool,
        done: impl Fn(&[u8], bool) -> bool,
    ) {
        poll_events_with_desc(session, deadline, output, lifecycle, "", done)
    }

    #[cfg(target_os = "macos")]
    fn poll_events_with_desc(
        session: &PtySession,
        deadline: Instant,
        output: &mut Vec<u8>,
        lifecycle: &mut bool,
        desc: &str,
        done: impl Fn(&[u8], bool) -> bool,
    ) {
        let start = Instant::now();
        while Instant::now() < deadline {
            match session.try_recv().expect("receive PTY event") {
                Some(PtyEvent::Output(bytes)) => output.extend(bytes),
                Some(PtyEvent::Eof | PtyEvent::Exited { .. }) => *lifecycle = true,
                Some(PtyEvent::Error(error)) => panic!("unexpected typed PTY error: {error}"),
                None => thread::sleep(Duration::from_millis(1)),
            }
            if done(output, *lifecycle) {
                return;
            }
        }
        let elapsed = start.elapsed();
        let stripped = strip_ansi(output);
        panic!(
            "PTY event polling deadline expired after {elapsed:?}\n\
             {desc}expected condition not met\n\
             raw output ({} bytes): {:?}\n\
             stripped output: {:?}\n\
             lifecycle={lifecycle}",
            output.len(),
            String::from_utf8_lossy(output),
            String::from_utf8_lossy(&stripped),
        );
    }

    #[cfg(target_os = "macos")]
    fn occurrences(haystack: &[u8], needle: &[u8]) -> usize {
        haystack
            .windows(needle.len())
            .filter(|window| *window == needle)
            .count()
    }

    /// Strip ANSI escape sequences from raw PTY output for human-readable
    /// diagnostics.
    #[cfg(target_os = "macos")]
    fn strip_ansi(bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                while i < bytes.len() && !bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        out
    }

    /// Mutation M4 (restoring `#[derive(Debug)]`) must expose the numeric byte
    /// list and fail here without relying on a production log statement.
    #[test]
    fn pty_event_debug_reports_output_shape_without_bytes() {
        const SECRET: &[u8] = b"NOREN-PTY-DBG-S3CR3T";

        let output_debug = format!("{:?}", PtyEvent::Output(SECRET.to_vec()));
        assert_eq!(
            output_debug,
            format!("Output {{ byte_count: {} }}", SECRET.len())
        );
        assert!(
            !output_debug.contains(&format!("{SECRET:?}")),
            "PTY output bytes leaked: {output_debug}"
        );

        assert_eq!(format!("{:?}", PtyEvent::Eof), "Eof");
        assert_eq!(
            format!("{:?}", PtyEvent::Exited { code: Some(17) }),
            "Exited { code: Some(17) }"
        );
        assert_eq!(
            format!("{:?}", PtyEvent::Error(PtyError::InvalidSize)),
            "Error(InvalidSize)"
        );
    }

    #[test]
    fn nonzero_size_round_trips_and_zero_is_rejected() {
        let size = PtySize::from_raw(24, 80).expect("valid size");
        assert_eq!(size.into_raw(), (24, 80));
        assert!(PtySize::from_raw(0, 80).is_none());
        assert!(PtySize::from_raw(24, 0).is_none());
    }

    #[test]
    fn home_validation_is_pure_and_requires_an_absolute_directory() {
        assert_eq!(validate_home(None), Err(PtyError::MissingHome));
        assert_eq!(
            validate_home(Some(OsString::from("relative"))),
            Err(PtyError::HomeNotAbsolute)
        );
        let missing = std::env::temp_dir().join("noren-definitely-missing-home");
        assert_eq!(
            validate_home(Some(missing.into_os_string())),
            Err(PtyError::HomeNotDirectory)
        );

        let directory = temp_directory();
        let policy = validate_home(Some(directory.clone().into_os_string())).expect("valid home");
        assert_eq!(
            policy.metadata(),
            LaunchMetadata {
                program: "/bin/zsh",
                term: "xterm-256color",
                term_program: "Noren-PoC",
                removes_columns: true,
                removes_lines: true,
            }
        );
        assert!(format!("{policy:?}").contains("<redacted>"));
        assert!(!format!("{policy:?}").contains(&directory.display().to_string()));
        fs::remove_dir(directory).expect("remove test directory");
    }

    #[test]
    fn policy_builder_is_fixed_and_has_no_caller_arguments() {
        let directory = temp_directory();
        let policy = validate_home(Some(directory.clone().into_os_string())).expect("valid home");
        let command = build_zsh_command(&policy);
        assert_eq!(command.get_argv(), &[OsString::from(ZSH_PROGRAM)]);
        assert_eq!(
            command.get_cwd().map(OsString::as_os_str),
            Some(directory.as_os_str())
        );
        assert_eq!(
            command.get_env("TERM"),
            Some(std::ffi::OsStr::new(TERM_VALUE))
        );
        assert_eq!(
            command.get_env("TERM_PROGRAM"),
            Some(std::ffi::OsStr::new(TERM_PROGRAM_VALUE))
        );
        assert_eq!(command.get_env("HOME"), Some(directory.as_os_str()));
        assert_eq!(command.get_env("COLUMNS"), None);
        assert_eq!(command.get_env("LINES"), None);
        fs::remove_dir(directory).expect("remove test directory");
    }

    #[test]
    fn ssh_destination_accepts_plain_targets_and_rejects_injection_classes() {
        assert!(SshDestination::new("web1.example").is_ok());
        assert!(SshDestination::new("user@10.0.0.9").is_ok());
        assert!(SshDestination::new(&"a".repeat(MAX_SSH_DESTINATION_BYTES)).is_ok());

        assert_eq!(SshDestination::new(""), Err(SshDestinationError::Empty));
        assert_eq!(
            SshDestination::new(&"a".repeat(MAX_SSH_DESTINATION_BYTES + 1)),
            Err(SshDestinationError::Oversize)
        );
        assert_eq!(
            SshDestination::new("-oProxyCommand=evil"),
            Err(SshDestinationError::LeadingHyphen)
        );
        for rejected in [
            "space here",
            "tab\there",
            "nl\nhere",
            "nul\0here",
            "esc\u{1b}here",
        ] {
            assert_eq!(
                SshDestination::new(rejected),
                Err(SshDestinationError::ControlOrWhitespace),
                "control/whitespace destination must be rejected: {rejected:?}"
            );
        }
    }

    #[test]
    fn ssh_destination_rejects_every_unexpanded_percent_token_by_keyword() {
        assert_eq!(
            SshDestination::new("user@%h.example"),
            Err(SshDestinationError::RawToken {
                token: SshPercentToken::Host
            })
        );
        assert_eq!(
            SshDestination::new("host.example:%p"),
            Err(SshDestinationError::RawToken {
                token: SshPercentToken::Port
            })
        );
        assert_eq!(
            SshDestination::new("%r@host.example"),
            Err(SshDestinationError::RawToken {
                token: SshPercentToken::RemoteUser
            })
        );
    }

    #[test]
    fn ssh_destination_error_messages_name_keyword_and_token_without_content() {
        const SECRET: &str = "hunter2-token";

        for (destination, token) in [
            (format!("{SECRET}@%h.example"), SshPercentToken::Host),
            (format!("host.example:%p-{SECRET}"), SshPercentToken::Port),
            (
                format!("%r-{SECRET}@host.example"),
                SshPercentToken::RemoteUser,
            ),
        ] {
            let error = SshDestination::new(&destination).expect_err("token-bearing destination");
            let text = error.to_string();
            assert!(
                text.contains(token.as_str()),
                "the message must name the token: {text}"
            );
            assert!(
                text.contains(token.keyword()),
                "the message must name the keyword: {text}"
            );
            assert!(
                !text.contains(SECRET),
                "the message must never carry destination content: {text}"
            );
            assert!(!format!("{error:?}").contains(SECRET));
        }

        for (destination, expected) in [
            ("", SshDestinationError::Empty),
            (
                &"a".repeat(MAX_SSH_DESTINATION_BYTES + 1),
                SshDestinationError::Oversize,
            ),
            ("-Wevil", SshDestinationError::LeadingHyphen),
            ("a b", SshDestinationError::ControlOrWhitespace),
        ] {
            let error = SshDestination::new(destination).expect_err("rejected destination");
            assert_eq!(error, expected);
            assert!(error.to_string().starts_with("SSH destination"));
            assert!(!error.to_string().contains("a".repeat(8).as_str()));
        }
    }

    #[test]
    fn ssh_destination_and_policy_debug_never_carry_the_destination() {
        const SECRET: &str = "noren-debug-secret-9f21";

        let destination =
            SshDestination::new(&format!("{SECRET}@example.com")).expect("secret-shaped target");
        assert!(!format!("{destination:?}").contains(SECRET));
        assert!(format!("{destination:?}").contains("<redacted>"));

        let policy = SshLaunchPolicy::inherit(destination);
        let inspected = format!("{policy:?}");
        assert!(
            !inspected.contains(SECRET),
            "debug must be redacted: {inspected}"
        );
        assert!(inspected.contains("<redacted>"));
    }

    #[test]
    fn ssh_command_argv_is_exactly_the_fixed_program_marker_and_destination() {
        let destination = SshDestination::new("deploy@web1.example").expect("valid destination");
        let policy = SshLaunchPolicy::inherit(destination);
        let command = build_ssh_command(&policy);

        assert_eq!(
            command.get_argv(),
            &[
                OsString::from(SSH_PROGRAM),
                OsString::from(SSH_END_OF_OPTIONS),
                OsString::from("deploy@web1.example"),
            ]
        );
        assert!(
            command.get_cwd().is_none(),
            "the ssh launch must not pin a working directory"
        );
        assert_eq!(
            command.get_env("TERM"),
            Some(std::ffi::OsStr::new(TERM_VALUE))
        );
        assert_eq!(
            command.get_env("TERM_PROGRAM"),
            Some(std::ffi::OsStr::new(TERM_PROGRAM_VALUE))
        );
        assert_eq!(command.get_env("COLUMNS"), None);
        assert_eq!(command.get_env("LINES"), None);
        // The child's HOME is the caller's HOME, byte for byte: ssh's own
        // agent and per-user configuration resolution must be inherited, not
        // redirected.
        assert_eq!(
            command.get_env("HOME"),
            std::env::var_os("HOME").as_deref(),
            "the ssh launch must inherit HOME unchanged"
        );
    }

    #[test]
    fn ssh_command_argv_carries_no_credential_shaped_option() {
        // Whatever the destination looks like, argv length is fixed at three
        // and no argument can be interpreted as an option: the marker ends
        // option parsing and the destination is the final argument.
        for target in ["plain", "user@host", "host:2222", "equals=x", "quotes'q"] {
            let destination = SshDestination::new(target).expect("valid destination");
            let command = build_ssh_command(&SshLaunchPolicy::inherit(destination));
            let argv = command.get_argv();
            assert_eq!(argv.len(), 3, "argv must stay fixed for {target:?}");
            assert_eq!(argv[0], OsString::from(SSH_PROGRAM));
            assert_eq!(argv[1], OsString::from(SSH_END_OF_OPTIONS));
            assert_eq!(argv[2], OsString::from(target));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn spawn_ssh_drives_the_system_client_and_surfaces_a_fast_failure() {
        // Drive the real /usr/bin/ssh through the production argv. A
        // destination beginning with `@` is rejected by ssh's own argument
        // parsing before any name resolution or connection, so the process
        // exits 255 with its usage diagnostic in milliseconds — deterministically,
        // with no network access and no credential. The test observes the real
        // fixed argv reaching the real binary and the failure flowing through
        // the normal PTY output path.
        const SSH_ARG_FAILURE: &str = "@noren-usage-refusal";

        let destination = SshDestination::new(SSH_ARG_FAILURE).expect("valid destination");
        let size = PtySize::from_raw(24, 80).expect("valid initial size");
        let mut session = PtySession::spawn_ssh(SshLaunchPolicy::inherit(destination), size)
            .expect("spawn the fixed system ssh client");

        let mut output = Vec::new();
        let mut saw_eof = false;
        let mut exit_code: Option<u32> = None;
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && exit_code.is_none() {
            match session.try_recv().expect("receive PTY event") {
                Some(PtyEvent::Output(bytes)) => output.extend_from_slice(&bytes),
                Some(PtyEvent::Eof) => saw_eof = true,
                Some(PtyEvent::Exited { code }) => exit_code = code,
                Some(PtyEvent::Error(error)) => panic!("unexpected typed PTY error: {error}"),
                None => thread::sleep(Duration::from_millis(2)),
            }
        }

        let code =
            exit_code.unwrap_or_else(|| {
                panic!(
                    "the system ssh client must terminate after its usage refusal; eof={saw_eof} output={}",
                    String::from_utf8_lossy(&output)
                )
            });
        assert_eq!(
            code, 255,
            "ssh reports its own argument-parsing failure as its error exit"
        );
        assert!(
            output.windows(b"usage:".len()).any(|w| w == b"usage:"),
            "the ssh client's own diagnostic must flow through the normal PTY output path"
        );
        assert!(
            saw_eof,
            "the reader must observe EOF after the child terminates"
        );
        session.shutdown().expect("bounded shutdown after ssh exit");
        session.shutdown().expect("shutdown remains idempotent");
    }

    #[test]
    fn agent_launch_policy_validates_and_redacts_its_debug() {
        assert_eq!(AgentLaunchPolicy::new("", &[]), Err(PtyError::CommandEmpty));
        assert_eq!(
            AgentLaunchPolicy::new("claude", &[]),
            Err(PtyError::CommandNotAbsolute)
        );

        // The policy's own Debug prints shape only: a program path can embed
        // a private directory, and an argument can carry anything.
        const SECRET: &str = "NOREN-AGENT-hunter2";
        let policy = AgentLaunchPolicy::new(
            &format!("/opt/{SECRET}/agent"),
            &[format!("--token={SECRET}"), format!("{SECRET};rm")],
        )
        .expect("absolute program with metacharacter args is valid data");
        let rendered = format!("{policy:?}");
        assert!(
            !rendered.contains(SECRET),
            "AgentLaunchPolicy Debug leaked argv text: {rendered}"
        );
        assert!(
            rendered.contains("argv: 3"),
            "the shape-only Debug names the argv length: {rendered}"
        );
        // Accessors expose the real values for argv construction.
        assert_eq!(policy.program(), format!("/opt/{SECRET}/agent"));
        assert_eq!(policy.args().len(), 2);
    }

    /// The agent command is the configured argv VERBATIM: no shell, no `-c`,
    /// no word splitting or re-quoting. Metacharacters survive as literal
    /// argv words, which is exactly why they cannot inject.
    #[test]
    fn agent_command_argv_is_exactly_the_configured_vector() {
        let home = temp_directory();
        let policy = validate_home(Some(home.clone().into_os_string())).expect("valid home");
        let agent = AgentLaunchPolicy::new(
            "/usr/local/bin/agent-cli",
            &[
                "--login".to_owned(),
                "; rm -rf /".to_owned(),
                "$(whoami)".to_owned(),
                "`id`".to_owned(),
            ],
        )
        .expect("validated policy");
        let command = build_agent_command(&agent, &policy);
        let argv: Vec<OsString> = command.get_argv().to_vec();
        assert_eq!(
            argv,
            vec![
                OsString::from("/usr/local/bin/agent-cli"),
                OsString::from("--login"),
                OsString::from("; rm -rf /"),
                OsString::from("$(whoami)"),
                OsString::from("`id`"),
            ],
            "argv must be the configured vector with no interpretation"
        );
        assert_eq!(argv.len(), 5, "no shell word was added or split");
        // The child's environment is the local-session shape: cwd/HOME are
        // the home, TERM is fixed, COLUMNS/LINES are removed.
        assert_eq!(
            command.get_cwd().map(OsString::as_os_str),
            Some(home.as_os_str())
        );
        assert_eq!(command.get_env("HOME"), Some(home.as_os_str()));
        assert_eq!(
            command.get_env("TERM"),
            Some(std::ffi::OsStr::new(TERM_VALUE))
        );
        assert_eq!(command.get_env("COLUMNS"), None);
        assert_eq!(command.get_env("LINES"), None);
        fs::remove_dir(home).expect("remove test directory");
    }

    /// A real configured agent command actually runs: /bin/echo's own
    /// output, read back through the PTY, is the evidence the child started
    /// — never an inference from the code path. Its clean exit code 0 is
    /// observed through the normal reaping path.
    #[cfg(target_os = "macos")]
    #[test]
    fn spawn_agent_runs_the_configured_command_to_a_verified_exit() {
        let marker = format!("NOREN_AGENT_ECHO_{}", std::process::id());
        let policy = AgentLaunchPolicy::new("/bin/echo", &[marker.clone()])
            .expect("absolute /bin/echo with one literal argument");
        let size = PtySize::from_raw(24, 80).expect("valid initial size");
        let mut session =
            PtySession::spawn_agent(policy, size).expect("spawn the configured agent command");

        let mut output = Vec::new();
        let mut lifecycle = false;
        poll_events(
            &session,
            Instant::now() + Duration::from_secs(10),
            &mut output,
            &mut lifecycle,
            |bytes, done| done && bytes.windows(marker.len()).any(|w| w == marker.as_bytes()),
        );
        assert!(
            output.windows(marker.len()).any(|w| w == marker.as_bytes()),
            "the child's own output must flow through the PTY: {}",
            String::from_utf8_lossy(&output)
        );
        session
            .shutdown()
            .expect("bounded shutdown after agent exit");
        session.shutdown().expect("shutdown remains idempotent");
    }

    /// A command that does not exist is a typed spawn failure at the seam,
    /// not a hang and not a silently created session.
    #[cfg(target_os = "macos")]
    #[test]
    fn spawn_agent_with_a_missing_program_fails_typed() {
        let missing = format!("/noren-definitely-missing-agent-{}", std::process::id());
        let policy =
            AgentLaunchPolicy::new(&missing, &[]).expect("absolute but nonexistent program");
        let size = PtySize::from_raw(24, 80).expect("valid initial size");
        let result = PtySession::spawn_agent(policy, size);
        assert!(
            matches!(
                result,
                Err(PtyError::Backend {
                    operation: PtyOperation::SpawnChild
                })
            ),
            "a missing program must fail the spawn, got {result:?}"
        );
    }

    #[test]
    fn payload_limits_are_exact() {
        assert_eq!(validate_input(&vec![0; READ_CHUNK_BYTES]), Ok(()));
        assert_eq!(
            validate_input(&vec![0; READ_CHUNK_BYTES + 1]),
            Err(PtyError::InputTooLarge)
        );
        assert_eq!(validate_reply(&vec![0; REPLY_BYTES_PER_MESSAGE]), Ok(()));
        assert_eq!(
            validate_reply(&vec![0; REPLY_BYTES_PER_MESSAGE + 1]),
            Err(PtyError::ReplyTooLarge)
        );
    }

    #[test]
    fn reply_window_rejects_excess_without_logging_payload() {
        let mut window = ReplyWindow::new();
        for _ in 0..(REPLY_BYTES_PER_SECOND / REPLY_BYTES_PER_MESSAGE) {
            assert_eq!(window.accept(REPLY_BYTES_PER_MESSAGE), Ok(()));
        }
        assert_eq!(window.accept(1), Err(PtyError::ReplyRateExceeded));
    }

    #[test]
    fn errors_have_no_source_or_sensitive_values() {
        let error = PtyError::Io {
            operation: PtyOperation::SpawnChild,
            kind: io::ErrorKind::PermissionDenied,
        };
        assert!(error.source().is_none());
        assert_eq!(
            error.to_string(),
            "PTY operation SpawnChild failed with PermissionDenied"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn fixed_zsh_round_trips_partial_ascii_and_split_utf8_then_reaps() {
        const MARKER: &[u8] = b"NOREN_PTY_PARTIAL_7f3a:\xe2\x98\x83\r\n";

        let home = TestHome::new();
        let mut session = test_session(&home);
        session.send_input(b"stty -echo\n").expect("disable echo");
        session
            .send_input(b"printf 'NOREN_PTY_PART")
            .expect("send partial command");
        session
            .send_input(b"IAL_7f3a:\xe2")
            .expect("send first UTF-8 byte");
        session
            .send_input(b"\x98\x83\\n'\nexit\n")
            .expect("complete UTF-8 and exit");

        let mut output = Vec::new();
        let mut lifecycle = false;
        poll_events(
            &session,
            Instant::now() + Duration::from_secs(2),
            &mut output,
            &mut lifecycle,
            |bytes, _| occurrences(bytes, MARKER) == 1,
        );
        assert_eq!(occurrences(&output, MARKER), 1);
        poll_events(
            &session,
            Instant::now() + Duration::from_secs(2),
            &mut output,
            &mut lifecycle,
            |_, observed| observed,
        );
        session.shutdown().expect("reap fixed zsh");
        session.shutdown().expect("shutdown remains idempotent");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ctrl_d_reaches_eof_or_exit_and_shutdown_remains_bounded() {
        let home = TestHome::new();
        let mut session = test_session(&home);
        session.send_input(&[0x04]).expect("send Ctrl-D");

        let mut output = Vec::new();
        let mut lifecycle = false;
        poll_events(
            &session,
            Instant::now() + Duration::from_secs(2),
            &mut output,
            &mut lifecycle,
            |_, observed| observed,
        );

        let started = Instant::now();
        session.shutdown().expect("reap zsh after Ctrl-D");
        assert!(started.elapsed() <= SHUTDOWN_DEADLINE);
        session.shutdown().expect("shutdown remains idempotent");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn live_resize_duplicate_and_storm_leave_last_size_authoritative() {
        const FINAL_SIZE: &[u8] = b"37 113\r\n";

        let home = TestHome::new();
        let mut session = test_session(&home);
        session.send_input(b"stty -echo\n").expect("disable echo");
        for (rows, cols) in [(31, 97), (31, 97), (32, 101), (35, 107), (37, 113)] {
            session
                .resize(PtySize::from_raw(rows, cols).expect("nonzero storm size"))
                .expect("queue resize");
        }
        session
            .send_input(b"/bin/stty size\nexit\n")
            .expect("query kernel PTY size");

        let mut output = Vec::new();
        let mut lifecycle = false;
        poll_events(
            &session,
            Instant::now() + Duration::from_secs(2),
            &mut output,
            &mut lifecycle,
            |bytes, _| occurrences(bytes, FINAL_SIZE) == 1,
        );
        assert_eq!(occurrences(&output, FINAL_SIZE), 1);
        session.shutdown().expect("reap resized zsh");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn output_pressure_close_has_one_bounded_shutdown() {
        let home = TestHome::new();
        let mut session = test_session(&home);
        session.send_input(b"stty -echo\n").expect("disable echo");
        session
            .send_input(b"while true; do print -r -- 0123456789abcdef0123456789abcdef; done\n")
            .expect("start builtin output loop");

        let mut output = Vec::new();
        let mut lifecycle = false;
        poll_events(
            &session,
            Instant::now() + Duration::from_secs(2),
            &mut output,
            &mut lifecycle,
            |bytes, _| bytes.len() >= READ_CHUNK_BYTES,
        );
        thread::sleep(Duration::from_millis(100));
        session.request_close();
        // Assert what the name claims — one bounded shutdown — on outcomes
        // and counts rather than elapsed time (issue #159).  The session's
        // own internal SHUTDOWN_DEADLINE is the hang-catcher: a supervisor
        // that loops or never finishes surfaces as Err(SupervisorJoinTimeout)
        // and a stuck reader as Err(ReaderJoinTimeout), so demanding the
        // orderly outcome fails deterministically on either.  (The previous
        // form accepted both errors and re-derived the same deadline from
        // wall-clock time, which raced the internal one at the margin.)
        let result = session.shutdown();
        assert_eq!(
            result,
            Ok(()),
            "shutdown must complete orderly within its own deadline"
        );

        // Bounded drain: shutdown() joins the supervisor before returning, so
        // no producer remains and the event channel yields at most the
        // OUTPUT_CHANNEL_CAPACITY chunks ever queued, then ends.
        let mut drained = 0usize;
        while let Some(_event) = session.try_recv().expect("drain PTY events") {
            drained += 1;
            assert!(
                drained <= OUTPUT_CHANNEL_CAPACITY,
                "drained {drained} events after shutdown; at most \
                 {OUTPUT_CHANNEL_CAPACITY} can be queued"
            );
        }

        // One shutdown suffices: the second call returns immediately via the
        // finished flag instead of waiting a second deadline.
        session.shutdown().expect("second shutdown is a no-op");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn spawn_in_home_uses_the_isolated_home_and_validates_it() {
        // The public seam must reject exactly what the inherited-home path
        // rejects, without reading or mutating the process environment.
        let missing = std::env::temp_dir().join("noren-pty-missing-spawn-in-home");
        assert!(matches!(
            PtySession::spawn_in_home(&missing, PtySize::from_raw(24, 80).expect("size")),
            Err(PtyError::HomeNotDirectory)
        ));

        let home = TestHome::new();
        let size = PtySize::from_raw(24, 80).expect("valid initial size");
        let mut session = PtySession::spawn_in_home(&home.0, size).expect("spawn in home");
        // The isolated home carries no startup files, so the shell answers
        // immediately; a real `$HOME` with slow startup files could not.
        session
            .send_input(b"print -r -- SPAWN_IN_HOME_OK\nexit\n")
            .expect("drive shell");
        let mut output = Vec::new();
        let mut lifecycle = false;
        poll_events(
            &session,
            Instant::now() + Duration::from_secs(2),
            &mut output,
            &mut lifecycle,
            |_, observed| observed,
        );
        // Echo is on (this test never disables it), so the marker appears both
        // as the echoed input line and as the printed output; at least one
        // occurrence proves the isolated shell answered.
        assert!(occurrences(&output, b"SPAWN_IN_HOME_OK") >= 1);
        session.shutdown().expect("reap isolated-home zsh");
    }

    #[test]
    fn dir_policy_validation_refuses_relative_and_missing_directories() {
        // A relative path is refused before the filesystem is touched.
        assert_eq!(
            DirLaunchPolicy::new(Path::new("relative/worktree")),
            Err(PtyError::CwdNotAbsolute)
        );
        // The registered-but-deleted worktree case: a typed refusal, never a
        // child that fails to spawn for an unstated reason (and never a
        // panic).
        let missing = std::env::temp_dir().join("noren-pty-missing-worktree-dir");
        assert_eq!(
            DirLaunchPolicy::new(&missing),
            Err(PtyError::CwdNotDirectory)
        );

        let directory = temp_directory();
        let policy = DirLaunchPolicy::new(&directory).expect("valid directory");
        assert_eq!(policy.dir(), directory.as_path());
        fs::remove_dir(&directory).expect("remove test directory");
    }

    #[test]
    fn dir_policy_builder_pins_cwd_and_leaves_home_inherited() {
        let directory = temp_directory();
        let policy = DirLaunchPolicy::new(&directory).expect("valid directory");
        let command = build_dir_zsh_command(&policy);
        assert_eq!(command.get_argv(), &[OsString::from(ZSH_PROGRAM)]);
        assert_eq!(
            command.get_cwd().map(OsString::as_os_str),
            Some(directory.as_os_str()),
            "the child's working directory is the policy directory"
        );
        assert_eq!(
            command.get_env("TERM"),
            Some(std::ffi::OsStr::new(TERM_VALUE))
        );
        assert_eq!(
            command.get_env("TERM_PROGRAM"),
            Some(std::ffi::OsStr::new(TERM_PROGRAM_VALUE))
        );
        // HOME is inherited unchanged: a directory-scoped session is still
        // the user's shell, with the user's own configuration.
        assert_eq!(
            command.get_env("HOME"),
            std::env::var_os("HOME").as_deref(),
            "the directory launch must inherit HOME unchanged"
        );
        assert_eq!(command.get_env("COLUMNS"), None);
        assert_eq!(command.get_env("LINES"), None);
        fs::remove_dir(&directory).expect("remove test directory");
    }

    #[test]
    fn dir_with_home_builder_pins_cwd_and_the_seam_home() {
        // The seam's directory and home are distinct directories, so an
        // assertion against the wrong one cannot pass by coincidence.
        let directory = temp_directory_with("seam-dir");
        let home = temp_directory_with("seam-home");
        let dir_policy = DirLaunchPolicy::new(&directory).expect("valid directory");
        let home_policy =
            validate_home(Some(home.clone().into_os_string())).expect("valid seam home");
        let command = build_dir_zsh_command_with_home(&dir_policy, &home_policy);

        assert_eq!(command.get_argv(), &[OsString::from(ZSH_PROGRAM)]);
        assert_eq!(
            command.get_cwd().map(OsString::as_os_str),
            Some(directory.as_os_str()),
            "the seam keeps the working directory on the policy directory"
        );
        assert_eq!(
            command.get_env("HOME"),
            Some(home.as_os_str()),
            "the seam sets HOME to the isolated home, never the inherited one"
        );
        assert_eq!(
            command.get_env("TERM"),
            Some(std::ffi::OsStr::new(TERM_VALUE))
        );
        assert_eq!(
            command.get_env("TERM_PROGRAM"),
            Some(std::ffi::OsStr::new(TERM_PROGRAM_VALUE))
        );
        assert_eq!(command.get_env("COLUMNS"), None);
        assert_eq!(command.get_env("LINES"), None);
        fs::remove_dir(&directory).expect("remove test directory");
        fs::remove_dir(&home).expect("remove test home");
    }

    #[test]
    fn dir_policy_debug_never_carries_the_directory() {
        const SECRET: &str = "noren-debug-secret-wt-4c31";
        let directory = temp_directory_with(SECRET);
        let policy = DirLaunchPolicy::new(&directory).expect("valid directory");
        let inspected = format!("{policy:?}");
        assert!(
            !inspected.contains(SECRET),
            "debug must be redacted: {inspected}"
        );
        assert!(inspected.contains("<redacted>"));
        fs::remove_dir(&directory).expect("remove test directory");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn spawn_in_dir_runs_the_child_in_that_directory() {
        let worktree = temp_directory_with("worktree-cwd");
        // The home must DIFFER from the requested cwd (issue #162).
        // `portable-pty` falls back to the child's `HOME` when a launch
        // carries no cwd, so a home that equalled the worktree let a
        // dropped `command.cwd` pass unnoticed: the fallback landed in
        // the very directory the test demanded. A distinct home keeps the
        // discrimination the test's name claims while preserving #156's
        // isolation — a controlled empty HOME, never the developer's
        // real one, whose startup files (oh-my-zsh, conda, nvm, …) could
        // outlast the deadline or read the terminal.
        let home = temp_directory_with("worktree-home");

        // The production entry `spawn_in_dir` inherits HOME unchanged so the
        // user's shell config applies; this live check drives the
        // `spawn_in_dir_with_home` seam — the same `DirLaunchPolicy`, the
        // same fixed builder, and the same session machinery, with HOME
        // pinned to the empty fixture directory — so the shell finds no
        // startup files and answers immediately. Unlike an env swap, the
        // seam never mutates process-global environment, which races the
        // parallel tests that read HOME (this exact race was observed:
        // `ssh_command_argv_...` failed while HOME was swapped).
        //
        // Mutation check: dropping `command.cwd` from the builder makes the
        // child land in the home fixture instead, and the pwd assertion
        // below fails — the fallback can no longer mask a missing cwd.
        let size = PtySize::from_raw(24, 80).expect("valid initial size");
        let mut session = PtySession::spawn_in_dir_with_home(&worktree, &home, size)
            .expect("spawn zsh in the worktree");

        session.send_input(b"pwd\nexit\n").expect("ask for the cwd");

        // macOS resolves /var to /private/var, and the shell reports the
        // canonical form; compare against the canonicalized fixture path.
        let canonical = fs::canonicalize(&worktree).expect("canonicalize the fixture path");
        let canonical_bytes = canonical.as_os_str().as_encoded_bytes().to_vec();
        let mut output = Vec::new();
        let mut lifecycle = false;
        let desc = format!("worktree={worktree:?} canonical={canonical:?}");
        poll_events_with_desc(
            &session,
            Instant::now() + Duration::from_secs(10),
            &mut output,
            &mut lifecycle,
            &desc,
            |bytes, _| occurrences(bytes, &canonical_bytes) >= 1,
        );
        assert!(
            occurrences(&output, &canonical_bytes) >= 1,
            "the child's own pwd must report the worktree directory"
        );
        session.shutdown().expect("reap worktree zsh");
        fs::remove_dir_all(&worktree).expect("remove worktree fixture");
        fs::remove_dir_all(&home).expect("remove home fixture");
    }
}
