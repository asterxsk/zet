//! The three surfaces that sit over the grid rather than framing it: the settings panel,
//! the find bar, and the scrollbar.
//!
//! They are together because they share a rule rather than a purpose. None of them is
//! part of the window's frame, none of them is modal, and all three take their depth from
//! a hairline and a one-step surface change — DESIGN.md's only depth mechanism in the
//! chrome, since there are no shadows, no gradients, and no glass.

use zet_font::Weight;

use crate::ChromeInput;
use crate::ScrollState;
use crate::geometry::{
    FIND_BAR_HEIGHT, PANEL_WIDTH, Rect, SCROLLBAR, SCROLLBAR_HOVER, SCROLLBAR_MIN_THUMB,
};
use crate::paint::{Painter, TextStyle};

/// The panel's skeleton.
///
/// DESIGN.md fixes the four sections and their order, and a heading is a heading. The
/// rows under each name a setting; what the settings *are* belongs to the configuration,
/// which this crate is not given, and inventing a second copy of it here is exactly how
/// a panel and a file start disagreeing. So a row is a label and a control-shaped
/// rectangle, which is the whole of what a layout can honestly draw until it is handed
/// values.
///
/// The headings are written in upper case rather than converted, because the conversion
/// would allocate four strings every frame to say the same thing.
const SECTIONS: [(&str, &[&str]); 4] = [
    ("APPEARANCE", &["Theme", "Text scale", "Reduce motion"]),
    ("TABS", &["Position", "Open default"]),
    (
        "TERMINAL",
        &["Font", "Size", "Cursor shape", "Cursor blink"],
    ),
    ("KEYS", &["New tab", "Close tab", "Find", "Settings"]),
];

/// The gap between the panel's edge and its content.
const PAD: f32 = 16.0;

/// The space a heading occupies, including the hairline under it.
const HEADING_BOX: f32 = 22.0;

/// The space under a section's last row and above the next heading.
const SECTION_GAP: f32 = 18.0;

/// A settings row's height.
const ROW: f32 = 28.0;

/// The placeholder a row's control is drawn as.
const CONTROL_WIDTH: f32 = 108.0;
const CONTROL_HEIGHT: f32 = 20.0;

/// The find bar's own padding, and the field inside it.
const FIND_PAD: f32 = 8.0;
const FIND_FIELD: f32 = 20.0;
const FIND_FIELD_WIDTH: f32 = 240.0;

/// The sizes DESIGN.md's type table gives the panel.
const HEADING_SIZE: f32 = 12.0;
const HEADING_TRACKING: f32 = 0.08;
const LABEL_SIZE: f32 = 13.0;
const HINT_SIZE: f32 = 12.0;

/// Draw the settings panel and answer where it went.
///
/// A 380-pixel panel against the right edge, on `surface-raised`, with a hairline
/// between it and anything behind it. It does not resize the grid: the terminal stays
/// visible behind it and keeps updating, which DESIGN.md says is the reason the panel
/// exists at all rather than a config file alone.
pub(crate) fn panel(
    paint: &mut Painter<'_>,
    input: &ChromeInput<'_>,
    top: f32,
    bottom: f32,
) -> Rect {
    let palette = *input.palette;
    // Narrower than 380 pixels of window means the panel is the window. Letting it hang
    // off the left edge would put the heading of a section nobody can read behind the
    // rail.
    let width = PANEL_WIDTH.min(input.size.width);
    let rect = Rect::new(
        input.size.width - width,
        top,
        width,
        (input.size.height - top - bottom).max(0.0),
    );
    paint.fill(rect, palette.surface_raised);
    paint.fill(
        Rect::new(rect.x, rect.y, 1.0, rect.height),
        palette.hairline,
    );

    let mut y = rect.y + PAD;
    for (heading, rows) in SECTIONS {
        let heading_box = Rect::new(rect.x + PAD, y, rect.width - 2.0 * PAD, HEADING_BOX);
        if heading_box.y > rect.bottom() {
            break;
        }
        let style =
            TextStyle::new(heading_size(), Weight::MEDIUM, palette.ink).tracking(HEADING_TRACKING);
        paint.centered(heading, heading_box, style);
        y += HEADING_BOX;
        paint.fill(Rect::new(rect.x, y, rect.width, 1.0), palette.hairline);
        y += 1.0 + PAD / 2.0;

        for row in rows {
            if y > rect.bottom() {
                break;
            }
            let row_box = Rect::new(rect.x + PAD, y, rect.width - 2.0 * PAD, ROW);
            let label = TextStyle::new(LABEL_SIZE, Weight::NORMAL, palette.ink);
            paint.centered(row, row_box, label);
            let control = Rect::new(
                row_box.right() - CONTROL_WIDTH,
                y + (ROW - CONTROL_HEIGHT) / 2.0,
                CONTROL_WIDTH,
                CONTROL_HEIGHT,
            );
            paint.fill(control, palette.hairline);
            y += ROW;
        }
        y += SECTION_GAP;
    }
    rect
}

/// The heading size, which is a constant rather than a literal so the type table reads
/// as one place.
const fn heading_size() -> f32 {
    HEADING_SIZE
}

/// Draw the find bar and answer where it went.
///
/// A 32-pixel row pinned above the grid's bottom edge, `surface-raised`, using the same
/// hairline language as everything else. It is not part of the grid and does not overlap
/// it: `Layout::bottom` is this row's height, and the caller takes it off the grid before
/// the grid is told how big it is.
pub(crate) fn find_bar(paint: &mut Painter<'_>, input: &ChromeInput<'_>, height: f32) -> Rect {
    let palette = *input.palette;
    let rect = Rect::new(0.0, input.size.height - height, input.size.width, height);
    paint.fill(rect, palette.surface_raised);
    paint.fill(Rect::new(0.0, rect.y, rect.width, 1.0), palette.hairline);

    let field = Rect::new(
        FIND_PAD,
        rect.y + (height - FIND_FIELD) / 2.0,
        FIND_FIELD_WIDTH.min(rect.width - 2.0 * FIND_PAD),
        FIND_FIELD,
    );
    // A focused input border is `hairline-strong`, and the field behind it is the
    // ground, so that the one thing on this row that takes typing looks like it.
    paint.fill(field, palette.ground);
    border(paint, field, palette.hairline_strong);
    let style = TextStyle::new(HINT_SIZE, Weight::NORMAL, palette.ink_mid);
    let baseline = paint.baseline_in(field, HINT_SIZE);
    paint.text("Find", field.x + FIND_PAD, baseline, style);
    rect
}

/// A one-pixel outline inside `rect`.
fn border(paint: &mut Painter<'_>, rect: Rect, color: zet_config::Rgb) {
    paint.fill(Rect::new(rect.x, rect.y, rect.width, 1.0), color);
    paint.fill(
        Rect::new(rect.x, rect.bottom() - 1.0, rect.width, 1.0),
        color,
    );
    paint.fill(Rect::new(rect.x, rect.y, 1.0, rect.height), color);
    paint.fill(
        Rect::new(rect.right() - 1.0, rect.y, 1.0, rect.height),
        color,
    );
}

/// Draw the scrollbar and answer its track and thumb, for hit testing.
///
/// "An 8px `hairline-strong` thumb on `ground`, growing to 10px on hover, with no track
/// and no arrows." No track is taken literally: the thumb is the only thing painted, and
/// the band it moves in is a hit region rather than a surface. That is also the only
/// reading under which the scrollbar does not paint over the grid's last column, which
/// the two-plane rule would have plenty to say about.
///
/// `None` when there is nothing to scroll. A full-height thumb is a scrollbar that says
/// "you are seeing everything", and a terminal spends most of its life seeing everything.
pub(crate) fn scrollbar(
    paint: &mut Painter<'_>,
    input: &ChromeInput<'_>,
    top: f32,
    bottom: f32,
    scroll: ScrollState,
) -> Option<(Rect, Rect)> {
    if scroll.visible >= 1.0 {
        return None;
    }
    let palette = *input.palette;
    let hovered = input
        .pointer
        .is_some_and(|(x, _)| x >= input.size.width - SCROLLBAR_HOVER);
    let width = if hovered { SCROLLBAR_HOVER } else { SCROLLBAR };
    let track = Rect::new(
        input.size.width - width,
        top,
        width,
        (input.size.height - top - bottom).max(0.0),
    );
    if track.height <= 0.0 {
        return None;
    }

    let visible = scroll.visible.clamp(0.0, 1.0);
    let height = (track.height * visible)
        .max(SCROLLBAR_MIN_THUMB)
        .min(track.height);
    let offset = scroll.offset.clamp(0.0, 1.0);
    let thumb = Rect::new(
        track.x,
        track.y + (track.height - height) * offset,
        width,
        height,
    );
    paint.fill(thumb, palette.hairline_strong);
    Some((track, thumb))
}

/// The find bar's height, for a caller that wants to know without opening it.
#[must_use]
pub const fn find_bar_height() -> f32 {
    FIND_BAR_HEIGHT
}
