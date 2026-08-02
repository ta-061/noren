# Summary

A standard-library-only API can model Noren and third-party bindings as normalized key sequences with explicit platform and scope sets. Analysis is deterministic and conservative: it reports conflicts only where platform and activation scope overlap, while enforcing stricter terminal-pane and Zellij pass-through policies.

The library describes conflicts; it does not register shortcuts, parse application configuration files, or assume undocumented Zellij, tmux, Vim, Neovim, or shell behavior.

# API

```rust
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Platform { MacOs, Linux }

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Scope {
    GlobalGui,
    TerminalPane,
    CommandPalette,
    ZellijPassThrough,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Modifier { Control, Alt, Shift, Super }

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Key {
    Character(char),
    Enter,
    Escape,
    Tab,
    Backspace,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Function(u8),
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct KeyStroke {
    pub modifiers: BTreeSet<Modifier>,
    pub key: Key,
}

/// One or more strokes; leaders are represented as sequence prefixes.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct KeyChord(pub Vec<KeyStroke>);

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SourceId(pub String);

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ExternalOwner {
    Zellij,
    Tmux,
    Vim,
    Neovim,
    Shell,
    OperatingSystem,
    Other(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BindingOwner {
    Noren { action: String },
    External(ExternalOwner),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NorenOrigin { Default, UserExplicit }

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BindingState {
    External,
    Noren {
        enabled: bool,
        configurable: bool,
        disableable: bool,
        origin: NorenOrigin,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionClass {
    GuiOnly,
    TerminalInputCapture,
    OpenCommandPalette,
    ExitPassThrough,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Binding {
    pub id: SourceId,
    pub owner: BindingOwner,
    pub chord: KeyChord,
    pub platforms: BTreeSet<Platform>,
    pub scopes: BTreeSet<Scope>,
    pub state: BindingState,
    pub action_class: ActionClass,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Policy {
    pub pass_through_exit: SourceId,
    pub pass_through_palette: Option<SourceId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Severity { Error, Warning, Info }

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum DiagnosticKind {
    ExactCollision,
    PrefixAmbiguity,
    PlatformSpecificShadowing,
    DuplicateNorenBinding,
    ForbiddenDefaultCapture,
    InvalidPassThroughCapture,
    AcceptableNonOverlap,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub severity: Severity,
    pub source_ids: Vec<SourceId>,
    pub reason: String,
    pub platforms: BTreeSet<Platform>,
    pub scopes: BTreeSet<Scope>,
    pub remediation: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidationError {
    EmptyChord,
    EmptyPlatformSet(SourceId),
    EmptyScopeSet(SourceId),
    InvalidFunctionKey(SourceId),
    NonConfigurableNorenBinding(SourceId),
    UnknownPolicyBinding(SourceId),
}

pub fn normalize(chord: KeyChord) -> Result<KeyChord, ValidationError>;

pub fn analyze(
    bindings: &[Binding],
    policy: &Policy,
    include_acceptable: bool,
) -> Result<Vec<Diagnostic>, Vec<ValidationError>>;
```

# Invariants

- Chords contain at least one stroke; function-key numbers use a documented supported range.
- Character keys use one documented Unicode case-normalization rule; modifiers are sets, so order and duplication are irrelevant.
- Every binding has at least one platform and scope.
- Disabled Noren bindings participate in validation but not conflict detection.
- Every Noren binding satisfies `configurable || disableable`.
- A default Noren binding active in `TerminalPane` cannot capture Control, Alt, Control+Alt, or any function key. `UserExplicit` bindings may.
- In `ZellijPassThrough`, enabled keyboard bindings are restricted to the configured exit leader and optional palette binding.
- `GuiOnly` actions never conflict in pass-through scope.
- Source IDs are unique and stable.

# Algorithm

1. Normalize and validate all bindings and policy references.
2. Remove disabled bindings from collision analysis.
3. Expand each binding into platform/scope activation tuples, excluding GUI-only/pass-through pairs.
4. Compare bindings sharing a tuple:
   - Equal chords between two Noren sources: `DuplicateNorenBinding`.
   - Equal chords involving different owners: `ExactCollision`.
   - One multi-stroke chord is a strict prefix of another: `PrefixAmbiguity`.
   - No shared tuple: optionally `AcceptableNonOverlap`.
5. If a collision affects only a proper subset of either binding’s declared platforms, emit or annotate `PlatformSpecificShadowing`.
6. Emit policy violations independently.
7. Sort by severity, kind, platform, scope, normalized chord, then source IDs; sort and deduplicate IDs and reason data.

A straightforward implementation is `O(n² × s)` time and `O(n + d)` space, where `s` is maximum sequence length and `d` diagnostics. Indexing by platform, scope, and first stroke can reduce typical comparisons without changing results.

# Test Matrix

| Case | Bindings | Expected |
|---|---|---|
| 1 | Noren default `Ctrl-A` in Terminal Pane | `ForbiddenDefaultCapture` |
| 2 | User-explicit `Ctrl-A` vs shell `Ctrl-A`, same scope/platform | `ExactCollision` |
| 3 | Noren `Ctrl-B` vs tmux `Ctrl-B,C` | `PrefixAmbiguity` |
| 4 | Two Noren actions use `Super-P` globally on macOS | `DuplicateNorenBinding` |
| 5 | `Super-P` collision on macOS, disjoint on Linux | `PlatformSpecificShadowing` |
| 6 | Same chord in Global GUI and Terminal Pane only | `AcceptableNonOverlap` |
| 7 | GUI-only action and Zellij input share chord in pass-through | No conflict |
| 8 | Unapproved Noren action active in pass-through | `InvalidPassThroughCapture` |
| 9 | Configured exit leader vs Zellij leader | `ExactCollision` |
| 10 | Disabled Noren binding matches Vim binding | No collision |
| 11 | Default `F5` in Terminal Pane | `ForbiddenDefaultCapture` |
| 12 | Noren binding neither configurable nor disableable | Validation error |

Each case should assert the complete structured diagnostic and deterministic ordering.

# Property Tests

- Permuting bindings, scopes, platforms, or modifier insertion order must produce identical normalized diagnostics.
- For any binding pair, adding a previously disjoint platform/scope cannot remove an existing conflict; disabling either Noren binding must remove pairwise collision diagnostics.

Property-test crates such as `proptest` are optional and require API/version verification; deterministic generated loops can use only `std`.

# Security and Reliability

Bound chord length, source-ID length, binding count, and diagnostic count to prevent configuration-driven CPU or memory exhaustion. Reject control characters in displayable IDs. Avoid locale-dependent normalization. Treat malformed configuration as validation failure, never as permission to capture input. Keep analysis pure and side-effect free, and never log typed terminal content.

# Uncertainties

OS-reserved shortcuts, keyboard-layout translation, Alt/Meta behavior, terminal emulator encoding, and actual leader timeouts vary. External bindings must come from user configuration or verified adapters; built-in assumptions should be labeled by version and provenance.

# Deferred Work

Configuration-file parsing, live key-event translation, layout-aware physical keys, leader timeout modeling, conflict-resolution UI, importing third-party configs, and platform accessibility shortcut databases are deliberately deferred.
