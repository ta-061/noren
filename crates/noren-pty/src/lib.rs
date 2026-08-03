//! Noren-owned process and PTY boundary for the macOS local-shell PoC.
//!
//! The public API deliberately exposes no `portable-pty` types. A session
//! always launches `/bin/zsh` without caller-controlled arguments or `-c`,
//! moves blocking I/O off the UI thread, bounds every queue and payload, and
//! owns child termination and reaping in one supervisor thread.

use portable_pty::{CommandBuilder, PtySize as PortablePtySize, native_pty_system};
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read, Write};
use std::num::NonZeroU16;
use std::path::PathBuf;
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
const TERM_VALUE: &str = "xterm-256color";
const TERM_PROGRAM_VALUE: &str = "Noren-PoC";
const SUPERVISOR_POLL: Duration = Duration::from_millis(10);
const READER_JOIN_BUDGET: Duration = Duration::from_millis(1_750);
const LIFECYCLE_SEND_BUDGET: Duration = Duration::from_millis(100);

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
    InvalidSize,
    InputTooLarge,
    ReplyTooLarge,
    ReplyRateExceeded,
    CommandQueueFull,
    ChannelDisconnected,
    SessionClosing,
    ReaderJoinTimeout,
    SupervisorJoinTimeout,
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
#[derive(Debug)]
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

    /// Spawn using an already validated fixed-zsh policy.
    fn spawn_with_policy(policy: ZshLaunchPolicy, size: PtySize) -> Result<Self, PtyError> {
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
                    policy,
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
    policy: ZshLaunchPolicy,
    size: PtySize,
    command_rx: Receiver<SupervisorCommand>,
    event_tx: SyncSender<PtyEvent>,
    ready_tx: SyncSender<Result<(), PtyError>>,
    done_tx: SyncSender<Result<(), PtyError>>,
    closing: Arc<AtomicBool>,
) {
    let setup = setup_pty(&policy, size, &event_tx, Arc::clone(&closing));
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
    policy: &ZshLaunchPolicy,
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
    let command = build_zsh_command(policy);
    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|_| PtyError::Backend {
            operation: PtyOperation::SpawnChild,
        })?;
    drop(pair.slave);

    let (reader_done_tx, reader_done_rx) = mpsc::sync_channel(1);
    let reader_events = event_tx.clone();
    let reader_thread = match thread::Builder::new()
        .name("noren-pty-reader".to_owned())
        .spawn(move || reader_main(reader, reader_events, reader_done_tx, closing))
    {
        Ok(thread) => thread,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(PtyError::io(PtyOperation::SpawnThread, &error));
        }
    };

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
    closing: Arc<AtomicBool>,
) {
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
        panic!("PTY event polling deadline expired");
    }

    #[cfg(target_os = "macos")]
    fn occurrences(haystack: &[u8], needle: &[u8]) -> usize {
        haystack
            .windows(needle.len())
            .filter(|window| *window == needle)
            .count()
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
        let started = Instant::now();
        let result = session.shutdown();
        assert!(started.elapsed() <= SHUTDOWN_DEADLINE);
        assert!(matches!(
            result,
            Ok(()) | Err(PtyError::ReaderJoinTimeout | PtyError::SupervisorJoinTimeout)
        ));
    }
}
