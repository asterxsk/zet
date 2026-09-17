//! One rasterised glyph: the pixels, and where they go.
//!
//! A [`Glyph`] is deliberately not a text run and not a laid-out line. A terminal cell
//! holds exactly one of these, so the type carries the position of the ink relative to
//! the cell and nothing about what comes next.

use crate::library::{FaceKey, Weight};

/// What a caller asks the stack to draw.
///
/// A character and a style, not a glyph id: resolving the character to a face is the
/// stack's job and the whole reason this crate exists. Turning a `GlyphSpec` into
/// pixels is what [`FontStack::rasterize`](crate::FontStack::rasterize) does.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GlyphSpec {
    /// The character to draw.
    pub ch: char,
    /// The weight to draw it at.
    pub weight: Weight,
    /// Whether to draw it italic.
    pub italic: bool,
}

impl GlyphSpec {
    /// A character in the regular upright style.
    #[must_use]
    pub const fn new(ch: char) -> Self {
        Self {
            ch,
            weight: Weight::NORMAL,
            italic: false,
        }
    }

    /// A character in the bold style.
    #[must_use]
    pub const fn bold(ch: char) -> Self {
        Self {
            ch,
            weight: Weight::BOLD,
            italic: false,
        }
    }

    /// A character in the italic style.
    #[must_use]
    pub const fn italic(ch: char) -> Self {
        Self {
            ch,
            weight: Weight::NORMAL,
            italic: true,
        }
    }

    /// The same character at a different weight.
    #[must_use]
    pub const fn with_weight(self, weight: Weight) -> Self {
        Self { weight, ..self }
    }

    /// The same character, italic or not.
    #[must_use]
    pub const fn with_italic(self, italic: bool) -> Self {
        Self { italic, ..self }
    }
}

impl From<char> for GlyphSpec {
    fn from(ch: char) -> Self {
        Self::new(ch)
    }
}

/// What the bytes in a [`Glyph`] mean.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GlyphContent {
    /// One 8-bit coverage sample per pixel, to be multiplied by the foreground
    /// colour.
    ///
    /// This is what every outline produces. It is not the glyph's colour because the
    /// glyph does not have one: the same rendered `A` is drawn in the theme's red for
    /// an error and in its green for a diff, and rasterising it twice would double the
    /// atlas for no reason.
    Alpha,
    /// Four bytes per pixel, premultiplied RGBA, to be drawn as it is.
    ///
    /// Colour emoji and colour outline fonts. These are never tinted: an emoji drawn
    /// in the theme's foreground colour is not an emoji.
    Color,
}

/// A rasterised glyph.
///
/// Not `Eq`: `advance` is a float, and pretending otherwise to please a derive would
/// be a lie about the type.
#[derive(Clone, PartialEq, Debug)]
pub struct Glyph {
    /// What to ask an atlas for to get this glyph again.
    pub key: GlyphKey,
    /// The distance from the pen position to the left edge of the bitmap.
    ///
    /// Negative for a glyph whose ink starts before the pen, which happens for an
    /// italic and for a fallback face wider than its cell.
    pub left: i32,
    /// The distance from the baseline **up** to the top row of the bitmap.
    ///
    /// This is the rasteriser's `bitmap_top`: positive for everything above the
    /// baseline, which is everything with ink. It counts up rather than down because
    /// that is the direction the shaping and rasterising layers already use, and
    /// flipping the sign here would be one more place to remember which way is which.
    pub top: i32,
    /// The bitmap's width in pixels. Zero for a glyph with no ink, such as a space.
    pub width: u32,
    /// The bitmap's height in pixels. Zero for a glyph with no ink.
    pub height: u32,
    /// The horizontal advance this face gives the glyph.
    ///
    /// The grid ignores it — every cell is the primary face's advance for `M`, which
    /// is the rule that keeps the columns from moving — but the chrome's text is
    /// proportional and has nothing else to lay out with.
    pub advance: f32,
    /// What `data` means.
    pub content: GlyphContent,
    /// The bitmap, row-major from the top, with no padding between rows.
    pub data: Vec<u8>,
}

impl Glyph {
    /// A glyph with no ink.
    ///
    /// What a space produces, and what a character nothing on the machine can draw
    /// produces once the stack has given up on it.
    #[must_use]
    pub fn blank(key: GlyphKey, advance: f32) -> Self {
        Self {
            key,
            left: 0,
            top: 0,
            width: 0,
            height: 0,
            advance,
            content: GlyphContent::Alpha,
            data: Vec::new(),
        }
    }

    /// Whether there is nothing to draw.
    #[must_use]
    pub fn is_blank(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Where this glyph's top-left corner goes, given the top-left corner of its cell
    /// and the baseline's distance down from it.
    ///
    /// The one conversion between this type's baseline-relative, y-up convention and
    /// the y-down convention every window and texture uses. Doing it here rather than
    /// in the renderer keeps the sign in one place.
    // A glyph's offset is bounded by the size of the glyph in pixels, which is nowhere
    // near 2^24 where `f32` starts rounding. The cast is exact for every value this can
    // hold.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn offset_in_cell(&self, baseline: f32) -> (f32, f32) {
        // `top` counts up from the baseline and `baseline` counts down from the top of
        // the cell, so subtracting is what turns one into the other.
        (self.left as f32, baseline - self.top as f32)
    }
}

/// The identity of a rasterised glyph, for an atlas to key on.
///
/// Two glyphs with the same key are the same pixels: the same face at the same size.
/// The character is not part of it — a face that maps two characters to one glyph
/// should rasterise once — and neither is the colour, which is applied when the atlas
/// is drawn from.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GlyphKey {
    /// The face the glyph was drawn from.
    pub face: FaceKey,
    /// The glyph's index within that face.
    pub glyph_id: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spec_carries_the_style_explicitly() {
        assert_eq!(GlyphSpec::new('a').weight, Weight::NORMAL);
        assert!(!GlyphSpec::new('a').italic);
        assert_eq!(GlyphSpec::bold('a').weight, Weight::BOLD);
        assert!(GlyphSpec::italic('a').italic);
        assert_eq!(GlyphSpec::from('a'), GlyphSpec::new('a'));
    }

    #[test]
    fn spec_builders_keep_the_character() {
        let spec = GlyphSpec::new('x')
            .with_weight(Weight::MEDIUM)
            .with_italic(true);
        assert_eq!(spec.ch, 'x');
        assert_eq!(spec.weight, Weight::MEDIUM);
        assert!(spec.italic);
    }

    /// A key for a glyph nothing will ever be asked to draw.
    fn a_key(glyph_id: u16) -> GlyphKey {
        GlyphKey {
            face: FaceKey {
                // Ids are handed out by the font database, so a test makes its own.
                family: fontique::FamilyId::new(),
                weight: Weight::NORMAL,
                italic: false,
            },
            glyph_id,
        }
    }

    #[test]
    fn the_cell_offset_flips_the_baseline_to_y_down() {
        let key = a_key(1);
        // A capital eight pixels above the baseline in a cell whose baseline is ten
        // pixels down lands two pixels below the top of the cell.
        let glyph = Glyph {
            key,
            left: 1,
            top: 8,
            width: 4,
            height: 8,
            advance: 6.0,
            content: GlyphContent::Alpha,
            data: vec![0; 32],
        };
        assert_eq!(glyph.offset_in_cell(10.0), (1.0, 2.0));
        assert!(!glyph.is_blank());
    }

    #[test]
    fn a_blank_glyph_has_nothing_to_draw() {
        let blank = Glyph::blank(a_key(0), 7.5);
        assert!(blank.is_blank());
        assert!(blank.data.is_empty());
        // A space still advances, which is the whole reason a blank glyph keeps one.
        assert!((blank.advance - 7.5).abs() < f32::EPSILON);
    }
}
