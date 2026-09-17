//! What one frame is made of.
//!
//! A frame is two arrays and a list of ranges into them. That is the whole design, and
//! it is deliberately flat: the expensive part of drawing a terminal is deciding *what*
//! to draw, and once that decision is a number in an array the graphics device is very
//! good at the rest. There is no scene graph, no retained tree, and no per-element
//! texture — a cell background, a chrome hairline, and the cursor are all rectangles,
//! and they all go in the same array.
//!
//! # Two kinds of thing
//!
//! A [`Quad`] is a rectangle of one colour. A [`GlyphQuad`] is a rectangle of texture.
//! Everything zet draws is one of those, which is why there are two pipelines and not
//! twelve.
//!
//! # Order is the drawing order
//!
//! The batches are submitted in the order they appear, and each one blends over what is
//! already there. So the list is where layering lives: cell backgrounds, then glyphs,
//! then the underline and the cursor on top, then the chrome over all of it. Nothing
//! carries a depth value, because a terminal is two-dimensional and always has been.

use std::ops::Range;

use bytemuck::{Pod, Zeroable};

/// A rectangle of one colour.
///
/// The colour is linear and premultiplied, because that is what a framebuffer wants and
/// converting at the last moment is a bug waiting to happen rather than a saving.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct Quad {
    /// `x`, `y`, `width`, `height` in physical pixels, from the top-left of the surface.
    pub rect: [f32; 4],
    /// Linear premultiplied RGBA.
    pub color: [f32; 4],
}

impl Quad {
    /// A rectangle of one opaque colour.
    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32, color: [f32; 4]) -> Self {
        Self {
            rect: [x, y, width, height],
            color,
        }
    }
}

/// One glyph, copied out of the atlas.
///
/// The tint is linear premultiplied like a [`Quad`]'s colour. It is ignored for a glyph
/// the atlas stored in colour, which is what `flags` says — an emoji is drawn as it was
/// rasterised and never in the theme's foreground.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct GlyphQuad {
    /// `x`, `y`, `width`, `height` in physical pixels.
    pub rect: [f32; 4],
    /// `u0`, `v0`, `u1`, `v1`, normalised within the atlas.
    pub uv: [f32; 4],
    /// Linear premultiplied RGBA.
    pub color: [f32; 4],
    /// How the sampled texel is to be read.
    pub flags: u32,
    /// Explicit padding so the struct's size is a multiple of its alignment. The
    /// graphics device reads this array as raw bytes, and a silent mismatch between
    /// what Rust lays out and what the shader expects is a wrong picture with no error.
    pub padding: [u32; 3],
}

/// How to read a glyph's texel.
pub mod glyph_flags {
    /// One coverage sample per pixel, multiplied by the tint.
    pub const ALPHA: u32 = 0;
    /// Premultiplied RGBA, drawn as it is.
    pub const COLOR: u32 = 1;
}

impl GlyphQuad {
    /// A glyph the atlas stored as coverage, to be drawn in `color`.
    #[must_use]
    pub fn alpha(rect: [f32; 4], uv: [f32; 4], color: [f32; 4]) -> Self {
        Self {
            rect,
            uv,
            color,
            flags: glyph_flags::ALPHA,
            padding: [0; 3],
        }
    }

    /// A glyph the atlas stored in colour, to be drawn as it is.
    ///
    /// `color` is still carried and still ignored — the vertex layout is fixed, and
    /// having two layouts so that one can leave out a field the shader does not read
    /// would be two pipelines for nothing.
    #[must_use]
    pub fn color(rect: [f32; 4], uv: [f32; 4]) -> Self {
        Self {
            rect,
            uv,
            color: [1.0, 1.0, 1.0, 1.0],
            flags: glyph_flags::COLOR,
            padding: [0; 3],
        }
    }
}

/// Which array a batch draws from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BatchKind {
    /// A run of [`Frame::quads`].
    Quads,
    /// A run of [`Frame::glyphs`].
    Glyphs,
}

/// A run of one kind of thing to draw in one go.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Batch {
    /// Which array the range indexes.
    pub kind: BatchKind,
    /// The half-open range.
    pub range: Range<u32>,
}

/// Everything one frame draws.
///
/// Rebuilt from scratch by whoever knows what changed, and thrown away after it is
/// submitted. It owns its arrays so that a frame can be built on one thread and handed
/// to another without a lifetime argument tying them together.
#[derive(Clone, PartialEq, Debug)]
pub struct Frame {
    /// Every rectangle in the frame.
    pub quads: Vec<Quad>,
    /// Every glyph in the frame.
    pub glyphs: Vec<GlyphQuad>,
    /// The runs to draw, in order.
    pub batches: Vec<Batch>,
    /// The colour the surface is cleared to before the first batch, linear.
    pub clear: [f32; 4],
}

impl Default for Frame {
    fn default() -> Self {
        Self::new()
    }
}

impl Frame {
    /// An empty frame.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            quads: Vec::new(),
            glyphs: Vec::new(),
            batches: Vec::new(),
            clear: [0.0, 0.0, 0.0, 1.0],
        }
    }

    /// Throw away everything but the capacity.
    ///
    /// The capacity is the point: a terminal rebuilds its frame every time anything
    /// changes, and reallocating two buffers sixty times a second is a cost with no
    /// benefit. `Frame` is reused across frames for this reason and `clear` is the only
    /// thing that survives.
    pub fn reset(&mut self) {
        self.quads.clear();
        self.glyphs.clear();
        self.batches.clear();
    }

    /// Add a rectangle.
    pub fn push_quad(&mut self, quad: Quad) {
        self.quads.push(quad);
    }

    /// Add a glyph.
    pub fn push_glyph(&mut self, glyph: GlyphQuad) {
        self.glyphs.push(glyph);
    }

    /// Start a run of rectangles.
    ///
    /// Together with [`Frame::end_quads`] this is how a caller says "everything between
    /// here and there is drawn all at once". Pushing without a batch produces a frame
    /// that draws nothing, which is why the renderer always pairs them.
    ///
    /// The two kinds of run may be open at the same time, and a caller that walks a grid
    /// once naturally does: it wants every rectangle of a row drawn before any of the
    /// row's glyphs, and it discovers both in the same pass. Each `end` therefore closes
    /// the run of its own kind rather than the most recent one.
    pub fn begin_quads(&mut self) {
        let start = u32::try_from(self.quads.len()).unwrap_or(u32::MAX);
        self.batches.push(Batch {
            kind: BatchKind::Quads,
            range: start..start,
        });
    }

    /// Finish the run of rectangles started by [`Frame::begin_quads`].
    pub fn end_quads(&mut self) {
        let end = u32::try_from(self.quads.len()).unwrap_or(u32::MAX);
        self.close(BatchKind::Quads, end);
    }

    /// Start a run of glyphs.
    pub fn begin_glyphs(&mut self) {
        let start = u32::try_from(self.glyphs.len()).unwrap_or(u32::MAX);
        self.batches.push(Batch {
            kind: BatchKind::Glyphs,
            range: start..start,
        });
    }

    /// Finish the run of glyphs started by [`Frame::begin_glyphs`].
    pub fn end_glyphs(&mut self) {
        let end = u32::try_from(self.glyphs.len()).unwrap_or(u32::MAX);
        self.close(BatchKind::Glyphs, end);
    }

    /// Close the innermost open run of one kind.
    fn close(&mut self, kind: BatchKind, end: u32) {
        if let Some(batch) = self
            .batches
            .iter_mut()
            .rev()
            .find(|batch| batch.kind == kind)
        {
            batch.range.end = end;
        }
    }

    /// Whether the frame would draw anything.
    ///
    /// A frame can legitimately be empty — a terminal with nothing on screen — and the
    /// renderer still has to clear and present, so this is a fact about the batches and
    /// not a reason to skip the frame.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.batches.iter().all(|batch| batch.range.is_empty())
    }

    /// Drop every batch whose range came out empty.
    ///
    /// A run with nothing in it still costs a draw call. A grid row that is entirely
    /// blank produces none of these because the grid renderer skips it, but a caller
    /// that batches defensively should not be able to make the device do nothing
    /// slowly.
    pub fn drop_empty_batches(&mut self) {
        self.batches.retain(|batch| !batch.range.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_batch_covers_exactly_what_was_pushed_into_it() {
        let mut frame = Frame::new();
        frame.begin_quads();
        frame.push_quad(Quad::new(0.0, 0.0, 1.0, 1.0, [1.0; 4]));
        frame.push_quad(Quad::new(1.0, 0.0, 1.0, 1.0, [1.0; 4]));
        frame.end_quads();
        assert_eq!(frame.batches.len(), 1);
        assert_eq!(frame.batches[0].kind, BatchKind::Quads);
        assert_eq!(frame.batches[0].range, 0..2);
        assert!(!frame.is_empty());
    }

    #[test]
    fn batches_keep_the_order_they_were_opened_in() {
        // Order is the only thing that decides what is on top, so it has to survive.
        let mut frame = Frame::new();
        frame.begin_quads();
        frame.end_quads();
        frame.begin_glyphs();
        frame.end_glyphs();
        frame.begin_quads();
        frame.end_quads();
        frame.drop_empty_batches();
        assert!(frame.batches.is_empty(), "empty runs should be dropped");

        frame.begin_quads();
        frame.push_quad(Quad::new(0.0, 0.0, 1.0, 1.0, [1.0; 4]));
        frame.end_quads();
        frame.begin_glyphs();
        frame.push_glyph(GlyphQuad::alpha([0.0; 4], [0.0; 4], [1.0; 4]));
        frame.end_glyphs();
        let kinds: Vec<BatchKind> = frame.batches.iter().map(|batch| batch.kind).collect();
        assert_eq!(kinds, vec![BatchKind::Quads, BatchKind::Glyphs]);
    }

    #[test]
    fn the_two_kinds_of_run_can_be_open_at_once() {
        // Walking a grid once produces the rectangles of a row and then its glyphs, and
        // closing a run must find its own kind rather than whichever was opened last.
        let mut frame = Frame::new();
        frame.begin_quads();
        frame.begin_glyphs();
        frame.push_quad(Quad::new(0.0, 0.0, 1.0, 1.0, [1.0; 4]));
        frame.push_glyph(GlyphQuad::alpha([0.0; 4], [0.0; 4], [1.0; 4]));
        frame.end_quads();
        frame.end_glyphs();
        assert_eq!(frame.batches[0].range, 0..1);
        assert_eq!(frame.batches[1].range, 0..1);
        frame.drop_empty_batches();
        assert_eq!(frame.batches.len(), 2);
    }

    #[test]
    fn resetting_keeps_the_capacity_and_drops_the_contents() {
        let mut frame = Frame::new();
        frame.begin_quads();
        frame.push_quad(Quad::new(0.0, 0.0, 1.0, 1.0, [1.0; 4]));
        frame.end_quads();
        let capacity = frame.quads.capacity();
        frame.reset();
        assert!(frame.quads.is_empty());
        assert!(frame.batches.is_empty());
        assert!(frame.glyphs.is_empty());
        assert_eq!(frame.quads.capacity(), capacity);
    }

    #[test]
    fn the_vertex_structs_are_the_size_the_shader_expects() {
        // The device reads these as bytes. A changed field, an added one, or a derive
        // that pads differently would be a wrong picture with no compile error, so the
        // sizes are asserted.
        assert_eq!(size_of::<Quad>(), 32);
        assert_eq!(size_of::<GlyphQuad>(), 64);
    }

    #[test]
    fn a_glyph_flag_says_how_to_read_the_texel() {
        let alpha = GlyphQuad::alpha([0.0; 4], [0.0; 4], [1.0, 0.0, 0.0, 1.0]);
        let color = GlyphQuad::color([0.0; 4], [0.0; 4]);
        assert_eq!(alpha.flags, glyph_flags::ALPHA);
        assert_eq!(color.flags, glyph_flags::COLOR);
        assert_ne!(alpha.flags, color.flags);
    }
}
