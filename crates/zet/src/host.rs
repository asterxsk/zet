//! The window, the event loop, and everything the app is not allowed to know about.
//!
//! [`App`] is the terminal as a state machine: it decides what a keystroke means, which
//! tab is active, and what a frame should contain, all without a window. This file is
//! the other half — the part that owns an `HWND`, a GPU device, and a clipboard, and
//! translates between the platform's events and the vocabulary [`App`] speaks. The split
//! is what lets the interesting half be tested.
//!
//! # The one rule
//!
//! Nothing here makes a decision the app should have made. Where a keystroke goes, what
//! a click selects, whether closing the last tab closes the window — all of that is
//! [`App`]'s, and this file asks rather than guesses. What lives here is exactly the set
//! of things a state machine cannot do: create a surface, read a clock, talk to Win32,
//! and exit the loop.
//!
//! # The frame, and why the layout lags by one
//!
//! The chrome draws itself into the same frame as the grid, and it is what decides where
//! the grid goes — the strip takes a row off the top, the rail takes a column off the
//! left, and that answer only exists once the chrome has been laid out. The frame draws
//! its batches in order and the grid has to be underneath, so the grid is positioned
//! from the *previous* frame's layout, and the fresh layout is compared against it
//! afterwards. When they differ — a tab opened, the window resized — another redraw is
//! requested at once.
//!
//! That is one frame of lag on a layout change and nothing at all in the steady state.
//! The alternative is worse: re-running the chrome's layout to measure it first would
//! advance the tab indicator's travel twice per frame, turning DESIGN.md's 140 ms slide
//! into a snap — trading a visible animation for a sixteenth of a second nobody sees.

// Every cast in this file is a pixel, a cell, or a scale factor crossing between the
// logical units the window reports and the physical ones the renderer draws in. A window
// is not sixteen million pixels wide and a terminal is not that many cells, so each of
// these is exact; the alternative is an attribute per line saying so.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Ime, MouseButton as WinitButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::ModifiersState;
use winit::window::{CursorIcon, Window, WindowId};

use zet_app::{Action, App, AppError, Command};
use zet_config::{Config, FontSettings, Palette, TabSettings};
use zet_font::{FontError, FontStack};
use zet_input::{Chord, Key, KeyEvent, KeyKind, Modifiers, MouseEvent, encode_mouse};
use zet_render::{Frame, Renderer, RendererError, View};
use zet_ui::{Caption, Chrome, ChromeInput, Hit, Layout, ScrollState, Size, TabInfo};

use crate::keys;
use crate::mouse;
use crate::platform::SystemSettings;
use crate::waker::Wake;

/// The app's own name, as the titlebar's name slot shows it.
const APP_NAME: &str = "zet";

/// The grid size a window with no renderer yet is measured against.
///
/// A window with no terminal in it is still a window, and its size is what a tab opened
/// from the keyboard will be created at. Twenty-four by eighty is the size a terminal
/// has defaulted to since before either of us was born.
const EMPTY_GRID: (u16, u16) = (80, 24);

/// The window's smallest allowed size in logical pixels.
///
/// Small enough to be out of the way, large enough that the caption buttons and one tab
/// still fit. Below that the window is a rectangle of chrome with nothing in it, which
/// is a worse answer than refusing to shrink further.
const MIN_SIZE: LogicalSize<f64> = LogicalSize::new(360.0, 240.0);

/// The size a window opens at, in logical pixels.
const OPEN_SIZE: LogicalSize<f64> = LogicalSize::new(1100.0, 720.0);

/// Lines a page of scrolling covers, for the scrollbar's bands.
const SCROLL_PAGE: i32 = 20;

/// What a binding row says while it is waiting for the user to press something.
const PRESS_A_KEY: &str = "Press a key";

/// How far one wheel notch moves the settings panel, in logical pixels.
///
/// The panel answers in pixels and is the only thing that knows how tall its own rows
/// are, so the host scrolls in pixels too. A little under two rows, so that a notch
/// always visibly moves the list and never skips past a row the user was aiming at.
const PANEL_WHEEL: f32 = 48.0;

/// The find bar, gathered up for one frame.
struct FindView<'a> {
    /// The matches on screen, in the window's coordinates, oldest first.
    marks: Vec<zet_render::Selection>,
    /// Which of them the find bar's arrows are on.
    active: Option<usize>,
    /// What the row says, or `None` while the bar is closed.
    line: Option<zet_ui::FindLine<'a>>,
}

/// Everything a frame needs from the find bar.
///
/// A free function over the app rather than a method on the host, because the row it
/// returns borrows the query: as a method it would borrow the whole host for as long as
/// the frame took to draw, and the frame wants the chrome, the renderer, and the frame
/// itself mutably while it does. Taking only the app leaves the rest of the host free.
///
/// The marks are in the window's coordinates rather than the history's — a match is a
/// position in a list that is growing underneath it, and only the session knows where
/// the view is scrolled to — so they are owned rather than borrowed.
fn find_view(app: &App) -> FindView<'_> {
    let (marks, active) = app.find_marks();
    let find = app.find();
    FindView {
        marks,
        active,
        line: find.is_open().then(|| zet_ui::FindLine {
            query: find.query(),
            position: find.tally(),
            capped: find.capped(),
        }),
    }
}

/// The terminal, attached to a window.
pub struct Host {
    /// The state machine. Everything the host needs to know is behind this.
    app: App,

    /// The accessibility settings, as of the last time they were read.
    settings: SystemSettings,
    /// The chrome's colours, derived from the config and the forced-colours highlight.
    palette: Palette,

    /// The window, once `resumed` has made one.
    window: Option<Arc<Window>>,
    /// The device that draws into it.
    renderer: Option<Renderer>,
    /// The chrome, and what it remembers between frames.
    chrome: Chrome,
    /// The frame, kept for its capacity. Rebuilt every time one is drawn.
    frame: Frame,

    /// The modifier state, which `winit` reports separately from the keys it applies to.
    held: ModifiersState,
    /// Where the pointer last was, in logical pixels.
    pointer: Option<(f64, f64)>,
    /// The cursor currently shown, so it is only set when it changes.
    cursor: CursorIcon,
    /// The cell a left button went down on, so a click with no drag can be told from one
    /// with a drag.
    pressed_at: Option<zet_vt::Pos>,
    /// Whether the window has focus, which decides whether the cursor is hollow.
    focused: bool,
    /// Where on the thumb a drag was grabbed, in logical pixels from the thumb's top.
    ///
    /// Kept so that the thumb does not jump under the pointer on the first move: the
    /// grab point is the place the user took hold of, and it stays under the pointer for
    /// the whole drag, which is what every scrollbar does.
    scroll_grab: Option<f32>,
    /// Scrolling sub-line remainders, accumulated so a trackpad's fractions are not
    /// dropped one event at a time.
    partial: f64,

    /// When the cursor's blink phase last changed, so the loop can wake for the next one.
    blink_at: Instant,
    /// The moment the host started, for the chrome's clock.
    origin: Instant,

    /// What the OS was last told this window is called, so it is only told when it
    /// changes. Every frame would be a `WM_SETTEXT` per frame.
    os_title: String,

    /// The grid face's settings as of the last load, so a change is noticed.
    styled: FontSettings,
    /// The chrome's settings as of the last build, so a change is noticed.
    tabbed: TabSettings,
    /// The layout the previous frame's grid was positioned with.
    placed: Layout,
    /// The grid size the sessions were last told about, so that a frame which did not
    /// change it does not resize every one of them again.
    fitted: (u16, u16),

    /// Whether the settings panel is open.
    ///
    /// The window's state rather than the app's, because an overlay is something the
    /// window draws and the app is the thing that draws nothing. The app already says as
    /// much where it declines to act on the binding.
    settings_open: bool,
    /// How far the panel is scrolled, in logical pixels from the top of its list.
    ///
    /// The number the caller asks for; the panel answers each frame with the number it
    /// actually used, clamped to what overflows, and that answer is what is kept. A
    /// window that grows therefore pulls the list back up on its own instead of leaving
    /// it scrolled past the end of a list that now fits.
    settings_scroll: f32,
    /// The action waiting for the user to press a key, if the panel asked for one.
    capturing: Option<Action>,
    /// The settings row the keyboard is on: an index into the rows the app answers with.
    ///
    /// `None` while the panel is open but the keyboard has not been asked for, which is
    /// a different state from "no row is focused" in the same way that a window with no
    /// focus is different from a window whose focus is nowhere in particular. The arrow
    /// keys belong to the shell until this is set.
    settings_focus: Option<usize>,

    /// Whether the keyboard has been walked off the end of the panel's rows.
    ///
    /// `settings_focus` alone cannot say which of the two `None`s it is, and the two
    /// want opposite things from the next `Tab`: a panel that has just opened takes it
    /// as a request to enter at the top row, and a panel the user has already tabbed
    /// out of has to let it reach the shell or the panel is a place you can only leave
    /// by remembering the chord that opened it. Cleared whenever focus is taken again,
    /// by the chord or by a click.
    settings_left: bool,
}

impl Host {
    /// Build a host around a loaded app.
    ///
    /// Reads the system's accessibility settings here rather than on the first frame,
    /// because the first frame should already be the right one: a high-contrast user
    /// seeing a normal-contrast window for a frame and then a white one is a flash
    /// bright enough to be worth avoiding.
    ///
    /// The app is told straight away, and not only when a later read notices a change.
    /// [`Host::reread_system`] returns early when the settings have not moved, which they
    /// have not — so a first read that only stored them in the host would leave the app
    /// holding the defaults for the whole session: a cursor blinking against a system
    /// that asked for no motion, and normal colours on a machine with high contrast on.
    #[must_use]
    pub fn new(mut app: App) -> Self {
        let settings = SystemSettings::read();
        app.system_accessibility(settings.reduce_motion, settings.high_contrast);
        let palette = zet_config::palette_for(app.config(), settings.highlight);
        let chrome = Chrome::new(&app.config().tabs, &app.config().window);
        let tabbed = app.config().tabs.clone();
        let styled = grid_settings(&app, text_scale(app.config(), settings.text_scale));
        let now = Instant::now();
        Self {
            app,
            settings,
            palette,
            window: None,
            renderer: None,
            chrome,
            frame: Frame::new(),
            held: ModifiersState::empty(),
            pointer: None,
            cursor: CursorIcon::Default,
            pressed_at: None,
            focused: true,
            scroll_grab: None,
            partial: 0.0,
            blink_at: now,
            origin: now,
            os_title: APP_NAME.to_owned(),
            styled,
            tabbed,
            placed: Layout::default(),
            // What `resumed` opens the first tab at, before any frame has been laid out
            // and therefore before anything knows how big the window really is.
            fitted: EMPTY_GRID,
            settings_open: false,
            settings_scroll: 0.0,
            capturing: None,
            settings_focus: None,
            settings_left: false,
        }
    }

    /// The window, if one has been made yet.
    fn window(&self) -> Option<&Arc<Window>> {
        self.window.as_ref()
    }

    /// The current window scale, or one before there is a window.
    fn scale(&self) -> f32 {
        self.window().map_or(1.0, |w| w.scale_factor() as f32)
    }

    /// The text scale the chrome and the grid are drawn at.
    fn text_scale(&self) -> f32 {
        text_scale(self.app.config(), self.settings.text_scale)
    }

    /// The window's size in logical pixels.
    fn logical_size(&self) -> (f64, f64) {
        let Some(window) = self.window() else {
            return (OPEN_SIZE.width, OPEN_SIZE.height);
        };
        let size = window.inner_size();
        let scale = window.scale_factor();
        (
            f64::from(size.width) / scale,
            f64::from(size.height) / scale,
        )
    }

    /// Create the window and the device that draws into it.
    fn attach(&mut self, loop_: &ActiveEventLoop) -> Result<(), StartupError> {
        let window = Arc::new(
            loop_
                .create_window(
                    Window::default_attributes()
                        .with_title(APP_NAME)
                        // DESIGN.md draws its own titlebar and caption buttons, and a
                        // second set drawn by Windows over them would be two titlebars
                        // stacked on each other.
                        .with_decorations(false)
                        .with_inner_size(OPEN_SIZE)
                        .with_min_inner_size(MIN_SIZE),
                )
                .map_err(StartupError::Window)?,
        );

        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        let grid = grid_settings(&self.app, self.text_scale());
        let renderer = Renderer::new(
            Arc::clone(&window),
            size.width,
            size.height,
            scale,
            &grid,
            self.chrome_face(scale)
                .map_err(|error| StartupError::Render(RendererError::from(error)))?,
        )?;

        self.styled = grid;
        self.renderer = Some(renderer);
        self.window = Some(window);
        Ok(())
    }

    /// The chrome's face, at this DPI and this text scale.
    ///
    /// Built here rather than through `zet_ui::fonts::stack` because the system's text
    /// scale has to be applied to the point size, and a user who asked Windows for 200%
    /// text is asking about the titlebar as much as about anything else.
    fn chrome_face(&self, scale: f32) -> Result<FontStack, FontError> {
        let mut settings = zet_ui::fonts::settings();
        settings.size *= self.text_scale();
        FontStack::load_embedded(
            &settings,
            scale,
            &[zet_ui::fonts::REGULAR, zet_ui::fonts::MEDIUM],
        )
    }

    /// Bring anything that can change without the app noticing up to date.
    ///
    /// Three things the app changes that the host owns: the terminal font's size, which
    /// the keyboard nudges; the tab strip's shape, which the chrome is built from; and
    /// the theme, which the palette is derived from. Comparing against what was last
    /// used is cheaper than a change notification and cannot drift out of sync with what
    /// was actually applied.
    fn reconcile(&mut self) {
        let wanted = grid_settings(&self.app, self.text_scale());
        if wanted != self.styled {
            // The scale comes from the live window rather than from a remembered copy,
            // so a frame drawn while the window is moving between monitors cannot
            // rasterise a face against the display it just left.
            let scale = self.scale();
            if let Some(renderer) = self.renderer.as_mut() {
                // A family the user typed wrong leaves the previous face in place, which
                // is the point of `restyle` returning a result rather than replacing: a
                // working terminal in the old font beats an empty window.
                let _ = renderer.restyle(&wanted, scale);
            }
            self.styled = wanted;
        }

        if self.app.config().tabs != self.tabbed {
            self.chrome = Chrome::new(&self.app.config().tabs, &self.app.config().window);
            self.tabbed = self.app.config().tabs.clone();
        }

        self.palette = zet_config::palette_for(self.app.config(), self.settings.highlight);
    }

    /// Re-read the system's accessibility settings and apply them.
    ///
    /// The app is told, because the grid's theme and the cursor's blink are its
    /// business; the chrome's palette is derived here because the chrome is this file's
    /// business. The app applies the configuration's own switches on top of what the
    /// system said, so a user who has turned one of them off keeps it off. The text scale
    /// is part of the font size, so a change to it invalidates the loaded faces — which
    /// is expressed by making `reconcile` see a different size.
    fn reread_system(&mut self) {
        let settings = SystemSettings::read();
        if settings == self.settings {
            return;
        }
        self.settings = settings;
        self.app
            .system_accessibility(settings.reduce_motion, settings.high_contrast);
        self.palette = zet_config::palette_for(self.app.config(), settings.highlight);
        // Nothing to invalidate by hand: `reconcile` derives the wanted size from the
        // text scale every time it runs, so the change is picked up on the next frame
        // without a flag to keep true.
    }

    /// Draw everything and put it on screen.
    fn redraw(&mut self) {
        // Before the renderer is borrowed: `reconcile` may reload a face, and it needs
        // the whole host to do it.
        self.reconcile();
        // The search is re-run here rather than when the query changes, because the
        // terminal's contents move the matches too: a line scrolled off the top is a
        // match two rows further up than it was. It is a no-op unless one of the two
        // actually moved, which is what keeps a terminal streaming output from walking
        // ten thousand rows sixty times a second to rediscover the same answer.
        self.app.refresh_find();

        let Some(window) = self.window.clone() else {
            return;
        };
        if self.renderer.is_none() {
            return;
        }

        // Everything that reads the whole host is gathered before the renderer is
        // borrowed mutably, and that ordering is load-bearing rather than stylistic: a
        // method call on `self` borrows all of it, and the renderer's borrow is held
        // across the rest of the frame because drawing a frame is what places the glyphs
        // the frame names.
        let scale = window.scale_factor() as f32;
        let (width, height) = self.logical_size();
        let now = Instant::now();
        let tabs = self.tabs();
        let scroll = self.scroll_state();
        let blink_on = self.app.blink_on(now);
        let selection = self.app.selection();
        let active = self.app.active_number();
        let theme = self.app.theme();
        let cursor_settings = self.app.config().cursor.clone();
        let elapsed = now.duration_since(self.origin).as_secs_f32();
        let finding = find_view(&self.app);
        let marks = zet_render::Marks::new(&finding.marks, finding.active);

        // The taskbar, Alt-Tab, and the window list are the places a user reads a title
        // without looking at the window, and six of them reading "zet" say nothing about
        // which is which. The active tab's name goes there; the drawn strip keeps the
        // app's own name, because the tab names are already on it.
        let title = self.os_window_title();
        if title != self.os_title {
            window.set_title(&title);
            self.os_title = title;
        }

        // The rows and the strings they borrow, both alive until the frame is done with
        // them. `ChromeInput` is a view rather than an owner, so something has to outlive
        // it, and this pair is that something.
        let lines = self.settings_open.then(|| self.app.settings());
        let panel = lines
            .as_ref()
            .map_or_else(Vec::new, |lines| panel_lines(lines, self.capturing));
        // Focus is an index into a list the app rebuilds from the file every frame, so a
        // change made from the panel can move the row out from under it: choosing a
        // block cursor drops the thickness row, and a highlight on the line below it
        // would silently be a highlight on a different setting. Re-clamped here rather
        // than remembered, because this is the only place that knows what the rows are.
        let focused = lines.as_ref().and_then(|lines| {
            self.settings_focus
                .filter(|line| lines.get(*line).is_some_and(|line| line.kind().is_some()))
        });

        let input = ChromeInput {
            palette: &self.palette,
            tabs: &tabs,
            active,
            settings_open: self.settings_open,
            settings: &panel,
            settings_scroll: self.settings_scroll,
            settings_focus: focused,
            find: finding.line,
            window_title: APP_NAME,
            size: Size {
                width: width as f32,
                height: height as f32,
            },
            scale,
            maximized: window.is_maximized(),
            reduce_motion: self.app.reduce_motion(),
            pointer: self.pointer.map(|(x, y)| (x as f32, y as f32)),
        };

        let frame = &mut self.frame;
        let placed = self.placed.grid;
        let chrome = &mut self.chrome;

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        // Copied rather than borrowed, because the renderer is needed mutably below.
        let metrics = *renderer.metrics();

        frame.reset();
        frame.clear = theme.background.to_linear();

        // The grid first, so the chrome lands on top of it. It is positioned with the
        // previous frame's layout — see the module documentation — and the fresh layout
        // is compared against it below.
        if let Some(session) = self.app.active() {
            let view = View {
                origin: (placed.x * scale, placed.y * scale),
                focused: self.focused,
                blink_on,
                selection,
                // Where the viewport is. The session owns the number, because it is the
                // session that knows how much history there is to clamp it against; the
                // renderer only needs to be told, and until it was, every scroll the
                // wheel, the keys and the scrollbar made moved nothing on screen.
                scroll_offset: session.scroll_offset(),
            };
            zet_render::draw_grid(
                session.term(),
                theme,
                &metrics,
                &cursor_settings,
                &view,
                marks,
                renderer,
                frame,
            );
        }

        chrome.set_time(elapsed);
        chrome.set_scroll(scroll);
        let fresh = {
            let mut face = ChromeFace(renderer.chrome());
            chrome.layout(&input, &mut face, frame)
        };

        // The grid was drawn where the layout said last frame. If the layout has moved,
        // one more frame puts it right — and because the comparison is against the
        // fresh layout, the second frame agrees with itself and the loop stops there.
        let moved = fresh.grid != placed;
        // The panel answers with the scroll it actually used rather than the one it was
        // asked for, and that answer is kept: a window that grows then pulls a scrolled
        // list back up on its own, instead of leaving it parked past the end of a list
        // that now fits.
        self.settings_scroll = fresh.settings_scroll;
        self.placed = fresh;
        if moved {
            window.request_redraw();
        }

        // Notify before presenting, so the platform can throttle a frame it is not ready
        // for rather than blocking inside the present.
        window.pre_present_notify();
        // A present that fails because the surface was lost is neither fatal nor worth a
        // message: the next frame reconfigures it, and a window being dragged between
        // monitors fails here routinely.
        let _ = renderer.present(frame);

        // Now that the layout exists, and not before it: see the note on `fit`. A session
        // told it has fewer columns than it has is a program drawing in the wrong place,
        // so this asks rather than assumes, and the answer is where the chrome put the
        // grid.
        self.fit();
    }

    /// What the OS should call this window.
    ///
    /// The active tab's name and then the app's, because a taskbar button is too narrow
    /// for the whole of most titles and the half that survives truncation should be the
    /// half that differs between two zet windows.
    ///
    /// A tab with no name — a program that never set one, which is most of them — gives
    /// the app's name alone. `zet` is a truer answer than `— zet`.
    fn os_window_title(&self) -> String {
        let name = self
            .app
            .active()
            .map(zet_session::Session::title)
            .unwrap_or_default();
        if name.is_empty() {
            APP_NAME.to_owned()
        } else {
            format!("{name} — {APP_NAME}")
        }
    }

    /// The tabs, as the strip needs them.
    fn tabs(&self) -> Vec<TabInfo> {
        let hovered = self
            .pointer
            .and_then(|(x, y)| match self.chrome.hit(x as f32, y as f32) {
                Hit::Tab(number) => Some(number),
                _ => None,
            });
        self.app
            .tab_numbers()
            .into_iter()
            .map(|number| TabInfo {
                index: number,
                title: self
                    .app
                    .sessions()
                    .get(number)
                    .map(zet_session::Session::title)
                    .unwrap_or_default(),
                hovered: hovered == Some(number),
            })
            .collect()
    }

    /// Where the scrollback is, for the scrollbar.
    fn scroll_state(&self) -> ScrollState {
        let Some(session) = self.app.active() else {
            return ScrollState::default();
        };
        let rows = usize::from(session.rows());
        let history = session.term().grid().scrollback_len();
        if history == 0 {
            return ScrollState::default();
        }
        // `scroll_offset` counts rows above the live screen, so how far *down* the
        // scrollback the viewport sits is the history minus that — the scrollbar's zero
        // is the oldest line, which is where a user expects the top of the bar to be.
        let above = session.scroll_offset().min(history);
        ScrollState {
            offset: (history - above) as f32 / history as f32,
            visible: rows as f32 / (rows + history) as f32,
        }
    }

    /// Put the viewport where a thumb dragged to `top` says it goes.
    ///
    /// The inverse of [`Host::scroll_state`]: the bar's zero is the oldest line, so a
    /// thumb at the top of its track is the far end of the scrollback and a thumb at the
    /// bottom is the live screen. The thumb is a proportion of the track that shrinks as
    /// the scrollback grows, so what the pointer is mapped onto is the distance the thumb
    /// can *travel* rather than the track's height — otherwise the two ends would be
    /// unreachable by exactly the thumb's height.
    fn drag_scrollbar(&mut self, top: f32) {
        let Some((track, thumb)) = self.chrome.scrollbar() else {
            return;
        };
        let Some(offset) = zet_ui::thumb_offset(track, thumb, top) else {
            return;
        };
        let Some(session) = self.app.active_mut() else {
            return;
        };
        let history = session.term().grid().scrollback_len();
        if history == 0 {
            return;
        }
        session.scroll_to((history as f32 * (1.0 - offset)).round() as usize);
    }

    /// The grid size the window has room for, in cells.
    fn grid_size(&self) -> (u16, u16) {
        let Some(renderer) = self.renderer.as_ref() else {
            return EMPTY_GRID;
        };
        // The same count the pointer is clamped to, from the same function, because the
        // two have to agree: a program told it has more columns than a click can reach
        // has columns nothing can be clicked in.
        let (cols, rows) = mouse::cells(self.placed.grid, renderer.metrics(), self.scale());
        // A window too short for one row is a real state — a window being dragged to the
        // top of the screen passes through it — and reporting zero columns would divide
        // by it further down.
        if cols == 0 || rows == 0 {
            return EMPTY_GRID;
        }
        (cols, rows)
    }

    /// Tell the sessions how much room they have, if that has changed.
    ///
    /// Called at the end of a frame rather than from the window's resize event, and the
    /// difference is the whole reason this exists. A resize event says how many pixels
    /// there are; how many *columns* that is depends on where the chrome puts the grid,
    /// which only the layout knows, and the layout is computed while the frame is being
    /// built. Fitting from the event meant fitting against the previous frame's layout
    /// and never correcting it — which is why a window that opened at 1100 by 720 spent
    /// its whole life as an eighty-column terminal.
    fn fit(&mut self) {
        let (cols, rows) = self.grid_size();
        let wanted = (cols.max(1), rows.max(1));
        if wanted == self.fitted {
            return;
        }
        self.fitted = wanted;
        self.app.resize(wanted.0, wanted.1);
    }

    /// Run whatever the app asked the host to do.
    fn carry_out(&mut self, loop_: &ActiveEventLoop, commands: Vec<Command>) {
        for command in commands {
            match command {
                Command::Copy(text) => crate::clipboard::set(&text),
                Command::Paste => {
                    if let Some(text) = crate::clipboard::get() {
                        self.app.paste(&text);
                    }
                }
                Command::NewWindow => {
                    // A second process rather than a second window in this loop. Two
                    // windows would otherwise share one `App` and therefore one tab
                    // list, and tabs belong to a window: a tab is where the next shell
                    // goes when you press the key, which is a per-window question.
                    if let Ok(executable) = std::env::current_exe() {
                        let _ = std::process::Command::new(executable).spawn();
                    }
                }
                Command::Quit => loop_.exit(),
            }
        }
    }

    /// A key event, on its way to a binding or to the shell.
    fn key(&mut self, event: &winit::event::KeyEvent, loop_: &ActiveEventLoop) {
        let Some(translated) = keys::translate(event, self.held) else {
            return;
        };

        // A row that asked for a key takes the next one whatever it is, because that is
        // what the user is looking at and the shell is not. `Escape` is the way out, and
        // it is spent rather than bound: a binding that cannot be undone without editing
        // the file is a binding that has to be got right first time.
        if let Some(action) = self.capturing.take() {
            if translated.key != Key::Escape {
                self.app.bind(
                    action,
                    Chord {
                        mods: translated.mods,
                        key: translated.key,
                    },
                );
                self.saved();
            }
            self.redraw();
            return;
        }

        // The two overlays belong to the window, which is where they are drawn, so the
        // binding is read here rather than left to the app. Nothing else is intercepted:
        // a key with no binding is a key for the shell, and that is the whole of the
        // keymap's design.
        // Through `action_for` rather than `bound`, because a chord is not an event:
        // asking the keymap directly would toggle the panel open on the press and shut
        // again on the release, which is one keystroke the user cannot see the effect
        // of.
        if self.app.action_for(&translated) == Some(Action::Settings) {
            self.settings_open = !self.settings_open;
            self.settings_scroll = 0.0;
            self.capturing = None;
            self.settings_focus = None;
            self.settings_left = false;
            self.redraw();
            return;
        }

        // The find bar's keys, and it has more of them than the panel does: it is a text
        // field, so what was typed is its own rather than the shell's. That is the one
        // place in this window where a letter does not reach the terminal, and it is the
        // whole point of a find bar — a query you cannot type is not a query.
        //
        // Before the panel, because a letter belongs to whichever of the two is asking
        // for text, and only one of them ever is.
        if self.app.find_key(&translated) {
            self.redraw();
            return;
        }

        // The panel's own keys, which it only has while the panel is open and only for
        // the ones it names. A letter is still a letter for the shell with the panel up,
        // which is what "it does not steal focus from the prompt" has to mean.
        if self.settings_open && self.panel_key(&translated) {
            self.redraw();
            return;
        }

        // DESIGN.md: the panel never traps focus, and a way out that does not depend on
        // remembering the chord you opened it with is the whole of that promise. Bare
        // `Escape` only — a modified one is somebody else's key, and the shell may well
        // be waiting for it.
        //
        // And a press only. The key going up is not a second Escape: without this the
        // documented two-stage Escape is defeated from the first press, because the
        // release closes a panel the press had already left open.
        if self.settings_open
            && translated.kind != KeyKind::Release
            && translated.key == Key::Escape
            && translated.mods.is_empty()
        {
            self.settings_open = false;
            self.settings_focus = None;
            self.capturing = None;
            self.redraw();
            return;
        }

        // Whether this key ran an action or went to the shell. An action has nothing
        // coming behind it: typing is answered by the program's echo, which arrives on
        // the pump and asks for a frame of its own, but opening the find bar, resizing
        // the font, or scrolling the view changes what is on screen and then stops
        // talking. Without this the bar would appear only once something else happened
        // to print.
        //
        // The question is asked of the app rather than of the keymap, so that the
        // answer is the same one the app acts on: a release matches a chord and runs
        // nothing, and a frame requested for it would be a redraw of a screen nothing
        // changed.
        let ran_an_action = self.app.action_for(&translated).is_some();
        let commands = self.app.key(&translated);
        if ran_an_action && let Some(window) = self.window() {
            window.request_redraw();
        }
        self.carry_out(loop_, commands);
    }

    /// A key aimed at the settings panel, and whether the panel took it.
    ///
    /// The panel's focus is entered and left with `Tab`, exactly as focus is traversed
    /// everywhere else, and `Tab` past the last row hands the keyboard back to the shell
    /// rather than wrapping. That is what "it never traps focus" has to mean: a panel
    /// that kept `Tab` until you remembered the chord you opened it with would be a
    /// panel you can get stuck in. Once it has been handed back, nothing the panel names
    /// is swallowed at all — the next `Tab` is the shell's, not a second chance to walk
    /// the rows.
    ///
    /// Only bare `Tab`, the four arrows, `Enter`, `Space` and `Escape` are named here.
    /// Everything else — every letter, and every chord with a modifier on it — falls
    /// through to the shell, because the terminal behind the panel is still live and
    /// typing into it is the reason the panel does not cover it.
    fn panel_key(&mut self, event: &KeyEvent) -> bool {
        // A repeat moves the highlight the way holding an arrow key should. A release is
        // not an event the panel has an opinion about.
        if event.kind == KeyKind::Release {
            return false;
        }
        if self.settings_left {
            return false;
        }
        let lines = self.app.settings();
        let rows: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.kind().is_some())
            .map(|(line, _)| line)
            .collect();
        if rows.is_empty() {
            return false;
        }

        // Where the focus is, in the list of rows rather than in the list of lines: the
        // two differ by the section headings, and every move here is along the rows.
        let here = self
            .settings_focus
            .and_then(|line| rows.iter().position(|row| *row == line));
        let focused = self
            .settings_focus
            .and_then(|line| lines.get(line))
            .and_then(zet_app::Line::id);

        let plain = event.mods.is_empty();
        match event.key {
            Key::Tab | Key::Down if plain => self.step_focus(&rows, here, true),
            Key::Tab if event.mods == Modifiers::SHIFT => self.step_focus(&rows, here, false),
            Key::Up if plain => self.step_focus(&rows, here, false),
            // `Escape` while a row is focused puts the keyboard back where it was, and
            // only a second one closes the panel. Two stages because they are two
            // different things to want, and the one you want first is the smaller.
            Key::Escape if plain && here.is_some() => self.settings_focus = None,
            Key::Left if plain && focused.is_some() => {
                if let Some(id) = focused {
                    self.adjust(id, true);
                }
            }
            Key::Right | Key::Enter | Key::Space if plain && focused.is_some() => {
                if let Some(id) = focused {
                    self.adjust(id, false);
                }
            }
            _ => return false,
        }
        true
    }

    /// Move the panel's keyboard focus one row, or off the end of the list.
    fn step_focus(&mut self, rows: &[usize], here: Option<usize>, forward: bool) {
        let next = next_focus(rows.len(), here, forward);
        // Landing on nothing is the panel handing the keyboard back, and it stays handed
        // back until something takes it again.
        self.settings_left = next.is_none();
        self.settings_focus = next.map(|at| rows[at]);
    }

    /// Write the configuration back, and say so when it could not be.
    ///
    /// A terminal that silently discards a setting the user just chose is worse than one
    /// that never offered it, so a failure is reported on stderr where a user who
    /// launched zet from a shell will see it. It is not fatal: the change is live either
    /// way, and losing it at exit is a smaller loss than losing the window.
    fn saved(&mut self) {
        let path = self.app.config_path().to_path_buf();
        if let Err(error) = zet_config::save(self.app.config(), &path) {
            eprintln!("zet: could not write {}: {error}", path.display());
        }
    }

    /// Carry out a click on a settings row.
    fn adjust(&mut self, id: zet_app::Id, back: bool) {
        match self.app.adjust(id, back) {
            zet_app::Effect::Capture(action) => self.capturing = Some(action),
            zet_app::Effect::Changed => {
                self.saved();
                // The chrome reads the panel's settings too, and the panel can change
                // them: a strip moved to the rail from inside the panel has to rebuild
                // the chrome, which `reconcile` notices on the next frame because the
                // config no longer matches what was built.
                self.redraw();
            }
            zet_app::Effect::None => {}
        }
    }

    /// Let go of everything the pointer was holding.
    ///
    /// Both drags in this window are the same shape: a press takes hold of something
    /// and every move afterwards moves it, until a release says otherwise. That makes
    /// the release load-bearing, and a release this window does not receive leaves the
    /// drag running for ever — the next move would scroll the view or sweep a
    /// selection with no button held at all.
    ///
    /// There is nothing to settle when one ends early: the view is already where the
    /// drag put it, and a half-swept selection is a selection.
    fn end_drags(&mut self) {
        self.scroll_grab = None;
        self.pressed_at = None;
    }

    /// The pointer moved, in the physical pixels the event carried.
    ///
    /// The conversion is here rather than at each of the five things that read the
    /// pointer, because they are all measured in logical pixels — the chrome's regions,
    /// the panel, the resize borders, the scrollbar, and the grid — and a pointer stored
    /// as it arrived is a pointer a scale factor away from the user on every display
    /// that is not at 100%.
    fn moved(&mut self, x: f64, y: f64) {
        let (x, y) = mouse::logical(x, y, self.scale());
        self.pointer = Some((x, y));
        let Some(window) = self.window.clone() else {
            return;
        };

        // A drag on the thumb is the pointer's, and nothing else's: a user holding the
        // scrollbar is not also sweeping a selection across the grid behind it.
        if let Some(grab) = self.scroll_grab {
            self.drag_scrollbar(y as f32 - grab);
            if let Some(window) = self.window() {
                window.request_redraw();
            }
            return;
        }

        if self.pressed_at.is_some()
            && let Some(at) = self.grid_cell(x, y)
        {
            self.app.select_to(at);
            // The selection is drawn, and nothing else about a pointer move asks for a
            // frame. Without this the sweep appears only when something else happens to
            // want one, and on a still screen with the cursor's blink turned off nothing
            // does until the button comes up — so the user drags across a screen that
            // does not answer.
            window.request_redraw();
        }

        // A resize border shows its arrow whenever the pointer is over it, and that
        // arrow is the only thing telling a user the window can be resized at all —
        // there is no frame drawn by Windows to suggest it.
        let (width, height) = self.logical_size();
        let cursor = match mouse::edge(x, y, width, height) {
            Some(edge) => edge.cursor(),
            None => CursorIcon::Default,
        };
        if cursor != self.cursor {
            window.set_cursor(cursor);
            self.cursor = cursor;
        }
    }

    /// A button went down or came up.
    fn button(&mut self, state: ElementState, button: WinitButton, loop_: &ActiveEventLoop) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let (x, y) = self.pointer.unwrap_or((0.0, 0.0));

        if state == ElementState::Pressed {
            let (width, height) = self.logical_size();
            // The window's own edge wins over everything else: a user reaching for the
            // border is not aiming at a tab that happens to be under it.
            if let Some(edge) = mouse::edge(x, y, width, height) {
                let _ = window.drag_resize_window(edge.direction());
                return;
            }
            match button {
                WinitButton::Left => {
                    if self.chrome_press(x, y, loop_) {
                        return;
                    }
                    if let Some(at) = self.grid_cell(x, y) {
                        self.pressed_at = Some(at);
                        self.app.select_from(at);
                        // A press starts a selection where a previous one was, so the
                        // thing on screen has changed even though the drag has not
                        // started yet.
                        window.request_redraw();
                    }
                }
                WinitButton::Middle => {
                    // Closing is a keyboard action, and DESIGN.md's strip has no close
                    // affordance on a tab — but a middle click on one is what every
                    // other terminal on this platform does, and a user who tries it
                    // should not be told no.
                    if let Hit::Tab(number) = self.chrome.hit(x as f32, y as f32) {
                        // Closing the last tab is closing the window, which is what the
                        // bound `close-tab` action does with the same answer. Dropping it
                        // left a window with no terminal in it: the shell was gone, the
                        // last frame was still painted, and nothing was listening for the
                        // exit that ends the loop.
                        match self.app.close_tab(number) {
                            Ok(true) => loop_.exit(),
                            Ok(false) => window.request_redraw(),
                            Err(_) => {}
                        }
                        return;
                    }
                }
                _ => {}
            }
        } else if button == WinitButton::Left && self.scroll_grab.take().is_some() {
            // Letting go of the thumb. There is nothing to settle: the view is already
            // where the drag put it.
        } else if button == WinitButton::Left
            && let Some(pressed) = self.pressed_at.take()
        {
            // A click that never moved is a click that clears the selection, which is
            // what a user who wants to type again is asking for. A drag leaves the
            // selection alone, because that is what the drag was for.
            if self.grid_cell(x, y) == Some(pressed) {
                self.app.select_none();
                // Clearing a selection nobody had is a frame of nothing, and clearing one
                // somebody had is the frame that takes it off the screen.
                window.request_redraw();
            }
        }

        // A button the chrome did not want goes to the program, if it asked for mouse
        // reporting. `encode_mouse` answers `None` when it did not.
        self.forward_mouse(state, button);
    }

    /// Hand a press to the chrome, and report whether it was the chrome's.
    fn chrome_press(&mut self, x: f64, y: f64, loop_: &ActiveEventLoop) -> bool {
        let Some(window) = self.window.clone() else {
            return false;
        };
        let handled = match self.chrome.hit(x as f32, y as f32) {
            Hit::Tab(number) => {
                self.app.activate(number);
                true
            }
            Hit::NewTab => {
                let (cols, rows) = self.grid_size();
                let _ = self.app.open_tab(cols.max(1), rows.max(1));
                true
            }
            Hit::Caption(caption) => {
                match caption {
                    Caption::Minimize => window.set_minimized(true),
                    Caption::Maximize => window.set_maximized(!window.is_maximized()),
                    // Closing the window, not the tab. A caption button that closed one
                    // terminal and left the window up would be a button that lies about
                    // what it does; `close-tab` is the key for that.
                    Caption::Close => loop_.exit(),
                }
                true
            }
            Hit::Drag => {
                let _ = window.drag_window();
                true
            }
            Hit::Scrollbar(place) => {
                let delta = match place {
                    zet_ui::Scrollbar::Above => SCROLL_PAGE,
                    zet_ui::Scrollbar::Below => -SCROLL_PAGE,
                    // Taking hold of the thumb, as opposed to clicking the band beside
                    // it, which pages. The grab point is where in the thumb the pointer
                    // went down, so the thumb does not jump to re-centre itself under it.
                    zet_ui::Scrollbar::Thumb => {
                        self.scroll_grab =
                            self.chrome.scrollbar().map(|(_, thumb)| y as f32 - thumb.y);
                        0
                    }
                };
                if delta != 0
                    && let Some(session) = self.app.active_mut()
                {
                    session.scroll(delta);
                }
                true
            }
            Hit::CloseTab(number) => {
                let _ = self.app.close_tab(number);
                true
            }
            Hit::Setting { line, part } => {
                // The list drawn this frame and the list read here are the same function
                // of the same configuration, so the index names the same row. A row that
                // is not there is not reachable — the chrome hit-tests what it drew.
                if let Some(id) = self.app.settings().get(line).and_then(zet_app::Line::id) {
                    // Clicking a row is also how the keyboard gets there: the pointer and
                    // the keyboard are two ways to the same highlight, and a panel where
                    // they were two different states would need two of everything.
                    self.settings_focus = Some(line);
                    self.settings_left = false;
                    self.adjust(id, part == zet_ui::SettingPart::Less);
                }
                true
            }
            // The panel's own surface, between and around its controls. Swallowed
            // rather than passed on: the terminal behind it stays visible, which
            // DESIGN.md gives as the reason the panel exists at all, but a click on a
            // surface is a click on the surface and not on what shows through it.
            Hit::Settings => true,
            Hit::None => false,
        };
        // Asked once, here, rather than in each arm that happens to change something.
        // Every hit but `Hit::None` is a press on a thing the window is drawing, and the
        // loop only draws when it is asked to: the tab strip, the new-tab mark, the close
        // mark and the scrollbar all moved state and left the frame that was already on
        // screen, so clicking a tab switched the terminal out from under a picture of the
        // old one and dragging the scrollbar moved a thumb that was not redrawn. One
        // request at the end is what makes the next hit target correct by default.
        if handled {
            window.request_redraw();
        }
        handled
    }

    /// The cell a point is over, if any.
    fn grid_cell(&self, x: f64, y: f64) -> Option<zet_vt::Pos> {
        let renderer = self.renderer.as_ref()?;
        mouse::cell(x, y, self.placed.grid, renderer.metrics(), self.scale())
    }

    /// Send a button event to the program, if it asked for mouse reporting.
    fn forward_mouse(&self, state: ElementState, button: WinitButton) {
        let Some(session) = self.app.active() else {
            return;
        };
        let Some((x, y)) = self.pointer else {
            return;
        };
        let Some(at) = self.grid_cell(x, y) else {
            return;
        };
        let event = MouseEvent {
            button: mouse::button(button),
            action: mouse::action(state),
            col: u16::try_from(at.col).unwrap_or(u16::MAX),
            row: u16::try_from(at.row).unwrap_or(u16::MAX),
            mods: keys::translate_modifiers(self.held),
        };
        if let Some(bytes) = encode_mouse(event, &session.term().modes()) {
            let _ = session.write(&bytes);
        }
    }

    /// The wheel turned.
    ///
    /// Where the scroll goes is not a decision this file gets to make. A program on the
    /// alternate screen — vim, htop, less — that has asked for mouse reporting wants the
    /// wheel as button presses, because it knows what its own scrollback means and zet
    /// does not. Everything else wants zet's scrollback, which is the only history there
    /// is. `encode_mouse` answers `None` for the first case when reporting is off, so
    /// asking it is the whole test.
    fn wheel(&mut self, delta: MouseScrollDelta) {
        let cell = self
            .renderer
            .as_ref()
            .map_or(0.0, |r| f64::from(r.metrics().cell_height));
        let lines = mouse::wheel(delta, cell / f64::from(self.scale()));
        if lines == 0.0 {
            return;
        }

        // The panel is a list that can be longer than the window, and a list with no
        // wheel is a list whose last rows are unreachable on a laptop with no End key.
        // The pointer decides, because both surfaces are on screen at once and the one
        // under the cursor is the one the user is looking at.
        if self.settings_open && self.pointer.is_some_and(|(x, y)| self.over_panel(x, y)) {
            self.settings_scroll = (self.settings_scroll - lines as f32 * PANEL_WHEEL).max(0.0);
            if let Some(window) = self.window.clone() {
                window.request_redraw();
            }
            return;
        }

        if self.wheel_to_program(lines) {
            return;
        }

        // A terminal has no sub-line scrolling, so a trackpad's fraction of a line is
        // accumulated rather than dropped: at sixty events a second, each worth a tenth
        // of a line, dropping the remainder is a scroll that never happens at all.
        self.partial += lines;
        let whole = self.partial.trunc();
        if whole == 0.0 {
            return;
        }
        self.partial -= whole;
        let Ok(step) = i32::try_from(whole as i64) else {
            return;
        };
        if let Some(session) = self.app.active_mut() {
            session.scroll(step);
        }
    }

    /// Send the wheel to the program, and say whether it wanted it.
    fn wheel_to_program(&self, lines: f64) -> bool {
        let Some(session) = self.app.active() else {
            return false;
        };
        let Some((x, y)) = self.pointer else {
            return false;
        };
        let Some(at) = self.grid_cell(x, y) else {
            return false;
        };
        let event = MouseEvent {
            button: mouse::wheel_button(lines),
            action: zet_input::MouseAction::Press,
            col: u16::try_from(at.col).unwrap_or(u16::MAX),
            row: u16::try_from(at.row).unwrap_or(u16::MAX),
            mods: keys::translate_modifiers(self.held),
        };
        let Some(bytes) = encode_mouse(event, &session.term().modes()) else {
            return false;
        };
        let _ = session.write(&bytes);
        true
    }

    /// Whether a point is inside the settings panel.
    ///
    /// Asked of the chrome rather than worked out from the panel's width, because the
    /// width is the chrome's business and a second copy of it here is a second place for
    /// it to be wrong. The chrome hit-tests what it actually drew, which is also what
    /// the user is looking at.
    fn over_panel(&self, x: f64, y: f64) -> bool {
        matches!(
            self.chrome.hit(x as f32, y as f32),
            Hit::Settings | Hit::Setting { .. }
        )
    }

    /// The window changed size.
    fn resized(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.resize(size.width, size.height, scale);
        }
        // The sessions are told their new size by the frame this asks for, from the
        // layout that frame builds.
        window.request_redraw();
    }

    /// The DPI changed, which is a different problem from the window changing size.
    ///
    /// The glyphs are rasterised at a size that is in physical pixels, so every one of
    /// them is wrong after this and both faces have to be rebuilt — which is why this
    /// does not go through `reconcile`'s size comparison, since the size has not changed.
    fn scaled(&mut self, scale: f32) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let Ok(chrome) = self.chrome_face(scale) else {
            return;
        };
        let size = window.inner_size();
        if let Some(renderer) = self.renderer.as_mut() {
            if renderer.set_scale(scale, chrome).is_err() {
                return;
            }
            renderer.resize(size.width, size.height, scale);
        }
        window.request_redraw();
    }
}

/// The settings rows, in the shape the chrome draws them.
///
/// The two crates' vocabularies meet here and nowhere else: `zet-app` decides what a row
/// means and answers with an id, `zet-ui` decides how it looks and is told a control.
///
/// A row that is waiting for a key says so rather than going on showing the chord it is
/// about to lose. Without that the user has no way to tell that the next key they press
/// is not going to the shell — which, with the panel drawn over a live terminal, is
/// exactly the mistake worth designing out.
fn panel_lines(lines: &[zet_app::Line], capturing: Option<Action>) -> Vec<zet_ui::SettingLine<'_>> {
    lines
        .iter()
        .map(|line| {
            let waiting =
                capturing.is_some_and(|action| line.id() == Some(zet_app::Id::Binding(action)));
            zet_ui::SettingLine {
                text: line.text(),
                control: line.kind().map(control_of),
                value: if waiting { PRESS_A_KEY } else { line.value() },
            }
        })
        .collect()
}

/// Where the panel's keyboard focus goes next.
///
/// `None` is a real destination and the one past the last row in either direction:
/// leaving the panel is how the shell gets its arrow keys back, and a `Tab` that wrapped
/// would be a `Tab` the terminal never sees again.
fn next_focus(rows: usize, here: Option<usize>, forward: bool) -> Option<usize> {
    match (here, forward) {
        (None, true) => Some(0),
        (None | Some(0), false) => None,
        (Some(at), false) => Some(at - 1),
        (Some(at), true) if at + 1 == rows => None,
        (Some(at), true) => Some(at + 1),
    }
}

/// The text scale the chrome and the grid are drawn at.
///
/// Windows' own text-size slider and the panel's row answer the same question, and the
/// configuration decides which of them is speaking: `0.0` is "follow the system", which
/// is the default and is what makes the system slider work without zet having to be told
/// about it twice. Any other value is the user overruling it.
fn text_scale(config: &Config, system: f32) -> f32 {
    match config.appearance.text_scale {
        0.0 => system,
        // The scale multiplies the font size, and a size that is not a number reaches
        // the font loader as one and is refused there — so a config the schema has
        // already reported as out of range would still be a window that never opened.
        // The fallback is the system's scale, which is what the file asked for when it
        // wrote the `0.0` above.
        scale => zet_config::clamp_or(
            scale,
            zet_config::MIN_TEXT_SCALE,
            zet_config::MAX_TEXT_SCALE,
            system,
        ),
    }
}

/// The grid's face settings, with the text scale applied.
fn grid_settings(app: &App, text_scale: f32) -> FontSettings {
    FontSettings {
        size: app.font_size() * text_scale,
        ..app.config().font.clone()
    }
}

/// The chrome's spelling of a settings row's control.
///
/// The same four things under two names, and this is the only place they meet: the app
/// decides what a row *does* and knows nothing about how it looks, the chrome decides
/// how it looks and is told nothing about what it means, and neither crate can see the
/// other. Two enums with four variants each is the whole of the price, and it is the
/// price of the app being testable without a window.
const fn control_of(kind: zet_app::Kind) -> zet_ui::Control {
    match kind {
        zet_app::Kind::Choice => zet_ui::Control::Choice,
        zet_app::Kind::Toggle => zet_ui::Control::Toggle,
        zet_app::Kind::Step => zet_ui::Control::Step,
        zet_app::Kind::Chord => zet_ui::Control::Chord,
    }
}

/// The chrome's glyphs, as the chrome's own trait wants them.
///
/// A one-field newtype, and nothing else could be: `zet_ui::GlyphSource` and
/// `zet_render::ChromeGlyphs` are both foreign here, so Rust's orphan rule forbids
/// implementing one for the other. Wrapping is the only way, and the wrapper's entire
/// content is the one method the chrome's trait adds to the renderer's.
struct ChromeFace<'a>(zet_render::ChromeGlyphs<'a>);

impl zet_render::GlyphSource for ChromeFace<'_> {
    fn place(&mut self, spec: zet_font::GlyphSpec) -> Option<zet_render::Placement> {
        self.0.place(spec)
    }
}

impl zet_ui::GlyphSource for ChromeFace<'_> {
    fn metrics(&self) -> &zet_font::Metrics {
        self.0.metrics()
    }
}

/// Why the window never opened.
#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    /// The platform refused to make a window.
    #[error("could not create a window: {0}")]
    Window(winit::error::OsError),

    /// A face could not be loaded, or no device could be had.
    #[error(transparent)]
    Render(#[from] RendererError),

    /// No shell was found on this machine.
    #[error(transparent)]
    App(#[from] AppError),
}

impl ApplicationHandler<Wake> for Host {
    fn resumed(&mut self, loop_: &ActiveEventLoop) {
        // `resumed` fires again after a suspend on the platforms that have them, and a
        // second window is not what that means.
        if self.window.is_some() {
            return;
        }
        if let Err(error) = self.attach(loop_) {
            startup_failed(&error);
            loop_.exit();
            return;
        }
        // A terminal that opens with no terminal in it is a rectangle. The first tab is
        // opened here rather than by the app because the app cannot know how big the
        // window is until the window exists.
        let (cols, rows) = self.grid_size();
        if let Err(error) = self.app.open_tab(cols.max(1), rows.max(1)) {
            startup_failed(&error.into());
            loop_.exit();
            return;
        }
        if let Some(window) = self.window() {
            window.request_redraw();
        }
    }

    fn user_event(&mut self, loop_: &ActiveEventLoop, wake: Wake) {
        // The font database is the one wake that is not about a session, and it is
        // handled and done with rather than falling through: there is no output to drain
        // and no tab that could have closed.
        if let Wake::Families(families) = wake {
            self.app.set_families(families);
            // The panel is the only thing that reads them, and a panel that is not open
            // does not need the frame.
            if self.settings_open
                && let Some(window) = self.window.clone()
            {
                window.request_redraw();
            }
            return;
        }

        // The session that woke us is not named and does not need to be: draining walks
        // every open session, which is a handful of non-blocking reads and therefore
        // cheaper than tracking which one moved.
        self.app.pump();

        // A tab whose shell exited is closed by the app, and the last one closing is
        // what ends the window — the same decision `Command::Quit` carries, made in the
        // same place, for a tab the user never touched.
        let closed = self.app.reap();
        if closed.is_empty() {
            if let Some(window) = self.window() {
                window.request_redraw();
            }
            return;
        }
        if self.app.sessions().is_empty() {
            loop_.exit();
        } else if let Some(window) = self.window() {
            window.request_redraw();
        }
    }

    fn window_event(&mut self, loop_: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => loop_.exit(),
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::Resized(_) => self.resized(),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scaled(scale_factor as f32);
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                if !focused {
                    // A drag does not survive the window losing focus, and it cannot
                    // be waited out: the button coming up is delivered to whoever has
                    // the pointer, which by then is not this window, so the release
                    // this is meant to end on is one that will never arrive.
                    self.end_drags();
                }
                // A focus change is one of the moments the accessibility settings are
                // re-read. A user who has just been in the Settings app is likely to
                // have changed one, and this is what makes the change take effect
                // without a restart.
                self.reread_system();
                if let Some(window) = self.window() {
                    window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => self.held = modifiers.state(),
            WindowEvent::KeyboardInput { event, .. } => self.key(&event, loop_),
            WindowEvent::Ime(Ime::Commit(text)) => {
                self.app.paste(&text);
                if let Some(window) = self.window() {
                    window.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => self.moved(position.x, position.y),
            WindowEvent::CursorLeft { .. } => {
                self.pointer = None;
                // Same reason as losing focus, and it is the commoner case: a drag
                // taken off the edge of the window is a drag whose release happens
                // somewhere this window is not listening.
                self.end_drags();
            }
            WindowEvent::MouseInput { state, button, .. } => self.button(state, button, loop_),
            WindowEvent::MouseWheel { delta, .. } => self.wheel(delta),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, loop_: &ActiveEventLoop) {
        // The cursor's blink is the only thing in zet that happens on a clock, so the
        // loop is only ever woken for it when there is a cursor to blink. A terminal
        // sitting idle with nothing to flash parks in `Wait` and uses no power at all.
        if !self.app.blinks() {
            loop_.set_control_flow(ControlFlow::Wait);
            return;
        }
        let now = Instant::now();
        let next = self.app.next_blink(now);
        if now >= self.blink_at {
            // The phase has flipped since the last frame was drawn, and the deadline has
            // moved past it, so the frame that follows this one cannot ask for another
            // the same way. That is what makes this terminate.
            self.blink_at = next;
            if let Some(window) = self.window() {
                window.request_redraw();
            }
        }
        loop_.set_control_flow(ControlFlow::WaitUntil(self.blink_at));
    }
}

/// Say why the window never opened, in the two places it can be seen.
///
/// Stderr, for a `zet` launched from a terminal that is still watching it, and a message
/// box for a `zet` launched from the Start menu, where stderr goes nowhere at all. The
/// second is the one that matters for a graphics failure, which is the failure a user is
/// least equipped to diagnose from a process that simply did not appear.
fn startup_failed(error: &StartupError) {
    let message = error.to_string();
    eprintln!("zet: {message}");
    let hint = match error {
        StartupError::Render(RendererError::Gpu(_)) => {
            "\n\nThis is usually a graphics driver problem. Setting WGPU_BACKEND=gl \
             before launching zet makes it try OpenGL instead of Direct3D."
        }
        _ => "",
    };
    crate::platform::alert("zet could not start", &format!("{message}{hint}"));
}

#[cfg(test)]
mod tests {
    // The comparisons here are exact on purpose: every one of them is an assertion that
    // an arithmetic result equals a literal, not a tolerance test dressed up as one.
    #![allow(clippy::float_cmp)]

    use super::*;
    use zet_config::Config;

    fn app() -> App {
        let waker: Arc<dyn zet_session::Waker> = Arc::new(zet_session::NoopWaker);
        match App::new(
            Config::default(),
            std::path::PathBuf::from("config.toml"),
            Vec::new(),
            waker,
        ) {
            Ok(app) => app,
            // A machine with no shell at all cannot reach `Host::new`, so the test that
            // would fail here is the test of a machine that cannot run zet at all.
            Err(error) => panic!("this machine has no shell: {error}"),
        }
    }

    #[test]
    fn a_host_with_no_window_reports_a_grid_size_rather_than_zero() {
        // Everything that opens a tab before the window exists asks this, and a zero
        // would become a session with no columns.
        let host = Host::new(app());
        assert_eq!(host.grid_size(), EMPTY_GRID);
    }

    #[test]
    fn the_app_is_told_what_the_system_said_before_the_first_frame() {
        // Two apps put on opposite answers are handed over, so that this is caught on any
        // machine: whatever the system says, one of the two was on the wrong side of it
        // and a `Host::new` that did not tell the app would leave it there.
        //
        // It matters because `reread_system` returns early when the settings have not
        // moved, and they have not moved — a first read that reached only the host would
        // leave the app on its defaults for the whole session, blinking a cursor on a
        // machine that asked for no motion and drawing normal colours to a user with high
        // contrast on.
        let mut on = app();
        on.system_accessibility(true, true);
        let mut off = app();
        off.system_accessibility(false, false);

        let told_on = Host::new(on);
        let told_off = Host::new(off);
        assert_eq!(
            told_on.settings, told_off.settings,
            "both hosts read the same system"
        );
        for host in [&told_on, &told_off] {
            assert_eq!(
                host.app.reduce_motion(),
                host.settings.reduce_motion,
                "the app was not told what the system said about motion"
            );
            assert_eq!(
                host.app.theme().slug == "zet-contrast",
                host.settings.high_contrast,
                "the app was not told what the system said about contrast"
            );
        }
    }

    #[test]
    fn the_chrome_palette_follows_the_forced_colours_highlight() {
        let settings = SystemSettings::default();
        assert_eq!(settings.highlight, None);
        let mut config = Config::default();
        let plain = zet_config::palette_for(&config, None);
        let forced = zet_config::palette_for(&config, Some(zet_config::Rgb::new(0, 120, 215)));
        assert_ne!(plain, forced);
        // The user's own setting still wins when they have turned it off, which is the
        // one place forced colours does not override them.
        config.appearance.follow_forced_colors = false;
        assert_eq!(
            zet_config::palette_for(&config, Some(zet_config::Rgb::new(0, 120, 215))),
            plain
        );
    }

    #[test]
    fn the_panel_hands_the_keyboard_back_once_tab_has_walked_off_the_end() {
        // "It never traps focus" has to mean the shell sees a `Tab` again at some point,
        // and the state the panel is in when it does is the whole of this: `None` means
        // either "not asked for yet" or "handed back", and the next `Tab` has to tell
        // them apart or the panel wraps and the shell never gets one.
        let mut host = Host::new(app());
        host.settings_open = true;
        let tab = KeyEvent {
            key: Key::Tab,
            mods: Modifiers::empty(),
            text: None,
            kind: KeyKind::Press,
        };
        let down = KeyEvent {
            key: Key::Down,
            mods: Modifiers::empty(),
            text: None,
            kind: KeyKind::Press,
        };

        assert!(
            host.panel_key(&tab),
            "the panel takes the Tab that enters it"
        );
        assert!(host.settings_focus.is_some(), "and it lands on a row");

        // Walk to the end of the rows. The bound is a guard against the loop outliving
        // the panel, not an expectation about how many rows there are.
        let mut steps = 0;
        while host.settings_focus.is_some() && steps < 200 {
            assert!(host.panel_key(&tab), "a Tab on a row is the panel's");
            steps += 1;
        }
        assert!(steps > 1, "the panel has more than one row");
        assert!(host.settings_left, "the last Tab walked off the end");

        assert!(!host.panel_key(&tab), "so this one is the shell's");
        assert!(!host.panel_key(&down), "and so are the arrow keys");
        assert!(
            host.settings_focus.is_none(),
            "the panel must not re-enter itself on the Tab that left it"
        );

        // The chord takes it back, which is the only way in that survives a user who
        // has already tabbed out once.
        host.settings_left = false;
        assert!(host.panel_key(&tab));
    }

    #[test]
    fn clicking_a_row_takes_the_keyboard_back_from_the_shell() {
        let mut host = Host::new(app());
        host.settings_open = true;
        host.settings_left = true;
        let Some(line) = host.app.settings().iter().position(|l| l.kind().is_some()) else {
            panic!("the panel has no rows to click");
        };
        host.settings_focus = Some(line);
        host.settings_left = false;
        assert!(host.panel_key(&KeyEvent {
            key: Key::Tab,
            mods: Modifiers::empty(),
            text: None,
            kind: KeyKind::Press,
        }));
    }

    #[test]
    fn the_text_scale_multiplies_the_configured_font_size() {
        let app = app();
        let plain = grid_settings(&app, 1.0);
        let doubled = grid_settings(&app, 2.0);
        assert_eq!(plain.family, doubled.family);
        assert_eq!(doubled.size, plain.size * 2.0);
    }

    #[test]
    fn the_panel_focus_walks_the_rows_and_then_leaves() {
        // Every step here is either "the next row" or "out", and out is what keeps the
        // panel from being somewhere you can get stuck: with the last row focused, one
        // more `Tab` has to hand the arrow keys back to the shell.
        assert_eq!(next_focus(3, None, true), Some(0), "in at the top");
        assert_eq!(next_focus(3, Some(0), true), Some(1));
        assert_eq!(next_focus(3, Some(1), true), Some(2));
        assert_eq!(next_focus(3, Some(2), true), None, "past the end is out");
        assert_eq!(next_focus(3, None, false), None, "backwards from nowhere");
        assert_eq!(
            next_focus(3, Some(0), false),
            None,
            "backwards past the top"
        );
        assert_eq!(next_focus(3, Some(2), false), Some(1));

        // One row is the case where "in", "out" and "wrapped" are all the same index.
        assert_eq!(next_focus(1, None, true), Some(0));
        assert_eq!(next_focus(1, Some(0), true), None);
        assert_eq!(next_focus(1, Some(0), false), None);
    }

    #[test]
    fn a_text_scale_of_zero_means_the_system_and_anything_else_means_the_user() {
        // The panel's row and Windows' own text-size slider answer the same question, and
        // this is the whole of who wins: the default defers, and every other value is
        // somebody having overruled it.
        let mut config = Config::default();
        assert!(config.appearance.text_scale.abs() < f32::EPSILON);
        assert!((text_scale(&config, 1.25) - 1.25).abs() < f32::EPSILON);
        config.appearance.text_scale = 2.0;
        assert!((text_scale(&config, 1.25) - 2.0).abs() < f32::EPSILON);
        assert!(
            (text_scale(&config, 3.0) - 2.0).abs() < f32::EPSILON,
            "a value the user set is not the system's to override"
        );
    }
}
