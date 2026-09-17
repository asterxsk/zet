//! The three surfaces that sit over the grid rather than framing it: the settings panel,
//! the find bar, and the scrollbar.
//!
//! They are together because they share a rule rather than a purpose. None of them is
//! part of the window's frame, none of them is modal, and all three take their depth from
//! a hairline and a one-step surface change — DESIGN.md's only depth mechanism in the
//! chrome, since there are no shadows, no gradients, and no glass.

use zet_font::Weight;

use crate::ChromeInput;
use crate::Control;
use crate::ScrollState;
use crate::SettingLine;
use crate::SettingPart;
use crate::geometry::{
    FIND_BAR_HEIGHT, PANEL_WIDTH, Rect, SCROLLBAR, SCROLLBAR_HOVER, SCROLLBAR_MIN_THUMB,
};
use crate::paint::{Painter, TextStyle};

/// The gap between the panel's edge and its content.
const PAD: f32 = 16.0;

/// The space a heading occupies, including the hairline under it.
const HEADING_BOX: f32 = 22.0;

/// The space above the first heading of a section, after the last row of the one before.
const SECTION_GAP: f32 = 18.0;

/// A settings row's height.
const ROW: f32 = 28.0;

/// A row's control.
///
/// Wide enough for `Cascadia Mono`, wide enough for a chord like `Ctrl+Shift+T`, and no
/// wider: the label needs the rest, and a control that takes half the panel is a control
/// that does not look like something you click once.
const CONTROL_WIDTH: f32 = 118.0;
const CONTROL_HEIGHT: f32 = 20.0;

/// The gap between a row's label and its control.
const LABEL_GAP: f32 = 12.0;

/// The find bar's own padding, and the field inside it.
const FIND_PAD: f32 = 8.0;
const FIND_FIELD: f32 = 20.0;
const FIND_FIELD_WIDTH: f32 = 240.0;

/// The word before the query, so the field says what it is for even when it is empty.
const FIND_LABEL: &str = "Find";

/// The gap after the label, and after the field.
const FIND_GAP: f32 = 8.0;

/// The caret at the end of the query: a hairline standing a little inside the field.
const CARET_WIDTH: f32 = 1.0;
const CARET_INSET: f32 = 4.0;

/// The sizes DESIGN.md's type table gives the panel.
const HEADING_SIZE: f32 = 12.0;
const HEADING_TRACKING: f32 = 0.08;
const LABEL_SIZE: f32 = 13.0;
const HINT_SIZE: f32 = 12.0;

/// What the panel drew, so the caller can hit-test it and keep its scroll honest.
pub(crate) struct Panel {
    /// The panel itself.
    pub rect: Rect,
    /// Every control that was on screen, and which line it belongs to.
    pub controls: Vec<(usize, SettingPart, Rect)>,
    /// The scroll this frame was drawn at, clamped to what actually overflows.
    pub scroll: f32,
}

/// Draw the settings panel and answer where it went.
///
/// A 380-pixel panel against the right edge, on `surface-raised`, with a hairline
/// between it and anything behind it. It does not resize the grid: the terminal stays
/// visible behind it and keeps updating, which DESIGN.md says is the reason the panel
/// exists at all rather than a config file alone.
///
/// The lines come from the caller. What a setting *is* belongs to the configuration,
/// which this crate is not given, and a second copy of it living next to the painter is
/// exactly how a panel and a file start disagreeing — so the panel owns the geometry,
/// the type, and the hit regions, and the caller owns the words and the values.
///
/// Lines that do not fit are scrolled rather than dropped. A panel that silently stops
/// listing settings once the window is short is a panel where the user cannot find a
/// setting and has no way to tell that it is there.
pub(crate) fn panel(
    paint: &mut Painter<'_>,
    input: &ChromeInput<'_>,
    top: f32,
    bottom: f32,
) -> Panel {
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

    // What the whole list would take, so the scroll can be clamped to the overflow
    // rather than to a number the caller guessed. This walks the lines twice, and the
    // second walk is the one that draws.
    let content = list_height(input.settings);
    let viewport = (rect.height - 2.0 * PAD).max(0.0);
    let mut scroll = input
        .settings_scroll
        .clamp(0.0, (content - viewport).max(0.0));
    // A row the keyboard is on has to be on screen, or the keys move a highlight nobody
    // can see and the panel looks broken rather than scrolled.
    if let Some(focus) = input.settings_focus {
        let top = content_top(input.settings, focus);
        if top < scroll {
            scroll = top;
        } else if top + ROW > scroll + viewport {
            scroll = top + ROW - viewport;
        }
        scroll = scroll.clamp(0.0, (content - viewport).max(0.0));
    }

    let mut controls = Vec::new();
    let mut y = rect.y + PAD - scroll;
    for (line, setting) in input.settings.iter().enumerate() {
        let focused = input.settings_focus == Some(line);
        let (lead, height) = block(line, setting);
        y += lead;
        let Some(control) = setting.control else {
            // A section heading, and the rule under it.
            let heading_box = Rect::new(rect.x + PAD, y, rect.width - 2.0 * PAD, HEADING_BOX);
            if visible(heading_box, rect) {
                let style = TextStyle::new(HEADING_SIZE, Weight::MEDIUM, palette.ink)
                    .tracking(HEADING_TRACKING);
                paint.centered(setting.text, heading_box, style);
            }
            let rule = Rect::new(rect.x, y + HEADING_BOX, rect.width, 1.0);
            if visible(rule, rect) {
                paint.fill(rule, palette.hairline);
            }
            y += height;
            continue;
        };

        let row = Rect::new(rect.x + PAD, y, rect.width - 2.0 * PAD, ROW);
        let control_rect = Rect::new(
            row.right() - CONTROL_WIDTH,
            y + (ROW - CONTROL_HEIGHT) / 2.0,
            CONTROL_WIDTH,
            CONTROL_HEIGHT,
        );
        if visible(row, rect) {
            // The label grows into whatever the control does not take, which is what
            // keeps a long family name from running under its own value.
            let label_box = Rect::new(
                row.x,
                y,
                (row.width - CONTROL_WIDTH - LABEL_GAP).max(0.0),
                ROW,
            );
            let style = TextStyle::new(LABEL_SIZE, Weight::NORMAL, palette.ink);
            paint.centered(setting.text, label_box, style);
            draw_control(paint, input, control, control_rect, setting.value, focused);
            for (part, rect) in parts(control, control_rect) {
                controls.push((line, part, rect));
            }
        }
        y += height;
    }

    Panel {
        rect,
        controls,
        scroll,
    }
}

/// Whether a box is worth drawing: it intersects the panel at all.
fn visible(box_: Rect, panel: Rect) -> bool {
    box_.bottom() > panel.y && box_.y < panel.bottom()
}

/// The eight pixels either side of a control that a hover reads as "on this one".
fn draw_control(
    paint: &mut Painter<'_>,
    input: &ChromeInput<'_>,
    control: Control,
    rect: Rect,
    value: &str,
    focused: bool,
) {
    let palette = *input.palette;
    // A control is the ground behind a hairline, which is DESIGN.md's one depth
    // mechanism applied to something that is not a surface: the panel is raised, so the
    // thing you can press is recessed into it.
    paint.fill(rect, palette.ground);
    let hovered = match (control, input.pointer) {
        (Control::Step, Some((x, y))) if rect.contains(x, y) => Some(x >= rect.center().0),
        (_, Some((x, y))) if rect.contains(x, y) => Some(false),
        _ => None,
    };
    if let Some(half) = hovered {
        // A stepper is two controls wearing one rectangle, so hovering one half fills
        // that half: with no glyphs to read, the fill is the only thing that says which
        // half a click is about to take.
        let lit = if half {
            Rect::new(rect.center().0, rect.y, rect.width / 2.0, rect.height)
        } else {
            Rect::new(rect.x, rect.y, rect.width / 2.0, rect.height)
        };
        paint.fill(lit, palette.hairline);
    }
    // Three weights of the same hairline, and no fourth: the control you are on is
    // `ink`, the one under the pointer is `hairline-strong`, and the rest are `hairline`.
    // `signal` is the obvious colour for a focus ring and the wrong one — DESIGN.md gives
    // it a 3px by 40px budget and it is the app's one lamp, which a border around a
    // 118-pixel control would spend several times over.
    let edge = if focused {
        palette.ink
    } else if hovered.is_some() {
        palette.hairline_strong
    } else {
        palette.hairline
    };
    border(paint, rect, edge);
    let style = TextStyle::new(LABEL_SIZE, Weight::NORMAL, palette.ink);
    paint.centered(value, rect, style);
}

/// The parts of a control a click can land on.
///
/// A stepper answers twice, because its two halves mean opposite things; everything else
/// answers once. Returned as a list rather than matched on at the call site so that "how
/// many ways can this be clicked" has exactly one answer in the crate.
fn parts(control: Control, rect: Rect) -> Vec<(SettingPart, Rect)> {
    match control {
        Control::Step => vec![
            (
                SettingPart::Less,
                Rect::new(rect.x, rect.y, rect.width / 2.0, rect.height),
            ),
            (
                SettingPart::More,
                Rect::new(rect.center().0, rect.y, rect.width / 2.0, rect.height),
            ),
        ],
        _ => vec![(SettingPart::Whole, rect)],
    }
}

/// What one line of the panel takes: the gap above it, and its own height.
///
/// The one place the panel's vertical rhythm is written down. The walk in [`panel`] and
/// the two measurements below all go through this, so the scroll the caller asked for
/// can be clamped to what actually overflows rather than to a number it guessed at, and
/// the three of them cannot drift apart.
///
/// The gap above a section heading belongs to the heading rather than to the section
/// before it, so the first heading of the panel is not pushed down by a gap with nothing
/// above it to separate it from.
fn block(line: usize, setting: &SettingLine<'_>) -> (f32, f32) {
    if setting.control.is_none() {
        (
            if line == 0 { 0.0 } else { SECTION_GAP },
            HEADING_BOX + 1.0 + PAD / 2.0,
        )
    } else {
        (0.0, ROW)
    }
}

/// How tall the whole list would be, drawn from the top.
fn list_height(lines: &[SettingLine<'_>]) -> f32 {
    lines
        .iter()
        .enumerate()
        .map(|(line, setting)| {
            let (lead, height) = block(line, setting);
            lead + height
        })
        .sum()
}

/// Where a line's content starts, measured from the top of the list.
///
/// The gap above a heading is not counted, because this answers "where would the list
/// have to be scrolled to for this line to be visible", and scrolling to a blank gap
/// puts nothing on screen. Every focusable line is a row, which has no gap at all.
fn content_top(lines: &[SettingLine<'_>], index: usize) -> f32 {
    let mut y = 0.0;
    for (line, setting) in lines.iter().enumerate().take(index + 1) {
        let (lead, height) = block(line, setting);
        y += lead;
        if line == index {
            break;
        }
        y += height;
    }
    y
}

/// Draw the find bar and answer where it went.
///
/// A 32-pixel row pinned above the grid's bottom edge, `surface-raised`, using the same
/// hairline language as everything else. It is not part of the grid and does not overlap
/// it: `Layout::bottom` is this row's height, and the caller takes it off the grid before
/// the grid is told how big it is.
///
/// Three things are on it: a labelled field holding the query, a caret, and a count. The
/// query is the one piece of stateful text in the chrome, so it is the one place where
/// what is drawn depends on what was typed — and a query longer than the field is shown
/// from its end rather than its beginning, because the caret is where the next character
/// goes and a caret off the right edge is a field that looks broken at exactly the moment
/// the user is typing into it.
///
/// The caret does not blink. DESIGN.md allows one authored moment in this app and it is
/// the tab indicator's travel; a second thing moving on screen is a second thing to look
/// at, and the caret is already the brightest hairline in the row.
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

    let baseline = paint.baseline_in(field, HINT_SIZE);
    let label = TextStyle::new(HINT_SIZE, Weight::NORMAL, palette.ink_dim);
    paint.text(FIND_LABEL, field.x + FIND_PAD, baseline, label);

    let Some(find) = input.find.as_ref() else {
        return rect;
    };
    let style = TextStyle::new(LABEL_SIZE, Weight::NORMAL, palette.ink);
    let left = field.x + FIND_PAD + paint.width(FIND_LABEL, label) + FIND_GAP;
    let room = (field.right() - FIND_PAD - left - CARET_WIDTH).max(0.0);
    let query = tail_that_fits(paint, find.query, style, room);
    let typed = paint.width(query, style);
    paint.text(query, left, baseline, style);
    paint.fill(
        Rect::new(
            left + typed + CARET_WIDTH,
            field.y + CARET_INSET,
            CARET_WIDTH,
            field.height - 2.0 * CARET_INSET,
        ),
        palette.ink,
    );

    let (count, color) = match find.position {
        Some((at, total)) => (
            format!("{at} of {total}{}", plus(find.capped)),
            palette.ink_mid,
        ),
        None if find.query.is_empty() => (String::new(), palette.ink_dim),
        None => ("No results".to_owned(), palette.ink_dim),
    };
    if !count.is_empty() {
        let style = TextStyle::new(HINT_SIZE, Weight::NORMAL, color);
        paint.text(&count, field.right() + FIND_GAP * 2.0, baseline, style);
    }
    rect
}

/// The longest tail of `query` that fits in `room`.
///
/// By `char` and not by byte: a query is what the user typed, and a slice in the middle
/// of a character is a panic in a paint loop.
fn tail_that_fits<'a>(
    paint: &mut Painter<'_>,
    query: &'a str,
    style: TextStyle,
    room: f32,
) -> &'a str {
    let mut start = 0;
    while start < query.len() && paint.width(&query[start..], style) > room {
        start += query[start..].chars().next().map_or(1, char::len_utf8);
    }
    &query[start..]
}

/// The mark a search that stopped counting wears.
const fn plus(capped: bool) -> &'static str {
    if capped { "+" } else { "" }
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
