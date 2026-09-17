//! Turning logical rectangles and text runs into quads and glyph quads.
//!
//! This module is the only place the window's scale is applied. Everything it is handed
//! is in the units DESIGN.md is written in; everything it emits is in the physical
//! pixels [`zet_render::Frame`] holds. That boundary is deliberate: it means a layout
//! bug can be read off the numbers in the code, and a DPI bug can only be in one place.
//!
//! The glyph bitmaps are the exception, and they are not an inconsistency: the source
//! was built at the window's scale, so its raster is already physical and is used at the
//! size it arrives in. The layout's *positions* are the logical half.

use zet_config::Rgb;
use zet_font::{GlyphSpec, Weight};
use zet_render::{GlyphQuad, Quad};

use crate::fonts::GlyphSource;
use crate::geometry::Rect;

/// How one run of chrome text is drawn.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TextStyle {
    size: f32,
    weight: Weight,
    color: Rgb,
    alpha: f32,
    tracking: f32,
}

impl TextStyle {
    /// A run at a size, a weight, and a colour.
    pub(crate) fn new(size: f32, weight: Weight, color: Rgb) -> Self {
        Self {
            size,
            weight,
            color,
            alpha: 1.0,
            tracking: 0.0,
        }
    }

    /// The same run with letter spacing, in ems.
    pub(crate) fn tracking(mut self, em: f32) -> Self {
        self.tracking = em;
        self
    }

    /// The same run at an alpha, which is what the indicator's cross-fade moves.
    pub(crate) fn faded(mut self, alpha: f32) -> Self {
        self.alpha = alpha;
        self
    }

    /// The same run at a fraction of its size, which is what the `#` of a tab's index
    /// is: part of the mark rather than a prefix set at its own size.
    pub(crate) fn scaled(mut self, factor: f32) -> Self {
        self.size *= factor;
        self
    }
}

/// Collects what the layout draws.
///
/// A pair of arrays rather than the frame itself, because the layout is free to
/// interleave a surface with the text on it and the frame is not: its batches are drawn
/// in the order they were opened, so all of the chrome's rectangles go in one batch and
/// all of its glyphs in the next, one draw call each.
pub(crate) struct Painter<'a> {
    fonts: &'a mut dyn GlyphSource,
    quads: &'a mut Vec<Quad>,
    glyphs: &'a mut Vec<GlyphQuad>,
    scale: f32,
}

impl<'a> Painter<'a> {
    /// A painter that appends to two arrays, emitting at `scale`.
    pub(crate) fn new(
        fonts: &'a mut dyn GlyphSource,
        quads: &'a mut Vec<Quad>,
        glyphs: &'a mut Vec<GlyphQuad>,
        scale: f32,
    ) -> Self {
        Self {
            fonts,
            quads,
            glyphs,
            scale,
        }
    }

    /// A rectangle of one colour.
    pub(crate) fn fill(&mut self, rect: Rect, color: Rgb) {
        self.quads.push(Quad::new(
            rect.x * self.scale,
            rect.y * self.scale,
            rect.width * self.scale,
            rect.height * self.scale,
            premultiplied(color, 1.0),
        ));
    }

    /// One logical pixel per pixel of the source's own raster at this role's size.
    ///
    /// The source is built at the window's scale, so its pixels are physical while a
    /// role's size is logical. Both numbers are "per em" in their own space, so their
    /// ratio converts one to the other, and the scale cancels: a role's advance is the
    /// same logical width at every DPI, which is the property that makes the strip's
    /// geometry testable without a window.
    fn unit(&self, size: f32) -> f32 {
        size / self.fonts.metrics().ppem
    }

    /// The line box one text role occupies: how tall it is, and how far down its
    /// baseline sits. Both logical.
    fn line(&self, size: f32) -> (f32, f32) {
        let metrics = self.fonts.metrics();
        let unit = self.unit(size);
        (metrics.cell_height * unit, metrics.baseline * unit)
    }

    /// The baseline that centres one role vertically in `rect`.
    pub(crate) fn baseline_in(&self, rect: Rect, size: f32) -> f32 {
        let (line, baseline) = self.line(size);
        rect.y + (rect.height - line) / 2.0 + baseline
    }

    /// How wide a run would be, for a caller that is measuring rather than drawing.
    ///
    /// The colour is not read by a measurement, so there is nothing to pass. Used by the
    /// strip, which has to know how wide a tab is before it knows where the next one
    /// starts.
    pub(crate) fn advance(&mut self, text: &str, size: f32, weight: Weight) -> f32 {
        let style = TextStyle::new(size, weight, Rgb::BLACK);
        self.width(text, style)
    }

    /// How wide a run would be, in logical pixels.
    pub(crate) fn width(&mut self, text: &str, style: TextStyle) -> f32 {
        let unit = self.unit(style.size);
        let mut total = 0.0;
        for ch in text.chars() {
            if let Some(placed) = self
                .fonts
                .place(GlyphSpec::new(ch).with_weight(style.weight))
            {
                total += placed.advance.mul_add(unit, style.tracking * style.size);
            }
        }
        total
    }

    /// A run of text with its baseline at `baseline`.
    ///
    /// The baseline rather than a top edge, because a tab's index is two sizes on one
    /// line: the `#` is set smaller and would sit wrong if the two runs were positioned
    /// by their tops.
    ///
    /// A glyph the source cannot place takes no space and draws nothing. `place` answers
    /// `None` only for a glyph wider than an entire atlas, so on the chrome's own text
    /// this cannot be reached; leaving the gap out rather than inventing an advance
    /// keeps the strip's arithmetic honest when it is.
    pub(crate) fn text(&mut self, text: &str, x: f32, baseline: f32, style: TextStyle) {
        let unit = self.unit(style.size);
        let color = premultiplied(style.color, style.alpha);
        let mut pen = x;
        for ch in text.chars() {
            let spec = GlyphSpec::new(ch).with_weight(style.weight);
            let Some(placed) = self.fonts.place(spec) else {
                continue;
            };
            if placed.width > 0 && placed.height > 0 {
                let left = placed.left as f32 * unit;
                let top = baseline - placed.top as f32 * unit;
                self.glyphs.push(GlyphQuad::alpha(
                    [
                        (pen + left) * self.scale,
                        top * self.scale,
                        placed.width as f32 * unit * self.scale,
                        placed.height as f32 * unit * self.scale,
                    ],
                    placed.uv,
                    color,
                ));
            }
            pen = placed
                .advance
                .mul_add(unit, pen + style.tracking * style.size);
        }
    }

    /// A run of text centred vertically in `rect` and starting at its left edge.
    pub(crate) fn centered(&mut self, text: &str, rect: Rect, style: TextStyle) {
        let baseline = self.baseline_in(rect, style.size);
        self.text(text, rect.x, baseline, style);
    }
}

/// A colour as a quad or a glyph tint wants it: linear and premultiplied.
///
/// The conversion is `zet_config`'s, because the sRGB transfer function belongs next to
/// the contrast arithmetic that depends on it. The premultiplication is here, because
/// it is a fact about how these two arrays are blended and nothing else in the workspace
/// needs to know it.
fn premultiplied(color: Rgb, alpha: f32) -> [f32; 4] {
    let [r, g, b, a] = color.to_linear_alpha(alpha);
    [r * a, g * a, b * a, a]
}
