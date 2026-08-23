//! Pure binary-private state transitions for sidebar persistence evidence.
//!
//! Filesystem I/O and diagnostic rendering stay in `main.rs`; this module
//! records only what those boundaries proved about the latest save and any
//! external replacement observed along the way.

use std::fmt;

/// One bounded state-file observation. Errors carry no untrusted details
/// here; the I/O boundary reports the typed error before using `Unavailable`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Observation {
    Observed(Option<Vec<u8>>),
    Unavailable,
}

/// The write and exact post-write observation from one persistence attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SaveOutcome {
    Failed,
    Written {
        intended: Vec<u8>,
        observed: Observation,
    },
}

/// Inputs to the state-only transition applied after the I/O boundary has
/// completed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AttemptOutcome {
    before: Observation,
    save: SaveOutcome,
}

impl AttemptOutcome {
    pub(super) fn new(before: Observation, save: SaveOutcome) -> Self {
        Self { before, save }
    }
}

/// Whether an exact comparison baseline has been established for the state
/// file. `Verified(None)` is a real, observed first-run absence; `Invalid` is
/// deliberately distinct so failed verification cannot reuse stale bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Baseline {
    #[default]
    Invalid,
    Verified(Option<Vec<u8>>),
}

/// Persistence diagnostics and the exact baseline that makes conflict
/// comparisons meaningful.
#[derive(Clone, Default, PartialEq, Eq)]
pub(super) struct PersistenceState {
    baseline: Baseline,
    /// Sticky once an external replacement is definitively observed.
    conflict: bool,
    /// Describes only the latest restore or persistence attempt.
    unverified: bool,
}

/// Shape-only [`Debug`] (issue #146): baseline validity and byte count,
/// conflict and verification flags — never the persisted bytes.
///
/// The baseline and the save observations hold the exact `sessions.toml`
/// document, which embeds user-derived workspace text; printing those bytes
/// through `Debug` would leak what the persistence boundary keeps out of
/// diagnostics. Any holder of a `PersistenceState` is safe by construction
/// instead of by a redacting container.
impl fmt::Debug for PersistenceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut shape = f.debug_struct("PersistenceState");
        match &self.baseline {
            Baseline::Invalid => {
                shape.field("baseline", &format_args!("invalid"));
            }
            Baseline::Verified(None) => {
                shape.field("baseline", &format_args!("absent"));
            }
            Baseline::Verified(Some(bytes)) => {
                shape.field("baseline", &format_args!("{} bytes", bytes.len()));
            }
        }
        shape
            .field("conflict", &self.conflict)
            .field("unverified", &self.unverified)
            .finish()
    }
}

impl PersistenceState {
    pub(super) fn restore_succeeded(&mut self, observed: Option<Vec<u8>>) {
        self.baseline = Baseline::Verified(observed);
        self.unverified = false;
    }

    pub(super) fn restore_failed(&mut self) {
        self.baseline = Baseline::Invalid;
        self.unverified = true;
    }

    /// Apply one persistence attempt without performing I/O.
    ///
    /// A changed pre-save observation against a valid baseline proves a
    /// conflict. After a successful write, definitive absence or bytes other
    /// than the intended document also prove that a peer replaced the file.
    /// `Unavailable` cannot prove replacement, so it is unverified-only.
    /// Only an exact post-save match establishes the next valid baseline.
    pub(super) fn apply_attempt(&mut self, outcome: AttemptOutcome) {
        let mut unverified = matches!(&outcome.before, Observation::Unavailable);

        if let (Baseline::Verified(baseline), Observation::Observed(current)) =
            (&self.baseline, &outcome.before)
        {
            if current != baseline {
                self.conflict = true;
            }
        }

        match outcome.save {
            SaveOutcome::Written {
                intended,
                observed: Observation::Observed(Some(observed)),
            } if observed == intended => {
                self.baseline = Baseline::Verified(Some(intended));
            }
            SaveOutcome::Written {
                observed: Observation::Observed(_),
                ..
            } => {
                self.baseline = Baseline::Invalid;
                self.conflict = true;
                unverified = true;
            }
            SaveOutcome::Failed
            | SaveOutcome::Written {
                observed: Observation::Unavailable,
                ..
            } => {
                self.baseline = Baseline::Invalid;
                unverified = true;
            }
        }

        self.unverified = unverified;
    }

    pub(super) const fn conflict(&self) -> bool {
        self.conflict
    }

    pub(super) const fn unverified(&self) -> bool {
        self.unverified
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observed(bytes: &[u8]) -> Observation {
        Observation::Observed(Some(bytes.to_vec()))
    }

    fn exact_save(intended: &[u8]) -> SaveOutcome {
        SaveOutcome::Written {
            intended: intended.to_vec(),
            observed: observed(intended),
        }
    }

    fn attempt(before: Observation, save: SaveOutcome) -> AttemptOutcome {
        AttemptOutcome::new(before, save)
    }

    #[test]
    fn missing_post_save_snapshot_sets_conflict_and_unverified_then_exact_retry_recovers_only_verification()
     {
        let baseline = b"verified baseline";
        let first_write = b"first intended bytes";
        let retry_write = b"retry intended bytes";
        let mut state = PersistenceState::default();
        state.restore_succeeded(Some(baseline.to_vec()));

        state.apply_attempt(attempt(
            observed(baseline),
            SaveOutcome::Written {
                intended: first_write.to_vec(),
                observed: Observation::Observed(None),
            },
        ));

        assert!(state.conflict());
        assert!(state.unverified());
        assert_eq!(state.baseline, Baseline::Invalid);

        state.apply_attempt(attempt(observed(first_write), exact_save(retry_write)));

        assert!(state.conflict(), "conflict is sticky across an exact retry");
        assert!(!state.unverified(), "the latest save is now verified");
        assert_eq!(
            state.baseline,
            Baseline::Verified(Some(retry_write.to_vec()))
        );
    }

    #[test]
    fn mismatched_post_save_snapshot_sets_conflict_and_unverified_then_exact_retry_recovers_only_verification()
     {
        let baseline = b"verified baseline";
        let first_write = b"first intended bytes";
        let peer = b"peer replacement bytes";
        let retry_write = b"retry intended bytes";
        let mut state = PersistenceState::default();
        state.restore_succeeded(Some(baseline.to_vec()));

        state.apply_attempt(attempt(
            observed(baseline),
            SaveOutcome::Written {
                intended: first_write.to_vec(),
                observed: observed(peer),
            },
        ));

        assert!(state.conflict());
        assert!(state.unverified());
        assert_eq!(state.baseline, Baseline::Invalid);

        state.apply_attempt(attempt(observed(peer), exact_save(retry_write)));

        assert!(state.conflict(), "conflict is sticky across an exact retry");
        assert!(!state.unverified(), "the latest save is now verified");
        assert_eq!(
            state.baseline,
            Baseline::Verified(Some(retry_write.to_vec()))
        );
    }

    #[test]
    fn sticky_conflict_survives_a_later_clean_save() {
        let baseline = b"verified baseline";
        let peer = b"peer bytes";
        let first_write = b"first write";
        let clean_write = b"clean write";
        let mut state = PersistenceState::default();
        state.restore_succeeded(Some(baseline.to_vec()));

        state.apply_attempt(attempt(observed(peer), exact_save(first_write)));
        assert!(state.conflict());
        assert!(!state.unverified());

        state.apply_attempt(attempt(observed(first_write), exact_save(clean_write)));

        assert!(state.conflict());
        assert!(!state.unverified());
        assert_eq!(
            state.baseline,
            Baseline::Verified(Some(clean_write.to_vec()))
        );
    }

    #[test]
    fn pre_save_deletion_against_verified_some_sets_sticky_conflict() {
        let baseline = b"verified baseline";
        let intended = b"intended bytes";
        let mut state = PersistenceState::default();
        state.restore_succeeded(Some(baseline.to_vec()));

        state.apply_attempt(attempt(Observation::Observed(None), exact_save(intended)));

        assert!(state.conflict());
        assert!(!state.unverified());
        assert_eq!(state.baseline, Baseline::Verified(Some(intended.to_vec())));
    }

    #[test]
    fn save_failure_then_exact_retry_clears_unverified_without_inventing_conflict() {
        let baseline = b"verified baseline";
        let retry_write = b"retry bytes";
        let mut state = PersistenceState::default();
        state.restore_succeeded(Some(baseline.to_vec()));

        state.apply_attempt(attempt(observed(baseline), SaveOutcome::Failed));
        assert!(!state.conflict());
        assert!(state.unverified());
        assert_eq!(state.baseline, Baseline::Invalid);

        state.apply_attempt(attempt(observed(baseline), exact_save(retry_write)));

        assert!(!state.conflict());
        assert!(!state.unverified());
        assert_eq!(
            state.baseline,
            Baseline::Verified(Some(retry_write.to_vec()))
        );
    }

    #[test]
    fn unavailable_pre_inspection_plus_exact_save_is_unverified_only() {
        let baseline = b"verified baseline";
        let intended = b"intended bytes";
        let mut state = PersistenceState::default();
        state.restore_succeeded(Some(baseline.to_vec()));

        state.apply_attempt(attempt(Observation::Unavailable, exact_save(intended)));

        assert!(!state.conflict());
        assert!(state.unverified());
        assert_eq!(state.baseline, Baseline::Verified(Some(intended.to_vec())));
    }

    #[test]
    fn unavailable_post_save_observation_is_unverified_only_and_invalidates_baseline() {
        let baseline = b"verified baseline";
        let intended = b"intended bytes";
        let mut state = PersistenceState::default();
        state.restore_succeeded(Some(baseline.to_vec()));

        state.apply_attempt(attempt(
            observed(baseline),
            SaveOutcome::Written {
                intended: intended.to_vec(),
                observed: Observation::Unavailable,
            },
        ));

        assert!(!state.conflict());
        assert!(state.unverified());
        assert_eq!(state.baseline, Baseline::Invalid);
    }

    #[test]
    fn failed_restore_can_recover_without_comparing_an_invalid_baseline() {
        let old = b"previously verified document";
        let corrupt = b"readable but invalid restore input";
        let recovered = b"recovered exact document";
        let mut state = PersistenceState::default();
        state.restore_succeeded(Some(old.to_vec()));
        state.restore_failed();

        assert!(state.unverified());
        assert_eq!(state.baseline, Baseline::Invalid);

        state.apply_attempt(attempt(observed(corrupt), exact_save(recovered)));

        assert!(!state.conflict());
        assert!(!state.unverified());
        assert_eq!(state.baseline, Baseline::Verified(Some(recovered.to_vec())));
    }

    #[test]
    fn successful_restore_replaces_invalid_baseline_and_clears_unverified() {
        let restored = b"restored exact document";
        let mut state = PersistenceState::default();
        state.restore_failed();

        state.restore_succeeded(Some(restored.to_vec()));

        assert!(!state.conflict());
        assert!(!state.unverified());
        assert_eq!(state.baseline, Baseline::Verified(Some(restored.to_vec())));
    }

    // ── Debug is shape-only (issue #146) ──
    //
    // The baseline holds the exact persisted document (user-derived
    // workspace text). The guard asserts directly on `format!("{:?}")` —
    // per PR #142 it must not depend on a production formatter existing —
    // so the sentinel bytes must not appear in any rendering, including
    // the decimal byte-list a derived impl would print.

    #[test]
    fn debug_reports_baseline_shape_without_persisted_bytes() {
        let sentinel = format!("NOREN-PERSIST-SENTINEL-{}", std::process::id());
        let mut state = PersistenceState::default();
        state.restore_succeeded(Some(sentinel.clone().into_bytes()));

        let rendered = format!("{state:?}");
        assert!(
            !rendered.contains(&sentinel),
            "debug leaked the document as text: {rendered}"
        );
        assert!(
            !rendered.contains(&format!("{:?}", sentinel.as_bytes())),
            "debug leaked the document as a byte list: {rendered}"
        );
        // Not vacuous: the baseline's validity and size stay visible.
        assert!(rendered.contains("PersistenceState"), "{rendered}");
        assert!(
            rendered.contains(&format!("baseline: {} bytes", sentinel.len())),
            "{rendered}"
        );
        assert!(rendered.contains("conflict: false"), "{rendered}");
        assert!(rendered.contains("unverified: false"), "{rendered}");
    }

    #[test]
    fn debug_reports_invalid_and_absent_baselines_by_name() {
        let invalid = format!("{:?}", PersistenceState::default());
        assert!(invalid.contains("baseline: invalid"), "{invalid}");

        let mut absent = PersistenceState::default();
        absent.restore_succeeded(None);
        let rendered = format!("{absent:?}");
        assert!(rendered.contains("baseline: absent"), "{rendered}");
    }
}
