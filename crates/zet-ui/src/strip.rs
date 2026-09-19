//! The tab strip: the app name, the tabs, the new-tab mark, the drag region, and the
//! caption buttons.
//!
//! Both positions of the strip are planned here, because they are one layout seen from
//! two directions rather than two layouts. What a tab *is* does not change — a number
//! counting from one, a cell of a certain width, an indicator on the edge the strip runs
//! along — and only the axis does. Keeping them together is what makes "the strip is the
//! same in both positions" a fact about the code rather than a claim about it.

use zet_config::{Rgb, TabPosition};
use zet_font::Weight;

use crate::Caption;
use crate::ChromeInput;
use crate::TabInfo;
use crate::geometry::{
    CAPTION_WIDTH, HASH_RATIO, INDICATOR, NAME_GAP, NAME_INSET, NAME_SIZE, NAME_TRACKING,
    RAIL_CELL, RAIL_CELL_FLOOR, RAIL_WIDTH, ROW_HEIGHT, Rect, Size, TAB_GAP, TAB_MAX_WIDTH,
    TAB_PADDING, TAB_SIZE, TRAVEL,
};
use crate::marks::{self, Mark};
use crate::paint::{Painter, TextStyle};

/// One tab's cell: where it is, and how much of its name fits on it.
///
/// The name is a count rather than a string. A title is already owned by whoever
/// supplied the [`TabInfo`] and this is a plan for drawing it, so the cell carries the
/// one number that is not recoverable from outside — how much of it there is room for —
/// and the drawing reads the characters back off the input.
pub(crate) struct TabCell {
    /// The tab's number, which is its identity.
    pub index: u32,
    /// The cell.
    pub rect: Rect,
    /// How many characters of the tab's title fit.
    ///
    /// Zero on a cell with no room for a name, which is every cell in the rail and every
    /// cell in a run that has been squeezed down to its numbers.
    pub title: usize,
}

/// Where the strip's parts are.
///
/// Planned before anything is drawn, because the indicator's travel is a question about
/// two of these rectangles and the hit regions are a question about all of them.
pub(crate) struct Strip {
    /// Which way the run goes.
    pub position: TabPosition,
    /// Whether the 40-pixel row exists at all.
    ///
    /// False with no tabs, and that one flag is the whole of the zero-tab rule: no
    /// surface, no name, no hairline, no drag region, and `Layout::top` of zero.
    pub row: bool,
    /// The app name's box, when the strip had room for it.
    pub name: Option<Rect>,
    /// The tabs that fit, in order, with their numbers.
    pub tabs: Vec<TabCell>,
    /// The new-tab mark's box, when there was room for it.
    pub plus: Option<Rect>,
    /// The draggable gap.
    pub drag: Option<Rect>,
    /// The three caption buttons, which exist whether or not anything else does.
    pub captions: [(Caption, Rect); 3],
}

/// The indicator's travel while one is in flight.
pub(crate) struct Travel {
    /// The position it started in. A window that changes the strip's position mid-travel
    /// has two rectangles that are not comparable, so the travel is dropped instead.
    pub position: TabPosition,
    /// When it started, on the caller's clock.
    pub start: f32,
    /// The bar's rectangle where it started, which is where it was drawn last frame
    /// rather than where it was heading: pressing `Ctrl+Tab` twice in a row is one bar
    /// moving twice, not one bar snapping back and starting again.
    pub from: Rect,
    /// The tab it is leaving, whose index cross-fades back down to its resting weight.
    pub from_index: u32,
}

/// Plan the strip for one frame.
pub(crate) fn plan(
    paint: &mut Painter<'_>,
    input: &ChromeInput<'_>,
    position: TabPosition,
) -> Strip {
    let captions = caption_boxes(input.size);
    if input.tabs.is_empty() {
        // "The strip does not collapse to a thin line and does not show an empty state
        // message. It is simply absent."
        return Strip {
            position,
            row: false,
            name: None,
            tabs: Vec::new(),
            plus: None,
            drag: None,
            captions,
        };
    }
    match position {
        TabPosition::Top => horizontal(paint, input, captions),
        TabPosition::Left => vertical(paint, input, captions),
    }
}

/// The three caption buttons, flush against the right edge.
///
/// In Windows' order, because these are among the most-hit pixels on the machine: a
/// close button that has moved is a habit broken for nothing.
fn caption_boxes(size: Size) -> [(Caption, Rect); 3] {
    let left = size.width - 3.0 * CAPTION_WIDTH;
    [
        (
            Caption::Minimize,
            Rect::new(left, 0.0, CAPTION_WIDTH, ROW_HEIGHT),
        ),
        (
            Caption::Maximize,
            Rect::new(left + CAPTION_WIDTH, 0.0, CAPTION_WIDTH, ROW_HEIGHT),
        ),
        (
            Caption::Close,
            Rect::new(left + 2.0 * CAPTION_WIDTH, 0.0, CAPTION_WIDTH, ROW_HEIGHT),
        ),
    ]
}

/// One row, the name at the left and the tabs running right from it.
fn horizontal(
    paint: &mut Painter<'_>,
    input: &ChromeInput<'_>,
    captions: [(Caption, Rect); 3],
) -> Strip {
    let limit = captions[0].1.x;
    let title = input.window_title;
    let name_width = paint.advance(title, NAME_SIZE, Weight::NORMAL);
    let name_block = if title.is_empty() {
        0.0
    } else {
        NAME_INSET + name_width + NAME_GAP
    };

    // The new-tab mark is the width of a tab with nothing to say: a number and its
    // padding. It is the affordance that must not be the thing that overflows, so it
    // never gives up room and is never the cell that gets squeezed.
    let plus_width = number_cell(paint, 1);
    let natural: f32 = input
        .tabs
        .iter()
        .map(|tab| tab_width(paint, tab, TAB_MAX_WIDTH))
        .sum();

    // "The app name is hidden when the tab strip needs the space. Tabs outrank
    // branding." In arithmetic that is one comparison: the name is drawn only when
    // every tab still fits beside it, and when it is not drawn it takes its inset with
    // it and the run starts at the window's edge.
    let shows_name = !title.is_empty() && name_block + natural + plus_width <= limit;
    let name = shows_name.then(|| Rect::new(NAME_INSET, 0.0, name_width, ROW_HEIGHT));

    let start = if shows_name { name_block } else { 0.0 };
    let cap = tab_cap(paint, input.tabs, (limit - start - plus_width).max(0.0));

    let mut cursor = start;
    let mut tabs = Vec::with_capacity(input.tabs.len());
    for tab in input.tabs {
        let width = tab_width(paint, tab, cap);
        // A tab that would run under the new-tab mark is not drawn and cannot be hit.
        // Sizing has already shrunk every cell towards its number to put that off, so
        // this is only reached when even a row of bare numbers does not fit: the tab is
        // still numbered, it simply has nowhere to be until the window grows or a tab
        // before it closes.
        if cursor + width + plus_width > limit {
            break;
        }
        let room = width - number_cell(paint, tab.index) - TAB_GAP;
        tabs.push(TabCell {
            index: tab.index,
            rect: Rect::new(cursor, 0.0, width, ROW_HEIGHT),
            title: title_fit(paint, &tab.title, room),
        });
        cursor += width;
    }

    let plus =
        (cursor + plus_width <= limit).then(|| Rect::new(cursor, 0.0, plus_width, ROW_HEIGHT));
    let end = plus.map_or(cursor, Rect::right);
    let drag = Rect::between(end, 0.0, limit, ROW_HEIGHT);

    Strip {
        position: TabPosition::Top,
        row: true,
        name,
        tabs,
        plus,
        drag: Some(drag),
        captions,
    }
}

/// A row across the top, and the tabs down a rail beneath it.
fn vertical(
    paint: &mut Painter<'_>,
    input: &ChromeInput<'_>,
    captions: [(Caption, Rect); 3],
) -> Strip {
    let limit = captions[0].1.x;
    let title = input.window_title;
    let name_width = paint.advance(title, NAME_SIZE, Weight::NORMAL);
    let name_block = if title.is_empty() {
        0.0
    } else {
        NAME_INSET + name_width + NAME_GAP
    };
    // The tabs are in the rail, so nothing in the row competes with the name and it is
    // on screen unless the window is too narrow to hold it and the caption buttons.
    let shows_name = !title.is_empty() && name_block <= limit;
    let name = shows_name.then(|| Rect::new(NAME_INSET, 0.0, name_width, ROW_HEIGHT));
    let drag = Rect::between(
        if shows_name { name_block } else { 0.0 },
        0.0,
        limit,
        ROW_HEIGHT,
    );

    // "Overflowing tabs compress the cell height to a 24px floor." The floor is the
    // whole of the rule this crate can honour: what comes next is the rail scrolling
    // under the wheel, and a wheel position is not in `ChromeInput`.
    let available = (input.size.height - ROW_HEIGHT).max(0.0);
    let cells = input.tabs.len() + 1;
    let cell = (available / cells as f32).clamp(RAIL_CELL_FLOOR, RAIL_CELL);

    let mut y = ROW_HEIGHT;
    let mut tabs = Vec::with_capacity(input.tabs.len());
    for tab in input.tabs {
        // One cell is held back for the new-tab mark, which is the affordance that must
        // not be the thing that overflows.
        if y + 2.0 * cell > input.size.height {
            break;
        }
        // The rail carries numbers and no names. It is forty-eight pixels wide and the
        // whole reason to choose it is that it gives the grid the rest, so a name in it
        // would be a name in the space the tabs were moved aside to free.
        tabs.push(TabCell {
            index: tab.index,
            rect: Rect::new(0.0, y, RAIL_WIDTH, cell),
            title: 0,
        });
        y += cell;
    }
    let plus = (y + cell <= input.size.height).then(|| Rect::new(0.0, y, RAIL_WIDTH, cell));

    Strip {
        position: TabPosition::Left,
        row: true,
        name,
        tabs,
        plus,
        drag: Some(drag),
        captions,
    }
}

/// How wide a tab's number is, in the weight the number is measured in.
fn number_width(paint: &mut Painter<'_>, index: u32, style: TextStyle) -> f32 {
    let mut buffer = [0u8; 10];
    let digits = index_text(&mut buffer, index);
    paint.width("#", style.scaled(HASH_RATIO)) + paint.width(digits, style)
}

/// The style a tab's number is measured in.
///
/// MEDIUM, because that is the heaviest the number is ever drawn and the two weights do
/// not have the same advances. A cell sized for the lighter one would shift its name by
/// a fraction of a pixel every time the tab became active, which over a row of tabs is
/// a row that twitches when the user switches between them.
fn number_style() -> TextStyle {
    TextStyle::new(TAB_SIZE, Weight::MEDIUM, Rgb::BLACK)
}

/// How wide the cell is that holds a number and nothing else.
///
/// The floor a tab is squeezed to, and the footprint the new-tab mark takes.
fn number_cell(paint: &mut Painter<'_>, index: u32) -> f32 {
    number_width(paint, index, number_style()) + 2.0 * TAB_PADDING
}

/// How wide a tab's cell is: its number, its name when there is room for one, the
/// padding either side, and never more than `cap`.
fn tab_width(paint: &mut Painter<'_>, tab: &TabInfo, cap: f32) -> f32 {
    let floor = number_cell(paint, tab.index);
    if tab.title.is_empty() {
        return floor.min(cap);
    }
    let natural = floor + TAB_GAP + paint.advance(&tab.title, TAB_SIZE, Weight::NORMAL);
    // The floor wins over the cap. A cell narrower than its number is a cell that shows
    // a clipped number, and half a number is worse than a tab that is not on screen.
    natural.min(cap).max(floor)
}

/// How wide a tab's cell may be, given how many there are and how much run they share.
///
/// Tabs share the strip. When they all fit at their natural width nothing is capped —
/// the share is larger than any of them wants and the ceiling is the design's own
/// [`TAB_MAX_WIDTH`]. When they do not, every cell gives up the same amount, so a row
/// that is running out of room degrades evenly instead of being one wide tab followed by
/// a row of clipped ones.
///
/// The floor is the number-only cell, because a tab showing no number is not a tab. Below
/// it the share stops shrinking and the run overflows, which is the point at which
/// [`horizontal`] starts leaving tabs off the end.
fn tab_cap(paint: &mut Painter<'_>, tabs: &[TabInfo], available: f32) -> f32 {
    if tabs.is_empty() {
        return TAB_MAX_WIDTH;
    }
    let floor = number_cell(paint, 1);
    let share = available / tabs.len() as f32;
    share.clamp(floor.min(TAB_MAX_WIDTH), TAB_MAX_WIDTH)
}

/// How many characters of `title` fit in `room`, with an ellipsis after them.
///
/// Counted rather than returned as a truncated string: a caller that has the title
/// already can slice it, and this runs once per tab per frame, so the crate's rule about
/// not allocating in the chrome holds here too.
///
/// A title that fits whole is never cut to make room for an ellipsis it does not need:
/// the room the ellipsis would take is only given up once there is a character it would
/// stand in for. Reserving it unconditionally costs a name a character at exactly the
/// width where the name would have fitted, which is the one width a user is most likely
/// to hit — a title they chose the length of.
fn title_fit(paint: &mut Painter<'_>, title: &str, room: f32) -> usize {
    if room <= 0.0 {
        return 0;
    }
    let ellipsis = paint.advance("…", TAB_SIZE, Weight::NORMAL);
    let mut buffer = [0u8; 4];
    let mut running = 0.0;
    let mut chars = 0;
    let mut whole = 0;
    let mut cut = 0;
    for (at, ch) in title.chars().enumerate() {
        chars = at + 1;
        running += paint.advance(ch.encode_utf8(&mut buffer), TAB_SIZE, Weight::NORMAL);
        if running <= room {
            whole = chars;
        }
        if running + ellipsis <= room {
            cut = chars;
        }
    }
    if whole == chars { whole } else { cut }
}

/// The decimal digits of a tab index, most significant first.
///
/// Written out rather than formatted, because this runs once per tab per frame and a
/// terminal's chrome is not a place to allocate a `String` sixty times a second.
fn index_text(buffer: &mut [u8; 10], index: u32) -> &str {
    let mut start = buffer.len();
    let mut value = index;
    loop {
        start -= 1;
        buffer[start] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    // Twelve lines above produce ASCII digits and nothing else.
    core::str::from_utf8(&buffer[start..]).unwrap_or("")
}

/// Draw the strip's own surfaces, its text, and the caption marks.
///
/// The indicator is not here: it is the one part of the strip that is between two
/// places, and where it is belongs to whoever is holding the clock.
pub(crate) fn draw(
    paint: &mut Painter<'_>,
    strip: &Strip,
    input: &ChromeInput<'_>,
    fade: Option<(u32, f32)>,
) {
    let palette = *input.palette;

    if strip.position == TabPosition::Left {
        let rail = Rect::new(
            0.0,
            ROW_HEIGHT,
            RAIL_WIDTH,
            (input.size.height - ROW_HEIGHT).max(0.0),
        );
        paint.fill(rail, palette.surface);
        // The divider between the rail and the grid.
        paint.fill(
            Rect::new(RAIL_WIDTH - 1.0, ROW_HEIGHT, 1.0, rail.height),
            palette.hairline,
        );
    }

    if strip.row {
        paint.fill(
            Rect::new(0.0, 0.0, input.size.width, ROW_HEIGHT),
            palette.surface,
        );
        // "Maximized: the row loses its bottom hairline."
        if !input.maximized {
            paint.fill(
                Rect::new(0.0, ROW_HEIGHT - 1.0, input.size.width, 1.0),
                palette.hairline,
            );
        }
        if let Some(name) = strip.name {
            let style =
                TextStyle::new(NAME_SIZE, Weight::NORMAL, palette.ink_mid).tracking(NAME_TRACKING);
            paint.centered(input.window_title, name, style);
        }
    }

    for cell in &strip.tabs {
        // A cell was planned from a tab that is still in the input, so this cannot miss;
        // skipping rather than indexing keeps that from being a panic if it ever does.
        let Some(info) = input.tabs.iter().find(|tab| tab.index == cell.index) else {
            continue;
        };
        let active = Some(cell.index) == input.active;
        let resting = if info.hovered {
            palette.ink_mid
        } else {
            palette.ink_dim
        };

        // How much of the active weight this index currently carries. At rest it is one
        // or zero; while the bar is in flight it is the cross-fade, and the tab the bar
        // is leaving is the same curve running down. Nothing else about a tab moves.
        let heavy = match fade {
            Some((_, t)) if active => t,
            Some((from, t)) if from == cell.index => 1.0 - t,
            _ if active => 1.0,
            _ => 0.0,
        };

        let x = if strip.position == TabPosition::Left {
            let width = number_cell(paint, cell.index) - 2.0 * TAB_PADDING;
            cell.rect.x + (cell.rect.width - width) / 2.0
        } else {
            cell.rect.x + TAB_PADDING
        };
        let baseline = paint.baseline_in(cell.rect, TAB_SIZE);

        if heavy > 0.002 {
            tab_label(
                paint,
                x,
                baseline,
                info,
                cell.title,
                TextStyle::new(TAB_SIZE, Weight::MEDIUM, palette.ink).faded(heavy),
                TextStyle::new(TAB_SIZE, Weight::NORMAL, palette.ink).faded(heavy),
            );
        }
        if heavy < 0.998 {
            tab_label(
                paint,
                x,
                baseline,
                info,
                cell.title,
                TextStyle::new(TAB_SIZE, Weight::NORMAL, resting).faded(1.0 - heavy),
                TextStyle::new(TAB_SIZE, Weight::NORMAL, palette.ink_mid).faded(1.0 - heavy),
            );
        }
    }

    if let Some(plus) = strip.plus {
        let style = TextStyle::new(TAB_SIZE, Weight::NORMAL, palette.ink_dim);
        paint.centered("+", plus, style);
    }
}

/// A tab's whole label: the `#` at seventy percent, the number, then as much of the name
/// as the cell has room for.
///
/// The number and the name are drawn as one run so that they cannot disagree about how
/// present the tab is, which is the one thing the indicator's cross-fade moves.
///
/// The name is sliced out of the title the caller already owns — by character, so a
/// multi-byte one is never cut in half — and an ellipsis is appended when the slice is
/// short. The cell reserved room for that ellipsis when it counted the characters, so
/// nothing here can overflow the cell.
fn tab_label(
    paint: &mut Painter<'_>,
    x: f32,
    baseline: f32,
    info: &TabInfo,
    fit: usize,
    number: TextStyle,
    name: TextStyle,
) {
    let mut buffer = [0u8; 10];
    let digits = index_text(&mut buffer, info.index);
    let hash = number.scaled(HASH_RATIO);
    let hash_width = paint.width("#", hash);
    paint.text("#", x, baseline, hash);
    paint.text(digits, x + hash_width, baseline, number);

    if fit == 0 {
        return;
    }
    let title = info.title.as_str();
    let cut = title
        .char_indices()
        .nth(fit)
        .map_or(title.len(), |(at, _)| at);
    let (shown, rest) = title.split_at(cut);
    let name_x = x + hash_width + paint.width(digits, number) + TAB_GAP;
    paint.text(shown, name_x, baseline, name);
    if !rest.is_empty() {
        let offset = paint.width(shown, name);
        paint.text("…", name_x + offset, baseline, name);
    }
}

/// Draw the three caption marks.
///
/// Last, and after the settings panel, because these are the window's own controls and
/// nothing in zet is allowed to cover them.
pub(crate) fn captions(paint: &mut Painter<'_>, strip: &Strip, input: &ChromeInput<'_>) {
    let palette = *input.palette;
    for (caption, rect) in &strip.captions {
        let hovered = input.pointer.is_some_and(|(x, y)| rect.contains(x, y));
        // DESIGN.md names one hover colour and it is the close button's. The other two
        // step up one ink rather than staying still, because a control with no hover
        // state is a control the user is not sure they are over.
        let color = match (caption, hovered) {
            (Caption::Close, true) => palette.danger,
            (_, true) => palette.ink,
            (_, false) => palette.ink_mid,
        };
        marks::draw(
            paint,
            mark_of(*caption, input.maximized),
            *rect,
            color,
            input.scale,
        );
    }
}

/// Which mark a caption button shows.
///
/// The middle button is one control with two shapes rather than two controls: it is the
/// same click either way, and which square it draws is the only thing the window's state
/// changes. Windows draws the restore glyph — two overlapping squares — on a window that
/// is already maximized, and a user who has maximized a window is looking for exactly
/// that difference.
const fn mark_of(caption: Caption, maximized: bool) -> Mark {
    match caption {
        Caption::Minimize => Mark::Minimize,
        Caption::Maximize if maximized => Mark::Restore,
        Caption::Maximize => Mark::Maximize,
        Caption::Close => Mark::Close,
    }
}

/// Where the indicator goes this frame.
///
/// One line, and the only one in the crate that moves.
pub(crate) fn indicator(
    strip: &Strip,
    input: &ChromeInput<'_>,
    travel: Option<&Travel>,
    now: f32,
) -> Option<(Rect, Rgb)> {
    let palette = *input.palette;
    let cell = strip
        .tabs
        .iter()
        .find(|cell| Some(cell.index) == input.active)?;
    let destination = bar_rect(strip.position, cell.rect);
    let rect = match travel {
        Some(travel) => travel.from.lerp(destination, progress(now - travel.start)),
        None => destination,
    };
    Some((rect, palette.signal))
}

/// The indicator's rectangle for a tab cell, on the edge the strip runs along.
fn bar_rect(position: TabPosition, cell: Rect) -> Rect {
    match position {
        TabPosition::Top => Rect::new(cell.x, ROW_HEIGHT - INDICATOR, cell.width, INDICATOR),
        TabPosition::Left => Rect::new(0.0, cell.y, INDICATOR, cell.height),
    }
}

/// The indicator's preview under a hovered tab.
///
/// Suppressed while one is travelling, because the bar in flight is the answer to "where
/// am I now" and a second one at rest would be a second answer.
pub(crate) fn hover_preview(
    strip: &Strip,
    input: &ChromeInput<'_>,
    travelling: bool,
) -> Option<Rect> {
    if travelling {
        return None;
    }
    let cell = strip.tabs.iter().find(|cell| {
        Some(cell.index) != input.active
            && input
                .tabs
                .iter()
                .any(|tab| tab.index == cell.index && tab.hovered)
    })?;
    Some(bar_rect(strip.position, cell.rect))
}

/// How far through its travel the indicator is, from zero to one, after `elapsed`
/// seconds.
///
/// Exponential ease-out, which is the curve DESIGN.md names for it: most of the distance
/// goes in the first third of the time and the rest settles rather than stopping.
pub(crate) fn progress(elapsed: f32) -> f32 {
    if elapsed <= 0.0 {
        0.0
    } else if elapsed >= TRAVEL {
        1.0
    } else {
        1.0 - 2f32.powf(-10.0 * elapsed / TRAVEL)
    }
}
