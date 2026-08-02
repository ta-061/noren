# Agent calibration task

Captured: 2026-08-03 (Asia/Tokyo)

Every candidate received the following text verbatim in a separate empty temporary
directory. Repository access, file access, tools, network access, and edits were
disabled by the prompt and command-level controls where supported.

## Prompt

You are participating in a capability calibration for the Noren terminal project. Do not access files, run tools, use the network, or edit anything. Return Markdown only, at most 1,200 words.

Task: design a small Rust library API and test plan for detecting keybinding conflicts in a terminal that must preserve Zellij, tmux, Vim, Neovim, and shell input.

Requirements:
1. Represent a normalized key chord, platform (macOS/Linux), binding owner/source, activation scope, and whether a Noren binding is configurable or disabled.
2. Model at least Global GUI, Terminal Pane, Command Palette, and Zellij Pass-through scopes.
3. In a focused terminal pane, default policy must not capture Control, Alt, Control+Alt, or function-key input unless a user explicitly binds it.
4. In Pass-through mode, keyboard capture is limited to a configurable exit leader and an optional command-palette binding; GUI-only actions are not keyboard conflicts.
5. Detect and distinguish exact collisions, leader/prefix ambiguities, platform-specific shadowing, duplicate Noren bindings, and acceptable non-overlaps.
6. Return deterministic structured diagnostics containing source IDs, severity, reason, affected platforms/scopes, and remediation.
7. All Noren shortcuts must be configurable or disableable.
8. Avoid claiming unverified third-party crate APIs. Prefer a standard-library design; label any optional crate as requiring verification.
9. Give public Rust type/function signatures, invariants, algorithm and complexity, at least eight table-driven test cases, two property-test ideas, security/reliability notes, uncertainties, and deliberately deferred work.
10. This is a design task only. Do not provide a full implementation.

Use exactly these top-level headings:
# Summary
# API
# Invariants
# Algorithm
# Test Matrix
# Property Tests
# Security and Reliability
# Uncertainties
# Deferred Work
