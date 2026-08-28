use super::*;
use noren_app::palette::CommandId;
use noren_app::passthrough::{self, collisions};
use noren_app::session_persistence::{MAX_SESSION_STATE_BYTES, load};
use noren_app::sidebar::EntryKind;

include!("../input_translation/tests.rs");
include!("../frame_geometry/tests.rs");

#[test]
fn terminal_modes_drive_cursor_and_keypad_encoding() {
    let mut app = NorenApp::default();
    assert_eq!(app.current_input_mode(), InputMode::normal());

    let mut terminal = TerminalState::new(2, 4).expect("valid terminal");
    terminal.feed_bytes(b"\x1b[?1h\x1b=");
    app.terminal = Some(terminal);
    let mode = app.current_input_mode();

    let arrow = KeyInput::new(Key::Arrow(Arrow::Up), KeyPhase::Pressed, Modifiers::empty());
    assert_eq!(
        KeyEncoder::encode_with(arrow, mode).as_deref(),
        Ok(b"\x1bOA".as_slice())
    );
    assert_eq!(
        KeyEncoder::encode_keypad_with(KeypadInput::new(KeypadKey::One, KeyPhase::Pressed), mode)
            .as_deref(),
        Ok(b"\x1bOq".as_slice())
    );
}

#[test]
fn display_row_count_counts_through_the_last_non_blank_row() {
    let mut terminal = TerminalState::new(4, 8).expect("valid terminal");
    terminal.feed_bytes(b"ab\r\ncd");
    assert_eq!(terminal.screen().display_row_count(), 2);

    terminal.feed_bytes(b"\r\n\r\nef");
    assert_eq!(terminal.screen().display_row_count(), 4);
    assert_eq!(
        terminal.screen().display_row_count(),
        terminal.snapshot().display_cells().len(),
        "live hit testing and snapshot rendering must select the same rows"
    );
}

#[test]
fn shared_row_layout_maps_selection_and_mouse_paths() {
    let metrics = GridGeometry::poc().cell_metrics();
    let frame_size = PhysicalSize::new(
        (renderer::SIDEBAR_COLS as u32 + 8) * metrics.width(),
        30 * metrics.height(),
    );
    let x = sidebar_pixel_width(metrics.width());
    let position_at = |row: u32| PhysicalPosition::new(x, f64::from(row * metrics.height()) + 1.0);
    let mapped_line = |app: &NorenApp, row| {
        app.grid_point_in_frame(position_at(row), frame_size)
            .map(GridPoint::line)
    };
    let mouse_cell = |app: &NorenApp, row| app.mouse_cell_in_frame(position_at(row), frame_size);

    // Underfilled: content and status remain at rows 0 and 1, with blank
    // space below. This is the 30-row form of the reviewed mismatch.
    let mut underfilled = TerminalState::new(30, 8).expect("valid terminal");
    underfilled.feed_bytes(b"A");
    let underfilled = NorenApp {
        terminal: Some(underfilled),
        show_status: true,
        ..Default::default()
    };
    assert_eq!(mapped_line(&underfilled, 0), Some(0));
    assert_eq!(mouse_cell(&underfilled, 0), Some((0, 0)));
    assert_eq!(mapped_line(&underfilled, 1), None, "row 1 is status");
    assert_eq!(
        mouse_cell(&underfilled, 1),
        None,
        "status is not reportable"
    );
    assert_eq!(mapped_line(&underfilled, 29), None, "underfill stays blank");

    // Status-only: row zero is chrome and no pixel row addresses terminal
    // content.
    let status_only = NorenApp {
        terminal: Some(TerminalState::new(30, 8).expect("valid terminal")),
        show_status: true,
        ..Default::default()
    };
    assert_eq!(mapped_line(&status_only, 0), None);
    assert_eq!(mouse_cell(&status_only, 0), None);
    assert_eq!(mapped_line(&status_only, 29), None);

    // A production-sized terminal reserves the status row before the PTY
    // and state are sized, so all 29 logical terminal rows remain visible.
    let mut reserved = TerminalState::new(29, 8).expect("valid terminal");
    reserved.feed_bytes(b"\x1b[29;1HZ");
    let reserved = NorenApp {
        terminal: Some(reserved),
        show_status: true,
        ..Default::default()
    };
    assert_eq!(NorenApp::content_terminal_rows(30), 29);
    assert_eq!(mapped_line(&reserved, 0), Some(0));
    assert_eq!(mouse_cell(&reserved, 0), Some((0, 0)));
    assert_eq!(mapped_line(&reserved, 28), Some(28));
    assert_eq!(mapped_line(&reserved, 29), None, "last row is status");

    let one_row = NorenApp {
        terminal: Some(TerminalState::new(1, 8).expect("valid terminal")),
        show_status: true,
        ..Default::default()
    };
    assert_eq!(NorenApp::content_terminal_rows(1), 1);
    assert_eq!(one_row.rendered_status_row(1), None);
}

#[test]
fn horizontal_frame_bounds_are_shared_by_selection_and_mouse_paths() {
    let mut terminal = TerminalState::new(1, 8).expect("valid terminal");
    terminal.feed_bytes(b"A");
    let app = NorenApp {
        terminal: Some(terminal),
        show_status: false,
        ..Default::default()
    };
    let metrics = app.geometry.cell_metrics();
    // Deliberately leave two extra terminal-side cells in the frame. A
    // position there is still in-frame and retains the historical clamp to
    // the terminal's last logical column.
    let frame_size = PhysicalSize::new(
        (renderer::SIDEBAR_COLS as u32 + 10) * metrics.width(),
        metrics.height(),
    );
    let terminal_x = sidebar_pixel_width(metrics.width());
    let mapped = |position, size| {
        (
            app.grid_point_in_frame(position, size),
            app.mouse_cell_in_frame(position, size),
        )
    };

    assert_eq!(
        mapped(PhysicalPosition::new(terminal_x, 1.0), frame_size),
        (Some(GridPoint::new(0, 0)), Some((0, 0))),
        "a valid in-frame position maps through both seams"
    );
    assert_eq!(
        mapped(
            PhysicalPosition::new(f64::from(frame_size.width) - 1.0, 1.0),
            frame_size,
        ),
        (Some(GridPoint::new(0, 7)), Some((7, 0))),
        "in-frame space past the logical grid still clamps to its last column"
    );
    assert_eq!(
        mapped(
            PhysicalPosition::new(f64::from(frame_size.width), 1.0),
            frame_size,
        ),
        (None, None),
        "the right frame edge is exclusive"
    );
    assert_eq!(
        mapped(
            PhysicalPosition::new(f64::from(frame_size.width) + 1.0, 1.0),
            frame_size,
        ),
        (None, None),
        "a position beyond the right frame edge is rejected"
    );
    assert_eq!(
        mapped(
            PhysicalPosition::new(0.0, 1.0),
            PhysicalSize::new(0, frame_size.height),
        ),
        (None, None),
        "a zero-width frame has no addressable position"
    );
    assert_eq!(
        mapped(
            PhysicalPosition::new(terminal_x, 0.0),
            PhysicalSize::new(frame_size.width, 0),
        ),
        (None, None),
        "a zero-height frame has no addressable position"
    );

    for invalid in [
        PhysicalPosition::new(f64::NAN, 1.0),
        PhysicalPosition::new(terminal_x, f64::INFINITY),
        PhysicalPosition::new(-1.0, 1.0),
        PhysicalPosition::new(terminal_x, -1.0),
    ] {
        assert_eq!(mapped(invalid, frame_size), (None, None));
    }
}

#[test]
fn background_only_row_is_content_for_status_and_hit_testing() {
    let mut terminal = TerminalState::new(4, 8).expect("valid terminal");
    terminal.feed_bytes(b"\x1b[48;2;73;18;146m ");
    assert_eq!(terminal.screen().display_row_count(), 1);
    assert_eq!(terminal.snapshot().display_cells().len(), 1);

    let app = NorenApp {
        terminal: Some(terminal),
        show_status: false,
        ..Default::default()
    };
    assert_eq!(app.status_row(), StatusRowSource::Runtime);

    let metrics = app.geometry.cell_metrics();
    let frame_size = PhysicalSize::new(
        (renderer::SIDEBAR_COLS as u32 + 8) * metrics.width(),
        4 * metrics.height(),
    );
    let position = PhysicalPosition::new(sidebar_pixel_width(metrics.width()), 1.0);
    assert_eq!(
        app.grid_point_in_frame(position, frame_size),
        Some(GridPoint::new(0, 0)),
        "the same background-only row must remain selectable"
    );
    assert_eq!(
        app.mouse_cell_in_frame(position, frame_size),
        Some((0, 0)),
        "the same background-only row must remain mouse-reportable"
    );
}

#[test]
fn paste_is_gated_in_the_app_without_a_terminal() {
    // With no terminal state, mode 2004 is unavailable, so encode_paste
    // gates rather than emitting an unbracketed paste.
    assert_eq!(encode_paste("hello", false), Err(PasteReject::Unbracketed));
}

#[test]
fn paste_is_bracketed_when_mode_2004_is_enabled() {
    let mut app = NorenApp::default();
    let mut terminal = TerminalState::new(2, 4).expect("valid terminal");
    terminal.feed_bytes(b"\x1b[?2004h");
    app.terminal = Some(terminal);

    assert_eq!(
        app.paste_bytes("ls -la"),
        Ok(b"\x1b[200~ls -la\x1b[201~".to_vec())
    );
}

#[test]
fn paste_is_gated_when_mode_2004_is_off_or_terminal_unavailable() {
    let mut app = NorenApp::default();
    // No terminal state at all: bracketed paste cannot be enabled.
    assert_eq!(app.paste_bytes("ls"), Err(PasteReject::Unbracketed));

    // Terminal state present but the application never enabled 2004.
    let terminal = TerminalState::new(2, 4).expect("valid terminal");
    app.terminal = Some(terminal);
    assert_eq!(app.paste_bytes("ls"), Err(PasteReject::Unbracketed));
}

#[test]
fn copy_selection_drops_an_expired_selection_without_copying() {
    let mut app = NorenApp::default();
    let mut terminal = TerminalState::new(2, 6).expect("valid terminal");
    terminal.feed_bytes(b"hello");
    app.selection = Some(Selection::new(
        &terminal,
        SelectionMode::Char,
        GridPoint::new(0, 0),
        GridPoint::new(0, 4),
    ));
    terminal.resize(3, 8).expect("valid resize");
    app.terminal = Some(terminal);

    // The resize expired the selection's stamp; copy clears the selection
    // and returns before any system clipboard access.
    app.copy_selection();
    assert!(app.selection.is_none());
}

#[test]
fn select_entire_grid_captures_all_visible_content() {
    let mut app = NorenApp::default();
    let mut terminal = TerminalState::new(3, 6).expect("valid terminal");
    terminal.feed_bytes(b"abc\r\ndef");
    app.terminal = Some(terminal);

    app.select_entire_grid();
    let terminal = app.terminal.as_ref().expect("terminal present");
    assert_eq!(
        app.selection
            .as_ref()
            .map(|selection| selection.extract(terminal)),
        Some("abc\ndef".to_owned())
    );
}

#[test]
fn terminal_event_finishes_the_session_without_closing_the_window() {
    let mut app = NorenApp::default();
    app.finish_pty("Noren shell reached EOF");

    assert!(app.pty.is_none());
    assert_eq!(app.status, "Noren shell reached EOF");
    assert!(app.show_status);
    assert!(app.redraw_needed);
}

#[test]
fn toggle_diagnostics_reports_live_state_and_clears_on_exit() {
    let mut app = NorenApp::default();
    let mut terminal = TerminalState::new(4, 8).expect("valid terminal");
    terminal.feed_bytes(b"\x1b[?1h");
    app.terminal = Some(terminal);

    app.toggle_diagnostics();
    assert!(app.diagnostics_visible);
    assert!(
        app.diagnostics_line.contains("grid=4x8"),
        "diagnostics: {}",
        app.diagnostics_line
    );
    assert!(
        app.diagnostics_line
            .contains("modes=alt:0 cursor:1 keypad:0"),
        "diagnostics: {}",
        app.diagnostics_line
    );
    assert!(
        app.diagnostics_line.contains("child=not launched"),
        "diagnostics: {}",
        app.diagnostics_line
    );

    app.toggle_diagnostics();
    assert!(!app.diagnostics_visible);
    assert!(app.diagnostics_line.is_empty());
}

#[test]
fn toggle_diagnostics_never_repeats_terminal_content() {
    let mut app = NorenApp::default();
    let mut terminal = TerminalState::new(2, 40).expect("valid terminal");
    terminal.feed_bytes(b"SECRET-MARKER-9f8e7d6c\n\n\n\n");
    app.terminal = Some(terminal);

    app.toggle_diagnostics();
    assert!(app.diagnostics_visible);
    assert!(
        !app.diagnostics_line.contains("SECRET"),
        "diagnostics: {}",
        app.diagnostics_line
    );
    assert!(
        !app.diagnostics_line.contains("9f8e7d6c"),
        "diagnostics: {}",
        app.diagnostics_line
    );
}

#[test]
fn configured_cell_sizes_drive_the_app_geometry() {
    let config = AppConfig::parse("[font]\ncell_width = 20\ncell_height = 40\n")
        .expect("valid configuration");
    let app = NorenApp::new(config);
    let mut expected = GridGeometry::with_cells(20, 40).expect("valid geometry");
    let mut actual = app.geometry;
    let grid = actual.update(Resize::new(900, 600)).expect("grid");
    assert_eq!(grid, expected.update(Resize::new(900, 600)).expect("grid"));
    assert_eq!((grid.rows(), grid.cols()), (15, 45));
}

/// The `[theme]` selection reaches the app's renderer input: `NorenApp`
/// carries exactly the palette the configuration named, and a missing
/// `[theme]` section carries the dark default. This is the app-level half of
/// the theme's reachability chain (configuration → app → renderer → pixels);
/// the frame oracle proves the drawing half offscreen.
#[test]
fn configured_theme_reaches_the_app_renderer_input() {
    for (text, expected) in [
        ("", noren_app::theme::ThemeName::Dark),
        (
            "[theme]\nname = \"dark\"\n",
            noren_app::theme::ThemeName::Dark,
        ),
        (
            "[theme]\nname = \"light\"\n",
            noren_app::theme::ThemeName::Light,
        ),
        (
            "[theme]\nname = \"high-contrast\"\n",
            noren_app::theme::ThemeName::HighContrast,
        ),
    ] {
        let config = AppConfig::parse(text).expect("valid configuration");
        assert_eq!(
            config.theme().name(),
            expected,
            "config must parse {text:?} to {expected}"
        );
        let app = NorenApp::new(config);
        assert_eq!(
            app.theme,
            expected.palette(),
            "NorenApp must carry the palette named by {text:?}"
        );
    }
    assert_eq!(NorenApp::default().theme, noren_app::theme::DARK);
}

#[test]
fn workspace_starts_empty_with_no_sidebar_rows() {
    let state = WorkspaceState::new();
    assert!(state.registry().is_empty());
    assert!(state.sidebar().is_empty());
    assert!(state.sidebar().rows().is_empty());
    assert!(
        state.sidebar().empty_state().is_some(),
        "empty sidebar must carry an empty-state notice"
    );
    assert_eq!(state.sidebar().viewport(), None);
    assert_eq!(state.sidebar().selected_row_count(), 0);
}

#[test]
fn creating_a_session_adds_a_session_row_to_the_sidebar() {
    let mut state = WorkspaceState::new();
    let id = state.create_session(SessionKind::Local);

    assert_eq!(state.registry().len(), 1);
    let rows = state.sidebar().rows();
    assert_eq!(rows.len(), 1, "sidebar must reflect the new session");
    assert_eq!(rows[0].kind(), EntryKind::Session);
    assert_eq!(rows[0].label(), id.to_string());
    assert!(
        rows[0].detail().is_some_and(|d| d.contains("local")),
        "session detail should mention the kind, got {:?}",
        rows[0].detail()
    );
}

#[test]
fn selecting_a_session_marks_exactly_one_row_selected() {
    let mut state = WorkspaceState::new();
    let first = state.create_session(SessionKind::Local);
    let _second = state.create_session(SessionKind::Local);
    assert_eq!(
        state.sidebar().selected_row_count(),
        0,
        "no session is selected initially"
    );

    state.select_session(first).expect("first session is live");

    assert_eq!(state.sidebar().selected_row_count(), 1);
    let selected = state
        .sidebar()
        .rows()
        .iter()
        .find(|row| row.is_selected())
        .expect("exactly one selected row");
    assert_eq!(selected.label(), first.to_string());
    let viewport = state
        .sidebar()
        .viewport()
        .expect("a selected session yields a viewport");
    assert_eq!(viewport.session_id(), first);
}

#[test]
fn selecting_the_other_session_moves_the_single_selection() {
    let mut state = WorkspaceState::new();
    let first = state.create_session(SessionKind::Local);
    let second = state.create_session(SessionKind::Local);
    state.select_session(first).expect("first is live");
    assert_eq!(state.sidebar().selected_row_count(), 1);

    state.select_session(second).expect("second is live");

    assert_eq!(state.sidebar().selected_row_count(), 1);
    let selected_label = state
        .sidebar()
        .rows()
        .iter()
        .find(|row| row.is_selected())
        .map(|row| row.label())
        .expect("one selected row");
    assert_eq!(selected_label, second.to_string());
}

#[test]
fn closing_the_selected_session_leaves_a_coherent_view() {
    let mut state = WorkspaceState::new();
    let first = state.create_session(SessionKind::Local);
    let _second = state.create_session(SessionKind::Local);
    state.select_session(first).expect("first is live");
    assert!(state.sidebar().viewport().is_some());

    state.close_session(first).expect("first is live");

    assert_eq!(state.registry().len(), 1);
    let rows = state.sidebar().rows();
    assert_eq!(rows.len(), 1, "closed session must vanish from sidebar");
    assert!(
        !rows.iter().any(|row| row.label() == first.to_string()),
        "closed session id must not appear"
    );
    assert_eq!(
        state.sidebar().selected_row_count(),
        0,
        "closing the selected session clears the selection"
    );
    assert!(
        state.sidebar().viewport().is_none(),
        "no viewport without a selection"
    );
}

#[test]
fn closing_all_sessions_shows_the_empty_state() {
    let mut state = WorkspaceState::new();
    let id = state.create_session(SessionKind::Local);
    state.close_session(id).expect("session is live");

    assert!(state.registry().is_empty());
    assert!(state.sidebar().is_empty());
    assert!(
        state.sidebar().empty_state().is_some(),
        "empty registry must produce an empty-state sidebar"
    );
    assert_eq!(state.sidebar().viewport(), None);
}

#[test]
fn selecting_a_stale_id_does_not_panic_or_mutate_the_view() {
    let mut state = WorkspaceState::new();
    let id = state.create_session(SessionKind::Local);
    state.close_session(id).expect("session is live");
    let rows_before = state.sidebar().rows().len();

    let result = state.select_session(id);

    assert_eq!(result, Err(SessionError::UnknownSession));
    assert_eq!(
        state.sidebar().rows().len(),
        rows_before,
        "a failed select must not change the view"
    );
}

#[test]
fn palette_carries_all_four_canonical_commands() {
    let state = WorkspaceState::new();
    let palette = state.palette();
    assert_eq!(palette.len(), 4);
    for id in [
        CommandId::SESSION_CREATE,
        CommandId::SESSION_SELECT,
        CommandId::SESSION_CLOSE,
        CommandId::SIDEBAR_FOCUS,
    ] {
        assert!(palette.get(id).is_some(), "palette must include {id}");
    }
    let hits = palette.search("session");
    assert!(
        hits.iter()
            .any(|hit| hit.command().id() == CommandId::SESSION_CREATE),
        "searching 'session' must find the create command"
    );
}

#[test]
fn pending_resize_applies_the_runtime_status_row_contract_to_terminal_state() {
    let mut app = NorenApp {
        terminal: Some(TerminalState::new(1, 1).expect("valid seed terminal")),
        ..Default::default()
    };
    let grid = app
        .geometry
        .update(Resize::new(WINDOW_WIDTH, WINDOW_HEIGHT))
        .expect("default window has a grid");
    assert_eq!(grid.rows(), 30);
    app.pending_grid = Some(grid);

    // This is the production resize seam called by `about_to_wait`. If it
    // regresses to `grid.rows()`, the assertion below observes 30 directly.
    app.apply_pending_resize();

    assert_eq!(
        app.terminal.as_ref().expect("terminal retained").size(),
        (29, terminal_cols(grid.cols()))
    );
}

/// MINOR-2: `sidebar_text_lines` is the seam between the view model and the
/// renderer. Drive a real `SidebarView` built by the workspace from a live
/// registry — not hardcoded strings — so a change in how rows are formatted
/// is caught here.
#[test]
fn sidebar_text_lines_format_a_real_workspace_sidebar() {
    // Empty workspace: the empty-state notice is the sole line.
    let empty = WorkspaceState::new();
    assert_eq!(
        sidebar_text_lines(empty.sidebar()),
        vec!["No sessions".to_string()],
        "empty workspace renders its empty-state notice"
    );

    let mut state = WorkspaceState::new();
    let first = state.create_session(SessionKind::Local);
    let second = state.create_session(SessionKind::Local);
    state.select_session(first).expect("first session is live");

    let lines = sidebar_text_lines(state.sidebar());
    assert_eq!(lines.len(), 2, "one formatted line per sidebar row");

    // The selected row is prefixed with '>' and the unselected with a
    // space; both carry the real descriptor's label and detail.
    assert!(
        lines[0].starts_with("> "),
        "selected row must be marked with '>': {:?}",
        lines[0]
    );
    assert!(
        lines[1].starts_with("  "),
        "unselected row must be marked with a space: {:?}",
        lines[1]
    );
    assert!(
        lines[0].contains(first.to_string().as_str()),
        "selected row carries the session label: {:?}",
        lines[0]
    );
    assert!(
        lines[1].contains(second.to_string().as_str()),
        "unselected row carries the session label: {:?}",
        lines[1]
    );
    // A freshly created session sits at the Starting status, so the detail
    // is derived from the real descriptor, not a constant.
    assert!(
        lines[0].contains("local · starting"),
        "detail comes from the real descriptor: {:?}",
        lines[0]
    );
}

// ── Pass-through gate integration tests ──────────────────────────────

/// The palette policy claims exactly two chords: Super+Escape (exit) and
/// Super+p (palette). Both live in the Super/Cmd modifier space that the
/// pinned Zellij v0.44.3 corpus never binds.
#[test]
fn palette_policy_claims_exactly_super_escape_and_super_p() {
    let policy = palette_policy(KeymapConfig::default());
    let claims = policy.claims();
    assert_eq!(claims.len(), 2, "exactly two claims");
    let corpus = passthrough::zellij_default_bindings();
    assert!(
        collisions(claims, &corpus).is_empty(),
        "palette policy must not collide with Zellij defaults"
    );
    let super_p =
        Chord::new(GateKeyCode::Char('p'), GateModifiers::empty().super_key()).expect("normalized");
    assert_eq!(
        policy.palette_claim().unwrap().seq.chords()[0],
        super_p,
        "palette claim is Super+p"
    );
}

/// Super+p is intercepted by the gate (opens the palette) and produces no
/// PTY bytes — confirming the palette claim works.
#[test]
fn super_p_is_intercepted_as_palette_open() {
    let policy = palette_policy(KeymapConfig::default());
    let mut gate = PassthroughGate::new();
    let chord =
        Chord::new(GateKeyCode::Char('p'), GateModifiers::empty().super_key()).expect("normalized");
    let decision = gate.press(&policy, chord);
    assert_eq!(
        decision.kind,
        GateKind::Intercepted(PassthroughAction::OpenCommandPalette)
    );
    assert!(decision.replayed.is_empty());
}

// ── Palette action tests ─────────────────────────────────────────────

/// Running the create command adds a session and the sidebar shows it.
#[test]
fn palette_create_action_adds_a_session_to_the_sidebar() {
    let mut app = NorenApp::default();
    assert!(app.workspace.sidebar().is_empty());

    app.run_workspace_action(WorkspaceAction::CreateSession);

    assert_eq!(app.workspace.registry().len(), 1);
    assert_eq!(app.workspace.sidebar().rows().len(), 1);
}

/// With one PTY, running select restores its actual owner rather than
/// moving the marker to an inactive model entry.
#[test]
fn palette_select_action_restores_the_active_session() {
    let mut app = NorenApp::default();
    let active = app.workspace.create_session(SessionKind::Local);
    let _inactive_one = app.workspace.create_session(SessionKind::Local);
    let _inactive_two = app.workspace.create_session(SessionKind::Local);
    app.workspace
        .select_session(active)
        .expect("active owner is live");
    app.active_session = Some(active);

    app.run_workspace_action(WorkspaceAction::SelectSession);

    assert_eq!(app.workspace.registry().selected(), Some(active));
    assert_eq!(app.active_session, Some(active));
}

#[test]
fn palette_select_cannot_move_input_ownership_to_an_inactive_session() {
    let mut app = NorenApp::default();
    let active = app.workspace.create_session(SessionKind::Local);
    let inactive = app.workspace.create_session(SessionKind::Local);
    app.workspace
        .select_session(inactive)
        .expect("inactive model row is selectable below the application seam");
    app.active_session = Some(active);

    app.run_workspace_action(WorkspaceAction::SelectSession);

    assert_eq!(app.workspace.registry().selected(), Some(active));
    assert_eq!(app.active_session, Some(active));
}

/// Running close removes the selected session and the sidebar updates.
#[test]
fn palette_close_action_removes_the_selected_session() {
    let mut app = NorenApp::default();
    let first = app.workspace.create_session(SessionKind::Local);
    let _second = app.workspace.create_session(SessionKind::Local);
    app.workspace.select_session(first).expect("first is live");
    assert_eq!(app.workspace.sidebar().rows().len(), 2);

    app.run_workspace_action(WorkspaceAction::CloseSession);

    assert_eq!(app.workspace.registry().len(), 1);
    assert_eq!(app.workspace.sidebar().rows().len(), 1);
    assert!(
        !app.workspace
            .sidebar()
            .rows()
            .iter()
            .any(|r| r.label() == first.to_string()),
        "closed session must not appear"
    );
}

/// Closing the row that owns the live PTY is a real close now: the child is
/// reaped through the bounded shutdown *before* the row is removed, the live
/// surface is detached, and — with no other live session left — the app falls
/// back to an honest empty view (no terminal, truthful status) instead of a
/// dead session's frozen frame.
///
/// This replaces `palette_close_cannot_remove_the_live_pty_owner`, which
/// pinned the interim behaviour where close refused every live row because it
/// could not reap a child; `close_session` can and does.
///
/// Mutation check: removing the reaping/detaching from `close_session`
/// (leaving the child running behind a removed row, or leaving the closed
/// session's surface attached) fails every assertion past the first.
#[test]
fn palette_close_reaps_the_live_owner_and_falls_back_to_an_empty_view() {
    let home = AppTestHome::new();
    let mut app = home.app();
    app.run_workspace_action(WorkspaceAction::CreateSession);
    let only = registry_ids(&app)[0];

    app.run_workspace_action(WorkspaceAction::CloseSession);

    assert!(
        app.workspace.registry().get(only).is_none(),
        "the closed row is gone from the registry"
    );
    assert!(
        app.pty.is_none() && app.terminal.is_none(),
        "no surface may outlive its session"
    );
    assert_eq!(app.active_session, None);
    assert!(
        app.parked_sessions.is_empty(),
        "the child handle was reaped and dropped, not parked or leaked"
    );
    assert!(app.workspace.sidebar().is_empty());
    assert!(app.show_status, "an empty workspace must say so honestly");
}

/// Escape dismisses the palette without running a command.
#[test]
fn escape_dismisses_palette_without_running_a_command() {
    let mut app = NorenApp::default();
    app.open_palette();
    assert!(app.palette_open);
    let count_before = app.workspace.registry().len();

    // Simulate Escape key: handle_palette_key checks for NamedKey::Escape.
    // We test the effect (close_palette) directly because a full winit
    // KeyEvent cannot be fabricated outside winit: `KeyEvent::platform_specific`
    // is private with no public constructor (DeviceId has `dummy()`, but the
    // event cannot be completed without that field).
    app.close_palette();

    assert!(!app.palette_open, "palette must be dismissed");
    assert_eq!(
        app.workspace.registry().len(),
        count_before,
        "no command must have run"
    );
}

// ── Configurable keybinding tests ────────────────────────────────────

fn gate_chord(code: GateKeyCode, modifiers: GateModifiers) -> Chord {
    Chord::new(code, modifiers).expect("normalized test chord")
}

fn super_chord(character: char) -> Chord {
    gate_chord(
        GateKeyCode::Char(character),
        GateModifiers::empty().super_key(),
    )
}

/// Mutation proof (b): the configured palette chord must reach the live
/// pass-through policy. If `palette_policy` ignored the keymap and always
/// claimed the hard-coded `super+p`, `super+k` would forward (never open)
/// and this test would fail.
#[test]
fn custom_palette_chord_opens_the_palette_and_releases_the_default() {
    let config = AppConfig::parse("[keys]\npalette_open = \"super+k\"\n")
        .expect("super+k is a claimable, distinct opener chord");
    let mut app = NorenApp::new(config);
    assert!(!app.palette_open);

    let consumed = app.gate_pressed_chord(super_chord('k'), InputMode::normal());
    assert!(
        consumed,
        "the configured chord must be consumed by the gate"
    );
    assert!(
        app.palette_open,
        "the configured chord must open the palette"
    );
}

#[test]
fn custom_palette_chord_replaces_the_default_claim_in_the_policy() {
    let config = AppConfig::parse("[keys]\npalette_open = \"super+k\"\n")
        .expect("super+k is a claimable, distinct opener chord");
    let app = NorenApp::new(config);
    let mut gate = PassthroughGate::new();

    let decision = gate.press(&app.passthrough_policy, super_chord('p'));
    assert_eq!(
        decision.kind,
        GateKind::Forwarded,
        "the default super+p must no longer be claimed"
    );

    let decision = gate.press(&app.passthrough_policy, super_chord('k'));
    assert_eq!(
        decision.kind,
        GateKind::Intercepted(PassthroughAction::OpenCommandPalette),
        "the configured super+k must be the palette claim"
    );
}

/// With no `[keys]` section the binary behaves exactly as before: `super+p`
/// opens the palette and the bare `c`/`s`/`x`/`f` characters dispatch the
/// four commands — including under modifiers, which the pre-configuration
/// palette ignored.
#[test]
fn default_keymap_keeps_the_pre_configuration_palette_behaviour() {
    let mut app = NorenApp::default();
    assert_eq!(app.keys, KeymapConfig::default());

    assert!(app.gate_pressed_chord(super_chord('p'), InputMode::normal()));
    assert!(
        app.palette_open,
        "the default super+p must still open the palette"
    );

    let before = app.workspace.registry().len();
    app.handle_palette_key_impl(
        &WinitKey::Character("c".into()),
        ElementState::Pressed,
        false,
    );
    assert!(!app.palette_open, "dispatch closes the palette");
    assert_eq!(
        app.workspace.registry().len(),
        before + 1,
        "the default c must still create a session"
    );

    // The legacy modifier-insensitive character match survives: the palette
    // matched logical characters regardless of held modifiers.
    app.open_palette();
    app.modifiers = Modifiers::empty().ctrl();
    app.handle_palette_key_impl(
        &WinitKey::Character("c".into()),
        ElementState::Pressed,
        false,
    );
    assert_eq!(app.workspace.registry().len(), before + 2);

    // An unbound single character still dismisses the palette.
    app.open_palette();
    app.handle_palette_key_impl(
        &WinitKey::Character("z".into()),
        ElementState::Pressed,
        false,
    );
    assert!(!app.palette_open, "an unbound character still dismisses");
    assert_eq!(app.workspace.registry().len(), before + 2);
}

/// A configured command chord with modifiers dispatches only on the exact
/// chord: the bare character neither dispatches nor runs the wrong command.
#[test]
fn custom_command_chord_dispatches_only_on_the_exact_chord() {
    let config = AppConfig::parse("[keys]\nsession_create = \"ctrl+n\"\n")
        .expect("ctrl+n is a valid command chord");
    let mut app = NorenApp::new(config);
    let before = app.workspace.registry().len();

    // Exact chord dispatches even though the binding carries modifiers.
    app.open_palette();
    app.modifiers = Modifiers::empty().ctrl();
    app.handle_palette_key_impl(
        &WinitKey::Character("n".into()),
        ElementState::Pressed,
        false,
    );
    assert!(!app.palette_open, "dispatch closes the palette");
    assert_eq!(
        app.workspace.registry().len(),
        before + 1,
        "the configured ctrl+n must create a session"
    );

    // The bare character does not dispatch: it dismisses without running.
    app.open_palette();
    app.modifiers = Modifiers::empty();
    app.handle_palette_key_impl(
        &WinitKey::Character("n".into()),
        ElementState::Pressed,
        false,
    );
    assert!(!app.palette_open);
    assert_eq!(app.workspace.registry().len(), before + 1);

    // The default c binding was replaced, so bare c no longer creates.
    app.open_palette();
    app.handle_palette_key_impl(
        &WinitKey::Character("c".into()),
        ElementState::Pressed,
        false,
    );
    assert!(!app.palette_open);
    assert_eq!(
        app.workspace.registry().len(),
        before + 1,
        "the replaced default must not dispatch"
    );
}

/// A command chord bound to a named key dispatches when that key is pressed
/// inside the open palette.
#[test]
fn named_key_command_chord_dispatches_in_the_open_palette() {
    let config = AppConfig::parse("[keys]\nsession_select = \"f2\"\n")
        .expect("f2 is a valid, distinct command chord");
    let mut app = NorenApp::new(config);
    app.open_palette();
    let selected = app.workspace.registry().selected();

    app.handle_palette_key_impl(&WinitKey::Named(NamedKey::F2), ElementState::Pressed, false);
    assert!(!app.palette_open, "the bound named key dispatches");
    assert_eq!(
        app.workspace.registry().selected(),
        selected,
        "session_select on an empty active session keeps the selection"
    );
}

/// The palette's one-glyph shortcut labels follow the configured chords
/// rather than the compiled-in characters.
#[test]
fn palette_labels_follow_the_configured_command_chords() {
    let config = AppConfig::parse("[keys]\nsession_create = \"n\"\nsession_close = \"f2\"\n")
        .expect("valid, distinct command chords");
    let app = NorenApp::new(config);
    // Selection 1 keeps lines 0 and 2 unselected, so each begins with the
    // marker, the shortcut glyph, and a space.
    let lines = palette_text_lines(app.workspace.palette(), 1, &app.keys);
    assert!(
        lines[0].starts_with(" N "),
        "create bound to n must show N, got {:?}",
        lines[0]
    );
    assert!(
        lines[2].starts_with(" ? "),
        "close bound to f2 has no glyph and must show ?, got {:?}",
        lines[2]
    );

    let default_lines = palette_text_lines(app.workspace.palette(), 1, &KeymapConfig::default());
    for (index, key) in ['C', 'S', 'X', 'F'].into_iter().enumerate() {
        let glyphs: Vec<char> = default_lines[index].chars().collect();
        assert_eq!(
            glyphs.first(),
            Some(&(if index == 1 { ']' } else { ' ' })),
            "line {index} marker"
        );
        assert_eq!(
            glyphs.get(1),
            Some(&key),
            "default line {index} must still show {key}, got {:?}",
            default_lines[index]
        );
    }
}

/// The pass-through seam is the live route for the palette claim: a
/// forwarded chord is not consumed, an intercepted one is.
#[test]
fn gate_pressed_chord_reports_consumption_for_each_outcome() {
    let mut app = NorenApp::default();
    assert!(!app.gate_pressed_chord(
        gate_chord(GateKeyCode::Char('a'), GateModifiers::empty()),
        InputMode::normal()
    ));
    assert!(
        !app.palette_open,
        "unclaimed chords forward without opening"
    );
    assert!(app.gate_pressed_chord(
        gate_chord(GateKeyCode::Escape, GateModifiers::empty().super_key()),
        InputMode::normal()
    ));
    assert!(
        !app.palette_open,
        "the exit leader is consumed, not palette"
    );
}

// ── Session status observation tests ─────────────────────────────────

/// Observing Running after creation changes the sidebar detail from
/// "starting" to "running".
#[test]
fn observe_running_updates_the_sidebar_detail() {
    let mut state = WorkspaceState::new();
    let id = state.create_session(SessionKind::Local);

    // Freshly created: status is "starting".
    let detail_before = state
        .sidebar()
        .rows()
        .first()
        .and_then(|r| r.detail())
        .unwrap_or_default();
    assert!(
        detail_before.contains("starting"),
        "fresh session should show starting, got {detail_before}"
    );

    state.observe_session(id, SessionStatus::Running);

    let detail_after = state
        .sidebar()
        .rows()
        .first()
        .and_then(|r| r.detail())
        .unwrap_or_default();
    assert!(
        detail_after.contains("running"),
        "observed session should show running, got {detail_after}"
    );
}

/// Observing Exited after Running changes the sidebar detail to "exited".
#[test]
fn observe_exited_after_running_updates_the_sidebar() {
    let mut state = WorkspaceState::new();
    let id = state.create_session(SessionKind::Local);
    state.observe_session(id, SessionStatus::Running);
    state.observe_session(id, SessionStatus::Exited { code: Some(0) });

    let detail = state
        .sidebar()
        .rows()
        .first()
        .and_then(|r| r.detail())
        .unwrap_or_default();
    assert!(
        detail.contains("exited"),
        "exited session should show exited, got {detail}"
    );
}

// ── Palette text rendering ───────────────────────────────────────────

/// The palette text lines include all four commands with a selection
/// marker on the currently selected one.
#[test]
fn palette_text_lines_show_selection_marker() {
    let state = WorkspaceState::new();
    let palette = state.palette();
    let lines = palette_text_lines(palette, 1, &KeymapConfig::default());
    assert_eq!(lines.len(), 4, "four commands");
    assert!(
        lines[1].starts_with(']'),
        "second line must be selected, got {:?}",
        lines[1]
    );
    assert!(
        !lines[0].starts_with(']'),
        "first line must not be selected, got {:?}",
        lines[0]
    );
}

// ── Authoritative application mouse modes ───────────────────────────

#[test]
fn app_multi_param_1002_1006_output_drives_sgr_encoding() {
    let mut app = NorenApp {
        terminal: Some(TerminalState::new(4, 8).expect("valid terminal")),
        ..Default::default()
    };
    app.apply_pty_output(b"\x1b[?1002;1006h");

    assert!(app.mouse_reportable(), "1002 must enable reporting");
    let drag = PointerEvent::move_to(Some(EncoderButton::Left), 2, 1, PointerModifiers::empty());
    assert_eq!(
        app.encode_mouse(drag).as_deref(),
        Some(b"\x1b[<32;3;2M".as_slice()),
        "1006 must select SGR for the application encoding path"
    );
}

#[test]
fn app_split_mouse_mode_output_uses_incremental_terminal_authority() {
    let mut app = NorenApp {
        terminal: Some(TerminalState::new(4, 8).expect("valid terminal")),
        ..Default::default()
    };
    let press = PointerEvent::press(EncoderButton::Left, 0, 0, PointerModifiers::empty());

    app.apply_pty_output(b"\x1b[?1002;");
    assert!(
        !app.mouse_reportable(),
        "an incomplete DECSET has no effect"
    );
    assert_eq!(app.encode_mouse(press), None);

    app.apply_pty_output(b"1006h");
    assert!(app.mouse_reportable());
    assert_eq!(
        app.encode_mouse(press).as_deref(),
        Some(b"\x1b[<0;1;1M".as_slice())
    );
}

#[test]
fn app_decrst_mouse_output_disables_encoding() {
    let mut app = NorenApp {
        terminal: Some(TerminalState::new(4, 8).expect("valid terminal")),
        ..Default::default()
    };
    let press = PointerEvent::press(EncoderButton::Left, 0, 0, PointerModifiers::empty());

    app.apply_pty_output(b"\x1b[?1000;1006h");
    assert_eq!(
        app.encode_mouse(press).as_deref(),
        Some(b"\x1b[<0;1;1M".as_slice())
    );

    app.apply_pty_output(b"\x1b[?1000l");
    assert!(!app.mouse_reportable());
    assert_eq!(app.encode_mouse(press), None);
    assert_eq!(
        app.current_mouse_modes(),
        MouseModes::disabled().with_sgr(true),
        "DECRST 1000 must not reset the independent 1006 flag"
    );
}

#[test]
fn app_current_mouse_modes_projects_all_six_terminal_flags() {
    let cases: &[(&[u8], MouseModes)] = &[
        (b"\x1b[?1000h", MouseModes::disabled().with_normal(true)),
        (
            b"\x1b[?1002h",
            MouseModes::disabled().with_button_event(true),
        ),
        (b"\x1b[?1003h", MouseModes::disabled().with_any_event(true)),
        (b"\x1b[?1005h", MouseModes::disabled().with_utf8(true)),
        (b"\x1b[?1006h", MouseModes::disabled().with_sgr(true)),
        (b"\x1b[?1015h", MouseModes::disabled().with_urxvt(true)),
    ];

    for &(output, expected) in cases {
        let mut app = NorenApp {
            terminal: Some(TerminalState::new(2, 4).expect("valid terminal")),
            ..Default::default()
        };
        app.apply_pty_output(output);
        assert_eq!(app.current_mouse_modes(), expected, "output {output:?}");
    }
}

#[test]
fn app_mouse_authority_defaults_disabled_and_live_mutation_enables_encoding() {
    let mut app = NorenApp {
        terminal: Some(TerminalState::new(4, 8).expect("valid terminal")),
        ..Default::default()
    };
    let press = PointerEvent::press(EncoderButton::Left, 0, 0, PointerModifiers::empty());

    assert_eq!(app.current_mouse_modes(), MouseModes::disabled());
    assert_eq!(app.encode_mouse(press), None);

    app.apply_pty_output(b"\x1b[?1000h");
    assert_eq!(
        app.encode_mouse(press).as_deref(),
        Some(b"\x1b[M\x20\x21\x21".as_slice()),
        "encoding must observe the mutated terminal rather than a separate disabled default"
    );
}

#[test]
fn apply_pty_output_preserves_terminal_bytes_and_order() {
    let mut app = NorenApp {
        terminal: Some(TerminalState::new(2, 8).expect("valid terminal")),
        ..Default::default()
    };
    app.redraw_needed = false;

    app.apply_pty_output(b"ab\x1b[?1000hcd");

    let snapshot = app.terminal.as_ref().expect("terminal present").snapshot();
    assert_eq!(snapshot.lines()[0], "abcd");
    assert!(app.current_mouse_modes().is_tracked());
    assert!(app.redraw_needed);
}

// ── Tracking / selection-bypass policy ──────────────────────────────

/// With no tracking mode set, events are not reportable and local
/// selection runs unchanged — byte-identical to the pre-tracking behaviour.
#[test]
fn no_tracking_mode_means_not_reportable() {
    let app = NorenApp::default();
    assert!(!app.mouse_reportable());
}

/// Mode 1000 without Shift: tracking active, events go to the PTY.
#[test]
fn mode_1000_without_shift_is_reportable() {
    let mut app = NorenApp {
        terminal: Some(TerminalState::new(2, 4).expect("valid terminal")),
        ..Default::default()
    };
    app.apply_pty_output(b"\x1b[?1000h");
    assert!(app.mouse_reportable());
}

/// Shift bypasses tracking so the user can still select text in a program
/// that enabled mouse reporting. This is the standard xterm/iTerm policy.
#[test]
fn shift_bypasses_tracking_for_local_selection() {
    let mut app = NorenApp {
        terminal: Some(TerminalState::new(2, 4).expect("valid terminal")),
        ..Default::default()
    };
    app.apply_pty_output(b"\x1b[?1000h");

    assert!(app.mouse_reportable(), "active without Shift");
    let press = PointerEvent::press(EncoderButton::Left, 0, 0, app.pointer_modifiers());
    assert_eq!(
        app.encode_mouse(press).as_deref(),
        Some(b"\x1b[M\x20\x21\x21".as_slice()),
        "the non-Shift encoding remains byte-compatible"
    );

    app.modifiers = Modifiers::empty().shift();
    assert!(!app.mouse_reportable(), "Shift bypasses tracking");
}

// ── Encoder integration: tracking modes ─────────────────────────────

/// Mode 1000 with X10 byte form: a left click at (0,0) produces
/// `ESC[M` followed by three offset bytes (32, 33, 33 for button-0,
/// col-1, row-1).
#[test]
fn mode_1000_x10_left_click_at_origin() {
    let grid = MouseGrid::new(10, 40).expect("grid");
    let modes = MouseModes::disabled().with_normal(true);
    let event = PointerEvent::press(EncoderButton::Left, 0, 0, PointerModifiers::empty());
    let bytes = MouseEncoder::encode(event, modes, grid).expect("must encode");
    // Cb=0→32, Cx=1→33, Cy=1→33
    assert_eq!(bytes, b"\x1b[M\x20\x21\x21");
}

/// Mode 1002 (button-event): drag with left button held produces motion
/// reports. A Move with a held button must produce bytes under 1002.
#[test]
fn mode_1002_drag_produces_motion_report() {
    let grid = MouseGrid::new(10, 40).expect("grid");
    let modes = MouseModes::disabled()
        .with_button_event(true)
        .with_sgr(true);
    let event = PointerEvent::move_to(Some(EncoderButton::Left), 2, 0, PointerModifiers::empty());
    let bytes = MouseEncoder::encode(event, modes, grid).expect("must encode");
    // Cb = 0 (button1) | 32 (motion) = 32; Cx=3, Cy=1
    let report = String::from_utf8(bytes).expect("SGR");
    assert_eq!(report, "\x1b[<32;3;1M");
}

/// Mode 1003 (any-event): hover with no button held produces motion
/// reports. Under 1002 alone this would return None.
#[test]
fn mode_1003_hover_produces_motion_report() {
    let grid = MouseGrid::new(10, 40).expect("grid");
    let modes = MouseModes::disabled().with_any_event(true).with_sgr(true);
    let event = PointerEvent::move_to(None, 2, 0, PointerModifiers::empty());
    let bytes = MouseEncoder::encode(event, modes, grid).expect("must encode");
    // Cb = 3 (no-button) | 32 (motion) = 35; Cx=3, Cy=1
    let report = String::from_utf8(bytes).expect("SGR");
    assert_eq!(report, "\x1b[<35;3;1M");
}

/// Mode 1003 hover must NOT report under 1002 (button-event) alone.
#[test]
fn mode_1002_hover_without_button_produces_nothing() {
    let grid = MouseGrid::new(10, 40).expect("grid");
    let modes = MouseModes::disabled().with_button_event(true);
    let event = PointerEvent::move_to(None, 2, 0, PointerModifiers::empty());
    assert_eq!(
        MouseEncoder::encode(event, modes, grid),
        None,
        "1002 must not report hover"
    );
}

/// Mode 1015 (urxvt): `CSI Cb ; Cx ; Cy M` — decimal, no angle bracket.
#[test]
fn mode_1015_uses_urxvt_format() {
    let grid = MouseGrid::new(10, 40).expect("grid");
    let modes = MouseModes::disabled().with_normal(true).with_urxvt(true);
    let event = PointerEvent::press(EncoderButton::Left, 0, 0, PointerModifiers::empty());
    let bytes = MouseEncoder::encode(event, modes, grid).expect("must encode");
    let report = String::from_utf8(bytes).expect("urxvt is ASCII");
    assert_eq!(report, "\x1b[0;1;1M");
}

/// No tracking mode: the encoder returns None for every event kind.
#[test]
fn no_modes_means_no_bytes_for_any_mouse_event() {
    let grid = MouseGrid::new(10, 40).expect("grid");
    let modes = MouseModes::disabled();
    let press = PointerEvent::press(EncoderButton::Left, 0, 0, PointerModifiers::empty());
    let release = PointerEvent::release(EncoderButton::Left, 0, 0, PointerModifiers::empty());
    let motion = PointerEvent::move_to(Some(EncoderButton::Left), 1, 0, PointerModifiers::empty());
    let wheel = PointerEvent::wheel(WheelDirection::Up, 0, 0, PointerModifiers::empty());
    for (label, event) in [
        ("press", press),
        ("release", release),
        ("motion", motion),
        ("wheel", wheel),
    ] {
        assert_eq!(
            MouseEncoder::encode(event, modes, grid),
            None,
            "{label} must produce nothing with no modes"
        );
    }
}

// ── Selection preservation with tracking disabled ───────────────────

/// With tracking disabled, a left press sets up the same selection state
/// as before (drag_origin, selection, drag_mode). We verify the gate
/// (`mouse_reportable`) is false so the handler follows the selection
/// branch; the selection code itself is unchanged.
#[test]
fn disabled_tracking_routes_to_selection_not_pty() {
    let mut terminal = TerminalState::new(4, 8).expect("valid terminal");
    terminal.feed_bytes(b"hello");
    let mut app = NorenApp {
        terminal: Some(terminal),
        ..Default::default()
    };

    // Tracking disabled: reportable is false.
    assert!(!app.mouse_reportable());

    // Attempt a press; without a window grid_point_at returns None and
    // no selection starts — but crucially, held_mouse_button stays None
    // (the tracking branch was not entered).
    app.cursor_position = Some(PhysicalPosition::new(0.0, 0.0));
    app.handle_mouse_button(ElementState::Pressed, MouseButton::Left);
    assert_eq!(app.held_mouse_button, None, "tracking branch must not run");
}

/// With tracking enabled and Shift held, the bypass routes to selection,
/// not to the PTY — held_mouse_button must stay None.
#[test]
fn shift_bypass_with_tracking_routes_to_selection() {
    let mut terminal = TerminalState::new(4, 8).expect("valid terminal");
    terminal.feed_bytes(b"hello\x1b[?1000h");
    let mut app = NorenApp {
        terminal: Some(terminal),
        modifiers: Modifiers::empty().shift(),
        ..Default::default()
    };

    // Tracking is on but Shift bypasses it.
    assert!(!app.mouse_reportable());

    app.cursor_position = Some(PhysicalPosition::new(0.0, 0.0));
    app.handle_mouse_button(ElementState::Pressed, MouseButton::Left);
    assert_eq!(
        app.held_mouse_button, None,
        "Shift bypass must not enter the tracking branch"
    );
}

/// With tracking enabled (no Shift), a press enters the tracking branch
/// (`handle_tracked_mouse_button` runs) but — without a valid terminal
/// position (no window in this harness) — does NOT set
/// `held_mouse_button`. The button is recorded only when the press is
/// actually reported. `drag_origin` stays None because the selection
/// branch was not entered.
#[test]
fn tracking_enabled_press_enters_tracking_branch() {
    let mut terminal = TerminalState::new(4, 8).expect("valid terminal");
    terminal.feed_bytes(b"\x1b[?1000h");
    let mut app = NorenApp {
        terminal: Some(terminal),
        ..Default::default()
    };

    assert!(app.mouse_reportable());

    app.cursor_position = Some(PhysicalPosition::new(
        sidebar_pixel_width(app.geometry.cell_width()),
        0.0,
    ));
    app.handle_mouse_button(ElementState::Pressed, MouseButton::Left);
    assert_eq!(
        app.held_mouse_button, None,
        "press with no valid terminal position must not record the button"
    );
    // The selection branch was not taken.
    assert_eq!(app.drag_origin, None);

    // Release is a no-op on held_mouse_button (already None).
    app.handle_mouse_button(ElementState::Released, MouseButton::Left);
    assert_eq!(app.held_mouse_button, None);
}

/// A press that produces no report (position outside the terminal grid,
/// e.g. inside the sidebar) must not seed `held_mouse_button`. Otherwise a
/// subsequent drag into the terminal would emit a motion report with no
/// preceding press — outside the xterm model. Without a window,
/// `mouse_cell_at` returns None for every position, simulating a
/// non-reportable press; the held button must stay None so the motion
/// handler has no button to carry.
#[test]
fn sidebar_press_does_not_produce_orphan_motion_report() {
    let mut terminal = TerminalState::new(4, 8).expect("valid terminal");
    terminal.feed_bytes(b"\x1b[?1000;1002h");
    let mut app = NorenApp {
        terminal: Some(terminal),
        ..Default::default()
    };

    assert!(app.mouse_reportable());

    // Press at a sidebar x-coordinate — `mouse_cell_at` returns None.
    app.cursor_position = Some(PhysicalPosition::new(0.0, 0.0));
    app.handle_mouse_button(ElementState::Pressed, MouseButton::Left);
    assert_eq!(
        app.held_mouse_button, None,
        "sidebar press must not record the held button"
    );

    // Now move to a terminal position. With no window, `mouse_cell_at`
    // still returns None, so no motion report is encoded — but even if it
    // did resolve, the button field would be None because
    // `held_mouse_button` was never set, so no orphan drag report.
    app.handle_mouse_move(PhysicalPosition::new(
        sidebar_pixel_width(app.geometry.cell_width()),
        0.0,
    ));
    assert_eq!(
        app.held_mouse_button, None,
        "no orphan motion report: held button is still None"
    );
}

// ── mouse_grid() application path ───────────────────────────────────

/// `mouse_grid()` must pass `terminal.size()` to `MouseGrid::new` in the
/// correct order: `MouseGrid::new(cols, rows)` while `size()` returns
/// `(rows, cols)`. A transposition swaps the bounds the encoder clamps to.
/// The terminal here is deliberately non-square (4 rows × 8 cols) so a
/// swap cannot happen to match the intended dimensions.
#[test]
fn mouse_grid_dimensions_match_terminal_in_order() {
    let terminal = TerminalState::new(4, 8).expect("valid terminal");
    assert_eq!(
        terminal.size(),
        (4, 8),
        "fixture is non-square: 4 rows, 8 cols"
    );
    let app = NorenApp {
        terminal: Some(terminal),
        ..Default::default()
    };

    let grid = app.mouse_grid().expect("terminal present");

    // cols and rows must follow the terminal, not be swapped.
    assert_eq!(grid.cols(), 8, "cols must equal terminal cols");
    assert_eq!(grid.rows(), 4, "rows must equal terminal rows");
}

/// A press near the right edge of a wide, short terminal must report its
/// true column. This is the assertion that catches the shipped transpose:
/// with the bounds swapped, an 8-column terminal would clamp column 7 to
/// column 4 (the 4-row bound minus one), reporting `Cx=4` instead of
/// `Cx=8`. Drives the real application path — `NorenApp::mouse_grid` —
/// rather than constructing `MouseGrid` directly.
#[test]
fn mouse_grid_right_edge_click_reports_true_column() {
    let mut terminal = TerminalState::new(4, 8).expect("valid terminal");
    terminal.feed_bytes(b"\x1b[?1000;1006h");
    let app = NorenApp {
        terminal: Some(terminal),
        ..Default::default()
    };

    // Press the rightmost cell of row 0 (0-based col 7 of 8).
    let event = PointerEvent::press(EncoderButton::Left, 7, 0, PointerModifiers::empty());
    let bytes = app.encode_mouse(event).expect("tracked: must encode");
    let report = String::from_utf8(bytes).expect("SGR is ASCII");

    // 1-based column must be 8, not the clamped 4 a transposed grid yields.
    assert_eq!(
        report, "\x1b[<0;8;1M",
        "right-edge press must keep its column"
    );
}

// ── SSH host discovery and deferred selection (Milestone 4 step 2) ──

static SSH_CASE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Panic-safe SSH-config fixture rooted in a freshly-created private
/// directory. Atomic directory creation defeats predictable-name symlink
/// pre-placement, and config creation itself is exclusive and no-follow.
struct SshConfigFixture {
    root: PathBuf,
    path: PathBuf,
    extra_files: std::cell::RefCell<Vec<PathBuf>>,
}

impl SshConfigFixture {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;

        for _ in 0..128 {
            let unique = SSH_CASE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "noren-ssh-config-fixture-{}-{unique}",
                std::process::id()
            ));
            let mut builder = std::fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(&root) {
                Ok(()) => {
                    let path = root.join("config");
                    return Self {
                        root,
                        path,
                        extra_files: std::cell::RefCell::new(Vec::new()),
                    };
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create private SSH fixture directory: {error}"),
            }
        }
        panic!("could not allocate a private SSH fixture directory")
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn try_write_new(&self, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&self.path)?;
        file.write_all(bytes.as_ref())
    }

    fn write_new(&self, bytes: impl AsRef<[u8]>) {
        self.try_write_new(bytes)
            .expect("create exclusive SSH config fixture");
    }

    fn write_sibling_new(&self, name: &str, bytes: impl AsRef<[u8]>) {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        let relative = std::path::Path::new(name);
        let mut components = relative.components();
        assert!(
            matches!(components.next(), Some(std::path::Component::Normal(_)))
                && components.next().is_none(),
            "fixture sibling must be one normal path component"
        );
        let path = self.root.join(relative);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .expect("create exclusive SSH include fixture");
        self.extra_files.borrow_mut().push(path);
        file.write_all(bytes.as_ref())
            .expect("write SSH include fixture");
    }

    fn replace(&self, bytes: impl AsRef<[u8]>) {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&self.path)
            .expect("open private SSH config fixture without following links");
        file.write_all(bytes.as_ref())
            .expect("replace private SSH config fixture");
    }
}

impl Drop for SshConfigFixture {
    fn drop(&mut self) {
        // Avoid recursive deletion so a surprising replacement can never
        // make cleanup follow a tree.
        for path in self.extra_files.get_mut().drain(..) {
            let _ = std::fs::remove_file(path);
        }
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.root);
    }
}

#[test]
fn configured_ssh_hosts_appear_as_distinct_sidebar_rows() {
    let fixture = SshConfigFixture::new();
    fixture.write_new(
            b"Host build\n  HostName build.example\n  User alice\n  Port 2222\nHost db\n  HostName db.example\n",
        );

    let mut app = NorenApp::default();
    app.load_ssh_hosts_from(fixture.path());

    let rows = app.workspace.sidebar().rows();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].kind(), EntryKind::SshConnection);
    assert_eq!(rows[0].label(), "SSH-OFF build");
    assert_eq!(rows[0].detail(), Some("not connected"));
    assert_eq!(rows[1].kind(), EntryKind::SshConnection);
    assert_eq!(rows[1].label(), "SSH-OFF db");
    assert_eq!(
        app.workspace
            .ssh_hosts
            .iter()
            .map(|host| host.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            SessionKind::Ssh {
                target: "build".to_owned()
            },
            SessionKind::Ssh {
                target: "db".to_owned()
            },
        ]
    );
    assert!(
        app.workspace
            .ssh_hosts
            .iter()
            .all(|host| host.source_label == "config #0")
    );
    assert_eq!(
        app.ssh_diagnostic.as_deref(),
        Some("Noren SSH: partial literal aliases; select one for source")
    );
}

#[test]
fn readable_config_without_literal_aliases_reports_none_found() {
    let fixture = SshConfigFixture::new();
    fixture.write_new(b"# no literal targets\nHost *.example\n");
    let mut app = NorenApp::default();

    app.load_ssh_hosts_from(fixture.path());

    assert!(app.workspace.sidebar().rows().is_empty());
    assert_eq!(
        app.ssh_diagnostic.as_deref(),
        Some("Noren SSH: partial literal aliases; none found")
    );
}

#[test]
fn included_ssh_host_selection_shows_bounded_root_relative_provenance() {
    let fixture = SshConfigFixture::new();
    fixture.write_new(b"Include included.conf\nHost root-only\n");
    fixture.write_sibling_new("included.conf", b"Host remote\n");

    // The click attempts a real launch; the deterministic seam disables the
    // spawn so this test observes the provenance line plus the observed
    // launch phase without starting any process.
    let mut app = app_with_deterministic_ssh_seam();
    app.load_ssh_hosts_from(fixture.path());
    assert_eq!(app.workspace.ssh_hosts[0].source_label, "included.conf #1");
    app.cursor_position = Some(PhysicalPosition::new(5.0, 1.0));

    assert!(app.handle_sidebar_click_in_frame(
        ElementState::Pressed,
        MouseButton::Left,
        PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT),
    ));
    assert_eq!(
        app.ssh_selection_status.as_deref(),
        Some("SSH partial source #1 included.conf; launch failed")
    );
    assert!(
        !app.ssh_selection_status
            .as_deref()
            .expect("selection provenance")
            .contains(fixture.root.to_string_lossy().as_ref()),
        "the retained UI label must not expose the absolute config root"
    );
}

#[test]
fn ssh_sidebar_label_preserves_short_targets() {
    assert_eq!(ssh_sidebar_label("stage"), "SSH-OFF stage");
    assert_eq!(ssh_sidebar_label("abcdef"), "SSH-OFF abcdef");
    assert_eq!(ssh_sidebar_label("abcdefg"), "SSH-OFF abc...");
}

#[test]
fn ssh_sidebar_label_truncates_multibyte_targets_on_a_scalar_boundary() {
    let label = ssh_sidebar_label("東京大阪京都札幌仙台横浜");

    assert_eq!(label, "SSH-OFF 東京大...");
    assert_eq!(label.chars().count(), SSH_SIDEBAR_LABEL_CHARS);
}

#[test]
fn ssh_status_source_keeps_tag_first_and_bounds_unicode_path() {
    let label = format!("parts/{} #12", "東京大阪京都札幌仙台横浜".repeat(4));
    let status_source = ssh_status_source_label(&label);

    assert!(status_source.starts_with("#12 "));
    assert!(status_source.ends_with(SSH_SIDEBAR_TRUNCATION_MARKER));
    assert!(status_source.chars().count() <= SSH_STATUS_SOURCE_CHARS);
}

#[test]
fn every_rendered_ssh_prefix_encodes_disconnected_state_within_sixteen_columns() {
    let fixture = SshConfigFixture::new();
    fixture.write_new("Host db\nHost configured-host-with-long-alias\nHost 東京大阪京都札幌\n");
    let mut workspace = WorkspaceState::new();
    let config = SshConfig::read(fixture.path()).expect("bounded SSH fixture parses");
    workspace.load_ssh_config(&config);

    let lines = sidebar_text_lines(workspace.sidebar());
    assert_eq!(lines.len(), 3);
    for line in lines {
        let rendered_prefix: String = line.chars().take(renderer::SIDEBAR_COLS).collect();
        assert!(
            rendered_prefix.contains(SSH_SIDEBAR_LABEL_PREFIX),
            "the rendered prefix must identify SSH as offline"
        );
    }
}

#[test]
fn pending_marker_identifies_exact_target_despite_colliding_truncated_labels() {
    let fixture = SshConfigFixture::new();
    fixture.write_new(b"Host abcdef-first\nHost abcdef-second\n");
    let mut workspace = WorkspaceState::new();
    let local = workspace.create_session(SessionKind::Local);
    workspace
        .select_session(local)
        .expect("created local session is selectable");
    workspace.observe_session(local, SessionStatus::Running);
    let config = SshConfig::read(fixture.path()).expect("bounded SSH fixture parses");
    workspace.load_ssh_config(&config);

    assert_eq!(workspace.sidebar().rows()[1].label(), "SSH-OFF abc...");
    assert_eq!(workspace.sidebar().rows()[2].label(), "SSH-OFF abc...");
    assert!(workspace.select_ssh_sidebar_row(2));
    assert_eq!(workspace.selected_ssh_target(), Some("abcdef-second"));

    let rows = workspace.sidebar().rows();
    assert!(!rows[0].is_selected(), "pending SSH supersedes live marker");
    assert!(
        !rows[1].is_selected(),
        "colliding first label stays unmarked"
    );
    assert!(rows[2].is_selected(), "the exact pending target is marked");
    assert_eq!(workspace.sidebar().selected_row_count(), 1);
    assert_eq!(
        workspace.sidebar().viewport().map(|view| view.session_id()),
        Some(local),
        "pending display state must not change the actual local viewport"
    );

    let lines = sidebar_text_lines(workspace.sidebar());
    assert!(lines[1].starts_with(' '));
    assert!(lines[2].starts_with('>'));
}

#[test]
fn exclusive_ssh_fixture_creation_rejects_a_preexisting_symlink() {
    let fixture = SshConfigFixture::new();
    std::os::unix::fs::symlink(&fixture.root, fixture.path())
        .expect("place synthetic fixture symlink");

    let result = fixture.try_write_new(b"Host must-not-be-written\n");

    assert!(
        result.is_err(),
        "create_new/no-follow must reject the symlink"
    );
    assert!(fixture.root.is_dir(), "symlink target remains a directory");
    assert!(
        std::fs::symlink_metadata(fixture.path())
            .expect("fixture link still exists")
            .file_type()
            .is_symlink()
    );
}

#[test]
fn near_one_mib_ssh_alias_keeps_full_identity_and_bounded_display_text() {
    let ssh_fixture = SshConfigFixture::new();
    let target = "a".repeat(1024 * 1024 - "Host \n".len());
    let mut config_text = String::with_capacity(1024 * 1024);
    config_text.push_str("Host ");
    config_text.push_str(&target);
    config_text.push('\n');
    assert_eq!(config_text.len(), 1024 * 1024);
    ssh_fixture.write_new(config_text.as_bytes());

    let mut workspace = WorkspaceState::new();
    let config = SshConfig::read(ssh_fixture.path()).expect("near-one-MiB SSH fixture parses");
    assert_eq!(workspace.load_ssh_config(&config), 0);

    let SessionKind::Ssh { target: cached } = &workspace.ssh_hosts[0].kind else {
        panic!("configured target remains an SSH identity");
    };
    assert!(
        cached == &target,
        "the full connection target remains intact"
    );

    let row = &workspace.sidebar().rows()[0];
    assert_eq!(row.label(), "SSH-OFF aaa...");
    assert_eq!(row.label().chars().count(), SSH_SIDEBAR_LABEL_CHARS);

    let redraw_lines = sidebar_text_lines(workspace.sidebar());
    assert_eq!(redraw_lines.len(), 1);
    assert_eq!(
        redraw_lines[0].chars().count(),
        SIDEBAR_ROW_PREFIX_CHARS + SSH_SIDEBAR_LABEL_CHARS + 1 + SSH_SIDEBAR_DETAIL.chars().count(),
        "redraw text stays bounded independently of target length"
    );

    assert!(workspace.select_ssh_sidebar_row(0));
    assert!(
        workspace.selected_ssh_target() == Some(target.as_str()),
        "pending selection retains the full connection target"
    );
}

#[test]
fn missing_ssh_config_is_silent_and_adds_no_rows() {
    let fixture = SshConfigFixture::new();
    let mut app = NorenApp::default();
    app.load_ssh_hosts_from(fixture.path());

    assert!(app.workspace.sidebar().rows().is_empty());
    assert!(app.ssh_diagnostic.is_none());
}

#[test]
fn malformed_ssh_config_starts_with_content_free_diagnostic() {
    let fixture = SshConfigFixture::new();
    let secret = "DO_NOT_LEAK_ssh_config_fixture";
    fixture.write_new(format!("Host broken\nPort nope # {secret}\n"));

    let mut app = NorenApp::default();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app.load_ssh_hosts_from(fixture.path());
    }));
    assert!(result.is_ok(), "malformed config must not panic");
    assert!(app.workspace.sidebar().rows().is_empty());
    let diagnostic = app.ssh_diagnostic.as_deref().expect("diagnostic surfaced");
    assert!(diagnostic.contains("SSH configuration error"));
    assert!(!diagnostic.contains(secret));
    assert!(!diagnostic.contains("nope"));
}

#[test]
fn post_startup_ssh_diagnostic_status_row_agrees_with_hit_testing_and_yields_to_runtime() {
    let fixture = SshConfigFixture::new();
    let secret = "DO_NOT_LEAK_post_startup_ssh_fixture";
    fixture.write_new(format!("Host broken\nPort nope # {secret}\n"));

    let mut app = NorenApp::default();
    app.load_ssh_hosts_from(fixture.path());
    let diagnostic = app
        .ssh_diagnostic
        .as_deref()
        .expect("diagnostic surfaced")
        .to_owned();
    assert!(!diagnostic.contains("nope"));
    assert!(!diagnostic.contains(secret));

    let mut terminal = TerminalState::new(29, 8).expect("valid terminal");
    // Put a marker in the last terminal row so all 29 rows are part of the
    // displayed snapshot. `record_pty_started` exercises the production
    // lifecycle transition without launching a process or opening SSH.
    terminal.feed_bytes(b"\x1b[29;8HZ");
    app.terminal = Some(terminal);
    assert!(app.pty.is_none());
    app.record_pty_started();
    assert!(app.pty.is_none(), "the lifecycle seam must not start a PTY");

    assert!(!app.show_status, "successful startup hides the ready line");
    let source = app.status_row();
    assert_eq!(source, StatusRowSource::SshDiagnostic);
    assert_eq!(
        source.text(
            app.status,
            app.ssh_selection_status.as_deref(),
            app.worktree_diagnostic.as_deref(),
            app.agent_diagnostic.as_deref(),
            app.ssh_diagnostic.as_deref(),
        ),
        diagnostic
    );

    let content_rows = app
        .terminal
        .as_ref()
        .expect("terminal present")
        .screen()
        .display_row_count();
    assert_eq!(content_rows, 29);
    let metrics = app.geometry.cell_metrics();
    let frame_width = (renderer::SIDEBAR_COLS as u32 + 8) * metrics.width();
    let frame_height = 30 * metrics.height();
    let frame_size = PhysicalSize::new(frame_width, frame_height);
    let layout = renderer::FrameRowLayout::new(
        frame_height,
        metrics,
        content_rows,
        NorenApp::status_row_present(30),
    )
    .expect("non-zero frame");
    assert_eq!(NorenApp::content_terminal_rows(30), 29);
    assert_eq!(layout.row_at(0), Some(renderer::FrameRow::Terminal(0)));
    assert_eq!(layout.row_at(28), Some(renderer::FrameRow::Terminal(28)));
    assert_eq!(layout.row_at(29), Some(renderer::FrameRow::Status));
    let snapshot = app.terminal.as_ref().expect("terminal present").snapshot();
    let status = source.text(
        app.status,
        app.ssh_selection_status.as_deref(),
        app.worktree_diagnostic.as_deref(),
        app.agent_diagnostic.as_deref(),
        app.ssh_diagnostic.as_deref(),
    );
    let vertices = renderer::glyph_vertices_for(
        renderer::Target::new(&app.theme, frame_width, frame_height, metrics),
        Some(&snapshot),
        Some(&[]),
        Some(status),
    );
    assert!(
        !vertices.is_empty(),
        "terminal content and the diagnostic must render"
    );
    let contains = |row: usize, col: usize| {
        let left = col as f32 * metrics.width() as f32 / frame_width as f32 * 2.0 - 1.0;
        let right = (col as f32 + 1.0) * metrics.width() as f32 / frame_width as f32 * 2.0 - 1.0;
        let top = 1.0 - row as f32 * metrics.height() as f32 / frame_height as f32 * 2.0;
        let bottom = 1.0 - (row as f32 + 1.0) * metrics.height() as f32 / frame_height as f32 * 2.0;
        vertices.iter().any(|vertex| {
            vertex.position[0] >= left
                && vertex.position[0] < right
                && vertex.position[1] <= top
                && vertex.position[1] > bottom
        })
    };
    assert!(
        contains(28, renderer::SIDEBAR_COLS + 7),
        "terminal line 28's marker must remain in frame row 28"
    );
    assert!(
        contains(29, renderer::SIDEBAR_COLS),
        "the retained diagnostic's first glyph must render in the last frame row"
    );

    let terminal_x = sidebar_pixel_width(metrics.width());
    assert_eq!(
        app.grid_point_in_frame(PhysicalPosition::new(terminal_x, 1.0), frame_size),
        Some(GridPoint::new(0, 0)),
        "frame row 0 maps to the first terminal line"
    );
    assert_eq!(
        app.mouse_cell_in_frame(PhysicalPosition::new(terminal_x, 1.0), frame_size),
        Some((0, 0)),
        "mouse mapping must share the first terminal line"
    );
    assert_eq!(
        app.grid_point_in_frame(
            PhysicalPosition::new(terminal_x, f64::from(28 * metrics.height()) + 1.0),
            frame_size,
        ),
        Some(GridPoint::new(28, 0)),
        "frame row 28 maps to the last terminal line"
    );
    assert_eq!(
        app.mouse_cell_in_frame(
            PhysicalPosition::new(terminal_x, f64::from(28 * metrics.height()) + 1.0),
            frame_size,
        ),
        Some((0, 28)),
        "mouse mapping reaches the same last terminal line"
    );
    assert_eq!(
        app.grid_point_in_frame(
            PhysicalPosition::new(terminal_x, f64::from(29 * metrics.height()) + 1.0,),
            frame_size,
        ),
        None,
        "the last frame row is diagnostic chrome, not selectable"
    );
    assert_eq!(
        app.mouse_cell_in_frame(
            PhysicalPosition::new(terminal_x, f64::from(29 * metrics.height()) + 1.0),
            frame_size,
        ),
        None,
        "the diagnostic row is not mouse-reportable"
    );

    app.finish_pty("Noren PTY operation failed");
    let source = app.status_row();
    assert_eq!(source, StatusRowSource::Runtime);
    assert_eq!(
        source.text(
            app.status,
            app.ssh_selection_status.as_deref(),
            app.worktree_diagnostic.as_deref(),
            app.agent_diagnostic.as_deref(),
            app.ssh_diagnostic.as_deref(),
        ),
        "Noren PTY operation failed",
        "a retained startup diagnostic must not mask a newer runtime status"
    );
    assert_eq!(app.ssh_diagnostic.as_deref(), Some(diagnostic.as_str()));

    fixture.replace(b"Host recovered\n");
    app.load_ssh_hosts_from(fixture.path());
    let discovery_notice = app
        .ssh_diagnostic
        .as_deref()
        .expect("a readable config keeps the partial-discovery notice");
    assert!(
        discovery_notice.contains("partial literal aliases"),
        "a clean application replaces the error with an honest scope notice"
    );
    assert!(!discovery_notice.contains("configuration error"));
}

#[test]
fn ssh_rows_stay_distinguishable_from_local_rows() {
    let fixture = SshConfigFixture::new();
    fixture.write_new(b"Host staging\n");

    let mut app = NorenApp::default();
    app.load_ssh_hosts_from(fixture.path());
    app.workspace.create_session(SessionKind::Local);

    let rows = app.workspace.sidebar().rows();
    assert_eq!(rows[0].kind(), EntryKind::Session);
    assert_eq!(rows[0].detail(), Some("local · starting"));
    assert_eq!(rows[1].kind(), EntryKind::SshConnection);
    assert_eq!(rows[1].label(), "SSH-OFF sta...");
    assert_eq!(rows[1].detail(), Some("not connected"));
}

#[test]
fn selecting_an_ssh_row_keeps_a_pending_choice_and_never_a_registry_connection() {
    let fixture = SshConfigFixture::new();
    fixture.write_new(b"Host staging\n");

    // Deterministic seam: the click path attempts the launch, and the
    // disabled spawn reports the attempt's failure without any process.
    let mut app = app_with_deterministic_ssh_seam();
    app.load_ssh_hosts_from(fixture.path());
    app.workspace.create_session(SessionKind::Local);
    app.cursor_position = Some(PhysicalPosition::new(5.0, 25.0));

    assert!(app.handle_sidebar_click_in_frame(
        ElementState::Pressed,
        MouseButton::Left,
        PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT),
    ));
    assert_eq!(app.workspace.selected_ssh_target(), Some("staging"));
    assert_eq!(app.workspace.registry().selected(), None);
    assert_eq!(app.workspace.registry().len(), 1);
    assert!(
        app.pty.is_none(),
        "the deterministic seam must leave no PTY behind"
    );
    app.show_status = false;
    let source = app.status_row();
    assert_eq!(source, StatusRowSource::SshSelection);
    assert_eq!(
        source.text(
            app.status,
            app.ssh_selection_status.as_deref(),
            app.worktree_diagnostic.as_deref(),
            app.agent_diagnostic.as_deref(),
            app.ssh_diagnostic.as_deref(),
        ),
        "SSH partial source #0 config; launch failed"
    );
    assert!(
        app.workspace.sidebar().viewport().is_none(),
        "SSH selection must not claim a connected viewport"
    );
}

#[test]
fn ssh_selection_does_not_hide_a_runtime_failure() {
    let fixture = SshConfigFixture::new();
    fixture.write_new(b"Host staging\n");
    let mut app = app_with_deterministic_ssh_seam();
    app.load_ssh_hosts_from(fixture.path());
    app.finish_pty("Noren PTY operation failed");
    app.cursor_position = Some(PhysicalPosition::new(5.0, 1.0));

    assert!(app.handle_sidebar_click_in_frame(
        ElementState::Pressed,
        MouseButton::Left,
        PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT),
    ));

    assert!(app.show_status);
    assert_eq!(app.status_row(), StatusRowSource::Runtime);
    assert_eq!(app.workspace.selected_ssh_target(), Some("staging"));
    assert!(app.ssh_selection_status.is_some());
}

#[test]
fn partial_undrawn_sidebar_row_cannot_select_a_hidden_ssh_entry() {
    let fixture = SshConfigFixture::new();
    fixture.write_new(b"Host visible\nHost hidden\n");

    let mut app = app_with_deterministic_ssh_seam();
    app.load_ssh_hosts_from(fixture.path());
    assert_eq!(app.workspace.sidebar().rows().len(), 2);

    let cell_height = app.geometry.cell_height();
    let frame_size = PhysicalSize::new(
        (renderer::SIDEBAR_COLS as u32) * app.geometry.cell_width(),
        cell_height + cell_height / 2,
    );
    let partial_row = PhysicalPosition::new(5.0, f64::from(cell_height) + 1.0);
    assert_eq!(
        app.sidebar_row_index(partial_row, frame_size),
        None,
        "the partial second cell row is not among the renderer's fully drawable rows"
    );
    app.cursor_position = Some(partial_row);
    assert!(!app.handle_sidebar_click_in_frame(
        ElementState::Pressed,
        MouseButton::Left,
        frame_size,
    ));
    assert_eq!(
        app.workspace.selected_ssh_target(),
        None,
        "the hidden SSH entry must remain unselected"
    );

    let outside_window = PhysicalPosition::new(5.0, f64::from(frame_size.height));
    assert_eq!(
        app.sidebar_row_index(outside_window, frame_size),
        None,
        "the bottom window edge is exclusive"
    );

    let visible_row = PhysicalPosition::new(5.0, 1.0);
    assert_eq!(app.sidebar_row_index(visible_row, frame_size), Some(0));
    app.cursor_position = Some(visible_row);
    assert!(app.handle_sidebar_click_in_frame(
        ElementState::Pressed,
        MouseButton::Left,
        frame_size,
    ));
    assert_eq!(app.workspace.selected_ssh_target(), Some("visible"));
    assert!(
        app.pty.is_none(),
        "SSH selection must remain non-connecting"
    );
}

#[test]
fn sidebar_scroll_reveals_and_selects_ssh_without_terminal_mouse_output() {
    let fixture = SshConfigFixture::new();
    fixture.write_new(b"Host alpha\nHost beta\nHost gamma\n");
    let mut app = app_with_deterministic_ssh_seam();
    for _ in 0..3 {
        let _ = app.workspace.registry.restore(SessionKind::Local);
    }
    app.workspace.create_session(SessionKind::Local);
    app.workspace.rebuild_sidebar();
    app.load_ssh_hosts_from(fixture.path());
    app.terminal = Some(TerminalState::new(2, 8).expect("valid terminal"));
    app.apply_pty_output(b"\x1b[?1000;1006h");

    let metrics = app.geometry.cell_metrics();
    let frame_size = PhysicalSize::new(
        renderer::SIDEBAR_COLS as u32 * metrics.width(),
        2 * metrics.height(),
    );
    let initial = visible_sidebar_text_lines(app.workspace.sidebar(), 0, 2);
    assert!(
        initial
            .iter()
            .all(|line| !line.contains(SSH_SIDEBAR_LABEL_PREFIX)),
        "restored/local rows initially hide SSH rows"
    );

    app.cursor_position = Some(PhysicalPosition::new(1.0, 1.0));
    app.redraw_needed = false;
    assert!(
        app.handle_sidebar_wheel_in_frame(MouseScrollDelta::LineDelta(0.0, -4.0), frame_size,),
        "sidebar wheel is consumed before tracked-terminal reporting"
    );
    assert_eq!(app.sidebar_scroll_offset, 4);
    assert!(app.redraw_needed);
    assert_eq!(
        app.mouse_cell_in_frame(PhysicalPosition::new(1.0, 1.0), frame_size),
        None,
        "sidebar coordinates cannot become PTY mouse coordinates"
    );
    assert!(app.pty.is_none(), "the local scroll route opens no PTY");

    let visible = visible_sidebar_text_lines(app.workspace.sidebar(), app.sidebar_scroll_offset, 2);
    assert!(visible[0].contains("SSH-OFF alpha"));
    assert!(app.handle_sidebar_click_in_frame(
        ElementState::Pressed,
        MouseButton::Left,
        frame_size,
    ));
    assert_eq!(app.workspace.selected_ssh_target(), Some("alpha"));
    assert!(
        visible_sidebar_text_lines(app.workspace.sidebar(), app.sidebar_scroll_offset, 2,)[0]
            .starts_with('>')
    );

    let tall_frame = PhysicalSize::new(frame_size.width, 7 * metrics.height());
    app.handle_resize(tall_frame);
    assert_eq!(
        app.sidebar_scroll_offset, 0,
        "a taller frame clamps the obsolete scroll offset"
    );
}

#[test]
fn active_local_sidebar_press_selects_it_and_clears_pending_ssh() {
    let mut app = NorenApp::default();
    let local = app.workspace.create_session(SessionKind::Local);
    app.workspace
        .select_session(local)
        .expect("created local session is selectable");
    app.active_session = Some(local);
    app.workspace.ssh_hosts.push(ConfiguredSshHost {
        kind: SessionKind::Ssh {
            target: "staging".to_owned(),
        },
        source_label: "inline #0".to_owned(),
    });
    app.workspace.rebuild_sidebar();
    assert!(app.workspace.select_ssh_sidebar_row(1));
    app.ssh_selection_status = Some("SSH partial source #0 inline; offline".to_owned());
    assert_eq!(app.workspace.selected_ssh_target(), Some("staging"));
    app.cursor_position = Some(PhysicalPosition::new(5.0, 1.0));

    assert!(
        app.handle_sidebar_click_in_frame(
            ElementState::Pressed,
            MouseButton::Left,
            PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT),
        ),
        "a visible local row is consumed by the sidebar"
    );
    assert_eq!(app.workspace.selected_ssh_target(), None);
    assert!(app.ssh_selection_status.is_none());
    assert_eq!(app.workspace.registry().selected(), Some(local));
    assert!(app.workspace.sidebar().rows()[0].is_selected());
    assert_eq!(app.workspace.sidebar().selected_row_count(), 1);
    assert_eq!(
        app.workspace
            .sidebar()
            .viewport()
            .map(|view| view.session_id()),
        Some(local)
    );
}

#[test]
fn inactive_local_sidebar_press_is_consumed_without_moving_the_pty_owner() {
    let mut app = NorenApp::default();
    let inactive = app.workspace.create_session(SessionKind::Local);
    let active = app.workspace.create_session(SessionKind::Local);
    app.workspace
        .select_session(active)
        .expect("active session is selectable");
    app.active_session = Some(active);
    app.cursor_position = Some(PhysicalPosition::new(5.0, 1.0));

    assert!(app.handle_sidebar_click_in_frame(
        ElementState::Pressed,
        MouseButton::Left,
        PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT),
    ));

    assert_eq!(app.workspace.local_sidebar_session(0), Some(inactive));
    // Input ownership stays with the active session, but the click SELECTS
    // the clicked row — the close command operates on the selection.
    assert_eq!(app.workspace.registry().selected(), Some(inactive));
    assert_eq!(app.active_session, Some(active));
    assert!(app.workspace.sidebar().rows()[0].is_selected());
}

#[test]
fn restored_local_sidebar_press_cannot_claim_live_input_ownership() {
    let mut app = NorenApp::default();
    let restored = app.workspace.registry.restore(SessionKind::Local);
    let active = app.workspace.create_session(SessionKind::Local);
    app.workspace
        .select_session(active)
        .expect("active session is selectable");
    app.workspace.rebuild_sidebar();
    app.active_session = Some(active);
    app.cursor_position = Some(PhysicalPosition::new(5.0, 1.0));

    assert!(app.handle_sidebar_click_in_frame(
        ElementState::Pressed,
        MouseButton::Left,
        PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT),
    ));

    assert_eq!(app.workspace.local_sidebar_session(0), Some(restored));
    // Input ownership stays with the active session; the click selects the
    // restored row so a close targets what the user pointed at.
    assert_eq!(app.workspace.registry().selected(), Some(restored));
    assert_eq!(app.active_session, Some(active));
    assert!(app.workspace.sidebar().rows()[0].is_selected());
}

#[test]
fn rebuild_sidebar_skips_non_ssh_host_facts_without_panicking() {
    let mut workspace = WorkspaceState::default();
    workspace.ssh_hosts.push(ConfiguredSshHost {
        kind: SessionKind::Local,
        source_label: "inline #0".to_owned(),
    });

    workspace.rebuild_sidebar();

    assert!(workspace.sidebar().rows().is_empty());
}

#[test]
fn many_ssh_hosts_are_bounded_and_report_the_omitted_count() {
    let fixture = SshConfigFixture::new();
    let config: String = (0..30)
        .map(|index| format!("Host configured-host-{index:02}-with-long-alias\n"))
        .collect();
    fixture.write_new(config);

    let mut app = NorenApp::default();
    app.load_ssh_hosts_from(fixture.path());

    let rows = app.workspace.sidebar().rows();
    assert_eq!(rows.len(), MAX_SSH_SIDEBAR_HOSTS);
    assert_eq!(
        app.workspace.ssh_hosts.first().map(|host| &host.kind),
        Some(&SessionKind::Ssh {
            target: "configured-host-00-with-long-alias".to_owned(),
        })
    );
    assert_eq!(
        app.workspace.ssh_hosts.last().map(|host| &host.kind),
        Some(&SessionKind::Ssh {
            target: "configured-host-23-with-long-alias".to_owned(),
        })
    );
    assert!(app.workspace.ssh_hosts.iter().all(|host| {
        host.kind
            != SessionKind::Ssh {
                target: "configured-host-24-with-long-alias".to_owned(),
            }
    }));
    assert!(rows.iter().all(|row| {
        row.label().chars().count() == SSH_SIDEBAR_LABEL_CHARS
            && row.label().ends_with(SSH_SIDEBAR_TRUNCATION_MARKER)
    }));
    let redraw_lines = sidebar_text_lines(app.workspace.sidebar());
    assert_eq!(redraw_lines.len(), MAX_SSH_SIDEBAR_HOSTS);
    assert!(redraw_lines.iter().all(|line| {
        line.chars().count()
            == SIDEBAR_ROW_PREFIX_CHARS
                + SSH_SIDEBAR_LABEL_CHARS
                + 1
                + SSH_SIDEBAR_DETAIL.chars().count()
    }));
    assert_eq!(app.workspace.ssh_hosts_omitted(), 6);
    assert!(
        app.ssh_diagnostic
            .as_deref()
            .is_some_and(|line| line.contains("showing first 24; 6 omitted"))
    );
}

#[test]
fn ssh_host_cap_is_exact_at_twenty_four() {
    for count in [23_usize, 24, 25] {
        let fixture = SshConfigFixture::new();
        let config: String = (0..count)
            .map(|index| format!("Host host-{index:02}\n"))
            .collect();
        fixture.write_new(config);
        let mut app = NorenApp::default();

        app.load_ssh_hosts_from(fixture.path());

        let retained = count.min(MAX_SSH_SIDEBAR_HOSTS);
        assert_eq!(app.workspace.ssh_hosts.len(), retained);
        assert_eq!(app.workspace.sidebar().rows().len(), retained);
        assert_eq!(
            app.workspace.ssh_hosts_omitted(),
            count.saturating_sub(MAX_SSH_SIDEBAR_HOSTS)
        );
        let last = app
            .workspace
            .ssh_hosts
            .last()
            .and_then(|host| match &host.kind {
                SessionKind::Ssh { target } => Some(target.as_str()),
                _ => None,
            });
        let expected_last = format!("host-{:02}", retained - 1);
        assert_eq!(last, Some(expected_last.as_str()));
        let notice = app.ssh_diagnostic.as_deref().expect("bounded notice");
        if count <= MAX_SSH_SIDEBAR_HOSTS {
            assert!(notice.contains("select one for source"));
            assert!(!notice.contains("showing first"));
        } else {
            assert!(notice.contains("showing first 24; 1 omitted"));
            assert!(app.workspace.ssh_hosts.iter().all(|host| {
                !matches!(&host.kind, SessionKind::Ssh { target } if target == "host-24")
            }));
        }
    }
}

// ── Sidebar state persistence (Milestone 3 final piece) ────────────
//
// These tests exercise the wiring of `save`/`load` into the application
// lifecycle through `WorkspaceState`. The persistence format itself is
// exhaustively tested in `tests/session_persistence.rs`; these tests
// verify WHEN save/load is called, not HOW the format works.

/// Per-test uniqueness: tests run concurrently and share the temp dir.
static PERSIST_CASE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn temp_state_path() -> PathBuf {
    let unique = PERSIST_CASE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "noren-sidebar-wire-test-{}-{unique}.toml",
        std::process::id()
    ));
    path
}

fn cleanup_state_file(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

/// Required: state saved through the workspace and then loaded round-trips
/// through the real file path, preserving every entry kind and the
/// positional selection.
#[test]
fn saved_state_round_trips_through_the_real_file_path() {
    let path = temp_state_path();
    let mut state = WorkspaceState::with_state_path(Some(path.clone()));
    let _local = state.create_session(SessionKind::Local);
    let _project = state.create_session(SessionKind::Project {
        root: PathBuf::from("/srv/noren"),
    });
    let _ssh = state.create_session(SessionKind::Ssh {
        target: "ops@bastion".to_owned(),
    });
    state
        .select_session(state.registry().sessions()[1].id())
        .expect("project session is live");

    let mut restored = WorkspaceState::with_state_path(Some(path.clone()));
    restored.restore().expect("state loads");

    assert_eq!(restored.registry().len(), 3);
    let kinds: Vec<SessionKind> = restored
        .registry()
        .sessions()
        .iter()
        .map(|d| d.kind().clone())
        .collect();
    assert_eq!(
        kinds,
        vec![
            SessionKind::Local,
            SessionKind::Project {
                root: PathBuf::from("/srv/noren")
            },
            SessionKind::Ssh {
                target: "ops@bastion".to_owned()
            },
        ]
    );
    let selected = restored.registry().selected().expect("selection restored");
    assert_eq!(
        restored
            .registry()
            .get(selected)
            .expect("selection resolves")
            .kind(),
        &SessionKind::Project {
            root: PathBuf::from("/srv/noren")
        },
    );
    cleanup_state_file(&path);
}

/// Required: a corrupt file leaves the app startable with an empty sidebar
/// and an error surfaced, not a panic.
#[test]
fn corrupt_state_file_surfaces_error_without_panicking() {
    let path = temp_state_path();
    std::fs::write(&path, b"this is not valid toml {{{{").expect("write corrupt fixture");

    let mut state = WorkspaceState::with_state_path(Some(path.clone()));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| state.restore()));
    let loaded = result.expect("restore must never panic");
    assert!(loaded.is_err(), "corrupt file must produce an error");
    assert!(
        state.registry().is_empty(),
        "corrupt file leaves empty registry"
    );
    assert!(
        state.sidebar().is_empty(),
        "corrupt file leaves empty sidebar"
    );
    assert!(
        state.persistence_unverified(),
        "the restore error is the current unsafe persistence outcome"
    );
    cleanup_state_file(&path);
}

/// Required: a missing file is not an error — first run must be silent.
#[test]
fn missing_state_file_is_silent_first_run() {
    let path = temp_state_path();
    assert!(!path.exists(), "fixture path must not exist yet");

    let mut state = WorkspaceState::with_state_path(Some(path.clone()));
    assert!(state.restore().is_ok(), "missing file must not error");
    assert!(state.registry().is_empty());
    assert!(state.sidebar().is_empty());
    assert!(!path.exists(), "restore must not create a file");
}

/// Required: creating a session then restarting shows it in the sidebar.
#[test]
fn creating_a_session_then_restarting_shows_it_in_the_sidebar() {
    let path = temp_state_path();

    let mut run1 = WorkspaceState::with_state_path(Some(path.clone()));
    run1.create_session(SessionKind::Local);
    run1.create_session(SessionKind::Ssh {
        target: "web1".to_owned(),
    });

    let mut run2 = WorkspaceState::with_state_path(Some(path.clone()));
    run2.restore().expect("state loads on restart");

    assert_eq!(
        run2.sidebar().rows().len(),
        2,
        "both sessions survive the restart"
    );
    assert!(!run2.sidebar().is_empty());
    cleanup_state_file(&path);
}

/// Required: a restored session is distinct from a shell that is starting.
/// Mutation check for Issue #110: changing `load_snapshot`'s restoration
/// path back to `create` makes the status and sidebar assertions fail.
#[test]
fn restored_sessions_are_restored_not_starting_or_running() {
    let path = temp_state_path();
    let mut state = WorkspaceState::with_state_path(Some(path.clone()));
    let id = state.create_session(SessionKind::Local);
    state.observe_session(id, SessionStatus::Running);

    let mut restored = WorkspaceState::with_state_path(Some(path.clone()));
    restored.restore().expect("state loads");

    for descriptor in restored.registry().sessions() {
        assert_eq!(
            descriptor.status(),
            &SessionStatus::Restored,
            "restored session must identify its no-process state"
        );
    }
    let detail = restored
        .sidebar()
        .rows()
        .first()
        .and_then(|r| r.detail())
        .unwrap_or_default();
    assert!(
        detail.contains("restored") && detail.contains("not running"),
        "detail identifies a restored, non-running session: {detail}"
    );
    assert!(
        !detail.ends_with("· running"),
        "detail must not claim running: {detail}"
    );
    assert!(
        restored.sidebar().viewport().is_none(),
        "selecting a restored session must not imply an attachment"
    );
    cleanup_state_file(&path);
}

/// Mutation check 1: create persists immediately. If the `persist` call in
/// `create_session` is removed, this test fails — the file would not exist.
#[test]
fn create_session_persists_immediately() {
    let path = temp_state_path();
    let mut state = WorkspaceState::with_state_path(Some(path.clone()));
    state.create_session(SessionKind::Local);

    assert!(path.exists(), "create must persist to the state file");
    let mut loaded = SessionRegistry::new();
    load(&path, &mut loaded).expect("file loads");
    assert_eq!(loaded.len(), 1, "one session was persisted");
    cleanup_state_file(&path);
}

/// Mutation check 2: close persists the removal. If the `persist` call in
/// `close_session` is removed, this test fails — the file would still show
/// two sessions.
#[test]
fn close_session_persists_the_removal() {
    let path = temp_state_path();
    let mut state = WorkspaceState::with_state_path(Some(path.clone()));
    let first = state.create_session(SessionKind::Local);
    let _second = state.create_session(SessionKind::Local);
    state.close_session(first).expect("first is live");

    let mut loaded = SessionRegistry::new();
    load(&path, &mut loaded).expect("file loads");
    assert_eq!(loaded.len(), 1, "close must persist: one session remains");
    cleanup_state_file(&path);
}

/// Mutation check 3 (save skipped): observe does NOT rewrite the state
/// file. Status is a runtime observation, not a persistable structural
/// change. If a `persist` call were incorrectly added to `observe_session`,
/// this test fails because the file's modification time would advance.
#[test]
fn observe_session_does_not_rewrite_the_state_file() {
    let path = temp_state_path();
    let mut state = WorkspaceState::with_state_path(Some(path.clone()));
    let id = state.create_session(SessionKind::Local);

    let before = std::fs::read(&path).expect("file exists after create");
    let mtime_before = std::fs::metadata(&path)
        .expect("file exists")
        .modified()
        .expect("modification time available");
    // Sleep past the filesystem's timestamp granularity so a rewrite would
    // be detectable as a changed mtime.
    std::thread::sleep(std::time::Duration::from_millis(20));

    state.observe_session(id, SessionStatus::Running);
    state.observe_session(id, SessionStatus::Exited { code: Some(0) });

    let after = std::fs::read(&path).expect("file still exists");
    let mtime_after = std::fs::metadata(&path)
        .expect("file exists")
        .modified()
        .expect("modification time available");
    assert_eq!(before, after, "observe must not change file content");
    assert_eq!(
        mtime_before, mtime_after,
        "observe must not rewrite the state file (mtime changed)"
    );

    let text = String::from_utf8(after).expect("state is UTF-8");
    assert!(
        !text.contains("running") && !text.contains("exited"),
        "status must not appear in the state file: {text}"
    );
    cleanup_state_file(&path);
}

/// Select persists the new selection positionally. If the `persist` call in
/// `select_session` is removed, the restored selection does not match.
#[test]
fn select_session_persists_the_selection() {
    let path = temp_state_path();
    let mut state = WorkspaceState::with_state_path(Some(path.clone()));
    let _first = state.create_session(SessionKind::Local);
    let second = state.create_session(SessionKind::Ssh {
        target: "host".to_owned(),
    });
    state.select_session(second).expect("second is live");

    let mut restored = WorkspaceState::with_state_path(Some(path.clone()));
    restored.restore().expect("state loads");

    let selected = restored.registry().selected().expect("selection persisted");
    assert_eq!(
        restored.registry().get(selected).expect("resolves").kind(),
        &SessionKind::Ssh {
            target: "host".to_owned()
        },
    );
    cleanup_state_file(&path);
}

// ── The quit path ──────────────────────────────────────────────────
//
// The tests above drive `WorkspaceState` directly and so never traverse
// what the app actually does on exit. These go through `NorenApp::teardown`
// — the whole of `NorenApp::close` except `event_loop.exit()` — and then
// read the file back from disk, which is what the next launch would see.

/// An app with `path` wired up as its state file, as `load_sidebar_state`
/// wires it in `main`.
fn app_with_state_path(path: &std::path::Path) -> NorenApp {
    let mut app = NorenApp::new(AppConfig::default());
    app.load_sidebar_state(Some(path.to_path_buf()));
    app
}

/// What the next launch would show: load the file into a fresh workspace.
fn sidebar_after_relaunch(path: &std::path::Path) -> WorkspaceState {
    let mut relaunched = WorkspaceState::with_state_path(Some(path.to_path_buf()));
    relaunched.restore().expect("state loads on relaunch");
    relaunched
}

/// THE regression test for the blocker: quitting with one active session
/// must not erase it. This is the single most common case — one session,
/// quit, relaunch — and the delete-then-save ordering failed it while every
/// `WorkspaceState`-level test passed.
///
/// Mutation check: restoring the original quit path in `teardown`
///
/// ```ignore
/// if let Some(id) = self.active_session.take() {
///     let _ = self.workspace.close_session(id);
/// }
/// self.workspace.persist();
/// ```
///
/// fails this test — the reloaded registry is empty.
#[test]
fn quitting_with_an_active_session_keeps_it_for_the_next_launch() {
    let path = temp_state_path();
    let mut app = app_with_state_path(&path);

    // Reproduce what `initialize` does when the PTY spawns: create, select,
    // observe Running, and mark it active.
    let id = app.workspace.create_session(SessionKind::Local);
    app.workspace.select_session(id).expect("session is live");
    app.workspace.observe_session(id, SessionStatus::Running);
    app.active_session = Some(id);

    // The real quit path.
    app.teardown();

    assert!(
        app.active_session.is_none(),
        "teardown releases the active session"
    );

    let relaunched = sidebar_after_relaunch(&path);
    assert_eq!(
        relaunched.registry().len(),
        1,
        "the session the user never asked to close must survive quitting"
    );
    assert_eq!(
        relaunched.registry().sessions()[0].kind(),
        &SessionKind::Local,
    );
    assert_eq!(
        relaunched.sidebar().rows().len(),
        1,
        "the sidebar is not empty after relaunch"
    );
    assert!(!relaunched.sidebar().is_empty());
    cleanup_state_file(&path);
}

/// Quitting must not silently downgrade the session's status claim either:
/// the shell is gone, so the restored entry is `Restored`, never `Running`.
/// Consistent with `restored_sessions_are_restored_not_starting_or_running`, but
/// reached through the quit path rather than a direct workspace mutation.
#[test]
fn session_restored_after_quitting_is_restored_not_running() {
    let path = temp_state_path();
    let mut app = app_with_state_path(&path);
    let id = app.workspace.create_session(SessionKind::Local);
    app.workspace.select_session(id).expect("session is live");
    app.workspace.observe_session(id, SessionStatus::Running);
    app.active_session = Some(id);

    app.teardown();

    let relaunched = sidebar_after_relaunch(&path);
    for descriptor in relaunched.registry().sessions() {
        assert_eq!(
            descriptor.status(),
            &SessionStatus::Restored,
            "a session whose PTY was torn down must not claim to be running"
        );
    }
    let detail = relaunched
        .sidebar()
        .rows()
        .first()
        .and_then(|r| r.detail())
        .unwrap_or_default();
    assert!(
        detail.contains("restored") && detail.contains("not running"),
        "detail identifies a restored, non-running session: {detail}"
    );
    assert!(
        !detail.ends_with("· running"),
        "detail must not claim running: {detail}"
    );
    assert!(
        relaunched.sidebar().viewport().is_none(),
        "a restored selected session must not imply an attachment"
    );
    cleanup_state_file(&path);
}

/// Quitting preserves the selection made through the palette, including
/// when the selected session is the active one. This is the case the
/// original `persist()` call was added for; it must keep working.
#[test]
fn quitting_preserves_the_selection_and_every_other_session() {
    let path = temp_state_path();
    let mut app = app_with_state_path(&path);
    let _first = app.workspace.create_session(SessionKind::Local);
    let second = app.workspace.create_session(SessionKind::Ssh {
        target: "ops@bastion".to_owned(),
    });
    app.workspace
        .select_session(second)
        .expect("second is live");
    app.active_session = Some(second);

    app.teardown();

    let relaunched = sidebar_after_relaunch(&path);
    assert_eq!(
        relaunched.registry().len(),
        2,
        "quitting closes no session, active or not"
    );
    let selected = relaunched
        .registry()
        .selected()
        .expect("selection survives quitting");
    assert_eq!(
        relaunched
            .registry()
            .get(selected)
            .expect("resolves")
            .kind(),
        &SessionKind::Ssh {
            target: "ops@bastion".to_owned()
        },
        "the active session is still the selected one after relaunch",
    );
    cleanup_state_file(&path);
}

/// A session the user *did* close stays closed: quitting must not resurrect
/// it. Guards the opposite direction from the blocker — the fix removes the
/// exit-time close, not the user-initiated one.
#[test]
fn a_session_the_user_closed_does_not_come_back_after_quitting() {
    let path = temp_state_path();
    let mut app = app_with_state_path(&path);
    let first = app.workspace.create_session(SessionKind::Local);
    let second = app.workspace.create_session(SessionKind::Local);
    app.active_session = Some(second);

    // The user closes `first` explicitly — this one really is a close.
    app.workspace.close_session(first).expect("first is live");

    app.teardown();

    let relaunched = sidebar_after_relaunch(&path);
    assert_eq!(
        relaunched.registry().len(),
        1,
        "the explicitly closed session stays closed; the active one survives"
    );
    cleanup_state_file(&path);
}

/// Quitting with no active session still saves. `teardown` must not make
/// its `persist` conditional on there being a session to release.
#[test]
fn quitting_with_no_active_session_still_persists() {
    let path = temp_state_path();
    let mut app = app_with_state_path(&path);
    app.workspace.create_session(SessionKind::Local);
    assert!(app.active_session.is_none(), "no PTY was ever spawned");
    cleanup_state_file(&path);

    app.teardown();

    assert!(path.exists(), "quit must write the state file");
    let relaunched = sidebar_after_relaunch(&path);
    assert_eq!(relaunched.registry().len(), 1);
    cleanup_state_file(&path);
}

/// Without a state path (HOME unset), persistence is entirely in-memory:
/// create does not touch disk and restore is a no-op.
#[test]
fn no_state_path_means_in_memory_only() {
    let mut state = WorkspaceState::with_state_path(None);
    assert!(state.restore().is_ok(), "no path → no-op restore");
    state.create_session(SessionKind::Local);
    assert_eq!(state.registry().len(), 1);
}

/// Mutation check for Issue #111: removing the baseline comparison makes
/// this cross-instance overwrite pass without setting the diagnostics
/// warning.
#[test]
fn second_instance_overwrite_is_detected_and_reported_by_diagnostics() {
    let path = temp_state_path();
    let mut first = WorkspaceState::with_state_path(Some(path.clone()));
    first.create_session(SessionKind::Local);

    let mut second = app_with_state_path(&path);
    first.create_session(SessionKind::Local);
    second.workspace.create_session(SessionKind::Local);

    assert!(second.workspace.persistence_conflict());
    second.toggle_diagnostics();
    assert!(
        second.diagnostics_line.contains("state=changed-underneath"),
        "diagnostics: {}",
        second.diagnostics_line
    );
    cleanup_state_file(&path);
}

// Pure persistence-transition tests moved beside the binary-private state
// machine in `persistence_state.rs`. Filesystem and diagnostics integration
// coverage remains here.

/// The diagnostic integration follows the state machine's two-stage outcome:
/// a definitive post-save replacement is currently unverified, while a later
/// exact retry clears that current flag and reveals the sticky conflict.
#[test]
fn post_save_absence_or_mismatch_then_exact_retry_moves_diagnostics_to_changed_underneath() {
    for (case, replacement) in [
        ("absence", Observation::Observed(None)),
        (
            "mismatch",
            Observation::Observed(Some(b"peer replacement".to_vec())),
        ),
    ] {
        let baseline = b"verified baseline".to_vec();
        let first_write = b"first intended bytes".to_vec();
        let retry_write = b"retry intended bytes".to_vec();
        let mut app = NorenApp::new(AppConfig::default());
        app.workspace
            .persistence
            .restore_succeeded(Some(baseline.clone()));
        app.workspace.persistence.apply_attempt(AttemptOutcome::new(
            Observation::Observed(Some(baseline)),
            SaveOutcome::Written {
                intended: first_write,
                observed: replacement.clone(),
            },
        ));

        assert!(app.workspace.persistence_conflict(), "case={case}");
        assert!(app.workspace.persistence_unverified(), "case={case}");
        app.toggle_diagnostics();
        assert!(
            app.diagnostics_line.ends_with("state=unverified"),
            "case={case}: {}",
            app.diagnostics_line
        );
        app.toggle_diagnostics();

        app.workspace.persistence.apply_attempt(AttemptOutcome::new(
            replacement,
            SaveOutcome::Written {
                intended: retry_write.clone(),
                observed: Observation::Observed(Some(retry_write)),
            },
        ));

        assert!(app.workspace.persistence_conflict(), "case={case}");
        assert!(!app.workspace.persistence_unverified(), "case={case}");
        app.toggle_diagnostics();
        assert!(
            app.diagnostics_line.ends_with("state=changed-underneath"),
            "case={case}: {}",
            app.diagnostics_line
        );
    }
}

/// Mutation check for Issue #122: if the `snapshot` error arm stops marking
/// persistence unverified, an oversized external replacement falsely reports
/// `state=ok` even though the conflict check could not inspect it.
#[test]
fn oversized_external_snapshot_never_reports_persistence_ok() {
    let path = temp_state_path();
    let mut app = app_with_state_path(&path);
    std::fs::write(&path, vec![b'#'; MAX_SESSION_STATE_BYTES as usize + 1])
        .expect("write oversized external replacement");

    app.workspace.create_session(SessionKind::Local);

    assert!(
        app.workspace.persistence_unverified(),
        "the failed external-change check must remain visible"
    );
    app.toggle_diagnostics();
    assert!(
        app.diagnostics_line.contains("state=unverified"),
        "diagnostics: {}",
        app.diagnostics_line
    );
    assert!(
        !app.diagnostics_line.contains("state=ok"),
        "diagnostics must not certify an uninspected save: {}",
        app.diagnostics_line
    );

    let mut saved = SessionRegistry::new();
    load(&path, &mut saved).expect("the atomic save itself still succeeds");
    assert_eq!(
        saved.len(),
        1,
        "the warning is about verification, not loss"
    );
    cleanup_state_file(&path);
}

/// Mutation check for Issue #122: replacing the state file with a directory
/// makes both inspection and atomic rename fail. Dropping failure propagation
/// would leave two in-memory sessions behind a false `state=ok` diagnostic.
#[test]
fn directory_replacement_save_failure_never_reports_persistence_ok() {
    let path = temp_state_path();
    let mut app = app_with_state_path(&path);
    app.workspace.create_session(SessionKind::Local);
    std::fs::remove_file(&path).expect("remove saved state fixture");
    std::fs::create_dir(&path).expect("replace the state path with a directory");

    app.workspace.create_session(SessionKind::Local);

    assert_eq!(app.workspace.registry().len(), 2, "memory did mutate");
    assert!(
        path.is_dir(),
        "the failed save did not replace the directory"
    );
    assert!(
        app.workspace.persistence_unverified(),
        "the failed save must remain visible"
    );
    app.toggle_diagnostics();
    assert!(
        app.diagnostics_line.contains("state=unverified"),
        "diagnostics: {}",
        app.diagnostics_line
    );
    assert!(
        !app.diagnostics_line.contains("state=ok"),
        "diagnostics must not certify unsaved state: {}",
        app.diagnostics_line
    );

    std::fs::remove_dir(&path).expect("remove directory replacement fixture");
}

/// Isolate the save-error arm from the pre-save inspection arm. The existing
/// file is readable and unchanged, but the new in-memory document is too large
/// to encode; removing save-error propagation therefore makes this test fail.
#[test]
fn save_failure_after_clean_inspection_never_reports_persistence_ok() {
    let path = temp_state_path();
    let mut app = app_with_state_path(&path);
    app.workspace.create_session(SessionKind::Local);
    let before = std::fs::read(&path).expect("read the clean baseline");

    app.workspace.create_session(SessionKind::Ssh {
        target: "x".repeat(MAX_SESSION_STATE_BYTES as usize + 1),
    });

    assert_eq!(
        std::fs::read(&path).expect("the baseline remains readable"),
        before,
        "a refused save must leave the prior safe state intact"
    );
    assert!(
        app.workspace.persistence_unverified(),
        "a save error after clean inspection must remain visible"
    );
    app.toggle_diagnostics();
    assert!(
        app.diagnostics_line.contains("state=unverified"),
        "diagnostics: {}",
        app.diagnostics_line
    );
    assert!(
        !app.diagnostics_line.contains("state=ok"),
        "diagnostics must not certify unsaved state: {}",
        app.diagnostics_line
    );
    cleanup_state_file(&path);
}

/// Clean persistence must remain distinguishable from both Issue #122 error
/// paths; making either the default or successful path unverified breaks this.
#[test]
fn single_instance_save_has_no_persistence_false_alarm() {
    let path = temp_state_path();
    let mut app = app_with_state_path(&path);
    app.workspace.create_session(SessionKind::Local);

    app.toggle_diagnostics();
    assert!(
        app.diagnostics_line.contains("state=ok"),
        "diagnostics: {}",
        app.diagnostics_line
    );
    assert!(
        !app.diagnostics_line.contains("changed-underneath"),
        "diagnostics: {}",
        app.diagnostics_line
    );
    assert!(
        !app.diagnostics_line.contains("unverified"),
        "diagnostics: {}",
        app.diagnostics_line
    );
    let saved = std::fs::read(&path).expect("read exact verified save");
    assert!(!saved.is_empty(), "the real save path wrote a document");
    cleanup_state_file(&path);
}

// ── SSH connect: real launches, refusals, and visible failure states ──

/// Test constructor with the deterministic ssh spawn seam disabled: the
/// click path runs its full validation and phase reporting without
/// launching any process.
fn app_with_deterministic_ssh_seam() -> NorenApp {
    app_with_deterministic_ssh_seam_and_config(AppConfig::default())
}

/// The same deterministic ssh seam, carrying a real configuration so a test
/// can exercise the configured surfaces (`[[agents]]` rows included).
fn app_with_deterministic_ssh_seam_and_config(config: AppConfig) -> NorenApp {
    NorenApp {
        ssh_spawn_enabled: false,
        ..NorenApp::new(config)
    }
}

/// Unique secret-shaped stand-in for a destination that could embed a
/// credential. Any surface that prints it is a leak.
fn ssh_secret_sentinel(tag: &str) -> String {
    format!("NOREN-SSHCONN-{tag}-hunter2-{}", std::process::id())
}

#[test]
fn ssh_click_surfaces_launch_failure_when_no_child_can_spawn() {
    let fixture = SshConfigFixture::new();
    fixture.write_new(b"Host web\n");
    let mut app = app_with_deterministic_ssh_seam();
    app.load_ssh_hosts_from(fixture.path());
    app.cursor_position = Some(PhysicalPosition::new(5.0, 1.0));

    assert!(app.handle_sidebar_click_in_frame(
        ElementState::Pressed,
        MouseButton::Left,
        PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT),
    ));

    // The failure is first-class: runtime status row, provenance line, and
    // the sidebar row all carry the launch failure.
    assert_eq!(app.status, "Noren ssh launch failed");
    assert!(app.show_status, "the launch failure must be visible");
    assert_eq!(
        app.ssh_selection_status.as_deref(),
        Some("SSH partial source #0 config; launch failed")
    );
    let row = &app.workspace.sidebar().rows()[0];
    assert_eq!(row.label(), "SSH-ERR web");
    assert_eq!(row.detail(), Some("launch failed"));
    assert!(
        app.pty.is_none(),
        "a failed launch must leave no PTY behind"
    );
    assert!(
        app.workspace.registry().sessions().is_empty(),
        "the refused registry must record no session"
    );
}

#[test]
fn ssh_spawn_failure_never_retires_the_running_local_session() {
    // Found by independent review: connect_ssh_target orders
    // retire_live_terminal() inside the Ok arm only, but no test pinned it —
    // a mutation moving the retire BEFORE the spawn attempt passed the whole
    // suite. This seeds a real live session and forces the spawn's Err arm,
    // so a failed ssh launch must leave the running local shell untouched.
    let home = AppTestHome::new();
    let mut app = home.app();
    app.run_workspace_action(WorkspaceAction::CreateSession);
    let live = registry_ids(&app)[0];
    assert!(app.pty.is_some(), "a live local session is running");

    let fixture = SshConfigFixture::new();
    fixture.write_new(b"Host web\n");
    app.ssh_spawn_force_failure = true;
    app.load_ssh_hosts_from(fixture.path());
    // Row 0 is the live local session; the SSH host sits at row 1.
    app.cursor_position = Some(PhysicalPosition::new(5.0, 25.0));

    assert!(app.handle_sidebar_click_in_frame(
        ElementState::Pressed,
        MouseButton::Left,
        PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT),
    ));

    assert_eq!(
        app.status, "Noren ssh launch failed",
        "the failure is surfaced first-class"
    );
    assert!(
        app.pty.is_some(),
        "a failed ssh spawn must not tear down the running local shell"
    );
    assert_eq!(
        app.active_session,
        Some(live),
        "the local session keeps the live view"
    );
    assert_eq!(
        session_status(&app, live),
        SessionStatus::Running,
        "the local session is not observed Exiting behind a failed launch"
    );
}

#[test]
fn ssh_click_refuses_a_raw_token_destination_without_spawning() {
    let secret = ssh_secret_sentinel("TOKEN");
    let fixture = SshConfigFixture::new();
    fixture.write_new(format!("Host %p-{secret}\n").as_bytes());
    let mut app = app_with_deterministic_ssh_seam();
    app.load_ssh_hosts_from(fixture.path());
    app.cursor_position = Some(PhysicalPosition::new(5.0, 1.0));

    assert!(app.handle_sidebar_click_in_frame(
        ElementState::Pressed,
        MouseButton::Left,
        PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT),
    ));

    let status = app
        .ssh_selection_status
        .as_deref()
        .expect("the typed refusal is visible");
    assert!(
        status.contains("%p"),
        "the refusal must name the token: {status}"
    );
    assert!(
        status.contains("Port"),
        "the refusal must name the keyword: {status}"
    );
    assert!(
        !status.contains(&secret),
        "the refusal must never carry destination content: {status}"
    );
    // The connect did not proceed.
    assert!(app.pty.is_none(), "a refused destination must not spawn");
    assert!(app.workspace.ssh_connection().is_none());
    let row = &app.workspace.sidebar().rows()[0];
    assert!(
        row.label().starts_with("SSH-OFF "),
        "the row must stay disconnected: {}",
        row.label()
    );
    assert_eq!(row.detail(), Some("not connected"));
    // No debug surface may print the destination either.
    assert!(!format!("{:?}", app.workspace).contains(&secret));
}

#[test]
fn ssh_exit_observation_maps_every_child_outcome_to_a_visible_phase() {
    assert_eq!(
        ssh_exit_observation(Some(0)),
        SshConnectionPhase::Closed,
        "a clean ssh exit is a close, not a failure"
    );
    for code in [1, 127, 255] {
        assert_eq!(
            ssh_exit_observation(Some(code)),
            SshConnectionPhase::ConnectFailed,
            "exit {code} is an unreachable-host/auth/disconnect failure"
        );
    }
    assert_eq!(
        ssh_exit_observation(None),
        SshConnectionPhase::Disconnected,
        "a code-less end is an immediate disconnect"
    );
    assert_eq!(ssh_launch_observation(true), SshConnectionPhase::Connecting);
    assert_eq!(
        ssh_launch_observation(false),
        SshConnectionPhase::LaunchFailed,
        "a failed spawn must never report success"
    );

    for phase in [
        SshConnectionPhase::Connecting,
        SshConnectionPhase::Connected,
        SshConnectionPhase::Closed,
        SshConnectionPhase::LaunchFailed,
        SshConnectionPhase::ConnectFailed,
        SshConnectionPhase::Disconnected,
    ] {
        assert!(!phase.status_text().is_empty());
        assert!(!phase.sidebar_detail().is_empty());
        assert_eq!(
            phase.sidebar_prefix().chars().count(),
            SSH_SIDEBAR_LABEL_PREFIX_CHARS,
            "the prefix must keep the fixed label arithmetic"
        );
        assert!(phase.sidebar_prefix().starts_with("SSH-"));
    }
}

#[test]
fn ssh_sidebar_state_prefixes_encode_the_connection_phase() {
    assert_eq!(
        ssh_state_label(SshConnectionPhase::Connected, "abcdef"),
        "SSH-ON  abcdef"
    );
    assert_eq!(
        ssh_state_label(SshConnectionPhase::Connecting, "abcdefg"),
        "SSH-ON  abc..."
    );
    assert_eq!(
        ssh_state_label(SshConnectionPhase::ConnectFailed, "abcdefg"),
        "SSH-ERR abc..."
    );
    assert_eq!(
        ssh_state_label(SshConnectionPhase::Disconnected, "abcdef"),
        "SSH-ERR abcdef"
    );
    assert_eq!(
        ssh_state_label(SshConnectionPhase::Closed, "abcdefg"),
        "SSH-OFF abc..."
    );
    // Long targets stay bounded in every phase.
    for phase in [
        SshConnectionPhase::Connecting,
        SshConnectionPhase::Connected,
        SshConnectionPhase::Closed,
        SshConnectionPhase::LaunchFailed,
        SshConnectionPhase::ConnectFailed,
        SshConnectionPhase::Disconnected,
    ] {
        let label = ssh_state_label(phase, &"x".repeat(2048));
        assert_eq!(label.chars().count(), SSH_SIDEBAR_LABEL_CHARS);
    }
}

#[test]
fn ssh_connection_marker_identifies_only_the_exact_connected_target() {
    let fixture = SshConfigFixture::new();
    fixture.write_new(b"Host abcdef-first\nHost abcdef-second\n");
    let mut workspace = WorkspaceState::new();
    let config = SshConfig::read(fixture.path()).expect("bounded SSH fixture parses");
    workspace.load_ssh_config(&config);

    workspace.set_ssh_connection("abcdef-second", SshConnectionPhase::Connected);
    let rows = workspace.sidebar().rows();
    assert!(
        rows[0].label().starts_with("SSH-OFF "),
        "{}",
        rows[0].label()
    );
    assert!(
        rows[1].label().starts_with("SSH-ON  "),
        "{}",
        rows[1].label()
    );
    assert_eq!(rows[1].detail(), Some("connected"));

    workspace.set_ssh_connection("abcdef-second", SshConnectionPhase::ConnectFailed);
    let rows = workspace.sidebar().rows();
    assert!(
        rows[1].label().starts_with("SSH-ERR "),
        "{}",
        rows[1].label()
    );
    assert_eq!(rows[1].detail(), Some("connection failed"));
}

#[test]
fn ssh_connect_flow_never_persists_or_debug_prints_the_destination() {
    let secret = ssh_secret_sentinel("PERSIST");
    let fixture = SshConfigFixture::new();
    fixture.write_new(format!("Host {secret}\n").as_bytes());
    let path = temp_state_path();
    let mut workspace = WorkspaceState::with_state_path(Some(path.clone()));
    let local = workspace.create_session(SessionKind::Local);
    workspace.observe_session(local, SessionStatus::Running);
    let config = SshConfig::read(fixture.path()).expect("bounded SSH fixture parses");
    workspace.load_ssh_config(&config);

    // The full connect path's workspace writes: pending selection, live
    // connection, observed failure. Row 1 is the host row: the local
    // session occupies row 0.
    assert!(workspace.select_ssh_sidebar_row(1));
    workspace.set_ssh_connection(&secret, SshConnectionPhase::Connecting);
    workspace.set_ssh_connection(&secret, SshConnectionPhase::ConnectFailed);
    workspace.persist();

    let written = std::fs::read_to_string(&path).expect("the real save path wrote a document");
    assert!(
        !written.contains(&secret),
        "the persisted sessions.toml must never carry the destination: {written}"
    );
    assert!(
        !written.contains("kind = \"ssh\""),
        "an SSH launch must never enter the persisted registry: {written}"
    );
    assert!(
        !format!("{workspace:?}").contains(&secret),
        "no debug surface may carry the destination"
    );
    assert!(
        workspace
            .registry()
            .sessions()
            .iter()
            .all(|descriptor| !matches!(descriptor.kind(), SessionKind::Ssh { .. })),
        "the live registry records no SSH session for the launch"
    );
    cleanup_state_file(&path);
}

#[test]
fn workspace_debug_holds_no_nested_content_through_the_sidebar_and_persistence() {
    // Issue #146: the guard must survive nesting. Sentinels are planted in
    // the NESTED leaves — the configured-host sidebar rows, the persistence
    // baseline bytes — and in the workspace's own content fields, then the
    // OUTER WorkspaceState is formatted directly with `{:?}` (per PR #142,
    // the guard never depends on a production formatter existing).
    let secret = ssh_secret_sentinel("NESTED");
    // Short enough to survive the sidebar's bounded-target truncation, so a
    // leaked row label carries it in full.
    let short_secret = format!("Ns{}", std::process::id() % 10_000);

    let fixture = SshConfigFixture::new();
    fixture.write_new(format!("Host {short_secret}\n").as_bytes());
    let mut workspace = WorkspaceState::with_state_path(Some(PathBuf::from(format!(
        "/tmp/{secret}-sessions.toml"
    ))));
    let config = SshConfig::read(fixture.path()).expect("bounded SSH fixture parses");
    workspace.load_ssh_config(&config);
    assert!(
        workspace.select_ssh_sidebar_row(0),
        "row 0 is the configured host (the registry is empty)"
    );
    workspace.set_ssh_connection(&short_secret, SshConnectionPhase::Connecting);
    // Direct plants for fields no public seam reaches from this scenario.
    workspace.selected_ssh_source_label = Some(secret.clone());
    let sentinel_bytes = secret.as_bytes().to_vec();
    workspace
        .persistence
        .restore_succeeded(Some(sentinel_bytes.clone()));

    let rendered = format!("{workspace:?}");
    assert!(
        !rendered.contains(&secret),
        "workspace debug leaked the long sentinel: {rendered}"
    );
    assert!(
        !rendered.contains(&short_secret),
        "workspace debug leaked the sidebar-visible sentinel: {rendered}"
    );
    assert!(
        !rendered.contains(&format!("{:?}", sentinel_bytes.as_slice())),
        "workspace debug leaked the persisted bytes as a byte list: {rendered}"
    );
    // Not vacuous: the workspace still describes its shape.
    assert!(rendered.contains("WorkspaceState"), "{rendered}");
    assert!(rendered.contains("SidebarView"), "{rendered}");
    assert!(rendered.contains("PersistenceState"), "{rendered}");
    assert!(rendered.contains("selected_row: Some(0)"), "{rendered}");
    assert!(
        rendered.contains("ssh_connection: Some(Connecting)"),
        "the fixed connection phase is safe shape: {rendered}"
    );
}

#[test]
fn configured_ssh_host_debug_reports_shape_without_target_or_source() {
    // Issue #146 triage: ConfiguredSshHost holds the SSH target and its
    // config-path provenance inside WorkspaceState itself — the same
    // secret, one container field away from Debug. The leaf must be safe
    // by construction, with the discriminant and lengths only.
    let secret = ssh_secret_sentinel("CFGHOST");
    let fixture = SshConfigFixture::new();
    fixture.write_new(format!("Host {secret}\n").as_bytes());
    let mut workspace = WorkspaceState::new();
    let config = SshConfig::read(fixture.path()).expect("bounded SSH fixture parses");
    workspace.load_ssh_config(&config);

    let rendered = format!("{:?}", workspace.ssh_hosts);
    assert!(
        !rendered.contains(&secret),
        "configured host debug leaked the target or source: {rendered}"
    );
    assert!(
        rendered.contains("ConfiguredSshHost { kind: \"ssh\""),
        "the launch-shape discriminant stays visible: {rendered}"
    );
    assert!(
        rendered.contains("source_label_chars"),
        "the provenance length stays visible: {rendered}"
    );
}

// ── Live multi-session bookkeeping (supervisor-backed switching, slice 1) ──
//
// The palette's `session_create` now spawns a real `/bin/zsh` PTY per row.
// These tests spawn real PTYs, following the `noren-pty` test convention; the
// children are reaped through `PtySession`'s bounded shutdown when the app (or
// its parked map) drops.

/// An isolated home directory for PTY children, following the `noren-pty`
/// suite's `TestHome` convention.
///
/// Tests that drive a shell by typing must not run it in the developer's real
/// `$HOME`: personal startup files can take arbitrarily long (over a minute on
/// some machines) or read the terminal, which would make the test's prompt
/// wait depend on personal configuration. An empty home gives the same fixed
/// `/bin/zsh` policy with a deterministic, immediate prompt.
struct AppTestHome(PathBuf);

impl AppTestHome {
    fn new() -> Self {
        static SEQUENCE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "noren-app-test-home-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&path).expect("create isolated test home");
        Self(path)
    }

    /// A default app whose spawned sessions run in this isolated home —
    /// local sessions and worktree sessions alike (a worktree child still
    /// starts *in* its worktree; only its `HOME` is isolated).
    ///
    /// Borrows the guard so it stays alive (and the directory stays present)
    /// for the whole test: the child validates the directory at spawn time.
    fn app(&self) -> NorenApp {
        NorenApp {
            test_pty_home: Some(self.0.clone()),
            ..NorenApp::default()
        }
    }
}

impl Drop for AppTestHome {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove isolated test home");
    }
}

/// Registry ids in sidebar order.
fn registry_ids(app: &NorenApp) -> Vec<SessionId> {
    app.workspace
        .registry()
        .sessions()
        .into_iter()
        .map(|descriptor| descriptor.id())
        .collect()
}

/// Observed status of a live registry row.
fn session_status(app: &NorenApp, id: SessionId) -> SessionStatus {
    app.workspace
        .registry()
        .get(id)
        .expect("session exists in the registry")
        .status()
        .clone()
}

/// Drain the live view until the shell has produced its first output.
///
/// The shell sessions these tests drive run in an isolated `HOME` (see
/// [`AppTestHome`]), so their prompt is immediate and deterministic; a
/// developer's real `$HOME` with slow startup files could not meet this
/// deadline. Input typed while zsh is still starting races its startup, so
/// every test that drives a shell by typing first waits for its prompt.
fn wait_for_shell_output(app: &mut NorenApp) {
    let start = Instant::now();
    let deadline = start + Duration::from_secs(10);
    loop {
        app.drain_pty();
        let ready = app
            .terminal
            .as_ref()
            .is_some_and(|terminal| terminal.screen().display_row_count() > 0);
        if ready {
            return;
        }
        if Instant::now() >= deadline {
            let rows = app
                .terminal
                .as_ref()
                .map(|terminal| terminal.screen().display_row_count());
            let text = app.terminal.as_ref().map(terminal_text).unwrap_or_default();
            panic!(
                "the spawned shell never produced its prompt\n\
                 expected: the live terminal to render at least one display row \
                 of shell output within 10s\n\
                 received: display_row_count={rows:?} after {:?} \
                 (terminal_attached={}, pty_attached={}), terminal said: {:?}",
                start.elapsed(),
                app.terminal.is_some(),
                app.pty.is_some(),
                text,
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The palette's session_create must spawn a real local PTY: the row is
/// observed `Running` (never left `Starting`), it owns the live surface, and a
/// second create parks the first live session instead of killing it.
///
/// Mutation check: making `session_create` add a model row without spawning
/// (no PTY, no `Running` observation) fails every assertion past the first.
#[test]
fn palette_create_spawns_a_real_local_pty_session() {
    let home = AppTestHome::new();
    let mut app = home.app();

    app.run_workspace_action(WorkspaceAction::CreateSession);

    let ids = registry_ids(&app);
    assert_eq!(ids.len(), 1);
    let first = ids[0];
    assert_eq!(
        session_status(&app, first),
        SessionStatus::Running,
        "a spawned session must be observed Running, not left Starting"
    );
    assert!(
        app.pty.is_some(),
        "the new session must own a real live PTY surface"
    );
    assert_eq!(app.active_session, Some(first));
    assert_eq!(app.workspace.registry().selected(), Some(first));

    app.run_workspace_action(WorkspaceAction::CreateSession);
    let ids = registry_ids(&app);
    assert_eq!(ids.len(), 2);
    let second = ids[1];
    assert_eq!(
        session_status(&app, second),
        SessionStatus::Running,
        "the second spawn is also a real running session"
    );
    assert_eq!(
        app.active_session,
        Some(second),
        "a new session takes the live view"
    );
    assert!(
        app.parked_sessions.contains_key(&first),
        "the previous live session must be parked, not dropped"
    );
    assert_eq!(
        session_status(&app, first),
        SessionStatus::Running,
        "parking keeps the first session truthfully Running"
    );
}

/// A parked session that exits in the background is observed through the
/// registry, reaped, and detached — it never stays `Running` and never leaves
/// a live-surface entry behind.
#[test]
fn a_parked_session_that_exits_is_observed_and_detached() {
    let home = AppTestHome::new();
    let mut app = home.app();
    app.run_workspace_action(WorkspaceAction::CreateSession);
    let first = registry_ids(&app)[0];
    // Wait for the shell to finish starting up before typing into it: input
    // queued while zsh is still reading its startup files races them. The
    // isolated home makes that startup immediate and deterministic.
    wait_for_shell_output(&mut app);
    // Tell the first shell to exit while it still owns the live view; the
    // exit is only observed after the session has been parked, because
    // nothing drains between these calls.
    app.send_input(b"exit\n");
    app.run_workspace_action(WorkspaceAction::CreateSession);
    let second = registry_ids(&app)[1];
    assert!(app.parked_sessions.contains_key(&first));

    let deadline = Instant::now() + Duration::from_secs(5);
    while session_status(&app, first) == SessionStatus::Running {
        app.drain_pty();
        app.drain_parked_sessions();
        assert!(
            Instant::now() < deadline,
            "the parked session's exit was never observed"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(
        matches!(session_status(&app, first), SessionStatus::Exited { .. }),
        "an exited parked session is observed as Exited (the code is None when \
         the reader's EOF wins the race with the supervisor's exit poll)"
    );
    assert!(
        !app.parked_sessions.contains_key(&first),
        "a dead parked session must be detached from the live bookkeeping"
    );
    assert_eq!(
        app.active_session,
        Some(second),
        "a parked exit must not disturb the live view"
    );
    assert_eq!(session_status(&app, second), SessionStatus::Running);
}

/// Quitting with several real live sessions persists them all for the next
/// launch, where they come back as `Restored` rows — live PTYs never survive a
/// restart, and the persisted model must say so.
#[test]
fn quitting_persists_spawned_sessions_for_the_next_launch() {
    let path = temp_state_path();
    let home = AppTestHome::new();
    let mut app = NorenApp {
        test_pty_home: Some(home.0.clone()),
        ..app_with_state_path(&path)
    };
    app.run_workspace_action(WorkspaceAction::CreateSession);
    app.run_workspace_action(WorkspaceAction::CreateSession);

    app.teardown();

    let text = std::fs::read_to_string(&path).expect("state saved on quit");
    assert_eq!(
        text.matches("kind = \"local\"").count(),
        2,
        "both spawned sessions persist: {text}"
    );

    let relaunched = sidebar_after_relaunch(&path);
    assert_eq!(relaunched.registry().len(), 2);
    for descriptor in relaunched.registry().sessions() {
        let status = descriptor.status().clone();
        assert_eq!(
            status,
            SessionStatus::Restored,
            "a relaunched row must be Restored, not Running"
        );
    }
    assert!(
        relaunched.registry().selected().is_some(),
        "the persisted selection survives the restart"
    );
    cleanup_state_file(&path);
}

#[cfg(target_os = "macos")]
#[test]
fn ssh_click_connects_the_system_client_end_to_end() {
    // The alias begins with `@`, which the system ssh client rejects during
    // its own argument parsing: the launch, the interactive I/O path, and
    // the failure mapping are all observed with no network access and no
    // credential.
    let fixture = SshConfigFixture::new();
    fixture.write_new(b"Host @noren-refuse\n");
    let mut app = NorenApp::default();
    app.load_ssh_hosts_from(fixture.path());
    app.terminal = Some(TerminalState::new(24, 80).expect("valid grid"));
    app.cursor_position = Some(PhysicalPosition::new(5.0, 1.0));

    assert!(app.handle_sidebar_click_in_frame(
        ElementState::Pressed,
        MouseButton::Left,
        PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT),
    ));

    // The click spawned the real system ssh client in the terminal's PTY.
    assert!(
        app.pty.is_some(),
        "the click must launch the system ssh client"
    );
    assert_eq!(
        app.workspace.ssh_connection().map(|(_, p)| *p),
        Some(SshConnectionPhase::Connecting)
    );
    assert_eq!(app.status, "Noren ssh connecting");

    // Pump the production drain loop until the child's failure surfaces.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        app.drain_pty();
        let phase = app.workspace.ssh_connection().map(|(_, p)| *p);
        if matches!(
            phase,
            Some(
                SshConnectionPhase::ConnectFailed
                    | SshConnectionPhase::Disconnected
                    | SshConnectionPhase::Closed
            )
        ) || std::time::Instant::now() >= deadline
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    assert_eq!(
        app.workspace.ssh_connection().map(|(_, phase)| *phase),
        Some(SshConnectionPhase::ConnectFailed),
        "ssh's own error exit must surface as a connection failure"
    );
    assert_eq!(app.status, "Noren ssh connection failed");
    assert!(app.show_status, "the failure must own the status row");
    assert!(app.pty.is_none(), "the failed child's PTY is retired");

    // ssh's own usage diagnostic flowed through the normal terminal path.
    let snapshot = app.terminal.as_ref().expect("terminal present").snapshot();
    let rendered: Vec<&str> = snapshot.lines().iter().map(String::as_str).collect();
    assert!(
        rendered.iter().any(|line| line.contains("usage:")),
        "the ssh diagnostic must reach the terminal content"
    );

    // The sidebar row carries the failure state.
    let row = &app.workspace.sidebar().rows()[0];
    assert!(row.label().starts_with("SSH-ERR "), "{}", row.label());
    assert_eq!(row.detail(), Some("connection failed"));
}

// ── Sidebar switching (supervisor-backed switching, slice 2) ────────────

/// Whether the session owns a live surface: it is the active one or parked.
fn has_live_surface(app: &NorenApp, id: SessionId) -> bool {
    app.active_session == Some(id) || app.parked_sessions.contains_key(&id)
}

/// Visible text of a terminal state, extracted through the selection path.
fn terminal_text(terminal: &TerminalState) -> String {
    Selection::entire_grid(terminal).extract(terminal)
}

/// Click a sidebar row in a default-sized synthetic frame.
fn click_sidebar_row(app: &mut NorenApp, row: usize) -> bool {
    app.cursor_position = Some(PhysicalPosition::new(
        5.0,
        f64::from(app.geometry.cell_height()) * row as f64 + 1.0,
    ));
    app.handle_sidebar_click_in_frame(
        ElementState::Pressed,
        MouseButton::Left,
        PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT),
    )
}

/// Clicking another session row must move the whole live view — the terminal
/// surface the renderer draws and `send_input` feeds — to the clicked
/// session, and switching back must show that session's own screen again.
///
/// Mutation check: making the switch a no-op that keeps rendering the old
/// session fails the screen-content assertions after every click.
#[test]
fn sidebar_click_switches_the_live_surface_between_sessions() {
    let mut app = NorenApp::default();
    app.run_workspace_action(WorkspaceAction::CreateSession);
    app.run_workspace_action(WorkspaceAction::CreateSession);
    let ids = registry_ids(&app);
    let (first, second) = (ids[0], ids[1]);
    assert_eq!(app.active_session, Some(second));

    // The second (live) session's screen gets a marker through the exact
    // byte path production uses for PTY output.
    app.apply_pty_output(b"SECOND-LIVE-7f31\r\n");

    assert!(click_sidebar_row(&mut app, 0), "row 0 is consumed");
    assert_eq!(app.active_session, Some(first), "the live view moved");
    assert_eq!(app.workspace.registry().selected(), Some(first));
    let text = terminal_text(app.terminal.as_ref().expect("first surface attached"));
    assert!(
        !text.contains("SECOND-LIVE-7f31"),
        "the live view must show the first session's screen, not the previous one"
    );
    assert!(
        has_live_surface(&app, second),
        "the parked session stays live"
    );

    // Each session's screen keeps its own bytes after a switch.
    app.apply_pty_output(b"FIRST-LIVE-7f31\r\n");
    assert!(click_sidebar_row(&mut app, 1));
    assert_eq!(app.active_session, Some(second));
    let text = terminal_text(app.terminal.as_ref().expect("second surface attached"));
    assert!(text.contains("SECOND-LIVE-7f31"));
    assert!(!text.contains("FIRST-LIVE-7f31"));

    assert!(click_sidebar_row(&mut app, 0));
    assert_eq!(app.active_session, Some(first));
    let text = terminal_text(app.terminal.as_ref().expect("first surface attached"));
    assert!(
        text.contains("FIRST-LIVE-7f31"),
        "switching back re-attaches the first session's own screen"
    );
    assert!(!text.contains("SECOND-LIVE-7f31"));
}

/// A row without a live surface (model-only, restored, or exited) is consumed
/// but cannot take input ownership: the current live session keeps it.
#[test]
fn switching_to_a_row_without_a_live_surface_keeps_the_current_owner() {
    let mut app = NorenApp::default();
    app.run_workspace_action(WorkspaceAction::CreateSession);
    let live = registry_ids(&app)[0];
    // A model-only row straight from the registry seam: no PTY behind it.
    let _model = app.workspace.create_session(SessionKind::Local);
    app.workspace.rebuild_sidebar();

    assert!(click_sidebar_row(&mut app, 1), "the model row is consumed");

    assert_eq!(
        app.active_session,
        Some(live),
        "a row without a live surface must not take the live view"
    );
    // The click selects the model row (close targets what was clicked);
    // input ownership stays with the live session.
    assert_eq!(app.workspace.registry().selected(), Some(_model));
    assert!(app.pty.is_some(), "the live surface is untouched");
    assert!(!has_live_surface(&app, _model));
}

/// Switching expires selections captured on the previous session's screen:
/// grid coordinates only address the content they were captured on.
#[test]
fn switching_expires_the_previous_sessions_selection() {
    let mut app = NorenApp::default();
    app.run_workspace_action(WorkspaceAction::CreateSession);
    app.run_workspace_action(WorkspaceAction::CreateSession);
    let ids = registry_ids(&app);
    app.apply_pty_output(b"SELECT-7f31");
    app.select_entire_grid();
    assert!(app.selection.is_some());

    assert!(click_sidebar_row(&mut app, 0));

    assert_eq!(app.active_session, Some(ids[0]));
    assert!(
        app.selection.is_none(),
        "a selection from the previous screen must not survive the switch"
    );
    assert!(app.drag_origin.is_none());
}

/// Creating a new session takes the live view through the same expiry a
/// sidebar switch runs: a selection captured on the old screen must not
/// survive into the new session's coordinates (found by independent review —
/// `spawn_local_session` once switched surfaces without expiring).
#[test]
fn creating_a_session_expires_the_previous_sessions_selection() {
    let home = AppTestHome::new();
    let mut app = home.app();
    app.run_workspace_action(WorkspaceAction::CreateSession);
    app.apply_pty_output(b"CREATE-9c2e");
    app.select_entire_grid();
    assert!(app.selection.is_some());

    app.run_workspace_action(WorkspaceAction::CreateSession);

    assert!(
        app.selection.is_none(),
        "a selection from the previous screen must not survive creation"
    );
    assert!(app.drag_origin.is_none());
}

/// Closing the row whose EXITED final frame is still displayed must clear
/// the surface and run the same fallback an active close does — not leave a
/// frozen frame behind a vanished row in an empty workspace (found by
/// independent review).
#[test]
fn closing_the_displayed_exited_frame_clears_the_surface_honestly() {
    let home = AppTestHome::new();
    let mut app = home.app();
    app.run_workspace_action(WorkspaceAction::CreateSession);
    let id = registry_ids(&app)[0];
    assert_eq!(app.active_session, Some(id));

    // The child exits: finish_pty keeps the final frame displayed with input
    // ownership already gone.
    app.finish_pty("Noren shell reached EOF");
    assert_eq!(app.active_session, None);
    assert!(app.terminal.is_some(), "the final frame stays displayed");

    app.run_workspace_action(WorkspaceAction::CloseSession);

    assert!(
        app.workspace.registry().get(id).is_none(),
        "the exited row is closed"
    );
    assert!(
        app.terminal.is_none(),
        "no closed session's frozen frame may stay on screen"
    );
    assert!(
        app.parked_sessions.is_empty() && app.active_session.is_none(),
        "an empty workspace is reported honestly"
    );
}

// ── Closing live sessions (supervisor-backed switching, slice 3) ────────

/// Closing the ACTIVE session while other live sessions exist falls back to
/// the topmost remaining live row (the lowest id), and that row's own screen
/// — not the closed session's — owns the live view afterwards.
///
/// Mutation check: skipping the reaping/detaching in `close_session` (or
/// falling back to the closed session's own surface) fails the marker and
/// ownership assertions.
#[test]
fn closing_the_active_session_falls_back_to_the_topmost_live_session() {
    let home = AppTestHome::new();
    let mut app = home.app();
    app.run_workspace_action(WorkspaceAction::CreateSession);
    let first = registry_ids(&app)[0];
    // First session's screen gets a marker while it still owns the live view.
    app.apply_pty_output(b"FIRST-SCREEN-9c2d\r\n");
    app.run_workspace_action(WorkspaceAction::CreateSession);
    let second = registry_ids(&app)[1];
    assert_eq!(app.active_session, Some(second));

    // The palette closes the selected row; the newest session is both
    // selected and active.
    app.run_workspace_action(WorkspaceAction::CloseSession);

    assert!(app.workspace.registry().get(second).is_none());
    assert_eq!(
        app.active_session,
        Some(first),
        "the live view falls back to the topmost remaining live session"
    );
    assert_eq!(app.workspace.registry().selected(), Some(first));
    assert!(
        app.pty.is_some(),
        "the fallback session's real PTY owns the live view"
    );
    let text = terminal_text(app.terminal.as_ref().expect("fallback surface"));
    assert!(
        text.contains("FIRST-SCREEN-9c2d"),
        "the fallback must show the first session's own screen"
    );
    assert_eq!(session_status(&app, first), SessionStatus::Running);
    assert!(app.parked_sessions.is_empty());
}

/// Closing a PARKED session reaps its child and leaves the live view on the
/// active session, untouched.
#[test]
fn closing_a_parked_session_reaps_it_and_keeps_the_live_view() {
    let home = AppTestHome::new();
    let mut app = home.app();
    app.run_workspace_action(WorkspaceAction::CreateSession);
    app.run_workspace_action(WorkspaceAction::CreateSession);
    let ids = registry_ids(&app);
    let (first, second) = (ids[0], ids[1]);
    assert!(app.parked_sessions.contains_key(&first));

    // Make the parked row the palette's target: registry selection is a
    // model-level choice and does not move the live view.
    app.workspace
        .select_session(first)
        .expect("parked row selectable");

    app.run_workspace_action(WorkspaceAction::CloseSession);

    assert!(app.workspace.registry().get(first).is_none());
    assert!(
        !app.parked_sessions.contains_key(&first),
        "the parked child handle was reaped and removed, not left in the map"
    );
    assert_eq!(
        app.active_session,
        Some(second),
        "the live view is untouched"
    );
    assert!(app.pty.is_some());
    assert_eq!(session_status(&app, second), SessionStatus::Running);
    assert_eq!(app.workspace.registry().selected(), None);
}

/// A structural close persists immediately: the next launch restores exactly
/// the surviving rows as `Restored` entries, never the closed one, and never
/// a live status for a shell that died with the previous launch.
#[test]
fn closing_a_session_persists_the_shrunk_sidebar_for_the_next_launch() {
    let home = AppTestHome::new();
    let path = temp_state_path();
    let mut app = NorenApp {
        test_pty_home: Some(home.0.clone()),
        ..app_with_state_path(&path)
    };
    app.run_workspace_action(WorkspaceAction::CreateSession);
    app.run_workspace_action(WorkspaceAction::CreateSession);
    app.run_workspace_action(WorkspaceAction::CloseSession);

    let relaunched = sidebar_after_relaunch(&path);
    assert_eq!(
        relaunched.registry().len(),
        1,
        "the closed row is not saved"
    );
    for descriptor in relaunched.registry().sessions() {
        assert_eq!(
            descriptor.status().clone(),
            SessionStatus::Restored,
            "a relaunched row is Restored: its shell did not survive the launch"
        );
    }
    cleanup_state_file(&path);
}

/// The palette's `session_select` cycles the live view through live sessions
/// in sidebar order, wrapping around, and each switch shows the target
/// session's own screen. With no other live session it re-affirms the current
/// owner instead of moving input to a model-only row.
///
/// Mutation check: making the switch a no-op that keeps rendering the old
/// session fails every screen-content assertion here.
#[test]
fn palette_select_cycles_the_live_view_between_live_sessions() {
    let home = AppTestHome::new();
    let mut app = home.app();
    // Three real sessions; each gets a marker on its own screen while it owns
    // the live view.
    app.run_workspace_action(WorkspaceAction::CreateSession);
    app.apply_pty_output(b"MARK-A-51e0\r\n");
    app.run_workspace_action(WorkspaceAction::CreateSession);
    app.apply_pty_output(b"MARK-B-51e0\r\n");
    app.run_workspace_action(WorkspaceAction::CreateSession);
    app.apply_pty_output(b"MARK-C-51e0\r\n");
    let ids = registry_ids(&app);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    assert_eq!(app.active_session, Some(c));

    app.run_workspace_action(WorkspaceAction::SelectSession);
    assert_eq!(app.active_session, Some(a), "cycles forward and wraps");
    assert_eq!(app.workspace.registry().selected(), Some(a));
    assert!(terminal_text(app.terminal.as_ref().expect("a attached")).contains("MARK-A-51e0"));

    app.run_workspace_action(WorkspaceAction::SelectSession);
    assert_eq!(app.active_session, Some(b));
    assert!(terminal_text(app.terminal.as_ref().expect("b attached")).contains("MARK-B-51e0"));

    app.run_workspace_action(WorkspaceAction::SelectSession);
    assert_eq!(app.active_session, Some(c));
    let text = terminal_text(app.terminal.as_ref().expect("c attached"));
    assert!(text.contains("MARK-C-51e0"));
    assert!(
        !text.contains("MARK-A-51e0") && !text.contains("MARK-B-51e0"),
        "the live view shows only the selected session's screen"
    );
}

/// With a single live session, `session_select` keeps that session as the
/// input owner; a model-only row never takes the live view through the
/// palette either.
#[test]
fn palette_select_with_one_live_session_reaffirms_its_owner() {
    let home = AppTestHome::new();
    let mut app = home.app();
    app.run_workspace_action(WorkspaceAction::CreateSession);
    let live = registry_ids(&app)[0];
    let _model = app.workspace.create_session(SessionKind::Local);
    app.workspace.rebuild_sidebar();

    app.run_workspace_action(WorkspaceAction::SelectSession);

    assert_eq!(app.active_session, Some(live));
    assert_eq!(app.workspace.registry().selected(), Some(live));
    assert!(!has_live_surface(&app, _model));
}

/// Restart honesty: rows restored from disk have no live process behind them,
/// so selecting one must not move the live view — while the persisted
/// selection itself survives the restart untouched. Live PTYs never survive a
/// restart and the model never pretends otherwise.
#[test]
fn a_restored_row_never_takes_the_live_view_from_the_running_session() {
    let path = temp_state_path();
    // Launch one: one real session, quit (which persists it), relaunch.
    {
        let home = AppTestHome::new();
        let mut first_launch = NorenApp {
            test_pty_home: Some(home.0.clone()),
            ..app_with_state_path(&path)
        };
        first_launch.run_workspace_action(WorkspaceAction::CreateSession);
        first_launch.teardown();
    }
    let home = AppTestHome::new();
    let mut app = NorenApp {
        test_pty_home: Some(home.0.clone()),
        ..app_with_state_path(&path)
    };
    // The relaunched app has one Restored row and no live surface yet.
    let restored = registry_ids(&app)[0];
    assert_eq!(session_status(&app, restored), SessionStatus::Restored);
    assert_eq!(app.active_session, None);

    // The user creates a new real session; it takes the live view.
    app.run_workspace_action(WorkspaceAction::CreateSession);
    let live = registry_ids(&app)[1];
    assert_eq!(app.active_session, Some(live));

    // Clicking the Restored row is consumed but cannot take input ownership:
    // its shell died with the previous launch. The click still SELECTS the
    // clicked row — the palette's close command operates on the selected
    // row, and re-selecting the live one would redirect a close onto a shell
    // the user did not point at (found by independent review).
    assert!(
        click_sidebar_row(&mut app, 0),
        "the restored row is consumed"
    );
    assert_eq!(
        app.active_session,
        Some(live),
        "a restored row has no live surface to attach"
    );
    assert_eq!(
        app.workspace.registry().selected(),
        Some(restored),
        "the click selects the clicked row, not the live one"
    );
    assert!(app.pty.is_some(), "the live surface is untouched");
    // And closing now removes the clicked restored row, never the live shell.
    app.run_workspace_action(WorkspaceAction::CloseSession);
    assert!(
        app.workspace.registry().get(restored).is_none(),
        "close targets the row the user just clicked"
    );
    assert!(
        app.workspace.registry().get(live).is_some(),
        "the live shell survives closing the restored row"
    );
    cleanup_state_file(&path);
}

// ── Git worktree discovery and worktree sessions ────────────────────────
//
// The live fixtures drive the real `git` binary (never a scraped fixture of
// its output): every worktree case the ROADMAP names — not a repository, a
// single worktree, a registered-but-deleted directory, a detached HEAD, and
// paths with spaces or non-ASCII — is created with real git plumbing and
// discovered through the production `git worktree list --porcelain` child.
// Skip policy follows the live Zellij harness: when git is not usable the
// live tests print a visible skip notice on stderr and return early — a
// skip is never mistaken for gathered evidence.

static WT_SEQUENCE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Whether the real `git` binary is usable on this machine.
fn git_usable() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Print the visible skip notice shared by every live git test.
fn report_worktree_skip(test: &str) {
    let notice = format!(
        "SKIP [{test}]: git is not installed (or `git --version` failed); \
         live worktree evidence was NOT gathered. This is a skip, not a pass."
    );
    use std::io::Write;
    match std::fs::OpenOptions::new().write(true).open("/dev/stderr") {
        Ok(mut file) => {
            let _ = file.write_all(notice.as_bytes());
            let _ = file.write_all(b"\n");
        }
        Err(_) => eprintln!("{notice}"),
    }
}

/// A private git repository fixture driven through the real `git` binary.
///
/// `root` is the main worktree; linked worktrees are created on demand as
/// siblings under the fixture's private base directory. The initial commit
/// is made on a fixed branch name so no assertion depends on the machine's
/// `init.defaultBranch`. Identity is supplied with per-command `-c` config
/// so the developer's global git configuration is never read or required.
struct GitWorktreeFixture {
    /// Private base directory holding the main worktree and its links.
    base: PathBuf,
    /// The main worktree, also the repository root.
    root: PathBuf,
    /// Per-fixture branch counter so linked-branch names are deterministic
    /// (`wt-1`, `wt-2`, ...) regardless of process-wide test ordering.
    branch_sequence: std::cell::Cell<usize>,
}

impl GitWorktreeFixture {
    /// The main worktree on branch `noren-trunk` with one empty commit, or
    /// `None` when git is unusable (the caller reports the skip).
    fn new() -> Option<Self> {
        if !git_usable() {
            return None;
        }
        let sequence = WT_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!(
            "noren-app-wt-fixture-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&base).expect("create private worktree fixture base");
        let root = base.join("main");
        let root_text = root.display().to_string();
        Self::git(None, &["init", &root_text]).expect("git init the fixture repository");
        // Fixed branch name + per-command identity: independent of the
        // machine's init.defaultBranch and global git config.
        Self::git(Some(&root), &["checkout", "-b", "noren-trunk"])
            .expect("create the fixed fixture branch");
        Self::git(
            Some(&root),
            &[
                "-c",
                "user.name=Noren",
                "-c",
                "user.email=noren@example",
                "commit",
                "--allow-empty",
                "-m",
                "init",
            ],
        )
        .expect("create the initial fixture commit");

        Some(Self {
            base,
            root,
            branch_sequence: std::cell::Cell::new(0),
        })
    }

    /// Run one real `git` command, optionally inside `dir`.
    fn git(dir: Option<&std::path::Path>, args: &[&str]) -> std::io::Result<()> {
        let mut command = std::process::Command::new("git");
        if let Some(dir) = dir {
            command.current_dir(dir);
        }
        command.args(args);
        let output = command.output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other("git fixture command failed"))
        }
    }

    /// Add a linked worktree and return its path and branch name. The
    /// branch is named from the fixture's sequence (git refnames cannot
    /// carry spaces); the PATH deliberately may contain spaces and
    /// non-ASCII because git must round-trip them.
    fn add_worktree(&self, name: &str) -> (PathBuf, String) {
        let path = self.base.join(name);
        let branch = format!(
            "wt-{}",
            self.branch_sequence.replace(self.branch_sequence.get() + 1) + 1
        );
        let path_text = path.display().to_string();
        Self::git(
            Some(&self.root),
            &["worktree", "add", "-b", &branch, &path_text],
        )
        .unwrap_or_else(|error| panic!("add worktree fixture {name}: {error}"));
        (path, branch)
    }

    /// Add a linked worktree with a detached HEAD and return its path.
    fn add_detached_worktree(&self, name: &str) -> PathBuf {
        let path = self.base.join(name);
        let path_text = path.display().to_string();
        Self::git(
            Some(&self.root),
            &["worktree", "add", "--detach", &path_text],
        )
        .unwrap_or_else(|error| panic!("add detached worktree fixture {name}: {error}"));
        path
    }

    /// The worktree at `path` stays REGISTERED while its directory is
    /// deleted from disk — the common stale case until `git worktree prune`.
    /// Equivalent to the user's `rm -rf` of a scratch checkout: the whole
    /// directory (including the `.git` link file) goes, the registration in
    /// the main repository's metadata stays.
    fn delete_worktree_directory(&self, path: &std::path::Path) {
        std::fs::remove_dir_all(path).expect("delete the worktree directory only");
    }
}

impl Drop for GitWorktreeFixture {
    fn drop(&mut self) {
        // The linked worktrees are siblings under the base, so one removal
        // takes the whole fixture; failures are ignored because a stale
        // worktree directory can already be gone.
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

/// A temp directory that is NOT a git repository (no git child needed).
fn non_repository_directory() -> PathBuf {
    let sequence = WT_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "noren-app-not-a-repo-{}-{sequence}",
        std::process::id()
    ));
    std::fs::create_dir(&path).expect("create plain fixture directory");
    path
}

/// Discovered worktree rows appear as `EntryKind::Worktree` sidebar rows a
/// user SEES on launch: the main worktree first with its branch, then the
/// linked worktrees, branched or detached, with spaces and non-ASCII in
/// paths intact. A detached HEAD shows the fixed placeholder, never a panic.
#[test]
fn discovered_worktrees_appear_as_sidebar_rows() {
    let Some(fixture) = GitWorktreeFixture::new() else {
        report_worktree_skip("discovered_worktrees_appear_as_sidebar_rows");
        return;
    };
    let (plain_path, plain_branch) = fixture.add_worktree("plain");
    let (_spaced_path, spaced_branch) = fixture.add_worktree("spaced 名前 wt");
    let detached_path = fixture.add_detached_worktree("detached-only");

    let mut app = NorenApp::default();
    app.load_git_worktrees_from(&fixture.root);

    let rows = app.workspace.sidebar().rows();
    assert_eq!(rows.len(), 4, "the main worktree plus three linked ones");
    // The main worktree is always listed first; the order of the linked
    // worktrees is git's readdir order, so they are found by path, not
    // position. Git prints resolved paths (/private/var on macOS), so the
    // fixture paths are canonicalized before comparing. Worktree facts and
    // sidebar rows are index-aligned (the registry is empty).
    assert_eq!(rows[0].kind(), EntryKind::Worktree);
    assert_eq!(rows[0].label(), "main");
    assert_eq!(rows[0].detail(), Some("noren-trunk"));
    let row_of = |path: &std::path::Path| -> usize {
        let canonical = std::fs::canonicalize(path).expect("canonicalize fixture path");
        app.workspace
            .worktrees
            .iter()
            .position(|worktree| worktree.path() == canonical)
            .unwrap_or_else(|| panic!("no discovered worktree for {canonical:?}"))
    };
    assert_eq!(
        rows[row_of(&plain_path)].detail(),
        Some(plain_branch.as_str())
    );
    assert_eq!(
        rows[row_of(&detached_path)].detail(),
        Some("(detached)"),
        "a detached HEAD reports the fixed placeholder"
    );
    let spaced_index = app
        .workspace
        .worktrees
        .iter()
        .position(|worktree| worktree.name_display() == "spaced 名前 wt")
        .expect("a path with spaces and non-ASCII round-trips into discovery");
    assert_eq!(rows[spaced_index].label(), "spaced 名前 wt");
    assert_eq!(rows[spaced_index].detail(), Some(spaced_branch.as_str()));
    assert!(
        app.worktree_diagnostic.is_none(),
        "an in-cap discovery adds no notice"
    );
}

/// A launch directory outside any git repository is the common case: no
/// rows, no diagnostic, no panic — like a missing SSH config.
#[test]
fn a_launch_directory_outside_a_repository_is_silent() {
    let dir = non_repository_directory();
    let mut app = NorenApp::default();
    app.load_git_worktrees_from(&dir);
    assert_eq!(app.workspace.sidebar().rows().len(), 0);
    assert!(app.workspace.worktrees.is_empty());
    assert!(
        app.worktree_diagnostic.is_none(),
        "not-a-repository is silent, exactly like a missing SSH config"
    );
    let _ = std::fs::remove_dir(&dir);
}

/// A nonexistent launch directory must report `LaunchDirectoryUnavailable`,
/// not `GitUnavailable`: git may be installed, the directory is not. The
/// `discover_worktrees` function checks `is_dir()` before spawning git, so
/// a path that does not exist reaches `LaunchDirectoryUnavailable` through
/// the normal error path.
#[test]
fn a_nonexistent_launch_directory_reports_launch_directory_unavailable() {
    let nonexistent = std::env::temp_dir().join(format!(
        "noren-nonexistent-dir-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));

    let mut app = NorenApp::default();
    app.load_git_worktrees_from(&nonexistent);
    let diagnostic = app
        .worktree_diagnostic
        .as_deref()
        .expect("a nonexistent dir must produce a diagnostic");
    assert!(
        diagnostic.contains("launch directory"),
        "the diagnostic must mention the directory, not git: {diagnostic}"
    );
    assert!(
        !diagnostic.contains("git is unavailable"),
        "a nonexistent dir must not be reported as git unavailable: {diagnostic}"
    );
}
#[test]
fn a_single_worktree_repository_lists_one_row() {
    let Some(fixture) = GitWorktreeFixture::new() else {
        report_worktree_skip("a_single_worktree_repository_lists_one_row");
        return;
    };
    let mut app = NorenApp::default();
    app.load_git_worktrees_from(&fixture.root);
    let rows = app.workspace.sidebar().rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind(), EntryKind::Worktree);
    assert_eq!(rows[0].detail(), Some("noren-trunk"));
}

/// The stale case the ROADMAP names: a worktree whose directory was deleted
/// from disk but is still registered. Discovery must list it (marked
/// missing), the sidebar must show the marker, and selecting the row must
/// refuse the launch before any session or child exists — no panic, no
/// hang, an honest visible status.
///
/// Mutation check (B): making discovery panic on a missing directory (for
/// example an `.expect` on the directory's presence) fails this test.
#[test]
fn a_registered_but_deleted_worktree_is_marked_and_refused() {
    let Some(fixture) = GitWorktreeFixture::new() else {
        report_worktree_skip("a_registered_but_deleted_worktree_is_marked_and_refused");
        return;
    };
    let (stale, stale_branch) = fixture.add_worktree("gone-wt");
    // Resolve before deleting: the canonical path is what git prints (and
    // what discovery retains), but a deleted directory can no longer be
    // canonicalized through the filesystem.
    let stale_canonical =
        std::fs::canonicalize(&stale).expect("canonicalize the stale worktree path");
    fixture.delete_worktree_directory(&stale);
    let _live = fixture.add_worktree("present-wt");

    let mut app = NorenApp::default();
    // Discovery over a real registration whose directory is gone must not
    // panic (a panic here fails the test) and must not hang (the runner is
    // one bounded git listing).
    app.load_git_worktrees_from(&fixture.root);

    let worktrees = &app.workspace.worktrees;
    assert_eq!(
        worktrees.len(),
        3,
        "main, the stale link, and the live link"
    );
    let stale_index = worktrees
        .iter()
        .position(|worktree| worktree.path() == stale_canonical)
        .expect("the stale registration is still listed");
    let stale_row = &worktrees[stale_index];
    assert!(!stale_row.directory_present());
    assert_eq!(
        stale_row.branch_display(),
        format!("{stale_branch} (missing)")
    );
    assert!(
        worktrees
            .iter()
            .any(|worktree| worktree.directory_present()),
        "the live link is still present"
    );

    // The sidebar row the user sees carries the missing marker.
    let missing_detail = format!("{stale_branch} (missing)");
    let rows = app.workspace.sidebar().rows();
    assert_eq!(rows[stale_index].detail(), Some(missing_detail.as_str()));

    // Selecting it refuses the launch: no session, no PTY, a visible status.
    assert!(click_sidebar_row(&mut app, stale_index));
    assert_eq!(app.status, "Noren worktree directory missing");
    assert!(app.show_status, "the refusal must own the status row");
    assert!(app.workspace.registry().sessions().is_empty());
    assert!(app.pty.is_none(), "a stale worktree must not spawn a child");
}

/// A repository with more worktrees than the sidebar cap keeps the first
/// rows in git listing order and reports the cap and the omitted count,
/// exactly like the bounded SSH host list.
#[test]
fn many_worktrees_are_capped_and_the_omitted_count_is_reported() {
    let Some(fixture) = GitWorktreeFixture::new() else {
        report_worktree_skip("many_worktrees_are_capped_and_the_omitted_count_is_reported");
        return;
    };
    // main + (cap + 2) linked worktrees = cap + 3 total; 3 are omitted.
    for index in 0..(MAX_WORKTREE_SIDEBAR_TEST_ROWS + 2) {
        fixture.add_worktree(&format!("capped-{index:02}"));
    }
    let mut app = NorenApp::default();
    app.load_git_worktrees_from(&fixture.root);

    let rows = app.workspace.sidebar().rows();
    assert_eq!(
        rows.len(),
        MAX_WORKTREE_SIDEBAR_TEST_ROWS,
        "the sidebar keeps exactly the capped row count"
    );
    assert_eq!(
        rows[0].label(),
        "main",
        "the main worktree is always retained"
    );
    assert_eq!(
        app.workspace.worktrees_omitted, 3,
        "beyond-cap worktrees are counted, not silently dropped"
    );
    let notice = app
        .worktree_diagnostic
        .as_deref()
        .expect("the cap is reported on the status row");
    assert!(
        notice.contains("showing first 24") && notice.contains("3 omitted"),
        "the notice names the cap and the omitted count: {notice}"
    );
}

/// Bound constant for the live cap test; must equal the shipped cap.
const MAX_WORKTREE_SIDEBAR_TEST_ROWS: usize = noren_app::git_worktree::MAX_WORKTREE_SIDEBAR_ROWS;

/// Selecting a worktree row must start a session whose working directory IS
/// that worktree — verified by reading the child's own `pwd` answer back
/// through the terminal, never by trusting the code path that set the cwd.
///
/// The child's `HOME` is isolated through the same [`AppTestHome`] seam as
/// every other shell-driving test (production inherits `HOME` unchanged): a
/// developer's `.zshrc` cannot stall the prompt past the deadline, while the
/// working directory stays the worktree, so the `pwd` proof is unaffected.
///
/// Mutation check (A): breaking the launch so the child starts elsewhere
/// (for example dropping the cwd from the launch policy) fails this test.
#[cfg(target_os = "macos")]
#[test]
fn selecting_a_worktree_row_starts_a_session_in_that_worktree() {
    let Some(fixture) = GitWorktreeFixture::new() else {
        report_worktree_skip("selecting_a_worktree_row_starts_a_session_in_that_worktree");
        return;
    };
    let (worktree, _branch) = fixture.add_worktree("launch-wt");
    let canonical = std::fs::canonicalize(&worktree).expect("canonicalize the worktree path");
    let home = AppTestHome::new();
    let mut app = NorenApp {
        test_pty_home: Some(home.0.clone()),
        ..NorenApp::default()
    };
    app.load_git_worktrees_from(&fixture.root);

    // Row 0 is the main worktree; find the linked worktree's row by path.
    let row = app
        .workspace
        .worktrees
        .iter()
        .position(|worktree| worktree.path() == canonical)
        .expect("the linked worktree is discovered");
    assert!(
        click_sidebar_row(&mut app, row),
        "the worktree row is consumed"
    );

    // The launch created a real Worktree-kind session that owns the live
    // surface and is observed Running.
    let ids = registry_ids(&app);
    assert_eq!(ids.len(), 1, "one registry session from the worktree row");
    let id = ids[0];
    assert_eq!(
        app.workspace.registry().get(id).map(|d| d.kind().clone()),
        Some(SessionKind::Worktree {
            path: canonical.clone()
        }),
        "the session's launch shape carries the worktree path"
    );
    assert_eq!(session_status(&app, id), SessionStatus::Running);
    assert_eq!(app.active_session, Some(id));
    assert!(app.pty.is_some(), "the worktree session owns a real PTY");

    // Read the child's ACTUAL working directory back: wait for the shell,
    // ask it, and search the terminal text for its own answer. The path the
    // session claims and the directory the child reports must agree. The
    // grid wraps long lines across rows, so the comparison strips the row
    // separators a wrap inserts — a wrapped path must still match whole.
    wait_for_shell_output(&mut app);
    app.send_input(b"pwd\n");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.drain_pty();
        let answered = app.terminal.as_ref().is_some_and(|terminal| {
            let unwrapped = terminal_text(terminal).replace(['\r', '\n'], "");
            unwrapped.contains(canonical.display().to_string().as_str())
        });
        if answered {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the child's own pwd never reported the worktree directory\n\
             expected: the terminal text to contain the worktree path {}\n\
             received: terminal said: {}",
            canonical.display(),
            app.terminal.as_ref().map(terminal_text).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Quitting with a worktree session and a local session persists BOTH
/// through the real sessions.toml path, and the next launch restores them —
/// the worktree kind round-trips with its path, and local-session
/// persistence is unharmed. This is also the direct guard that the
/// worktree launch path did not reintroduce the historic quit-path erase:
/// teardown must save exactly the sessions it had.
#[test]
fn quitting_persists_worktree_sessions_beside_local_sessions() {
    let Some(fixture) = GitWorktreeFixture::new() else {
        report_worktree_skip("quitting_persists_worktree_sessions_beside_local_sessions");
        return;
    };
    let (worktree, _branch) = fixture.add_worktree("persist-wt");
    let canonical = std::fs::canonicalize(&worktree).expect("canonicalize the worktree path");
    let path = temp_state_path();
    let home = AppTestHome::new();
    let mut app = NorenApp {
        test_pty_home: Some(home.0.clone()),
        ..app_with_state_path(&path)
    };
    app.load_git_worktrees_from(&fixture.root);

    // One local session (palette) and one worktree session (row click).
    app.run_workspace_action(WorkspaceAction::CreateSession);
    let row = app
        .workspace
        .worktrees
        .iter()
        .position(|worktree| worktree.path() == canonical)
        .expect("the worktree is discovered before the click");
    // Row 0 is the local session; the worktree rows are offset by it.
    assert!(
        click_sidebar_row(&mut app, 1 + row),
        "the worktree row launches"
    );
    let ids = registry_ids(&app);
    assert_eq!(ids.len(), 2);
    assert_eq!(session_status(&app, ids[0]), SessionStatus::Running);
    assert_eq!(session_status(&app, ids[1]), SessionStatus::Running);

    app.teardown();

    let text = std::fs::read_to_string(&path).expect("state saved on quit");
    assert_eq!(
        text.matches("kind = \"local\"").count(),
        1,
        "the local session still persists: {text}"
    );
    assert_eq!(
        text.matches("kind = \"worktree\"").count(),
        1,
        "the worktree session persists with its kind: {text}"
    );
    assert!(
        text.contains("path = "),
        "the worktree entry carries its path for restoration: {text}"
    );

    // The next launch restores both rows as Restored with intact kinds —
    // the worktree path survives the round-trip byte for byte.
    let relaunched = sidebar_after_relaunch(&path);
    assert_eq!(relaunched.registry().len(), 2);
    let kinds: Vec<SessionKind> = relaunched
        .registry()
        .sessions()
        .iter()
        .map(|descriptor| descriptor.kind().clone())
        .collect();
    assert_eq!(
        kinds,
        vec![
            SessionKind::Local,
            SessionKind::Worktree {
                path: canonical.clone()
            },
        ]
    );
    for descriptor in relaunched.registry().sessions() {
        assert_eq!(
            descriptor.status(),
            &SessionStatus::Restored,
            "a relaunched row must be Restored, never a phantom Running"
        );
    }
    cleanup_state_file(&path);
}

/// A worktree path can embed a username or a private directory name. Every
/// debug and status surface the discovery/launch flow can reach must stay
/// free of it. The persisted sessions.toml file deliberately DOES carry the
/// path — it is the user's own private state and exactly what restoration
/// needs, the same class as a project root; the redaction discipline
/// governs Debug and error output, not the state file (SSH destinations, by
/// contrast, may embed credentials and so never enter the registry at all).
#[test]
fn worktree_paths_never_reach_debug_or_status_surfaces() {
    let secret = format!("NOREN-WT-hunter2-{}", std::process::id());
    let Some(fixture) = GitWorktreeFixture::new() else {
        report_worktree_skip("worktree_paths_never_reach_debug_or_status_surfaces");
        return;
    };
    let (worktree, _branch) = fixture.add_worktree(&secret);
    let canonical = std::fs::canonicalize(&worktree).expect("canonicalize the worktree path");
    let mut app = NorenApp::default();
    app.load_git_worktrees_from(&fixture.root);
    assert!(
        canonical.display().to_string().contains(&secret),
        "fixture self-check: the secret is really in the path"
    );

    // The workspace (sidebar rows, worktree facts) never prints it.
    assert!(
        !format!("{:?}", app.workspace).contains(&secret),
        "workspace debug leaked the worktree path"
    );
    // Status surfaces carry fixed text only.
    if let Some(notice) = app.worktree_diagnostic.as_deref() {
        assert!(!notice.contains(&secret), "notice leaked: {notice}");
    }

    // A registry session of this shape (the state a launch or a restore
    // leaves behind) never prints it through the descriptor either.
    let id = app
        .workspace
        .create_session(SessionKind::Worktree { path: canonical });
    let rendered = format!("{:?}", app.workspace.registry().get(id));
    assert!(
        !rendered.contains(&secret),
        "descriptor debug leaked the worktree path: {rendered}"
    );
    assert!(
        rendered.contains("Worktree"),
        "not vacuous — the descriptor still names its shape: {rendered}"
    );
    assert!(
        !format!("{:?}", app.workspace).contains(&secret),
        "workspace debug leaked after the session was created"
    );
}

// ── Mixed sidebars: every row kind present at once ────────────────────
//
// Independent review (issue #156 follow-up): the click path resolves each
// row kind by subtracting the row counts of the kinds above it
// (`local_sidebar_session`, `worktree_sidebar_row`, `select_ssh_sidebar_row`),
// and every existing test exercised one row kind in isolation — an index
// arithmetic error (the `MouseGrid::new(rows, cols)` class) passed the whole
// suite. A mixed sidebar is not an edge case: it is the DEFAULT state of
// this repository's own development environment.

/// Real directories for synthetic worktree facts. Discovery parsing is pure,
/// but a worktree row click launches a session rooted at the row's
/// directory, so the directories must exist for the launch to identify the
/// clicked row by its path.
fn mixed_worktree_directories(count: usize) -> Vec<PathBuf> {
    (0..count)
        .map(|index| {
            let sequence = WT_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "noren-app-mixed-wt-{}-{sequence}-{index}",
                std::process::id()
            ));
            std::fs::create_dir(&path).expect("create mixed-sidebar worktree directory");
            path
        })
        .collect()
}

/// `git worktree list --porcelain` text for the given directories, one
/// branched record each, in listing order.
fn synthetic_porcelain(paths: &[PathBuf]) -> String {
    let mut text = String::new();
    for (index, path) in paths.iter().enumerate() {
        text.push_str(&format!(
            "worktree {}\nbranch refs/heads/mix-{index:02}\n\n",
            path.display()
        ));
    }
    text
}

/// The paths of every registry session launched from a worktree row, in
/// registry order — the observable identity of WHICH worktree row a click
/// selected.
fn worktree_session_paths(app: &NorenApp) -> Vec<PathBuf> {
    app.workspace
        .registry()
        .sessions()
        .iter()
        .filter_map(|descriptor| match descriptor.kind() {
            SessionKind::Worktree { path } => Some(path.clone()),
            _ => None,
        })
        .collect()
}

/// Row order is FIXED — sessions first, then discovered worktree facts, then
/// configured SSH hosts, then configured agents — and the click-path offset
/// arithmetic silently depends on it, so the rendered order is pinned here
/// with all four kinds present at once, including each kind's internal order
/// (registry order, git listing order, config order). Agent-kind sessions
/// render as ordinary session rows; a configured agent row (`EntryKind::Agent`)
/// appears only for `[[agents]]` configuration entries.
#[test]
fn sidebar_rows_render_sessions_then_worktrees_then_ssh_hosts_then_agents() {
    let fixture = SshConfigFixture::new();
    fixture.write_new(b"Host zulu\nHost alpha\n");
    let records = git_worktree::parse_worktree_porcelain(
        "worktree /srv/mix-main\nbranch refs/heads/mix-main\n\
         \nworktree /srv/mix-a\nbranch refs/heads/mix-a\n\
         \nworktree /srv/mix-b\ndetached\n",
    )
    .expect("synthetic porcelain parses");
    let config = AppConfig::parse(
        "[[agents]]\nname = \"m-one\"\ncommand = \"/bin/true\"\n\
         [[agents]]\nname = \"m-two\"\ncommand = \"/bin/true\"\n",
    )
    .expect("valid agents configuration");

    let mut app = NorenApp::new(config);
    app.workspace
        .load_worktrees(WorktreeDiscovery::from_records(records));
    app.load_ssh_hosts_from(fixture.path());
    let _agent = app.workspace.create_session(SessionKind::Agent {
        name: "reserved".to_owned(),
    });
    let _local = app.workspace.create_session(SessionKind::Local);
    app.workspace.rebuild_sidebar();

    let rows = app.workspace.sidebar().rows();
    let kinds: Vec<EntryKind> = rows.iter().map(|row| row.kind()).collect();
    assert_eq!(
        kinds,
        vec![
            EntryKind::Session,
            EntryKind::Session,
            EntryKind::Worktree,
            EntryKind::Worktree,
            EntryKind::Worktree,
            EntryKind::SshConnection,
            EntryKind::SshConnection,
            EntryKind::Agent,
            EntryKind::Agent,
        ],
        "the fixed order is sessions, worktrees, SSH hosts, then agents"
    );
    // Worktree rows keep git's listing order (main first).
    assert_eq!(
        rows[2..5]
            .iter()
            .map(|row| row.label().to_owned())
            .collect::<Vec<_>>(),
        vec!["mix-main", "mix-a", "mix-b"]
    );
    // SSH rows keep the config's declaration order.
    assert_eq!(
        rows[5..7]
            .iter()
            .map(|row| row.label().to_owned())
            .collect::<Vec<_>>(),
        vec!["SSH-OFF zulu", "SSH-OFF alpha"]
    );
    // Agent rows keep the configuration's declaration order (short names:
    // the label target budget is six characters, like an SSH target's).
    assert_eq!(
        rows[7..9]
            .iter()
            .map(|row| row.label().to_owned())
            .collect::<Vec<_>>(),
        vec!["AGT-OFF m-one", "AGT-OFF m-two"]
    );
    assert_eq!(rows[7].detail(), Some("not running"));
    assert!(
        app.agent_diagnostic.is_none(),
        "an in-cap agent list adds no notice"
    );
}

/// Clicking each row kind in a mixed sidebar must select the row the user
/// actually clicked — the default sidebar shape of this repository, which
/// holds sessions, worktrees, SSH hosts, and configured agents at once. The
/// boundaries are asserted explicitly: the last worktree row, the first SSH
/// row, the last SSH row, the first and last agent rows, and the row
/// immediately after the last agent row (a dead click that must select
/// nothing, not wrap).
///
/// Mutation checks:
/// - dropping `+ worktree_rows` from `select_ssh_sidebar_row` makes the
///   first-SSH-row click resolve to a DIFFERENT host (the host list here is
///   longer than the worktree list, so the misresolved index lands on a
///   real host, not a dead click);
/// - dropping `+ self.ssh_hosts.len()` from `agent_sidebar_row` (the agent
///   block's offset arithmetic) makes the first-agent-row click a dead
///   click or an SSH connect instead of an agent launch;
/// - dropping the session-row offset from `worktree_sidebar_row` resolves a
///   worktree click to another worktree's row;
/// - dropping the session bound from `local_sidebar_session` makes
///   non-session rows claim a session id.
#[cfg(target_os = "macos")]
#[test]
fn mixed_sidebar_clicks_select_the_row_the_user_clicked() {
    // Five SSH hosts — more than the three worktrees — so a misresolved SSH
    // index lands on a real, different host instead of dead-clicking.
    let fixture = SshConfigFixture::new();
    fixture.write_new(b"Host alpha\nHost bravo\nHost charlie\nHost delta\nHost echo\n");
    let worktree_paths = mixed_worktree_directories(3);
    let records = git_worktree::parse_worktree_porcelain(&synthetic_porcelain(&worktree_paths))
        .expect("synthetic porcelain parses");
    // Two configured agents whose launches are real, cheap, and immediate.
    let config = AppConfig::parse(&format!(
        "[[agents]]\nname = \"mix-agent-one\"\ncommand = \"/bin/echo\"\nargs = [\"{}\"]\n\
         [[agents]]\nname = \"mix-agent-two\"\ncommand = \"/bin/echo\"\nargs = [\"{}\"]\n",
        "MIX-AGENT-ONE", "MIX-AGENT-TWO"
    ))
    .expect("valid agents configuration");

    // The deterministic seam keeps SSH row clicks from spawning any process;
    // worktree and agent row clicks really launch (that is their observable
    // identity).
    let mut app = app_with_deterministic_ssh_seam_and_config(config);
    app.workspace
        .load_worktrees(WorktreeDiscovery::from_records(records));
    app.load_ssh_hosts_from(fixture.path());
    let sessions = [
        app.workspace.create_session(SessionKind::Local),
        app.workspace.create_session(SessionKind::Project {
            root: PathBuf::from("/srv/noren"),
        }),
    ];

    // Layout: rows 0-1 sessions, rows 2-4 worktrees, rows 5-9 SSH hosts,
    // rows 10-11 configured agents.
    assert_eq!(
        app.workspace.sidebar().rows().len(),
        12,
        "two session rows, three worktree rows, five SSH rows, two agent rows"
    );

    // Session rows select their own session, first and last alike.
    assert!(click_sidebar_row(&mut app, 0), "the first session row");
    assert_eq!(app.workspace.registry().selected(), Some(sessions[0]));
    assert!(click_sidebar_row(&mut app, 1), "the last session row");
    assert_eq!(app.workspace.registry().selected(), Some(sessions[1]));

    // First SSH row (row 5): selects the FIRST host, and never moves the
    // registry selection.
    assert!(click_sidebar_row(&mut app, 5), "the first SSH row");
    assert_eq!(app.workspace.selected_ssh_target(), Some("alpha"));
    assert_eq!(
        app.workspace.registry().selected(),
        Some(sessions[1]),
        "an SSH row click never selects a registry session"
    );
    // Last SSH row (row 9): selects the LAST host.
    assert!(click_sidebar_row(&mut app, 9), "the last SSH row");
    assert_eq!(app.workspace.selected_ssh_target(), Some("echo"));

    // First agent row (row 10): launches the FIRST configured agent — the
    // created session's Agent name is the observable identity of WHICH row
    // the click resolved to.
    assert!(click_sidebar_row(&mut app, 10), "the first agent row");
    assert_eq!(
        agent_session_names(&app),
        vec!["mix-agent-one".to_owned()],
        "the click launches the agent the user pointed at"
    );

    // Each launch adds a session row, shifting the rows below: with three
    // sessions the agent block occupies rows 11-12. Last agent row.
    assert!(click_sidebar_row(&mut app, 12), "the last agent row");
    assert_eq!(
        agent_session_names(&app),
        vec!["mix-agent-one".to_owned(), "mix-agent-two".to_owned()],
        "the last agent row launches the second agent, not the first again"
    );
    assert!(
        agent_session_names(&app)[0] != agent_session_names(&app)[1],
        "the two agent clicks resolved to two DIFFERENT agents"
    );

    // Row 14 — immediately after the last agent row (with four sessions the
    // layout is sessions 0-3, worktrees 4-6, SSH 7-11, agents 12-13) —
    // selects nothing: not a session, not a host, not an agent, no
    // wrap-around, and no state changes at all. (The agent launches already
    // cleared the pending SSH choice by selecting their own sessions, so
    // the invariant here is that a dead click changes NOTHING.)
    let registry_len = app.workspace.registry().len();
    let selected = app.workspace.registry().selected();
    assert!(
        !click_sidebar_row(&mut app, 14),
        "the row after the last agent row is a dead click"
    );
    assert_eq!(
        app.workspace.registry().selected(),
        selected,
        "a dead click must not move the registry selection"
    );
    assert_eq!(
        app.workspace.registry().len(),
        registry_len,
        "a dead click must not create a session"
    );

    // Last worktree row (row 6 with four sessions): launches a session
    // rooted at the THIRD worktree — the row the user clicked, not the one
    // an offset error would resolve to.
    assert!(click_sidebar_row(&mut app, 6), "the last worktree row");
    assert_eq!(
        worktree_session_paths(&app),
        vec![worktree_paths[2].clone()],
        "the click launches the worktree the user pointed at"
    );

    // With five sessions the worktree block occupies rows 5-7. First
    // worktree row.
    assert!(click_sidebar_row(&mut app, 5), "the first worktree row");
    assert_eq!(
        worktree_session_paths(&app),
        vec![worktree_paths[2].clone(), worktree_paths[0].clone()]
    );

    // With six sessions the worktree block occupies rows 6-8; the middle
    // worktree is row 7.
    assert!(click_sidebar_row(&mut app, 7), "a middle worktree row");
    assert_eq!(
        worktree_session_paths(&app),
        vec![
            worktree_paths[2].clone(),
            worktree_paths[0].clone(),
            worktree_paths[1].clone()
        ]
    );

    // After the launches the accumulated counts changed (seven sessions),
    // so the SSH and agent boundaries are re-asserted at their new offsets:
    // sessions 0-6, worktrees 7-9, SSH 10-14, agents 15-16.
    assert_eq!(
        app.workspace.sidebar().rows().len(),
        17,
        "seven session rows, three worktree rows, five SSH rows, two agent rows"
    );
    assert!(click_sidebar_row(&mut app, 10), "the first SSH row again");
    assert_eq!(
        app.workspace.selected_ssh_target(),
        Some("alpha"),
        "the SSH offset must track the grown session block"
    );
    assert!(
        !click_sidebar_row(&mut app, 17),
        "the row after the last agent row stays dead after the shifts"
    );

    for path in &worktree_paths {
        let _ = std::fs::remove_dir(path);
    }
}

/// The names of every registry session launched from an agent row, in
/// registry order — the observable identity of WHICH agent row a click
/// selected.
fn agent_session_names(app: &NorenApp) -> Vec<String> {
    app.workspace
        .registry()
        .sessions()
        .iter()
        .filter_map(|descriptor| match descriptor.kind() {
            SessionKind::Agent { name } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

// ── Configured agents: rows, launches, failures, persistence ────────────
//
// The `[[agents]]` configuration slice (M5): configured agents are sidebar
// facts until a row is selected; selecting launches the configured command
// as a shell-free argv vector in a PTY (see `noren_pty::AgentLaunchPolicy`),
// exactly like a local or SSH session; a missing or non-executable command
// is a visible failure on the row, never a hang and never a silent no-op.

/// Config text for `count` uniquely named agents running the same program
/// with one distinguishing argument. Names stay inside the six-character
/// label target budget so cap assertions read exact labels.
fn many_agents_config(count: usize, command: &str) -> AppConfig {
    let mut text = String::new();
    for index in 0..count {
        text.push_str(&format!(
            "[[agents]]\nname = \"ag-{index:02}\"\ncommand = \"{command}\"\nargs = [\"a-{index:02}\"]\n"
        ));
    }
    AppConfig::parse(&text).expect("synthetic agents configuration parses")
}

/// Configured agents appear as `EntryKind::Agent` rows in configuration
/// order, with bounded labels: a name longer than the target budget is
/// truncated with the shared ASCII marker, never overflowing the sidebar
/// column.
#[test]
fn configured_agents_appear_as_sidebar_rows_with_bounded_labels() {
    let config = AppConfig::parse(
        "[[agents]]\nname = \"claude\"\ncommand = \"/bin/true\"\n\
         [[agents]]\nname = \"a-very-long-agent-name-that-exceeds-the-budget\"\n\
         command = \"/bin/true\"\n",
    )
    .expect("valid configuration");
    let app = NorenApp::new(config);

    let rows = app.workspace.sidebar().rows();
    let kinds: Vec<EntryKind> = rows.iter().map(|row| row.kind()).collect();
    assert_eq!(kinds, vec![EntryKind::Agent, EntryKind::Agent]);
    assert_eq!(rows[0].label(), "AGT-OFF claude");
    assert_eq!(rows[0].detail(), Some("not running"));
    // The label budget mirrors the SSH target budget (six characters after
    // the state prefix): three kept characters plus the shared ASCII
    // truncation marker — never the full overlong name.
    assert_eq!(
        rows[1].label(),
        "AGT-OFF a-v...",
        "an overlong name is truncated with the shared marker: {}",
        rows[1].label()
    );
    assert!(app.workspace.agents_omitted == 0);
}

/// More configured agents than the sidebar cap keep the first rows in
/// configuration order and report the cap and the omitted count on the
/// status row, exactly like the bounded SSH host and worktree lists.
#[test]
fn many_configured_agents_are_capped_and_the_omitted_count_is_reported() {
    let config = many_agents_config(MAX_AGENT_SIDEBAR_ROWS + 2, "/bin/true");
    let mut app = NorenApp::new(config);

    let rows = app.workspace.sidebar().rows();
    assert_eq!(
        rows.len(),
        MAX_AGENT_SIDEBAR_ROWS,
        "the sidebar keeps exactly the capped row count"
    );
    assert_eq!(rows[0].label(), "AGT-OFF ag-00");
    assert_eq!(
        rows[MAX_AGENT_SIDEBAR_ROWS - 1].label(),
        format!("AGT-OFF ag-{:02}", MAX_AGENT_SIDEBAR_ROWS - 1),
        "the retained block is the FIRST {} in configuration order",
        MAX_AGENT_SIDEBAR_ROWS
    );
    assert_eq!(
        app.workspace.agents_omitted, 2,
        "beyond-cap agents are counted, not silently dropped"
    );
    let notice = app
        .agent_diagnostic
        .as_deref()
        .expect("the cap is reported on the status row");
    assert!(
        notice.contains("showing first 24") && notice.contains("2 omitted"),
        "the notice names the cap and the omitted count: {notice}"
    );
    // The notice owns the status row whenever no runtime status shows.
    app.show_status = false;
    assert_eq!(app.status_row(), StatusRowSource::AgentDiagnostic);
    assert_eq!(
        app.status_row().text(
            app.status,
            app.ssh_selection_status.as_deref(),
            app.worktree_diagnostic.as_deref(),
            app.agent_diagnostic.as_deref(),
            app.ssh_diagnostic.as_deref(),
        ),
        notice
    );
}

/// Selecting an agent row must START the configured command — verified by
/// reading the child's own output back through the terminal and its reaped
/// exit code, never by trusting the code path that spawned it.
///
/// Mutation checks: making the click add a model row without spawning (no
/// PTY, no `Running` observation), or pointing argv at the wrong agent,
/// fails the assertions below.
#[cfg(target_os = "macos")]
#[test]
fn selecting_an_agent_row_launches_the_configured_command_in_a_pty() {
    let marker = format!("NOREN-AGENT-OUTPUT-{}", std::process::id());
    let config = AppConfig::parse(&format!(
        "[[agents]]\nname = \"echo-agent\"\ncommand = \"/bin/echo\"\nargs = [\"{marker}\"]\n"
    ))
    .expect("valid configuration");
    let mut app = NorenApp::new(config);
    assert!(click_sidebar_row(&mut app, 0), "the agent row is consumed");

    // The launch created a real Agent-kind session that owns the live
    // surface and is observed Running.
    let ids = registry_ids(&app);
    assert_eq!(ids.len(), 1, "one registry session from the agent row");
    let id = ids[0];
    assert_eq!(
        app.workspace.registry().get(id).map(|d| d.kind().clone()),
        Some(SessionKind::Agent {
            name: "echo-agent".to_owned()
        }),
        "the session's launch shape carries the configured agent name"
    );
    assert_eq!(session_status(&app, id), SessionStatus::Running);
    assert_eq!(app.active_session, Some(id));
    assert!(app.pty.is_some(), "the agent session owns a real PTY");

    // Read the child's ACTUAL output back: the marker /bin/echo printed is
    // the evidence the configured command ran.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.drain_pty();
        let answered = app
            .terminal
            .as_ref()
            .is_some_and(|terminal| terminal_text(terminal).contains(&marker));
        if answered {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the agent child never produced its marker output\n\
             expected: terminal text containing {marker}\n\
             received: terminal said: {}",
            app.terminal.as_ref().map(terminal_text).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    // And the child's exit is observed through the reaping path: the
    // session row honestly reports `Exited`, never a stuck `Running`. The
    // exact code shape depends on a pre-existing drain race — the reader may
    // deliver EOF before the supervisor's reaped exit event (the same race
    // the SSH path's EOF grace handles), which records `Exited { code:
    // None }` — so the assertion pins terminal honesty, not the code value.
    // The marker output above is the child-started evidence.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.drain_pty();
        match session_status(&app, id) {
            SessionStatus::Exited { .. } => break,
            SessionStatus::Running => {}
            other => panic!("unexpected session status for /bin/echo: {other:?}"),
        }
        assert!(
            Instant::now() < deadline,
            "the /bin/echo child was never reaped; status: {:?}",
            session_status(&app, id)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A configured command that does not exist is a clear, visible failure on
/// the row — the configured row shows `AGT-ERR`, the created session row
/// shows `failed`, and the fixed status line owns the status row — never a
/// hang and never a silent no-op.
///
/// Mutation check (B): making a missing command report success (observe
/// `Running`, skip the status line, or skip the row marker) fails every
/// assertion past the click.
#[cfg(target_os = "macos")]
#[test]
fn an_agent_row_with_a_missing_command_is_a_visible_failure_never_a_hang() {
    let config = AppConfig::parse(&format!(
        "[[agents]]\nname = \"ghost\"\ncommand = \"/noren-missing-agent-{}\"\n",
        std::process::id()
    ))
    .expect("valid configuration");
    let mut app = NorenApp::new(config);

    // The click returns promptly (a hang fails the test runner's deadline);
    // nothing was spawned.
    assert!(
        click_sidebar_row(&mut app, 0),
        "the agent row is consumed even when the launch fails"
    );

    // The failure is first-class on every surface a user can see.
    assert_eq!(app.status, "Noren agent launch failed");
    assert!(app.show_status, "the launch failure must be visible");
    let rows = app.workspace.sidebar().rows();
    // Session rows precede the configured agent rows: the failed launch
    // inserted its session row at position 0.
    assert_eq!(rows[1].label(), "AGT-ERR ghost");
    assert_eq!(rows[1].detail(), Some("launch failed"));
    // The created session row reports `failed`, never a phantom `Running`.
    let ids = registry_ids(&app);
    assert_eq!(
        ids.len(),
        1,
        "the failed launch still creates its session row"
    );
    assert_eq!(
        session_status(&app, ids[0]),
        SessionStatus::Failed {
            reason: "PTY spawn failed".to_owned()
        }
    );
    assert_eq!(rows[0].kind(), EntryKind::Session);
    assert_eq!(rows[0].detail(), Some("agent · failed"));
    assert!(
        app.pty.is_none(),
        "a failed launch must leave no PTY behind"
    );
}

/// A failed agent launch must not tear down a running local session (the
/// SSH slice found this class of bug by independent review).
#[cfg(target_os = "macos")]
#[test]
fn an_agent_launch_failure_never_retires_the_running_local_session() {
    let home = AppTestHome::new();
    let config = AppConfig::parse(&format!(
        "[[agents]]\nname = \"ghost\"\ncommand = \"/noren-missing-agent-{}\"\n",
        std::process::id()
    ))
    .expect("valid configuration");
    let mut app = NorenApp {
        test_pty_home: Some(home.0.clone()),
        ..NorenApp::new(config)
    };
    app.run_workspace_action(WorkspaceAction::CreateSession);
    let live = registry_ids(&app)[0];
    assert!(app.pty.is_some(), "a live local session is running");

    // Row 0 is the live local session; the agent row sits at row 1.
    assert!(click_sidebar_row(&mut app, 1), "the agent row is consumed");

    assert_eq!(
        app.status, "Noren agent launch failed",
        "the failure is surfaced first-class"
    );
    assert!(
        app.pty.is_some(),
        "a failed agent launch must not tear down the running local shell"
    );
    assert_eq!(
        app.active_session,
        Some(live),
        "the local session keeps the live view"
    );
    assert_eq!(
        session_status(&app, live),
        SessionStatus::Running,
        "the local session is not observed Exiting behind a failed launch"
    );
}

/// Quitting with an agent session and a local session persists BOTH through
/// the real sessions.toml path, and the next launch restores them — the
/// agent kind round-trips with its name, local-session persistence is
/// unharmed, and the configured COMMAND never reaches the state file (only
/// the session's display name persists, the same class as a worktree path).
#[cfg(target_os = "macos")]
#[test]
fn quitting_persists_agent_sessions_beside_local_sessions() {
    let path = temp_state_path();
    let home = AppTestHome::new();
    let config = AppConfig::parse(
        "[[agents]]\nname = \"persist-agent\"\ncommand = \"/bin/echo\"\nargs = [\"NOREN-PERSIST\"]\n",
    )
    .expect("valid configuration");
    let mut app = NorenApp {
        test_pty_home: Some(home.0.clone()),
        ..app_with_deterministic_ssh_seam_and_config(config)
    };
    app.workspace.state_path = Some(path.clone());

    // One local session (palette) and one agent session (row click).
    app.run_workspace_action(WorkspaceAction::CreateSession);
    assert!(click_sidebar_row(&mut app, 1), "the agent row launches");
    let ids = registry_ids(&app);
    assert_eq!(ids.len(), 2);
    assert_eq!(session_status(&app, ids[0]), SessionStatus::Running);
    assert_eq!(session_status(&app, ids[1]), SessionStatus::Running);

    app.teardown();

    let text = std::fs::read_to_string(&path).expect("state saved on quit");
    assert_eq!(
        text.matches("kind = \"local\"").count(),
        1,
        "the local session still persists: {text}"
    );
    assert_eq!(
        text.matches("kind = \"agent\"").count(),
        1,
        "the agent session persists with its kind: {text}"
    );
    assert!(
        text.contains("name = \"persist-agent\""),
        "the agent entry carries its display name for restoration: {text}"
    );
    assert!(
        !text.contains("/bin/echo") && !text.contains("NOREN-PERSIST"),
        "the configured command and args never reach sessions.toml: {text}"
    );

    // The next launch restores both rows as Restored with intact kinds.
    let relaunched = sidebar_after_relaunch(&path);
    assert_eq!(relaunched.registry().len(), 2);
    let kinds: Vec<SessionKind> = relaunched
        .registry()
        .sessions()
        .iter()
        .map(|descriptor| descriptor.kind().clone())
        .collect();
    assert_eq!(
        kinds,
        vec![
            SessionKind::Local,
            SessionKind::Agent {
                name: "persist-agent".to_owned()
            },
        ]
    );
    for descriptor in relaunched.registry().sessions() {
        assert_eq!(
            descriptor.status(),
            &SessionStatus::Restored,
            "a relaunched row must be Restored, never a phantom Running"
        );
    }
    cleanup_state_file(&path);
}

/// A configured agent's command can embed a private path. Every debug and
/// status surface the launch flow can reach must stay free of it. The
/// persisted sessions.toml deliberately carries the agent NAME (the user's
/// own state, exactly what restoration needs — the same class as a worktree
/// path) but never the command or its args.
#[cfg(target_os = "macos")]
#[test]
fn agent_commands_never_reach_debug_status_or_state_surfaces() {
    const NAME_SENTINEL: &str = "NOREN-AGENT-NAME-hunter2";
    const COMMAND_SENTINEL: &str = "/noren/NOREN-AGENT-CMD-hunter2-missing";
    let config = AppConfig::parse(&format!(
        "[[agents]]\nname = \"{NAME_SENTINEL}\"\ncommand = \"{COMMAND_SENTINEL}\"\nargs = [\"{COMMAND_SENTINEL}\"]\n"
    ))
    .expect("valid configuration");
    let path = temp_state_path();
    let mut app = NorenApp::new(config);
    app.workspace.state_path = Some(path.clone());

    // The workspace (configured agents, sidebar view, registry) never
    // prints the command — not before the click...
    assert!(
        !format!("{:?}", app.workspace).contains(COMMAND_SENTINEL),
        "workspace debug leaked the agent command"
    );
    // ...and not after a failed launch (whose failure surfaces are fixed
    // text).
    assert!(click_sidebar_row(&mut app, 0), "the agent row is consumed");
    assert_eq!(app.status, "Noren agent launch failed");
    assert!(
        !format!("{:?}", app.workspace).contains(COMMAND_SENTINEL),
        "workspace debug leaked the agent command after the failure"
    );
    app.teardown();

    let text = std::fs::read_to_string(&path).expect("state saved on quit");
    assert!(
        !text.contains(COMMAND_SENTINEL),
        "the configured command must never reach sessions.toml: {text}"
    );
    assert!(
        text.contains(NAME_SENTINEL),
        "the agent NAME is the persisted identity, like a worktree path: {text}"
    );
    cleanup_state_file(&path);
}
