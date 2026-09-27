//! The surfaces that are not the window's frame: the settings page, the find bar, the profile
//! picker, and the scrollbar.
//!
//! They are together because they share a rule rather than a purpose. None of them is part of
//! the window's frame — the three popovers sit over the grid and the page takes its place — none
//! of them is modal, and all four take their depth from a hairline and a one-step surface change,
//! which is DESIGN.md's only depth mechanism in the chrome, since there are no shadows, no
//! gradients, and no glass.

use std::ops::Range;

use zet_font::Weight;

use crate::ChromeInput;
use crate::Control;
use crate::Hit;
use crate::PickerLine;
use crate::ScrollState;
use crate::SettingLine;
use crate::SettingPart;
use crate::geometry::{
    FIND_BAR_HEIGHT, MENU_PAD, MENU_ROW, PANEL_RAIL, Rect, SCROLLBAR, SCROLLBAR_HOVER,
    SCROLLBAR_MIN_THUMB, lerp,
};
use crate::hover::Hover;
use crate::paint::{Painter, TextStyle};

/// The gap between the page's edge and its content.
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
/// wider: the label needs the rest, and a control that takes half the page is a control
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

/// The sizes DESIGN.md's type table gives the page.
const HEADING_SIZE: f32 = 12.0;
const HEADING_TRACKING: f32 = 0.08;
const LABEL_SIZE: f32 = 13.0;
const HINT_SIZE: f32 = 12.0;

/// The profile picker's own width, and the lamp on the row the question is on.
///
/// Wide enough for the longest shell name an ordinary Windows machine offers —
/// `Windows PowerShell` — with the padding and the lamp beside it. The lamp is two pixels
/// because that is what the tab strip's active marker and the rail's are, and DESIGN.md
/// budgets `signal` at 3px by 40px: this is the same lamp, rotated to the other edge.
const PICKER_WIDTH: f32 = 300.0;
const PICKER_LAMP: f32 = 2.0;

/// What the picker is for, so the list says what it is even when it has one row.
///
/// The find bar's label earns its place for exactly this reason and the argument is the
/// same here: a list of shell names over a terminal could be anything, and a popover is
/// the one place in the chrome where a word costs nothing.
const PICKER_LABEL: &str = "New tab";

/// What the picker drew, so the caller can hit-test it.
pub(crate) struct Popover {
    /// The popover itself.
    pub rect: Rect,
    /// Every row that was on screen, and which shell it offers.
    pub rows: Vec<(usize, Rect)>,
}

/// Draw the profile picker and answer where it went.
///
/// A `surface-raised` popover under the strip, against the grid's left edge, with a
/// hairline between it and the terminal behind it. It is not a modal and it does not
/// resize the grid: the terminal keeps running underneath, and every letter still reaches
/// it, which is the rule the settings page follows and the reason a shell can be picked
/// without stopping what the current one is doing.
///
/// The shape is the strip's, scaled down. A row the question is on carries a two-pixel
/// `signal` bar on its left edge — the same lamp as the active tab, moved to the edge the
/// rail uses — and is written in `ink` while the rest are `ink-mid`, so the readout is one
/// colour and one lamp rather than a second kind of highlight invented for a list. The
/// weight is not touched: DESIGN.md gives 500 to the active tab and the section headings
/// and to nothing else.
///
/// The names come from the caller for the reason the page's rows do — this crate is not
/// given the configuration and a second copy of what a profile *is* living next to the
/// painter is how the two start disagreeing.
///
/// Rows that do not fit are scrolled to rather than dropped, and the row the question is
/// on is scrolled into view first: a list that runs off the bottom of the window is a
/// shell the user can select and cannot see.
pub(crate) fn profile_picker(
    paint: &mut Painter<'_>,
    input: &ChromeInput<'_>,
    picker: &PickerLine<'_>,
    left: f32,
    top: f32,
    bottom: f32,
    hover: &Hover,
) -> Popover {
    let palette = *input.palette;
    let width = PICKER_WIDTH.min((input.size.width - left - 2.0 * PAD).max(0.0));
    let room = (input.size.height - top - bottom - 2.0 * PAD - HEADING_BOX).max(0.0);
    // Through `i32` on the way: `room` is clamped to zero above, so the sign is not in
    // question, and this is the one conversion in the crate where `rustc` cannot see that
    // for itself — the pixel count it comes from is a float and a row count is not.
    let visible = usize::try_from((room / ROW).floor() as i32)
        .unwrap_or(0)
        .min(picker.profiles.len());
    if visible == 0 {
        // A window too short to hold a caption and one row. Nothing is drawn, and nothing
        // is hittable either, which is the honest answer: there is no room for a question.
        return Popover {
            rect: Rect::new(left + PAD, top + PAD, width, 0.0),
            rows: Vec::new(),
        };
    }

    // The lit row is what the list is scrolled to. Everything above it gives way, so the
    // answer is on screen at the moment the user is choosing it, and the top of the list
    // is what is lost — which is where the user already looked.
    let first = picker.at.saturating_sub(visible.saturating_sub(1));
    let last = (first + visible).min(picker.profiles.len());
    let rect = Rect::new(
        left + PAD,
        top + PAD,
        width,
        PAD + HEADING_BOX + (last - first) as f32 * ROW + PAD,
    );
    paint.fill(rect, palette.surface_raised);
    border(paint, rect, palette.hairline, 1.0);

    let caption = Rect::new(
        rect.x + PAD,
        rect.y + PAD,
        (rect.width - 2.0 * PAD).max(0.0),
        HEADING_BOX,
    );
    let label = TextStyle::new(HINT_SIZE, Weight::NORMAL, palette.ink_dim);
    paint.text(
        PICKER_LABEL,
        caption.x,
        paint.baseline_in(caption, HINT_SIZE),
        label,
    );

    let mut rows = Vec::with_capacity(last - first);
    let mut y = caption.bottom();
    for (at, name) in picker.profiles.iter().enumerate().take(last).skip(first) {
        let row = Rect::new(rect.x, y, rect.width, ROW);
        let lit = at == picker.at;
        let hovered = hover.of(Hit::Profile(at));
        // The pointer's own row fills, which the picker did without and the menu does: both
        // are lists of things to press, and with no glyphs to read the fill is what says what
        // a click is about to do. The lamp is the other question — which row is *chosen* —
        // and the two are drawn together without competing, because one is two pixels at the
        // edge and the other is the row behind it.
        paint.fill_at(row, palette.hairline, hovered);
        if lit {
            paint.fill(
                Rect::new(row.x, row.y, PICKER_LAMP, row.height),
                palette.signal,
            );
        }
        // The name is given the rest of the row, less the lamp and the padding on both
        // sides, so a long one is cut by the painter rather than running under the edge.
        let text = Rect::new(
            row.x + PAD + PICKER_LAMP,
            row.y,
            (row.width - 2.0 * PAD - PICKER_LAMP).max(0.0),
            ROW,
        );
        let style = TextStyle::new(
            LABEL_SIZE,
            Weight::NORMAL,
            if lit { palette.ink } else { palette.ink_mid },
        );
        paint.centered(name, text, style);
        rows.push((at, row));
        y += ROW;
    }

    Popover { rect, rows }
}

/// What a menu drew, so the caller can hit-test it.
pub(crate) struct Menu {
    /// The menu itself, which takes a click on its padding rather than passing it on.
    pub rect: Rect,
    /// Every row that was drawn, and the item it offers.
    pub rows: Vec<(usize, Rect)>,
}

/// Draw a context menu and answer where it went.
///
/// The same surface as the picker — `surface-raised` behind a hairline — opened at the
/// pointer rather than pinned to a corner, because a menu is about the thing under the
/// pointer and a menu somewhere else is a menu the user has to look for. Where it goes
/// when there is no room is [`menu_rect`]'s decision, which is a plain function of the
/// numbers and is tested as one.
///
/// The row under the pointer is filled. That is the page's rule rather than the picker's:
/// a control in the page fills the half a click will take, and a menu row is one control
/// whose whole face is the target, so the fill is the whole row. The lamp the picker uses
/// would be wrong here — the picker's lamp says which of several rows is *chosen*, and in a
/// menu nothing is chosen until it is clicked.
///
/// Nothing is dimmed or disabled. An item that cannot be done is not offered at all: a
/// menu of greyed-out words is a menu that shows the user everything they cannot have, and
/// the caller is the only thing that knows which those are.
pub(crate) fn context_menu(
    paint: &mut Painter<'_>,
    input: &ChromeInput<'_>,
    menu: &crate::MenuLine<'_>,
    hover: &Hover,
) -> Menu {
    let palette = *input.palette;
    let width =
        crate::geometry::menu_width(menu.items, |item| paint.width(item, item_style(&palette)));
    let rect = crate::geometry::menu_rect(menu.at, width, menu.items.len(), input.size);
    paint.fill(rect, palette.surface_raised);
    border(paint, rect, palette.hairline, 1.0);

    let mut rows = Vec::with_capacity(menu.items.len());
    let mut y = rect.y + MENU_PAD;
    for (at, item) in menu.items.iter().enumerate() {
        let row = Rect::new(rect.x, y, rect.width, MENU_ROW);
        paint.fill_at(row, palette.hairline, hover.of(Hit::MenuItem(at)));
        // The label is given the menu less its padding, so a long item is cut by the
        // painter rather than running under the hairline — which cannot happen while the
        // width is measured from the items, and is what keeps it true if it ever is not.
        let text = Rect::new(
            row.x + MENU_PAD,
            row.y,
            (row.width - 2.0 * MENU_PAD).max(0.0),
            row.height,
        );
        paint.centered(item, text, item_style(&palette));
        rows.push((at, row));
        y += MENU_ROW;
    }

    Menu { rect, rows }
}

/// How a menu's items are set.
///
/// The page's label size and `ink`, because a menu is a list of words to read and press
/// rather than a value to read off: the picker's dimmer `ink-mid` is for rows that are
/// being chosen between, and every row of a menu is a thing the user can have.
fn item_style(palette: &zet_config::Palette) -> TextStyle {
    TextStyle::new(LABEL_SIZE, Weight::NORMAL, palette.ink)
}

/// What the page drew, so the caller can hit-test it and keep its scroll honest.
pub(crate) struct Page {
    /// The page itself.
    pub rect: Rect,
    /// Every control that was on screen, and which line it belongs to.
    pub controls: Vec<(usize, SettingPart, Rect)>,
    /// The rail's items, and the heading line each one shows.
    ///
    /// Empty when there is no rail, which is most callers: a list with no headings, or with
    /// one, is one page and no rail.
    pub sections: Vec<(usize, Rect)>,
    /// The heading line of the section that was drawn, when a rail was drawn at all.
    ///
    /// The answer to what the caller asked for, which is why it exists: the caller names a
    /// section and the page decides what that means, so the layout has to be able to say
    /// which one it actually drew — a name that matched nothing shows the first section, and
    /// a caller keeping its own state in step needs to know that.
    pub shown: Option<usize>,
    /// The scroll this frame was drawn at, clamped to what actually overflows.
    pub scroll: f32,
}

/// One section of the page: a heading, and the rows under it.
///
/// The sections are the caller's headings and nothing else. What a setting *is* belongs to the
/// configuration, which this crate is not given, so the words are the caller's and the
/// grouping is what those words already imply.
struct Section<'a> {
    /// The line the section starts on: its heading's line, or 0 for an unnamed leading run.
    start: usize,
    /// The line the next section starts on, or the end of the list.
    end: usize,
    /// The heading's text, which is what a caller names a section by.
    ///
    /// `None` for rows that came before any heading. Nothing current produces one, and a
    /// caller who does gets a page with no rail rather than rows quietly dropped.
    name: Option<&'a str>,
}

/// The page's sections, in the order the caller put them in.
///
/// Always at least one, because a list with no headings has one section: itself.
fn sections<'a>(lines: &'a [SettingLine<'a>]) -> Vec<Section<'a>> {
    let mut out: Vec<Section<'_>> = Vec::new();
    for (line, setting) in lines.iter().enumerate() {
        if setting.row != crate::Row::Heading {
            continue;
        }
        if let Some(last) = out.last_mut() {
            last.end = line;
        }
        out.push(Section {
            start: line,
            end: lines.len(),
            name: Some(setting.text),
        });
    }
    match out.first() {
        // Rows above the first heading are a section of their own, unnamed. They are not
        // dropped: a page that stopped listing rows because someone put them in the wrong
        // order is a page that hides settings.
        Some(first) if first.start != 0 => out.insert(
            0,
            Section {
                start: 0,
                end: first.start,
                name: None,
            },
        ),
        Some(_) => {}
        None => out.push(Section {
            start: 0,
            end: lines.len(),
            name: None,
        }),
    }
    out
}

/// Which section a caller's name asks for, or the first one.
///
/// By name rather than by index, because indexes move: the `Problems` section disappears the
/// moment the last diagnostic is fixed, and every heading after it shifts down by however
/// many lines it was. A caller holding an index would find itself on a different section the
/// frame after the fix; a caller holding a name stays where it was.
fn resolve(name: Option<&str>, all: &[Section<'_>]) -> usize {
    name.and_then(|name| all.iter().position(|section| section.name == Some(name)))
        .unwrap_or(0)
}

/// Draw the rail, and answer where each of its items went.
///
/// The page's one control that is not a setting: the sections, in the order the caller put
/// them in, with the one being shown filled. An item is the section's own name in the same type
/// as the heading it stands for, because it *is* that heading: a rail item is not a second name
/// for a section, it is the section's name moved to where it can be used to choose.
///
/// Laid out from the top of the page with the page's own padding, one row apart. A rail
/// longer than the page is not scrolled: sections are counted in single digits in every
/// configuration this app has, and a scrolling rail is a second scroll for the pointer's
/// wheel to be sorted between.
fn draw_rail(
    paint: &mut Painter<'_>,
    input: &ChromeInput<'_>,
    page: Rect,
    width: f32,
    sections: &[Section<'_>],
    chosen: usize,
    hover: &Hover,
) -> Vec<(usize, Rect)> {
    let palette = *input.palette;
    let mut items = Vec::with_capacity(sections.len());
    let mut y = page.y + PAD;
    for (index, section) in sections.iter().enumerate() {
        let item = Rect::new(page.x + PAD, y, (width - 2.0 * PAD).max(0.0), ROW);
        // The page's own pair of fills: `hairline` for the one you are on, `ground` for the
        // one under the pointer. The same two the page gives a control and its hover, which
        // is what makes the rail read as part of the same surface rather than a second thing
        // bolted to its edge.
        // The section you are on is `hairline` and is drawn on the frame it is chosen, which
        // is not a hover: it is a fill that says which page this is, and a page that faded in
        // would be a page that had not decided yet. Only the pointer's own item fades.
        if index == chosen {
            paint.fill(item, palette.hairline);
        } else {
            paint.fill_at(item, palette.ground, hover.of(Hit::Section(section.start)));
        }
        let style =
            TextStyle::new(HEADING_SIZE, Weight::MEDIUM, palette.ink).tracking(HEADING_TRACKING);
        let baseline = paint.baseline_in(item, HEADING_SIZE);
        // The caller's own words, neither upper-cased nor otherwise altered: `Appearance` is
        // how the app spells it, and a case transform here would be the painter disagreeing
        // with the configuration file about the name of a section.
        paint.text(section.name.unwrap_or_default(), item.x, baseline, style);
        items.push((section.start, item));
        y += ROW;
    }
    // The seam between the rail and the page: one pixel, the same hairline the page's own
    // left edge wears, because a seam inside a surface is a depth step and this crate has
    // exactly one way to say that.
    paint.fill(
        Rect::new(page.x + width, page.y, 1.0, page.height),
        palette.hairline,
    );
    items
}

/// How wide the rail is inside a page of `width`.
///
/// Two fifths of the page at most, which only bites in a window too narrow to be showing the
/// page's full width anyway: at that point the rail is the part that gives, because a rail of
/// section names is legible truncated and a page of settings values is not.
fn rail_width(width: f32) -> f32 {
    PANEL_RAIL.min(width * 0.4)
}

/// Draw the settings page and answer where it went.
///
/// The page occupies the content area — the same rectangle the grid would have had — on
/// `surface-raised`, with a hairline between it and anything beside it. It is not an overlay
/// and does not resize anything: the caller does not draw the terminal while this is up, so
/// what is behind it is the window's own ground rather than a grid this is covering.
///
/// The lines come from the caller. What a setting *is* belongs to the configuration,
/// which this crate is not given, and a second copy of it living next to the painter is
/// exactly how a page and a file start disagreeing — so the page owns the geometry,
/// the type, and the hit regions, and the caller owns the words and the values.
///
/// Lines that do not fit are scrolled rather than dropped. A page that silently stops
/// listing settings once the window is short is a page where the user cannot find a
/// setting and has no way to tell that it is there.
pub(crate) fn page(
    paint: &mut Painter<'_>,
    input: &ChromeInput<'_>,
    content: Rect,
    hover: &Hover,
) -> Page {
    let palette = *input.palette;
    // The content area, given rather than worked out here: it is the grid's rectangle, and
    // the chrome already computes it — a second opinion about where the content starts would
    // be a page that disagreed with the terminal about the same row of pixels.
    let rect = content;
    paint.fill(rect, palette.surface_raised);
    // A hairline on the left edge only when something is there to be separated from: with the
    // tabs in a rail the page begins beside it and the seam is real, and with the tabs in a row
    // the page begins at the window's own edge, where a one-pixel line is a line drawn on
    // nothing.
    if rect.x > 0.0 {
        paint.fill(
            Rect::new(rect.x, rect.y, 1.0, rect.height),
            palette.hairline,
        );
    }

    // The rail, and the page beside it. The rail exists only when there is more than one
    // section to choose between and every one of them has a name: a rail with a single item
    // in it is a control that cannot do anything, and a list of rows with no headings at all
    // — which a caller is free to hand over, and which this crate's own tests do — is one
    // page and no rail, exactly as the page was before there was a rail.
    let all = sections(input.settings);
    let railed = all.len() > 1 && all.iter().all(|section| section.name.is_some());
    let chosen = resolve(input.settings_section, &all);
    let rail = if railed { rail_width(rect.width) } else { 0.0 };
    let page = Rect::new(rect.x + rail, rect.y, rect.width - rail, rect.height);
    // The chosen section's rows and not its heading, when there is a rail: the rail says what
    // the section is, and a page repeating the word under the rail item that already says it
    // is a line of nothing. With no rail the page is the whole list, headings and all, which
    // is what the page drew before any of this existed.
    let lines: Range<usize> = if railed {
        all[chosen].start + 1..all[chosen].end
    } else {
        0..input.settings.len()
    };

    let mut sections_out = Vec::new();
    if railed {
        sections_out = draw_rail(paint, input, rect, rail, &all, chosen, hover);
    }

    // What the page would take, so the scroll can be clamped to the overflow rather than to a
    // number the caller guessed. This walks the page twice, and the second walk is the one
    // that draws.
    let scroll = page_scroll(input, &lines, page);

    let mut controls = Vec::new();
    let mut y = page.y + PAD - scroll;
    for (at, setting) in input.settings[lines.clone()].iter().enumerate() {
        // The line's index in the whole list rather than in the page, because that is what a
        // click on it names and what the caller resolves against its own rows.
        let line = lines.start + at;
        let focused = input.settings_focus == Some(line);
        let (lead, height) = block(setting, line == lines.start);
        y += lead;
        if setting.row == crate::Row::Heading {
            // A section heading, and the rule under it.
            //
            // One decision for the two, on a box that covers both. The rule is the
            // heading's, and it sits the height of the heading's box below the top of it;
            // asked separately it can pass the containment test while the text above it
            // does not, which draws a full-width hairline with nothing over it — a line
            // that reads as a rule for whichever row happens to sit above it.
            let heading_box = Rect::new(page.x + PAD, y, page.width - 2.0 * PAD, HEADING_BOX);
            let rule = Rect::new(page.x, y + HEADING_BOX, page.width, 1.0);
            // The box the pair is culled by: the heading's, one hairline taller.
            let block = Rect::new(
                heading_box.x,
                heading_box.y,
                heading_box.width,
                HEADING_BOX + 1.0,
            );
            if visible(block, page) {
                let style = TextStyle::new(HEADING_SIZE, Weight::MEDIUM, palette.ink)
                    .tracking(HEADING_TRACKING);
                // The caller's own words. DESIGN.md used to fix the case here — "12px
                // uppercase with a hairline under it" — and sentence case is what the
                // document says now: a heading is set at 12px with a hairline under it, and
                // the case is the app's. The app owns the words; the page owns how a heading
                // is set, and no longer includes shouting among the things it decides.
                paint.centered(setting.text, heading_box, style);
                paint.fill(rule, palette.hairline);
            }
            y += height;
            continue;
        }

        let row = Rect::new(page.x + PAD, y, page.width - 2.0 * PAD, ROW);
        let crate::Row::Control(control) = setting.row else {
            // Something the configuration file got wrong: a line to read rather than a
            // control to click.
            if visible(row, page) {
                note(paint, setting, row, &palette);
            }
            y += height;
            continue;
        };

        let control_rect = Rect::new(
            row.right() - CONTROL_WIDTH,
            y + (ROW - CONTROL_HEIGHT) / 2.0,
            CONTROL_WIDTH,
            CONTROL_HEIGHT,
        );
        if visible(row, page) {
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
            draw_control(
                paint,
                input,
                control,
                control_rect,
                setting.value,
                focused,
                line,
                hover,
            );
            for (part, rect) in parts(control, control_rect) {
                controls.push((line, part, rect));
            }
        }
        y += height;
    }

    Page {
        rect,
        controls,
        sections: sections_out,
        shown: railed.then(|| all[chosen].start),
        scroll,
    }
}

/// Something the configuration file got wrong: a message, and how serious it is.
///
/// Nothing is pushed into the page's controls. There is nothing here to click, and a hit
/// region for it would be a row that answers a click by doing nothing.
///
/// The severity sits where a control's value would, so the word lands in the same column
/// as the settings below it and the list reads as one table. The message keeps `ink` and
/// the severity `ink_mid`, which is the page's own split between what a row says and
/// what it is — and no colour is spent on the pair, because `error` and `warning` are two
/// degrees of the same thing, and a shade that meant "bad" would be claiming a difference
/// the page does not know how to draw at the warning end.
///
/// Culling is the caller's, and so is the row's height: this is handed a box that is
/// already known to be on screen.
fn note(
    paint: &mut Painter<'_>,
    setting: &SettingLine<'_>,
    row: Rect,
    palette: &zet_config::Palette,
) {
    let value_box = Rect::new(row.right() - CONTROL_WIDTH, row.y, CONTROL_WIDTH, ROW);
    let label_box = Rect::new(
        row.x,
        row.y,
        (row.width - CONTROL_WIDTH - LABEL_GAP).max(0.0),
        ROW,
    );
    paint.centered(
        setting.text,
        label_box,
        TextStyle::new(LABEL_SIZE, Weight::NORMAL, palette.ink),
    );
    paint.centered(
        setting.value,
        value_box,
        TextStyle::new(LABEL_SIZE, Weight::NORMAL, palette.ink_mid),
    );
}

/// Whether a box can be drawn without any of it escaping the page.
///
/// Containment, not intersection. There is no scissor under this — the painter pushes
/// rectangles and glyphs straight into the frame — so a row that is scrolled half off
/// the top of the list draws its control, its border and its value over whatever the
/// chrome put above the page, which is the tab strip. A row is drawn once all of it is
/// on the page, and the page's own padding is wide enough that it slides in over that
/// rather than over the strip.
///
/// Both axes, since the rail arrived. On the page's left the old answer was "nothing can be
/// there" — a row is laid out the page's padding in from the page's own edge — and that is
/// no longer true: the rail is inside the surface, and a row wide enough to reach it would be
/// a setting drawn under a section name.
fn visible(box_: Rect, page: Rect) -> bool {
    box_.x >= page.x
        && box_.right() <= page.right()
        && box_.y >= page.y
        && box_.bottom() <= page.bottom()
}

/// The eight pixels either side of a control that a hover reads as "on this one".
#[allow(clippy::too_many_arguments)]
fn draw_control(
    paint: &mut Painter<'_>,
    input: &ChromeInput<'_>,
    control: Control,
    rect: Rect,
    value: &str,
    focused: bool,
    line: usize,
    hover: &Hover,
) {
    let palette = *input.palette;
    // A control is the ground behind a hairline, which is DESIGN.md's one depth
    // mechanism applied to something that is not a surface: the page is raised, so the
    // thing you can press is recessed into it.
    paint.fill(rect, palette.ground);
    // The fill follows the click: a half is lit by asking for the hover value of the very
    // `Hit` the click handler is given, so the two cannot disagree about how many ways a
    // control can be pressed. A stepper answers twice and one half lights; every other
    // control answers once and all of itself lights, which is what its whole face being the
    // click target means. Filling the left half of everything said a toggle was two controls
    // wearing one rectangle, and a click on the right half — which works — lit a region it
    // was not in.
    //
    // The edge is the *whole* control's, though, which a stepper's two halves do not
    // describe: so it takes the larger of them, which is the half the pointer is in and zero
    // when it is in neither.
    let mut lit = 0.0_f32;
    for (part, part_rect) in parts(control, rect) {
        let half = hover.of(Hit::Setting { line, part });
        lit = lit.max(half);
        paint.fill_at(part_rect, palette.hairline, half);
    }
    // Three weights of the same hairline, and no fourth: the control you are on is
    // `ink`, the one under the pointer is `hairline-strong`, and the rest are `hairline`.
    // `signal` is the obvious colour for a focus ring and the wrong one — DESIGN.md gives
    // it a 3px by 40px budget and it is the app's one lamp, which a border around a
    // 118-pixel control would spend several times over.
    //
    // Focus outranks the pointer for the same reason the active tab outranks it: the row the
    // keyboard is on is where the next keystroke goes, and a row that dimmed to
    // `hairline-strong` because a mouse happened to cross it would be lying about that.
    if focused {
        border(paint, rect, palette.ink, 1.0);
    } else {
        border(paint, rect, palette.hairline, 1.0 - lit);
        border(paint, rect, palette.hairline_strong, lit);
    }
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

/// How far the page is scrolled this frame, clamped to what actually overflows.
///
/// The scroll the caller asked for and the row the keyboard is on are two answers to one
/// question, and the second wins: a row the keyboard is on has to be on screen, or the keys
/// move a highlight nobody can see and the page looks broken rather than scrolled. A focus
/// that is not in this page is left alone — the caller moves the section and the focus
/// together, so a mismatch is a frame in flight rather than a row to scroll to.
///
/// What it is clamped to is what the page overflows rather than the whole list, because the
/// rail means the page is a section and the caller's number was measured against a page that is
/// no longer being drawn.
fn page_scroll(input: &ChromeInput<'_>, lines: &Range<usize>, page: Rect) -> f32 {
    let content = list_height(input.settings, lines);
    let viewport = (page.height - 2.0 * PAD).max(0.0);
    let overflow = (content - viewport).max(0.0);
    let mut scroll = input.settings_scroll.clamp(0.0, overflow);
    if let Some(focus) = input.settings_focus.filter(|focus| lines.contains(focus)) {
        let top = content_top(input.settings, lines, focus);
        if top < scroll {
            scroll = top;
        } else if top + ROW > scroll + viewport {
            scroll = top + ROW - viewport;
        }
        scroll = scroll.clamp(0.0, overflow);
    }
    scroll
}

/// What one line of the page takes: the gap above it, and its own height.
///
/// The one place the page's vertical rhythm is written down. The walk in [`page`] and
/// the two measurements below all go through this, so the scroll the caller asked for
/// can be clamped to what actually overflows rather than to a number it guessed at, and
/// the three of them cannot drift apart.
///
/// The gap above a section heading belongs to the heading rather than to the section
/// before it, so the heading that opens a page is not pushed down by a gap with nothing
/// above it to separate it from. `first` is the page's first line rather than the list's,
/// which is the change the rail made: a heading that used to be the fourth line of the
/// page is now the first line of a page.
fn block(setting: &SettingLine<'_>, first: bool) -> (f32, f32) {
    match setting.row {
        crate::Row::Heading => (
            if first { 0.0 } else { SECTION_GAP },
            HEADING_BOX + 1.0 + PAD / 2.0,
        ),
        // A problem is a row's height and takes a row's place: it is one line of text, and
        // the section gap belongs above the heading that names the section rather than
        // above each thing in it.
        crate::Row::Note | crate::Row::Control(_) => (0.0, ROW),
    }
}

/// How tall the page would be, drawn from its top.
fn list_height(lines: &[SettingLine<'_>], page: &Range<usize>) -> f32 {
    page.clone()
        .map(|line| {
            let (lead, height) = block(&lines[line], line == page.start);
            lead + height
        })
        .sum()
}

/// Where a line's content starts, measured from the top of the page.
///
/// The gap above a heading is not counted, because this answers "where would the page
/// have to be scrolled to for this line to be visible", and scrolling to a blank gap
/// puts nothing on screen. Every focusable line is a row, which has no gap at all.
///
/// `index` is a line in the whole list, which is what the caller owns; the page it is
/// measured in is passed along with it, because the scroll this is compared against is the
/// page's. A focus outside the page measures zero, which is a caller that has not moved the
/// two together yet.
fn content_top(lines: &[SettingLine<'_>], page: &Range<usize>, index: usize) -> f32 {
    let mut y = 0.0;
    for line in page.clone().take_while(|line| *line <= index) {
        let (lead, height) = block(&lines[line], line == page.start);
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
    border(paint, field, palette.hairline_strong, 1.0);

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

/// A one-pixel outline inside `rect`, at a coverage.
///
/// The coverage is what a hover cross-fading a control's edge is made of: the edge it is
/// leaving and the edge it is arriving at are drawn over each other, each at its share of the
/// transition. A zero coverage draws nothing at all, which is what makes the resting case
/// cost exactly what it did before there was a hover.
fn border(paint: &mut Painter<'_>, rect: Rect, color: zet_config::Rgb, alpha: f32) {
    paint.fill_at(Rect::new(rect.x, rect.y, rect.width, 1.0), color, alpha);
    paint.fill_at(
        Rect::new(rect.x, rect.bottom() - 1.0, rect.width, 1.0),
        color,
        alpha,
    );
    paint.fill_at(Rect::new(rect.x, rect.y, 1.0, rect.height), color, alpha);
    paint.fill_at(
        Rect::new(rect.right() - 1.0, rect.y, 1.0, rect.height),
        color,
        alpha,
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
    hover: &Hover,
) -> Option<(Rect, Rect)> {
    if scroll.visible >= 1.0 {
        return None;
    }
    let palette = *input.palette;
    // The width is the one thing in the chrome that a hover moves rather than colours, so it
    // is the one place the value is a distance. The band that lights it is wider than the
    // track it lights — see `Chrome::hover_target`, which is where that is squared with the
    // key the value is filed under — so the target is one and the drawn width is a lerp of it.
    let width = lerp(
        SCROLLBAR,
        SCROLLBAR_HOVER,
        hover.of(Hit::Scrollbar(crate::Scrollbar::Thumb)),
    );
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
