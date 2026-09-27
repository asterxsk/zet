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
use crate::Hit;
use crate::TabId;
use crate::TabInfo;
use crate::geometry::{
    CAPTION_WIDTH, HASH_RATIO, INDICATOR, NAME_GAP, NAME_INSET, NAME_SIZE, NAME_TRACKING,
    RAIL_CELL, RAIL_CELL_FLOOR, RAIL_WIDTH, ROW_HEIGHT, Rect, SETTINGS_CELL, Size, TAB_GAP,
    TAB_MAX_WIDTH, TAB_PADDING, TAB_SIZE, TRAVEL,
};
use crate::hover::Hover;
use crate::marks::{self, Mark};
use crate::paint::{Painter, TextStyle};

/// How strong the wash under a caption button that is not close is at full hover.
///
/// The three caption buttons share one behaviour — the button fills — and two colours: close
/// spends `danger`, because it is the one that takes something away, and the other two spend
/// a tenth of `ink`, which is a wash rather than a fill. Ten percent is what Windows uses for
/// the same two buttons, and it is the strength at which the mark over it still reads.
///
/// A `pub(crate)` constant rather than a local one, because a fade is only assertable at its
/// ends and halfway between them if the caller can name the value it is heading for.
pub(crate) const CAPTION_WASH: f32 = 0.10;

/// One tab's cell: where it is, and how much of its name fits on it.
///
/// The name is a count rather than a string. A title is already owned by whoever
/// supplied the [`TabInfo`] and this is a plan for drawing it, so the cell carries the
/// one number that is not recoverable from outside — how much of it there is room for —
/// and the drawing reads the characters back off the input.
pub(crate) struct TabCell {
    /// The tab's id, which is its identity.
    pub id: TabId,
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
    /// The settings control's box, when there was room for it after the new-tab mark.
    ///
    /// Its rect is planned here with the rest of the strip, because the two controls share
    /// the run the tabs do not have — but it is *drawn* from `Chrome::overdraw` rather than
    /// from [`draw`], with the window's other controls rather than with the strip's text.
    /// See [`settings_mark`].
    pub settings: Option<Rect>,
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
    /// The tab it is leaving, whose label cross-fades back down to its resting weight.
    pub from_id: TabId,
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
            settings: None,
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
    // never gives up room and is never the cell that gets squeezed. The settings control
    // beside it is a fixed sixteen pixels rather than a cell of any kind, and shares the
    // guarantee: the run of tabs stops before both of them.
    let plus_width = number_cell(paint, TabId::Terminal(1));
    let settings_width = SETTINGS_CELL;
    let controls = plus_width + settings_width;
    let natural: f32 = input
        .tabs
        .iter()
        .map(|tab| tab_width(paint, tab, TAB_MAX_WIDTH))
        .sum();

    // "The app name is hidden when the tab strip needs the space. Tabs outrank
    // branding." In arithmetic that is one comparison: the name is drawn only when
    // every tab still fits beside it, and when it is not drawn it takes its inset with
    // it and the run starts at the window's edge.
    let shows_name = !title.is_empty() && name_block + natural + controls <= limit;
    let name = shows_name.then(|| Rect::new(NAME_INSET, 0.0, name_width, ROW_HEIGHT));

    let start = if shows_name { name_block } else { 0.0 };
    // Both widths come off the share, not just the new-tab mark's: sizing the tabs against
    // room the loop below will not give them would shrink every cell towards its number to
    // make space for a tab that then cannot be drawn at all.
    let cap = tab_cap(paint, input.tabs, (limit - start - controls).max(0.0));

    let mut cursor = start;
    let mut tabs = Vec::with_capacity(input.tabs.len());
    for tab in input.tabs {
        let width = tab_width(paint, tab, cap);
        // A tab that would run under either control is not drawn and cannot be hit. Sizing
        // has already shrunk every cell towards its number to put that off, so this is only
        // reached when even a row of bare numbers does not fit: the tab is still numbered,
        // it simply has nowhere to be until the window grows or a tab before it closes.
        if cursor + width + controls > limit {
            break;
        }
        let room = width - number_cell(paint, tab.id) - number_gap(tab.id);
        tabs.push(TabCell {
            id: tab.id,
            rect: Rect::new(cursor, 0.0, width, ROW_HEIGHT),
            title: title_fit(paint, &tab.title, room),
        });
        cursor += width;
    }

    let plus =
        (cursor + plus_width <= limit).then(|| Rect::new(cursor, 0.0, plus_width, ROW_HEIGHT));
    // The settings control is dropped rather than squeezed when the two do not both fit: a
    // control drawn a few pixels narrower than its own mark is a mark with its edge cut off,
    // and the chord that opens the page is still there. The width it would have taken
    // becomes drag region, which is where a press on a strip with no room for controls
    // should land anyway.
    let settings = (cursor + controls <= limit)
        .then(|| Rect::new(cursor + plus_width, 0.0, settings_width, ROW_HEIGHT));
    let end = settings.map_or_else(|| plus.map_or(cursor, Rect::right), Rect::right);
    let drag = Rect::between(end, 0.0, limit, ROW_HEIGHT);

    Strip {
        position: TabPosition::Top,
        row: true,
        name,
        tabs,
        plus,
        settings,
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
    // Two cells more than there are tabs: one for the new-tab mark and one for the settings
    // control beneath it.
    let cells = input.tabs.len() + 2;
    let cell = (available / cells as f32).clamp(RAIL_CELL_FLOOR, RAIL_CELL);

    let mut y = ROW_HEIGHT;
    let mut tabs = Vec::with_capacity(input.tabs.len());
    for tab in input.tabs {
        // Two cells are held back, for the two controls below the tabs: they are the
        // affordances that must not be the things that overflow.
        if y + 3.0 * cell > input.size.height {
            break;
        }
        // The rail carries numbers and no names. It is forty-eight pixels wide and the
        // whole reason to choose it is that it gives the grid the rest, so a name in it
        // would be a name in the space the tabs were moved aside to free.
        tabs.push(TabCell {
            id: tab.id,
            rect: Rect::new(0.0, y, RAIL_WIDTH, cell),
            title: 0,
        });
        y += cell;
    }
    let plus = (y + cell <= input.size.height).then(|| Rect::new(0.0, y, RAIL_WIDTH, cell));
    // Directly below the new-tab mark, and present only when that one is: a settings control
    // sitting where the new-tab mark would have been, in a window too short for both, would
    // be the two controls swapping places as the window is resized.
    let settings = (plus.is_some() && y + 2.0 * cell <= input.size.height)
        .then(|| Rect::new(0.0, y + cell, RAIL_WIDTH, cell));

    Strip {
        position: TabPosition::Left,
        row: true,
        name,
        tabs,
        plus,
        settings,
        drag: Some(drag),
        captions,
    }
}

/// Draw the settings mark into the box the strip planned for it.
///
/// Called from the chrome's overdrawn layer rather than from [`draw`], with the window's other
/// controls: it goes through [`marks::draw`] like the caption buttons, so it is a rectangle on
/// the device grid rather than a character in the chrome's face, and it belongs with the
/// captions rather than with the strip's text.
///
/// Two states, and neither of them is a fill. The mark is `ink-mid` while there is no settings
/// tab and `ink` while there is one, which is how the mark stays lit for as long as the page
/// exists — the same rule the active tab's number follows, and for the same reason: what is
/// open is the thing that is bright. Under the pointer the resting ink steps up to `ink` over
/// the hover, which for a shut tab is a visible change and for an open one is nothing, because
/// a mark that is already lit has no second look to give.
pub(crate) fn settings_mark(
    paint: &mut Painter<'_>,
    strip: &Strip,
    input: &ChromeInput<'_>,
    hover: &Hover,
) {
    let Some(rect) = strip.settings else {
        return;
    };
    if input.tabs.iter().any(|tab| tab.id == TabId::Settings) {
        marks::draw(
            paint,
            Mark::Sliders,
            rect,
            input.palette.ink,
            1.0,
            input.scale,
        );
        return;
    }
    let lit = hover.of(Hit::SettingsButton);
    if lit < 0.998 {
        marks::draw(
            paint,
            Mark::Sliders,
            rect,
            input.palette.ink_mid,
            1.0 - lit,
            input.scale,
        );
    }
    if lit > 0.002 {
        marks::draw(
            paint,
            Mark::Sliders,
            rect,
            input.palette.ink,
            lit,
            input.scale,
        );
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
/// The floor a tab is squeezed to, and the footprint the new-tab mark takes. A settings cell
/// has no number, so its floor is the padding alone: the callers that ask for a floor are
/// asking "how narrow can this cell be and still show what it has", and what a settings cell
/// has is its name, which needs exactly the padding it is drawn with.
fn number_cell(paint: &mut Painter<'_>, id: TabId) -> f32 {
    match id {
        TabId::Terminal(index) => number_width(paint, index, number_style()) + 2.0 * TAB_PADDING,
        TabId::Settings => 2.0 * TAB_PADDING,
    }
}

/// The gap between a cell's number and its name.
///
/// Zero for a settings cell, which has no number for a gap to separate anything from. It is a
/// gap and not padding: the cell already has padding either side, and a settings cell asked for
/// both would be eight pixels wider than the name it holds with a hole in front of it.
const fn number_gap(id: TabId) -> f32 {
    match id {
        TabId::Terminal(_) => TAB_GAP,
        TabId::Settings => 0.0,
    }
}

/// How wide a tab's cell is: its number, its name when there is room for one, the
/// padding either side, and never more than `cap`.
fn tab_width(paint: &mut Painter<'_>, tab: &TabInfo, cap: f32) -> f32 {
    let floor = number_cell(paint, tab.id);
    if tab.title.is_empty() {
        return floor.min(cap);
    }
    let natural = floor + number_gap(tab.id) + paint.advance(&tab.title, TAB_SIZE, Weight::NORMAL);
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
    let floor = number_cell(paint, TabId::Terminal(1));
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
    fade: Option<(TabId, f32)>,
    hover: &Hover,
) {
    let palette = *input.palette;

    // Guarded by `strip.row` as well as the position, because a rail without a row is a
    // rail with no cells in it: the layout already hands the grid the whole window in that
    // case, so this would otherwise be 48px of empty surface drawn over the first column
    // the shell is writing to. "The strip is simply absent" is about the whole strip.
    if strip.row && strip.position == TabPosition::Left {
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
        let Some(info) = input.tabs.iter().find(|tab| tab.id == cell.id) else {
            continue;
        };
        let active = Some(cell.id) == input.active;
        // The active tab outranks the pointer, exactly as it does for the preview bar: its
        // number is `ink` whether the pointer is on it or not, and a hovered active tab that
        // faded towards `ink-mid` would be the strip's own rule losing to the mouse.
        let lit = if active {
            0.0
        } else {
            hover.of(Hit::Tab(cell.id))
        };

        // How much of the active weight this index currently carries. At rest it is one
        // or zero; while the bar is in flight it is the cross-fade, and the tab the bar
        // is leaving is the same curve running down. Nothing else about a tab moves.
        let heavy = match fade {
            Some((_, t)) if active => t,
            Some((from, t)) if from == cell.id => 1.0 - t,
            _ if active => 1.0,
            _ => 0.0,
        };

        let x = if strip.position == TabPosition::Left {
            let width = number_cell(paint, cell.id) - 2.0 * TAB_PADDING;
            cell.rect.x + (cell.rect.width - width) / 2.0
        } else {
            cell.rect.x + TAB_PADDING
        };
        let baseline = paint.baseline_in(cell.rect, TAB_SIZE);

        if heavy > 0.002 {
            let number = TextStyle::new(TAB_SIZE, Weight::MEDIUM, palette.ink).faded(heavy);
            let name = TextStyle::new(TAB_SIZE, Weight::NORMAL, palette.ink).faded(heavy);
            let start = tab_number(paint, x, baseline, info, number);
            tab_name(paint, start, baseline, info, cell.title, name);
        }
        if heavy < 0.998 {
            // The resting tab, in as many passes as the number has inks to be between. The
            // name is `ink-mid` whichever of them the number is on, so it is drawn once —
            // once per pass would be the same glyphs laid down twice, which is a name that
            // gets brighter as the number fades across.
            let light = 1.0 - heavy;
            let name = TextStyle::new(TAB_SIZE, Weight::NORMAL, palette.ink_mid).faded(light);
            let mut start = None;
            if lit < 0.998 {
                let number = TextStyle::new(TAB_SIZE, Weight::NORMAL, palette.ink_dim)
                    .faded(light * (1.0 - lit));
                start = Some(tab_number(paint, x, baseline, info, number));
            }
            if lit > 0.002 {
                let number =
                    TextStyle::new(TAB_SIZE, Weight::NORMAL, palette.ink_mid).faded(light * lit);
                start = Some(tab_number(paint, x, baseline, info, number));
            }
            if let Some(start) = start {
                tab_name(paint, start, baseline, info, cell.title, name);
            }
        }
    }

    if let Some(plus) = strip.plus {
        // Under the pointer the `+` is white rather than dim, and that is the whole of it: no
        // fill, no haze, nothing behind the mark. The two passes are the cross-fade the tab
        // numbers have always used, and it is the same pair of draws the settings mark wears
        // — one control's two looks, laid over each other at the share of the transition each
        // one holds.
        let lit = hover.of(Hit::NewTab);
        if lit < 0.998 {
            let style = TextStyle::new(TAB_SIZE, Weight::NORMAL, palette.ink_dim).faded(1.0 - lit);
            paint.centered("+", plus, style);
        }
        if lit > 0.002 {
            let style = TextStyle::new(TAB_SIZE, Weight::NORMAL, palette.ink).faded(lit);
            paint.centered("+", plus, style);
        }
    }
}

/// A tab's number: the `#` at seventy percent, then its digits.
///
/// Returns where the name that follows it starts. That is the one thing a caller cannot work
/// out for itself: the digits' advance belongs to the weight they are set in, and the
/// indicator's cross-fade moves the number between two weights.
///
/// The two halves of a tab's label are drawn separately rather than as one run, which is what
/// they used to be. The reason they were one was that they could not disagree about how
/// present the tab is, and that is still true of the *fade*: both callers pass the same alpha
/// to the number and the name. What the split buys is the hover, which moves the number's ink
/// and leaves the name's alone, and a run drawn once per ink would be a name laid down twice.
fn tab_number(
    paint: &mut Painter<'_>,
    x: f32,
    baseline: f32,
    info: &TabInfo,
    number: TextStyle,
) -> f32 {
    // A settings tab has no number to draw and no number's worth of space to skip, so its
    // name starts where the cell's padding left it. This is the only place the two kinds of
    // tab draw differently, and it is one early return rather than a second label-drawing
    // path: what comes back is the x the name starts at either way.
    let TabId::Terminal(index) = info.id else {
        return x;
    };
    let mut buffer = [0u8; 10];
    let digits = index_text(&mut buffer, index);
    let hash = number.scaled(HASH_RATIO);
    let hash_width = paint.width("#", hash);
    paint.text("#", x, baseline, hash);
    paint.text(digits, x + hash_width, baseline, number);
    x + hash_width + paint.width(digits, number) + TAB_GAP
}

/// A tab's name, as much of it as the cell has room for.
///
/// The name is sliced out of the title the caller already owns — by character, so a
/// multi-byte one is never cut in half — and an ellipsis is appended when the slice is
/// short. The cell reserved room for that ellipsis when it counted the characters, so
/// nothing here can overflow the cell.
fn tab_name(
    paint: &mut Painter<'_>,
    x: f32,
    baseline: f32,
    info: &TabInfo,
    fit: usize,
    name: TextStyle,
) {
    if fit == 0 {
        return;
    }
    let title = info.title.as_str();
    let cut = title
        .char_indices()
        .nth(fit)
        .map_or(title.len(), |(at, _)| at);
    let (shown, rest) = title.split_at(cut);
    paint.text(shown, x, baseline, name);
    if !rest.is_empty() {
        let offset = paint.width(shown, name);
        paint.text("…", x + offset, baseline, name);
    }
}

/// Draw the three caption marks.
///
/// Last but the menu, because these are the window's own controls and nothing in zet is
/// allowed to cover them.
pub(crate) fn captions(
    paint: &mut Painter<'_>,
    strip: &Strip,
    input: &ChromeInput<'_>,
    hover: &Hover,
) {
    let palette = *input.palette;
    for (caption, rect) in &strip.captions {
        let lit = hover.of(Hit::Caption(*caption));
        let closing = *caption == Caption::Close;
        // Under the pointer the button fills and the mark inverts against the fill, which is
        // what Windows does with the same three buttons. Close fills `danger` and takes a
        // `ground` mark: dark on the coral measures 7.05:1 and `ink` on it measures 2.15:1,
        // so the mark has to go the other way to stay readable. The other two fill a tenth
        // of `ink`, and their mark steps up to `ink` rather than staying `ink-mid`, because
        // a control with no hover state is a control the user is not sure they are over.
        paint.fill_at(
            *rect,
            if closing { palette.danger } else { palette.ink },
            if closing { lit } else { lit * CAPTION_WASH },
        );
        let mark = mark_of(*caption, input.maximized);
        marks::draw(paint, mark, *rect, palette.ink_mid, 1.0 - lit, input.scale);
        marks::draw(
            paint,
            mark,
            *rect,
            if closing { palette.ground } else { palette.ink },
            lit,
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
        .find(|cell| Some(cell.id) == input.active)?;
    let destination = bar_rect(strip.position, cell.rect);
    let rect = match travel {
        Some(travel) => travel
            .from
            .lerp(destination, progress(now - travel.start, TRAVEL)),
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

/// The indicator's preview under a hovered tab, and how far in it has faded.
///
/// Suppressed while one is travelling, because the bar in flight is the answer to "where
/// am I now" and a second one at rest would be a second answer. The active tab is never the
/// previewed one: its bar is already there, at `signal` rather than `signal-dim`.
///
/// The colour is the caller's and the lit value is this function's, because the bar is drawn
/// by whoever holds the palette and where it goes is a question about the strip.
pub(crate) fn hover_preview(
    strip: &Strip,
    input: &ChromeInput<'_>,
    travelling: bool,
    hover: &Hover,
) -> Option<(Rect, f32)> {
    if travelling {
        return None;
    }
    let cell = strip.tabs.iter().find(|cell| {
        Some(cell.id) != input.active
            && input
                .tabs
                .iter()
                .any(|tab| tab.id == cell.id && tab.hovered)
    })?;
    let lit = hover.of(Hit::Tab(cell.id));
    (lit > 0.0).then(|| (bar_rect(strip.position, cell.rect), lit))
}

/// How far through a transition of `over` seconds this is, from zero to one, after
/// `elapsed` seconds.
///
/// Exponential ease-out, which is the curve DESIGN.md names for the indicator's travel:
/// most of the distance goes in the first third of the time and the rest settles rather
/// than stopping. A hover is the same curve over a different span, so the span is the
/// caller's and the shape is this function's.
pub(crate) fn progress(elapsed: f32, over: f32) -> f32 {
    if elapsed <= 0.0 {
        0.0
    } else if elapsed >= over {
        1.0
    } else {
        1.0 - 2f32.powf(-10.0 * elapsed / over)
    }
}
