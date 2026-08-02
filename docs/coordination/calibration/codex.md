# Summary

A standard-library-first Rust library should analyze declarative bindings without intercepting input. It models single chords and multi-chord leader sequences, evaluates scope/platform overlap, applies Noren’s terminal/pass-through policies, and returns stable, machine-readable diagnostics.

Disabled bindings and GUI-only actions do not participate in keyboard-conflict detection. Every enabled Noren shortcut must be configurable and disableable.

# API

```rust
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Platform { MacOs, Linux }

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Modifier { Control, Alt, Shift, Super }

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Key {
    Character(char), Enter, Escape, Tab, Backspace,
    ArrowUp, ArrowDown, ArrowLeft, ArrowRight,
    Function(u8),
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct KeyChord {
    pub modifiers: BTreeSet<Modifier>,
    pub key: Key,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct KeySequence(pub Vec<KeyChord>);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Scope {
    GlobalGui,
    TerminalPane,
    CommandPalette,
    ZellijPassThrough,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Owner {
    Noren,
    Zellij,
    Tmux,
    Vim,
    Neovim,
    Shell,
    OperatingSystem,
    Other(String),
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Source {
    pub id: String,
    pub owner: Owner,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingState { Enabled, Disabled }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NorenOrigin { Default, ExplicitUser }

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NorenOptions {
    pub configurable: bool,
    pub disableable: bool,
    pub state: BindingState,
    pub origin: NorenOrigin,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Activation {
    Keyboard(KeySequence),
    GuiOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Binding {
    pub source: Source,
    pub activation: Activation,
    pub platforms: BTreeSet<Platform>,
    pub scopes: BTreeSet<Scope>,
    pub noren: Option<NorenOptions>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PassThroughPolicy {
    pub exit_leader_source_id: String,
    pub palette_source_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShadowRule {
    pub source_id: String,
    pub platform: Platform,
    pub sequence: KeySequence,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DiagnosticKind {
    ExactCollision,
    PrefixAmbiguity,
    PlatformShadowing,
    DuplicateNorenBinding,
    InvalidDefaultCapture,
    InvalidPassThroughCapture,
    InvalidNorenConfiguration,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Severity { Error, Warning }

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub severity: Severity,
    pub source_ids: Vec<String>,
    pub platforms: BTreeSet<Platform>,
    pub scopes: BTreeSet<Scope>,
    pub reason: String,
    pub remediation: String,
}

pub struct AnalysisInput<'a> {
    pub bindings: &'a [Binding],
    pub shadow_rules: &'a [ShadowRule],
    pub pass_through: &'a PassThroughPolicy,
}

pub fn normalize_chord(chord: KeyChord) -> Result<KeyChord, ValidationError>;
pub fn analyze(input: AnalysisInput<'_>) -> Vec<Diagnostic>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidationError {
    EmptySequence,
    InvalidFunctionKey(u8),
    InvalidCharacter(char),
}
```

# Invariants

- Character keys use a documented canonical form, such as lowercase Unicode characters; Shift remains explicit.
- Modifier sets contain no duplicates. Function-key range is fixed and documented, for example F1–F24.
- Sequences are non-empty.
- Source IDs are unique and stable.
- A Noren-owned binding has `noren: Some`; other owners have `None`.
- Every Noren binding has both `configurable` and `disableable` set to `true`; violations are errors.
- Disabled bindings are ignored after configuration validation.
- A default Noren binding active in `TerminalPane` cannot begin with Control, Alt, Control+Alt, or a function key. `ExplicitUser` bindings may.
- Pass-through permits only the configured exit leader and optional palette source. `GuiOnly` bindings never conflict there.
- Diagnostics and embedded source IDs are lexicographically sorted and deduplicated.

# Algorithm

1. Validate and normalize bindings.
2. Remove disabled and GUI-only bindings from keyboard comparisons.
3. Enforce terminal default-capture and pass-through allowlist rules.
4. Expand each binding across its platform/scope sets into indexed records.
5. Group records by normalized sequence to find exact collisions. Two enabled Noren records with the same sequence and overlapping context produce `DuplicateNorenBinding`; otherwise `ExactCollision`.
6. Build a sequence trie per platform/scope. If one sequence ends at an ancestor of another, report `PrefixAmbiguity`.
7. Compare bindings with `ShadowRule`s. Report `PlatformShadowing` only for the rule’s platform.
8. Merge equivalent findings across platforms/scopes, then sort by kind, source IDs, platforms, scopes, reason, and remediation.

Scope overlap is explicit: identical scopes overlap; `GlobalGui` overlaps keyboard scopes except that GUI-only actions are excluded. `CommandPalette` overlaps only while the palette is active. Pass-through is treated as replacing ordinary terminal capture.

With `n` bindings and total `k` chords, indexing and trie construction are `O(k log n)` using standard-library ordered maps; diagnostic output can be `O(n²)` when many bindings collide.

# Test Matrix

| Case | Inputs | Expected |
|---|---|---|
| 1 | Noren `Ctrl-b`, tmux `Ctrl-b`, Terminal Pane, Linux | Exact collision |
| 2 | Noren default `Alt-x`, Terminal Pane | Invalid default capture |
| 3 | Same binding marked ExplicitUser | Accepted |
| 4 | Zellij leader `Ctrl-g`; Noren `Ctrl-g p` | Prefix ambiguity |
| 5 | Two Noren actions on `Super-k`, Global GUI, macOS | Duplicate Noren binding |
| 6 | `Super-Space` plus macOS OS shadow rule | Platform shadowing on macOS only |
| 7 | Same chord in Terminal Pane and inactive Command Palette | Acceptable non-overlap |
| 8 | Disabled Noren binding collides with Vim | No collision |
| 9 | Pass-through Noren exit leader | Accepted |
| 10 | Pass-through arbitrary Noren terminal action | Invalid pass-through capture |
| 11 | Pass-through GUI-only action sharing a chord label | No keyboard conflict |
| 12 | Noren binding not disableable | Invalid Noren configuration |
| 13 | Linux-only tmux binding versus macOS-only Noren binding | Acceptable non-overlap |
| 14 | `F5` passed to shell with no explicit Noren binding | No capture/conflict |

# Property Tests

- Permuting bindings, platform sets, modifier insertion order, or shadow rules must produce byte-for-byte identical diagnostics.
- For any valid dataset, disabling a Noren binding cannot introduce a new collision, ambiguity, or shadowing diagnostic.

A property-testing crate such as `proptest` is optional and its current API/version must be verified before adoption.

# Security and Reliability

Treat configuration as untrusted: cap source-ID length, sequence length, binding count, and diagnostic count. Reject control characters in IDs. Avoid locale-dependent normalization and hash-order-dependent output. Never infer third-party defaults; accept Zellij, tmux, editor, shell, and OS bindings as caller-supplied data. Fail closed on malformed Noren configuration, but preserve terminal input when runtime configuration cannot be loaded.

# Uncertainties

Unicode keyboard normalization, Alt versus Meta behavior, macOS Command handling, keyboard-layout translation, terminal escape-sequence timing, and actual third-party defaults require platform/runtime validation. Scope overlap may need refinement if Command Palette execution suspends terminal delivery.

# Deferred Work

Runtime event interception, configuration parsing, keyboard-layout APIs, escape-sequence decoding, automatic tmux/Zellij/Vim discovery, UI presentation, conflict-resolution editing, persistence, telemetry, and verified integrations with third-party crates are deliberately excluded.
