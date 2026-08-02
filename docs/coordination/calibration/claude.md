# Summary

This design specifies `noren-keymap`, a small, dependency-light Rust library that detects conflicts between Noren's own keybindings and the keyboard input expected by terminal-resident applications (Zellij, tmux, Vim, Neovim, and the shell itself). The core stance is that a terminal emulator is a *guest host*: in a focused terminal pane, the terminal application owns the keyboard, and Noren must justify every key it steals.

The library is a pure function over declarative data. It takes a set of `Binding` records (Noren's own, plus a bundled corpus of known third-party defaults) and returns a deterministic, sorted list of `Diagnostic` values. It performs no I/O, spawns no threads, and holds no global state, which makes it trivially testable and embeddable in both a config-load path and a CLI lint subcommand.

Five conflict classes are distinguished: exact collisions, leader/prefix ambiguity (Noren capturing a key that is the prefix of a multi-key third-party sequence, e.g. tmux `C-b`), platform-specific shadowing (a binding that conflicts only on macOS or only on Linux), duplicate Noren bindings within one scope, and acceptable non-overlaps (reported at `Info` for auditability, never as failures). Severity is a function of conflict class and scope, not of the individual binding, so policy stays in one place.

Design is standard-library only: `std::collections::BTreeMap`/`BTreeSet` for ordered, deterministic output; no `serde`, no `bitflags`, no terminal crates in the core. Serialization of the config format is deliberately left to the embedding application behind a feature flag, so the core carries no unverified third-party API claims.

# API

```rust
// ---- Key representation ----

/// A physical key, normalized. Non-exhaustive: the corpus may grow.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum KeyCode {
    Char(char),          // stored lowercase; case carried by Shift
    Function(u8),        // F1..F24, validated 1..=24
    Enter, Tab, Backspace, Escape, Space,
    Left, Right, Up, Down,
    Home, End, PageUp, PageDown, Insert, Delete,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,    // Option on macOS
    pub shift: bool,
    pub meta: bool,   // Command on macOS, Super on Linux
}

/// A single normalized chord. Constructed only via `try_new`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Chord { /* private fields: code, mods */ }

impl Chord {
    pub fn try_new(code: KeyCode, mods: Modifiers) -> Result<Self, ChordError>;
    pub fn code(&self) -> KeyCode;
    pub fn mods(&self) -> Modifiers;
    /// Canonical text form, e.g. "C-M-x", "F5", "S-Tab". Round-trips with `parse`.
    pub fn to_canonical(&self) -> String;
}

impl std::str::FromStr for Chord { type Err = ChordError; /* ... */ }

/// A key sequence: one or more chords (tmux `C-b` then `c` is length 2).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ChordSeq { /* private: Vec<Chord>, non-empty */ }

impl ChordSeq {
    pub fn try_new(chords: Vec<Chord>) -> Result<Self, ChordError>;
    pub fn chords(&self) -> &[Chord];
    pub fn is_prefix_of(&self, other: &ChordSeq) -> bool;
}

// ---- Context ----

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Platform { MacOs, Linux }

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Scope {
    GlobalGui,        // window/tab management, app-level
    TerminalPane,     // a focused terminal with a live PTY
    CommandPalette,   // modal overlay; owns the keyboard
    ZellijPassthrough // near-total surrender of the keyboard
}

#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Owner {
    Noren,
    ThirdParty { app: String }, // "zellij", "tmux", "vim", "neovim", "readline"
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Mutability { Configurable, Disableable, Fixed }

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Binding {
    pub id: SourceId,             // stable, unique; newtype over String
    pub owner: Owner,
    pub seq: ChordSeq,
    pub scopes: BTreeSet<Scope>,
    pub platforms: BTreeSet<Platform>,
    pub mutability: Mutability,
    pub enabled: bool,
    pub user_bound: bool,         // set by explicit user config, not defaults
}

// ---- Diagnostics ----

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity { Error, Warning, Info }

#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum ConflictKind {
    ExactCollision,
    PrefixAmbiguity { shadowed_prefix: ChordSeq },
    PlatformShadowing,
    DuplicateNorenBinding,
    PolicyViolation { rule: PolicyRule },
    AcceptableNonOverlap,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Diagnostic {
    pub kind: ConflictKind,
    pub severity: Severity,
    pub sources: Vec<SourceId>,   // sorted, deduped
    pub scopes: BTreeSet<Scope>,
    pub platforms: BTreeSet<Platform>,
    pub reason: String,
    pub remediation: Remediation,
}

#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Remediation {
    RebindNoren { binding: SourceId, suggested: Vec<ChordSeq> },
    DisableNoren { binding: SourceId },
    RestrictScope { binding: SourceId, drop: BTreeSet<Scope> },
    MakeGuiOnly { binding: SourceId },
    NoActionRequired,
}

// ---- Policy and entry point ----

#[derive(Clone, Debug)]
pub struct Policy {
    /// Modifier patterns Noren must not capture in TerminalPane by default.
    pub reserved_pane_modifiers: Vec<ModifierPattern>,
    pub reserve_function_keys_in_pane: bool,
    pub passthrough_exit_leader: ChordSeq,
    pub passthrough_palette: Option<ChordSeq>,
    pub report_non_overlaps: bool,
}

impl Default for Policy { /* Requirement 3 defaults */ }

#[derive(Clone, Debug, Default)]
pub struct Keymap { /* private: Vec<Binding> */ }

impl Keymap {
    pub fn from_bindings(b: Vec<Binding>) -> Result<Self, KeymapError>;
    pub fn bindings(&self) -> &[Binding];
}

/// The single entry point. Pure, deterministic, allocation-only.
pub fn analyze(map: &Keymap, policy: &Policy) -> Report;

pub struct Report { /* private */ }

impl Report {
    pub fn diagnostics(&self) -> &[Diagnostic];   // sorted, stable
    pub fn max_severity(&self) -> Option<Severity>;
    pub fn has_errors(&self) -> bool;
}

/// Curated defaults for known terminal applications. Data, not detection.
pub mod corpus {
    pub fn zellij_defaults() -> Vec<Binding>;
    pub fn tmux_defaults() -> Vec<Binding>;
    pub fn vim_defaults() -> Vec<Binding>;      // covers Neovim overlap
    pub fn neovim_extras() -> Vec<Binding>;
    pub fn readline_defaults() -> Vec<Binding>; // shell line editing
}
```

# Invariants

1. **Chord normalization is total and idempotent.** `Chord::try_new` lowercases `Char`, rejects `Char` values that are control codepoints or whitespace (use `KeyCode::Space`/`Tab`/`Enter`), and rejects `Function(n)` for `n == 0 || n > 24`. `normalize(normalize(c)) == normalize(c)`.
2. **Shift is not synthesized.** `Char('A')` is invalid; the caller must express it as `Char('a')` with `shift: true`. This prevents the same chord having two representations.
3. **`ChordSeq` is non-empty**, enforced at construction; `chords()` never returns an empty slice.
4. **`SourceId` is unique within a `Keymap`.** `from_bindings` returns `KeymapError::DuplicateSourceId` otherwise — this is a caller error, distinct from `DuplicateNorenBinding`, which is a *keymap* conflict between two distinct IDs on the same chord.
5. **Every Noren binding satisfies `mutability != Fixed`.** Enforced in `from_bindings`; violating it is a construction error, which is how Requirement 7 becomes unrepresentable rather than merely tested.
6. **Analysis is pure.** `analyze` performs no I/O, reads no environment, and has no interior mutability. Given equal inputs it returns an equal `Report`.
7. **Output is totally ordered.** Diagnostics sort by `(severity, kind, sources, scopes, platforms)`, all of which derive `Ord`. Two runs on the same input produce byte-identical rendered output.
8. **Disabled bindings are inert.** `enabled: false` bindings participate in no conflict except `DuplicateNorenBinding` detection against other disabled bindings, which is suppressed. They are simply filtered at entry.
9. **Scope semantics.** `TerminalPane` conflicts are evaluated against the third-party corpus; `GlobalGui` bindings only conflict with `TerminalPane` if they also declare `TerminalPane` in `scopes`. A `GlobalGui`-only binding is never a keyboard conflict with Vim (Requirement 4's GUI-only clause).
10. **Pass-through minimality.** In `ZellijPassthrough`, the only permitted captured sequences are `policy.passthrough_exit_leader` and, if `Some`, `policy.passthrough_palette`. Any other Noren binding declaring that scope yields `PolicyViolation` at `Error`.

# Algorithm

`analyze` runs five passes over the enabled bindings. Let *n* be the binding count, *s* the number of scopes (4), *p* platforms (2), and *k* the maximum sequence length (small, ≤ 4).

**Pass 0 — Partition.** Filter disabled bindings. Bucket the rest into a `BTreeMap<(Scope, Platform), Vec<&Binding>>` by expanding each binding's scope × platform cross product. Cost O(n·s·p).

**Pass 1 — Exact collisions.** Within each bucket, build a `BTreeMap<ChordSeq, Vec<&Binding>>`. Any key with ≥ 2 entries is a collision. If all entries are `Owner::Noren`, emit `DuplicateNorenBinding`; if it mixes Noren and third-party, emit `ExactCollision`. Third-party-only overlaps (Vim and tmux both wanting `C-b`) are *not* Noren's problem and are skipped. Cost O(n·s·p·log n·k).

**Pass 2 — Prefix ambiguity.** For each bucket, insert all third-party sequences into a trie keyed by `Chord`. For each Noren sequence, walk the trie: if the walk terminates at an interior node with children, the Noren binding shadows a longer third-party sequence — emit `PrefixAmbiguity { shadowed_prefix }`. The tmux case (Noren binds `C-b`, tmux uses `C-b` + `c`) lands here at `Error` in `TerminalPane`. The reverse direction (third party is a prefix of a Noren binding) is a `Warning`: the terminal app will consume the first chord and Noren's binding is unreachable. Cost O(n·k) after O(n·k) trie construction.

**Pass 3 — Platform shadowing.** For each Noren binding, compute the set of platforms on which it collides (from passes 1–2 results, keyed by binding ID). If that set is a strict non-empty subset of the binding's declared platforms, upgrade the diagnostics to `PlatformShadowing` and record the exact affected platforms. This is where macOS `Cmd` vs Linux `Ctrl` divergence surfaces: a `Meta`-based Noren binding is usually clean on macOS and a `Ctrl` equivalent is usually dirty on Linux. Cost O(n·p).

**Pass 4 — Policy.** Two rules. (a) *Reserved pane input*: for each Noren binding whose scopes include `TerminalPane`, if its first chord matches any `reserved_pane_modifiers` pattern (Ctrl, Alt, Ctrl+Alt) or is a function key with `reserve_function_keys_in_pane`, and `user_bound == false`, emit `PolicyViolation` at `Error`. If `user_bound == true`, downgrade to `Info` — the user explicitly asked for it, which is Requirement 3's escape hatch. (b) *Pass-through minimality*: apply invariant 10. Cost O(n).

**Pass 5 — Non-overlaps.** If `policy.report_non_overlaps`, every Noren binding not appearing in any prior diagnostic gets an `AcceptableNonOverlap` at `Info` with `Remediation::NoActionRequired`. Cost O(n).

**Finalize.** Concatenate, dedupe by structural equality, and `sort_unstable` by the invariant-7 key (structural sort makes stability irrelevant). Total: **O(n·s·p·k·log n)** time, **O(n·s·p·k)** space. With realistic *n* in the low hundreds and *s·p* = 8, this is microseconds — fast enough to run on every config reload without a cache.

Remediation suggestions come from a scope-aware generator: for a `TerminalPane` conflict it prefers `MakeGuiOnly` or `RestrictScope`; for a `GlobalGui` conflict it prefers `RebindNoren` with candidates drawn from `Meta`-prefixed chords (macOS-friendly) filtered against the full corpus, capped at three suggestions for output stability.

# Test Matrix

| # | Scenario | Inputs | Scope / Platform | Expected |
|---|---|---|---|---|
| 1 | Exact collision with tmux prefix | Noren `C-b` (pane), tmux `C-b`+`c` | TerminalPane / both | `PrefixAmbiguity{shadowed_prefix: C-b}`, `Error`, remediation `MakeGuiOnly` or `RestrictScope` |
| 2 | Vim normal-mode collision | Noren `C-w` (pane), vim `C-w`+`v` | TerminalPane / both | `PrefixAmbiguity`, `Error`, both source IDs present |
| 3 | Default policy blocks Ctrl in pane | Noren `C-t` (pane), `user_bound: false`, no third-party match | TerminalPane / both | `PolicyViolation{ReservedPaneModifier}`, `Error` — fires even with zero corpus overlap |
| 4 | Explicit user binding permitted | Same as #3 but `user_bound: true` | TerminalPane / both | `Info`, not `Error`; `has_errors() == false` |
| 5 | Function key reserved by default | Noren `F5` (pane), `user_bound: false` | TerminalPane / both | `PolicyViolation`, `Error`; with `reserve_function_keys_in_pane: false` → no diagnostic |
| 6 | Platform shadowing | Noren `Meta+t` (pane+gui), Linux corpus binds `Super+t` | both | `PlatformShadowing`, `platforms == {Linux}` only; macOS clean |
| 7 | GUI-only action is not a conflict | Noren `Meta+n` scoped `{GlobalGui}` only; Vim binds nothing relevant | GlobalGui / both | `AcceptableNonOverlap` at `Info` when `report_non_overlaps`, else empty |
| 8 | Duplicate Noren bindings | Noren `noren.split` and `noren.new_tab` both `Meta+d`, same scope | GlobalGui / macOS | `DuplicateNorenBinding`, `Error`, `sources` contains both IDs sorted |
| 9 | Pass-through minimality | Noren binds exit leader `C-g C-g` plus `C-p` palette plus `Meta+w` close, all in `ZellijPassthrough` | Passthrough / both | Exactly one `PolicyViolation` for `Meta+w`; leader and palette clean |
| 10 | Command palette owns keyboard | Noren `Escape` in `CommandPalette`; Vim binds `Escape` in pane | both | No diagnostic — scopes are disjoint, modal overlay is exclusive |
| 11 | Disabled binding is inert | Noren `C-b` (pane) with `enabled: false`, tmux corpus loaded | TerminalPane / both | Empty diagnostics |
| 12 | Fixed mutability rejected | `Binding{owner: Noren, mutability: Fixed}` | — | `Keymap::from_bindings` → `Err(KeymapError::FixedNorenBinding)` |

Each row is a struct literal in a `#[test]` driven by a `case!` helper that asserts on the full sorted `Vec<Diagnostic>`, not just a count — so an unexpected extra diagnostic fails the test.

# Property Tests

**P1 — Determinism and order independence.** For any generated `Vec<Binding>` (with unique IDs), `analyze(&Keymap::from_bindings(v.clone())?, &p)` equals `analyze(&Keymap::from_bindings(shuffle(v))?, &p)`. This pins invariants 6 and 7 simultaneously, and is the property most likely to catch a `HashMap` creeping into the implementation.

**P2 — Pane-capture soundness.** For any generated keymap under `Policy::default()`, if a Noren binding is enabled, scoped to `TerminalPane`, has `user_bound == false`, and its first chord carries `ctrl || alt` or is a function key, then the report contains at least one `Error`-severity diagnostic naming that binding's `SourceId`. This is Requirement 3 stated as an executable invariant rather than a set of examples, and it holds regardless of what the corpus contains.

Two further properties worth adding: canonical round-tripping (`Chord::from_str(&c.to_canonical()) == Ok(c)`) and prefix antisymmetry (`a.is_prefix_of(b) && b.is_prefix_of(a)` implies `a == b`).

Generators should be hand-rolled over a small fixed alphabet — a `SmallRng` with a recorded seed and a shrink-by-truncation loop is sufficient and avoids depending on a specific proptest/quickcheck API surface. If a property-testing crate is adopted, its API must be verified against the installed version before use.

# Security and Reliability

The library is pure and I/O-free, which eliminates most of the attack surface by construction. The residual concerns are about *input trust* and *failure modes*:

- **Untrusted config as input.** Binding definitions come from user config files, which may be adversarial or merely corrupt. Parsing lives outside the core, but `Keymap::from_bindings` must remain the only constructor and must enforce every invariant, including a cap on `ChordSeq` length (suggest 8) and on total binding count (suggest 4096). Without the length cap, a pathological sequence inflates trie depth and diagnostic strings.
- **No panics on any input.** All fallible paths return `Result`. No indexing, no `unwrap`, no `expect` in library code; a debug-assert-only guard is acceptable. Add a fuzz target over `Chord::from_str` and, if a config parser is later added, over that too.
- **Denial of service via combinatorics.** Complexity is near-linear, but remediation suggestion generation searches candidate chords. Cap suggestions at three and cap the candidate search at a fixed budget so a large keymap cannot make it quadratic.
- **Fail-safe defaults.** If analysis returns errors at config load, Noren should keep the *previous* keymap and surface diagnostics, not fall back to an empty or partially-applied keymap. A partially-applied keymap is the state most likely to strand a user in pass-through mode with no working exit leader.
- **Exit-leader liveness.** The most serious reliability failure is an unreachable exit from pass-through. A dedicated check should verify `passthrough_exit_leader` is non-empty and conflict-free before pass-through is enterable at all; treat this as a hard precondition in the embedding application, not merely a diagnostic.
- **No secrets, no telemetry.** Diagnostics contain source IDs and canonical chord text only. `SourceId` values should be developer-chosen identifiers, not user-supplied strings echoed verbatim, if diagnostics are ever logged.

# Uncertainties

- **Corpus fidelity.** The bundled defaults for Zellij, tmux, Vim, Neovim, and readline are a curated snapshot, not authoritative. They drift with upstream releases and are routinely overridden by user configs. The library should treat the corpus as *advisory data*, version it explicitly, and allow the embedding application to replace it wholesale. I have not verified the current default keymaps of any of these tools against their present documentation.
- **Modality is unmodeled.** Vim's `C-w` means different things in normal, insert, and terminal mode; tmux's prefix is stateful. The current design flattens this into a single `TerminalPane` scope, which over-reports. Modal state is not observable from the terminal emulator anyway, so conservative over-reporting is arguably correct — but it is a real limitation, not a neutral choice.
- **Kitty keyboard protocol / CSI-u.** When the enhanced protocol is negotiated, the set of distinguishable chords changes materially (e.g. `C-i` becomes separable from `Tab`). Whether `Chord` should carry a protocol-mode dimension is genuinely open, and it interacts with normalization invariants 1 and 2.
- **macOS Option-as-Meta.** Whether Option produces `Alt` or composes an accented character is a per-profile terminal setting. `Modifiers::alt` on macOS is therefore ambiguous, and the platform-shadowing pass may be under- or over-sensitive depending on that setting.
- **Third-party-only overlaps.** Pass 1 deliberately ignores tmux-vs-Vim conflicts. That is probably right — Noren cannot fix them — but users may expect them surfaced informationally.
- **Complexity figures** are analytical, not measured. No benchmarks exist yet.

# Deferred Work

Explicitly out of scope for this iteration, in rough priority order:

1. **Config serialization.** No `serde` derives in the core. A `serde` feature flag adding `Serialize`/`Deserialize` to the public data types is the obvious next step, but adopting it requires verifying the crate's current API and committing to its semver surface.
2. **Live keymap introspection.** Reading actual `~/.tmux.conf`, `init.lua`, or Zellij KDL config to replace the static corpus. High value, high fragility; needs a parser per tool.
3. **Modal scope modeling** (see Uncertainties) — likely a `Mode` dimension alongside `Scope`.
4. **Automatic remediation application.** Currently the library only *suggests*; rewriting the user's config is a separate, higher-risk capability requiring confirmation flows.
5. **Interactive conflict resolution UI** and a `noren keys lint` CLI subcommand rendering `Report` as human-readable text and JSON.
6. **Kitty/CSI-u protocol dimension** on `Chord`.
7. **Benchmarks and fuzz targets** — the complexity claims above should be validated before the corpus grows.
8. **Localization** of `reason` and `Remediation` strings; currently English-only, which makes them unsuitable as a stable machine interface. Machine consumers should key off `ConflictKind`, not `reason`.

---

Two notes on the framing. First, this is design only, as requested — no implementation, and the signatures above are a proposal to be compiled against, not code I have verified builds. Second, I have not read any Noren source; the scope names, the `Owner` variants, and the pass-through model are inferred from your requirements, so if Noren's existing architecture names these differently, treat the structure as the contribution and the vocabulary as negotiable.
