//! Integration tests for the Zellij pass-through lane (M3-5, ADR 0003
//! boundary).
//!
//! The file lease for this lane excludes `lib.rs`, so the pass-through module
//! is included from source; an integration lane later declares it in the
//! crate. The tests bind the pass-through decision to the app's byte contract
//! through [`noren_app::KeyEncoder`]: unclaimed input must produce exactly
//! the bytes a direct encode produces, and claimed input must produce none.

#[path = "../src/passthrough.rs"]
mod passthrough;

use noren_app::{
    Arrow, CursorKeyMode, FunctionKey, InputMode, Key, KeyEncoder, KeyInput, KeyPhase,
    Modifiers as AppModifiers,
};
use passthrough::{
    CLAIM_ID_EXIT, CLAIM_ID_PALETTE, Chord, ChordSeq, CollisionKind, GateKind, KeyCode, Modifiers,
    PassthroughAction, PassthroughClaim, PassthroughGate, PassthroughPolicy, PolicyError,
    RecoveryRoute, SeqError, ZELLIJ_FIXTURE_COMMIT, ZELLIJ_FIXTURE_TAG, collisions,
    default_exit_claim,
};

// ── Mapping between the app key event and pass-through chords ───────────

fn function_key_number(key: FunctionKey) -> u8 {
    match key {
        FunctionKey::F1 => 1,
        FunctionKey::F2 => 2,
        FunctionKey::F3 => 3,
        FunctionKey::F4 => 4,
        FunctionKey::F5 => 5,
        FunctionKey::F6 => 6,
        FunctionKey::F7 => 7,
        FunctionKey::F8 => 8,
        FunctionKey::F9 => 9,
        FunctionKey::F10 => 10,
        FunctionKey::F11 => 11,
        FunctionKey::F12 => 12,
    }
}

fn key_code_of(key: Key) -> Option<KeyCode> {
    match key {
        Key::Character(' ') => Some(KeyCode::Space),
        Key::Character(character) => Some(KeyCode::Char(character)),
        Key::Enter => Some(KeyCode::Enter),
        Key::Backspace => Some(KeyCode::Backspace),
        Key::Tab => Some(KeyCode::Tab),
        Key::Escape => Some(KeyCode::Escape),
        Key::Arrow(Arrow::Up) => Some(KeyCode::Up),
        Key::Arrow(Arrow::Down) => Some(KeyCode::Down),
        Key::Arrow(Arrow::Left) => Some(KeyCode::Left),
        Key::Arrow(Arrow::Right) => Some(KeyCode::Right),
        Key::Delete => Some(KeyCode::Delete),
        Key::Insert => Some(KeyCode::Insert),
        Key::Home => Some(KeyCode::Home),
        Key::End => Some(KeyCode::End),
        Key::PageUp => Some(KeyCode::PageUp),
        Key::PageDown => Some(KeyCode::PageDown),
        Key::Function(function_key) => Some(KeyCode::Function(function_key_number(function_key))),
    }
}

fn modifiers_of(modifiers: AppModifiers) -> Modifiers {
    let mut mapped = Modifiers::empty();
    if modifiers.is_ctrl() {
        mapped = mapped.ctrl();
    }
    if modifiers.is_alt() {
        mapped = mapped.alt();
    }
    if modifiers.is_shift() {
        mapped = mapped.shift();
    }
    if modifiers.is_super() {
        mapped = mapped.super_key();
    }
    mapped
}

fn chord_of(input: KeyInput) -> Option<Chord> {
    Chord::new(key_code_of(input.key())?, modifiers_of(input.modifiers())).ok()
}

fn pressed(key: Key, modifiers: AppModifiers) -> KeyInput {
    KeyInput::new(key, KeyPhase::Pressed, modifiers)
}

// ── The pass-through byte pipeline under test ───────────────────────────

/// Applies the pass-through gate to app key events and emits exactly the PTY
/// bytes the child receives: every forwarded or replayed event goes through
/// the app byte encoder untouched; every intercepted event contributes none.
#[derive(Default)]
struct Harness {
    gate: PassthroughGate,
    held: Vec<KeyInput>,
    forward_bytes: Vec<Vec<u8>>,
}

impl Harness {
    fn step(&mut self, policy: &PassthroughPolicy, input: KeyInput, mode: InputMode) -> GateKind {
        if input.phase() == KeyPhase::Released {
            return GateKind::Forwarded;
        }
        let Some(chord) = chord_of(input) else {
            self.forward_bytes.push(Self::encode(input, mode));
            return GateKind::Forwarded;
        };
        let decision = self.gate.press(policy, chord);
        // The expected replay is computed from the harness's own held-stream,
        // independently of the decision, so a dropped or truncated replay can
        // never self-satisfy the assertion.
        let expected_replayed: Vec<Chord> = self
            .held
            .iter()
            .map(|held| chord_of(*held).expect("harness feeds mappable chords"))
            .collect();
        match decision.kind {
            GateKind::Forwarded => {
                assert_eq!(
                    decision.replayed, expected_replayed,
                    "a mismatch must replay every held leader chord, in order"
                );
                for held in self.held.drain(..) {
                    self.forward_bytes.push(Self::encode(held, mode));
                }
                self.forward_bytes.push(Self::encode(input, mode));
            }
            GateKind::Pending => {
                assert!(
                    decision.replayed.is_empty(),
                    "a pending leader must not replay anything yet"
                );
                self.held.push(input);
            }
            GateKind::Intercepted(_) => {
                assert!(
                    decision.replayed.is_empty(),
                    "a completed leader consumes its chords; nothing replays"
                );
            }
        }
        decision.kind
    }

    fn encode(input: KeyInput, mode: InputMode) -> Vec<u8> {
        KeyEncoder::encode_with(input, mode).unwrap_or_default()
    }

    fn timeout(&mut self, mode: InputMode) {
        let replayed = self.gate.replay_timeout();
        let expected: Vec<Chord> = self
            .held
            .iter()
            .map(|held| chord_of(*held).expect("harness feeds mappable chords"))
            .collect();
        assert_eq!(
            replayed, expected,
            "a timed-out leader must replay every held chord, in order"
        );
        for held in self.held.drain(..) {
            self.forward_bytes.push(Self::encode(held, mode));
        }
    }

    /// All bytes the child has received, in order.
    fn bytes(&self) -> Vec<u8> {
        self.forward_bytes.iter().flatten().copied().collect()
    }
}

// ── Minimal manifest ────────────────────────────────────────────────────

#[test]
fn default_manifest_claims_exactly_one_justified_chord() {
    let policy = PassthroughPolicy::default_policy();
    let claims = policy.claims();
    assert_eq!(
        claims.len(),
        1,
        "pass-through must claim as little as possible"
    );

    let exit = claims[0];
    assert_eq!(exit.id, CLAIM_ID_EXIT);
    assert_eq!(exit.action, PassthroughAction::ExitToWorkspace);
    assert!(
        !exit.justification.is_empty(),
        "every claimed chord must carry a justification"
    );
    assert_eq!(exit.seq.chords().len(), 1);
    assert_eq!(
        exit.seq.chords()[0],
        Chord::new(KeyCode::Escape, Modifiers::empty().super_key()).expect("normalized")
    );
    assert_eq!(exit, &default_exit_claim());
    assert!(policy.palette_claim().is_none());
}

// ── Corpus sanity: documented Zellij anchors, pinned version ───────────

#[test]
fn corpus_is_pinned_and_contains_the_documented_anchors() {
    assert_eq!(ZELLIJ_FIXTURE_TAG, "v0.44.3");
    assert_eq!(
        ZELLIJ_FIXTURE_COMMIT,
        "55a2121b73dce4be624cda425a960e893000777c"
    );

    let corpus = passthrough::zellij_default_bindings();
    assert!(corpus.len() >= 90, "corpus must not silently shrink");

    fn bound(corpus: &[passthrough::ZellijBinding], chord: Chord) -> bool {
        corpus.iter().any(|binding| binding.chord == chord)
    }
    let c = Modifiers::empty().ctrl();
    let a = Modifiers::empty().alt();

    // Anchors recorded in docs/compatibility/zellij.md: mode entry Ctrl p
    // (pane), Ctrl t (tab), Ctrl o (session), Ctrl g (locked), Alt bindings
    // shared outside locked mode, Session-mode d detach.
    assert!(bound(&corpus, Chord::new(KeyCode::Char('p'), c).unwrap()));
    assert!(bound(&corpus, Chord::new(KeyCode::Char('t'), c).unwrap()));
    assert!(bound(&corpus, Chord::new(KeyCode::Char('o'), c).unwrap()));
    assert!(bound(&corpus, Chord::new(KeyCode::Char('g'), c).unwrap()));
    assert!(bound(&corpus, Chord::new(KeyCode::Char('n'), a).unwrap()));
    assert!(bound(&corpus, Chord::new(KeyCode::Char('h'), a).unwrap()));
    assert!(bound(&corpus, Chord::new(KeyCode::Char('j'), a).unwrap()));
    assert!(bound(&corpus, Chord::new(KeyCode::Char('k'), a).unwrap()));
    assert!(bound(&corpus, Chord::new(KeyCode::Char('l'), a).unwrap()));
    assert!(bound(&corpus, Chord::new(KeyCode::Left, a).unwrap()));
    assert!(bound(&corpus, Chord::new(KeyCode::Down, a).unwrap()));
    assert!(bound(&corpus, Chord::new(KeyCode::Up, a).unwrap()));
    assert!(bound(&corpus, Chord::new(KeyCode::Right, a).unwrap()));
    assert!(bound(
        &corpus,
        Chord::new(KeyCode::Char('d'), Modifiers::empty()).unwrap()
    ));
    assert!(bound(&corpus, Chord::new(KeyCode::Char('f'), a).unwrap()));

    // No documented default occupies the Super/Command modifier space.
    assert!(
        corpus
            .iter()
            .all(|binding| !binding.chord.modifiers().is_super()),
        "the pinned corpus binds no Super/Cmd chord"
    );
}

// ── Collision assertions against Zellij's documented defaults ──────────

#[test]
fn default_manifest_has_zero_collisions_with_zellij_defaults() {
    let policy = PassthroughPolicy::default_policy();
    let corpus = passthrough::zellij_default_bindings();
    let found = collisions(policy.claims(), &corpus);
    assert!(
        found.is_empty(),
        "default pass-through manifest must not touch Zellij defaults: {found:?}"
    );
}

#[test]
fn collision_detector_flags_documented_zellij_chords() {
    let corpus = passthrough::zellij_default_bindings();

    // Ctrl g is documented (locked entry/unlock): an exact claim collides.
    let claim_g = PassthroughClaim {
        id: CLAIM_ID_EXIT,
        action: PassthroughAction::ExitToWorkspace,
        seq: ChordSeq::single(Chord::new(KeyCode::Char('g'), Modifiers::empty().ctrl()).unwrap()),
        justification: "test",
    };
    let found = collisions([&claim_g], &corpus);
    assert!(!found.is_empty());
    assert!(found.iter().all(|c| c.claim_id == CLAIM_ID_EXIT));
    assert!(found.iter().any(|c| c.kind == CollisionKind::Exact));

    // A two-chord leader starting on Ctrl g still swallows the documented
    // single chord: Zellij's sequence is a strict prefix of the claim.
    let claim_gg = PassthroughClaim {
        id: CLAIM_ID_EXIT,
        action: PassthroughAction::ExitToWorkspace,
        seq: ChordSeq::new(vec![
            Chord::new(KeyCode::Char('g'), Modifiers::empty().ctrl()).unwrap(),
            Chord::new(KeyCode::Char('g'), Modifiers::empty().ctrl()).unwrap(),
        ])
        .unwrap(),
        justification: "test",
    };
    let found = collisions([&claim_gg], &corpus);
    assert!(
        found
            .iter()
            .any(|c| c.kind == CollisionKind::ZellijPrefixesClaim)
    );

    // Ctrl p (pane mode entry) and bare d (session detach) collide too.
    for (code, modifiers) in [
        (KeyCode::Char('p'), Modifiers::empty().ctrl()),
        (KeyCode::Char('d'), Modifiers::empty()),
    ] {
        let claim = PassthroughClaim {
            id: CLAIM_ID_EXIT,
            action: PassthroughAction::ExitToWorkspace,
            seq: ChordSeq::single(Chord::new(code, modifiers).unwrap()),
            justification: "test",
        };
        assert!(
            !collisions([&claim], &corpus).is_empty(),
            "detector must flag {code:?} claim"
        );
    }

    // Complement control: a Super-space claim reports nothing.
    let found = collisions([PassthroughPolicy::default_policy().exit_claim()], &corpus);
    assert!(found.is_empty());
}

// ── Fail-closed construction: no trap, no overreach ────────────────────

fn claim_with_seq(id: &'static str, action: PassthroughAction, seq: ChordSeq) -> PassthroughClaim {
    PassthroughClaim {
        id,
        action,
        seq,
        justification: "test",
    }
}

fn super_chord(code: KeyCode, shift: bool) -> Chord {
    let mut modifiers = Modifiers::empty().super_key();
    if shift {
        modifiers = modifiers.shift();
    }
    Chord::new(code, modifiers).unwrap()
}

#[test]
fn policy_rejects_manifests_that_could_trap_or_overreach() {
    let exit_seq = ChordSeq::single(super_chord(KeyCode::Escape, false));

    // No manifest at all: no exit leader, no pass-through.
    assert_eq!(
        PassthroughPolicy::try_new(vec![]),
        Err(PolicyError::MissingExitClaim)
    );

    // Palette only (missing exit leader) is refused.
    assert_eq!(
        PassthroughPolicy::try_new(vec![claim_with_seq(
            CLAIM_ID_PALETTE,
            PassthroughAction::OpenCommandPalette,
            ChordSeq::single(super_chord(KeyCode::Char('p'), true)),
        )]),
        Err(PolicyError::MissingExitClaim)
    );

    // An exit leader on a documented Zellij chord is refused before
    // pass-through could activate.
    let colliding = claim_with_seq(
        CLAIM_ID_EXIT,
        PassthroughAction::ExitToWorkspace,
        ChordSeq::single(Chord::new(KeyCode::Char('g'), Modifiers::empty().ctrl()).unwrap()),
    );
    match PassthroughPolicy::try_new(vec![colliding]) {
        Err(PolicyError::Collision(collision)) => {
            assert_eq!(collision.claim_id, CLAIM_ID_EXIT);
        }
        other => panic!("expected collision rejection, got {other:?}"),
    }

    // Id whitelist: unknown claims cannot enter the pass-through scope.
    assert!(matches!(
        PassthroughPolicy::try_new(vec![claim_with_seq(
            "noren.tab.close",
            PassthroughAction::ExitToWorkspace,
            exit_seq.clone(),
        )]),
        Err(PolicyError::UnknownClaim(id)) if id == "noren.tab.close"
    ));

    // Id/action pairing is enforced.
    assert_eq!(
        PassthroughPolicy::try_new(vec![claim_with_seq(
            CLAIM_ID_EXIT,
            PassthroughAction::OpenCommandPalette,
            exit_seq.clone(),
        )]),
        Err(PolicyError::WrongActionForClaim {
            id: CLAIM_ID_EXIT,
            expected: PassthroughAction::ExitToWorkspace,
        })
    );

    // Duplicates are refused.
    let first = claim_with_seq(
        CLAIM_ID_EXIT,
        PassthroughAction::ExitToWorkspace,
        exit_seq.clone(),
    );
    let second = claim_with_seq(
        CLAIM_ID_EXIT,
        PassthroughAction::ExitToWorkspace,
        exit_seq.clone(),
    );
    assert_eq!(
        PassthroughPolicy::try_new(vec![first, second]),
        Err(PolicyError::DuplicateClaimId(CLAIM_ID_EXIT))
    );

    // An unjustified claim is refused.
    let mut unjustified = claim_with_seq(
        CLAIM_ID_EXIT,
        PassthroughAction::ExitToWorkspace,
        exit_seq.clone(),
    );
    unjustified.justification = "";
    assert_eq!(
        PassthroughPolicy::try_new(vec![unjustified]),
        Err(PolicyError::EmptyJustification { id: CLAIM_ID_EXIT })
    );

    // Prefix ambiguity between two claims is refused.
    assert_eq!(
        PassthroughPolicy::try_new(vec![
            claim_with_seq(
                CLAIM_ID_EXIT,
                PassthroughAction::ExitToWorkspace,
                exit_seq.clone(),
            ),
            claim_with_seq(
                CLAIM_ID_PALETTE,
                PassthroughAction::OpenCommandPalette,
                ChordSeq::new(vec![
                    super_chord(KeyCode::Escape, false),
                    super_chord(KeyCode::Char('p'), true),
                ])
                .unwrap(),
            ),
        ]),
        Err(PolicyError::AmbiguousLeader {
            prefix: CLAIM_ID_EXIT,
            extended: CLAIM_ID_PALETTE,
        })
    );
}

#[test]
fn chord_sequences_are_bounded_and_normalized() {
    assert_eq!(ChordSeq::new(vec![]), Err(SeqError::Empty));
    let too_long = vec![super_chord(KeyCode::Escape, false); 9];
    assert_eq!(
        ChordSeq::new(too_long),
        Err(SeqError::TooLong { max: 8, got: 9 })
    );

    // Case folding: one chord, one representation.
    let upper = Chord::new(KeyCode::Char('G'), Modifiers::empty()).unwrap();
    let lower = Chord::new(KeyCode::Char('g'), Modifiers::empty()).unwrap();
    assert_eq!(upper, lower);

    assert_eq!(
        Chord::new(KeyCode::Char('\n'), Modifiers::empty()),
        Err(passthrough::ChordError::ControlOrWhitespaceChar)
    );
    assert_eq!(
        Chord::new(KeyCode::Char(' '), Modifiers::empty()),
        Err(passthrough::ChordError::ControlOrWhitespaceChar)
    );
    assert_eq!(
        Chord::new(KeyCode::Function(0), Modifiers::empty()),
        Err(passthrough::ChordError::FunctionKeyOutOfRange)
    );
    assert_eq!(
        Chord::new(KeyCode::Function(25), Modifiers::empty()),
        Err(passthrough::ChordError::FunctionKeyOutOfRange)
    );
}

#[test]
fn exit_plus_optional_palette_is_the_maximal_manifest() {
    let policy = PassthroughPolicy::try_new(vec![
        claim_with_seq(
            CLAIM_ID_EXIT,
            PassthroughAction::ExitToWorkspace,
            ChordSeq::single(super_chord(KeyCode::Escape, false)),
        ),
        claim_with_seq(
            CLAIM_ID_PALETTE,
            PassthroughAction::OpenCommandPalette,
            ChordSeq::single(super_chord(KeyCode::Char('p'), true)),
        ),
    ])
    .expect("exit plus palette is the permitted maximum");
    assert_eq!(policy.claims().len(), 2);
    assert!(policy.palette_claim().is_some());
}

// ── Byte-for-byte forwarding of everything Noren does not claim ────────

fn forwarding_corpus() -> Vec<KeyInput> {
    let plain = AppModifiers::empty();
    let ctrl = AppModifiers::empty().ctrl();
    let alt = AppModifiers::empty().alt();
    let shift = AppModifiers::empty().shift();
    let mut events = Vec::new();

    for character in ['a', 'Z', '1', '.', '~', ' ', 'é', '界'] {
        events.push(pressed(Key::Character(character), plain));
    }
    for key in [
        Key::Enter,
        Key::Backspace,
        Key::Tab,
        Key::Escape,
        Key::Arrow(Arrow::Up),
        Key::Arrow(Arrow::Down),
        Key::Arrow(Arrow::Left),
        Key::Arrow(Arrow::Right),
        Key::Delete,
        Key::Insert,
        Key::Home,
        Key::End,
        Key::PageUp,
        Key::PageDown,
        Key::Function(FunctionKey::F1),
        Key::Function(FunctionKey::F5),
        Key::Function(FunctionKey::F12),
    ] {
        events.push(pressed(key, plain));
    }
    // Ctrl control bytes.
    for character in ['a', 'c', 'd', 'z', '[', '@', ' '] {
        events.push(pressed(Key::Character(character), ctrl));
    }
    // Alt as ESC prefix, including a chord Zellij itself binds (Alt f):
    // Noren does not claim it, so it must still be forwarded.
    for character in ['f', 'x', 'é'] {
        events.push(pressed(Key::Character(character), alt));
    }
    // Modifier combinations of named keys.
    events.push(pressed(Key::Tab, shift));
    events.push(pressed(Key::Arrow(Arrow::Up), shift));
    events.push(pressed(Key::Arrow(Arrow::Left), ctrl));
    events.push(pressed(Key::Arrow(Arrow::Right), alt));
    events.push(pressed(Key::Home, ctrl.shift()));
    events.push(pressed(Key::Delete, alt.ctrl()));
    events.push(pressed(Key::Enter, ctrl));
    events.push(pressed(Key::Backspace, ctrl));
    events.push(pressed(Key::Character('c'), alt.ctrl()));
    // Unclaimed Super chords: forwarded like anything else Noren does not
    // interpret (the encoder's own Super drop is a separate, documented
    // encoder limitation, not a pass-through transformation).
    events.push(pressed(
        Key::Character('x'),
        AppModifiers::empty().super_key(),
    ));
    events
}

#[test]
fn unbound_input_is_forwarded_byte_for_byte() {
    let policy = PassthroughPolicy::default_policy();
    for mode in [
        InputMode::normal(),
        InputMode::normal().with_cursor(CursorKeyMode::Application),
    ] {
        let mut harness = Harness::default();
        let mut oracle = Vec::new();
        for input in forwarding_corpus() {
            let kind = harness.step(&policy, input, mode);
            assert_eq!(
                kind,
                GateKind::Forwarded,
                "unclaimed chord must forward: {input:?}"
            );
            oracle.extend(KeyEncoder::encode_with(input, mode).unwrap_or_default());
        }
        assert_eq!(
            harness.bytes(),
            oracle,
            "forwarded bytes must match a direct encode exactly"
        );
        assert!(harness.gate.pending().is_empty());
    }
}

#[test]
fn claimed_exit_chord_is_interpreted_and_produces_no_bytes() {
    let policy = PassthroughPolicy::default_policy();
    let exit = pressed(Key::Escape, AppModifiers::empty().super_key());

    let mut harness = Harness::default();
    let kind = harness.step(&policy, exit, InputMode::normal());
    assert_eq!(
        kind,
        GateKind::Intercepted(PassthroughAction::ExitToWorkspace)
    );
    assert!(harness.bytes().is_empty());

    // Autorepeat of the claimed chord is consumed the same way, and input
    // after the intercept returns to byte-for-byte forwarding.
    let repeated = KeyInput::new(
        Key::Escape,
        KeyPhase::Repeat,
        AppModifiers::empty().super_key(),
    );
    let kind = harness.step(&policy, repeated, InputMode::normal());
    assert_eq!(
        kind,
        GateKind::Intercepted(PassthroughAction::ExitToWorkspace)
    );
    let next = pressed(Key::Character('x'), AppModifiers::empty());
    let kind = harness.step(&policy, next, InputMode::normal());
    assert_eq!(kind, GateKind::Forwarded);
    assert_eq!(harness.bytes(), b"x");
}

#[test]
fn releases_bypass_the_gate_and_follow_the_encoder_contract() {
    let policy = PassthroughPolicy::default_policy();
    let mut harness = Harness::default();
    let released = KeyInput::new(
        Key::Character('x'),
        KeyPhase::Released,
        AppModifiers::empty(),
    );
    let kind = harness.step(&policy, released, InputMode::normal());
    assert_eq!(kind, GateKind::Forwarded);
    assert_eq!(
        harness.bytes(),
        KeyEncoder::encode_with(released, InputMode::normal()).unwrap_or_default()
    );
    assert!(harness.gate.pending().is_empty());
}

#[test]
fn zellij_bound_chords_noren_does_not_claim_are_forwarded() {
    let policy = PassthroughPolicy::default_policy();
    let corpus = passthrough::zellij_default_bindings();
    let mut harness = Harness::default();
    let mut forwarded = 0;
    for binding in &corpus {
        let Some(app_modifiers) = app_modifiers_of(binding.chord.modifiers()) else {
            continue;
        };
        let Some(key) = app_key_of(binding.chord.code()) else {
            continue;
        };
        let input = pressed(key, app_modifiers);
        let kind = harness.step(&policy, input, InputMode::normal());
        assert_eq!(
            kind,
            GateKind::Forwarded,
            "Noren must forward Zellij's own {:?} chord: {binding:?}",
            binding.mode
        );
        forwarded += 1;
    }
    assert!(
        forwarded >= 60,
        "expected to forward most of the corpus, got {forwarded}"
    );
}

fn app_modifiers_of(modifiers: Modifiers) -> Option<AppModifiers> {
    if modifiers.is_super() {
        return None;
    }
    let mut mapped = AppModifiers::empty();
    if modifiers.is_ctrl() {
        mapped = mapped.ctrl();
    }
    if modifiers.is_alt() {
        mapped = mapped.alt();
    }
    if modifiers.is_shift() {
        mapped = mapped.shift();
    }
    Some(mapped)
}

fn app_key_of(code: KeyCode) -> Option<Key> {
    match code {
        KeyCode::Char(' ') => Some(Key::Character(' ')),
        KeyCode::Char(character) => Some(Key::Character(character)),
        KeyCode::Enter => Some(Key::Enter),
        KeyCode::Tab => Some(Key::Tab),
        KeyCode::Backspace => Some(Key::Backspace),
        KeyCode::Escape => Some(Key::Escape),
        KeyCode::Up => Some(Key::Arrow(Arrow::Up)),
        KeyCode::Down => Some(Key::Arrow(Arrow::Down)),
        KeyCode::Left => Some(Key::Arrow(Arrow::Left)),
        KeyCode::Right => Some(Key::Arrow(Arrow::Right)),
        KeyCode::Home => Some(Key::Home),
        KeyCode::End => Some(Key::End),
        KeyCode::PageUp => Some(Key::PageUp),
        KeyCode::PageDown => Some(Key::PageDown),
        KeyCode::Delete => Some(Key::Delete),
        KeyCode::Insert => Some(Key::Insert),
        KeyCode::Function(1) => Some(Key::Function(FunctionKey::F1)),
        KeyCode::Function(_) => None,
        KeyCode::Space => Some(Key::Character(' ')),
    }
}

// ── Leader sequences: pending, replay, timeout ─────────────────────────

/// Two-chord exit leader on printable chords absent from the pinned corpus
/// (bare `a` and `g` appear in no mode). Unlike Super chords these encode
/// non-empty bytes, so replay order is observable at the byte boundary: a
/// lost replay changes what the child receives, not just metadata.
fn printable_leader_policy() -> PassthroughPolicy {
    PassthroughPolicy::try_new(vec![claim_with_seq(
        CLAIM_ID_EXIT,
        PassthroughAction::ExitToWorkspace,
        ChordSeq::new(vec![
            Chord::new(KeyCode::Char('a'), Modifiers::empty()).unwrap(),
            Chord::new(KeyCode::Char('g'), Modifiers::empty()).unwrap(),
        ])
        .unwrap(),
    )])
    .expect("leader is collision-free against the pinned corpus")
}

fn leader_first() -> KeyInput {
    pressed(Key::Character('a'), AppModifiers::empty())
}

fn leader_second() -> KeyInput {
    pressed(Key::Character('g'), AppModifiers::empty())
}

#[test]
fn leader_completion_intercepts_and_mismatch_replays_in_order() {
    let policy = printable_leader_policy();
    let first = leader_first();
    let second = leader_second();
    let other = pressed(Key::Character('x'), AppModifiers::empty());

    // Completion: both chords consumed, zero bytes reach the child.
    let mut harness = Harness::default();
    assert_eq!(
        harness.step(&policy, first, InputMode::normal()),
        GateKind::Pending
    );
    assert!(harness.bytes().is_empty());
    assert_eq!(
        harness.step(&policy, second, InputMode::normal()),
        GateKind::Intercepted(PassthroughAction::ExitToWorkspace)
    );
    assert!(harness.bytes().is_empty());

    // Mismatch: the held leader chord must reach the child BEFORE the
    // mismatching chord, observed at the byte boundary. A lost or reordered
    // replay changes these bytes (the child would see "x" instead of "ax"),
    // which is exactly the no-lost-input property the matrix forbids
    // breaking.
    let mut harness = Harness::default();
    assert_eq!(
        harness.step(&policy, first, InputMode::normal()),
        GateKind::Pending
    );
    assert!(
        harness.bytes().is_empty(),
        "the held leader chord must emit no bytes yet"
    );
    assert_eq!(
        harness.step(&policy, other, InputMode::normal()),
        GateKind::Forwarded
    );
    assert!(harness.gate.pending().is_empty());
    assert_eq!(
        harness.bytes(),
        b"ax",
        "the held leader chord must be replayed byte-for-byte before the mismatching chord"
    );
}

#[test]
fn a_second_live_claim_does_not_swallow_a_held_leader_prefix() {
    // Exit is the two-chord leader [a, g]; palette is the single chord q.
    // Both chords are unbound in the pinned corpus and share no prefix, so
    // the manifest validates and both claims are live at once.
    let policy = PassthroughPolicy::try_new(vec![
        claim_with_seq(
            CLAIM_ID_EXIT,
            PassthroughAction::ExitToWorkspace,
            ChordSeq::new(vec![
                Chord::new(KeyCode::Char('a'), Modifiers::empty()).unwrap(),
                Chord::new(KeyCode::Char('g'), Modifiers::empty()).unwrap(),
            ])
            .unwrap(),
        ),
        claim_with_seq(
            CLAIM_ID_PALETTE,
            PassthroughAction::OpenCommandPalette,
            ChordSeq::single(Chord::new(KeyCode::Char('q'), Modifiers::empty()).unwrap()),
        ),
    ])
    .expect("manifest is collision-free and prefix-unambiguous");

    // A standalone palette chord intercepts.
    let mut harness = Harness::default();
    assert_eq!(
        harness.step(
            &policy,
            pressed(Key::Character('q'), AppModifiers::empty()),
            InputMode::normal()
        ),
        GateKind::Intercepted(PassthroughAction::OpenCommandPalette)
    );
    assert!(harness.bytes().is_empty());

    // A palette chord after a held exit prefix completes neither claim: the
    // held prefix must replay first, then the new chord forwards as literal
    // input. The byte stream proves the order.
    let mut harness = Harness::default();
    assert_eq!(
        harness.step(&policy, leader_first(), InputMode::normal()),
        GateKind::Pending
    );
    assert_eq!(
        harness.step(
            &policy,
            pressed(Key::Character('q'), AppModifiers::empty()),
            InputMode::normal()
        ),
        GateKind::Forwarded
    );
    assert!(harness.gate.pending().is_empty());
    assert_eq!(harness.bytes(), b"aq");
}

#[test]
fn leader_timeout_replays_held_chords_for_forwarding() {
    let policy = printable_leader_policy();
    let first = leader_first();

    let mut gate = PassthroughGate::new();
    let decision = gate.press(&policy, chord_of(first).unwrap());
    assert_eq!(decision.kind, GateKind::Pending);
    assert!(decision.replayed.is_empty());

    let replayed = gate.replay_timeout();
    assert_eq!(replayed, vec![chord_of(first).unwrap()]);
    assert!(gate.pending().is_empty());

    // After a timeout the gate is clean: an unmodified chord forwards.
    let plain = Chord::new(KeyCode::Char('x'), Modifiers::empty()).unwrap();
    let decision = gate.press(&policy, plain);
    assert_eq!(decision.kind, GateKind::Forwarded);
    assert!(decision.replayed.is_empty());

    // The byte pipeline expresses the same timeout: held chords are replayed
    // through the encoder instead of being consumed. The printable leader
    // makes this byte-observable: a dropped replay would leave an empty
    // stream where the child is owed b"a".
    let mut harness = Harness::default();
    assert_eq!(
        harness.step(&policy, first, InputMode::normal()),
        GateKind::Pending
    );
    harness.timeout(InputMode::normal());
    assert!(harness.gate.pending().is_empty());
    assert_eq!(
        harness.bytes(),
        b"a",
        "a timed-out leader must be forwarded byte-for-byte"
    );
}

// ── Recovery: no trapped user ──────────────────────────────────────────

#[test]
fn recovery_routes_are_always_reachable() {
    for policy in [
        PassthroughPolicy::default_policy(),
        PassthroughPolicy::try_new(vec![
            claim_with_seq(
                CLAIM_ID_EXIT,
                PassthroughAction::ExitToWorkspace,
                ChordSeq::single(super_chord(KeyCode::Escape, false)),
            ),
            claim_with_seq(
                CLAIM_ID_PALETTE,
                PassthroughAction::OpenCommandPalette,
                ChordSeq::single(super_chord(KeyCode::Char('p'), true)),
            ),
        ])
        .expect("valid manifest"),
    ] {
        let routes = policy.recovery_routes();
        assert!(
            routes.contains(&RecoveryRoute::KeyboardExit(
                policy.exit_claim().seq.clone()
            )),
            "the keyboard exit leader must be a recovery route"
        );
        assert!(
            routes.contains(&RecoveryRoute::PointerInvokedPalette),
            "the pointer-invoked palette must remain reachable regardless of keyboard state"
        );
    }
}

#[test]
fn an_invalid_manifest_means_pass_through_is_not_enterable() {
    // The anti-trap invariant, conversely: every manifest the validator
    // rejects can never activate pass-through, so a disabled, invalid, or
    // colliding leader never yields a session with no recovery route.
    let invalid = [
        vec![],
        vec![claim_with_seq(
            CLAIM_ID_EXIT,
            PassthroughAction::ExitToWorkspace,
            ChordSeq::single(Chord::new(KeyCode::Char('g'), Modifiers::empty().ctrl()).unwrap()),
        )],
    ];
    for manifest in invalid {
        assert!(PassthroughPolicy::try_new(manifest).is_err());
    }
}
