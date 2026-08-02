# Summary

A std-only Rust library (`noren-keys`) that normalizes key chords, models binding ownership and activation scopes, and statically detects conflicts between Noren GUI bindings and preserved terminal input (Zellij, tmux, Vim, Neovim, shell). Analysis is deterministic, policy-driven, and emits structured diagnostics. Default policy treats focused terminal panes as pass-through for Control/Alt/Control+Alt/function keys unless the user explicitly binds them. Pass-through mode admits only an exit leader and an optional palette binding. Design only; no implementation provided.

# API

```rust
pub enum Platform { MacOs, Linux }

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Modifier { Control, Alt, Shift, Super } // Super = Cmd on macOS

pub struct ModifierSet { bits: u8 } // sorted, deduped; std-only bitset

pub enum Key {
    Char(char),        // case-folded to lowercase
    Function(u8),      // F1..=F24
    Enter, Esc, Tab, Backspace,
    Up, Down, Left, Right, Home, End, PageUp, PageDown,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyChord { pub mods: ModifierSet, pub key: Key }

pub enum ChordSpec {
    Single(KeyChord),
    Leader(Vec<KeyChord>), // len >= 2, leader prefix semantics
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Owner { Noren, Zellij, Tmux, Vim, Neovim, Shell }

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope { GlobalGui, TerminalPane, CommandPalette, ZellijPassThrough }

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BindingId(pub u64); // stable, caller-assigned

pub struct Binding {
    pub id: BindingId,
    pub owner: Owner,
    pub scope: Scope,
    pub spec: ChordSpec,
    pub platforms: PlatformSet,   // non-empty subset
    pub configurable: bool,       // invariant: true when owner == Noren
    pub disabled: bool,
    pub user_defined: bool,       // explicit user override of default policy
}

pub struct TerminalPolicy {
    pub exit_leader: ChordSpec,               // required in ZellijPassThrough
    pub palette_binding: Option<KeyChord>,    // optional in ZellijPassThrough
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity { Error, Warning, Info }

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    ExactCollision,      // same chord, same scope/platform, different owners
    LeaderAmbiguity,     // single chord equals prefix of a leader sequence
    PlatformShadowing,   // overlap on one platform only, or global vs terminal
    DuplicateNoren,      // two enabled Noren bindings, same chord+scope
    AcceptableNonOverlap,
}

pub struct Diagnostic {
    pub sources: Vec<BindingId>,   // sorted, len >= 2 (or 1 for duplicates note)
    pub severity: Severity,
    pub reason: Reason,
    pub platforms: PlatformSet,
    pub scopes: Vec<Scope>,        // sorted
    pub remediation: Remediation,
}

pub enum Remediation {
    Rebind { suggested: KeyChord },
    DisableOneOf,
    MoveScope { to: Scope },
    MarkUserDefined,
    None,
}

pub struct Report { pub diagnostics: Vec<Diagnostic> } // sorted, deterministic

pub struct ConflictEngine { /* private */ }

impl ConflictEngine {
    pub fn new(policy: TerminalPolicy) -> Self;
    pub fn add_binding(&mut self, b: Binding) -> Result<(), EngineError>;
    pub fn remove_binding(&mut self, id: BindingId) -> bool;
    pub fn analyze(&self) -> Report;
}

pub enum EngineError {
    DuplicateId(BindingId),
    NorenNotConfigurable,
    EmptyPlatforms,
    InvalidLeader,
}
```

Optional crate: `bitflags` for `ModifierSet`/`PlatformSet` — **requires verification**; a hand-rolled `u8` bitset keeps this std-only.

# Invariants

- `owner == Noren` ⟹ `configurable == true`; violation is `EngineError::NorenNotConfigurable`.
- `platforms` is non-empty; `Leader` length ≥ 2; chords are normalized (mods sorted/deduped, `Char` lowercased).
- `BindingId`s are unique within the engine.
- Disabled bindings are inactive: never conflict, never shadow.
- In `TerminalPane`, a Noren binding whose chord carries `Control`, `Alt`, `Control+Alt`, or `Function(_)` is inactive **unless** `user_defined == true`.
- In `ZellijPassThrough`, only `policy.exit_leader` and `policy.palette_binding` are active; all other Noren bindings (GUI-only actions) are excluded, so they cannot be keyboard conflicts.
- `analyze()` output is a pure function of inputs: sorted by `(severity, reason, sources, platforms, scopes)`.

# Algorithm

1. **Normalize** each chord on insert (O(m log m) per chord for modifier sort; m ≤ 4).
2. **Activate** bindings per `(Platform, Scope)` using policy filters (terminal default-deny, pass-through allowlist). O(n).
3. **Index** active bindings into `BTreeMap<(Scope, Platform, KeyChord), Vec<BindingId>>` for exact collisions; a parallel map keyed on leader prefixes (`Vec<KeyChord>` prefixes) for ambiguity. O(n log n).
4. **Exact collision**: buckets with ≥ 2 distinct owners → `Error`. Same owner Noren twice → `DuplicateNoren` (`Error`).
5. **Leader ambiguity**: any `Single(c)` active in the same scope/platform where `c` equals the first chord of a `Leader` sequence, or one leader is a strict prefix of another → `Warning`.
6. **Platform shadowing**: chord overlaps between owners on exactly one platform, or a `GlobalGui` Noren chord equals a `TerminalPane` preserved-owner chord → `Warning`, with `platforms` narrowed to the affected set.
7. **Acceptable non-overlap**: pairs sharing a key but disjoint modifiers/scopes, or disjoint platforms → `Info` (emitted only when explicitly requested via a flag; default off to keep reports small).
8. **Determinism**: sort diagnostics; suggested rebinds come from a fixed candidate list scanned in order.

Complexity: O(n log n + p) time, O(n) space, where p is total pair count within buckets (bounded by n in practice). No I/O, no randomness.

# Test Matrix

| # | Setup (scope/owner/chord) | Platform | Expectation |
|---|---|---|---|
| 1 | Noren `GlobalGui` Ctrl+S vs Vim `TerminalPane` Ctrl+S | both | `AcceptableNonOverlap` only (different scopes); no Error |
| 2 | Noren `TerminalPane` Ctrl+S (default) vs Vim `TerminalPane` Ctrl+S | both | No diagnostic: Noren binding inactive by default-deny |
| 3 | Same as #2 but `user_defined = true` | both | `ExactCollision`, `Error`, remediation `DisableOneOf` |
| 4 | Noren `TerminalPane` Alt+J (default) vs Neovim `TerminalPane` Alt+J | both | No diagnostic (Alt captured by default-deny) |
| 5 | Noren `TerminalPane` F5 (default) vs tmux `TerminalPane` F5 | both | No diagnostic (function key default-deny) |
| 6 | Noren leader `[Ctrl+G, S]` `GlobalGui` vs Noren `Single(Ctrl+G)` `GlobalGui` | both | `LeaderAmbiguity`, `Warning` |
| 7 | Noren `GlobalGui` Cmd+P (`Super+P`) `platforms={MacOs}` vs Shell `Ctrl+P` `TerminalPane` Linux-only | macOS/Linux | No collision; platform sets disjoint → `Info` if requested |
| 8 | Noren `GlobalGui` Ctrl+P on `{MacOs,Linux}` vs Shell `TerminalPane` Ctrl+P on `{Linux}` | Linux only | `PlatformShadowing`, `Warning`, `platforms={Linux}` |
| 9 | Two Noren `GlobalGui` Ctrl+K, both enabled | both | `DuplicateNoren`, `Error` |
| 10 | `ZellijPassThrough`: Noren GUI action Ctrl+B active elsewhere vs Zellij prefix Ctrl+B | both | No diagnostic (GUI action excluded in pass-through) |
| 11 | `ZellijPassThrough`: exit leader `Ctrl+G` vs Zellij binding `Ctrl+G` | both | `ExactCollision`, `Error` on the exit leader (allowed/expected surface) |
| 12 | Disabled Noren Ctrl+S vs Vim Ctrl+S, same scope | both | No diagnostic |

# Property Tests

1. **Insertion-order independence**: for any permutation of a binding set, `analyze()` yields byte-identical `Report` (after normalization). Use randomized permutations; assert equality.
2. **Default-deny soundness**: for any generated Noren `TerminalPane` binding with `user_defined == false` whose chord contains `Control`, `Alt`, or `Function`, no `Error` diagnostic lists it against a preserved owner. Corollary: disabling one side of any reported pair removes that diagnostic.

# Security and Reliability

- No `unsafe`, no I/O, no environment reads; analysis is pure and reproducible.
- Bound input sizes: leader length ≤ 8, modifier count ≤ 4, `F(u8)` ≤ 24; reject otherwise to avoid unbounded work.
- Keymap string parsing (if added) must be allocation-bounded and non-regex to avoid ReDoS; prefer a small hand-written parser.
- Deterministic ordering prevents flaky CI and diffable config audits.
- Diagnostics never echo raw user text beyond normalized chord representations, limiting injection surface in UIs that render them.

# Uncertainties

- Exact chord semantics of Zellij/tmux/Vim defaults vary by version and user config; the engine treats them as data, not hardcoded truth.
- Alt delivery differs across terminals (ESC-prefix vs 8-bit vs kitty protocol); normalization here assumes a decoded `Alt` modifier is available upstream.
- `Super`/Cmd rarely reaches terminal panes on macOS; modeling it is conservative.
- `bitflags` and any serialization crate are optional and **require verification** before adoption.
- Whether `AcceptableNonOverlap` should ever be surfaced by default is a product decision.

# Deferred Work

- Windows platform and full kitty keyboard protocol disambiguation.
- Live reload/watching of config files and incremental re-analysis.
- Chord serialization/parsing and user-facing keymap editor integration.
- Fuzzy "near-miss" suggestions and localized remediation text.
- Runtime capture layer (this library is static analysis only) and telemetry on suppressed conflicts.
