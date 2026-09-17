//! Character attributes set by SGR.
//!
//! Stored as a `u16` because every cell carries one. The underline *style* is four
//! mutually exclusive bits rather than a packed field so that a plain underline is a
//! single bit test, which is the case that runs for every cell of every frame.

use bitflags::bitflags;

bitflags! {
    /// The SGR attributes of a cell.
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
    pub struct Attrs: u16 {
        /// SGR 1. Rendered as a heavier weight, not a synthetic embolden.
        const BOLD = 1 << 0;
        /// SGR 2. Rendered by dimming the foreground toward the background.
        const DIM = 1 << 1;
        /// SGR 3.
        const ITALIC = 1 << 2;
        /// SGR 4. Plain single underline, the common case.
        const UNDERLINE = 1 << 3;
        /// SGR 5. The renderer blinks at the configured cursor rate.
        const BLINK = 1 << 4;
        /// SGR 7. Foreground and background swap at render time.
        const REVERSE = 1 << 5;
        /// SGR 8. The cell paints its background only.
        const HIDDEN = 1 << 6;
        /// SGR 9.
        const STRIKETHROUGH = 1 << 7;
        /// SGR 21.
        const DOUBLE_UNDERLINE = 1 << 8;
        /// SGR 4:3.
        const CURLY_UNDERLINE = 1 << 9;
        /// SGR 4:4.
        const DOTTED_UNDERLINE = 1 << 10;
        /// SGR 4:5.
        const DASHED_UNDERLINE = 1 << 11;
    }
}

/// Every bit that describes how the underline is drawn.
const UNDERLINE_BITS: Attrs = Attrs::from_bits_truncate(
    Attrs::UNDERLINE.bits()
        | Attrs::DOUBLE_UNDERLINE.bits()
        | Attrs::CURLY_UNDERLINE.bits()
        | Attrs::DOTTED_UNDERLINE.bits()
        | Attrs::DASHED_UNDERLINE.bits(),
);

/// How a cell's underline is drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub enum UnderlineStyle {
    /// No underline.
    #[default]
    None,
    /// SGR 4.
    Single,
    /// SGR 21.
    Double,
    /// SGR 4:3. Undercurl, the spelling-checker underline.
    Curly,
    /// SGR 4:4.
    Dotted,
    /// SGR 4:5.
    Dashed,
}

impl Attrs {
    /// The underline style this cell asks for.
    ///
    /// The style bits are treated as a priority list, so a program that sets two
    /// conflicting styles gets the most decorative one rather than nothing.
    pub const fn underline_style(self) -> UnderlineStyle {
        if self.contains(Attrs::CURLY_UNDERLINE) {
            UnderlineStyle::Curly
        } else if self.contains(Attrs::DASHED_UNDERLINE) {
            UnderlineStyle::Dashed
        } else if self.contains(Attrs::DOTTED_UNDERLINE) {
            UnderlineStyle::Dotted
        } else if self.contains(Attrs::DOUBLE_UNDERLINE) {
            UnderlineStyle::Double
        } else if self.contains(Attrs::UNDERLINE) {
            UnderlineStyle::Single
        } else {
            UnderlineStyle::None
        }
    }

    /// Whether any underline style is set.
    pub const fn has_underline(self) -> bool {
        self.intersects(UNDERLINE_BITS)
    }

    /// Clear every underline style. `SGR 24` and the start of a new underline style.
    pub fn clear_underline(&mut self) {
        self.remove(UNDERLINE_BITS);
    }

    /// Whether this cell needs the renderer to do more than paint a glyph in a colour.
    pub const fn is_plain(self) -> bool {
        self.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attrs_are_two_bytes() {
        assert_eq!(core::mem::size_of::<Attrs>(), 2);
    }

    #[test]
    fn bold_and_dim_both_set_is_not_plain() {
        let a = Attrs::BOLD | Attrs::DIM;
        assert!(!a.is_plain());
        assert!(a.contains(Attrs::BOLD));
        assert!(a.contains(Attrs::DIM));
    }

    #[test]
    fn underline_styles_are_prioritised() {
        assert_eq!(Attrs::empty().underline_style(), UnderlineStyle::None);
        assert_eq!(Attrs::UNDERLINE.underline_style(), UnderlineStyle::Single);
        assert_eq!(
            Attrs::DOUBLE_UNDERLINE.underline_style(),
            UnderlineStyle::Double
        );
        assert_eq!(Attrs::DOTTED_UNDERLINE.underline_style(), UnderlineStyle::Dotted);
        assert_eq!(Attrs::DASHED_UNDERLINE.underline_style(), UnderlineStyle::Dashed);
        assert_eq!(Attrs::CURLY_UNDERLINE.underline_style(), UnderlineStyle::Curly);
    }

    #[test]
    fn a_program_setting_two_styles_gets_the_decorative_one() {
        let a = Attrs::UNDERLINE | Attrs::CURLY_UNDERLINE;
        assert_eq!(a.underline_style(), UnderlineStyle::Curly);
        assert!(a.has_underline());
    }

    #[test]
    fn clearing_underline_leaves_the_other_attributes_alone() {
        let mut a = Attrs::BOLD | Attrs::CURLY_UNDERLINE | Attrs::ITALIC;
        a.clear_underline();
        assert_eq!(a, Attrs::BOLD | Attrs::ITALIC);
        assert!(!a.has_underline());
    }

    #[test]
    fn every_underline_style_bit_is_covered_by_has_underline() {
        for style in [
            Attrs::UNDERLINE,
            Attrs::DOUBLE_UNDERLINE,
            Attrs::CURLY_UNDERLINE,
            Attrs::DOTTED_UNDERLINE,
            Attrs::DASHED_UNDERLINE,
        ] {
            assert!(style.has_underline(), "{style:?} should count as an underline");
            let mut cleared = style;
            cleared.clear_underline();
            assert!(cleared.is_empty(), "{style:?} left residue after clearing");
        }
    }
}
