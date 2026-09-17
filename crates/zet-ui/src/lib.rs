//! zet's chrome: the titlebar, the tab strip, the caption buttons, the scrollbar, the
//! settings panel, and the find bar.
//!
//! This crate is pure computation on the CPU. It opens no window, touches no device, and
//! never reads a terminal grid. It is handed a description of the window, it fills a
//! [`zet_render::Frame`] with rectangles, and it answers where the user just clicked.
//! That is the whole of it, and the boundary is load-bearing: everything the chrome does
//! can be asserted against a frame and a hit test, with no window and no GPU in the loop.
//!
//! # Two planes, never mixed
//!
//! [DESIGN.md] splits the window into the **chrome plane**, which is zet's, and the
//! **grid plane**, which is the program's. The chrome draws from [`zet_config::Palette`]
//! and nothing else; the grid draws from [`zet_config::Theme`] and nothing else; and the
//! signal colour stops at the grid's boundary. This crate is the strongest form of that
//! rule, because it *cannot* break it: it holds a palette for the length of one call and
//! has no way to name a theme at all. The test that draws a whole frame and checks every
//! colour against the palette is there because that is a property a regression could take
//! away, not a property the types make impossible.
//!
//! # What the chrome is not told
//!
//! Three things it needs are not in [`ChromeInput`], and they are worth naming here
//! because each one is a deliberate limit rather than an oversight:
//!
//! - **Nothing about the grid.** Not the cursor, not the scrollback, not the cell size.
//! - **No clock.** The indicator's travel is measured against a time the caller supplies
//!   through [`Chrome::set_time`], because a transition that reads `Instant::now` cannot
//!   be asserted seventy milliseconds into itself. A caller that never sets one gets a
//!   bar that stays where it was, which is a bug that shows up immediately rather than a
//!   flashing one that does not.
//! - **No scrollback position.** It changes without the configuration changing and
//!   without the window changing size, so it arrives through [`Chrome::set_scroll`]
//!   rather than being carried on every frame with everything else.
//!
//! # Text
//!
//! The chrome's typeface is IBM Plex Sans at two weights, self-hosted in this crate and
//! registered into the font database before anything is resolved — see [`fonts`]. The
//! layout asks for a character and is told where its pixels are; it never rasterises,
//! never owns a texture, and never knows that an atlas exists. The atlas it draws from
//! is the renderer's, shared with the grid: one texture, two faces, told apart by the
//! face in each glyph's key.
//!
//! **Known gap:** DESIGN.md asks for tabular figures in every number the chrome draws,
//! and `zet_font::GlyphSpec` has no way to ask for an OpenType feature. The numbers are
//! set in Plex Sans's default (proportional) figures. Faking it by spacing digits by hand
//! would be a lie about the font's own metrics, so it is left as it is and recorded here.
//!
//! [DESIGN.md]: https://github.com/asterxsk/zet/blob/main/DESIGN.md

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all, clippy::pedantic)]
// Two casts, both of them a count or a glyph dimension turned into a distance in the
// same units as the window it is drawn in: a tab index, a glyph's width, a number of
// rails cells. All of them are smaller than a window and a window is smaller than the
// 2^24 where `f32` starts rounding, so every one of these is exact. The alternative is
// an attribute per line saying "a terminal is not sixteen million pixels wide", which is
// not a fact any reader needs told.
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

pub mod fonts;
mod geometry;
mod marks;
mod overlays;
mod paint;
mod strip;
#[cfg(test)]
mod tests;

use std::mem;

use zet_config::{TabPosition, TabSettings, WindowSettings};
use zet_render::{GlyphQuad, Quad};

pub use crate::fonts::GlyphSource;
pub use crate::geometry::{Rect, Size};

/// The height of the one row that holds the app name, the tabs, the drag region, and the
/// caption buttons.
///
/// Exposed because the caller has to lay a window out around it, and because a window
/// with no tabs is exactly this much shorter.
pub use crate::geometry::ROW_HEIGHT;

/// Everything the chrome needs to draw itself for one frame.
pub struct ChromeInput<'a> {
    /// The chrome palette. The only colours the chrome has.
    pub palette: &'a zet_config::Palette,
    /// The tabs, in the order they are shown.
    pub tabs: &'a [TabInfo],
    /// The number of the active tab, or `None` when nothing is open.
    pub active: Option<u32>,
    /// Whether the settings panel is open.
    pub settings_open: bool,
    /// What the settings panel shows: headings and rows, in order.
    ///
    /// Supplied by the caller rather than decided here, and that is the crate's rule
    /// about the configuration — this crate is not given it, and a second copy of what a
    /// setting *is* living next to the painter is exactly how a panel and a file start
    /// disagreeing. The panel owns the geometry and the chrome's type; the caller owns
    /// the words and the values.
    ///
    /// Ignored unless [`ChromeInput::settings_open`].
    pub settings: &'a [SettingLine<'a>],
    /// How far the settings panel is scrolled, in logical pixels.
    ///
    /// The caller owns it for the same reason it owns the rows: a wheel over the panel
    /// arrives at the window, and the window is not this crate's. The panel clamps it to
    /// what actually overflows and reports the clamped value back through
    /// [`Layout::settings_scroll`].
    pub settings_scroll: f32,
    /// The settings row the keyboard is on, if any.
    ///
    /// An index into [`ChromeInput::settings`], and `None` while the panel is open but
    /// the keyboard has not been asked for. The two are different states and the
    /// difference is the point: a panel that took the arrow keys the moment it opened
    /// would have stolen them from the shell, and the terminal behind it is supposed to
    /// still be usable.
    pub settings_focus: Option<usize>,
    /// What the find bar is showing, or `None` when it is closed.
    ///
    /// Whether the bar is open is this and nothing else, so there is no second flag for a
    /// caller to keep in step with it: a bar that is drawn but does not take the grid's
    /// height, or takes the height and is not drawn, is a bug that only exists because
    /// two values said two things.
    pub find: Option<FindLine<'a>>,
    /// The text in the titlebar's name slot, which the app name goes in.
    pub window_title: &'a str,
    /// The whole window's size in logical pixels.
    pub size: Size,
    /// The DPI scale.
    pub scale: f32,
    /// Whether the window is maximized, which drops the bottom hairline.
    pub maximized: bool,
    /// Whether the OS asks for reduced motion. Read live; it can change mid-session.
    pub reduce_motion: bool,
    /// The pointer's position in logical pixels, for hover states that are not a tab's.
    ///
    /// A tab's own hover is [`TabInfo::hovered`], because the caller has already asked
    /// [`Chrome::hit`] and knows the answer better than a rectangle comparison does.
    pub pointer: Option<(f32, f32)>,
}

/// What the find bar is showing.
///
/// Three things and no more, because those are what the row has room to say: what was
/// typed, which match the arrows are on out of how many, and whether the search stopped
/// counting. Where the matches *are* is not here — the caller has already painted them
/// into the grid, and the bar is a query and a count rather than a second view of the
/// terminal.
pub struct FindLine<'a> {
    /// The query, as typed.
    pub query: &'a str,
    /// Which match the arrows are on, counted from one, and how many there are.
    ///
    /// `None` when nothing has been typed or nothing matched, which are the same thing to
    /// look at and different things to say.
    pub position: Option<(usize, usize)>,
    /// Whether the search stopped at its limit rather than running out.
    pub capped: bool,
}

impl Default for FindLine<'_> {
    /// A bar that has just opened, with nothing typed into it yet.
    fn default() -> Self {
        FindLine {
            query: "",
            position: None,
            capped: false,
        }
    }
}

/// One line of the settings panel: a section heading, or a setting.
///
/// A flat list rather than sections of rows, because the panel draws it as a flat list —
/// headings are a type size and a rule, not a container — and a nested shape would make
/// the caller build two levels of vectors to say what one already did. The caller says
/// which line is which by its position in the slice, and gets that position back in
/// [`Hit::Setting`].
pub struct SettingLine<'a> {
    /// The heading's name, or the row's label.
    pub text: &'a str,
    /// The row's control, or `None` on a section heading.
    pub control: Option<Control>,
    /// What the control shows. Ignored on a heading.
    pub value: &'a str,
}

/// What a settings row's control does when it is clicked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Control {
    /// One of a fixed list of values. A click takes the next one.
    Choice,
    /// Yes or no. A click flips it.
    Toggle,
    /// A number, or anything else with an order. The left half lowers it and the right
    /// half raises it.
    Step,
    /// A key binding. A click starts recording one.
    Chord,
}

/// Which part of a settings row's control a click landed on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingPart {
    /// The whole control: a choice takes its next value, a toggle flips, a chord starts
    /// recording.
    Whole,
    /// The left half of a stepper, which lowers the value.
    Less,
    /// The right half of a stepper, which raises it.
    More,
}

/// One tab, as the strip needs to know it.
pub struct TabInfo {
    /// The tab's number. Creation order, never renumbered on close.
    pub index: u32,
    /// The tab's own title: what the program set with `OSC 0`/`OSC 2`, or the profile's
    /// name when it set nothing.
    ///
    /// The strip draws it after the number, truncated to whatever the cell has room for.
    /// An empty title is not an error and is the one case where a tab is a number and
    /// nothing else, which is what every tab was before names existed.
    pub title: String,
    /// Whether the pointer is over this tab's cell.
    pub hovered: bool,
}

/// What the chrome took for itself, so the caller knows where the grid goes.
///
/// All logical pixels, and all zero for a window with nothing in it. The grid is what is
/// left after the chrome has taken its edges, which is why the chrome answers this rather
/// than the caller working it out: two places computing "where does the grid go" is one
/// place too many.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Layout {
    /// The height of the titlebar and tab-strip row, or 0 when there are no tabs.
    pub top: f32,
    /// The width of the vertical tab rail, or 0 in horizontal mode.
    pub left: f32,
    /// The height of the find bar, or 0 when it is closed.
    pub bottom: f32,
    /// The rect the terminal grid occupies, in logical pixels.
    pub grid: Rect,
    /// How far the settings panel is actually scrolled, after clamping.
    ///
    /// Reported back because the panel is the only thing that knows how tall its content
    /// is: a caller that kept its own scroll would let the user scroll past the end of a
    /// list it cannot measure, and the wheel would then do nothing for a while before
    /// the content caught up.
    pub settings_scroll: f32,
}

/// What the user hit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    /// A tab, by its number.
    Tab(u32),
    /// The new-tab mark.
    NewTab,
    /// A tab's close affordance.
    ///
    /// Never returned by this version. DESIGN.md's strip is a number, a bar, and nothing
    /// else, and its hover rule is exhaustive — the index moves to `ink-mid` and the bar
    /// previews, and nothing else happens — so a close mark on a tab would be an
    /// invention this crate is not entitled to make. Closing is a keyboard action
    /// (`close-tab`, which the default keymap binds) and, by convention, a middle click
    /// on [`Hit::Tab`]. The variant stays because a caller matching on `Hit` should not
    /// have to be rewritten when that convention becomes an affordance.
    CloseTab(u32),
    /// A caption button.
    Caption(Caption),
    /// The draggable region between the last tab and the caption buttons.
    Drag,
    /// The settings panel is open and the point is inside it, but not on a control.
    Settings,
    /// A settings row's control, by the line it is on and the part of it that was hit.
    Setting {
        /// Which line of [`ChromeInput::settings`] it is on.
        line: usize,
        /// Which part of that line's control.
        part: SettingPart,
    },
    /// The scrollbar.
    Scrollbar(Scrollbar),
    /// Nothing the chrome owns.
    None,
}

/// A caption button.
///
/// The button, not the mark on it: [`Caption::Maximize`] is one control whose shape
/// changes with the window's state, so which of the two squares it draws is the layout's
/// decision and not the caller's. See [`crate::marks`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Caption {
    /// Minimize.
    Minimize,
    /// Maximize, or restore a window that is already maximized.
    Maximize,
    /// Close.
    Close,
}

/// What a thumb dragged to `top` says the scrollback's position is.
///
/// The inverse of the placement the scrollbar is drawn by, and it lives next to the
/// chrome that draws the thumb rather than next to the caller that drags it, because the
/// two are one fact written twice and only this side can be tested against the real
/// thing: a caller in another crate would have to restate the layout to check itself.
///
/// What the pointer is mapped onto is the distance the thumb can *travel* rather than the
/// track's height. The thumb is a proportion of the track and shrinks as the scrollback
/// grows, so the two only agree at the top: with a track-height divisor the thumb's top
/// would have to reach the bottom of the track to mean "the end", and it stops a thumb's
/// height short of it — the further back the history, the smaller the thumb and the wider
/// the band near the end that no drag can cross.
///
/// `None` when the thumb cannot move: a thumb as tall as its track is a scrollback with
/// nothing scrolled off, and there is no division to do.
#[must_use]
pub fn thumb_offset(track: Rect, thumb: Rect, top: f32) -> Option<f32> {
    let travel = track.height - thumb.height;
    if travel <= 0.0 {
        return None;
    }
    Some(((top - track.y) / travel).clamp(0.0, 1.0))
}

/// Where on the scrollbar the point fell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scrollbar {
    /// On the thumb.
    Thumb,
    /// In the band, above the thumb.
    Above,
    /// In the band, below the thumb.
    Below,
}

/// Where the scrollback is, for the scrollbar.
///
/// The chrome is not given the terminal, so the two numbers the scrollbar is made of are
/// handed over separately. Both are fractions rather than counts, because a scrollbar is
/// a picture of a proportion and the counts behind it are the grid plane's business.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ScrollState {
    /// How far down the scrollback the viewport sits, from 0 at the top to 1 at the
    /// bottom.
    pub offset: f32,
    /// The fraction of the scrollback the viewport shows.
    ///
    /// One or more means there is nothing to scroll, and a terminal spends most of its
    /// life there: the default draws no scrollbar at all.
    pub visible: f32,
}

impl Default for ScrollState {
    fn default() -> Self {
        Self {
            offset: 0.0,
            visible: 1.0,
        }
    }
}

/// What the chrome remembered from one frame to the next.
enum Region {
    Tab {
        index: u32,
        rect: Rect,
    },
    NewTab(Rect),
    Caption {
        caption: Caption,
        rect: Rect,
    },
    Drag(Rect),
    Settings(Rect),
    Setting {
        line: usize,
        part: SettingPart,
        rect: Rect,
    },
    Scrollbar {
        track: Rect,
        thumb: Rect,
    },
}

impl Region {
    /// Whether the point is in this region, and what it is if so.
    fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        match self {
            Self::Tab { index, rect } if rect.contains(x, y) => Some(Hit::Tab(*index)),
            Self::NewTab(rect) if rect.contains(x, y) => Some(Hit::NewTab),
            Self::Caption { caption, rect } if rect.contains(x, y) => Some(Hit::Caption(*caption)),
            Self::Drag(rect) if rect.contains(x, y) => Some(Hit::Drag),
            Self::Settings(rect) if rect.contains(x, y) => Some(Hit::Settings),
            Self::Setting { line, part, rect } if rect.contains(x, y) => Some(Hit::Setting {
                line: *line,
                part: *part,
            }),
            Self::Scrollbar { track, thumb } if track.contains(x, y) => {
                Some(Hit::Scrollbar(if thumb.contains(x, y) {
                    Scrollbar::Thumb
                } else if y < thumb.y {
                    Scrollbar::Above
                } else {
                    Scrollbar::Below
                }))
            }
            _ => None,
        }
    }
}

/// The window's chrome, and what it remembers between frames.
///
/// Cheap to build and cheap to keep: the only state is the previous frame's shape, which
/// is what lets the indicator know where it is coming from. Rebuild it when the
/// configuration changes and not otherwise.
pub struct Chrome {
    position: TabPosition,
    /// What the caller last said the time was. The chrome never reads a clock.
    now: f32,
    /// The active tab as of the last layout, which is how a change is noticed.
    active: Option<u32>,
    /// The indicator's travel, while one is in flight.
    travel: Option<strip::Travel>,
    /// Where the indicator was drawn last frame.
    ///
    /// Kept so that a change mid-travel continues from where the bar *is* rather than
    /// from the tab it was heading for. That is the difference between holding down
    /// `Ctrl+Tab` and watching one bar move, and holding it down and watching a bar
    /// snap backwards on every step.
    indicator: Option<Rect>,
    scroll: ScrollState,
    /// The scrollbar's track and thumb as of the last layout, for the one caller that
    /// has to turn a drag into a position.
    ///
    /// `Hit` answers *which* control a point landed on and deliberately not where inside
    /// it, because every other control in the window is a rectangle you either hit or do
    /// not. A scrollbar is the one whose answer depends on how far down it the pointer is,
    /// so it is the one that has to publish its geometry as well as its hit region.
    scrollbar: Option<(Rect, Rect)>,
    layout: Layout,
    regions: Vec<Region>,
    /// Two arrays the layout fills and the frame is then given, so that every rectangle
    /// goes into one batch and every glyph into the next. Retained across frames for the
    /// same reason a frame is: this runs sixty times a second.
    quads: Vec<Quad>,
    glyphs: Vec<GlyphQuad>,
}

impl Chrome {
    /// Build the chrome for one configuration. Cheap; rebuild it when the config changes.
    #[must_use]
    pub fn new(tabs: &TabSettings, window: &WindowSettings) -> Self {
        // Nothing in `WindowSettings` reaches the chrome. The background and the opacity
        // are the grid plane's, remembering where the window was is the app's, and
        // whether the window *is* maximized arrives per frame in `ChromeInput` because it
        // changes without the configuration changing. The argument is here because the
        // two are written down together and a caller should not have to remember which of
        // them the chrome wanted.
        let _ = window;
        Self {
            position: tabs.position,
            now: 0.0,
            active: None,
            travel: None,
            indicator: None,
            scroll: ScrollState::default(),
            scrollbar: None,
            layout: Layout::default(),
            regions: Vec::new(),
            quads: Vec::new(),
            glyphs: Vec::new(),
        }
    }

    /// Tell the chrome what time it is, in seconds.
    ///
    /// Called before [`Chrome::layout`], every frame. The chrome reads no clock of its
    /// own, which is what makes the indicator's travel a function of its inputs and
    /// therefore a thing a test can stand in the middle of. Any monotonically increasing
    /// clock will do — only differences are ever taken — and seconds is the unit
    /// `Instant` hands out.
    pub fn set_time(&mut self, now: f32) {
        self.now = now;
    }

    /// Where the scrollback is, for the scrollbar.
    pub fn set_scroll(&mut self, scroll: ScrollState) {
        self.scroll = scroll;
    }

    /// Draw the chrome for this frame into `frame`.
    ///
    /// The frame is appended to rather than replaced, so a caller draws the grid and then
    /// the chrome over it. `frame.clear` is left alone for the same reason: the ground
    /// behind everything is the window's, and the chrome has no opinion about it.
    pub fn layout(
        &mut self,
        input: &ChromeInput<'_>,
        fonts: &mut dyn GlyphSource,
        frame: &mut zet_render::Frame,
    ) -> Layout {
        // The two arrays are moved out for the duration rather than borrowed, because the
        // planner needs `&mut self` and the painter needs `&mut` of both. Taking them also
        // means they are put back with their capacity intact, which is the point of
        // keeping them at all.
        let mut quads = mem::take(&mut self.quads);
        let mut glyphs = mem::take(&mut self.glyphs);
        quads.clear();
        glyphs.clear();
        self.regions.clear();

        let layout = {
            let mut paint = paint::Painter::new(fonts, &mut quads, &mut glyphs, input.scale);
            self.plan(input, &mut paint)
        };

        // Every rectangle in one batch and every glyph in the next. The frame draws its
        // batches in the order they were opened, so this is also the layering rule: the
        // chrome's text is always over the chrome's surfaces, without the layout having to
        // interleave anything.
        if !quads.is_empty() {
            frame.begin_quads();
            for quad in &quads {
                frame.push_quad(*quad);
            }
            frame.end_quads();
        }
        if !glyphs.is_empty() {
            frame.begin_glyphs();
            for glyph in &glyphs {
                frame.push_glyph(*glyph);
            }
            frame.end_glyphs();
        }

        self.quads = quads;
        self.glyphs = glyphs;
        layout
    }

    /// What is at this point, in logical pixels relative to the window's top-left.
    ///
    /// Answers from the layout of the most recent [`Chrome::layout`] call. Regions are
    /// tested in the order the later ones would be drawn over the earlier ones, so a
    /// caption button wins over the panel it floats above and a tab wins over the
    /// settings panel it is never underneath.
    #[must_use]
    pub fn hit(&self, x: f32, y: f32) -> Hit {
        self.regions
            .iter()
            .find_map(|region| region.hit(x, y))
            .unwrap_or(Hit::None)
    }

    /// The layout of the most recent [`Chrome::layout`] call.
    #[must_use]
    pub const fn last_layout(&self) -> &Layout {
        &self.layout
    }

    /// The scrollbar's track and thumb as of the most recent [`Chrome::layout`] call.
    ///
    /// `None` when there is nothing to scroll and no bar was drawn. The pair is what a
    /// drag needs: a thumb dragged to a place on its track is a position, and a position
    /// is what the track's rectangle is for.
    #[must_use]
    pub const fn scrollbar(&self) -> Option<(Rect, Rect)> {
        self.scrollbar
    }

    /// Everything the last layout made hittable.
    ///
    /// Kept crate-private rather than exposed, because a caller that wants to know where
    /// a tab is has [`Chrome::hit`], and a second way to ask is a second thing to keep
    /// true. The crate's own tests use it, which is the one caller that needs the
    /// question answered from the other direction.
    #[cfg(test)]
    pub(crate) fn regions(&self) -> &[Region] {
        &self.regions
    }

    /// Draw one frame, and say what the chrome took.
    fn plan(&mut self, input: &ChromeInput<'_>, paint: &mut paint::Painter<'_>) -> Layout {
        let size = input.size;
        let strip_plan = strip::plan(paint, input, self.position);
        let top = if strip_plan.row { ROW_HEIGHT } else { 0.0 };
        let left = if strip_plan.row && self.position == TabPosition::Left {
            crate::geometry::RAIL_WIDTH
        } else {
            0.0
        };
        let bottom = if input.find.is_some() {
            overlays::find_bar_height()
        } else {
            0.0
        };

        // The indicator's motion is settled before anything is drawn, because the
        // indicator is what reads it.
        self.retarget(input, &strip_plan);
        let fade = self
            .travel
            .as_ref()
            .map(|travel| (travel.from_index, strip::progress(self.now - travel.start)));

        strip::draw(paint, &strip_plan, input, fade);
        if let Some((rect, color)) =
            strip::indicator(&strip_plan, input, self.travel.as_ref(), self.now)
        {
            paint.fill(rect, color);
            self.indicator = Some(rect);
        } else {
            self.indicator = None;
        }
        if let Some(rect) = strip::hover_preview(&strip_plan, input, self.travel.is_some()) {
            paint.fill(rect, input.palette.signal_dim);
        }

        let panel = input
            .settings_open
            .then(|| overlays::panel(paint, input, top, bottom));
        let settings_scroll = panel.as_ref().map_or(0.0, |panel| panel.scroll);
        if bottom > 0.0 {
            overlays::find_bar(paint, input, bottom);
        }
        let scroll = overlays::scrollbar(paint, input, top, bottom, self.scroll);

        // Last, because these are the window's own controls and nothing in zet may cover
        // them: with no tabs there is no row, and they are the only thing left on the
        // window that can be clicked.
        strip::captions(paint, &strip_plan, input);

        for (caption, rect) in &strip_plan.captions {
            self.regions.push(Region::Caption {
                caption: *caption,
                rect: *rect,
            });
        }
        for cell in &strip_plan.tabs {
            self.regions.push(Region::Tab {
                index: cell.index,
                rect: cell.rect,
            });
        }
        if let Some(rect) = strip_plan.plus {
            self.regions.push(Region::NewTab(rect));
        }
        if let Some(rect) = strip_plan.drag {
            self.regions.push(Region::Drag(rect));
        }
        // The controls before the panel that contains them, because `Chrome::hit` answers
        // with the first region that holds the point: pushed the other way round, every
        // control would be shadowed by the surface it sits on and no setting would ever
        // be clicked.
        if let Some(panel) = &panel {
            for (line, part, rect) in &panel.controls {
                self.regions.push(Region::Setting {
                    line: *line,
                    part: *part,
                    rect: *rect,
                });
            }
            self.regions.push(Region::Settings(panel.rect));
        }
        self.scrollbar = scroll;
        if let Some((track, thumb)) = scroll {
            self.regions.push(Region::Scrollbar { track, thumb });
        }

        let grid = Rect::between(left, top, size.width, size.height - bottom);
        self.layout = Layout {
            top,
            left,
            bottom,
            grid,
            settings_scroll,
        };
        self.layout
    }

    /// Notice a change of active tab, and start the indicator moving if it is one.
    fn retarget(&mut self, input: &ChromeInput<'_>, strip_plan: &strip::Strip) {
        // A travel that has arrived, or that belongs to the other position of the strip,
        // is over.
        if self.travel.as_ref().is_some_and(|travel| {
            travel.position != self.position || self.now - travel.start >= crate::geometry::TRAVEL
        }) {
            self.travel = None;
        }

        if input.active == self.active {
            return;
        }
        let previous = self.active;
        self.active = input.active;

        // A bar travelling from nowhere is a bar appearing, and one that cannot arrive is
        // a bar that would have to leave the screen to get there. Neither is the motion
        // DESIGN.md asks for, so neither starts a travel.
        let Some(from) = self.indicator else {
            self.travel = None;
            return;
        };
        let Some(previous) = previous else {
            self.travel = None;
            return;
        };
        let arrives = strip_plan
            .tabs
            .iter()
            .any(|cell| Some(cell.index) == input.active);
        let leaves = strip_plan.tabs.iter().any(|cell| cell.index == previous);
        if input.reduce_motion || !arrives || !leaves {
            self.travel = None;
            return;
        }

        self.travel = Some(strip::Travel {
            position: self.position,
            start: self.now,
            from,
            from_index: previous,
        });
    }
}
