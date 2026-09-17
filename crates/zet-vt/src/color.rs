//! A colour packed into four bytes.
//!
//! Every cell carries two of these, so the representation is chosen for size rather
//! than for ergonomics. Three states share one `u32`, distinguished by the top byte:
//! default (the theme's foreground or background), an index into the 256-colour
//! palette, or a direct RGB triple.

use core::fmt;

/// A colour as stored in a cell.
///
/// Use [`Color::spec`] to read it back out. The constructors are `const` so themes
/// and tests can build palettes without runtime work.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Color(u32);

const DEFAULT: u32 = 0x0000_0000;
const INDEXED_TAG: u32 = 0x0100_0000;
const RGB_TAG: u32 = 0x0200_0000;
const TAG_MASK: u32 = 0xFF00_0000;
const PAYLOAD_MASK: u32 = 0x00FF_FFFF;

impl Color {
    /// The theme's default foreground or background, depending on the field.
    pub const DEFAULT: Color = Color(DEFAULT);

    /// An index into the 256-colour palette.
    pub const fn indexed(index: u8) -> Self {
        Color(INDEXED_TAG | index as u32)
    }

    /// A 24-bit direct colour.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color(RGB_TAG | ((r as u32) << 16) | ((g as u32) << 8) | b as u32)
    }

    /// Whether this colour defers to the theme.
    pub const fn is_default(self) -> bool {
        self.0 == DEFAULT
    }

    /// The three states this colour can be in.
    pub const fn spec(self) -> ColorSpec {
        match self.0 & TAG_MASK {
            DEFAULT => ColorSpec::Default,
            INDEXED_TAG => ColorSpec::Indexed((self.0 & PAYLOAD_MASK) as u8),
            _ => ColorSpec::Rgb(
                ((self.0 >> 16) & 0xFF) as u8,
                ((self.0 >> 8) & 0xFF) as u8,
                (self.0 & 0xFF) as u8,
            ),
        }
    }

    /// The raw packed value. Useful for hashing a cell into a glyph cache key.
    pub const fn to_bits(self) -> u32 {
        self.0
    }

    /// Rebuild a colour from [`Color::to_bits`].
    pub const fn from_bits(bits: u32) -> Self {
        Color(bits)
    }
}

impl Default for Color {
    fn default() -> Self {
        Color::DEFAULT
    }
}

impl fmt::Debug for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.spec() {
            ColorSpec::Default => f.write_str("Color::DEFAULT"),
            ColorSpec::Indexed(i) => write!(f, "Color::indexed({i})"),
            ColorSpec::Rgb(r, g, b) => write!(f, "Color::rgb({r}, {g}, {b})"),
        }
    }
}

/// The readable form of a [`Color`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorSpec {
    /// Defer to the theme.
    Default,
    /// An index into the 256-colour palette. `0..16` are the theme's ANSI colours,
    /// `16..232` the colour cube, `232..256` the grey ramp.
    Indexed(u8),
    /// A direct colour.
    Rgb(u8, u8, u8),
}

/// The sixteen ANSI colours, in SGR order.
///
/// A theme supplies these. zet's own themes are held to a 4.5:1 contrast floor
/// against their background; imported palettes ship as published.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(u8)]
pub enum NamedColor {
    /// Index 0.
    Black = 0,
    /// Index 1.
    Red = 1,
    /// Index 2.
    Green = 2,
    /// Index 3.
    Yellow = 3,
    /// Index 4.
    Blue = 4,
    /// Index 5.
    Magenta = 5,
    /// Index 6.
    Cyan = 6,
    /// Index 7.
    White = 7,
    /// Index 8. The "bright" half is a rendering convention, not a lightness rule:
    /// several themes make bright black a mid grey and bright white a soft off-white.
    BrightBlack = 8,
    /// Index 9.
    BrightRed = 9,
    /// Index 10.
    BrightGreen = 10,
    /// Index 11.
    BrightYellow = 11,
    /// Index 12.
    BrightBlue = 12,
    /// Index 13.
    BrightMagenta = 13,
    /// Index 14.
    BrightCyan = 14,
    /// Index 15.
    BrightWhite = 15,
}

impl NamedColor {
    /// The palette index for this colour.
    pub const fn index(self) -> u8 {
        self as u8
    }

    /// Parse an SGR foreground parameter in `30..=37` or `90..=97`.
    pub const fn from_sgr_fg(param: u16) -> Option<Self> {
        match param {
            30 => Some(Self::Black),
            31 => Some(Self::Red),
            32 => Some(Self::Green),
            33 => Some(Self::Yellow),
            34 => Some(Self::Blue),
            35 => Some(Self::Magenta),
            36 => Some(Self::Cyan),
            37 => Some(Self::White),
            90 => Some(Self::BrightBlack),
            91 => Some(Self::BrightRed),
            92 => Some(Self::BrightGreen),
            93 => Some(Self::BrightYellow),
            94 => Some(Self::BrightBlue),
            95 => Some(Self::BrightMagenta),
            96 => Some(Self::BrightCyan),
            97 => Some(Self::BrightWhite),
            _ => None,
        }
    }

    /// Parse an SGR background parameter in `40..=47` or `100..=107`.
    pub const fn from_sgr_bg(param: u16) -> Option<Self> {
        match param {
            40..=47 => Self::from_sgr_fg(param - 10),
            100..=107 => Self::from_sgr_fg(param - 10),
            _ => None,
        }
    }
}

/// Resolve an index in `16..256` to RGB. Indices below 16 are the theme's business.
///
/// The cube and the grey ramp are fixed by the xterm specification, so they are
/// computed here rather than stored.
pub const fn indexed_to_rgb(index: u8) -> Option<(u8, u8, u8)> {
    if index < 16 {
        return None;
    }
    if index < 232 {
        let i = index - 16;
        let steps = [0u8, 95, 135, 175, 215, 255];
        let r = steps[(i / 36) as usize];
        let g = steps[((i % 36) / 6) as usize];
        let b = steps[(i % 6) as usize];
        return Some((r, g, b));
    }
    let v = 8 + (index - 232) * 10;
    Some((v, v, v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_zero_and_roundtrips() {
        assert!(Color::DEFAULT.is_default());
        assert_eq!(Color::DEFAULT.spec(), ColorSpec::Default);
        assert_eq!(Color::from_bits(0), Color::DEFAULT);
    }

    #[test]
    fn indexed_roundtrips_every_index() {
        for i in 0..=u8::MAX {
            let c = Color::indexed(i);
            assert_eq!(c.spec(), ColorSpec::Indexed(i));
            assert!(!c.is_default());
        }
    }

    #[test]
    fn rgb_roundtrips_including_black() {
        // RGB black must not collide with the default sentinel, which is the whole
        // reason the tag byte exists.
        let black = Color::rgb(0, 0, 0);
        assert!(!black.is_default());
        assert_eq!(black.spec(), ColorSpec::Rgb(0, 0, 0));
        assert_ne!(black.to_bits(), Color::DEFAULT.to_bits());

        for (r, g, b) in [(255, 255, 255), (1, 2, 3), (255, 0, 128)] {
            assert_eq!(Color::rgb(r, g, b).spec(), ColorSpec::Rgb(r, g, b));
        }
    }

    #[test]
    fn color_is_four_bytes() {
        assert_eq!(core::mem::size_of::<Color>(), 4);
    }

    #[test]
    fn sgr_parameters_map_to_the_right_names() {
        assert_eq!(NamedColor::from_sgr_fg(31), Some(NamedColor::Red));
        assert_eq!(NamedColor::from_sgr_fg(94), Some(NamedColor::BrightBlue));
        assert_eq!(NamedColor::from_sgr_fg(38), None);
        assert_eq!(NamedColor::from_sgr_bg(41), Some(NamedColor::Red));
        assert_eq!(NamedColor::from_sgr_bg(101), Some(NamedColor::BrightRed));
        assert_eq!(NamedColor::from_sgr_bg(39), None);
    }

    #[test]
    fn cube_endpoints_match_xterm() {
        assert_eq!(indexed_to_rgb(16), Some((0, 0, 0)));
        assert_eq!(indexed_to_rgb(21), Some((0, 0, 255)));
        assert_eq!(indexed_to_rgb(231), Some((255, 255, 255)));
        assert_eq!(indexed_to_rgb(196), Some((255, 0, 0)));
    }

    #[test]
    fn grey_ramp_matches_xterm() {
        assert_eq!(indexed_to_rgb(232), Some((8, 8, 8)));
        assert_eq!(indexed_to_rgb(255), Some((238, 238, 238)));
    }

    #[test]
    fn first_sixteen_are_the_themes_business() {
        for i in 0..16 {
            assert_eq!(indexed_to_rgb(i), None);
        }
    }
}
