//! Rectangles, sizes, and the measurements DESIGN.md fixes.
//!
//! Everything here is in **logical** pixels: the numbers as they are written in the
//! design document, before the window's DPI scale is applied. The scale is applied once,
//! where a rectangle is turned into a quad, which is what keeps the arithmetic in this
//! crate readable as the design rather than as a converted form of it.

/// A width and a height in logical pixels.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Size {
    /// The width.
    pub width: f32,
    /// The height.
    pub height: f32,
}

/// A rectangle in logical pixels, from the window's top-left corner.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Rect {
    /// The left edge.
    pub x: f32,
    /// The top edge.
    pub y: f32,
    /// The width.
    pub width: f32,
    /// The height.
    pub height: f32,
}

impl Rect {
    /// A rectangle from its top-left corner and its size.
    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The right edge.
    #[must_use]
    pub fn right(self) -> f32 {
        self.x + self.width
    }

    /// The bottom edge.
    #[must_use]
    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    /// The middle, which is where a control is hit.
    #[must_use]
    pub fn center(self) -> (f32, f32) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    /// Whether a point is inside.
    ///
    /// The right and bottom edges are exclusive, so that two rectangles that share an
    /// edge do not both claim the pixel on it. Every region this crate hit-tests is
    /// adjacent to another one, and a shared pixel is a region that wins by accident of
    /// which was pushed first.
    #[must_use]
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// The rectangle `height` tall with its top edge at `y`.
    #[must_use]
    pub const fn with_y(self, y: f32) -> Self {
        Self { y, ..self }
    }

    /// A rectangle between two edges, clamped so an inverted one is empty rather than
    /// negative.
    #[must_use]
    pub fn between(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self::new(left, top, (right - left).max(0.0), (bottom - top).max(0.0))
    }

    /// The rectangle part of the way from `self` to `other`.
    ///
    /// Used by the indicator, which is the one thing in the chrome that is ever between
    /// two places.
    #[must_use]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self::new(
            lerp(self.x, other.x, t),
            lerp(self.y, other.y, t),
            lerp(self.width, other.width, t),
            lerp(self.height, other.height, t),
        )
    }
}

/// A point part of the way from `from` to `to`.
#[must_use]
pub fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

/// The height of the one row that holds the app name, the tabs, the drag region, and the
/// caption buttons.
///
/// One row and not two: stacking a titlebar above a tab strip spends 36 pixels saying
/// nothing, and this is the change DESIGN.md says makes zet feel denser than the
/// terminal it replaces.
pub const ROW_HEIGHT: f32 = 40.0;

/// The width of a caption button.
///
/// Windows' own metric, deliberately. These are among the most-hit pixels on the
/// machine and muscle memory for them is not zet's to redesign.
pub const CAPTION_WIDTH: f32 = 46.0;

/// The gap between the window's left edge and the app name.
pub const NAME_INSET: f32 = 12.0;

/// The gap between the app name and the first tab.
pub const NAME_GAP: f32 = 14.0;

/// The app name's size.
pub const NAME_SIZE: f32 = 12.0;

/// The app name's letter spacing, in ems.
pub const NAME_TRACKING: f32 = 0.02;

/// The size of a tab's index, and therefore of the new-tab mark.
pub const TAB_SIZE: f32 = 13.0;

/// The padding either side of a tab's index.
pub const TAB_PADDING: f32 = 12.0;

/// The gap between a tab's number and its name.
pub const TAB_GAP: f32 = 8.0;

/// The widest a tab's cell grows, however few tabs there are and however long the names.
///
/// A strip of one very long title is a strip with one tab on it. This is the ceiling
/// that keeps a tab a cell in a run rather than a banner, and it is what makes the
/// names share the row instead of taking it.
pub const TAB_MAX_WIDTH: f32 = 180.0;

/// The `#` is set at this fraction of the index's size.
///
/// Part of the mark rather than a prefix: it is what makes a row of numbers read as a
/// set of numbered channels rather than as a list.
pub const HASH_RATIO: f32 = 0.7;

/// The thickness of the active indicator.
///
/// DESIGN.md's palette rule gives `signal` a ceiling of three pixels by forty, and this
/// is the bar that rule is about.
pub const INDICATOR: f32 = 2.0;

/// How long the indicator takes to travel, in milliseconds.
///
/// Milliseconds, because DESIGN.md says "140ms" and a number that has to be converted
/// before it can be compared with the document is a number that will drift from it.
/// The clock this is measured against counts seconds, so the one conversion lives in
/// [`TRAVEL`] rather than at each use.
pub const TRAVEL_MS: f32 = 140.0;

/// The same span in the seconds [`Chrome::set_time`](crate::Chrome::set_time) counts in.
pub const TRAVEL: f32 = TRAVEL_MS / 1000.0;

/// The width of the vertical tab rail.
pub const RAIL_WIDTH: f32 = 48.0;

/// The height of a rail cell.
pub const RAIL_CELL: f32 = 36.0;

/// The height a rail cell compresses to before the rail has to overflow.
pub const RAIL_CELL_FLOOR: f32 = 24.0;

/// The width of the settings panel.
pub const PANEL_WIDTH: f32 = 380.0;

/// How long the settings panel takes to slide in.
///
/// Milliseconds for the same reason [`TRAVEL_MS`] is: DESIGN.md says "180ms", and this is
/// the one other transition the document asks for.
pub const PANEL_SLIDE_MS: f32 = 180.0;

/// The same span in the seconds [`Chrome::set_time`](crate::Chrome::set_time) counts in.
pub const PANEL_SLIDE: f32 = PANEL_SLIDE_MS / 1000.0;

/// The height of a row of a menu.
pub const MENU_ROW: f32 = 28.0;

/// The space either side of a menu's items, and above the first and below the last.
pub const MENU_PAD: f32 = 16.0;

/// The narrowest a menu gets, however short its items are.
///
/// A menu sized exactly to a two-word item is a sliver, and a sliver reads as a tooltip
/// with something wrong with it rather than as a list to choose from.
pub const MENU_MIN_WIDTH: f32 = 120.0;

/// How wide a menu is for the items it holds.
///
/// Measured rather than fixed, so that a menu is as wide as what is in it: the items are
/// words of whatever length the actions have, and a width chosen here would be a second
/// place that has to be right about all of them. `measure` is how wide one item draws,
/// which is the font's business and not this function's.
#[must_use]
pub fn menu_width(items: &[&str], mut measure: impl FnMut(&str) -> f32) -> f32 {
    let widest = items
        .iter()
        .map(|item| measure(item))
        .fold(0.0_f32, f32::max);
    (widest + 2.0 * MENU_PAD).max(MENU_MIN_WIDTH)
}

/// Where a menu of `rows` items goes when it is opened at a point.
///
/// The point is where the pointer was, which is a hint about where the menu belongs and
/// not a fact about where it fits. When there is no room below, it opens upward with its
/// bottom edge at the pointer; when there is no room to the right, it opens leftward the
/// same way. Clamping instead would pin the menu against the window's edge with the
/// pointer somewhere in the middle of it, which for a menu means the item under the
/// pointer is not the one that was aimed at.
///
/// A menu larger than the window has nowhere to flip to, and is narrowed or shortened to
/// the window and pinned to the top-left corner. A caller that offers more items than the
/// window is tall has a menu whose bottom rows cannot be reached, which is the one case
/// this function cannot answer for: the answer is to offer fewer items.
#[must_use]
pub fn menu_rect(at: (f32, f32), width: f32, rows: usize, window: Size) -> Rect {
    let width = width.min(window.width);
    let height = (MENU_ROW * rows as f32 + 2.0 * MENU_PAD).min(window.height);
    let x = if at.0 + width > window.width {
        at.0 - width
    } else {
        at.0
    };
    let y = if at.1 + height > window.height {
        at.1 - height
    } else {
        at.1
    };
    Rect::new(
        x.max(0.0).min(window.width - width),
        y.max(0.0).min(window.height - height),
        width,
        height,
    )
}

/// The height of the find bar.
pub const FIND_BAR_HEIGHT: f32 = 32.0;

/// The scrollbar's width at rest.
pub const SCROLLBAR: f32 = 8.0;

/// The scrollbar's width under the pointer.
pub const SCROLLBAR_HOVER: f32 = 10.0;

/// The shortest a scrollbar's thumb is allowed to get.
pub const SCROLLBAR_MIN_THUMB: f32 = 24.0;
