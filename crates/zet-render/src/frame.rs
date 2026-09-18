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
    /// `x0`, `y0`, `x1`, `y1` in the atlas's texels. The fragment shader divides by the
    /// texture's own size, because the atlas grows downward under a frame that is already
    /// being built.
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

/// A rectangle of two colours, one at each end of an axis.
///
/// The one thing in a frame that is not a flat colour, and it is here rather than in the
/// quad array because it is a different kind of thing: a quad is one colour for the whole
/// of itself and this is a colour that changes across itself. `window.background`'s
/// gradient is the only caller.
///
/// The colours are linear and premultiplied like a [`Quad`]'s, and the mix happens in the
/// shader between two values that are already in that space — interpolating premultiplied
/// linear values between two opaque colours is interpolating the colours, which is what a
/// gradient is.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct Gradient {
    /// `x`, `y`, `width`, `height` in physical pixels, from the top-left of the surface.
    pub rect: [f32; 4],
    /// The colour the axis starts at, linear premultiplied.
    pub from: [f32; 4],
    /// The colour it ends at.
    pub to: [f32; 4],
    /// `dx`, `dy`, the length to divide the projection by, and where the first stop sits.
    ///
    /// The direction is a vector rather than an angle because that is the only form the
    /// shader wants it in, and the other two are carried here rather than worked out per
    /// pixel: together they are what makes the first stop land on one edge of the
    /// rectangle and the second on the other, at every angle. The shader is then one
    /// subtract, one dot product, one divide and one mix — the frame is built on the CPU
    /// and the device is given arithmetic it cannot get wrong.
    pub axis: [f32; 4],
}

impl Gradient {
    /// A two-stop gradient across a rectangle, at an angle in degrees clockwise from
    /// pointing right.
    ///
    /// The arithmetic that turns the angle into an axis lives here rather than in the
    /// shader, because it is the part that is easy to get subtly wrong. A gradient whose
    /// length ignored the angle would run out of colour before the far corner and clamp
    /// for the rest of the rectangle, which reads as a hard edge in a soft background.
    ///
    /// The projection of the rectangle onto the direction is what the two numbers are:
    /// the interval it covers is `low` to `low + length`, and `t` is how far along that
    /// interval a pixel is. At an angle of zero they are the left and right edges; at
    /// ninety, the top and the bottom; at forty-five, two opposite corners.
    #[must_use]
    pub fn new(
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        from: [f32; 4],
        to: [f32; 4],
        angle: f32,
    ) -> Self {
        let radians = angle.to_radians();
        let (dx, dy) = (radians.cos(), radians.sin());
        // How far the rectangle reaches along the direction, and where that interval
        // starts. Both are sums of the two sides' contributions: a box is the sum of its
        // parts under a linear projection, whatever the angle between them.
        let length = (width * dx).abs() + (height * dy).abs();
        let low = (width * dx).min(0.0) + (height * dy).min(0.0);
        Self {
            rect: [x, y, width, height],
            from,
            to,
            // A rectangle with no extent in either direction has no interval to be a
            // fraction of, and a divide by zero in a shader is a NaN across the screen
            // rather than a colour. Nothing draws such a background, and this is what
            // makes that a fact about the frame instead of a hope about its callers.
            axis: [dx, dy, length.max(f32::EPSILON), low],
        }
    }
}

/// A picture covering the window, cropped to fill it.
///
/// The instance data for the one rectangle a picture is drawn as, and the arithmetic that
/// decides which part of the texture lands in it is in [`PictureQuad::new`] rather than in
/// the shader: the aspect ratio of the window against the aspect ratio of the picture is
/// the whole of the decision, and it is the kind of thing that is a one-line test on the
/// CPU and an afternoon of squinting on the device.
///
/// The colours come from the texture rather than from a vertex, so what is here is where
/// to draw it and how much of the picture to use. `opacity` is the configuration's own and
/// is applied to the sampled texel; the rest of the picture's numbers are the texture's.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct PictureQuad {
    /// `x`, `y`, `width`, `height` in physical pixels: always the whole window.
    pub rect: [f32; 4],
    /// The part of the texture that lands in it, as `u0`, `v0`, `u1`, `v1`.
    pub uv: [f32; 4],
    /// How strongly to draw it, from zero to one.
    pub opacity: f32,
}

impl PictureQuad {
    /// Cover a `surface` of this many physical pixels with a picture of this many texels.
    ///
    /// Scaled to fill, which means the shorter of the two ratios: a picture wider than the
    /// window is cropped left and right, a picture taller than it is cropped top and
    /// bottom, and one of exactly the window's shape is cropped nowhere. What is never
    /// done is stretching, which would turn a photograph of a person into a photograph of
    /// a different person, or fitting, which would leave bars of the theme's ground down
    /// two sides of a background.
    ///
    /// The crop is centred, so the part of the picture that is lost is the same amount off
    /// each edge. `uv` is the fraction of the texture that survives: a window half as wide
    /// as the picture it is showing keeps the middle half of it.
    #[must_use]
    pub fn new(surface: (f32, f32), picture: (u32, u32), opacity: f32) -> Self {
        let (width, height) = (surface.0.max(f32::EPSILON), surface.1.max(f32::EPSILON));
        let (across, down) = (picture.0.max(1) as f32, picture.1.max(1) as f32);
        // The fraction of the picture the window shows, in each direction, at the scale
        // that fills it. Both are at most one: the axis that decided the scale shows all
        // of itself and the other shows less.
        let shown_x = (width / across / (width / across).max(height / down)).min(1.0);
        let shown_y = (height / down / (width / across).max(height / down)).min(1.0);
        let (left, top) = ((1.0 - shown_x) / 2.0, (1.0 - shown_y) / 2.0);
        Self {
            rect: [0.0, 0.0, width, height],
            uv: [left, top, left + shown_x, top + shown_y],
            opacity,
        }
    }
}

/// What the window is painted on before anything else in the frame.
///
/// The two kinds of background that are not the clear colour. `Background::Solid` is not
/// here at all: it is the theme's ground, which is what the surface is cleared to, and a
/// value that drew it a second time would be the same pixels for the price of a shader.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Backdrop {
    /// Two colours across the window.
    Gradient(Gradient),
    /// The window's picture.
    ///
    /// The pixels are not here: they are a texture on the device, uploaded once when the
    /// configuration names them, and a frame that carried them would be copying megabytes
    /// per redraw to say something that has not changed. What is left for the frame to
    /// decide is how strongly to draw it.
    Picture {
        /// How much of the picture shows, from zero to one.
        opacity: f32,
    },
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
    /// What covers the window before any batch, if it is not the clear colour alone.
    ///
    /// Not a batch, and deliberately: it can only ever be the first thing drawn, so
    /// making it a run would give a caller a way to get the order wrong that buys nothing.
    /// It is a field beside `clear` because it is the same kind of thing — what the frame
    /// is painted on — and because a frame with a backdrop and a frame without one differ
    /// in one value rather than in the shape of the list.
    pub backdrop: Option<Backdrop>,
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
            backdrop: None,
        }
    }

    /// Throw away everything but the capacity.
    ///
    /// The capacity is the point: a terminal rebuilds its frame every time anything
    /// changes, and reallocating two buffers sixty times a second is a cost with no
    /// benefit. `Frame` is reused across frames for this reason, and `clear` and
    /// `backdrop` are the only things that survive — both of them answers to a question
    /// about the window rather than about its contents, and both set on the way into a
    /// frame by the same caller that resets it.
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

    /// The `t` the fragment shader computes for a pixel.
    ///
    /// Written out here rather than trusted, because it is the other half of the contract
    /// `Gradient::new` has to keep: the numbers in `axis` exist to make this come out
    /// between zero and one across the rectangle, and only running it shows whether they
    /// do. The formula is the one in `backdrop.wgsl`, to the letter.
    fn fraction(gradient: &Gradient, x: f32, y: f32) -> f32 {
        let [dx, dy, length, low] = gradient.axis;
        let [ox, oy, ..] = gradient.rect;
        (((x - ox) * dx + (y - oy) * dy) - low) / length
    }

    /// A rectangle wide enough that a wrong axis is visible in the fourth decimal place,
    /// and not square, so that an angle's two sides cannot cancel each other out.
    const RECT: (f32, f32, f32, f32) = (10.0, 20.0, 200.0, 100.0);

    fn gradient_at(angle: f32) -> Gradient {
        Gradient::new(
            RECT.0,
            RECT.1,
            RECT.2,
            RECT.3,
            [1.0, 1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0, 1.0],
            angle,
        )
    }

    #[test]
    fn a_gradient_starts_on_one_edge_and_ends_on_the_other_at_every_quarter_turn() {
        let (x, y, w, h) = RECT;
        let cases = [
            // Angle, the corner the first colour is at, the corner the second is at.
            (0.0, (x, y), (x + w, y)),
            (90.0, (x, y), (x, y + h)),
            (180.0, (x + w, y), (x, y)),
            (270.0, (x, y + h), (x, y)),
        ];
        for (angle, first, last) in cases {
            let gradient = gradient_at(angle);
            let start = fraction(&gradient, first.0, first.1);
            let end = fraction(&gradient, last.0, last.1);
            assert!(
                start.abs() < 1e-4,
                "at {angle} degrees the first stop is at {start} of the way along, not 0"
            );
            assert!(
                (end - 1.0).abs() < 1e-4,
                "at {angle} degrees the second stop is at {end} of the way along, not 1"
            );
        }
    }

    #[test]
    fn a_diagonal_gradient_puts_its_halfway_line_through_the_other_two_corners() {
        // A square, so that the answer is a symmetry rather than a number: at forty-five
        // degrees the 50% line runs between the two corners the axis does not touch, and
        // every one of them is halfway along it. This is the case a length that ignored
        // the angle would get wrong — it would reach full colour at the top-right corner
        // and clamp for the whole of the bottom-left half.
        let gradient = Gradient::new(0.0, 0.0, 100.0, 100.0, [1.0; 4], [0.0; 4], 45.0);
        for (x, y) in [(0.0, 0.0), (100.0, 100.0), (100.0, 0.0), (0.0, 100.0)] {
            let t = fraction(&gradient, x, y);
            assert!(
                (0.0..=1.0).contains(&t),
                "the corner at ({x}, {y}) is off the end of the gradient at {t}"
            );
        }
        assert!((fraction(&gradient, 100.0, 0.0) - 0.5).abs() < 1e-4);
        assert!((fraction(&gradient, 0.0, 100.0) - 0.5).abs() < 1e-4);
        assert!(fraction(&gradient, 0.0, 0.0).abs() < 1e-4);
        assert!((fraction(&gradient, 100.0, 100.0) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_gradient_with_no_extent_has_a_length_to_divide_by() {
        // Not a window anyone can make, and a shader dividing by zero is a screen of NaN
        // rather than a colour: the frame is where that has to be a fact and not a hope.
        let gradient = Gradient::new(0.0, 0.0, 0.0, 0.0, [1.0; 4], [0.0; 4], 30.0);
        assert!(gradient.axis[2] > 0.0);
        assert!(fraction(&gradient, 0.0, 0.0).is_finite());
    }

    /// The fraction of the picture the window shows, as the shader would interpolate it:
    /// `uv` is the rectangle of texture the quad's corners name, so what it covers is the
    /// difference across it.
    fn shown(quad: &PictureQuad) -> (f32, f32) {
        (quad.uv[2] - quad.uv[0], quad.uv[3] - quad.uv[1])
    }

    /// Whether a quad's `uv` is this rectangle of the texture, to within a rounding.
    ///
    /// Every number a picture is described by is a division or two of numbers that are not
    /// powers of two, so the arithmetic here is compared the way the gradient's is: an
    /// exact comparison would be a test of the compiler's rounding rather than of the crop.
    fn shows(quad: &PictureQuad, uv: [f32; 4]) -> bool {
        quad.uv
            .iter()
            .zip(uv)
            .all(|(got, want)| (got - want).abs() < 1e-6)
    }

    #[test]
    fn a_picture_the_shape_of_the_window_is_shown_whole() {
        let quad = PictureQuad::new((800.0, 600.0), (1600, 1200), 1.0);
        assert_eq!(shown(&quad), (1.0, 1.0));
        assert!(shows(&quad, [0.0, 0.0, 1.0, 1.0]));
        // And it covers the window, which is the other half of the answer.
        assert!(
            quad.rect
                .iter()
                .zip([0.0, 0.0, 800.0, 600.0])
                .all(|(got, want)| (got - want).abs() < 1e-6)
        );
    }

    #[test]
    fn a_picture_wider_than_the_window_keeps_its_middle_and_loses_its_ends() {
        // A 2:1 picture in a square window: the height fills it exactly and the width has
        // twice as much as fits, so half of it — the middle half — is what is left. A
        // scale that took the smaller ratio instead would stretch it, and one that took
        // the larger without centring would keep the left half.
        let quad = PictureQuad::new((500.0, 500.0), (2000, 1000), 1.0);
        assert_eq!(shown(&quad), (0.5, 1.0));
        assert!(shows(&quad, [0.25, 0.0, 0.75, 1.0]));
    }

    #[test]
    fn a_picture_taller_than_the_window_keeps_its_middle_and_loses_its_ends() {
        let quad = PictureQuad::new((500.0, 500.0), (1000, 2000), 1.0);
        assert_eq!(shown(&quad), (1.0, 0.5));
        assert!(shows(&quad, [0.0, 0.25, 1.0, 0.75]));
    }

    #[test]
    fn the_picture_never_shows_outside_itself_in_either_direction() {
        // Every pair of sizes, including the degenerate ones: whatever the window and
        // whatever the picture, nothing sampled is outside the texture — a wrap or a
        // clamp there would be the picture's far edge smeared down one side of the
        // window, which is a bug that only shows up at one window size.
        let sizes = [1.0, 3.0, 640.0, 1920.0];
        let pictures = [(1, 1), (4, 3), (3, 4), (3840, 2160), (1, 900)];
        for width in sizes {
            for height in sizes {
                for picture in pictures {
                    let quad = PictureQuad::new((width, height), picture, 1.0);
                    let (shown_x, shown_y) = shown(&quad);
                    assert!(
                        shown_x > 0.0 && shown_x <= 1.0 && shown_y > 0.0 && shown_y <= 1.0,
                        "{picture:?} in a {width}x{height} window shows {shown_x}x{shown_y}"
                    );
                    assert!(quad.uv[0] >= 0.0 && quad.uv[2] <= 1.0);
                    assert!(quad.uv[1] >= 0.0 && quad.uv[3] <= 1.0);
                }
            }
        }
    }

    #[test]
    fn a_picture_is_cropped_by_the_same_amount_off_both_ends() {
        // Centring, stated as the thing it is rather than as the arithmetic that gets
        // there: the margin on the left is the margin on the right.
        let quad = PictureQuad::new((640.0, 480.0), (4000, 1000), 1.0);
        assert!((quad.uv[0] - (1.0 - quad.uv[2])).abs() < 1e-6);
        assert!((quad.uv[1] - (1.0 - quad.uv[3])).abs() < 1e-6);
    }

    #[test]
    fn the_opacity_reaches_the_instance_untouched() {
        // The frame says how strongly to draw it and the renderer multiplies by it; a
        // clamp here would silently disagree with the configuration that validated it.
        let opacity = PictureQuad::new((10.0, 10.0), (10, 10), 0.4).opacity;
        assert!((opacity - 0.4).abs() < f32::EPSILON);
    }

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
