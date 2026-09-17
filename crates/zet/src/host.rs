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

use zet_app::{App, AppError, Command};
use zet_config::{FontSettings, Palette, TabSettings};
use zet_font::{FontError, FontStack};
use zet_input::{MouseEvent, encode_mouse};
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
    /// Scrolling sub-line remainders, accumulated so a trackpad's fractions are not
    /// dropped one event at a time.
    partial: f64,

    /// When the cursor's blink phase last changed, so the loop can wake for the next one.
    blink_at: Instant,
    /// The moment the host started, for the chrome's clock.
    origin: Instant,

    /// The grid face's settings as of the last load, so a change is noticed.
    styled: FontSettings,
    /// The chrome's settings as of the last build, so a change is noticed.
    tabbed: TabSettings,
    /// The layout the previous frame's grid was positioned with.
    placed: Layout,
    /// The grid size the sessions were last told about, so that a frame which did not
    /// change it does not resize every one of them again.
    fitted: (u16, u16),
}

impl Host {
    /// Build a host around a loaded app.
    ///
    /// Reads the system's accessibility settings here rather than on the first frame,
    /// because the first frame should already be the right one: a high-contrast user
    /// seeing a normal-contrast window for a frame and then a white one is a flash
    /// bright enough to be worth avoiding.
    #[must_use]
    pub fn new(app: App) -> Self {
        let settings = SystemSettings::read();
        let palette = zet_config::palette_for(app.config(), settings.highlight);
        let chrome = Chrome::new(&app.config().tabs, &app.config().window);
        let tabbed = app.config().tabs.clone();
        let styled = grid_settings(&app, settings.text_scale);
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
            partial: 0.0,
            blink_at: now,
            origin: now,
            styled,
            tabbed,
            placed: Layout::default(),
            // What `resumed` opens the first tab at, before any frame has been laid out
            // and therefore before anything knows how big the window really is.
            fitted: EMPTY_GRID,
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
        let grid = grid_settings(&self.app, self.settings.text_scale);
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
        settings.size *= self.settings.text_scale;
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
        let wanted = grid_settings(&self.app, self.settings.text_scale);
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
    /// The app is told about forced contrast because the *grid's* theme changes; the
    /// chrome's palette is derived here because the chrome is this file's business. The
    /// text scale is part of the font size, so a change to it invalidates the loaded
    /// faces — which is expressed by making `reconcile` see a different size.
    fn reread_system(&mut self) {
        let settings = SystemSettings::read();
        if settings == self.settings {
            return;
        }
        self.settings = settings;
        self.app.force_contrast(settings.high_contrast);
        self.palette = zet_config::palette_for(self.app.config(), settings.highlight);
        // Nothing to invalidate by hand: `reconcile` derives the wanted size from
        // `settings.text_scale` every time it runs, so the change is picked up on the
        // next frame without a flag to keep true.
    }

    /// Draw everything and put it on screen.
    fn redraw(&mut self) {
        // Before the renderer is borrowed: `reconcile` may reload a face, and it needs
        // the whole host to do it.
        self.reconcile();

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
        let input = ChromeInput {
            palette: &self.palette,
            tabs: &tabs,
            active,
            settings_open: false,
            window_title: APP_NAME,
            size: Size {
                width: width as f32,
                height: height as f32,
            },
            scale,
            maximized: window.is_maximized(),
            reduce_motion: self.settings.reduce_motion,
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
            };
            zet_render::draw_grid(
                session.term(),
                theme,
                &metrics,
                &cursor_settings,
                &view,
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

    /// The grid size the window has room for, in cells.
    fn grid_size(&self) -> (u16, u16) {
        let Some(renderer) = self.renderer.as_ref() else {
            return EMPTY_GRID;
        };
        let metrics = renderer.metrics();
        if metrics.cell_width <= 0.0 || metrics.cell_height <= 0.0 {
            return EMPTY_GRID;
        }
        let scale = f64::from(self.scale());
        let grid = self.placed.grid;
        let width = f64::from(grid.width) * scale;
        let height = f64::from(grid.height) * scale;
        // A window too short for one row is a real state — a window being dragged to the
        // top of the screen passes through it — and reporting zero columns would divide
        // by it further down.
        if width < f64::from(metrics.cell_width) || height < f64::from(metrics.cell_height) {
            return EMPTY_GRID;
        }
        (
            (width / f64::from(metrics.cell_width)) as u16,
            (height / f64::from(metrics.cell_height)) as u16,
        )
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
        let commands = self.app.key(&translated);
        self.carry_out(loop_, commands);
    }

    /// The pointer moved.
    fn moved(&mut self, x: f64, y: f64) {
        self.pointer = Some((x, y));
        let Some(window) = self.window.clone() else {
            return;
        };

        if self.pressed_at.is_some()
            && let Some(at) = self.grid_cell(x, y)
        {
            self.app.select_to(at);
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
                    }
                }
                WinitButton::Middle => {
                    // Closing is a keyboard action, and DESIGN.md's strip has no close
                    // affordance on a tab — but a middle click on one is what every
                    // other terminal on this platform does, and a user who tries it
                    // should not be told no.
                    if let Hit::Tab(number) = self.chrome.hit(x as f32, y as f32) {
                        let _ = self.app.close_tab(number);
                        return;
                    }
                }
                _ => {}
            }
        } else if button == WinitButton::Left
            && let Some(pressed) = self.pressed_at.take()
        {
            // A click that never moved is a click that clears the selection, which is
            // what a user who wants to type again is asking for. A drag leaves the
            // selection alone, because that is what the drag was for.
            if self.grid_cell(x, y) == Some(pressed) {
                self.app.select_none();
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
        match self.chrome.hit(x as f32, y as f32) {
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
                    // Dragging the thumb needs the track's rectangle, which the chrome
                    // does not publish. The wheel and the two bands are what a user
                    // actually reaches for, so this is a gap rather than a bug.
                    zet_ui::Scrollbar::Thumb => 0,
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
            // A point inside the settings panel is a click on a control this version
            // does not have. Swallowing it keeps it away from the shell, which is the
            // right answer while the panel is deferred.
            Hit::Settings => true,
            Hit::None => false,
        }
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

/// The grid's face settings, with the system's text scale applied.
fn grid_settings(app: &App, text_scale: f32) -> FontSettings {
    FontSettings {
        size: app.font_size() * text_scale,
        ..app.config().font.clone()
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
        // The session that woke us is not named and does not need to be: draining walks
        // every open session, which is a handful of non-blocking reads and therefore
        // cheaper than tracking which one moved. The `let` is exhaustive because there
        // is one thing the loop can be woken for.
        let Wake::Output = wake;
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
            WindowEvent::CursorLeft { .. } => self.pointer = None,
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
    fn the_text_scale_multiplies_the_configured_font_size() {
        let app = app();
        let plain = grid_settings(&app, 1.0);
        let doubled = grid_settings(&app, 2.0);
        assert_eq!(plain.family, doubled.family);
        assert_eq!(doubled.size, plain.size * 2.0);
    }
}
