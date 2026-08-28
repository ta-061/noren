//! Cursor shape vocabulary shared by configuration and the renderer
//! (issues #197/#200).
//!
//! A visible cursor ships as the default: the caret is drawn with no
//! configuration, and this vocabulary only changes *how* it is drawn —
//! shape and colour are user choices, visibility of the product is not.
//! The `[cursor]` configuration table selects a shape here; the renderer
//! resolves its ink against the actual terminal cell.

use std::fmt;

/// The shape the cursor is drawn in, as selected by the `[cursor]`
/// configuration table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorShape {
    /// A filled cell rectangle; the glyph beneath draws in the resolved cell
    /// background so the inverse pair stays readable. The default.
    #[default]
    Block,
    /// A narrow vertical stroke on the left edge of the lead cell.
    Bar,
    /// A horizontal stroke along the bottom of the cell span.
    Underline,
}

impl CursorShape {
    /// Every accepted `[cursor]` shape name, in documentation order.
    pub const NAMES: [&'static str; 3] = ["block", "bar", "underline"];

    /// Resolve one configuration value to a shape.
    ///
    /// Matching is exact (case-sensitive), the same closed-vocabulary rule
    /// theme names follow: accepting `Block` or `BLOCK` would be a second
    /// spelling of one setting, and an unknown value is the caller's typed
    /// error, never a fallback.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "block" => Some(Self::Block),
            "bar" => Some(Self::Bar),
            "underline" => Some(Self::Underline),
            _ => None,
        }
    }

    /// The canonical configuration name of this shape.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::Bar => "bar",
            Self::Underline => "underline",
        }
    }
}

impl fmt::Display for CursorShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::CursorShape;

    /// The closed vocabulary round-trips through its exact names, and the
    /// case-sensitive rule rejects second spellings.
    #[test]
    fn shape_names_parse_exactly_and_round_trip() {
        for name in CursorShape::NAMES {
            let shape = CursorShape::parse(name).expect("a documented name parses");
            assert_eq!(shape.as_str(), name);
            assert_eq!(shape.to_string(), name);
        }
        assert_eq!(CursorShape::default(), CursorShape::Block);
        for near_miss in ["Block", "BLOCK", "bars", "under-line", ""] {
            assert!(
                CursorShape::parse(near_miss).is_none(),
                "{near_miss:?} must not parse — the vocabulary is closed"
            );
        }
    }
}
