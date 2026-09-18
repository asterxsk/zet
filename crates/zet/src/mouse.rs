//! The pointer, as `winit` reports it and as zet needs it.
//!
//! The grid is a rectangle of cells, the window is a rectangle of pixels, and the two
//! are related by the cell size in [`Metrics`] and by where [`zet_ui::Chrome`] put the
//! grid. Everything here is that arithmetic, plus the three questions a terminal
//! actually asks about a pointer: which cell, which button, and is it near enough to the
//! window's edge to be a resize.
//!
//! # Why the host does the resize itself
//!
//! The window has no decorations — DESIGN.md draws its own titlebar and caption buttons —
//! and Windows puts its resize borders on the frame that was just turned off. So the hit
//! testing `WM_NCHITTEST` would have done moves here: a band inside each edge, and a
//! corner where two bands meet. A window that cannot be resized by dragging its edge is
//! not a window, and no amount of correct chrome makes up for it.
//!
//! # The units, which are the whole difficulty
//!
//! `winit` reports a pointer position in *physical* pixels, and the event that carries it
//! says so in its type. [`Metrics`] is in *physical* pixels too, because that is what the
//! rasteriser measured and what the GPU draws with. The chrome's [`Rect`] is in *logical*
//! pixels, because it was laid out from a logical window size. So the one thing every
//! function here has to do is divide the cell size by the scale before comparing it to
//! anything the pointer said, and the one thing a caller has to do is hand over a point
//! in the pixels the window was measured in — [`logical`] is that conversion, and a
//! version of this file that forgot either would put a click on the wrong cell by a
//! factor of the user's display scaling, which on a 200% laptop is every click.

// A pointer position is a logical pixel count and a cell is a physical one, so every
// conversion here is a division by a DPI scale and a floor. Both operands are window
// sized, which is nowhere near the range where an `f32` or an `f64` rounds, so each of
// these is exact and an attribute per line saying so would be noise.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use winit::event::{ElementState, MouseButton as WinitButton, MouseScrollDelta};
use winit::window::ResizeDirection;
use zet_font::Metrics;
use zet_input::{MouseAction, MouseButton};
use zet_ui::Rect;
use zet_vt::Pos;

/// The band inside each edge that starts a resize, in logical pixels.
///
/// One CSS pixel at 100% is what Windows itself uses for a resizable frame, and a band
/// wider than that steals clicks from the grid — which is the worse failure, because a
/// resize is recoverable by trying again and a click that never reached the shell is
/// invisible.
pub const BORDER: f64 = 6.0;

/// Which cell a point in the window falls in, or [`None`] when it is outside the grid.
///
/// `grid` is the grid's rectangle in logical pixels and `scale` is the window's DPI,
/// which is what turns the metrics' physical pixels into the logical ones the point was
/// measured in. A point in the chrome — the titlebar, the tab strip, the scrollbar — is
/// not in the grid, and answering with a cell there would send clicks to the shell that
/// were meant for zet.
#[must_use]
pub fn cell(x: f64, y: f64, grid: Rect, metrics: &Metrics, scale: f32) -> Option<Pos> {
    if !grid.contains(x as f32, y as f32) {
        return None;
    }
    let (cell_width, cell_height) = logical_cell(metrics, scale);
    if cell_width <= 0.0 || cell_height <= 0.0 {
        return None;
    }
    let col = ((x - f64::from(grid.x)) / cell_width).floor();
    let row = ((y - f64::from(grid.y)) / cell_height).floor();
    // The rectangle belongs to the window, not to the terminal. A grid that is not a
    // whole number of cells across has a strip of pixels past its last column, and a
    // point in that strip floors to one cell past the end — which the program is then
    // told about as column `cols + 1`. Clamping to the cells the rectangle actually holds
    // keeps the answer inside the grid the program has, and still lets a click on the
    // last pixel of the window land on the last column, which is what a user aiming at it
    // expects.
    //
    // The count is [`cells`]' rather than an arithmetic of this function's own, because
    // the number the program was resized to is the number this has to clamp to, and two
    // ways of counting it disagree.
    let (cols, rows) = cells(grid, metrics, scale);
    let last = |count: u16| f64::from(count.max(1)) - 1.0;
    Some(Pos::new(
        row.clamp(0.0, last(rows)) as usize,
        col.clamp(0.0, last(cols)) as usize,
    ))
}

/// How many whole cells a grid rectangle holds.
///
/// One function because it is one number. [`crate::host::Host::grid_size`] asks this how
/// many columns and rows the session should be told it has, and [`cell`] asks it how far
/// the pointer is allowed to reach; a terminal whose program has 125 columns and whose
/// pointer can only reach 124 has a last column that cannot be clicked in, which is a
/// difference a user meets before anyone reading the code does.
///
/// The order of operations is the whole of it. The rectangle is logical pixels and the
/// cell is physical ones, and multiplying the rectangle by the scale before dividing is
/// not the same sum as dividing by the cell already scaled: at 125% with an eleven-pixel
/// cell the first is exactly 125.0 and the second is 124.99999999999999, whose floor is a
/// column short.
#[must_use]
pub fn cells(grid: Rect, metrics: &Metrics, scale: f32) -> (u16, u16) {
    if metrics.cell_width <= 0.0 || metrics.cell_height <= 0.0 {
        return (0, 0);
    }
    let scale = if scale > 0.0 { f64::from(scale) } else { 1.0 };
    (
        (f64::from(grid.width) * scale / f64::from(metrics.cell_width)) as u16,
        (f64::from(grid.height) * scale / f64::from(metrics.cell_height)) as u16,
    )
}

/// A cell's size in logical pixels.
fn logical_cell(metrics: &Metrics, scale: f32) -> (f64, f64) {
    let scale = if scale > 0.0 { f64::from(scale) } else { 1.0 };
    (
        f64::from(metrics.cell_width) / scale,
        f64::from(metrics.cell_height) / scale,
    )
}

/// A point as the platform reported it, in the logical pixels everything here is in.
///
/// `winit` measures the pointer in physical pixels — `CursorMoved` and the wheel's
/// `PixelDelta` both — while every surface this module hit-tests is measured in logical
/// ones: the chrome's regions, the settings panel, the resize borders, and the grid
/// rectangle the caller passes to [`cell`]. A point stored as it arrived and then tested
/// as if it were logical is a point one scale factor from where the user is pointing,
/// which at 150% is a close caption at the window's right edge that no click can reach
/// and a tab strip that switches to the tab next to the one under the pointer.
///
/// Zero is not a scale, and a division by it would answer with an infinity and put the
/// pointer nowhere at all. Every other value, including a negative one, is left to the
/// caller: the platform is the one that decides what it means.
#[must_use]
pub fn logical(x: f64, y: f64, scale: f32) -> (f64, f64) {
    let scale = if scale > 0.0 { f64::from(scale) } else { 1.0 };
    (x / scale, y / scale)
}

/// Which edge or corner of the window a point is on, if any.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edge {
    /// The top edge.
    North,
    /// The bottom edge.
    South,
    /// The left edge.
    West,
    /// The right edge.
    East,
    /// The top-left corner.
    NorthWest,
    /// The top-right corner.
    NorthEast,
    /// The bottom-left corner.
    SouthWest,
    /// The bottom-right corner.
    SouthEast,
}

impl Edge {
    /// The direction to drag this edge in.
    #[must_use]
    pub const fn direction(self) -> ResizeDirection {
        match self {
            Self::North => ResizeDirection::North,
            Self::South => ResizeDirection::South,
            Self::West => ResizeDirection::West,
            Self::East => ResizeDirection::East,
            Self::NorthWest => ResizeDirection::NorthWest,
            Self::NorthEast => ResizeDirection::NorthEast,
            Self::SouthWest => ResizeDirection::SouthWest,
            Self::SouthEast => ResizeDirection::SouthEast,
        }
    }

    /// The cursor to show while the pointer is here.
    #[must_use]
    pub const fn cursor(self) -> winit::window::CursorIcon {
        match self {
            Self::North | Self::South => winit::window::CursorIcon::NsResize,
            Self::West | Self::East => winit::window::CursorIcon::EwResize,
            Self::NorthWest | Self::SouthEast => winit::window::CursorIcon::NwseResize,
            Self::NorthEast | Self::SouthWest => winit::window::CursorIcon::NeswResize,
        }
    }
}

/// Which edge a point is on, or [`None`] when it is in the body of the window.
///
/// Corners win over edges: a point in the top-left corner is a corner, because a
/// diagonal resize is what a user dragging there is asking for. The band is measured
/// inward from each side, so a window narrower than two bands has a middle that is
/// still not an edge rather than a band that swallowed it.
#[must_use]
pub fn edge(x: f64, y: f64, width: f64, height: f64) -> Option<Edge> {
    // A window narrower than twice the band has no interior, and every point in it would
    // test as some edge or other. Below that size the honest answer is that the window
    // is too small to have resize borders at all, which is also what Windows does.
    if width <= BORDER * 2.0 || height <= BORDER * 2.0 {
        return None;
    }
    let west = x < BORDER;
    let east = x >= width - BORDER;
    let north = y < BORDER;
    let south = y >= height - BORDER;

    Some(match (north, south, west, east) {
        (true, _, true, _) => Edge::NorthWest,
        (true, _, _, true) => Edge::NorthEast,
        (_, true, true, _) => Edge::SouthWest,
        (_, true, _, true) => Edge::SouthEast,
        (true, _, _, _) => Edge::North,
        (_, true, _, _) => Edge::South,
        (_, _, true, _) => Edge::West,
        (_, _, _, true) => Edge::East,
        _ => return None,
    })
}

/// The zet button a `winit` button is.
///
/// [`MouseButton::None`] for a button zet does not forward, which is a mouse with more
/// buttons than a terminal protocol has names for. Sending a guess would be worse than
/// sending nothing: the shell would see a click it cannot attribute to anything.
#[must_use]
pub const fn button(button: WinitButton) -> MouseButton {
    match button {
        WinitButton::Left => MouseButton::Left,
        WinitButton::Middle => MouseButton::Middle,
        WinitButton::Right => MouseButton::Right,
        WinitButton::Back => MouseButton::Back,
        WinitButton::Forward => MouseButton::Forward,
        WinitButton::Other(_) => MouseButton::None,
    }
}

/// What a button event did.
#[must_use]
pub const fn action(state: ElementState) -> MouseAction {
    match state {
        ElementState::Pressed => MouseAction::Press,
        ElementState::Released => MouseAction::Release,
    }
}

/// The button a wheel event is reported as.
///
/// A wheel event carries no button, because a wheel is not one. The terminal protocols
/// encode a scroll as a button press that never gets a release, which is a fact about
/// the wire rather than about the mouse — so the translation lives here, at the edge,
/// rather than in the encoder.
///
/// `lines` is [`wheel`]'s answer, positive away from the user, so the sign is read the
/// same way in both places and a program that scrolls the wrong way would be wrong in
/// exactly one of them rather than in two.
#[must_use]
pub const fn wheel_button(lines: f64) -> MouseButton {
    if lines > 0.0 {
        MouseButton::WheelUp
    } else {
        MouseButton::WheelDown
    }
}

/// How many lines a wheel event scrolled, positive away from the user.
///
/// A line-based delta is already in lines. A pixel-based delta — which is what a
/// precision trackpad sends — is divided by the height of a line, so that a trackpad's
/// fine-grained scroll moves the scrollback at the speed the finger is moving rather
/// than one line per event, which at a hundred events a second is unusable.
///
/// `winit` counts a scroll *away* from the user as positive, which is the opposite of
/// what a terminal's scrollback offset means: positive here moves the view up into
/// history. The sign is flipped once, here, so that nothing below has to remember.
#[must_use]
pub fn wheel(delta: MouseScrollDelta, line_height: f64) -> f64 {
    let lines = match delta {
        MouseScrollDelta::LineDelta(_, y) => f64::from(y),
        MouseScrollDelta::PixelDelta(position) => {
            if line_height <= 0.0 {
                0.0
            } else {
                position.y / line_height
            }
        }
    };
    -lines
}

#[cfg(test)]
mod tests {
    // The comparisons here are exact on purpose: every one of them is an assertion that
    // an arithmetic result equals a literal, not a tolerance test dressed up as one.
    #![allow(clippy::float_cmp)]

    use super::*;

    fn metrics() -> Metrics {
        // A ten by twenty cell at 100%, which is close to the real thing at 13pt and
        // makes every expected value in this module readable as arithmetic.
        Metrics {
            ppem: 13.0,
            cell_width: 10.0,
            cell_height: 20.0,
            baseline: 15.0,
            underline_top: 17.0,
            underline_thickness: 1.0,
            strikeout_top: 10.0,
            cap_height: 9.0,
            x_height: 7.0,
        }
    }

    #[test]
    fn a_point_from_the_platform_is_measured_in_the_pixels_the_window_is() {
        // `CursorMoved` hands over physical pixels. The chrome, the panel, the resize
        // borders and the grid are all logical, so an unconverted point is wrong by the
        // display's scale — which is every click on a laptop at 150% and none at all on
        // a monitor at 100%, and is why this reads as a broken close button rather than
        // as arithmetic.
        assert_eq!(logical(150.0, 75.0, 1.5), (100.0, 50.0));
        assert_eq!(logical(150.0, 75.0, 1.0), (150.0, 75.0));
        assert_eq!(logical(150.0, 75.0, 2.0), (75.0, 37.5));
    }

    #[test]
    fn a_scale_that_is_not_a_scale_leaves_the_point_alone() {
        // Not a realistic value, but the division is the whole function and an infinity
        // out of it puts the pointer nowhere rather than at the origin, which is the
        // difference between a window responding oddly and a window not responding.
        assert_eq!(logical(150.0, 75.0, 0.0), (150.0, 75.0));
    }

    #[test]
    fn a_physical_pointer_lands_on_the_cell_the_user_is_pointing_at() {
        // The two steps composed, which is how the host uses them: a cell is ten by
        // twenty *physical* pixels, so at 150% it is 6.67 by 13.33 logical ones, and a
        // pointer the platform reports at physical (20, 40) is over the cell at
        // logical (13.33, 26.67).
        let grid = Rect::new(0.0, 0.0, 800.0, 600.0);
        let (x, y) = logical(20.0, 40.0, 1.5);
        assert_eq!(cell(x, y, grid, &metrics(), 1.5), Some(Pos::new(2, 2)));
        // The same coordinates read as logical rather than converted — which is what the
        // host did — are a cell past the one the user is pointing at.
        assert_eq!(
            cell(20.0, 40.0, grid, &metrics(), 1.5),
            Some(Pos::new(3, 3))
        );
    }

    #[test]
    fn a_point_in_the_origin_cell_is_row_zero_column_zero() {
        let grid = Rect::new(0.0, 40.0, 800.0, 600.0);
        assert_eq!(cell(0.0, 40.0, grid, &metrics(), 1.0), Some(Pos::new(0, 0)));
        assert_eq!(cell(9.9, 59.9, grid, &metrics(), 1.0), Some(Pos::new(0, 0)));
    }

    #[test]
    fn a_point_one_cell_over_is_one_cell_over() {
        let grid = Rect::new(0.0, 40.0, 800.0, 600.0);
        assert_eq!(
            cell(10.0, 60.0, grid, &metrics(), 1.0),
            Some(Pos::new(1, 1))
        );
        assert_eq!(
            cell(25.0, 105.0, grid, &metrics(), 1.0),
            Some(Pos::new(3, 2))
        );
    }

    #[test]
    fn the_chrome_above_the_grid_is_not_a_cell() {
        // The titlebar is forty logical pixels tall. A click there is a click on the
        // strip, and sending it to the shell would put the cursor in the grid.
        let grid = Rect::new(0.0, 40.0, 800.0, 600.0);
        assert_eq!(cell(100.0, 20.0, grid, &metrics(), 1.0), None);
    }

    #[test]
    fn the_rail_beside_the_grid_is_not_a_cell() {
        let grid = Rect::new(48.0, 40.0, 752.0, 600.0);
        assert_eq!(cell(20.0, 100.0, grid, &metrics(), 1.0), None);
        assert_eq!(
            cell(48.0, 100.0, grid, &metrics(), 1.0),
            Some(Pos::new(3, 0))
        );
    }

    #[test]
    fn the_same_logical_point_lands_on_the_same_cell_at_any_dpi() {
        // The bug this module is written around, stated as the invariant rather than as
        // one worked example: a display at 200% has twice as many physical pixels per
        // cell and reports the pointer in the same logical ones, so a click has to land
        // on the same cell either way. A version that compared the two directly would
        // answer every point on a 200% laptop with a cell twice as far along.
        let plain = metrics();
        let mut doubled = metrics();
        doubled.cell_width = 20.0;
        doubled.cell_height = 40.0;
        let grid = Rect::new(0.0, 0.0, 800.0, 600.0);

        for (x, y) in [(0.0, 0.0), (5.0, 10.0), (25.0, 55.0), (199.0, 399.0)] {
            assert_eq!(
                cell(x, y, grid, &plain, 1.0),
                cell(x, y, grid, &doubled, 2.0),
                "({x}, {y}) should be the same cell at either scale"
            );
        }
        // And the cells are the ones the arithmetic says, not merely equal to each other.
        assert_eq!(cell(5.0, 10.0, grid, &doubled, 2.0), Some(Pos::new(0, 0)));
        assert_eq!(cell(10.0, 20.0, grid, &doubled, 2.0), Some(Pos::new(1, 1)));
    }

    #[test]
    fn a_zero_scale_is_treated_as_unscaled_rather_than_dividing_by_zero() {
        let grid = Rect::new(0.0, 0.0, 800.0, 600.0);
        assert_eq!(
            cell(10.0, 20.0, grid, &metrics(), 0.0),
            Some(Pos::new(1, 1))
        );
    }

    #[test]
    fn a_click_on_the_last_pixel_still_lands_in_the_grid() {
        let grid = Rect::new(0.0, 0.0, 800.0, 600.0);
        let at = cell(799.9, 599.9, grid, &metrics(), 1.0).expect("the last pixel is in the grid");
        assert_eq!(at, Pos::new(29, 79));
    }

    #[test]
    fn a_click_past_the_last_whole_column_is_still_in_the_grid() {
        // A grid 805 pixels wide holds eighty whole ten-pixel cells and five pixels of an
        // eighty-first. A point in that strip is inside the grid's rectangle, so it used
        // to be reported to the program as column 81 of 80 — a coordinate the program has
        // no cell for, and one a wheel at the very edge of the window sends on every
        // notch.
        let grid = Rect::new(0.0, 0.0, 805.0, 605.0);
        assert_eq!(
            cell(804.0, 300.0, grid, &metrics(), 1.0),
            Some(Pos::new(15, 79))
        );
        assert_eq!(
            cell(300.0, 604.0, grid, &metrics(), 1.0),
            Some(Pos::new(29, 30))
        );
    }

    #[test]
    fn the_last_column_the_program_has_is_one_the_pointer_can_reach() {
        // 1100 logical pixels at 125% is 1375 physical ones, which is exactly 125 cells
        // of eleven. Dividing by the cell already scaled reads the same sum and gives
        // 124.99999999999999, whose floor is a column the program has and the pointer
        // cannot: a click on the rightmost column reported one to its left, a drag to
        // the edge stopping a column short of where the user let go, and — because the
        // anchor is clamped too — a press and release on that column agreeing that
        // nothing was dragged, which throws the selection away.
        let mut wide = metrics();
        wide.cell_width = 11.0;
        let grid = Rect::new(0.0, 0.0, 1100.0, 600.0);
        assert_eq!(cells(grid, &wide, 1.25), (125, 37));
        assert_eq!(
            cell(1095.0, 300.0, grid, &wide, 1.25),
            Some(Pos::new(18, 124))
        );
        assert_eq!(
            cell(1099.9, 599.9, grid, &wide, 1.25),
            Some(Pos::new(36, 124)),
            "the last pixel of the window is the last column, not the one before it"
        );
    }

    #[test]
    fn a_rectangle_too_short_for_a_cell_holds_none() {
        let grid = Rect::new(0.0, 0.0, 9.0, 19.0);
        assert_eq!(cells(grid, &metrics(), 1.0), (0, 0));
        // And a cell of no width at all is not a division by zero.
        let mut flat = metrics();
        flat.cell_width = 0.0;
        assert_eq!(cells(Rect::new(0.0, 0.0, 800.0, 600.0), &flat, 1.0), (0, 0));
    }

    #[test]
    fn the_body_of_the_window_is_not_an_edge() {
        assert_eq!(edge(100.0, 100.0, 800.0, 600.0), None);
        assert_eq!(edge(400.0, 300.0, 800.0, 600.0), None);
    }

    #[test]
    fn each_side_is_its_own_edge() {
        assert_eq!(edge(1.0, 300.0, 800.0, 600.0), Some(Edge::West));
        assert_eq!(edge(799.0, 300.0, 800.0, 600.0), Some(Edge::East));
        assert_eq!(edge(400.0, 1.0, 800.0, 600.0), Some(Edge::North));
        assert_eq!(edge(400.0, 599.0, 800.0, 600.0), Some(Edge::South));
    }

    #[test]
    fn a_corner_wins_over_the_two_edges_it_belongs_to() {
        assert_eq!(edge(1.0, 1.0, 800.0, 600.0), Some(Edge::NorthWest));
        assert_eq!(edge(799.0, 1.0, 800.0, 600.0), Some(Edge::NorthEast));
        assert_eq!(edge(1.0, 599.0, 800.0, 600.0), Some(Edge::SouthWest));
        assert_eq!(edge(799.0, 599.0, 800.0, 600.0), Some(Edge::SouthEast));
    }

    #[test]
    fn a_window_too_small_for_borders_has_none() {
        // A window eight pixels wide has no point that is not within six of an edge, and
        // answering `West` for its middle would make it impossible to click anything.
        assert_eq!(edge(4.0, 4.0, 8.0, 600.0), None);
        assert_eq!(edge(4.0, 4.0, 800.0, 8.0), None);
    }

    #[test]
    fn every_edge_has_the_cursor_that_points_the_way_it_drags() {
        assert_eq!(Edge::West.cursor(), winit::window::CursorIcon::EwResize);
        assert_eq!(
            Edge::NorthEast.cursor(),
            winit::window::CursorIcon::NeswResize
        );
        assert_eq!(Edge::North.cursor(), winit::window::CursorIcon::NsResize);
        assert_eq!(
            Edge::SouthWest.cursor(),
            winit::window::CursorIcon::NeswResize
        );
    }

    #[test]
    fn the_three_buttons_a_terminal_has_are_forwarded() {
        assert_eq!(button(WinitButton::Left), MouseButton::Left);
        assert_eq!(button(WinitButton::Middle), MouseButton::Middle);
        assert_eq!(button(WinitButton::Right), MouseButton::Right);
    }

    #[test]
    fn a_button_zet_cannot_name_is_dropped() {
        assert_eq!(button(WinitButton::Other(9)), MouseButton::None);
    }

    #[test]
    fn a_line_delta_is_already_lines_and_the_sign_is_a_scroll_up() {
        use winit::event::MouseScrollDelta::{LineDelta, PixelDelta};
        // winit counts a scroll away from the user as positive; scrolling up into
        // history is what that means for a terminal.
        assert_eq!(wheel(LineDelta(0.0, 1.0), 20.0), -1.0);
        assert_eq!(wheel(LineDelta(0.0, -1.0), 20.0), 1.0);
        assert_eq!(
            wheel(
                PixelDelta(winit::dpi::PhysicalPosition::new(0.0, 40.0)),
                20.0
            ),
            -2.0
        );
    }

    #[test]
    fn a_pixel_delta_with_no_line_height_is_not_a_division_by_zero() {
        assert_eq!(
            wheel(
                MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, 40.0)),
                0.0
            ),
            0.0
        );
    }

    #[test]
    fn a_wheel_direction_decides_the_button() {
        // Positive is away from the user, which is up — the same sign `wheel` returns,
        // so the scrollback and the program can never disagree about which way is up.
        assert_eq!(wheel_button(1.0), MouseButton::WheelUp);
        assert_eq!(wheel_button(-1.0), MouseButton::WheelDown);
    }
}
