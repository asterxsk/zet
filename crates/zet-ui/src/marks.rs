//! The window's own controls, drawn as geometry.
//!
//! The three caption buttons used to be text: `−`, `□`, and `×` taken from IBM Plex
//! Sans. That is not what they are. A caption button is a piece of Windows, it is hit
//! hundreds of times a day, and its marks are drawn to Windows' own metrics — a
//! hairline stroke, a square exactly ten pixels on a side, a cross at forty-five
//! degrees. Plex's `□` is a geometric shape at the weight of the surrounding text, its
//! `×` is a multiplication sign, and neither is the thing a user is aiming at. Worse,
//! [`crate::fonts::settings`] loads the chrome's face with no fallback chain, so a mark
//! Plex happens not to carry is not a missing mark — it is `.notdef`, a box, drawn in
//! the place of the control.
//!
//! So the marks are geometry, and this module rasterises them.
//!
//! # Why the CPU, and why rectangles
//!
//! The alternative was an atlas entry per mark, and it was rejected on the layering
//! rather than on the pixels. [`zet_render::GlyphSource`] is keyed by a face and a glyph
//! id; a mark has neither, so an atlas route means either a new key type threaded
//! through the renderer or a synthetic face invented for four shapes. Both are more
//! machinery than the thing they draw, and [`crate::Chrome`] is already documented as
//! "pure computation on the CPU" that "fills a frame with rectangles".
//!
//! # Hinted, not antialiased
//!
//! Every edge of every mark is snapped to the device pixel grid, and that is the whole
//! reason this module works in physical pixels rather than in the design's logical ones.
//! The first version did not snap: it antialiased a one-pixel stroke with a coverage
//! ramp, which put half a pixel of grey on each of two rows and drew the minimise bar as
//! a soft two-pixel smear. Correct antialiasing of a line that is exactly one pixel wide
//! and exactly one pixel out of place is still the wrong picture. Windows hints these
//! glyphs for the same reason.
//!
//! So the straight marks are whole-pixel rectangles and the strokes that scale are
//! rounded to whole pixels before anything is drawn. The cross is the exception: two
//! forty-five-degree lines cannot sit on a grid of squares, so it keeps the coverage
//! ramp, which is what makes its arms smooth rather than stepped.

use zet_config::Rgb;

use crate::geometry::Rect;
use crate::paint::Painter;

/// The side of the square a caption mark is drawn in, in logical pixels.
///
/// Windows' own figure for the ten-pixel box its caption glyphs sit in.
pub(crate) const MARK_BOX: f32 = 10.0;

/// How far the restore mark's back square is offset, in logical pixels.
const RESTORE_OFFSET: f32 = 2.0;

/// One of the window's controls, as a shape rather than as a character.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Mark {
    /// A single horizontal bar.
    Minimize,
    /// One hollow square.
    Maximize,
    /// Two overlapping squares, for a window that is already maximized.
    Restore,
    /// A diagonal cross.
    Close,
}

/// How thick a mark's strokes are, in whole physical pixels.
///
/// Snapped rather than scaled continuously. A one-pixel hairline at 125% is 1.25 pixels,
/// which the grid cannot hold, and the whole point of a caption mark is that it is a hard
/// edge — so it stays one pixel until there is a whole second pixel to spend.
fn stroke(scale: f32) -> f32 {
    scale.round().max(1.0)
}

/// Draw one mark, centred in `button`.
///
/// `button` is the whole clickable rectangle in logical pixels — 46 by 40, Windows'
/// metric — and the mark is centred inside it, which is why this takes the button rather
/// than a box.
pub(crate) fn draw(paint: &mut Painter<'_>, mark: Mark, button: Rect, color: Rgb, scale: f32) {
    // The centre snapped to a pixel boundary, and the box measured out from it in whole
    // pixels. Everything straight below is then a whole number of pixels wide and lands
    // on the grid, which is the entire difference between a caption glyph and a grey
    // rectangle.
    let (cx, cy) = button.center();
    let centre_x = (cx * scale).round();
    let centre_y = (cy * scale).round();
    let half = (MARK_BOX * scale / 2.0).round().max(1.0);
    let thickness = stroke(scale);

    let left = centre_x - half;
    let top = centre_y - half;
    let side = half * 2.0;

    match mark {
        Mark::Minimize => {
            // Centred on the middle row, and one row tall at 100% DPI.
            let bar = centre_y - (thickness / 2.0).floor();
            paint.physical(left, bar, side, thickness, color, 1.0);
        }
        Mark::Maximize => square(paint, left, top, side, thickness, color),
        Mark::Restore => {
            let offset = (RESTORE_OFFSET * scale).round().max(1.0);
            // The back square shows only the two edges the front one does not cover. A
            // full outline behind a full outline puts both strokes on the same pixels in
            // the corner where they overlap, which draws a blot rather than two windows.
            paint.physical(left + offset, top - offset, side, thickness, color, 1.0);
            paint.physical(
                left + offset + side - thickness,
                top - offset,
                thickness,
                side,
                color,
                1.0,
            );
            square(paint, left, top, side, thickness, color);
        }
        Mark::Close => cross(paint, left, top, side, thickness, color),
    }
}

/// A hollow square, stroked `thickness`, as four bars.
fn square(paint: &mut Painter<'_>, left: f32, top: f32, side: f32, thickness: f32, color: Rgb) {
    // The top and bottom bars run the full width so that the corners are covered exactly
    // once; the sides then only have to span what is between them.
    let inner = side - thickness * 2.0;
    paint.physical(left, top, side, thickness, color, 1.0);
    paint.physical(left, top + side - thickness, side, thickness, color, 1.0);
    if inner > 0.0 {
        paint.physical(left, top + thickness, thickness, inner, color, 1.0);
        paint.physical(
            left + side - thickness,
            top + thickness,
            thickness,
            inner,
            color,
            1.0,
        );
    }
}

/// Two diagonals across the box, antialiased one pixel at a time.
fn cross(paint: &mut Painter<'_>, left: f32, top: f32, side: f32, thickness: f32, color: Rgb) {
    let (right, bottom) = (left + side, top + side);
    let half = thickness / 2.0;
    let reach = half + 1.0;

    for y in (top - reach).floor() as i32..(bottom + reach).ceil() as i32 {
        for x in (left - reach).floor() as i32..(right + reach).ceil() as i32 {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            // The larger of the two, not the sum: where the arms meet, one of them
            // already covers the pixel and adding the second would brighten the crossing.
            let coverage = segment(px, py, (left, top), (right, bottom), half).max(segment(
                px,
                py,
                (left, bottom),
                (right, top),
                half,
            ));
            if coverage > 0.0 {
                paint.physical(x as f32, y as f32, 1.0, 1.0, color, coverage);
            }
        }
    }
}

/// How much of the pixel centred at (`px`, `py`) a stroked segment covers.
///
/// The distance to the segment, on a linear ramp half a pixel wide. Approximate for a
/// diagonal and exact for an axis-aligned edge that lands on the grid — which is every
/// other mark in this file, and the reason only this one needs a coverage ramp at all.
fn segment(px: f32, py: f32, from: (f32, f32), to: (f32, f32), half: f32) -> f32 {
    let (ax, ay) = from;
    let (bx, by) = to;
    let (dx, dy) = (bx - ax, by - ay);
    let length_squared = dx.mul_add(dx, dy * dy);
    let t = if length_squared == 0.0 {
        0.0
    } else {
        (((px - ax) * dx + (py - ay) * dy) / length_squared).clamp(0.0, 1.0)
    };
    let (nx, ny) = (ax + t * dx, ay + t * dy);
    let distance = (px - nx).hypot(py - ny);
    (half + 0.5 - distance).clamp(0.0, 1.0)
}
