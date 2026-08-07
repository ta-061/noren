//! Zellij pass-through policy: which chords Noren interprets and which bytes
//! it forwards untouched to the running session.
//!
//! Boundary (ADR 0003, owner-decided): Noren manages the workspace OUTSIDE the
//! terminal; Zellij manages it INSIDE. Noren has no tabs, no panes, no layout,
//! no splits, and never reads or persists Zellij's internal layout. This module
//! therefore carries no workspace-model state at all — only the decision of
//! whether a pressed chord is claimed by Noren or forwarded byte-for-byte to
//! the child, plus the recovery guarantees around that decision.
//!
//! The design follows the pass-through obligations recorded in the
//! [Zellij compatibility matrix](https://github.com/ta-061/noren/blob/main/docs/compatibility/zellij.md):
//!
//! - Unbound input is forwarded byte-for-byte. A chord Noren does not claim
//!   reaches the terminal unchanged, including modifiers.
//! - Noren claims as little as possible. The frozen default manifest claims a
//!   single chord (the pass-through exit leader), and
//!   [`PassthroughPolicy::try_new`] rejects any manifest beyond the exit
//!   leader and one optional command-palette chord.
//! - A recovery path always exists. Pass-through is only constructible when a
//!   valid keyboard exit leader remains, and a pointer-invoked palette surface
//!   is modeled as reachable regardless of keyboard state, so no configuration
//!   can trap a user in a Noren-only mode.
//! - Collisions with Zellij's documented v0.44.3 defaults are asserted
//!   mechanically against a pinned corpus, never assumed.
//!
//! Evidence status: this is implementation of Noren-side policy only. It is
//! not the `noren_zellij_pass_through` byte-oracle evidence run, which the
//! compatibility matrix assigns to `codex-lab` on `Z-PROTO`/`Z-SSH` targets.

use std::fmt;
use std::iter;

/// Zellij release tag the default-binding corpus is pinned to.
///
/// Mirrors the [versioned upstream fixture](https://github.com/ta-061/noren/blob/main/docs/compatibility/zellij.md#versioned-upstream-fixture).
pub const ZELLIJ_FIXTURE_TAG: &str = "v0.44.3";

/// Upstream source commit the default-binding corpus is pinned to.
pub const ZELLIJ_FIXTURE_COMMIT: &str = "55a2121b73dce4be624cda425a960e893000777c";

/// Maximum chords in one claimed leader sequence.
///
/// Bounded so a pathological configured leader cannot inflate match depth or
/// the replay buffer.
pub const MAX_LEADER_CHORDS: usize = 8;

/// Claim identity for the keyboard exit leader. Exactly one must exist in any
/// valid policy; it is the keyboard half of the recovery path.
pub const CLAIM_ID_EXIT: &str = "noren.passthrough.exit";

/// Claim identity for the optional keyboard chord opening the command palette.
/// The palette's pointer-invoked surface exists regardless of this claim.
pub const CLAIM_ID_PALETTE: &str = "noren.palette.open";

/// Normalized physical key identity for pass-through decisions.
///
/// Case is carried by the Shift modifier, never by the character: `Char` is
/// stored lowercase so one chord has exactly one representation. Whitespace
/// and control codepoints are rejected in favor of the named variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeyCode {
    /// A printable character, case-folded to lowercase.
    Char(char),
    /// Function key F1 through F24.
    Function(u8),
    Enter,
    Tab,
    Backspace,
    Escape,
    Space,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
}

/// Active modifier keys for a pass-through chord.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Modifiers {
    ctrl: bool,
    alt: bool,
    shift: bool,
    super_key: bool,
}

impl Modifiers {
    /// The empty modifier set.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            ctrl: false,
            alt: false,
            shift: false,
            super_key: false,
        }
    }

    /// Add the Control modifier.
    #[must_use]
    pub const fn ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }

    /// Add the Alt/Option modifier.
    #[must_use]
    pub const fn alt(mut self) -> Self {
        self.alt = true;
        self
    }

    /// Add the Shift modifier.
    #[must_use]
    pub const fn shift(mut self) -> Self {
        self.shift = true;
        self
    }

    /// Add the Super/Command modifier.
    #[must_use]
    pub const fn super_key(mut self) -> Self {
        self.super_key = true;
        self
    }

    /// Whether Control is held.
    #[must_use]
    pub const fn is_ctrl(self) -> bool {
        self.ctrl
    }

    /// Whether Alt/Option is held.
    #[must_use]
    pub const fn is_alt(self) -> bool {
        self.alt
    }

    /// Whether Shift is held.
    #[must_use]
    pub const fn is_shift(self) -> bool {
        self.shift
    }

    /// Whether Super/Command is held.
    #[must_use]
    pub const fn is_super(self) -> bool {
        self.super_key
    }
}

/// Why a chord could not be normalized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChordError {
    /// `Char` must be printable and non-whitespace; use `KeyCode::Space`,
    /// `KeyCode::Tab`, or `KeyCode::Enter` instead.
    ControlOrWhitespaceChar,
    /// Function keys are F1 through F24.
    FunctionKeyOutOfRange,
}

impl fmt::Display for ChordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ControlOrWhitespaceChar => f.write_str(
                "Char chords must be printable and non-whitespace; \
                 use Space, Tab, or Enter variants",
            ),
            Self::FunctionKeyOutOfRange => f.write_str("function keys are F1 through F24"),
        }
    }
}

impl std::error::Error for ChordError {}

/// One normalized chord: a key identity plus its modifier set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Chord {
    code: KeyCode,
    modifiers: Modifiers,
}

impl Chord {
    /// Normalize and construct a chord.
    ///
    /// ASCII characters are case-folded to lowercase; uppercase input is not a
    /// distinct representation but Shift plus lowercase. Control and
    /// whitespace codepoints are rejected for `Char`.
    pub fn new(code: KeyCode, modifiers: Modifiers) -> Result<Self, ChordError> {
        let code = match code {
            KeyCode::Char(character) => {
                if character.is_control() || character.is_whitespace() {
                    return Err(ChordError::ControlOrWhitespaceChar);
                }
                KeyCode::Char(character.to_ascii_lowercase())
            }
            KeyCode::Function(number) if !(1..=24).contains(&number) => {
                return Err(ChordError::FunctionKeyOutOfRange);
            }
            other => other,
        };
        Ok(Self { code, modifiers })
    }

    /// The normalized key identity.
    #[must_use]
    pub const fn code(self) -> KeyCode {
        self.code
    }

    /// The modifier set.
    #[must_use]
    pub const fn modifiers(self) -> Modifiers {
        self.modifiers
    }
}

/// Why a chord sequence could not be constructed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeqError {
    /// A leader sequence must contain at least one chord.
    Empty,
    /// A leader sequence longer than [`MAX_LEADER_CHORDS`] is rejected.
    TooLong {
        /// The enforced cap.
        max: usize,
        /// The length that was supplied.
        got: usize,
    },
}

impl fmt::Display for SeqError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("a leader sequence must contain at least one chord"),
            Self::TooLong { max, got } => {
                write!(f, "leader sequence has {got} chords; the cap is {max}")
            }
        }
    }
}

impl std::error::Error for SeqError {}

/// A non-empty, length-bounded sequence of chords.
///
/// The default pass-through claims are single-chord sequences; multi-chord
/// leaders exist so a future configurable leader can be validated by the same
/// collision and replay rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChordSeq {
    chords: Vec<Chord>,
}

impl ChordSeq {
    /// Construct a sequence, enforcing the non-empty and length caps.
    pub fn new(chords: Vec<Chord>) -> Result<Self, SeqError> {
        if chords.is_empty() {
            return Err(SeqError::Empty);
        }
        if chords.len() > MAX_LEADER_CHORDS {
            return Err(SeqError::TooLong {
                max: MAX_LEADER_CHORDS,
                got: chords.len(),
            });
        }
        Ok(Self { chords })
    }

    /// A single-chord sequence.
    #[must_use]
    pub fn single(chord: Chord) -> Self {
        Self {
            chords: vec![chord],
        }
    }

    /// The chords in order; never empty.
    #[must_use]
    pub fn chords(&self) -> &[Chord] {
        &self.chords
    }

    /// Whether every chord of `self` equals the leading chords of `other`,
    /// including the case where the sequences are equal.
    #[must_use]
    pub fn is_prefix_of(&self, other: &ChordSeq) -> bool {
        self.chords.len() <= other.chords.len()
            && self.chords.iter().zip(&other.chords).all(|(a, b)| a == b)
    }
}

/// One binding record from the pinned Zellij default-keymap corpus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZellijBinding {
    /// The Zellij mode in which the chord is bound (the default preset binds
    /// only single chords; modes, not multi-chord sequences, provide Zellij's
    /// multi-step behavior).
    pub mode: &'static str,
    /// The bound chord.
    pub chord: Chord,
}

fn chord(code: KeyCode, modifiers: Modifiers) -> Chord {
    Chord::new(code, modifiers).expect("corpus chords are normalized constants")
}

fn ch(character: char, modifiers: Modifiers) -> Chord {
    chord(KeyCode::Char(character), modifiers)
}

/// Curated snapshot of the chords Zellij's bundled default preset binds,
/// pinned to [`ZELLIJ_FIXTURE_TAG`] at [`ZELLIJ_FIXTURE_COMMIT`].
///
/// Provenance: the in-tree evidence anchors are the source-backed statements
/// in [`docs/compatibility/zellij.md`](https://github.com/ta-061/noren/blob/main/docs/compatibility/zellij.md)
/// (mode entry `Ctrl p`/`Ctrl t`/`Ctrl o`/`Ctrl g`, the shared Alt bindings
/// outside locked mode, Session-mode `d` detach, and the pane/tab mode
/// binding categories). Entries beyond those anchors are reconstructed from
/// the pinned upstream `default.kdl`; the corpus is advisory data that a
/// future `Z-PROTO` byte trace may refine. It is deliberately a superset
/// within modes: including an extra chord can only tighten Noren's claim
/// space, never weaken the no-collision assertion.
///
/// The corpus covers every mode of the default preset, not only `normal`,
/// because a focused pane can reach each mode; a Noren claim on any bound
/// chord would steal it from the session somewhere in its state space.
#[must_use]
pub fn zellij_default_bindings() -> Vec<ZellijBinding> {
    let c = Modifiers::empty().ctrl();
    let a = Modifiers::empty().alt();
    let mut bindings: Vec<ZellijBinding> = Vec::new();

    // Shared bindings active in every mode except locked.
    let shared_except_locked = [
        chord(KeyCode::Char('g'), c),
        chord(KeyCode::Char('q'), c),
        chord(KeyCode::Char('p'), c),
        chord(KeyCode::Char('t'), c),
        chord(KeyCode::Char('n'), c),
        chord(KeyCode::Char('s'), c),
        chord(KeyCode::Char('o'), c),
        chord(KeyCode::Char('h'), c),
        chord(KeyCode::Char('b'), c),
        chord(KeyCode::Char('n'), a),
        chord(KeyCode::Char('h'), a),
        chord(KeyCode::Char('j'), a),
        chord(KeyCode::Char('k'), a),
        chord(KeyCode::Char('l'), a),
        chord(KeyCode::Left, a),
        chord(KeyCode::Down, a),
        chord(KeyCode::Up, a),
        chord(KeyCode::Right, a),
        ch('=', a),
        ch('-', a),
        ch('+', a),
        ch('[', a),
        ch(']', a),
        chord(KeyCode::Char('f'), a),
    ];
    bindings.extend(shared_except_locked.into_iter().map(|c| ZellijBinding {
        mode: "shared_except locked",
        chord: c,
    }));

    // Locked mode: the same chord unlocks.
    bindings.push(ZellijBinding {
        mode: "locked",
        chord: chord(KeyCode::Char('g'), c),
    });

    // Pane mode.
    let pane = [
        chord(KeyCode::Escape, Modifiers::empty()),
        ch('e', Modifiers::empty()),
        ch('f', Modifiers::empty()),
        ch('z', Modifiers::empty()),
        ch('w', Modifiers::empty()),
        ch('x', Modifiers::empty()),
        ch('c', Modifiers::empty()),
        ch('n', Modifiers::empty()),
        ch('d', Modifiers::empty()),
        ch('r', Modifiers::empty()),
        ch('j', Modifiers::empty()),
        ch('k', Modifiers::empty()),
        ch('h', Modifiers::empty()),
        ch('l', Modifiers::empty()),
        ch('p', Modifiers::empty()),
        ch('<', c),
        ch('>', c),
        chord(KeyCode::Left, Modifiers::empty()),
        chord(KeyCode::Down, Modifiers::empty()),
        chord(KeyCode::Up, Modifiers::empty()),
        chord(KeyCode::Right, Modifiers::empty()),
    ];
    bindings.extend(pane.into_iter().map(|c| ZellijBinding {
        mode: "pane",
        chord: c,
    }));

    // Tab mode.
    let tab = [
        chord(KeyCode::Escape, Modifiers::empty()),
        ch('r', Modifiers::empty()),
        ch('x', Modifiers::empty()),
        ch('n', Modifiers::empty()),
        ch('s', Modifiers::empty()),
        ch('b', Modifiers::empty()),
        ch(']', Modifiers::empty()),
        ch('h', Modifiers::empty()),
        ch('l', Modifiers::empty()),
        ch('j', Modifiers::empty()),
        ch('k', Modifiers::empty()),
        ch('1', Modifiers::empty()),
        ch('2', Modifiers::empty()),
        ch('3', Modifiers::empty()),
        ch('4', Modifiers::empty()),
        ch('5', Modifiers::empty()),
        ch('6', Modifiers::empty()),
        ch('7', Modifiers::empty()),
        ch('8', Modifiers::empty()),
        ch('9', Modifiers::empty()),
        chord(KeyCode::Tab, Modifiers::empty()),
        chord(KeyCode::Left, Modifiers::empty()),
        chord(KeyCode::Down, Modifiers::empty()),
        chord(KeyCode::Up, Modifiers::empty()),
        chord(KeyCode::Right, Modifiers::empty()),
    ];
    bindings.extend(tab.into_iter().map(|c| ZellijBinding {
        mode: "tab",
        chord: c,
    }));

    // Resize mode.
    let resize = [
        chord(KeyCode::Escape, Modifiers::empty()),
        ch('j', Modifiers::empty()),
        ch('k', Modifiers::empty()),
        ch('h', Modifiers::empty()),
        ch('l', Modifiers::empty()),
        ch('=', Modifiers::empty()),
        ch('-', Modifiers::empty()),
        ch('+', Modifiers::empty()),
        chord(KeyCode::Down, Modifiers::empty()),
        chord(KeyCode::Up, Modifiers::empty()),
        chord(KeyCode::Left, Modifiers::empty()),
        chord(KeyCode::Right, Modifiers::empty()),
    ];
    bindings.extend(resize.into_iter().map(|c| ZellijBinding {
        mode: "resize",
        chord: c,
    }));

    // Move mode.
    let move_mode = [
        chord(KeyCode::Escape, Modifiers::empty()),
        ch('n', Modifiers::empty()),
        ch('h', Modifiers::empty()),
        ch('j', Modifiers::empty()),
        ch('k', Modifiers::empty()),
        ch('l', Modifiers::empty()),
        chord(KeyCode::Tab, Modifiers::empty()),
        chord(KeyCode::Down, Modifiers::empty()),
        chord(KeyCode::Up, Modifiers::empty()),
        chord(KeyCode::Left, Modifiers::empty()),
        chord(KeyCode::Right, Modifiers::empty()),
    ];
    bindings.extend(move_mode.into_iter().map(|c| ZellijBinding {
        mode: "move",
        chord: c,
    }));

    // Scroll/Search mode.
    let scroll = [
        chord(KeyCode::Escape, Modifiers::empty()),
        ch('j', Modifiers::empty()),
        ch('k', Modifiers::empty()),
        ch('d', Modifiers::empty()),
        ch('u', Modifiers::empty()),
        ch('s', Modifiers::empty()),
        ch('e', Modifiers::empty()),
        chord(KeyCode::Char('f'), c),
        chord(KeyCode::Char('b'), c),
        chord(KeyCode::PageUp, Modifiers::empty()),
        chord(KeyCode::PageDown, Modifiers::empty()),
        chord(KeyCode::Down, Modifiers::empty()),
        chord(KeyCode::Up, Modifiers::empty()),
    ];
    bindings.extend(scroll.into_iter().map(|c| ZellijBinding {
        mode: "scroll",
        chord: c,
    }));

    // Session mode.
    let session = [
        chord(KeyCode::Escape, Modifiers::empty()),
        ch('d', Modifiers::empty()),
        ch('w', Modifiers::empty()),
    ];
    bindings.extend(session.into_iter().map(|c| ZellijBinding {
        mode: "session",
        chord: c,
    }));

    // Tmux-compatible mode (entered via Ctrl b).
    let tmux = [
        chord(KeyCode::Char('b'), c),
        chord(KeyCode::Char('z'), c),
        ch('[', Modifiers::empty()),
        ch('"', Modifiers::empty()),
        ch('%', Modifiers::empty()),
        ch('z', Modifiers::empty()),
        ch('c', Modifiers::empty()),
        ch(',', Modifiers::empty()),
        ch('p', Modifiers::empty()),
        ch('n', Modifiers::empty()),
        ch('d', Modifiers::empty()),
        chord(KeyCode::Char('h'), a),
        chord(KeyCode::Char('j'), a),
        chord(KeyCode::Char('k'), a),
        chord(KeyCode::Char('l'), a),
        chord(KeyCode::Left, Modifiers::empty()),
        chord(KeyCode::Down, Modifiers::empty()),
        chord(KeyCode::Up, Modifiers::empty()),
        chord(KeyCode::Right, Modifiers::empty()),
        chord(KeyCode::Left, a),
        chord(KeyCode::Down, a),
        chord(KeyCode::Up, a),
        chord(KeyCode::Right, a),
    ];
    bindings.extend(tmux.into_iter().map(|c| ZellijBinding {
        mode: "tmux",
        chord: c,
    }));

    bindings
}

/// The Noren action a claimed pass-through chord dispatches.
///
/// These are pass-through decision tokens, not the app-wide action registry
/// (owned by a separate lane); an integrator maps them onto it. The variant
/// set is frozen by the minimality rule: nothing else may be claimed while
/// pass-through is active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassthroughAction {
    /// Leave pass-through and return the keyboard to the Noren workspace.
    ExitToWorkspace,
    /// Open the command palette.
    OpenCommandPalette,
}

/// One chord sequence Noren claims while pass-through is active.
///
/// Every claim carries a justification: a claimed chord is one Zellij can no
/// longer use, so the manifest must defend each entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassthroughClaim {
    /// Stable claim identity; one of [`CLAIM_ID_EXIT`] or [`CLAIM_ID_PALETTE`].
    pub id: &'static str,
    /// The action dispatched when the sequence completes.
    pub action: PassthroughAction,
    /// The claimed sequence; single-chord by default.
    pub seq: ChordSeq,
    /// Why this claim is worth one less chord for Zellij. Never empty.
    pub justification: &'static str,
}

/// The frozen default exit leader: `Super+Escape`.
///
/// Justification: the Super/Command modifier space has zero intersection with
/// the pinned Zellij default corpus (which binds no Super/Cmd chord) and with
/// the applications inside its panes (host convention keeps Super/Cmd chords
/// at the window layer, away from terminal children). ADR 0003 assigns that
/// window/workspace layer to Noren. Noren reads keys before the PTY, so the
/// exit leader cannot be shadowed by any child binding in any session state.
#[must_use]
pub fn default_exit_claim() -> PassthroughClaim {
    PassthroughClaim {
        id: CLAIM_ID_EXIT,
        action: PassthroughAction::ExitToWorkspace,
        seq: ChordSeq::single(chord(KeyCode::Escape, Modifiers::empty().super_key())),
        justification: "Super+Escape lives entirely outside the Zellij v0.44.3 default chord \
                        space and the child-application chord space; it reaches Noren before \
                        any PTY byte, so pass-through can always be exited from any session \
                        state without stealing a chord Zellij or its children could use",
    }
}

/// Why a pass-through manifest was rejected.
///
/// Rejection is fail-closed: pass-through must not become active under a
/// manifest that could trap a user or steal a documented Zellij default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyError {
    /// The manifest must contain the exit leader claim.
    MissingExitClaim,
    /// A claim id outside the permitted pass-through set.
    UnknownClaim(String),
    /// A claim id paired with the wrong action.
    WrongActionForClaim {
        /// The offending claim id.
        id: &'static str,
        /// The only action permitted for that id.
        expected: PassthroughAction,
    },
    /// The same claim id appeared twice.
    DuplicateClaimId(&'static str),
    /// A claim without a justification violates the minimal-claim rule.
    EmptyJustification {
        /// The offending claim id.
        id: &'static str,
    },
    /// One claim's sequence is a prefix of another's, making the shorter
    /// leader unreachable or the longer one ambiguous.
    AmbiguousLeader {
        /// The claim whose sequence is the prefix.
        prefix: &'static str,
        /// The claim whose sequence extends it.
        extended: &'static str,
    },
    /// A claimed sequence overlaps a pinned Zellij default binding.
    Collision(Collision),
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingExitClaim => f.write_str("pass-through requires an exit leader claim"),
            Self::UnknownClaim(id) => write!(f, "claim id is not permitted in pass-through: {id}"),
            Self::WrongActionForClaim { id, expected } => {
                write!(f, "claim {id} must dispatch {expected:?}")
            }
            Self::DuplicateClaimId(id) => write!(f, "claim id appears twice: {id}"),
            Self::EmptyJustification { id } => {
                write!(
                    f,
                    "claim {id} has no justification; every claimed chord costs Zellij one"
                )
            }
            Self::AmbiguousLeader { prefix, extended } => write!(
                f,
                "claim {prefix}'s sequence is a prefix of claim {extended}'s sequence"
            ),
            Self::Collision(collision) => write!(f, "{collision}"),
        }
    }
}

impl std::error::Error for PolicyError {}

/// How a claimed sequence overlaps a pinned Zellij default sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollisionKind {
    /// Both bind the identical sequence.
    Exact,
    /// The claimed sequence is a strict prefix of a Zellij sequence: Noren
    /// would intercept before Zellij's sequence could complete. The pinned
    /// corpus is single-chord, so only `Exact` and `ZellijPrefixesClaim` are
    /// reachable with it today; the branch is retained as defense for a
    /// future multi-chord corpus.
    ClaimPrefixesZellij,
    /// A Zellij sequence is a strict prefix of the claimed sequence: Noren
    /// would hold the Zellij chord waiting for further leader chords.
    ZellijPrefixesClaim,
}

/// One overlap between a Noren claim and the pinned Zellij default corpus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collision {
    /// The offending claim's id.
    pub claim_id: &'static str,
    /// The offending claimed sequence.
    pub claim_seq: ChordSeq,
    /// The Zellij mode holding the overlapping binding.
    pub zellij_mode: &'static str,
    /// The overlapping Zellij sequence (single-chord in the pinned corpus).
    pub zellij_seq: ChordSeq,
    /// The overlap shape.
    pub kind: CollisionKind,
}

impl fmt::Display for Collision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "claim {} collides with a Zellij {} default ({:?})",
            self.claim_id, self.zellij_mode, self.kind
        )
    }
}

/// Every overlap between the supplied claims and the supplied Zellij corpus,
/// in deterministic (claim order, corpus order).
///
/// The overlap test is sequence-prefix based: identical sequences collide,
/// and either sequence being a strict prefix of the other collides, because
/// Noren reads keys before the PTY and would hold or swallow the shared
/// leading chords. An empty result is the absence assertion the pass-through
/// requirement demands.
#[must_use]
pub fn collisions<'a>(
    claims: impl IntoIterator<Item = &'a PassthroughClaim>,
    corpus: &[ZellijBinding],
) -> Vec<Collision> {
    let mut found = Vec::new();
    for claim in claims {
        for binding in corpus {
            let zellij_seq = ChordSeq::single(binding.chord);
            let kind = if claim.seq == zellij_seq {
                CollisionKind::Exact
            } else if claim.seq.is_prefix_of(&zellij_seq) {
                CollisionKind::ClaimPrefixesZellij
            } else if zellij_seq.is_prefix_of(&claim.seq) {
                CollisionKind::ZellijPrefixesClaim
            } else {
                continue;
            };
            found.push(Collision {
                claim_id: claim.id,
                claim_seq: claim.seq.clone(),
                zellij_mode: binding.mode,
                zellij_seq,
                kind,
            });
        }
    }
    found
}

/// A recovery route out of pass-through.
///
/// The compatibility matrix requires an approved non-keyboard recovery
/// (pointer-invoked command palette or GUI/menu surface) to remain reachable
/// regardless of binding state or configuration validity; it is modeled here
/// as unconditionally present alongside the keyboard exit leader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryRoute {
    /// The configured keyboard exit leader.
    KeyboardExit(ChordSeq),
    /// The pointer-invoked command palette / GUI surface, independent of
    /// keyboard state.
    PointerInvokedPalette,
}

/// A validated pass-through manifest.
///
/// Construction is the enforcement point for the lane's requirements: the
/// manifest is minimal (exit leader plus at most one palette chord), every
/// claim is justified, no two claims shadow each other, and no claim
/// overlaps the pinned Zellij default corpus. A failed construction means
/// pass-through must not become active under that manifest — that is the
/// anti-trap guarantee, never a silent fallback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassthroughPolicy {
    exit: PassthroughClaim,
    palette: Option<PassthroughClaim>,
}

impl Default for PassthroughPolicy {
    fn default() -> Self {
        Self::default_policy()
    }
}

impl PassthroughPolicy {
    /// The frozen default: exactly one claim, the `Super+Escape` exit leader.
    /// Nothing else is claimed until a justified need exists.
    ///
    /// Built through [`PassthroughPolicy::try_new`] so the default passes the
    /// same collision, ambiguity, and justification validation as any
    /// configured manifest; the expect is defense-in-depth, guarded by
    /// `default_manifest_has_zero_collisions_with_zellij_defaults`.
    ///
    /// # Panics
    ///
    /// Only if the frozen default claim is edited into an invalid manifest.
    #[must_use]
    pub fn default_policy() -> Self {
        Self::try_new(vec![default_exit_claim()])
            .expect("the frozen default manifest is valid and collision-free")
    }

    /// Validate and build a policy from a claim manifest.
    ///
    /// Only [`CLAIM_ID_EXIT`] (required, exactly once) and
    /// [`CLAIM_ID_PALETTE`] (optional, at most once) are permitted; anything
    /// else is rejected before pass-through could ever activate.
    ///
    /// # Errors
    ///
    /// Returns the first [`PolicyError`] in deterministic validation order:
    /// unknown ids, id/action mismatches, duplicates, empty justifications,
    /// a missing exit claim, leader-prefix ambiguity, then corpus collisions.
    pub fn try_new(claims: Vec<PassthroughClaim>) -> Result<Self, PolicyError> {
        let mut exit: Option<PassthroughClaim> = None;
        let mut palette: Option<PassthroughClaim> = None;
        for claim in claims {
            let slot = match claim.id {
                CLAIM_ID_EXIT => &mut exit,
                CLAIM_ID_PALETTE => &mut palette,
                other => return Err(PolicyError::UnknownClaim(other.to_owned())),
            };
            let expected = if claim.id == CLAIM_ID_EXIT {
                PassthroughAction::ExitToWorkspace
            } else {
                PassthroughAction::OpenCommandPalette
            };
            if claim.action != expected {
                return Err(PolicyError::WrongActionForClaim {
                    id: claim.id,
                    expected,
                });
            }
            if slot.is_some() {
                return Err(PolicyError::DuplicateClaimId(claim.id));
            }
            if claim.justification.is_empty() {
                return Err(PolicyError::EmptyJustification { id: claim.id });
            }
            *slot = Some(claim);
        }

        let Some(exit_claim) = exit else {
            return Err(PolicyError::MissingExitClaim);
        };

        if let Some(palette_claim) = &palette
            && (exit_claim.seq.is_prefix_of(&palette_claim.seq)
                || palette_claim.seq.is_prefix_of(&exit_claim.seq))
        {
            return Err(PolicyError::AmbiguousLeader {
                prefix: if exit_claim.seq.is_prefix_of(&palette_claim.seq) {
                    CLAIM_ID_EXIT
                } else {
                    CLAIM_ID_PALETTE
                },
                extended: if exit_claim.seq.is_prefix_of(&palette_claim.seq) {
                    CLAIM_ID_PALETTE
                } else {
                    CLAIM_ID_EXIT
                },
            });
        }

        let manifest: Vec<&PassthroughClaim> =
            iter::once(&exit_claim).chain(palette.as_ref()).collect();
        let corpus = zellij_default_bindings();
        if let Some(collision) = collisions(manifest, &corpus).into_iter().next() {
            return Err(PolicyError::Collision(collision));
        }

        Ok(Self {
            exit: exit_claim,
            palette,
        })
    }

    /// The exit leader claim.
    #[must_use]
    pub const fn exit_claim(&self) -> &PassthroughClaim {
        &self.exit
    }

    /// The optional palette claim.
    #[must_use]
    pub const fn palette_claim(&self) -> Option<&PassthroughClaim> {
        self.palette.as_ref()
    }

    /// Every claimed sequence, exit leader first.
    #[must_use]
    pub fn claims(&self) -> Vec<&PassthroughClaim> {
        self.iter_claims().collect()
    }

    /// Borrowing iterator over the claims, exit leader first. Allocation-free
    /// because the gate walks it on every key press.
    fn iter_claims(&self) -> impl Iterator<Item = &PassthroughClaim> {
        iter::once(&self.exit).chain(self.palette.as_ref())
    }

    /// The recovery routes out of pass-through, always non-empty.
    ///
    /// The keyboard exit leader is present because construction requires it,
    /// and the pointer-invoked palette is present unconditionally by the
    /// compatibility matrix's non-keyboard recovery obligation.
    #[must_use]
    pub fn recovery_routes(&self) -> Vec<RecoveryRoute> {
        vec![
            RecoveryRoute::KeyboardExit(self.exit.seq.clone()),
            RecoveryRoute::PointerInvokedPalette,
        ]
    }
}

/// Whether one key press was interpreted by Noren or forwarded to the child.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateKind {
    /// The chord matched no claim prefix; forward it (and any replayed
    /// chords) byte-for-byte to the child.
    Forwarded,
    /// The chord extends a claimed leader but does not complete it; it is
    /// held, producing no bytes yet. A later completion intercepts; a
    /// mismatch or timeout replays the held chords as forwarding.
    Pending,
    /// The chord completed a claimed sequence; Noren consumes it and the
    /// child receives nothing for it.
    Intercepted(PassthroughAction),
}

/// The outcome of one pressed chord under a policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateDecision {
    /// How the pressed chord itself was routed.
    pub kind: GateKind,
    /// Previously held leader chords that must be forwarded to the child
    /// before this chord's own outcome, in order. Empty unless a partial
    /// leader match failed.
    pub replayed: Vec<Chord>,
}

/// Streaming matcher that applies a [`PassthroughPolicy`] to a sequence of
/// pressed chords.
///
/// Only key presses enter the gate. Releases always follow the forwarding
/// path (the byte encoder drops them anyway), and autorepeats are routed like
/// presses. The gate holds no timer; a caller owning the wall clock invokes
/// [`PassthroughGate::replay_timeout`] when a pending leader expires, and
/// forwards every returned chord.
#[derive(Clone, Debug, Default)]
pub struct PassthroughGate {
    pending: Vec<Chord>,
}

impl PassthroughGate {
    /// A gate with no held leader state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Chords currently held as a partial leader match.
    #[must_use]
    pub fn pending(&self) -> &[Chord] {
        &self.pending
    }

    /// Route one pressed chord.
    ///
    /// Allocation-free on the common path: claims are borrowed, and the
    /// pending prefix is matched in place rather than cloned into a
    /// candidate.
    pub fn press(&mut self, policy: &PassthroughPolicy, chord: Chord) -> GateDecision {
        let pending = self.pending.as_slice();
        for claim in policy.iter_claims() {
            let seq = claim.seq.chords();
            if seq.len() == pending.len() + 1
                && seq.starts_with(pending)
                && seq[pending.len()] == chord
            {
                self.pending.clear();
                return GateDecision {
                    kind: GateKind::Intercepted(claim.action),
                    replayed: Vec::new(),
                };
            }
        }
        if policy.iter_claims().any(|claim| {
            let seq = claim.seq.chords();
            seq.len() > pending.len() + 1 && seq.starts_with(pending) && seq[pending.len()] == chord
        }) {
            self.pending.push(chord);
            return GateDecision {
                kind: GateKind::Pending,
                replayed: Vec::new(),
            };
        }
        let replayed = std::mem::take(&mut self.pending);
        GateDecision {
            kind: GateKind::Forwarded,
            replayed,
        }
    }

    /// A pending leader expired on time. Returns the held chords in order so
    /// the caller can forward each one byte-for-byte; nothing is consumed.
    pub fn replay_timeout(&mut self) -> Vec<Chord> {
        std::mem::take(&mut self.pending)
    }
}
