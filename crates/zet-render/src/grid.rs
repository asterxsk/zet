//! Turning a terminal's cells into rectangles.
//!
//! This is the whole of the grid's appearance, and it is a pure function of a
//! [`Term`], a [`Theme`], and a [`Metrics`]: nothing here rasterises, allocates, or
//! talks to a device. The pixels come from a [`GlyphSource`], which is the atlas, and
//! the geometry goes into a [`Frame`], which is two arrays. What is left — the part
//! that is actually worth testing — is the set of decisions about *where* and *in what
//! colour*, and every one of them is asserted in this module's tests.
//!
//! # Why the whole grid is walked every frame
//!
//! [`zet_vt::Damage`] knows which rows changed, and the obvious use of it here would be
//! to emit only those rows. That does not work with a frame that is rebuilt from
//! scratch and a surface that is cleared before it: a row left out of the frame is a row
//! that is erased. Repainting only what changed needs the previous frame's rectangles to
//! survive — a retained buffer, plus an explicit erase of wherever the cursor used to
//! be — and until that exists, walking every cell is both correct and cheap. A 200x50
//! grid is ten thousand cells of a hash lookup and a subtraction, and what it saves is
//! the thing that actually costs: the atlas already holds every glyph, so a cell that
//! did not change is a lookup and a rectangle.
//!
//! The damage tracker is not wasted. [`zet_app`] uses it to decide whether to draw at
//! all, which is the difference between an idle terminal and one burning a core to
//! redraw an unchanging picture.
//!
//! [`zet_app`]: https://github.com/asterxsk/zet

use zet_config::rgb::linear_to_srgb;
use zet_config::{CursorSettings, CursorShape, Rgb, Theme};
use zet_font::{GlyphSpec, Metrics, Weight};
use zet_vt::{Attrs, Cell, CellFlags, Pos, Term, UnderlineStyle};

use crate::atlas::Placement;
use crate::frame::{Frame, GlyphQuad, Quad};

/// Where a glyph's pixels are, and where they go.
///
/// The one thing the grid renderer needs from the rest of the crate, and the reason it
/// can be tested without a font, a device, or a texture: a test supplies its own
/// placements and asserts the geometry that came out.
pub trait GlyphSource {
    /// Place one character, rasterising and packing it if this is the first time.
    ///
    /// `None` when the glyph cannot be drawn at all — a character wider than the atlas.
    /// A character nothing on the machine can draw is not `None`: it comes back as the
    /// primary face's `.notdef`, which is a box, and a box is information.
    fn place(&mut self, spec: GlyphSpec) -> Option<Placement>;
}

/// A run of cells the user has selected.
///
/// Held by the caller rather than by the terminal, because a selection is zet's
/// invention: the program writing to the pty has no idea it exists and must not be told.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Selection {
    start: (usize, usize),
    end: (usize, usize),
}

impl Selection {
    /// The selection between two cells, inclusive, in either order.
    #[must_use]
    pub fn new(anchor: Pos, head: Pos) -> Self {
        let a = (anchor.row, anchor.col);
        let b = (head.row, head.col);
        if a <= b {
            Selection { start: a, end: b }
        } else {
            Selection { start: b, end: a }
        }
    }

    /// Whether a cell is inside it.
    #[must_use]
    pub fn contains(&self, row: usize, col: usize) -> bool {
        (row, col) >= self.start && (row, col) <= self.end
    }

    /// Whether any of a row is inside it, which is the cheap test a row needs before it
    /// walks its cells.
    #[must_use]
    pub fn touches_row(&self, row: usize) -> bool {
        row >= self.start.0 && row <= self.end.0
    }

    /// The two corners, from the top left to the bottom right.
    ///
    /// For the caller that has to read the cells rather than paint them — copying a
    /// selection out as text, which is the only other thing a selection is for.
    #[must_use]
    pub const fn bounds(&self) -> ((usize, usize), (usize, usize)) {
        (self.start, self.end)
    }

    /// The columns of one row that are inside it.
    ///
    /// A half-open range, because the caller iterates it. The first row starts at the
    /// selection's own column and the last ends at its own; everything between is whole.
    #[must_use]
    pub fn columns_in(&self, row: usize, cols: usize) -> core::ops::Range<usize> {
        if !self.touches_row(row) {
            return 0..0;
        }
        let start = if row == self.start.0 { self.start.1 } else { 0 };
        let end = if row == self.end.0 {
            self.end.1.saturating_add(1)
        } else {
            cols
        };
        start.min(end)..end.min(cols)
    }
}

/// The find bar's matches, as the grid should paint them.
///
/// Passed beside [`View`] rather than inside it. A selection is one pair of corners and
/// a `Copy` value that belongs in a view; this is a list, and putting it in the view
/// would cost the view its `Copy` and make every caller of [`draw_grid`] care about
/// the find bar whether it has one open or not.
///
/// `areas` is sorted, which is what lets a row find the handful of matches that touch
/// it without walking the list once per cell.
#[derive(Clone, Copy, Default)]
pub struct Marks<'a> {
    /// Every match, oldest first.
    pub areas: &'a [Selection],
    /// Which of them the find bar's arrows are on.
    pub active: Option<usize>,
}

impl<'a> Marks<'a> {
    /// The matches, and which one the arrows are on.
    #[must_use]
    pub const fn new(areas: &'a [Selection], active: Option<usize>) -> Self {
        Marks { areas, active }
    }

    /// The matches that touch one row, and the index each of them has in `areas`.
    fn over(&self, row: usize) -> (usize, &'a [Selection]) {
        // A match that begins above this row and ends on it counts, so the first
        // candidate is the first one whose *end* has reached this row.
        let first = self.areas.partition_point(|area| area.bounds().1.0 < row);
        let rest = &self.areas[first..];
        let count = rest.partition_point(|area| area.bounds().0.0 <= row);
        (first, &rest[..count])
    }
}

/// How the cursor is drawn this frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cursor {
    /// Which shape.
    pub shape: CursorShape,
    /// The thickness in physical pixels of a bar, an underline, or a hollow outline.
    pub thickness: u8,
    /// Whether to draw it at all.
    pub visible: bool,
}

impl Cursor {
    /// Work out this frame's cursor.
    ///
    /// Three inputs, in order of authority. `shown` is the program's: `DECTCEM` off
    /// means the program has deliberately hidden the cursor and no setting overrides
    /// that. An unfocused window is drawn as a hollow outline rather than being hidden
    /// or left blinking, because a terminal you cannot tell is focused is a terminal you
    /// type into by mistake.
    #[must_use]
    pub fn resolve(settings: &CursorSettings, focused: bool, blink_on: bool, shown: bool) -> Self {
        let thickness = settings.thickness;
        if !shown {
            return Cursor {
                shape: settings.shape,
                thickness,
                visible: false,
            };
        }
        if focused {
            // A cursor with blinking off is always lit: the setting says "do not blink",
            // not "do not show".
            Cursor {
                shape: settings.shape,
                thickness,
                visible: !settings.blink || blink_on,
            }
        } else {
            Cursor {
                shape: CursorShape::HollowBlock,
                thickness,
                visible: true,
            }
        }
    }
}

/// Everything about a frame that is not the terminal's contents.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct View {
    /// Where the grid's top-left corner is, in physical pixels. The chrome above the
    /// grid is what makes this non-zero.
    pub origin: (f32, f32),
    /// Whether the window has focus.
    pub focused: bool,
    /// Whether the blink phase is on. Drives both the cursor and `SGR 5` text.
    pub blink_on: bool,
    /// What the user has selected, if anything.
    pub selection: Option<Selection>,
}

impl View {
    /// A view of a focused, blinking, unselected grid at the origin.
    #[must_use]
    pub const fn new() -> Self {
        View {
            origin: (0.0, 0.0),
            focused: true,
            blink_on: true,
            selection: None,
        }
    }
}

impl Default for View {
    fn default() -> Self {
        Self::new()
    }
}

/// Draw a terminal's grid into a frame.
///
/// The frame is appended to, not cleared, so a caller draws the grid and then the chrome
/// over it. Every rectangle is in physical pixels, which is what the device wants and
/// what the metrics already are.
#[allow(clippy::too_many_arguments)]
pub fn draw_grid(
    term: &Term,
    theme: &Theme,
    metrics: &Metrics,
    settings: &CursorSettings,
    view: &View,
    marks: Marks<'_>,
    glyphs: &mut dyn GlyphSource,
    frame: &mut Frame,
) {
    let grid = term.grid();
    let (origin_x, origin_y) = view.origin;
    let modes = term.modes();
    let cursor_at = term.cursor();
    let cursor = Cursor::resolve(settings, view.focused, view.blink_on, modes.cursor_visible);

    // One rectangle for the whole grid, so that a screen of default backgrounds costs
    // one quad instead of one per cell. Chrome is drawn around and over this, and the
    // surface itself is cleared to the theme's ground, which is a third colour.
    frame.begin_quads();
    frame.push_quad(Quad::new(
        origin_x,
        origin_y,
        grid.cols() as f32 * metrics.cell_width,
        grid.rows() as f32 * metrics.cell_height,
        theme.background.to_linear(),
    ));
    frame.end_quads();

    for row in 0..grid.rows() {
        let line = grid.row(row);
        let y = origin_y + row as f32 * metrics.cell_height;

        // Rows are stored trimmed, so a row that has never been written is zero cells
        // long and costs nothing to skip. So does a row the cursor is not on and nothing
        // has selected.
        let cursor_row = cursor.visible && cursor_at.row == row;
        let selected_row = view
            .selection
            .is_some_and(|selection| selection.touches_row(row));
        let (first_mark, marked_row) = marks.over(row);
        if line.content_len() == 0 && !cursor_row && !selected_row && marked_row.is_empty() {
            continue;
        }

        frame.begin_quads();
        frame.begin_glyphs();

        for col in 0..grid.cols() {
            let cell = line.get(col);
            let x = origin_x + col as f32 * metrics.cell_width;
            let rect = [x, y, metrics.cell_width, metrics.cell_height];

            let (foreground, background) = cell_colors(&cell, theme, modes.reverse_video);
            let selected = view
                .selection
                .is_some_and(|selection| selection.contains(row, col));
            let found = marked_row
                .iter()
                .position(|area| area.contains(row, col))
                .map(|at| first_mark + at);

            let ordinary = background.to_array() != theme.background.to_array();
            if ordinary || selected || found.is_some() {
                let fill = if found.is_none() && selected {
                    Some(theme.selection)
                } else {
                    ordinary.then_some(background)
                };
                if let Some(fill) = fill {
                    frame.push_quad(Quad::new(
                        rect[0],
                        rect[1],
                        rect[2],
                        rect[3],
                        fill.to_linear(),
                    ));
                }
                // A match is laid over whatever the cell already was rather than
                // replacing it, so a highlight on a program's own background tints it
                // instead of painting it out. The one the arrows are on is laid on
                // whole; the rest are half, because the grid has one "marked" colour
                // and two weights of it is the whole of what this app knows about
                // depth.
                if let Some(found) = found {
                    let alpha = if marks.active == Some(found) {
                        1.0
                    } else {
                        0.5
                    };
                    frame.push_quad(Quad::new(
                        rect[0],
                        rect[1],
                        rect[2],
                        rect[3],
                        faded(theme.selection, alpha),
                    ));
                }
            }

            let lit_by_cursor = cursor_row && cursor_at.col == col;
            if lit_by_cursor {
                push_cursor(frame, cursor, rect, metrics, theme);
            }

            // A blinking cell is drawn on the lit half of the cycle and left as its
            // background on the other, which is what every terminal does and what the
            // attribute means.
            let blinking_off = cell.attrs.contains(Attrs::BLINK) && !view.blink_on;
            // Half of a wide character must not be drawn on its own: the spacer carries
            // a space, and a leading spacer belongs to a character off the left edge.
            let half_of_a_wide_one =
                !cell.flags.is_empty() && !cell.flags.contains(CellFlags::WIDE_CHAR);

            if !half_of_a_wide_one && !blinking_off && !cell.attrs.contains(Attrs::HIDDEN) {
                // What the glyph is drawn in. A block cursor inverts the text it sits
                // on, which is the design's "the cell's character inverted"; on a cell
                // that is already the theme's background that is exactly the theme's
                // background, and on a coloured cell it is that colour, which stays
                // legible where a fixed colour would not.
                let text_color = if lit_by_cursor && cursor.shape == CursorShape::Block {
                    background
                } else {
                    foreground
                };
                if let Some(spec) = glyph_spec(&cell) {
                    push_glyph(frame, spec, rect, text_color, metrics, glyphs);
                }
            }

            push_decoration(frame, &cell, rect, foreground, metrics);
        }

        frame.end_quads();
        frame.end_glyphs();
    }

    // A row whose every cell was default still opened a batch, and an empty run is a
    // draw call that draws nothing.
    frame.drop_empty_batches();
}

/// The two colours a cell is drawn in, after reverse video and dimming.
///
/// A cell stores `Color`, which may be the theme's default, an ANSI index, or a direct
/// value. Resolving it is the theme's job, and the only decisions here are the ones the
/// theme is not allowed to make: `SGR 7` swaps the pair, and the screen-wide reverse of
/// `DECSCNM` swaps every one of them.
fn cell_colors(cell: &Cell, theme: &Theme, screen_reversed: bool) -> (Rgb, Rgb) {
    let mut foreground = theme.resolve(cell.fg, false);
    let mut background = theme.resolve(cell.bg, true);
    if screen_reversed ^ cell.attrs.contains(Attrs::REVERSE) {
        core::mem::swap(&mut foreground, &mut background);
    }
    if cell.attrs.contains(Attrs::DIM) {
        foreground = dim(foreground, background);
    }
    (foreground, background)
}

/// A colour at less than full opacity, in the linear premultiplied form a quad wants.
///
/// Premultiplied rather than straight, because that is the frame's contract: the device
/// blends `src + dst * (1 - src.a)`, so a colour that has not been multiplied through
/// by its own alpha comes out too bright, and the more transparent it is the brighter
/// it looks.
fn faded(color: Rgb, alpha: f32) -> [f32; 4] {
    let [r, g, b, _] = color.to_linear();
    [r * alpha, g * alpha, b * alpha, alpha]
}

/// A character with its face, or nothing when there is nothing to draw.
///
/// A space produces no glyph rather than a blank one: the atlas would give it a
/// zero-sized placement and the device would skip it anyway, and asking costs a hash
/// lookup per space in the grid, which on a screen of prose is most of it.
fn glyph_spec(cell: &Cell) -> Option<GlyphSpec> {
    if cell.ch == ' ' {
        return None;
    }
    // A combining mark is part of its base character's cluster, and the cluster was
    // composed into `ch` before it reached the grid, so there is one glyph per cell and
    // never a second one stacked on top.
    let mut spec = GlyphSpec::new(cell.ch);
    if cell.attrs.contains(Attrs::BOLD) {
        spec = spec.with_weight(Weight::BOLD);
    }
    if cell.attrs.contains(Attrs::ITALIC) {
        spec = spec.with_italic(true);
    }
    Some(spec)
}

/// Put one glyph's rectangle into the frame.
fn push_glyph(
    frame: &mut Frame,
    spec: GlyphSpec,
    rect: [f32; 4],
    color: Rgb,
    metrics: &Metrics,
    glyphs: &mut dyn GlyphSource,
) {
    let Some(placement) = glyphs.place(spec) else {
        return;
    };
    if placement.width == 0 || placement.height == 0 {
        return;
    }

    // The rasteriser reports the bitmap's position relative to the pen and the baseline.
    // This is the one place those are turned into a corner of a rectangle, and the sign
    // flip lives in `Glyph::offset_in_cell` so that there is only one copy of it.
    let (mut left, top) = (
        placement.left as f32,
        metrics.baseline - placement.top as f32,
    );
    // A face that is not the primary one can be proportional or a different pitch, and
    // the grid's columns do not move for it. A glyph narrower than its cell is centred
    // in the cell rather than left-aligned, which is what makes a proportional fallback
    // look deliberate rather than broken. A wider one is left where its own bearing puts
    // it and is allowed to overflow, because squeezing an ideograph into a Latin cell
    // makes it unreadable.
    let slack = metrics.cell_width - placement.advance;
    if slack > 0.0 {
        left += slack / 2.0;
    }

    let at = [
        rect[0] + left,
        rect[1] + top,
        placement.width as f32,
        placement.height as f32,
    ];
    let quad = if placement.color {
        GlyphQuad::color(at, placement.uv)
    } else {
        GlyphQuad::alpha(at, placement.uv, color.to_linear())
    };
    frame.push_glyph(quad);
}

/// Draw the cursor, which is the one element both planes claim.
fn push_cursor(
    frame: &mut Frame,
    cursor: Cursor,
    rect: [f32; 4],
    metrics: &Metrics,
    theme: &Theme,
) {
    let [x, y, width, height] = rect;
    let color = theme.cursor.to_linear();
    let thickness = f32::from(cursor.thickness.max(1));
    let bar = |frame: &mut Frame, rect: [f32; 4]| {
        frame.push_quad(Quad::new(rect[0], rect[1], rect[2], rect[3], color));
    };

    match cursor.shape {
        CursorShape::Block => bar(frame, [x, y, width, height]),
        CursorShape::Bar => bar(frame, [x, y, thickness, height]),
        CursorShape::Underline => bar(frame, [x, y + height - thickness, width, thickness]),
        // An outline rather than a fill, so the character underneath stays readable.
        // This is also what an unfocused window draws, and its whole job is to be
        // visible without competing with the text it surrounds.
        CursorShape::HollowBlock => {
            let thin = metrics.underline_thickness.max(1.0);
            bar(frame, [x, y, width, thin]);
            bar(frame, [x, y + height - thin, width, thin]);
            bar(frame, [x, y, thin, height]);
            bar(frame, [x + width - thin, y, thin, height]);
        }
    }
}

/// Draw the underline and the strikeout.
fn push_decoration(frame: &mut Frame, cell: &Cell, rect: [f32; 4], color: Rgb, metrics: &Metrics) {
    let style = cell.attrs.underline_style();
    if style != UnderlineStyle::None && !cell.attrs.contains(Attrs::HIDDEN) {
        let thickness = metrics.underline_thickness.max(1.0);
        push_rule(
            frame,
            style,
            rect[0],
            rect[1] + metrics.underline_top,
            rect[2],
            thickness,
            color,
        );
    }
    if cell.attrs.contains(Attrs::STRIKETHROUGH) && !cell.attrs.contains(Attrs::HIDDEN) {
        let thickness = metrics.underline_thickness.max(1.0);
        frame.push_quad(Quad::new(
            rect[0],
            rect[1] + metrics.strikeout_top,
            rect[2],
            thickness,
            color.to_linear(),
        ));
    }
}

/// Draw one style of underline across a cell.
///
/// The four decorated styles are drawn as runs of short bars rather than as a stroked
/// path, because the pipeline draws rectangles and a rectangle is enough to say
/// "dotted", "dashed", and "wavy" at the sizes an underline occupies. Every pattern is
/// pitched off the stroke width, so a thicker underline is a larger pattern rather than
/// a denser one.
fn push_rule(
    frame: &mut Frame,
    style: UnderlineStyle,
    x: f32,
    y: f32,
    width: f32,
    thickness: f32,
    color: Rgb,
) {
    let color = color.to_linear();
    let mut bar = |x: f32, y: f32, width: f32| {
        frame.push_quad(Quad::new(x, y, width, thickness, color));
    };

    match style {
        UnderlineStyle::None => {}
        UnderlineStyle::Single => bar(x, y, width),
        UnderlineStyle::Double => {
            bar(x, y, width);
            bar(x, y + thickness * 2.0, width);
        }
        UnderlineStyle::Dotted => dash(x, width, thickness * 3.0, thickness, |_, at, w| {
            bar(at, y, w);
        }),
        UnderlineStyle::Dashed => dash(x, width, thickness * 6.0, thickness * 3.0, |_, at, w| {
            bar(at, y, w);
        }),
        // A wave drawn as a staircase: two rows, alternating. At the one and two pixel
        // strokes an underline is ever set at, this reads as a wave, and it costs two
        // rectangles per period instead of a stroked path.
        UnderlineStyle::Curly => dash(x, width, thickness * 2.0, thickness, |index, at, w| {
            let step = if index % 2 == 0 { 0.0 } else { thickness };
            bar(at, y + step, w);
        }),
    }
}

/// Lay `mark`-wide marks from `x` across `width`, one every `pitch`.
///
/// The marks are numbered from zero so that a pattern which alternates has something to
/// alternate on: the marks always land on multiples of the pitch, so their position says
/// nothing about which one they are.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn dash(x: f32, width: f32, pitch: f32, mark: f32, mut draw: impl FnMut(u32, f32, f32)) {
    if pitch <= 0.0 {
        return;
    }
    let count = (width / pitch).ceil() as u32;
    for index in 0..count {
        let at = x + index as f32 * pitch;
        let remaining = (x + width - at).max(0.0);
        if remaining <= 0.0 {
            break;
        }
        draw(index, at, mark.min(remaining));
    }
}

/// Move a colour halfway toward another in linear space.
///
/// Not `SGR 2` as a metaphor: linear space is where "half as bright" is a division by
/// two, and halving an sRGB byte instead would dim a light foreground by far less than
/// a dark one.
fn dim(foreground: Rgb, background: Rgb) -> Rgb {
    let (from, to) = (foreground.to_linear(), background.to_linear());
    let mixed = |channel: usize| linear_to_srgb(f32::midpoint(from[channel], to[channel]));
    Rgb::new(mixed(0), mixed(1), mixed(2))
}

#[cfg(test)]
mod tests {
    // A rectangle is four floats and the assertion is that they are the four it should
    // be. Every one of them is a whole number by construction, so an epsilon would be a
    // way to pass while being wrong.
    #![allow(clippy::float_cmp)]

    use super::*;
    use crate::frame::BatchKind;
    use zet_config::theme::ZET_DARK;
    use zet_vt::{Parser, Term};

    /// A cell eight by eighteen with the baseline fourteen pixels down. Every dimension
    /// is a whole number and no two are equal, so a test that reads the wrong one says
    /// so rather than passing.
    const METRICS: Metrics = Metrics {
        ppem: 16.0,
        cell_width: 8.0,
        cell_height: 18.0,
        baseline: 14.0,
        underline_top: 16.0,
        underline_thickness: 1.0,
        strikeout_top: 9.0,
        cap_height: 11.0,
        x_height: 8.0,
    };

    /// Seven by nine of ink with its top ten pixels above the baseline, which puts its
    /// top-left corner four pixels down the cell.
    const INK: (i32, i32, u32, u32) = (0, 10, 7, 9);

    struct FakeGlyphs {
        advance: f32,
        asked: Vec<GlyphSpec>,
    }

    impl FakeGlyphs {
        fn new() -> Self {
            FakeGlyphs {
                advance: METRICS.cell_width,
                asked: Vec::new(),
            }
        }

        fn with_advance(advance: f32) -> Self {
            FakeGlyphs {
                advance,
                asked: Vec::new(),
            }
        }
    }

    impl GlyphSource for FakeGlyphs {
        fn place(&mut self, spec: GlyphSpec) -> Option<Placement> {
            self.asked.push(spec);
            let (left, top, width, height) = INK;
            Some(Placement {
                uv: [0.0, 0.0, 0.25, 0.25],
                left,
                top,
                width,
                height,
                advance: self.advance,
                color: false,
            })
        }
    }

    fn term(cols: usize, rows: usize, bytes: &[u8]) -> Term {
        let mut term = Term::new(cols, rows);
        let mut parser = Parser::new();
        parser.advance_slice(bytes, &mut term);
        term
    }

    fn render(term: &Term, view: &View) -> Frame {
        let mut frame = Frame::new();
        draw_grid(
            term,
            &ZET_DARK,
            &METRICS,
            &CursorSettings::default(),
            view,
            Marks::default(),
            &mut FakeGlyphs::new(),
            &mut frame,
        );
        frame
    }

    fn render_with(term: &Term, view: &View, glyphs: &mut FakeGlyphs) -> Frame {
        let mut frame = Frame::new();
        draw_grid(
            term,
            &ZET_DARK,
            &METRICS,
            &CursorSettings::default(),
            view,
            Marks::default(),
            glyphs,
            &mut frame,
        );
        frame
    }

    /// Every rectangle drawn in one colour.
    fn quads_of(frame: &Frame, color: [f32; 4]) -> Vec<Quad> {
        frame
            .quads
            .iter()
            .copied()
            .filter(|quad| quad.color == color)
            .collect()
    }

    #[test]
    fn a_blank_screen_is_one_rectangle_under_the_cursor() {
        let frame = render(&Term::new(4, 2), &View::new());
        assert!(frame.glyphs.is_empty());
        // One rectangle fills the grid, because a screen of default backgrounds should
        // not be one rectangle per cell.
        assert_eq!(frame.quads[0].rect, [0.0, 0.0, 32.0, 36.0]);
        assert_eq!(frame.quads[0].color, ZET_DARK.background.to_linear());
        // And the cursor, which is on the blank first cell.
        assert_eq!(frame.quads.len(), 2);
    }

    #[test]
    fn a_character_becomes_one_glyph_in_its_cell() {
        let frame = render(&term(4, 2, b"A"), &View::new());
        assert_eq!(frame.glyphs.len(), 1);
        let glyph = frame.glyphs[0];
        assert_eq!(glyph.rect, [0.0, 14.0 - 10.0, 7.0, 9.0]);
        assert_eq!(glyph.color, ZET_DARK.foreground.to_linear());
        assert_eq!(glyph.flags, crate::frame::glyph_flags::ALPHA);
    }

    #[test]
    fn a_space_is_not_a_glyph_and_not_a_lookup() {
        // A screen of prose is mostly spaces, and asking the atlas for each one is a
        // hash lookup to be told there is nothing there.
        let mut glyphs = FakeGlyphs::new();
        let frame = render_with(&term(4, 1, b"a b"), &View::new(), &mut glyphs);
        assert_eq!(frame.glyphs.len(), 2);
        assert_eq!(glyphs.asked.len(), 2, "{:?}", glyphs.asked);
    }

    #[test]
    fn a_coloured_background_is_a_rectangle_behind_the_text() {
        let frame = render(&term(4, 1, b"\x1b[41mA"), &View::new());
        let background = quads_of(&frame, ZET_DARK.ansi[1].to_linear());
        assert_eq!(background.len(), 1);
        assert_eq!(background[0].rect, [0.0, 0.0, 8.0, 18.0]);
        // The row's rectangles are submitted before the row's glyphs, which is the whole
        // of layering: a background drawn afterwards would be drawn over the text.
        let kinds: Vec<BatchKind> = frame.batches.iter().map(|batch| batch.kind).collect();
        assert_eq!(
            kinds,
            vec![BatchKind::Quads, BatchKind::Quads, BatchKind::Glyphs]
        );
    }

    #[test]
    fn a_default_background_is_not_a_rectangle() {
        let frame = render(&term(4, 1, b"A"), &View::new());
        assert_eq!(quads_of(&frame, ZET_DARK.background.to_linear()).len(), 1);
    }

    #[test]
    fn reverse_video_swaps_the_two_colours() {
        let frame = render(&term(4, 1, b"\x1b[7mA"), &View::new());
        assert_eq!(frame.glyphs[0].color, ZET_DARK.background.to_linear());
        assert_eq!(
            quads_of(&frame, ZET_DARK.foreground.to_linear()).len(),
            1,
            "the cell should be painted in what is now its background"
        );
    }

    #[test]
    fn the_screen_wide_reverse_mode_swaps_every_cell() {
        // `DECSCNM`. A program that sets it is asking for the whole screen reversed, and
        // a cell that also sets `SGR 7` cancels it out rather than reversing twice.
        let term = term(4, 1, b"\x1b[?5hA");
        assert!(term.modes().reverse_video);
        let frame = render(&term, &View::new());
        assert_eq!(frame.glyphs[0].color, ZET_DARK.background.to_linear());
    }

    #[test]
    fn a_dim_cell_is_drawn_halfway_to_its_background() {
        let frame = render(&term(4, 1, b"\x1b[2mA"), &View::new());
        let dimmed = frame.glyphs[0].color;
        assert_ne!(dimmed, ZET_DARK.foreground.to_linear());

        // Halfway in linear space, and not halfway in sRGB: halving an sRGB byte dims a
        // light foreground far less than a dark one, which is the whole reason the mix
        // happens where it does.
        let (from, to) = (
            ZET_DARK.foreground.to_linear(),
            ZET_DARK.background.to_linear(),
        );
        let halfway = Rgb::new(
            linear_to_srgb(f32::midpoint(from[0], to[0])),
            linear_to_srgb(f32::midpoint(from[1], to[1])),
            linear_to_srgb(f32::midpoint(from[2], to[2])),
        );
        assert_eq!(dimmed, halfway.to_linear());

        // And the two mixes are actually different, or the test would pass either way.
        let (fg, bg) = (
            ZET_DARK.foreground.to_array(),
            ZET_DARK.background.to_array(),
        );
        let srgb_mix = Rgb::new(
            u16::midpoint(u16::from(fg[0]), u16::from(bg[0])) as u8,
            u16::midpoint(u16::from(fg[1]), u16::from(bg[1])) as u8,
            u16::midpoint(u16::from(fg[2]), u16::from(bg[2])) as u8,
        );
        assert_ne!(dimmed, srgb_mix.to_linear(), "that was the sRGB mix");
    }

    #[test]
    fn a_hidden_cell_paints_its_background_and_nothing_else() {
        // `SGR 8`. The characters are still there and must occupy the space, and none of
        // them may be readable — including through the underline.
        let frame = render(&term(4, 1, b"\x1b[8;4;41mA"), &View::new());
        assert!(frame.glyphs.is_empty());
        let background = quads_of(&frame, ZET_DARK.ansi[1].to_linear());
        assert_eq!(background.len(), 1, "the background is still painted");
        assert!(quads_of(&frame, ZET_DARK.foreground.to_linear()).is_empty());
    }

    #[test]
    fn the_cursor_is_a_block_in_the_cursor_colour_over_the_cell_it_sits_on() {
        let frame = render(&term(4, 1, b"A"), &View::new());
        let cursor = quads_of(&frame, ZET_DARK.cursor.to_linear());
        assert_eq!(cursor.len(), 1);
        assert_eq!(cursor[0].rect, [8.0, 0.0, 8.0, 18.0]);
    }

    #[test]
    fn the_character_under_a_block_cursor_is_inverted() {
        let term = term(4, 1, b"A");
        let view = View {
            selection: None,
            ..View::new()
        };
        let frame = render(&term, &view);
        // A single character on one cell, then the cursor moves to the next one. The
        // cursor's own cell is blank, so move it back to prove the inversion.
        assert_eq!(frame.glyphs.len(), 1);

        let mut back = term;
        back.grid_mut().damage_mut().mark_all();
        let mut parser = Parser::new();
        parser.advance_slice(b"\x1b[1;1H", &mut back);
        let frame = render(&back, &view);
        assert_eq!(frame.glyphs[0].color, ZET_DARK.background.to_linear());
    }

    #[test]
    fn an_unfocused_window_draws_a_hollow_cursor_and_never_hides_it() {
        let view = View {
            focused: false,
            blink_on: false,
            ..View::new()
        };
        let frame = render(&term(4, 1, b"A"), &view);
        // Four edges, and it is lit even though the blink phase is off.
        assert_eq!(quads_of(&frame, ZET_DARK.cursor.to_linear()).len(), 4);
    }

    #[test]
    fn a_blinking_cursor_is_absent_on_the_off_phase() {
        let view = View {
            blink_on: false,
            ..View::new()
        };
        let frame = render(&term(4, 1, b"A"), &view);
        assert!(quads_of(&frame, ZET_DARK.cursor.to_linear()).is_empty());
    }

    #[test]
    fn a_program_that_hides_the_cursor_is_obeyed() {
        // `DECTCEM` off outranks every setting there is.
        let frame = render(&term(4, 1, b"\x1b[?25lA"), &View::new());
        assert!(quads_of(&frame, ZET_DARK.cursor.to_linear()).is_empty());
    }

    #[test]
    fn a_bar_cursor_is_a_stripe_at_the_left_and_an_underline_one_at_the_bottom() {
        let settings = |shape| CursorSettings {
            shape,
            blink: false,
            thickness: 2,
        };
        let term = term(4, 1, b"A");
        let mut frame = Frame::new();
        draw_grid(
            &term,
            &ZET_DARK,
            &METRICS,
            &settings(CursorShape::Bar),
            &View::new(),
            Marks::default(),
            &mut FakeGlyphs::new(),
            &mut frame,
        );
        let bar = quads_of(&frame, ZET_DARK.cursor.to_linear());
        assert_eq!(bar[0].rect, [8.0, 0.0, 2.0, 18.0]);

        let mut frame = Frame::new();
        draw_grid(
            &term,
            &ZET_DARK,
            &METRICS,
            &settings(CursorShape::Underline),
            &View::new(),
            Marks::default(),
            &mut FakeGlyphs::new(),
            &mut frame,
        );
        let underline = quads_of(&frame, ZET_DARK.cursor.to_linear());
        assert_eq!(underline[0].rect, [8.0, 16.0, 8.0, 2.0]);
    }

    #[test]
    fn a_selection_paints_behind_the_text() {
        let mut term = term(4, 1, b"ab");
        let mut parser = Parser::new();
        parser.advance_slice(b"\x1b[1;4H", &mut term);
        let view = View {
            selection: Some(Selection::new(Pos::new(0, 0), Pos::new(0, 1))),
            ..View::new()
        };
        let frame = render(&term, &view);
        let selected = quads_of(&frame, ZET_DARK.selection.to_linear());
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].rect, [0.0, 0.0, 8.0, 18.0]);
        // The text keeps its own colour and is drawn over the highlight.
        assert_eq!(frame.glyphs[0].color, ZET_DARK.foreground.to_linear());
    }

    #[test]
    fn a_selection_given_backwards_covers_the_same_cells() {
        let forwards = Selection::new(Pos::new(1, 2), Pos::new(3, 4));
        let backwards = Selection::new(Pos::new(3, 4), Pos::new(1, 2));
        assert_eq!(forwards, backwards);
        assert!(forwards.contains(2, 0));
        assert!(!forwards.contains(0, 0));
        assert!(forwards.touches_row(1));
        assert!(!forwards.touches_row(0));
    }

    #[test]
    fn a_selected_blank_row_is_still_walked() {
        // The row is trimmed to nothing and would otherwise be skipped, and a selection
        // over empty space has to be visible.
        let term = Term::new(4, 2);
        let view = View {
            selection: Some(Selection::new(Pos::new(1, 3), Pos::new(1, 3))),
            ..View::new()
        };
        let frame = render(&term, &view);
        let selected = quads_of(&frame, ZET_DARK.selection.to_linear());
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].rect, [24.0, 18.0, 8.0, 18.0]);
    }

    #[test]
    fn an_underline_is_a_bar_at_the_metrics_row() {
        let frame = render(&term(4, 1, b"\x1b[4mA"), &View::new());
        let bars = quads_of(&frame, ZET_DARK.foreground.to_linear());
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].rect, [0.0, 16.0, 8.0, 1.0]);
    }

    #[test]
    fn a_double_underline_is_two_bars_and_a_strikeout_is_above_the_baseline() {
        let frame = render(&term(4, 1, b"\x1b[21mA"), &View::new());
        let bars = quads_of(&frame, ZET_DARK.foreground.to_linear());
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].rect[1], 16.0);
        assert_eq!(bars[1].rect[1], 18.0);

        let frame = render(&term(4, 1, b"\x1b[9mA"), &View::new());
        let bars = quads_of(&frame, ZET_DARK.foreground.to_linear());
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].rect, [0.0, 9.0, 8.0, 1.0]);
    }

    #[test]
    fn the_decorated_underlines_are_patterns_and_not_the_same_bar() {
        let bars = |bytes: &[u8]| {
            let frame = render(&term(4, 1, bytes), &View::new());
            quads_of(&frame, ZET_DARK.foreground.to_linear()).len()
        };
        let dotted = bars(b"\x1b[4:4mA");
        let dashed = bars(b"\x1b[4:5mA");
        let curly = bars(b"\x1b[4:3mA");
        assert!(dotted > 1 && dashed > 1 && curly > 1);
        assert_ne!(dotted, dashed, "the two patterns should not be identical");
        // A wave moves between two rows; a dotted line does not.
        let wavy = render(&term(4, 1, b"\x1b[4:3mA"), &View::new());
        let mut rows: Vec<u32> = quads_of(&wavy, ZET_DARK.foreground.to_linear())
            .iter()
            .map(|quad| quad.rect[1].to_bits())
            .collect();
        rows.sort_unstable();
        rows.dedup();
        assert_eq!(rows.len(), 2, "a wave has a top and a bottom");
    }

    #[test]
    fn a_wide_character_draws_once_and_its_spacer_draws_nothing() {
        let term = term(4, 1, "\u{4e2d}".as_bytes());
        assert!(
            term.grid()
                .row(0)
                .get(0)
                .flags
                .contains(CellFlags::WIDE_CHAR)
        );
        assert!(term.grid().row(0).get(1).is_wide_spacer());
        let mut glyphs = FakeGlyphs::new();
        let frame = render_with(&term, &View::new(), &mut glyphs);
        assert_eq!(frame.glyphs.len(), 1);
        assert_eq!(glyphs.asked.len(), 1);
        assert_eq!(glyphs.asked[0].ch, '\u{4e2d}');
    }

    #[test]
    fn a_face_narrower_than_the_cell_is_centred_in_it() {
        // The rule that keeps the columns from moving: a fallback that is proportional or
        // a different pitch draws inside the cell it was given, and the cell does not
        // grow to fit it.
        let mut glyphs = FakeGlyphs::with_advance(4.0);
        let frame = render_with(&term(4, 1, b"A"), &View::new(), &mut glyphs);
        assert_eq!(frame.glyphs[0].rect[0], 2.0);
    }

    #[test]
    fn a_face_wider_than_the_cell_overflows_to_the_right() {
        let mut glyphs = FakeGlyphs::with_advance(16.0);
        let frame = render_with(&term(4, 1, b"A"), &View::new(), &mut glyphs);
        assert_eq!(frame.glyphs[0].rect[0], 0.0);
    }

    #[test]
    fn the_grid_is_where_the_view_says_it_is() {
        // The chrome is above the grid, so the grid's origin is not the window's.
        let view = View {
            origin: (16.0, 40.0),
            ..View::new()
        };
        let frame = render(&term(4, 1, b"A"), &view);
        assert_eq!(frame.quads[0].rect, [16.0, 40.0, 32.0, 18.0]);
        assert_eq!(frame.glyphs[0].rect[1], 40.0 + 4.0);
        assert_eq!(frame.glyphs[0].rect[0], 16.0);
    }

    #[test]
    fn bold_and_italic_ask_the_font_for_their_own_face() {
        let mut glyphs = FakeGlyphs::new();
        let _ = render_with(&term(4, 1, b"\x1b[1;3mA"), &View::new(), &mut glyphs);
        assert_eq!(glyphs.asked[0].weight, Weight::BOLD);
        assert!(glyphs.asked[0].italic);
    }

    #[test]
    fn a_glyph_that_cannot_be_placed_draws_nothing_rather_than_a_wrong_rectangle() {
        struct Nothing;
        impl GlyphSource for Nothing {
            fn place(&mut self, _spec: GlyphSpec) -> Option<Placement> {
                None
            }
        }
        let mut frame = Frame::new();
        draw_grid(
            &term(4, 1, b"A"),
            &ZET_DARK,
            &METRICS,
            &CursorSettings::default(),
            &View::new(),
            Marks::default(),
            &mut Nothing,
            &mut frame,
        );
        assert!(frame.glyphs.is_empty());
    }

    #[test]
    fn a_colour_glyph_is_drawn_as_it_is_and_not_tinted() {
        struct Colour;
        impl GlyphSource for Colour {
            fn place(&mut self, _spec: GlyphSpec) -> Option<Placement> {
                Some(Placement {
                    uv: [0.0, 0.0, 0.25, 0.25],
                    left: 0,
                    top: 10,
                    width: 7,
                    height: 9,
                    advance: METRICS.cell_width,
                    color: true,
                })
            }
        }
        let mut frame = Frame::new();
        draw_grid(
            &term(4, 1, b"A"),
            &ZET_DARK,
            &METRICS,
            &CursorSettings::default(),
            &View::new(),
            Marks::default(),
            &mut Colour,
            &mut frame,
        );
        assert_eq!(frame.glyphs[0].flags, crate::frame::glyph_flags::COLOR);
    }

    #[test]
    fn no_batch_is_left_empty() {
        // A row whose every cell was default opens a batch and pushes nothing into it,
        // and an empty run is a draw call that draws nothing.
        let frame = render(&Term::new(4, 8), &View::new());
        assert!(frame.batches.iter().all(|batch| !batch.range.is_empty()));
    }
}
