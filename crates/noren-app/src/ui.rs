//! Pure presentation text derived from validated application configuration.
//!
//! These helpers are shared by the live app and the rendered-frame oracle.
//! Keeping chord presentation here means application chrome cannot drift from
//! the [`KeymapConfig`](crate::config::KeymapConfig) the input path honors.

use crate::config::{KeymapConfig, UiConfig};
use crate::passthrough::{Chord, KeyCode};

/// Build the persistent command-palette affordance from the active keymap.
///
/// `None` is returned only for the explicit `[ui] show_palette_hint = false`
/// override. The default config therefore always yields a visible hint.
#[must_use]
pub fn palette_hint(keys: KeymapConfig, ui: UiConfig) -> Option<String> {
    ui.show_palette_hint()
        .then(|| format!("{} Commands", chord_label(keys.palette_open())))
}

/// Terminal-side copy for an empty workspace's direct recovery action.
///
/// The fixed 16-column sidebar keeps the compact `No sessions` locator. The
/// otherwise blank terminal area has enough room to name the configured
/// create-session chord without truncating ordinary bindings, and the input
/// path honors this chord directly while the workspace is empty.
#[must_use]
pub fn empty_workspace_recovery(keys: KeymapConfig) -> Vec<String> {
    vec![
        "No sessions yet".to_owned(),
        format!(
            "Press {} to create a session",
            chord_label(keys.session_create())
        ),
    ]
}

/// Human-readable text for one normalized configured chord.
///
/// This is intentionally generated from [`Chord`] rather than copied from a
/// default string: rebinding an action changes both input dispatch and every
/// UI label that describes it. ASCII words are used because the current
/// bitmap renderer cannot draw the macOS Command glyph faithfully.
#[must_use]
pub fn chord_label(chord: Chord) -> String {
    let modifiers = chord.modifiers();
    let mut parts: Vec<String> = Vec::new();
    if modifiers.is_ctrl() {
        parts.push("Ctrl".to_owned());
    }
    if modifiers.is_alt() {
        parts.push("Alt".to_owned());
    }
    if modifiers.is_shift() {
        parts.push("Shift".to_owned());
    }
    if modifiers.is_super() {
        parts.push("Super".to_owned());
    }
    parts.push(match chord.code() {
        KeyCode::Char(character) => character.to_ascii_uppercase().to_string(),
        KeyCode::Function(number) => format!("F{number}"),
        KeyCode::Enter => "Enter".to_owned(),
        KeyCode::Tab => "Tab".to_owned(),
        KeyCode::Backspace => "Backspace".to_owned(),
        KeyCode::Escape => "Escape".to_owned(),
        KeyCode::Space => "Space".to_owned(),
        KeyCode::Up => "Up".to_owned(),
        KeyCode::Down => "Down".to_owned(),
        KeyCode::Left => "Left".to_owned(),
        KeyCode::Right => "Right".to_owned(),
        KeyCode::Home => "Home".to_owned(),
        KeyCode::End => "End".to_owned(),
        KeyCode::PageUp => "PageUp".to_owned(),
        KeyCode::PageDown => "PageDown".to_owned(),
        KeyCode::Insert => "Insert".to_owned(),
        KeyCode::Delete => "Delete".to_owned(),
    });
    parts.join("+")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    #[test]
    fn palette_hint_is_default_on_and_follows_the_active_keymap() {
        let default = AppConfig::default();
        assert_eq!(
            palette_hint(default.keys(), default.ui()).as_deref(),
            Some("Super+P Commands")
        );

        let rebound = AppConfig::parse("[keys]\npalette_open = \"ctrl+shift+k\"\n")
            .expect("valid rebound keymap");
        assert_eq!(
            palette_hint(rebound.keys(), rebound.ui()).as_deref(),
            Some("Ctrl+Shift+K Commands")
        );
    }

    #[test]
    fn explicit_ui_override_removes_the_palette_hint() {
        let config = AppConfig::parse("[ui]\nshow_palette_hint = false\n")
            .expect("valid palette-hint opt-out");
        assert_eq!(palette_hint(config.keys(), config.ui()), None);
    }

    #[test]
    fn empty_workspace_recovery_follows_the_configured_create_chord() {
        let rebound = AppConfig::parse("[keys]\nsession_create = \"ctrl+n\"\n")
            .expect("valid create-session rebind");
        assert_eq!(
            empty_workspace_recovery(rebound.keys()),
            ["No sessions yet", "Press Ctrl+N to create a session"]
        );
    }
}
