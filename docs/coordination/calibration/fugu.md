# Summary

A standard-library-only analyzer compares normalized Noren and external key sequences where platforms and activation contexts overlap. It enforces terminal-preservation and pass-through policies and returns deterministic, actionable diagnostics without performing I/O or registering shortcuts.

# API

```rust
use std::{collections::BTreeSet, num::NonZeroU8};

pub struct SourceId(String);

pub enum Platform { MacOs, Linux }

pub enum ActivationScope {
    GlobalGui,
    TerminalPane,
    CommandPalette,
    ZellijPassThrough,
}

pub enum Modifier { Control, Alt, Shift, Super }

pub enum Key {
    Char(char),
    Function(NonZeroU8),
    Enter, Escape, Tab, Backspace,
    Up, Down, Left, Right,
    Home, End, PageUp, PageDown,
    Insert, Delete,
}

pub struct KeyChord {
    key: Key,
    modifiers: BTreeSet<Modifier>,
}

pub struct KeySequence(Box<[KeyChord]>);

pub enum ExternalOwner {
    Zellij,
    Tmux,
    Vim,
    Neovim,
    Shell,
    Other(String),
}

pub enum NorenOrigin { BuiltInDefault, ExplicitUser }

pub enum NorenRole {
    Ordinary,
    PassThroughExitLeader,
    OpenCommandPalette,
}

pub struct NorenMeta {
    pub origin: NorenOrigin,
    pub configurable: bool,
    pub disableable: bool,
    pub disabled: bool,
}

pub enum BindingOwner {
    Noren { meta: NorenMeta, role: NorenRole },
    External(ExternalOwner),
}

pub enum Trigger {
    Keyboard(KeySequence),
    GuiOnly,
}

pub struct Binding {
    pub id: SourceId,
    pub owner: BindingOwner,
    pub trigger: Trigger,
    pub platforms: BTreeSet<Platform>,
    pub scopes: BTreeSet<ActivationScope>,
}

pub enum Severity { Error, Warning }

pub enum SequenceOverlap {
    Exact,
    Prefix { leader: SourceId },
}

pub enum DiagnosticReason {
    ExactCollision,
    LeaderPrefixAmbiguity { leader: SourceId },
    PlatformSpecificShadowing { overlap: SequenceOverlap },
    DuplicateNorenBinding,
    UnsafeDefaultTerminalCapture,
    ForbiddenPassThroughCapture,
    NonConfigurableExitLeader,
    MultiplePassThroughBindings { role: NorenRole },
}

pub enum Remediation {
    ChangeOrDisableNoren { ids: Vec<SourceId> },
    RequireExplicitUserBinding { id: SourceId },
    RestrictScopes { ids: Vec<SourceId> },
    KeepOneNorenBinding { ids: Vec<SourceId> },
    MakeExitLeaderConfigurable { id: SourceId },
}

pub struct Diagnostic {
    pub source_ids: Vec<SourceId>,
    pub severity: Severity,
    pub reason: DiagnosticReason,
    pub affected_platforms: BTreeSet<Platform>,
    pub affected_scopes: BTreeSet<ActivationScope>,
    pub remediation: Remediation,
}

pub struct Report {
    pub diagnostics: Vec<Diagnostic>,
}

pub struct Limits {
    pub max_bindings: usize,
    pub max_sequence_len: usize,
    pub max_source_id_bytes: usize,
}

pub enum ModelError { EmptySourceId, EmptySequence }

pub enum InputIssue {
    InvalidLimits,
    TooManyBindings,
    DuplicateSourceId(SourceId),
    EmptyPlatforms(SourceId),
    EmptyScopes(SourceId),
    SourceIdTooLong(SourceId),
    SequenceTooLong(SourceId),
    NorenNotConfigurableOrDisableable(SourceId),
}

pub struct AnalyzeError {
    pub issues: Vec<InputIssue>,
}

impl SourceId {
    pub fn new(value: impl Into<String>) -> Result<Self, ModelError>;
}

impl KeyChord {
    pub fn normalized(
        key: Key,
        modifiers: impl IntoIterator<Item = Modifier>,
    ) -> Self;
}

impl KeySequence {
    pub fn new(chords: Vec<KeyChord>) -> Result<Self, ModelError>;
}

pub fn analyze(
    bindings: &[Binding],
    limits: &Limits,
) -> Result<Report, AnalyzeError>;
```

Value types implement `Clone`, `Debug`, equality, and ordering where meaningful.

# Invariants

- Source IDs and sequences are nonempty; IDs are unique per analysis. Platform and scope sets are nonempty.
- Modifier order and duplicates are removed. Left/right modifiers collapse. ASCII uppercase letters normalize to lowercase plus `Shift`; non-ASCII characters remain exact.
- Every Noren shortcut satisfies `configurable || disableable`, including disabled shortcuts.
- Disabled Noren bindings and `GuiOnly` triggers never participate in keyboard policy or collision checks.
- Scopes are exact capture contexts. A global keyboard shortcut active over a terminal must include `TerminalPane` and, if applicable, `ZellijPassThrough`; `GlobalGui` alone does not imply either.
- For a built-in Noren sequence active in either terminal context, any chord containing `Control` or `Alt`, or using `Function(_)`, is forbidden. `ExplicitUser` bypasses only this policy, not collision analysis.
- Pass-through permits at most one enabled keyboard exit leader and one enabled keyboard palette binding per platform. All other Noren keyboard roles are forbidden there; the exit leader must be configurable.
- Equal sequences collide. A proper prefix creates leader ambiguity. Disjoint platforms/scopes and unequal, non-prefix sequences are acceptable non-overlaps.
- External-versus-external pairs are not Noren conflicts and are omitted.

# Algorithm

1. Validate limits and all bindings. Return every input issue sorted by issue kind and source ID; empty input returns an empty report.
2. Inspect enabled Noren keyboard bindings for unsafe defaults and pass-through violations. Count exit and palette bindings separately per platform.
3. Compare each remaining pair containing at least one Noren binding. Intersect platforms and scopes; skip empty intersections.
4. Classify equality as `DuplicateNorenBinding` for two Noren sources, otherwise `ExactCollision`. Classify a proper prefix as `LeaderPrefixAmbiguity`, recording the shorter source.
5. If overlapping platform sets differ, use `PlatformSpecificShadowing` while retaining whether the underlying relation is exact or prefix.
6. Exact, duplicate, and policy violations are errors; prefix ambiguities are warnings. Platform shadowing inherits the underlying relation’s severity.
7. Sort and deduplicate source IDs and diagnostics by severity, reason, source IDs, platforms, then scopes.

For `n` bindings and maximum sequence length `l`, time is `O(n²l + n log n)` and memory is `O(n + d)` for `d` diagnostics.

# Test Matrix

`TP` means Terminal Pane, `PT` Pass-through, and `CP` Command Palette.

| Case | Input | Expected |
|---|---|---|
| Empty | No bindings | Empty report |
| Exact | Explicit Noren `C-b`, tmux `C-b`, TP/both OS | Exact collision, error |
| Prefix | Noren `C-x`, Vim `C-x C-s`, TP | Prefix ambiguity, warning |
| Shadowing | Noren `Super-p` both OS, Neovim same on macOS | Platform shadowing, macOS |
| Duplicate | Two Noren `F2` bindings in CP | Duplicate Noren, error |
| Reserved defaults | Default `C-a`, `A-x`, `C-A-z`, and `F5` in TP | Four unsafe-capture errors |
| Later reserved chord | Default `Super-k C-a` in TP | Unsafe-capture error |
| Explicit reserved | Explicit-user `A-x` in TP, no peer | No diagnostic |
| Forbidden PT | Ordinary Noren `Super-k` in PT | Forbidden capture |
| Valid PT | Configurable exit `Super-g` and one palette binding | No diagnostic |
| Bad exit | PT exit with `configurable=false` | Non-configurable-exit error |
| Multiple PT | Two enabled palette bindings on macOS | Multiple-binding error |
| Zellij collision | Exit `Super-g` and Zellij `Super-g` in PT | Exact collision |
| GUI-only | GUI action in PT matching a Zellij key | No conflict |
| Non-overlap | Same chord but disjoint platform or scope | No conflict |
| Disabled | Disabled Noren binding matching shell input | No conflict |
| Invalid model | Duplicate ID, empty scope, or excessive sequence | Sorted `AnalyzeError` |

# Property Tests

1. Permuting bindings, modifiers, platforms, or scopes must produce a structurally identical ordered report; normalization must be idempotent.
2. Compare bounded generated sequences against an exhaustive oracle: only equality or proper-prefix relationships with overlapping platform/scope sets can conflict, and pair classification is symmetric.

A deterministic standard-library generator is sufficient. Any optional property-testing crate requires API and version verification.

# Security and Reliability

Enforce limits before quadratic comparison. Use no `unsafe`, recursion, filesystem, network, process execution, or OS registration. On invalid configuration, install no Noren keyboard captures and retain a non-keyboard exit path. Analyze immutable snapshots, escape source IDs when displayed, and never log users’ typed input.

# Uncertainties

Backend semantics need confirmation for macOS Option composition, Linux AltGr appearing as Control+Alt, logical versus physical keys, and OS-reserved shortcuts. Terminal protocols may alias `C-i`/Tab or `C-m`/Enter, encode Alt as an Escape prefix, or use extended keyboard protocols. External defaults depend on caller-supplied configuration data.

# Deferred Work

Third-party configuration parsers, keyboard-layout translation, terminal-byte equivalence, external-versus-external advisory reports, leader timeout behavior, runtime dispatch, OS registration, persistence, migration, UI workflows, and the full implementation.
