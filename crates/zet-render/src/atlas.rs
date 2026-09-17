//! Where rasterised glyphs are kept.
//!
//! One texture holds every glyph the session has needed. A glyph is rasterised once,
//! copied in here, and from then on drawing it is a rectangle with texture coordinates
//! on it — which is the whole reason a terminal can repaint a screen full of text at
//! 60Hz without touching a font.
//!
//! # Why a shelf packer
//!
//! Glyphs in a terminal are two sizes: as tall as the font, and as wide as the font.
//! A general rectangle packer would buy a few percent more density and cost an order of
//! magnitude more code and a repack whenever the texture grows. Shelves pack a row of
//! similar-height glyphs at a time and never need to move anything.
//!
//! # Growing never invalidates
//!
//! The texture only ever grows **downward**. Its width is fixed at construction, so
//! adding rows does not change the stride of the rows already there, and every texture
//! coordinate handed out so far stays correct. That is a load-bearing property: a
//! `Frame` that has already been built, or is being built, does not become a set of
//! wrong coordinates because a glyph arrived that did not fit.
//!
//! It is only true because [`Placement::uv`] is in **texels**. A coordinate stored as a
//! fraction of the atlas height is a coordinate that silently means a different row the
//! moment the height doubles, and nothing downstream can repair it: the quads for the
//! frame being built were made before the growth, and the texture is uploaded after it.
//! Texels do not move, so the shader divides by the texture's own size at sample time
//! and the property holds for a frame built across a growth, a cache that outlives one,
//! and anything else that holds a placement for longer than a moment.
//!
//! # What is in it
//!
//! An alpha glyph is stored as white with its coverage in the alpha channel. A colour
//! glyph is stored as it came out of the rasteriser. Both kinds then go through one
//! shader with one blend state, because for the first the tint multiplies white into a
//! premultiplied result and for the second the tint is not read.

use std::collections::HashMap;

use zet_font::{Glyph, GlyphContent, GlyphKey};

/// The width every atlas is built at.
///
/// Fixed for the life of the atlas, because a growing width would change the row stride
/// and invalidate every coordinate already handed out. Wide enough for any glyph a
/// terminal font has: the widest thing in Unicode is a handful of CJK compatibility
/// ideographs at about twice a cell, and a fallback face overflows its cell rather than
/// being scaled into it.
const WIDTH: u32 = 512;

/// The height an atlas starts at, and the tallest it is allowed to become.
///
/// The ceiling is 8MB of texture, which holds several thousand distinct glyphs — far
/// more than a terminal session uses. A session that needs more than this is showing
/// more than a screenful of distinct characters at once, and the alternative to a
/// ceiling is a process that runs the machine out of video memory because a program is
/// printing the whole of Unicode.
const START_HEIGHT: u32 = 512;
const MAX_HEIGHT: u32 = 4096;

/// One pixel of gap around every glyph.
///
/// Bilinear sampling reaches half a texel outside the rectangle it is asked for. Without
/// a gap, the neighbour's ink bleeds in as a faint second copy of the wrong letter.
const PADDING: u32 = 1;

/// Where a glyph ended up.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Placement {
    /// `x0`, `y0`, `x1`, `y1` in the atlas's texels.
    ///
    /// Texels and not a fraction of the atlas, because the atlas grows downward: a
    /// fraction of a height that has since doubled names a row twice as far down. See
    /// the module comment.
    pub uv: [f32; 4],
    /// The distance from the pen position to the left edge of the bitmap.
    pub left: i32,
    /// The distance from the baseline up to the top row of the bitmap.
    pub top: i32,
    /// The bitmap's width in pixels.
    pub width: u32,
    /// The bitmap's height in pixels.
    pub height: u32,
    /// The horizontal advance the face gives the glyph.
    pub advance: f32,
    /// Whether the atlas stored it in colour.
    pub color: bool,
}

/// A row of glyphs of similar height.
#[derive(Clone, Copy, Debug)]
struct Shelf {
    /// The row's top edge.
    y: u32,
    /// The tallest glyph on the row, which is how much vertical space it owns.
    height: u32,
    /// The first free x on the row.
    x: u32,
}

/// The glyph texture and the packing state that fills it.
pub struct Atlas {
    pixels: Vec<u8>,
    height: u32,
    shelves: Vec<Shelf>,
    placements: HashMap<GlyphKey, Placement>,
    changed: bool,
}

impl Default for Atlas {
    fn default() -> Self {
        Self::new()
    }
}

impl Atlas {
    /// An empty atlas at the starting size.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pixels: vec![0; (WIDTH * START_HEIGHT * 4) as usize],
            height: START_HEIGHT,
            shelves: Vec::new(),
            placements: HashMap::new(),
            changed: false,
        }
    }

    /// The atlas width in pixels. Constant.
    #[must_use]
    pub const fn width(&self) -> u32 {
        WIDTH
    }

    /// The atlas height in pixels. Grows, never shrinks.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The raw RGBA pixels, row-major from the top.
    ///
    /// Handed straight to the graphics device. Only valid after [`Atlas::changed`] has
    /// been seen, because uploading this every frame would be several megabytes of bus
    /// traffic to say nothing new.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Whether anything has been written since this was last cleared.
    #[must_use]
    pub const fn changed(&self) -> bool {
        self.changed
    }

    /// Note that the pixels have been sent to the device.
    pub const fn mark_uploaded(&mut self) {
        self.changed = false;
    }

    /// Whether this glyph has already been placed.
    #[must_use]
    pub fn get(&self, key: &GlyphKey) -> Option<Placement> {
        self.placements.get(key).copied()
    }

    /// How many glyphs are in the atlas.
    #[must_use]
    pub fn len(&self) -> usize {
        self.placements.len()
    }

    /// Whether the atlas is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.placements.is_empty()
    }

    /// Put a glyph in, and say where it went.
    ///
    /// `None` when there is no room even after growing, which is a glyph wider than the
    /// atlas or taller than its ceiling. The caller draws nothing for it, which is the
    /// right failure: a glyph that cannot be stored is one that would have to be
    /// re-rasterised every frame, and a blank is better than a stall.
    ///
    /// A glyph with no ink — a space — is stored as a placement with no pixels, so that
    /// the caller gets its advance back without the atlas growing by an empty rectangle.
    pub fn insert(&mut self, key: GlyphKey, glyph: &Glyph) -> Option<Placement> {
        if let Some(placed) = self.get(&key) {
            return Some(placed);
        }
        if glyph.is_blank() {
            let placement = Placement {
                uv: [0.0; 4],
                left: glyph.left,
                top: glyph.top,
                width: 0,
                height: 0,
                advance: glyph.advance,
                color: false,
            };
            self.placements.insert(key, placement);
            return Some(placement);
        }

        let cell_width = glyph.width + PADDING * 2;
        let cell_height = glyph.height + PADDING * 2;
        if cell_width > WIDTH {
            return None;
        }

        let (cell_x, cell_y) = self.allocate(cell_width, cell_height)?;
        // The pixel a glyph starts at, worked out once and used for both the copy and
        // the texture coordinates. Deriving the copy position back out of floating point
        // coordinates would be a round trip through a division that is exact only by
        // luck, and this is also what keeps the coordinates valid across a growth.
        let x = cell_x + PADDING;
        let y = cell_y + PADDING;
        let placement = Placement {
            uv: [
                x as f32,
                y as f32,
                (x + glyph.width) as f32,
                (y + glyph.height) as f32,
            ],
            left: glyph.left,
            top: glyph.top,
            width: glyph.width,
            height: glyph.height,
            advance: glyph.advance,
            color: glyph.content == GlyphContent::Color,
        };
        self.blit(x, y, glyph);
        self.placements.insert(key, placement);
        self.changed = true;
        Some(placement)
    }

    /// Find a corner for a cell of this size, growing the atlas if there is none.
    fn allocate(&mut self, cell_width: u32, cell_height: u32) -> Option<(u32, u32)> {
        for shelf in &mut self.shelves {
            if shelf.height >= cell_height && shelf.x + cell_width <= WIDTH {
                let spot = (shelf.x, shelf.y);
                shelf.x += cell_width;
                return Some(spot);
            }
        }
        self.new_shelf(cell_width, cell_height)
    }

    /// Start a shelf, growing downward if it does not fit.
    fn new_shelf(&mut self, cell_width: u32, cell_height: u32) -> Option<(u32, u32)> {
        let top = self
            .shelves
            .last()
            .map_or(0, |shelf| shelf.y + shelf.height);
        if top + cell_height > self.height {
            // Only ever downward, and only ever a doubling. The width is what makes the
            // existing rows still line up, and doubling is what keeps a session that
            // needs one more glyph from paying for a repack.
            let wanted = (self.height * 2).min(MAX_HEIGHT);
            if top + cell_height > wanted {
                return None;
            }
            self.grow(wanted);
        }
        self.shelves.push(Shelf {
            y: top,
            height: cell_height,
            x: cell_width,
        });
        Some((0, top))
    }

    /// Extend the pixel buffer downward.
    ///
    /// The rows already there keep their offset, which is the whole point: a placement
    /// holds texels, so it still names the same pixel of the same glyph after this.
    fn grow(&mut self, height: u32) {
        self.pixels.resize((WIDTH * height * 4) as usize, 0);
        self.height = height;
    }

    /// Copy a glyph's ink into the atlas at an already-decided pixel.
    fn blit(&mut self, x: u32, y: u32, glyph: &Glyph) {
        let width = glyph.width as usize;
        for row in 0..glyph.height as usize {
            let destination = (((y + row as u32) * WIDTH + x) * 4) as usize;
            match glyph.content {
                // Coverage only, so it is stored as white with the coverage in the
                // alpha channel. A colour stored here would be a lie: the same `A` is
                // drawn in the theme's red for an error and its green for a diff, and
                // rasterising it twice would double the atlas for nothing.
                //
                // One byte per pixel, which is why the two arms index the source
                // differently: an outline's mask is coverage and a colour bitmap is
                // four channels, and reading a mask at four bytes a pixel reads past
                // the end of it.
                GlyphContent::Alpha => {
                    let source = row * width;
                    for column in 0..width {
                        let coverage = glyph.data[source + column];
                        let target = destination + column * 4;
                        self.pixels[target] = 255;
                        self.pixels[target + 1] = 255;
                        self.pixels[target + 2] = 255;
                        self.pixels[target + 3] = coverage;
                    }
                }
                GlyphContent::Color => {
                    let span = width * 4;
                    let source = row * span;
                    self.pixels[destination..destination + span]
                        .copy_from_slice(&glyph.data[source..source + span]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // A texture coordinate is a float and the assertion is that it is the one it should
    // be. The values are exact by construction, so an epsilon would only hide a mistake.
    #![allow(clippy::float_cmp)]

    use super::*;
    use zet_font::{FaceKey, FamilyId, Weight};

    /// One face for a whole test.
    ///
    /// Ids are handed out per call, so a test that built a fresh `FaceKey` for every
    /// glyph would be asking the atlas about a different face every time and would
    /// never hit the cache it is trying to test.
    fn face() -> FaceKey {
        FaceKey {
            family: FamilyId::new(),
            weight: Weight::NORMAL,
            italic: false,
        }
    }

    fn key(face: FaceKey, glyph_id: u16) -> GlyphKey {
        GlyphKey { face, glyph_id }
    }

    /// A solid block of coverage, which stands in for a rasterised glyph.
    fn ink(size: (u32, u32), coverage: u8) -> Glyph {
        let (width, height) = size;
        Glyph {
            key: GlyphKey {
                face: face(),
                glyph_id: 1,
            },
            left: 1,
            top: height as i32,
            width,
            height,
            advance: width as f32,
            content: GlyphContent::Alpha,
            data: vec![coverage; (width * height) as usize],
        }
    }

    fn colored(size: (u32, u32)) -> Glyph {
        let mut glyph = ink(size, 200);
        glyph.content = GlyphContent::Color;
        glyph.data = [10, 20, 30, 200].repeat((size.0 * size.1) as usize);
        glyph
    }

    /// The pixel at an atlas coordinate, as RGBA.
    fn pixel(atlas: &Atlas, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * WIDTH + x) * 4) as usize;
        [
            atlas.pixels[at],
            atlas.pixels[at + 1],
            atlas.pixels[at + 2],
            atlas.pixels[at + 3],
        ]
    }

    #[test]
    fn a_placed_glyph_is_where_its_texture_coordinates_say_it_is() {
        let mut atlas = Atlas::new();
        let glyph = ink((8, 10), 255);
        let placement = atlas.insert(key(face(), 1), &glyph).expect("it fits");
        assert_eq!(placement.width, 8);
        assert_eq!(placement.height, 10);
        assert_eq!(placement.advance, 8.0);
        assert!(!placement.color);

        let x = placement.uv[0] as u32;
        let y = placement.uv[1] as u32;
        assert_eq!(pixel(&atlas, x, y), [255, 255, 255, 255]);
        // One pixel outside is the padding, which is empty. Without it, bilinear
        // sampling would reach into the neighbouring glyph.
        assert_eq!(pixel(&atlas, x - 1, y)[3], 0);
        // And one pixel past the far edge is empty too.
        assert_eq!(pixel(&atlas, x + 8, y + 5)[3], 0);
    }

    #[test]
    fn coverage_lands_in_the_alpha_channel_and_the_colour_stays_white() {
        // The invariant the shader's alpha path depends on: sampling this must give a
        // white texel whose alpha is the coverage, so that the tint multiplies it into
        // a premultiplied result.
        let mut atlas = Atlas::new();
        let placement = atlas
            .insert(key(face(), 1), &ink((4, 4), 128))
            .expect("it fits");
        let x = placement.uv[0] as u32;
        let y = placement.uv[1] as u32;
        assert_eq!(pixel(&atlas, x, y), [255, 255, 255, 128]);
    }

    #[test]
    fn a_colour_glyph_keeps_its_colour() {
        let mut atlas = Atlas::new();
        let placement = atlas
            .insert(key(face(), 1), &colored((4, 4)))
            .expect("it fits");
        assert!(placement.color);
        let x = placement.uv[0] as u32;
        let y = placement.uv[1] as u32;
        assert_eq!(pixel(&atlas, x, y), [10, 20, 30, 200]);
    }

    #[test]
    fn the_same_glyph_asked_for_twice_is_placed_once() {
        let mut atlas = Atlas::new();
        let glyph = ink((6, 6), 255);
        let key = key(face(), 7);
        let first = atlas.insert(key, &glyph).expect("it fits");
        let second = atlas.insert(key, &glyph).expect("it fits");
        assert_eq!(first, second);
        assert_eq!(atlas.len(), 1);
    }

    #[test]
    fn a_glyph_with_no_ink_takes_no_room() {
        // A space. It has an advance and no pixels, and growing the atlas by an empty
        // rectangle for every space in a document would be absurd.
        let mut atlas = Atlas::new();
        let key = key(face(), 3);
        let blank = Glyph::blank(key, 7.5);
        let placement = atlas.insert(key, &blank).expect("it is placed");
        assert_eq!(placement.width, 0);
        assert_eq!(placement.height, 0);
        assert!((placement.advance - 7.5).abs() < f32::EPSILON);
        assert!(atlas.shelves.is_empty(), "an empty glyph took a shelf");
        assert!(!atlas.changed(), "nothing was written, so nothing changed");
    }

    #[test]
    fn glyphs_do_not_overlap() {
        let mut atlas = Atlas::new();
        let face = face();
        let mut seen = Vec::new();
        for id in 0..40 {
            let placement = atlas
                .insert(key(face, id), &ink((9, 12), 255))
                .expect("it fits");
            let u = placement.uv[0] as u32;
            let v = placement.uv[1] as u32;
            for (x, y) in &seen {
                let apart = placement.width + u <= *x
                    || *x + placement.width <= u
                    || placement.height + v <= *y
                    || *y + placement.height <= v;
                assert!(apart, "glyph at ({u},{v}) overlaps one at ({x},{y})");
            }
            seen.push((u, v));
        }
    }

    #[test]
    fn a_placement_names_the_same_texels_before_and_after_a_growth() {
        // The load-bearing property. If growing changed the stride, or moved a shelf, or
        // if a coordinate were a fraction of the height rather than a texel row, a frame
        // already built would draw the wrong glyph — and a frame is built *before* the
        // texture is re-uploaded, so nothing downstream could put it right.
        let mut atlas = Atlas::new();
        let face = face();
        let first = key(face, 1);
        let first_uv = atlas
            .insert(first, &ink((16, 16), 255))
            .expect("it fits")
            .uv;

        // The glyph goes at the first shelf with one pixel of padding on each side, so
        // its rectangle is exactly this. Anything that divided by the atlas height would
        // read the first two as a fraction and land on a different row.
        assert_eq!(
            first_uv,
            [PADDING as f32, PADDING as f32, 17.0, 17.0],
            "the coordinates should be texels, not fractions of the atlas"
        );

        let mut grown_to = 0;
        for id in 2..2000u16 {
            let before = atlas.height();
            let placement = atlas
                .insert(key(face, id), &ink((16, 16), 255))
                .expect("the atlas should keep growing");
            if atlas.height() > before {
                grown_to = atlas.height();
                assert_eq!(atlas.height(), before * 2, "growth is a doubling");
            }
            assert_eq!(placement.width, 16);
        }
        assert!(
            grown_to >= START_HEIGHT * 2,
            "2000 glyphs should not have fitted in {START_HEIGHT} rows"
        );
        assert_eq!(
            atlas.get(&first).expect("still placed").uv,
            first_uv,
            "growing the atlas moved a glyph that was already placed"
        );

        // And the texel it names is still the glyph's own top-left corner. The row below
        // the 16-pixel bitmap belongs to the next shelf, so a coordinate that had drifted
        // would land on another glyph's ink rather than on padding.
        let x = first_uv[0] as u32;
        let y = first_uv[1] as u32;
        assert_eq!(pixel(&atlas, x, y), [255, 255, 255, 255]);
        assert_eq!(
            pixel(&atlas, x, y + 16)[3],
            0,
            "the row past the bitmap should still be the padding below it"
        );
    }

    #[test]
    fn the_height_has_a_ceiling() {
        // A program printing all of Unicode must run out of atlas rather than out of
        // video memory.
        let mut atlas = Atlas::new();
        let face = face();
        let mut placed = 0;
        for id in 0..u16::MAX {
            if atlas.insert(key(face, id), &ink((24, 24), 255)).is_none() {
                break;
            }
            placed += 1;
        }
        assert!(placed > 500, "only {placed} glyphs fit, which is too few");
        assert_eq!(atlas.height(), MAX_HEIGHT);
        assert!(
            atlas
                .insert(key(face, 60000), &ink((24, 24), 255))
                .is_none(),
            "the atlas should have refused"
        );
    }

    #[test]
    fn a_glyph_wider_than_the_atlas_is_refused_rather_than_wrapping() {
        let mut atlas = Atlas::new();
        assert!(
            atlas
                .insert(key(face(), 1), &ink((WIDTH + 1, 8), 255))
                .is_none()
        );
    }

    #[test]
    fn uploading_is_a_flag_and_not_a_guess() {
        let mut atlas = Atlas::new();
        assert!(!atlas.changed());
        let _ = atlas.insert(key(face(), 1), &ink((4, 4), 255));
        assert!(atlas.changed());
        atlas.mark_uploaded();
        assert!(!atlas.changed());
    }

    #[test]
    fn a_glyphs_position_is_carried_through_untouched() {
        // The renderer places a glyph from the placement, not from the `Glyph`, so a
        // placement that dropped the baseline offset would put every glyph on its
        // baseline.
        let mut atlas = Atlas::new();
        let mut glyph = ink((5, 7), 255);
        glyph.left = -2;
        glyph.top = 9;
        glyph.advance = 6.5;
        let placement = atlas.insert(key(face(), 1), &glyph).expect("it fits");
        assert_eq!(placement.left, -2);
        assert_eq!(placement.top, 9);
        assert!((placement.advance - 6.5).abs() < f32::EPSILON);
    }
}
