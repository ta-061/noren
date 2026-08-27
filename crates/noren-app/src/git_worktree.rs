//! Bounded, read-only discovery of the git worktrees of the repository Noren
//! was launched in.
//!
//! This is a workspace-filesystem adapter in the same position as
//! [`crate::ssh_config`]: it produces bounded sidebar facts and never launches
//! a session. Launching a worktree session is the application's job, through
//! the shared [`crate::session`] vocabulary.
//!
//! # Source of truth
//!
//! Discovery shells out to `git worktree list --porcelain` (the stable,
//! machine-readable form; the human format is never scraped) with the child's
//! working directory set to the launch directory, then parses the output with
//! a pure, total parser ([`parse_worktree_porcelain`]). Nothing else about git
//! is invoked: no status, no fetch, no network.
//!
//! # Honest about the filesystem
//!
//! A worktree can be registered while its directory has been deleted from
//! disk. That is common (a `rm -rf` of a scratch checkout leaves the
//! registration behind until `git worktree prune`) and is handled as data:
//! the row is discovered, [`DiscoveredWorktree::directory_present`] reports
//! `false`, and nothing panics or blocks. A detached HEAD is likewise data:
//! [`DiscoveredWorktree::branch_display`] is `"(detached)"`.
//!
//! # Bounded
//!
//! A repository can have very many worktrees. Discovery keeps at most
//! [`MAX_WORKTREE_SIDEBAR_ROWS`] rows and reports the omitted count (see
//! [`WorktreeDiscovery::omitted`]), exactly like the bounded SSH host list.
//!
//! # Secrets
//!
//! A worktree path can contain a username or a private directory name, and a
//! branch name is user-derived text. Every type in this module therefore has a
//! shape-only [`std::fmt::Debug`] and every error message is a fixed string:
//! no path, branch, or git output byte is ever printed through `Debug`,
//! `Display`, or a diagnostic.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Maximum worktree rows retained for the sidebar. Rows beyond this cap are
/// dropped at discovery time and counted in
/// [`WorktreeDiscovery::omitted`], mirroring the bounded SSH host list.
pub const MAX_WORKTREE_SIDEBAR_ROWS: usize = 24;

/// Maximum bytes of `git worktree list --porcelain` output accepted before
/// discovery refuses the rest as [`WorktreeListError::TooLarge`]. The output
/// is path-per-line metadata; a pathological repository cannot grow Noren's
/// memory without bound through it.
pub const MAX_WORKTREE_OUTPUT_BYTES: usize = 256 * 1024;

/// Fixed argv for the discovery child. No option is caller-controlled.
const GIT_PROGRAM: &str = "git";
const GIT_WORKTREE_ARGS: [&str; 3] = ["worktree", "list", "--porcelain"];

/// Typed, content-free failure of worktree discovery.
///
/// Every message is a fixed string. The launch directory, worktree paths,
/// branch names, and git's own output bytes are never carried: all of them
/// are user- or environment-derived text, and a path may embed a username or
/// a private directory name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorktreeListError {
    /// The `git` child could not be spawned (not installed, or not on the
    /// search path). Discovery is unavailable, not failed.
    GitUnavailable,
    /// `git` ran and exited non-zero: the launch directory is not inside a
    /// git repository, or git refused the listing for its own reasons.
    NotARepository,
    /// The launch directory could not be resolved as a working directory for
    /// the child.
    LaunchDirectoryUnavailable,
    /// The porcelain output was not valid UTF-8.
    NotUtf8,
    /// The porcelain output exceeded [`MAX_WORKTREE_OUTPUT_BYTES`].
    TooLarge,
    /// The porcelain output was structurally malformed (a record without a
    /// `worktree` line, or an unknown attribute key).
    Malformed,
}

impl fmt::Display for WorktreeListError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GitUnavailable => {
                f.write_str("git is unavailable; worktree discovery is disabled")
            }
            Self::NotARepository => {
                f.write_str("launch directory is not a git repository; no worktrees listed")
            }
            Self::LaunchDirectoryUnavailable => {
                f.write_str("launch directory could not be used to run git")
            }
            Self::NotUtf8 => f.write_str("git worktree listing is not valid UTF-8"),
            Self::TooLarge => f.write_str("git worktree listing exceeds its byte limit"),
            Self::Malformed => f.write_str("git worktree listing is malformed"),
        }
    }
}

impl std::error::Error for WorktreeListError {}

/// One worktree record parsed from `git worktree list --porcelain`.
///
/// Paths are kept verbatim from the porcelain line (git quotes nothing in
/// porcelain form; a path with spaces or non-ASCII characters is one line's
/// payload), so spaces and Unicode round-trip without scraping.
#[derive(Clone, PartialEq, Eq)]
pub struct RawWorktree {
    path: PathBuf,
    branch: Option<String>,
    bare: bool,
    detached: bool,
}

impl RawWorktree {
    /// The worktree's absolute path as git printed it.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The full ref name (`refs/heads/...`) when a branch is checked out.
    #[must_use]
    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    /// Whether this record is the repository's bare worktree.
    #[must_use]
    pub const fn is_bare(&self) -> bool {
        self.bare
    }

    /// Whether HEAD is detached in this worktree.
    #[must_use]
    pub const fn is_detached(&self) -> bool {
        self.detached
    }
}

/// Shape-only [`Debug`] (the #142/#146/#148 discipline): variant presence
/// only, never the path or the branch — both are user- or environment-derived
/// text, and a path may embed a username or a private directory name.
impl fmt::Debug for RawWorktree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RawWorktree")
            .field("bare", &self.bare)
            .field("detached", &self.detached)
            .field("has_branch", &self.branch.is_some())
            .finish_non_exhaustive()
    }
}

impl RawWorktree {
    /// The short display name of the checked-out branch (the ref's last
    /// path component), or a fixed placeholder for a detached HEAD or a bare
    /// worktree. Placeholder text is fixed ASCII so the bitmap font renders
    /// it.
    #[must_use]
    pub fn branch_display(&self) -> String {
        if let Some(branch) = &self.branch {
            let short = branch.rsplit('/').next().unwrap_or(branch);
            return short.to_owned();
        }
        if self.bare {
            "(bare)".to_owned()
        } else {
            "(detached)".to_owned()
        }
    }

    /// The worktree's display name: the final path component as UTF-8 text,
    /// or the fixed placeholder when the component is not UTF-8.
    #[must_use]
    pub fn name_display(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "(worktree)".to_owned())
    }
}

/// One worktree fact prepared for the sidebar: the parsed record plus the
/// on-disk presence observed at discovery time.
#[derive(Clone, PartialEq, Eq)]
pub struct DiscoveredWorktree {
    raw: RawWorktree,
    directory_present: bool,
}

impl DiscoveredWorktree {
    /// Observe the filesystem once and build the sidebar fact.
    #[must_use]
    pub fn observe(raw: RawWorktree) -> Self {
        let directory_present = raw.path().is_dir();
        Self {
            raw,
            directory_present,
        }
    }

    /// The parsed record.
    #[must_use]
    pub const fn raw(&self) -> &RawWorktree {
        &self.raw
    }

    /// The worktree's absolute path, the launch target of a session.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.raw.path()
    }

    /// Whether the worktree's directory existed on disk at discovery time.
    /// A registered-but-deleted worktree reports `false`; it is still listed
    /// (honestly) but must not be launched.
    #[must_use]
    pub const fn directory_present(&self) -> bool {
        self.directory_present
    }

    /// The bounded sidebar label: the directory's final component.
    #[must_use]
    pub fn name_display(&self) -> String {
        self.raw.name_display()
    }

    /// The bounded sidebar detail: branch (or detached/bare placeholder)
    /// plus a missing-directory marker when the directory is gone.
    #[must_use]
    pub fn branch_display(&self) -> String {
        let branch = self.raw.branch_display();
        if self.directory_present {
            branch
        } else {
            format!("{branch} (missing)")
        }
    }
}

/// Shape-only [`Debug`] (the #142/#146/#148 discipline): presence flags only,
/// never the path or branch text.
impl fmt::Debug for DiscoveredWorktree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiscoveredWorktree")
            .field("directory_present", &self.directory_present)
            .finish_non_exhaustive()
    }
}

/// The outcome of one discovery pass: the bounded row list and how many
/// worktrees the cap omitted.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct WorktreeDiscovery {
    rows: Vec<DiscoveredWorktree>,
    omitted: usize,
}

impl WorktreeDiscovery {
    /// The empty outcome (nothing discovered, nothing omitted).
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            rows: Vec::new(),
            omitted: 0,
        }
    }

    /// Build the bounded outcome from parsed records in git's listing order:
    /// the first [`MAX_WORKTREE_SIDEBAR_ROWS`] become rows, the rest are
    /// counted as omitted.
    #[must_use]
    pub fn from_records(records: Vec<RawWorktree>) -> Self {
        let omitted = records.len().saturating_sub(MAX_WORKTREE_SIDEBAR_ROWS);
        let rows = records
            .into_iter()
            .take(MAX_WORKTREE_SIDEBAR_ROWS)
            .map(DiscoveredWorktree::observe)
            .collect();
        Self { rows, omitted }
    }

    /// The retained worktree rows, in git's listing order (the main worktree
    /// first).
    #[must_use]
    pub fn rows(&self) -> &[DiscoveredWorktree] {
        &self.rows
    }

    /// How many worktrees existed beyond the retained rows.
    #[must_use]
    pub const fn omitted(&self) -> usize {
        self.omitted
    }

    /// Total worktrees git listed (retained plus omitted).
    #[must_use]
    pub const fn total(&self) -> usize {
        self.rows.len() + self.omitted
    }
}

/// Parse `git worktree list --porcelain` output into records, in order.
///
/// The porcelain grammar (stable since git 2.7): records separated by blank
/// lines; the first line of a record is `worktree <path>`; a record may carry
/// `HEAD <sha>`, `branch <ref>`, `bare`, `detached`, and (newer git)
/// `prunable <reason>` / `locked <reason>` lines. Unknown future attributes
/// are ignored rather than rejected so a newer git cannot break discovery;
/// a record without its leading `worktree` line, or a truncated final path,
/// is [`WorktreeListError::Malformed`].
///
/// Total: no panic on any input, including empty text (zero records).
pub fn parse_worktree_porcelain(text: &str) -> Result<Vec<RawWorktree>, WorktreeListError> {
    let mut records: Vec<RawWorktree> = Vec::new();
    let mut current: Option<RawWorktree> = None;
    for line in text.lines() {
        if line.is_empty() {
            if let Some(record) = current.take() {
                records.push(record);
            }
            continue;
        }
        if let Some(path) = line.strip_prefix("worktree ") {
            if current.is_some() {
                // A new record began without a blank separator; git never
                // emits this, so the document is malformed.
                return Err(WorktreeListError::Malformed);
            }
            if path.is_empty() {
                return Err(WorktreeListError::Malformed);
            }
            current = Some(RawWorktree {
                path: PathBuf::from(path),
                branch: None,
                bare: false,
                detached: false,
            });
            continue;
        }
        let Some(record) = current.as_mut() else {
            // An attribute line before any `worktree` line.
            return Err(WorktreeListError::Malformed);
        };
        if let Some(branch) = line.strip_prefix("branch ") {
            if branch.is_empty() {
                return Err(WorktreeListError::Malformed);
            }
            record.branch = Some(branch.to_owned());
        } else if line == "bare" {
            record.bare = true;
        } else if line == "detached" {
            record.detached = true;
        }
        // `HEAD <sha>`, `prunable ...`, `locked ...`, and unknown future
        // attributes are intentionally not retained.
    }
    if let Some(record) = current.take() {
        records.push(record);
    }
    Ok(records)
}

/// Discover the worktrees of the repository containing `launch_dir` by
/// running the fixed `git worktree list --porcelain` child there.
///
/// Never panics: every failure — git unavailable, not a repository, output
/// not UTF-8 or oversized or malformed — is a typed, content-free
/// [`WorktreeListError`]. Success yields the bounded
/// [`WorktreeDiscovery`], whose rows include registered-but-deleted
/// worktrees marked [`DiscoveredWorktree::directory_present`] == `false`.
pub fn discover_worktrees(launch_dir: &Path) -> Result<WorktreeDiscovery, WorktreeListError> {
    let output = Command::new(GIT_PROGRAM)
        .args(GIT_WORKTREE_ARGS)
        .current_dir(launch_dir)
        .output()
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => WorktreeListError::GitUnavailable,
            std::io::ErrorKind::NotADirectory => WorktreeListError::LaunchDirectoryUnavailable,
            _ => WorktreeListError::LaunchDirectoryUnavailable,
        })?;
    if !output.status.success() {
        // `git worktree list` exits 128 outside a repository; every non-zero
        // exit means "no listing for this directory" and none of git's
        // stderr is retained.
        return Err(WorktreeListError::NotARepository);
    }
    if output.stdout.len() > MAX_WORKTREE_OUTPUT_BYTES {
        return Err(WorktreeListError::TooLarge);
    }
    let text = std::str::from_utf8(&output.stdout).map_err(|_| WorktreeListError::NotUtf8)?;
    let records = parse_worktree_porcelain(text)?;
    Ok(WorktreeDiscovery::from_records(records))
}

#[cfg(test)]
mod tests {
    //! Pure-parser and bounding tests. The live-git cases (real `git worktree
    //! add`, deleted directories, detached HEADs) live in the binary test
    //! module `main/tests.rs`, which owns the app-level fixtures.

    use super::*;

    /// The exact porcelain shape of a two-worktree repository: a branched
    /// main worktree plus a branched linked worktree.
    const TWO_WORKTREES: &str = "worktree /Users/dev/noren\n\
                                 HEAD 6f755f8a4d2b6f4f8da7bd0e92a17f5f018d94c6\n\
                                 branch refs/heads/main\n\
                                 \n\
                                 worktree /Users/dev/noren-w-breadth\n\
                                 HEAD 189c040b1e5f8da7bd0e92a17f5f018d94c6aa\n\
                                 branch refs/heads/feat/worktree-sessions\n";

    #[test]
    fn empty_output_yields_no_records() {
        assert_eq!(parse_worktree_porcelain(""), Ok(Vec::new()));
        // A trailing separator with no following record is also empty.
        assert_eq!(parse_worktree_porcelain("\n"), Ok(Vec::new()));
    }

    #[test]
    fn two_worktrees_parse_in_listing_order_with_branches() {
        let records =
            parse_worktree_porcelain(TWO_WORKTREES).expect("well-formed porcelain parses");
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].path(), Path::new("/Users/dev/noren"));
        assert_eq!(records[0].branch(), Some("refs/heads/main"));
        assert!(!records[0].is_bare());
        assert!(!records[0].is_detached());
        assert_eq!(records[0].branch_display(), "main");
        assert_eq!(records[0].name_display(), "noren");
        assert_eq!(records[1].path(), Path::new("/Users/dev/noren-w-breadth"));
        assert_eq!(records[1].branch_display(), "worktree-sessions");
    }

    #[test]
    fn a_single_worktree_repository_parses() {
        let text = "worktree /srv/repo\nHEAD abcdef0123456789\nbranch refs/heads/trunk\n";
        let records = parse_worktree_porcelain(text).expect("single record parses");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].path(), Path::new("/srv/repo"));
        assert_eq!(records[0].branch(), Some("refs/heads/trunk"));
    }

    #[test]
    fn a_detached_head_has_no_branch_and_reports_detached() {
        let text = "worktree /srv/detached\nHEAD fedcba9876543210\ndetached\n";
        let records = parse_worktree_porcelain(text).expect("detached record parses");
        assert_eq!(records.len(), 1);
        assert!(records[0].is_detached());
        assert_eq!(records[0].branch(), None);
        assert_eq!(records[0].branch_display(), "(detached)");
    }

    #[test]
    fn a_bare_worktree_reports_bare_and_a_placeholder_branch() {
        let text = "worktree /srv/repo.git\nbare\n\nworktree /srv/repo\nHEAD abc\nbranch refs/heads/main\n";
        let records = parse_worktree_porcelain(text).expect("bare record parses");
        assert_eq!(records.len(), 2);
        assert!(records[0].is_bare());
        assert_eq!(records[0].branch_display(), "(bare)");
        assert!(!records[1].is_bare());
    }

    #[test]
    fn paths_with_spaces_and_non_ascii_parse_verbatim() {
        let text =
            "worktree /Users/üser namé/秘密 の フォルダ/wt\nHEAD abc\nbranch refs/heads/tôpik\n";
        let records = parse_worktree_porcelain(text).expect("non-ASCII path parses");
        assert_eq!(
            records[0].path(),
            Path::new("/Users/üser namé/秘密 の フォルダ/wt")
        );
        assert_eq!(records[0].name_display(), "wt");
        assert_eq!(records[0].branch_display(), "tôpik");
    }

    #[test]
    fn newer_git_attributes_are_ignored_not_rejected() {
        // `prunable`/`locked` exist in git >= 2.17 output; unknown future
        // attributes must not break discovery of the worktree itself.
        let text = "worktree /srv/prunable\nHEAD abc\nprunable gitdir file points to non-existent location\nlocked\n";
        let records = parse_worktree_porcelain(text).expect("unknown attributes are ignored");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].path(), Path::new("/srv/prunable"));
    }

    #[test]
    fn a_record_without_a_worktree_line_is_malformed() {
        assert_eq!(
            parse_worktree_porcelain("HEAD abc\n"),
            Err(WorktreeListError::Malformed)
        );
        // Two `worktree` lines without a blank separator never happen in
        // real output and are refused rather than silently splitting.
        assert_eq!(
            parse_worktree_porcelain("worktree /a\nworktree /b\n"),
            Err(WorktreeListError::Malformed)
        );
        assert_eq!(
            parse_worktree_porcelain("worktree \n"),
            Err(WorktreeListError::Malformed)
        );
    }

    #[test]
    fn discovery_bounds_rows_and_reports_the_omitted_count() {
        let records: Vec<RawWorktree> = (0..(MAX_WORKTREE_SIDEBAR_ROWS + 5))
            .map(|index| RawWorktree {
                path: PathBuf::from(format!("/srv/wt-{index}")),
                branch: Some(format!("refs/heads/b-{index}")),
                bare: false,
                detached: false,
            })
            .collect();
        let discovery = WorktreeDiscovery::from_records(records);
        assert_eq!(discovery.rows().len(), MAX_WORKTREE_SIDEBAR_ROWS);
        assert_eq!(discovery.omitted(), 5);
        assert_eq!(discovery.total(), MAX_WORKTREE_SIDEBAR_ROWS + 5);
        // The cap keeps the FIRST worktrees in git's listing order: the main
        // worktree is always retained.
        assert_eq!(discovery.rows()[0].path(), Path::new("/srv/wt-0"));
    }

    #[test]
    fn error_messages_are_fixed_content_free_strings() {
        // Every variant's Display is a fixed string; none of them can carry a
        // path, branch, or git output text because none of them accepts any.
        for error in [
            WorktreeListError::GitUnavailable,
            WorktreeListError::NotARepository,
            WorktreeListError::LaunchDirectoryUnavailable,
            WorktreeListError::NotUtf8,
            WorktreeListError::TooLarge,
            WorktreeListError::Malformed,
        ] {
            let text = error.to_string();
            assert!(!text.is_empty());
            assert!(!text.contains('/'), "no path text: {text}");
        }
    }

    #[test]
    fn debug_output_is_shape_only_for_secret_shaped_paths() {
        // The #142/#146/#148 discipline: a worktree path (and branch) can
        // embed a username or private directory name; Debug must not print
        // either, at any nesting this module owns.
        let secret = format!("NOREN-WT-hunter2-{}", std::process::id());
        let record = RawWorktree {
            path: PathBuf::from(format!("/Users/{secret}/wt")),
            branch: Some(format!("refs/heads/{secret}")),
            bare: false,
            detached: false,
        };
        let discovered = DiscoveredWorktree {
            raw: record.clone(),
            directory_present: true,
        };
        let discovery = WorktreeDiscovery::from_records(vec![record]);
        for rendered in [
            format!("{discovered:?}"),
            format!("{:?}", discovery.rows()[0]),
            format!("{:?}", discovery.rows()[0].raw()),
        ] {
            assert!(
                !rendered.contains(&secret),
                "debug surface leaked path or branch text: {rendered}"
            );
        }
    }

    #[test]
    fn a_missing_directory_row_keeps_the_record_and_marks_it() {
        // The runner-level fixture (a real deleted worktree) lives in the
        // binary tests; this pins the data model: a record whose directory
        // does not exist is still a row, marked not present, with the
        // missing marker in its detail text.
        let record = RawWorktree {
            path: PathBuf::from("/definitely/not/a/real/directory/wt-gone"),
            branch: Some("refs/heads/gone".to_owned()),
            bare: false,
            detached: false,
        };
        let discovery = WorktreeDiscovery::from_records(vec![record]);
        let row = &discovery.rows()[0];
        assert!(!row.directory_present());
        assert_eq!(row.branch_display(), "gone (missing)");
        assert_eq!(
            row.path(),
            Path::new("/definitely/not/a/real/directory/wt-gone")
        );
    }
}
