//! One character cell.

use bitflags::bitflags;

use crate::attrs::Attrs;
use crate::color::Color;

bitflags! {
    /// Grid-level state that is not an SGR attribute.
    ///
    /// A double-width character occupies two cells. The first carries the character
    /// and `WIDE_CHAR`; the second carries a space and `WIDE_CHAR_SPACER` so that the
    /// renderer knows to skip it rather than draw a stray glyph. Keeping the spacer in
    /// the grid, rather than deriving it at render time, is what makes overwriting
    /// half of a wide character behave the way it does in a real terminal.
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
    pub struct CellFlags: u8 {
        /// This cell holds a double-width character.
        const WIDE_CHAR = 1 << 0;
        /// This cell is the trailing half of the double-width character to its left.
        const WIDE_CHAR_SPACER = 1 << 1;
        /// This cell is the leading half of a wide character that starts off-screen to
        /// the right, so it must not be drawn as a narrow character.
        const LEADING_WIDE_CHAR_SPACER = 1 << 2;
    }
}

/// A character cell.
///
/// Twenty bytes. This is the hottest structure in the program, since a full repaint
/// walks every one of them, so the layout is deliberate: the character first, then
/// the two colours as packed `u32`s, then the small fields.
///
/// A cell has no notion of "empty". Blank is the space character with default colours,
/// which means erasing and writing a space are the same operation and the renderer
/// never has to branch on occupancy.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Cell {
    /// The character. Grapheme clusters are composed before they reach the grid, so
    /// this is the base character of the cluster.
    pub ch: char,
    /// Foreground. `Color::DEFAULT` means the theme's foreground.
    pub fg: Color,
    /// Background. `Color::DEFAULT` means the theme's background.
    pub bg: Color,
    /// SGR attributes.
    pub attrs: Attrs,
    /// Index into the screen's hyperlink table plus one, or zero for no link.
    pub link: u16,
    /// Grid-level flags.
    pub flags: CellFlags,
}

impl Cell {
    /// A blank cell: a space with default colours and no attributes.
    pub const fn blank() -> Self {
        Cell {
            ch: ' ',
            fg: Color::DEFAULT,
            bg: Color::DEFAULT,
            attrs: Attrs::empty(),
            link: 0,
            flags: CellFlags::empty(),
        }
    }

    /// Reset this cell for an erase, keeping a background colour.
    ///
    /// Erasing with a background is how `BCE` works: after `SGR 41`, clearing the
    /// screen leaves red behind rather than the theme's background. Getting this
    /// wrong is the cause of the white flash you see in terminals that skip it.
    pub fn reset_with(&mut self, bg: Color) {
        *self = Cell {
            bg,
            ..Cell::blank()
        };
    }

    /// Reset for an erase that also clears the background.
    pub fn reset(&mut self) {
        *self = Cell::blank();
    }

    /// Whether this cell would draw nothing visible.
    ///
    /// Used by the damage tracker to avoid publishing runs of blank cells on a line
    /// that was already blank.
    pub const fn is_blank(self) -> bool {
        self.ch == ' '
            && self.attrs.is_empty()
            && self.flags.is_empty()
            && self.fg.is_default()
            && self.bg.is_default()
            && self.link == 0
    }

    /// Whether this cell is the trailing half of a wide character.
    pub const fn is_wide_spacer(self) -> bool {
        self.flags.contains(CellFlags::WIDE_CHAR_SPACER)
    }

    /// Start a fresh character while keeping the current pen.
    ///
    /// Writing a character must not inherit the previous character's grid flags, or a
    /// narrow character overwriting half of a wide one would leave a ghost spacer.
    pub fn set_char(&mut self, ch: char, fg: Color, bg: Color, attrs: Attrs, link: u16) {
        self.ch = ch;
        self.fg = fg;
        self.bg = bg;
        self.attrs = attrs;
        self.link = link;
        self.flags = CellFlags::empty();
    }
}

impl Default for Cell {
    fn default() -> Self {
        Cell::blank()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::NamedColor;

    #[test]
    fn blank_cell_is_a_default_space() {
        let c = Cell::blank();
        assert_eq!(c.ch, ' ');
        assert!(c.fg.is_default());
        assert!(c.bg.is_default());
        assert!(c.attrs.is_plain());
        assert_eq!(c.link, 0);
        assert!(c.is_blank());
    }

    #[test]
    fn cell_stays_within_twenty_bytes() {
        // Guards against a field being added without thinking about the render loop.
        assert!(
            core::mem::size_of::<Cell>() <= 20,
            "Cell grew to {} bytes",
            core::mem::size_of::<Cell>()
        );
    }

    #[test]
    fn erase_keeps_the_background_for_bce() {
        let red = Color::indexed(NamedColor::Red.index());
        let mut c = Cell {
            ch: 'x',
            fg: Color::rgb(1, 2, 3),
            bg: red,
            attrs: Attrs::BOLD,
            link: 7,
            flags: CellFlags::WIDE_CHAR,
        };
        c.reset_with(red);
        assert_eq!(c.ch, ' ');
        assert_eq!(c.bg, red);
        assert!(c.fg.is_default());
        assert!(c.attrs.is_plain());
        assert_eq!(c.link, 0);
        assert!(c.flags.is_empty());
    }

    #[test]
    fn full_erase_clears_the_background_too() {
        let mut c = Cell {
            bg: Color::rgb(255, 0, 0),
            ..Cell::blank()
        };
        c.reset();
        assert!(c.bg.is_default());
    }

    #[test]
    fn a_narrow_char_overwriting_a_wide_spacer_clears_the_flag() {
        let mut c = Cell {
            flags: CellFlags::WIDE_CHAR_SPACER,
            ..Cell::blank()
        };
        assert!(c.is_wide_spacer());
        c.set_char('a', Color::DEFAULT, Color::DEFAULT, Attrs::empty(), 0);
        assert!(!c.is_wide_spacer());
        assert_eq!(c.ch, 'a');
    }

    #[test]
    fn a_styled_space_is_not_blank() {
        // Trailing whitespace that carries a background must still be painted.
        let c = Cell {
            bg: Color::rgb(0, 0, 255),
            ..Cell::blank()
        };
        assert!(!c.is_blank());
    }
}
