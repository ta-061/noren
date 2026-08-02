# Summary

A small, std-only Rust crate (`noren-keys`) that takes a normalized description of all keybindings (Noren + hosted apps: Zellij, tmux, Vim, Neovim, shell) plus a capture policy, and returns deterministic, structured conflict diagnostics. It distinguishes exact collisions, leader/prefix ambiguity, platform-specific shadowing, duplicate Noren bindings, policy violations, and acceptable non-overlaps. No external crates are required; any optional crate is explicitly marked as needing verification.

# API

```rust
use std::borrow::Cow;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Modifiers(pub u8); // bits: CTRL=1 ALT=2 SHIFT=4 SUPER=8
impl Modifiers {
    pub const CTRL: Self = Self(1);
    pub const ALT:  Self = Self(2);
    pub const SHIFT:Self = Self(4);
    pub const SUPER:Self = Self(8);
    pub fn contains(self, other: Self) -> bool { self.0 & other.0 == other.0 }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    Char(char), Function(u8), Enter, Tab, Backspace, Escape, Space,
    Arrow(Cardinal), Home, End, PageUp, PageDown, Insert, Delete,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Cardinal { Up, Down, Left, Right }

/// A normalized single key chord; invariant: letters are lower-case,
/// SHIFT folded into Key::Char casing only for printable letters.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeyChord { pub mods: Modifiers, pub key: Key }
impl KeyChord {
    pub fn normalize(input: KeyChord) -> Self; // canonical form
}

/// One binding may be a multi-key sequence (Vim-style leader).
/// Single-chord bindings are length-1 sequences.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeySequence(pub Vec<KeyChord>); // length >= 1, chords normalized

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Platform { Macos, Linux, All }
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scope { GlobalGui, TerminalPane, CommandPalette, ZellijPassThrough }

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum BindingSource {
    Noren(NorenId), // NorenId = Cow<'static,str>
    Zellij, Tmux, Vim, Neovim, Shell,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BindingOrigin { NorenDefault, UserExplicit }

#[derive(Clone, PartialEq, Eq)]
pub struct BindingEntry {
    pub source: BindingSource,
    pub seq: KeySequence,
    pub scope: Scope,
    pub platforms: Platform,
    pub origin: BindingOrigin,
    pub disabled: bool,
}

/// Encodes req. 3 (focused pane) and req. 4 (pass-through) capture rules.
#[derive(Clone, Copy)]
pub struct CapturePolicy {
    pub terminal_forward_default: bool, // true => Ctrl/Alt/Ctrl+Alt/F-keys forward
    pub passthrough_exit_leader: KeySequence,
    pub passthrough_palette: Option<KeySequence>,
}
impl CapturePolicy {
    /// True if Noren may capture this single chord by default in the scope.
    pub fn captures_by_default(self, c: KeyChord, scope: Scope) -> bool;
    /// Live Noren sequences in pass-through mode (leader + optional palette).
    pub fn passthrough_live(self) -> &'static [KeySeqRef];
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity { Info, Warning, Error }

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    ExactCollision, LeaderPrefixAmbiguity, PlatformShadowing,
    DuplicateNoren, DefaultCaptureViolation, AcceptableNonOverlap,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub reason: Reason,
    pub severity: Severity,
    pub source_ids: Vec<BindingSource>,
    pub platforms: Platform,
    pub scopes: Vec<Scope>,
    pub remediation: Remediation,
    pub detail: Cow<'static, str>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Remediation {
    RebindNoren(NorenId),
    RestrictPlatform(Platform),
    MakeUserExplicit,
    DisableBinding(NorenId),
    NoneNeeded,
}

#[derive(Clone)]
pub struct ConflictReport { pub diagnostics: Vec<Diagnostic> }

/// Entry point. Pure, deterministic, allocation-bounded by input size.
pub fn detect_conflicts(
    bindings: &[BindingEntry],
    policy: &CapturePolicy,
) -> ConflictReport;

/// Verifies Noren-side invariant: every Noren binding is configurable or disabled.
pub fn validate_noren_configurability(bindings: &[BindingEntry]) -> Result<(), NorenId>;
```

Optional: `IndexMap` / `trie` crates could speed up prefix checks, but **not used** — verify APIs before adoption.

# Invariants

1. `KeySequence` length ≥ 1; every `KeyChord` is in `normalize` form (letters lower-cased; `Shift+letter` represented by `Key::Char(uppercase)`).
2. Every `BindingSource::Noren(_)` entry must be configurable (`origin` known) or `disabled == true`; `validate_noren_configurability` enforces req. 7.
3. In `Scope::TerminalPane`, any `BindingOrigin::NorenDefault` whose chord has `CTRL`, `ALT`, both, or `Key::Function(_)` is a `DefaultCaptureViolation` (req. 3). `UserExplicit` overrides are allowed.
4. In `Scope::ZellijPassThrough`, only `policy.passthrough_exit_leader` and `policy.passthrough_palette` are live; `Scope::GlobalGui` actions are never treated as keyboard conflicts (req. 4).
5. `Platform::All` expands to `{Macos, Linux}` for all collision/shadow math; output collapses back when both columns agree.
6. `detect_conflicts` is total: never panics on malformed input (unknown chords are dropped with an `Info` diagnostic).

# Algorithm

1. **Normalize & filter**: `O(N)` — canonicalize all sequences; drop disabled entries from collision math (they still appear as `Info`).
2. **Expand platforms**: each entry yields ≤ 2 concrete `(scope, platform)` keys.
3. **Exact collisions**: bucket entries by `(scope, platform, seq)`. Buckets with ≥ 2 distinct `BindingSource` → `ExactCollision` (`Error`). Two `Noren` sources in one bucket → `DuplicateNoren` (`Error`).
4. **Prefix ambiguity**: per `(scope, platform)`, sort sequences lexicographically and build a trie. If sequence A is a strict prefix of B → `LeaderPrefixAmbiguity` (`Warning`), unless B is `UserExplicit` and A is host-app (then `AcceptableNonOverlap`).
5. **Platform shadowing**: for an `ExactCollision` present on only one concrete platform while Noren declared `All` → relabel `PlatformShadowing` (`Warning`), remediation `RestrictPlatform`.
6. **Policy checks**: apply `CapturePolicy::captures_by_default` to each Noren default in `TerminalPane`; `SUPER`-only chords and GUI-only actions are exempt → `AcceptableNonOverlap` (`Info`).
7. **Sort & dedupe** diagnostics by `(reason, severity, source_ids, platforms)` for deterministic output.

Complexity: `O(N log N + K)` where `K` = total chords across sequences (trie build). Determinism via total ordering; no `HashMap` iteration order leaks into output.

# Test Matrix

Table-driven via `&[TestCase]` where `TestCase { bindings, policy, expect: Expect }`.

| # | Setup | Expected |
|---|-------|----------|
| 1 | Noren `Ctrl-T` & Zellij `Ctrl-T`, TerminalPane, All | `ExactCollision`, Error |
| 2 | Noren `Cmd-K` (Super) vs Vim `k`, TerminalPane | `AcceptableNonOverlap`, Info |
| 3 | Noren `Ctrl-X` & Noren `Ctrl-X Ctrl-F`, CommandPalette | `LeaderPrefixAmbiguity`, Warning |
| 4 | Two Noren bindings same seq/scope/platform | `DuplicateNoren`, Error |
| 5 | NorenDefault `Alt-L`, TerminalPane | `DefaultCaptureViolation`, Error |
| 6 | Noren `Cmd+Shift+P` All vs macOS GUI same | `PlatformShadowing`, Warning → Linux |
| 7 | Pass-through: Noren GlobalGui `Ctrl-O` | `AcceptableNonOverlap` (GUI-only), Info |
| 8 | Disabled Noren binding vs Zellij same chord | suppressed + `Info`, no Error |
| 9 | NorenDefault `F11` fullscreen, TerminalPane | `DefaultCaptureViolation`, Error |
| 10 | Same chord, `GlobalGui` vs `CommandPalette` | `AcceptableNonOverlap`, Info |

# Property Tests

1. **Determinism/order-invariance**: for any permutation of `bindings`, `detect_conflicts` yields an identical, sorted `diagnostics` vector (commutative + idempotent).
2. **Clean-set soundness**: if no two entries share `(scope, platform, seq)`, no sequence is a prefix of another, and all Noren defaults satisfy `captures_by_default`, then the report contains no `Error`/`Warning`.

# Security and Reliability

- Config parsing lives outside the crate; callers pass already-validated `BindingEntry`s, so untrusted parsing DoS is bounded by input length. Recommend callers cap sequence length (e.g., 8) and `Vec` sizes.
- `detect_conflicts` is pure and allocation-bounded; safe to run on every config edit.
- Deterministic ordering prevents user-facing nondeterminism and makes snapshot tests stable.
- macOS `Option`/`Alt` composition is normalized at the frontend boundary; the crate treats `ALT` uniformly and documents the assumption.
- No `unsafe`, no I/O, no threads.

# Uncertainties

- Whether `Alt+letter` producing a composed Unicode char on macOS should count as `ALT` or a literal `Char` — needs frontend verification.
- Source of truth for host-app bindings: assumed caller-provided static tables (zellij.kdl / tmux.conf / vim maps). The crate does not parse these.
- Whether `Function` keys F13–F24 are reachable on target keyboards.
- Semantics of "user explicitly binds it" (req. 3): whether `UserExplicit` is sufficient or also requires an opt-in flag.

# Deferred Work

- Parsing real host-app config files into `BindingEntry`.
- Remediation UI / suggestion ranking.
- Multi-key sequence timeouts and cancel gestures.
- Per-pane focus sub-scoping beyond the four modelled scopes.
- International layout / dead-key handling beyond macOS–Linux.
- Fuzzing harness for the normalizer and trie.
