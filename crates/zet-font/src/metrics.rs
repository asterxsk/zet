//! The cell.
//!
//! Every measurement the renderer and the input layer need in order to know how big a
//! cell is and where the furniture goes inside it, resolved once when the font is
//! loaded and never recomputed per frame.
//!
//! Everything here is in pixels at the current scale, rounded to whole pixels. A
//! terminal's rows and columns have to land on the same pixel grid as the glyphs drawn
//! into them, or the text is soft and the underlines drift; a fractional cell size
//! would also make the column count change when the window moved by one pixel.

use crate::library::Face;

/// The size and internal proportions of one cell.
///
/// The face's own ascent, descent and leading are not kept as separate fields. The
/// cell height is their sum and the baseline is the ascent, so storing all four would
/// be three ways to say one number and one way to say another.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Metrics {
    /// The size the cell was measured at, in pixels per em.
    pub ppem: f32,
    /// The width of every cell, which is the primary face's advance for `M`.
    pub cell_width: f32,
    /// The height of every cell: the face's ascent, descent and leading, never less
    /// than its em box.
    pub cell_height: f32,
    /// How far down from the top of the cell the baseline sits, which is the ascent.
    pub baseline: f32,
    /// From the top of the cell down to the top of an underline stroke.
    pub underline_top: f32,
    /// How thick an underline and a strikeout are.
    pub underline_thickness: f32,
    /// From the top of the cell down to the top of a strikeout stroke.
    pub strikeout_top: f32,
    /// The height of a capital letter above the baseline.
    pub cap_height: f32,
    /// The height of a lowercase `x` above the baseline.
    pub x_height: f32,
}

impl Metrics {
    /// Measure a cell from a face at `ppem` pixels per em.
    ///
    /// `None` when the face cannot be read. Everything is rounded to whole pixels and
    /// no dimension is allowed to reach zero, because a zero-width cell is an infinite
    /// number of columns.
    #[must_use]
    pub fn from_face(face: &Face, ppem: f32) -> Option<Self> {
        let font = face.font()?;
        let scaled = font.metrics(&[]).scale(ppem);

        // The advance of `M` rather than of any other glyph: it is the one a monospaced
        // face is designed around, and for a proportional face it is the conventional
        // stand-in the user has implicitly agreed to by choosing one.
        let cell_width = font
            .glyph_metrics(&[])
            .scale(ppem)
            .advance_width(font.charmap().map('M'))
            .round()
            .max(1.0);

        let ascent = scaled.ascent.round().max(0.0);
        // A face whose own ascent and descent come to less than its em box — several
        // do — would give lines closer together than the glyphs are tall, so the em box
        // is the floor.
        let cell_height =
            (ascent + scaled.descent.round().max(0.0) + scaled.leading.round().max(0.0))
                .max(ppem.round())
                .max(1.0);

        // `swash` reports both offsets relative to the baseline and signed the way the
        // font's tables are: an underline is below the baseline and so negative. The
        // renderer wants a distance down from the top of the cell, which is the same
        // subtraction for both.
        let underline_thickness = scaled.stroke_size.round().max(1.0);
        let underline_top = (ascent - scaled.underline_offset)
            .clamp(0.0, (cell_height - underline_thickness).max(0.0));
        let strikeout_top = (ascent - scaled.strikeout_offset).clamp(0.0, cell_height);

        Some(Self {
            ppem,
            cell_width,
            cell_height,
            baseline: ascent,
            underline_top,
            underline_thickness,
            strikeout_top,
            cap_height: scaled.cap_height.round(),
            x_height: scaled.x_height.round(),
        })
    }

    /// How many columns and rows fit in a window of this many logical pixels.
    ///
    /// Floored, never rounded: a cell that only half fits is a cell the user cannot
    /// read, and a terminal that draws half a column shows a scrollbar for text that
    /// is not there.
    #[must_use]
    pub fn grid_size(&self, width: f32, height: f32) -> (u16, u16) {
        (
            clamp_u16((width / self.cell_width).floor()),
            clamp_u16((height / self.cell_height).floor()),
        )
    }

    /// How many pixels wide and tall a grid of this size is.
    #[must_use]
    pub fn grid_extent(&self, columns: u16, rows: u16) -> (f32, f32) {
        (
            f32::from(columns) * self.cell_width,
            f32::from(rows) * self.cell_height,
        )
    }
}

/// A count that fits a `u16`, which is what the pseudoconsole and the grid both take.
///
/// A window is never 65535 columns wide, but a `Metrics` with a NaN cell width divides
/// to an infinity, and a NaN must not become a column count. Saturating rather than
/// wrapping is the right answer: a grid too big to allocate fails on the allocation,
/// which is a diagnosable failure, instead of on a silently wrapped column count.
fn clamp_u16(value: f32) -> u16 {
    if value.is_nan() {
        return 0;
    }
    // `as` saturates at both bounds, and `value` is already known to be non-negative.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let clamped = value.min(f32::from(u16::MAX)) as u16;
    clamped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{FontLibrary, Weight};

    fn metrics(ppem: f32) -> Metrics {
        let mut library = FontLibrary::new();
        let face = library
            .resolve("Consolas", Weight::NORMAL, false)
            .expect("Consolas is on every Windows install");
        Metrics::from_face(&face, ppem).expect("a readable face")
    }

    /// Assert that a measurement landed on a whole pixel, to the precision `f32` has
    /// after a round trip through the font's units per em.
    fn assert_whole(value: f32, what: &str) {
        assert!(
            value.fract().abs() < 1.0e-4,
            "{what} is {value}, which is not a whole number of pixels"
        );
    }

    #[test]
    fn every_cell_is_a_whole_number_of_pixels() {
        for ppem in [9.0, 11.5, 13.0, 16.0, 21.7] {
            let m = metrics(ppem);
            assert_whole(m.cell_width, "the cell width");
            assert_whole(m.cell_height, "the cell height");
            assert_whole(m.baseline, "the baseline");
        }
    }

    #[test]
    fn a_cell_gets_bigger_with_the_size() {
        let small = metrics(12.0);
        let large = metrics(24.0);
        assert!(large.cell_width > small.cell_width);
        assert!(large.cell_height > small.cell_height);
    }

    #[test]
    fn the_baseline_is_inside_the_cell() {
        let m = metrics(13.0);
        assert!(m.baseline > 0.0);
        assert!(
            m.baseline < m.cell_height,
            "baseline {} is not inside a cell of {}",
            m.baseline,
            m.cell_height
        );
    }

    #[test]
    fn the_cell_is_at_least_as_tall_as_the_em_box() {
        assert!(metrics(13.0).cell_height >= 13.0);
    }

    #[test]
    fn a_monospaced_cell_is_the_advance_of_every_glyph() {
        // The property the whole grid rests on: if `i` and `W` did not advance by the
        // same amount, every column after the first would drift.
        let mut library = FontLibrary::new();
        let face = library
            .resolve("Consolas", Weight::NORMAL, false)
            .expect("a face");
        let m = Metrics::from_face(&face, 13.0).expect("metrics");
        let advances = face
            .font()
            .expect("a readable face")
            .glyph_metrics(&[])
            .scale(13.0);
        assert!((m.cell_width - advances.advance_width(face.glyph_id('i'))).abs() <= 1.0);
        assert!((m.cell_width - advances.advance_width(face.glyph_id('W'))).abs() <= 1.0);
    }

    #[test]
    fn the_underline_sits_below_the_baseline_and_inside_the_cell() {
        let m = metrics(13.0);
        assert!(
            m.underline_top >= m.baseline,
            "an underline at {} is above a baseline at {}",
            m.underline_top,
            m.baseline
        );
        assert!(m.underline_top + m.underline_thickness <= m.cell_height);
        assert!(m.underline_thickness >= 1.0);
    }

    #[test]
    fn the_strikeout_sits_above_the_baseline() {
        let m = metrics(13.0);
        assert!(
            m.strikeout_top < m.baseline,
            "a strikeout at {} is below a baseline at {}",
            m.strikeout_top,
            m.baseline
        );
    }

    #[test]
    fn x_height_is_shorter_than_cap_height() {
        let m = metrics(13.0);
        assert!(m.cap_height > 0.0);
        assert!(m.x_height > 0.0);
        assert!(m.x_height < m.cap_height);
    }

    #[test]
    fn a_window_holds_the_cells_that_fit_in_it() {
        let m = metrics(13.0);
        let (columns, rows) = m.grid_size(800.0, 600.0);
        assert!(columns > 0 && rows > 0);
        // The floor is the promise: the grid never claims a cell it cannot draw.
        let (width, height) = m.grid_extent(columns, rows);
        assert!(width <= 800.0);
        assert!(height <= 600.0);
        let (one_more, _) = m.grid_size(width + m.cell_width, 600.0);
        assert_eq!(one_more, columns + 1);
    }

    #[test]
    fn a_window_smaller_than_one_cell_asks_for_no_cells() {
        let m = metrics(13.0);
        assert_eq!(m.grid_size(2.0, 2.0), (0, 0));
        assert_eq!(m.grid_size(0.0, 0.0), (0, 0));
    }

    #[test]
    fn a_window_that_is_not_a_number_asks_for_nothing_rather_than_everything() {
        let m = metrics(13.0);
        assert_eq!(m.grid_size(f32::NAN, f32::NAN), (0, 0));
        assert_eq!(
            m.grid_size(f32::INFINITY, f32::INFINITY),
            (u16::MAX, u16::MAX)
        );
    }
}
