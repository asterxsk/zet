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

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, Ime, MouseButton as WinitButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::ModifiersState;
use winit::window::{CursorIcon, Window, WindowId};

use zet_app::{Action, App, AppError, Command};
use zet_config::{Config, FontSettings, Palette, TabSettings, WindowSettings};
use zet_font::{FontError, FontStack};
use zet_input::{Chord, Key, KeyEvent, KeyKind, Modifiers, MouseEvent, encode_focus, encode_mouse};
use zet_render::{Frame, Picture, Renderer, RendererError, View};
use zet_ui::{Caption, Chrome, ChromeInput, Hit, Layout, ScrollState, Size, TabId, TabInfo};

use crate::keys;
use crate::mouse;
use crate::placement;
use crate::platform::{SystemSettings, set_corners, set_opacity};
use crate::waker::Wake;

/// The app's own name, as the titlebar's name slot shows it.
const APP_NAME: &str = "zet";

/// What the settings tab is called, in the strip and in the window's title.
///
/// Lower case, because the strip's own names are whatever a program set and there is no rule
/// that makes an app's own tab shout when the shells' do not. It is also the whole of the cell:
/// a settings tab has no number in front of it, so this word is the only thing that says which
/// tab it is.
const SETTINGS_TAB: &str = "settings";

/// The window's background, as the frame wants it: a thing to draw or nothing at all.
///
/// The configuration's own vocabulary turned into the renderer's, in one place, because
/// the two disagree about what "solid" means in a way that is correct on both sides:
/// `Background::Solid` is the theme's ground, which reaches the frame as the clear colour
/// and therefore as no backdrop at all, and a caller that turned it into a flat gradient
/// would be drawing the ground twice and paying for a shader to do it.
///
/// The size is the surface's, in physical pixels: a gradient is defined across the window
/// and a window that is resized has its gradient resized with it, which is what a
/// background is rather than what a picture in it would be. A picture is the other way
/// round — it has a size of its own that the window does not change — which is why the
/// frame only carries how strongly to draw it and the renderer keeps the rest.
fn backdrop(
    background: &zet_config::Background,
    width: f32,
    height: f32,
) -> Option<zet_render::Backdrop> {
    match background {
        zet_config::Background::Solid => None,
        zet_config::Background::Gradient { from, to, angle } => {
            Some(zet_render::Backdrop::Gradient(zet_render::Gradient::new(
                0.0,
                0.0,
                width,
                height,
                from.to_linear(),
                to.to_linear(),
                *angle,
            )))
        }
        zet_config::Background::Image { opacity, .. } => {
            Some(zet_render::Backdrop::Picture { opacity: *opacity })
        }
    }
}

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

/// How long a frame lasts while a hover is fading, in the event loop's own deadline.
///
/// The chrome's hover is a transition of a hundred and ten milliseconds, and this is the
/// rate it is sampled at: two frames more than the sixty a second a display is likely to
/// draw, so that a fade is never the frames missing rather than the curve. The loop only ever
/// asks for them while `Chrome::moving` says something is in flight, which is a tenth of a
/// second per control the pointer crosses and nothing at all while it sits still.
const HOVER_FRAME: Duration = Duration::from_millis(16);

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

/// The shells the popover offers, in the order the app would open them.
///
/// A free function over the app for the reason [`find_view`] is one: the strings the
/// popover draws are borrowed from the profile list, and something has to hold them for
/// as long as the `ChromeInput` that points at them. As a method it would borrow the whole
/// host to build a list of `&str` and the frame needs the rest of the host mutably while
/// it draws.
fn profile_names(app: &App) -> Vec<&str> {
    app.profiles()
        .iter()
        .map(|profile| profile.name.as_str())
        .collect()
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
    /// What the chrome said was under the pointer when it last drew.
    ///
    /// A move that lands on the same control is a move that changes nothing anyone can see,
    /// so it is not a frame. Only a move that changes this is: a terminal that repainted for
    /// every motion event would be repainting sixty times a second while the pointer crossed
    /// the grid, which is the idle cost that makes a terminal feel expensive.
    hover_at: Hit,
    /// The cursor currently shown, so it is only set when it changes.
    cursor: CursorIcon,
    /// The cell a left button went down on, so a click with no drag can be told from one
    /// with a drag.
    pressed_at: Option<zet_vt::Pos>,
    /// The mouse button being held, for the motion reports a program asked for.
    ///
    /// A drag report is "the pointer moved with button *this* one down", and the button
    /// is only known here: the platform's move events carry no buttons, and a report
    /// that guessed would tell a program a drag was happening when nothing was held.
    mouse_held: Option<zet_input::MouseButton>,
    /// Whether the window has focus, which decides whether the cursor is hollow.
    focused: bool,
    /// Where on the thumb a drag was grabbed, in logical pixels from the thumb's top.
    ///
    /// Kept so that the thumb does not jump under the pointer on the first move: the
    /// grab point is the place the user took hold of, and it stays under the pointer for
    /// the whole drag, which is what every scrollbar does.
    scroll_grab: Option<f32>,
    /// The last press on the tab row's drag region, and where it landed.
    ///
    /// Read by the next press on that region to tell a double-click from two clicks, which
    /// the platform never does for us: winit registers its window class without
    /// `CS_DBLCLKS`, and a `decorations(false)` window has no non-client area for Windows
    /// to send `WM_NCLBUTTONDBLCLK` about. DESIGN.md's "the drag region is any horizontal
    /// gap between the last tab and the caption buttons. Double-click maximizes" is kept
    /// here or nowhere. Cleared by every press anywhere in the window, because the pair is
    /// two presses on the same few pixels: a click on a tab between them is not part of
    /// one. See [`mouse::double_click`], which is the whole of the rule.
    titlebar_press: Option<(Instant, (f64, f64))>,
    /// Scrolling sub-line remainders, accumulated so a trackpad's fractions are not
    /// dropped one event at a time.
    partial: f64,

    /// When the cursor's blink phase last changed, so the loop can wake for the next one.
    blink_at: Instant,
    /// When the frame a program is holding becomes due, while one is being held.
    ///
    /// Set when output arrives mid-repaint and cleared when the frame is finally asked
    /// for. A held frame is a frame nobody has requested, so the loop has to come back
    /// for it on its own — see [`App::frame_hold`], which is where the instant comes from.
    hold_until: Option<Instant>,
    /// The moment the host started, for the chrome's clock.
    origin: Instant,

    /// What the OS was last told this window is called, so it is only told when it
    /// changes. Every frame would be a `WM_SETTEXT` per frame.
    os_title: String,

    /// The grid face's settings as of the last load, so a change is noticed.
    styled: FontSettings,
    /// The chrome's settings as of the last build, so a change is noticed.
    tabbed: TabSettings,
    /// The file the window's picture was decoded from, as of the last upload.
    ///
    /// A picture is the one part of a background that is not a value a frame can carry:
    /// its pixels live in a texture, and getting them there is a file read and a decode.
    /// Keeping the path is how a redraw tells whether there is anything to upload, and it
    /// is compared rather than the file's timestamp because the configuration is the only
    /// thing that can change it — a picture edited on disk under a running zet is a new
    /// picture the next time zet starts.
    pictured: Option<PathBuf>,
    /// The window's settings as of the last time they were applied to the window.
    ///
    /// The opacity is a Win32 call rather than a value a frame reads, so a change to it
    /// has to be noticed and made rather than simply drawn. Comparing against what was
    /// last applied is what `styled` and `tabbed` do, and for the same reason: it cannot
    /// drift from what the window is actually wearing. The rest of the section is read
    /// where a window is made — a window cannot start maximized halfway through its life —
    /// and is carried here only because the four are written down together.
    windowed: WindowSettings,
    /// Whether the corners were last asked for square, or [`None`] if never asked.
    ///
    /// The same shape as `windowed` and for the same reason, with one difference the
    /// attribute forces: `DWMWA_WINDOW_CORNER_PREFERENCE` has no getter, so unlike the
    /// opacity the window cannot be asked what it is wearing and the caller has to
    /// remember. A `bool` rather than the [`CornerPreference`] itself, because the
    /// maximized state is the input the decision is a function of.
    ///
    /// [`CornerPreference`]: winit::platform::windows::CornerPreference
    corners: Option<bool>,
    /// The layout the previous frame's grid was positioned with.
    placed: Layout,
    /// The grid size the sessions were last told about, so that a frame which did not
    /// change it does not resize every one of them again.
    fitted: (u16, u16),

    /// Where the settings tab is: shut, behind, or on screen.
    ///
    /// The window's state rather than the app's, because a page of settings is something the
    /// window draws and the app is the thing that draws nothing. The app already says as much
    /// where it declines to act on the binding.
    settings_tab: SettingsTab,
    /// How far the settings page is scrolled, in logical pixels from the top of its list.
    ///
    /// The number the caller asks for; the page answers each frame with the number it
    /// actually used, clamped to what overflows, and that answer is what is kept. A
    /// window that grows therefore pulls the list back up on its own instead of leaving
    /// it scrolled past the end of a list that now fits.
    settings_scroll: f32,
    /// Which section of the page to show, by its heading's name.
    ///
    /// `None` is the first section. A name rather than an index because the headings move
    /// under the page: `Problems` is a section only while the configuration has something
    /// wrong with it, so every index after it shifts the moment a setting fixes the last
    /// diagnostic, and a window holding an index would find itself on a different section
    /// without anything having been asked for.
    ///
    /// Kept in step with what the page drew, each frame, the way `settings_scroll` is: the
    /// page resolves the name against the headings it was handed, and this is told which one
    /// that turned out to be.
    settings_section: Option<String>,
    /// The action waiting for the user to press a key, if the page asked for one.
    capturing: Option<Action>,
    /// The settings row the keyboard is on: an index into the rows the app answers with.
    ///
    /// `None` while the page is up but the keyboard has not been asked for, which is
    /// a different state from "no row is focused" in the same way that a window with no
    /// focus is different from a window whose focus is nowhere in particular. The arrow
    /// keys belong to the page until this is set.
    settings_focus: Option<usize>,

    /// Whether the keyboard has been walked off the end of the page's rows.
    ///
    /// `settings_focus` alone cannot say which of the two `None`s it is, and the two
    /// want opposite things from the next `Tab`: a page that has just opened takes it
    /// as a request to enter at the top row, and a page the user has already tabbed
    /// out of has to let it reach the shell or the page is a place you can only leave
    /// by remembering the chord that opened it. Cleared whenever focus is taken again,
    /// by the chord or by a click.
    settings_left: bool,

    /// Whether the window is on its way out.
    ///
    /// Set where the loop is told to stop, and read by [`Host::redraw`], which returns
    /// without drawing while it is true. An event that ends the loop can have asked for a
    /// frame before it reached the end — the close chord does, at the top of [`Host::key`] —
    /// and the last frame a window draws should be a frame with something in it. A strip with
    /// no tabs in it is the state `DESIGN.md` calls unreachable, and this is what keeps it
    /// unreachable now that a window can outlive its last shell.
    closing: bool,
}

/// Where the settings tab is.
///
/// Three states and not two, because "the tab exists" and "the tab is what you are looking at"
/// are different facts and a pair of bools would let one of them lie: a window with the tab open
/// and a terminal on screen is a window where the mark has to stay lit, a click on the tab has
/// to bring the page back, and the close chord has to close the tab rather than the shell the
/// user is not looking at.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum SettingsTab {
    /// No settings tab.
    #[default]
    Shut,
    /// The tab is in the strip and a terminal is the tab on screen.
    Behind,
    /// The tab is in the strip and is the tab on screen.
    Shown,
}

impl SettingsTab {
    /// Whether the settings page is the thing on screen.
    const fn shows(self) -> bool {
        matches!(self, Self::Shown)
    }

    /// Whether the tab exists at all, on screen or not.
    const fn open(self) -> bool {
        !matches!(self, Self::Shut)
    }
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
        // Nothing has been applied to a window yet — there is no window — so the record
        // starts empty rather than claiming the configuration is already in force.
        // `attach` applies it to the window it makes and fills this in.
        let windowed = WindowSettings::default();
        // Nothing has been decoded yet, and `reconcile` uploads whatever the configuration
        // names the first time it runs.
        let pictured = None;
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
            hover_at: Hit::None,
            cursor: CursorIcon::Default,
            pressed_at: None,
            mouse_held: None,
            focused: true,
            scroll_grab: None,
            titlebar_press: None,
            partial: 0.0,
            blink_at: now,
            hold_until: None,
            origin: now,
            os_title: APP_NAME.to_owned(),
            styled,
            tabbed,
            pictured,
            windowed,
            // Nothing has been asked of the compositor yet, and the answer is not one the
            // window can be asked for back, so the first write is unconditional.
            corners: None,
            placed: Layout::default(),
            // What `resumed` opens the first tab at, before any frame has been laid out
            // and therefore before anything knows how big the window really is.
            fitted: EMPTY_GRID,
            settings_tab: SettingsTab::Shut,
            settings_scroll: 0.0,
            settings_section: None,
            capturing: None,
            settings_focus: None,
            settings_left: false,
            closing: false,
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

    /// The screens, as `placement` describes them.
    ///
    /// Asked of the event loop rather than cached: a display can be plugged in or
    /// unplugged between two windows of the same session, and both the question "is this
    /// position still on a screen" and the answer have to be about the screens there are
    /// now.
    fn screens(loop_: &ActiveEventLoop) -> Vec<placement::Screen> {
        loop_
            .available_monitors()
            .map(|monitor| {
                let at = monitor.position();
                let size = monitor.size();
                placement::Screen {
                    at: placement::Position::new(at.x, at.y),
                    // Signed because a position is: the two are added together to ask
                    // whether a point is on a screen. A display two billion pixels wide
                    // would flip the sign here, and there is no such display.
                    size: (size.width.cast_signed(), size.height.cast_signed()),
                }
            })
            .collect()
    }

    /// Where the window is to open, if the user asked for it and it is still a place.
    ///
    /// `None` for every reason at once — the setting off, no file, a file that is not a
    /// position, a position on a monitor that has been unplugged — because they all want
    /// the same thing from the caller: let the window system place the window.
    fn remembered(&self, loop_: &ActiveEventLoop) -> Option<PhysicalPosition<i32>> {
        if !self.app.config().window.remember_position {
            return None;
        }
        let path = placement::default_path()?;
        let position = placement::load(&path)?;
        let position = placement::on_a_screen(position, &Self::screens(loop_))?;
        Some(PhysicalPosition::new(position.x, position.y))
    }

    /// Write down where the window is, for the next time it opens.
    ///
    /// The outer corner rather than the inner one: it is the corner the window system
    /// positions and the one the user drags by, and a window recalled by its inner corner
    /// would drift by the width of a frame every time it was closed.
    ///
    /// A corner that is on no screen is not written, which is the same rule the file is
    /// read under and is what keeps a window closed while minimized from teaching the
    /// next one where the taskbar's own coordinates are.
    fn remember(&self, loop_: &ActiveEventLoop) {
        if !self.app.config().window.remember_position {
            return;
        }
        let Some(window) = self.window() else {
            return;
        };
        let Ok(at) = window.outer_position() else {
            return;
        };
        let Some(path) = placement::default_path() else {
            return;
        };
        let position = placement::Position::new(at.x, at.y);
        if placement::on_a_screen(position, &Self::screens(loop_)).is_some() {
            placement::save(&path, position);
        }
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
        let attributes = Window::default_attributes()
            .with_title(APP_NAME)
            // DESIGN.md draws its own titlebar and caption buttons, and a second set
            // drawn by Windows over them would be two titlebars stacked on each other.
            .with_decorations(false)
            .with_inner_size(OPEN_SIZE)
            .with_min_inner_size(MIN_SIZE)
            // Asked for at creation rather than called afterwards, because a window that
            // is maximized after it opens is a window the user watches jump — once, on
            // the frame they were looking at. The remembered position is set here for
            // the same reason: a window that moves itself is a window that moved.
            .with_maximized(self.app.config().window.start_maximized);
        let attributes = match self.remembered(loop_) {
            Some(position) => attributes.with_position(position),
            None => attributes,
        };

        let window = Arc::new(
            loop_
                .create_window(attributes)
                .map_err(StartupError::Window)?,
        );
        set_opacity(&window, self.app.config().window.opacity);
        self.corners = set_corners(&window, self.corners, window.is_maximized());
        self.windowed = self.app.config().window.clone();

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

        // The picture is the one part of the background that is not a value in a frame:
        // the pixels are a texture, and how they get there is a file read and a decode.
        // Uploaded when the configuration names a different file, and only then — the
        // opacity rides in the frame like every other colour, so turning a picture down
        // does not re-read it.
        let named = match &self.app.config().window.background {
            zet_config::Background::Image { path, .. } => Some(path.clone()),
            _ => None,
        };
        if named != self.pictured {
            // Decoded before the renderer is borrowed, both because it touches the disk
            // and because a picture that will not decode has to reach the device as
            // nothing at all rather than as the last one still up.
            let decoded = named.as_ref().and_then(|path| crate::picture::load(path));
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.set_picture(decoded.as_ref().map(|picture| Picture {
                    width: picture.width,
                    height: picture.height,
                    pixels: &picture.pixels,
                }));
                self.pictured = named;
            }
        }

        // The one thing in the window section that reaches the window: the rest of it is
        // read where a window is made or where the frame is drawn, and the opacity is a
        // call to the platform that has to be made rather than a value to be read.
        if self.app.config().window != self.windowed {
            let settings = self.app.config().window.clone();
            if let Some(window) = self.window() {
                set_opacity(window, settings.opacity);
            }
            self.windowed = settings;
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

    /// Begin a frame: the capacity kept, and what it is painted on settled.
    ///
    /// The colour the surface is cleared to and the backdrop over it are the two answers
    /// to "what is behind everything", and they are set together because the pair can
    /// disagree. `Background::Solid` is the theme's ground, which reaches the frame as the
    /// clear colour and as no backdrop at all, and a caller that took the ground from one
    /// setting and the gradient from another would have a window whose floor and whose
    /// wallpaper were two different decisions.
    fn begin_frame(&mut self, size: (u32, u32)) {
        let ground = self.app.theme().background.to_linear();
        let background = self.app.config().window.background.clone();
        self.frame.reset();
        self.frame.clear = ground;
        self.frame.backdrop = backdrop(&background, size.0 as f32, size.1 as f32);
    }

    /// Draw everything and put it on screen.
    ///
    /// Long by nature rather than by accident, and the length is not the kind a split
    /// fixes: everything that reads the whole host is gathered into locals before the
    /// renderer is borrowed mutably, and the renderer's borrow is then held across the
    /// rest of the frame. An extracted half would have to be handed the renderer, the
    /// frame, the chrome, the input, and the handful of locals the grid is drawn from —
    /// past seven arguments to move one screenful of work one call frame away, which is
    /// the same function with worse names.
    #[allow(clippy::too_many_lines)]
    fn redraw(&mut self) {
        // A frame asked for by the event that ended the loop is not drawn. The close chord
        // asks for one before it hands the command over, so without this the strip would be
        // drawn once with the last tab already gone from it — the state `DESIGN.md` says no
        // user can reach, drawn on the way out.
        if self.closing {
            return;
        }
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
        // The surface's size in physical pixels, and the frame's ground settled with it.
        // Both here rather than below, because the renderer is borrowed mutably for the
        // rest of the frame and neither of these needs it for anything else.
        let Some(size) = self.renderer.as_ref().map(Renderer::size) else {
            return;
        };
        self.begin_frame(size);

        // Everything that reads the whole host is gathered before the renderer is
        // borrowed mutably, and that ordering is load-bearing rather than stylistic: a
        // method call on `self` borrows all of it, and the renderer's borrow is held
        // across the rest of the frame because drawing a frame is what places the glyphs
        // the frame names.
        let scale = window.scale_factor() as f32;
        let (width, height) = self.logical_size();
        let now = Instant::now();
        let tabs = self.tabs();
        // No bar while the settings page is up: the page is the content area, so a scrollbar
        // down its right edge would be a picture of a scrollback that is not on screen and
        // cannot be scrolled. The terminal's own position is untouched — `scroll_state` is not
        // asked, and `self.app` still knows where the viewport is — so coming back to the shell
        // finds it where it was left.
        let scroll = if self.settings_tab.shows() {
            ScrollState::default()
        } else {
            self.scroll_state()
        };
        let blink_on = self.app.blink_on(now);
        let selection = self.app.selection();
        // The tab on screen, which is the settings tab when the page is up and the app's own
        // active shell otherwise. The app's active terminal is not cleared while the page is
        // shown — a shell that is not the tab you are looking at is still a shell that is
        // running — so this is what says which of the two the strip is highlighting.
        let active = if self.settings_tab.shows() {
            Some(TabId::Settings)
        } else {
            self.app.active_number().map(TabId::Terminal)
        };
        let theme = self.app.theme();
        let cursor_settings = self.app.config().cursor.clone();
        let elapsed = now.duration_since(self.origin).as_secs_f32();
        let finding = find_view(&self.app);
        let marks = zet_render::Marks::new(&finding.marks, finding.active);
        // The popover's rows while the app is asking which shell to open, and `None` when
        // it is not: a binding of its own because the `ChromeInput` below borrows it.
        let names = profile_names(&self.app);
        let picker = self.app.picker_at().map(|at| zet_ui::PickerLine {
            profiles: &names,
            at,
        });

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
        let lines = self.settings_tab.open().then(|| self.app.settings());
        let rows = lines
            .as_ref()
            .map_or_else(Vec::new, |lines| panel_lines(lines, self.capturing));
        // Focus is an index into a list the app rebuilds from the file every frame, so a
        // change made from the page can move the row out from under it: choosing a
        // block cursor drops the thickness row, and a highlight on the line below it
        // would silently be a highlight on a different setting. Re-clamped here rather
        // than remembered, because this is the only place that knows what the rows are.
        let focused = lines.as_ref().and_then(|lines| {
            self.settings_focus
                .filter(|line| lines.get(*line).is_some_and(|line| line.kind().is_some()))
        });

        // The menu's words, alive for as long as `ChromeInput` borrows them. The items are
        // rebuilt from the tab count every frame by the same call the chooser answers from,
        // so the row under the pointer and the thing that happens cannot part company.
        let menu_items = self.app.tab_menu_items();
        let menu_words: Vec<&str> = menu_items.iter().map(|item| item.label).collect();
        let menu = self.app.tab_menu().map(|menu| zet_ui::MenuLine {
            items: &menu_words,
            at: menu.at,
        });

        let input = ChromeInput {
            palette: &self.palette,
            tabs: &tabs,
            active,
            settings: &rows,
            settings_scroll: self.settings_scroll,
            settings_focus: focused,
            settings_section: self.settings_section.as_deref(),
            find: finding.line,
            picker,
            menu,
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

        // The grid first, so the chrome lands on top of it. It is positioned with the
        // previous frame's layout — see the module documentation — and the fresh layout
        // is compared against it below.
        //
        // Not at all while the settings page is up, and that is what makes it a page rather
        // than an overlay: a tab is one thing on screen at a time, and a terminal drawn under
        // the page is a frame of work nobody sees. The session keeps running — nothing here
        // stops it reading its pty — and the next frame after the page is closed draws it
        // again where the layout says, which has not moved.
        if let Some(session) = self.app.active().filter(|_| !self.settings_tab.shows()) {
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
        // The page answers with the scroll it actually used rather than the one it was
        // asked for, and that answer is kept: a window that grows then pulls a scrolled
        // list back up on its own, instead of leaving it parked past the end of a list
        // that now fits.
        //
        // Only while it is on screen. A page that is behind a terminal is not drawn, so it
        // answers nothing, and an answer of "the scroll I used" from a page that drew nothing
        // would snap a scrolled list back to the top the moment the user switched away.
        if self.settings_tab.shows() {
            self.settings_scroll = fresh.settings_scroll;
        }
        // The section the page actually drew, which is the answer to the name this window
        // asked for: a name that is no heading in the current list shows the first section,
        // and a window that kept asking for it would ask again on every frame. Keeping the
        // answer is what makes the choice survive the list changing under it — a setting that
        // fixes the last diagnostic takes `Problems` away with it.
        if let Some(line) = fresh.settings_section
            && let Some(name) = heading_name(&self.app.settings(), line)
            && self.settings_section.as_deref() != Some(name)
        {
            self.settings_section = Some(name.to_owned());
        }
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
    ///
    /// The settings page names itself, and takes the place of the shell's name while it is the
    /// tab on screen: a window that is showing settings and calls itself after a shell is a
    /// window that lies about what it is showing. A page that is open behind a terminal does not
    /// reach this, because the tab the user is looking at is the terminal.
    fn os_window_title(&self) -> String {
        let name = if self.settings_tab.shows() {
            SETTINGS_TAB.to_owned()
        } else {
            self.app
                .active()
                .map(zet_session::Session::title)
                .unwrap_or_default()
        };
        if name.is_empty() {
            APP_NAME.to_owned()
        } else {
            format!("{name} — {APP_NAME}")
        }
    }

    /// The tabs, as the strip needs them.
    ///
    /// The settings tab is appended last, and is present whenever it is open — on screen or
    /// behind a terminal. It is a tab that exists, so it is in the strip whether or not it is
    /// the one being looked at, and the close gesture that works on any other cell works on it.
    fn tabs(&self) -> Vec<TabInfo> {
        // The pointer's answer, which counts the × on a cell as the cell.
        let hovered = self
            .pointer
            .and_then(|(x, y)| hovered_tab(self.chrome.hit(x as f32, y as f32)));
        let mut tabs: Vec<TabInfo> = self
            .app
            .tab_numbers()
            .into_iter()
            .map(|number| TabInfo {
                id: TabId::Terminal(number),
                title: self
                    .app
                    .sessions()
                    .get(number)
                    .map(zet_session::Session::title)
                    .unwrap_or_default(),
                hovered: hovered == Some(TabId::Terminal(number)),
            })
            .collect();
        if self.settings_tab.open() {
            tabs.push(TabInfo {
                id: TabId::Settings,
                title: SETTINGS_TAB.to_owned(),
                hovered: hovered == Some(TabId::Settings),
            });
        }
        tabs
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
                Command::OpenUrl(url) => crate::platform::open_url(&url),
                // stderr and not a message box, for the reason a configuration that
                // could not be written goes to stderr: the window is up and working, and
                // a modal box over a terminal for something a user can live with is a
                // worse interruption than the thing it is reporting. A user who launched
                // zet from a shell is the one who can do anything about it.
                Command::Report(message) => eprintln!("zet: {message}"),
                Command::NewWindow => {
                    // A second process rather than a second window in this loop. Two
                    // windows would otherwise share one `App` and therefore one tab
                    // list, and tabs belong to a window: a tab is where the next shell
                    // goes when you press the key, which is a per-window question.
                    if let Ok(executable) = std::env::current_exe() {
                        let _ = std::process::Command::new(executable).spawn();
                    }
                }
                // The user asked for the window to close, and it closes: nothing about the
                // tabs is consulted, because closing a window is not closing a tab and a
                // window that refused because a page was still open would be a quit chord
                // that did not quit.
                Command::Quit => self.finish(loop_),
                // The app's last tab went, which is not the same thing. The settings page is
                // a tab of the window rather than a shell of the app, so the app cannot know
                // whether anything is left in the window — this is the whole of why the
                // command exists rather than the app quitting by itself.
                Command::LastTabClosed => {
                    self.bring_the_page_forward();
                    if self.nothing_left() {
                        self.finish(loop_);
                    }
                }
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

        // The overlays and the settings tab belong to the window, which is where they are drawn,
        // so their keys are read here rather than left to the app. Everything about that is in
        // `page_key`; what this line is for is that a key it takes goes no further.
        if self.page_key(&translated) {
            // The page can be the last thing in the window: `Escape` and the close chord both
            // close it, and a window with no shells in it and no page is a window with nothing
            // left to show. Asked here rather than inside `page_key`, which is a rule about keys
            // — it answers true for an arrow key that moved a highlight, and only one of the
            // things it takes can empty the window.
            if self.nothing_left() {
                self.finish(loop_);
            }
            return;
        }

        // Which shell is active, before the key runs, so that a chord which switches tabs can be
        // noticed afterwards. The whole of the next/previous/activate-tab family moves the app
        // to a shell, and a page that stayed on screen while the app believed a terminal was in
        // front would be a page with a strip that highlighted nothing in it.
        let ran_an_action = self.app.action_for(&translated).is_some();
        let was_active = self.app.active_number();
        let commands = self.app.key(&translated);
        if self.settings_tab.shows() && self.app.active_number() != was_active {
            self.settings_tab = SettingsTab::Behind;
        }
        if ran_an_action && let Some(window) = self.window() {
            window.request_redraw();
        }
        self.carry_out(loop_, commands);
    }

    /// The settings tab's claim on a translated key, and whether it took it.
    ///
    /// Split out of [`Self::key`] so that the rules can be asserted without a window: this is
    /// the whole of what the page does with a key, and every one of the branches is a decision
    /// about which of the two things on the other side of this call — the app's keymap and the
    /// shell behind the shell behind the page — is entitled to hear it. Taking it means it
    /// reached neither.
    ///
    /// The order is the order the user's things are stacked in, and it is load-bearing:
    ///
    /// - the settings chord, which is what opened the tab and is what puts it away;
    /// - the close chord, which the user aimed at what is on screen — a page is not a shell, so
    ///   it closes the page and not the shell behind it, which is alive and unmentioned;
    /// - the pane's own keys, each of which is a thing the user is looking at;
    /// - `Escape`, the way out that does not depend on remembering the chord;
    /// - and finally the swallow: an unbound key does not reach a shell that is not on screen.
    ///   A chord does, because a chord is the user asking zet itself for something.
    fn page_key(&mut self, translated: &KeyEvent) -> bool {
        // Through `action_for` rather than `bound`, because a chord is not an event: asking the
        // keymap directly would toggle the page open on the press and shut again on the release,
        // which is one keystroke the user cannot see the effect of.
        if self.app.action_for(translated) == Some(Action::Settings) {
            self.toggle_settings();
            return true;
        }

        // Only on screen. A page behind a terminal is not the tab being looked at, so the chord
        // there means what it always meant, and closes the shell in front of the user.
        if self.settings_tab.shows() && self.app.action_for(translated) == Some(Action::CloseTab) {
            self.close_settings();
            self.redraw();
            return true;
        }

        // An open menu's one key. Before the picker, because a menu is the thing the user opened
        // most recently and the thing drawn over everything else.
        if self.menu_key(translated) {
            self.redraw();
            return true;
        }

        // The profile picker's four keys, while it is asking. Before the find bar, because a
        // question about a new tab is the thing on top and the thing the user has just opened,
        // and a `Down` that scrolled a find result instead of moving the highlight would be the
        // window answering the wrong question.
        if self.app.picker_key(translated) {
            self.redraw();
            return true;
        }

        // The find bar's keys, and it has more of them than the page does: it is a text field,
        // so what was typed is its own rather than the shell's. That is the one place in this
        // window where a letter does not reach the terminal, and it is the whole point of a find
        // bar — a query you cannot type is not a query. Before the page, because a letter belongs
        // to whichever of the two is asking for text, and only one of them ever is.
        if self.app.find_key(translated) {
            self.redraw();
            return true;
        }

        // The page's own keys, which it only has while it is the tab on screen and only for the
        // ones it names.
        if self.settings_tab.shows() && self.panel_key(translated) {
            self.redraw();
            return true;
        }

        // DESIGN.md: the settings page never traps focus, and a way out that does not depend on
        // remembering the chord you opened it with is the whole of that promise. Bare `Escape`
        // only — a modified one is somebody else's key, and the shell may well be waiting for it.
        //
        // A press only. The key going up is not a second Escape: without this the documented
        // two-stage Escape is defeated from the first press, because the release closes a page
        // the press had already left open.
        if self.settings_tab.shows()
            && translated.kind != KeyKind::Release
            && translated.key == Key::Escape
            && translated.mods.is_empty()
        {
            self.close_settings();
            self.redraw();
            return true;
        }

        // Everything else, and the one place this window departs from the old panel's rule. The
        // panel let an unbound key through to the shell because the shell was on screen beside
        // it, and "it does not steal focus from the prompt" meant exactly that. A page has no
        // prompt behind it: the terminal is a different tab, and a letter typed at a settings
        // page that reached it would be a letter typed into a shell the user cannot see, with
        // the answer arriving on a screen that is not being shown.
        self.settings_tab.shows() && !self.app.owns(translated)
    }

    /// A key aimed at an open context menu, and whether the menu took it.
    ///
    /// `Escape`, and nothing else. Every action a menu offers is on a chord — that is what
    /// PRODUCT.md's "fully keyboard-operable" is paid for with, and it is why a menu is a
    /// mouse affordance rather than a keyboard one — so the only key the menu itself needs
    /// is the one that puts it away. Arrow keys are deliberately not named here: they
    /// reach the shell, where they are history and cursor movement, and a menu that took
    /// them would be a menu that made the terminal behind it stop working while it was up.
    fn menu_key(&mut self, event: &KeyEvent) -> bool {
        if event.kind == KeyKind::Release
            || event.key != Key::Escape
            || !event.mods.is_empty()
            || self.app.tab_menu().is_none()
        {
            return false;
        }
        self.app.close_tab_menu();
        true
    }

    /// A key aimed at the settings page, and whether the page took it.
    ///
    /// The page's focus is entered and left with `Tab`, exactly as focus is traversed
    /// everywhere else, and `Tab` past the last row walks the focus off the end. That is what
    /// "it never traps focus" has to mean: a page that kept `Tab` until you remembered the chord
    /// you opened it with would be a page you can get stuck in. Once the focus has been walked
    /// off, nothing the page names is swallowed at all — the next `Tab` is not a second chance to
    /// walk the rows, and with a page on screen it is swallowed rather than typed, as every other
    /// unbound key is.
    ///
    /// Only bare `Tab`, the four arrows, `Enter`, `Space` and `Escape` are named here. Everything
    /// else — every letter, and every chord with a modifier on it — is not the page's, and where
    /// it goes after that is the caller's business rather than this function's.
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

    /// Show the section a heading names, and give the keyboard back.
    ///
    /// A method of its own rather than four lines inside the press arm, because a press is the
    /// one thing about the rail the window cannot be asked to do without an HWND: `chrome_press`
    /// answers "not mine" for everything until there is a window to draw into and a layout to
    /// hit-test against, and this is the part that is worth a test.
    ///
    /// The keyboard goes back where it was, which is what "a click on a name, not on a row"
    /// means: there is nowhere in the new page for the old highlight to be, and the panel's rule
    /// is that it holds the arrow keys only when it has been asked to.
    fn show_section(&mut self, line: usize) -> bool {
        let lines = self.app.settings();
        let Some(name) = heading_name(&lines, line) else {
            return false;
        };
        self.settings_section = Some(name.to_owned());
        self.settings_scroll = 0.0;
        self.settings_focus = None;
        self.settings_left = false;
        true
    }

    /// Move the panel's keyboard focus one row, or off the end of the list.
    fn step_focus(&mut self, rows: &[usize], here: Option<usize>, forward: bool) {
        let next = next_focus(rows.len(), here, forward);
        // Landing on nothing is the panel handing the keyboard back, and it stays handed
        // back until something takes it again.
        self.settings_left = next.is_none();
        self.settings_focus = next.map(|at| rows[at]);
        if let Some(line) = self.settings_focus {
            self.follow_section(line);
        }
    }

    /// Show the section the keyboard has walked into.
    ///
    /// The rows are one list and the sections are drawn one at a time, so walking off the end
    /// of a section lands on the first row of the next one — which is a row that is not on
    /// screen unless the page moves with it. It moves here, and the scroll goes back to the
    /// top with it: the position the old page was scrolled to is a position in a list that is
    /// no longer being drawn.
    ///
    /// Which section a row is in is read from the headings above it, because that is exactly
    /// what a section is, and the alternative — a second index kept in step with the first —
    /// is a second thing to be wrong.
    fn follow_section(&mut self, line: usize) {
        let lines = self.app.settings();
        let Some(name) = lines
            .iter()
            .take(line + 1)
            .rev()
            .find_map(|line| match line {
                zet_app::Line::Heading(name) => Some(*name),
                _ => None,
            })
        else {
            return;
        };
        if self.settings_section.as_deref() == Some(name) {
            return;
        }
        self.settings_section = Some(name.to_owned());
        self.settings_scroll = 0.0;
    }

    /// Open the settings tab, bring it forward, or close it.
    ///
    /// The window's state rather than the app's, for the reason the field is: a page of settings
    /// is something the window draws and the app draws nothing. One function rather than three
    /// call sites, because the chord and the strip's own mark are two ways to ask for one thing —
    /// a mark that opened the page and a chord that toggled it would leave a user pressing the
    /// mark again to get rid of it and being handed a second page.
    ///
    /// Three states in, three states out:
    ///
    /// - shut opens it, on screen, at the top of its list;
    /// - behind a terminal brings it forward, and keeps the scroll and the section it was left on,
    ///   because nothing has changed about the page — the user is coming back to where they were;
    /// - on screen closes it, and the terminal that is then the tab on screen is the app's own
    ///   active one, which has been left alone the whole time.
    fn toggle_settings(&mut self) {
        match self.settings_tab {
            SettingsTab::Shut => {
                self.settings_tab = SettingsTab::Shown;
                // A page that opened with the last session's highlight still on a row would take
                // the arrow keys the moment it appeared, and one that opened scrolled to where a
                // list it is no longer showing was left would open in the middle.
                self.settings_scroll = 0.0;
                self.capturing = None;
                self.settings_focus = None;
                self.settings_left = false;
            }
            SettingsTab::Behind => self.settings_tab = SettingsTab::Shown,
            SettingsTab::Shown => self.close_settings(),
        }
        self.redraw();
    }

    /// Close the settings tab, and let go of everything it was holding.
    ///
    /// Nothing is left of the tab: it is shut, so the mark goes back to `ink-mid` and the tab is
    /// gone from the strip. The state it was left in — which section, how far down — goes with it,
    /// because a tab that is gone has no scroll to come back to.
    ///
    /// The shell behind it is not touched. It was never closed and it was never resized; it is a
    /// tab that stopped being the one on screen, and closing the page makes it the one on screen
    /// again.
    fn close_settings(&mut self) {
        self.settings_tab = SettingsTab::Shut;
        self.settings_scroll = 0.0;
        self.settings_focus = None;
        self.settings_left = false;
        self.capturing = None;
    }

    /// Nothing is left in the window: no terminal, and no settings tab either.
    ///
    /// The one question the window's ending is a function of. Closing the last tab used to be
    /// closing the window, which was true while every tab was a shell: `close-tab` on the last
    /// one ended the process, and a window with no terminals in it was a window already on its
    /// way out. The settings tab is a tab that outlives the last shell, so the window does too,
    /// and what the ending asks is not "is there a tab" but "is there anything at all".
    ///
    /// A predicate rather than a decision, because the four doors that can empty the window are
    /// not the same code: three of them close a shell and one of them closes the page.
    fn nothing_left(&self) -> bool {
        self.app.sessions().is_empty() && !self.settings_tab.open()
    }

    /// Bring the settings page to the front when it is now the only tab in the window.
    ///
    /// `Behind` means "the tab is in the strip and a terminal is the tab on screen", so a page
    /// behind the last shell is a page the user cannot see on a window that is still in front of
    /// them — and the strip it is in says it is open. The last terminal closing hands the screen
    /// to the page rather than leaving the user looking at a frame of a terminal that no longer
    /// exists.
    ///
    /// Nothing happens while a terminal is left, which is why the doors that close a tab can
    /// call it unconditionally: it is the answer to "what is on screen now that one has gone",
    /// and the answer is usually "the shell that was already there".
    fn bring_the_page_forward(&mut self) {
        if self.app.sessions().is_empty() && self.settings_tab.open() {
            self.settings_tab = SettingsTab::Shown;
            self.redraw();
        }
    }

    /// Close the tab a gesture on the strip was aimed at.
    ///
    /// The × on a cell and a middle click on one are the same act, and they are the two
    /// gestures this window has ever offered for one: close *that* tab, which is not always the
    /// tab on screen — a user aiming at a tab's × has aimed at the tab, not at what is in front
    /// of them — and then say what is on screen now that it has gone.
    ///
    /// A settings cell closes the page instead of a shell, because that is what its tab is. The
    /// shell behind the page is not the tab the user closed and is not touched.
    ///
    /// Whether the window has anything left is not answered here but asked at each door, because
    /// the doors are of two kinds: this one closes a tab the user pointed at, and the chord
    /// closes the tab on screen. Both end in the same question and it is the same question.
    fn close_from_strip(&mut self, id: TabId) {
        match id {
            TabId::Terminal(number) => {
                let _ = self.app.close_tab(number);
            }
            TabId::Settings => self.close_settings(),
        }
        self.bring_the_page_forward();
    }

    /// End the window, and stop drawing it.
    ///
    /// Every way out of the loop goes through here rather than through `loop_.exit()`, because
    /// telling the loop to stop and telling the window not to draw again are one decision: the
    /// redraw that [`Host::redraw`] refuses is one asked for by the very event that ended the
    /// loop, and a window that drew it would draw the strip with nothing in it.
    fn finish(&mut self, loop_: &ActiveEventLoop) {
        self.closing = true;
        loop_.exit();
    }

    /// Write the configuration back, and say so when it could not be.
    ///
    /// A terminal that silently discards a setting the user just chose is worse than one
    /// that never offered it, so a failure is reported on stderr where a user who
    /// launched zet from a shell will see it. It is not fatal: the change is live either
    /// way, and losing it at exit is a smaller loss than losing the window.
    ///
    /// The write is the app's rather than this file's because the app is where the
    /// Problems section lives, and a save is what makes those rows expire — a host that
    /// wrote the file itself would leave the panel describing a file that no longer
    /// exists.
    fn saved(&mut self) {
        if let Err(error) = self.app.save() {
            eprintln!(
                "zet: could not write {}: {error}",
                self.app.config_path().display()
            );
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
        // And the double-click pair, which is not a hold but is the same kind of loose end: a
        // press on the drag region, a pointer taken out of the window, and a press back in
        // within the interval and the slop would maximize a window on two presses with a
        // journey between them. The window losing focus is the other way in here, and it is
        // the same argument.
        self.titlebar_press = None;
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
        // The chrome's hover is the one thing a plain move can change, and it is a transition
        // rather than a state, so a move onto a control owes a frame and a move within one
        // does not. Asked of the chrome rather than worked out here, because what is under the
        // pointer is the chrome's own answer and a second copy of it here is a second copy
        // that can be wrong.
        let at = self.chrome.hit(x as f32, y as f32);
        let hovered_moved = at != self.hover_at;
        self.hover_at = at;
        // Sent before any of the drags below, and before the early return the scrollbar
        // drag takes: the pointer is over the grid or it is not, and that question has
        // nothing to do with what this window happens to be doing with the drag.
        self.forward_motion();
        let Some(window) = self.window.clone() else {
            return;
        };
        if hovered_moved {
            window.request_redraw();
        }

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
        // Remembered for the moves that follow, which say nothing about buttons: a drag
        // report is "the pointer moved with this button down", and a wheel is never
        // held, so a wheel press reports the move as one with nothing held.
        match (state, mouse::button(button)) {
            (ElementState::Pressed, held) if held != zet_input::MouseButton::None => {
                self.mouse_held = Some(held);
            }
            // Only the button that is held ends the hold: another one coming up while
            // this one is still down is not the end of a drag, and reporting it as one
            // would tell the program the pointer had been let go when it had not.
            (ElementState::Released, held) if self.mouse_held == Some(held) => {
                self.mouse_held = None;
            }
            _ => {}
        }

        if state == ElementState::Pressed {
            // Taken here, before any of the paths below can decide anything, because every
            // press is the reason the pair from the last one no longer counts: a press on a
            // caption button, on the window's own edge, on a tab, or in the terminal is not
            // the second half of a double-click on the drag region, and only the arm that
            // answers a press on the drag region puts one back.
            let titlebar = self.titlebar_press.take();
            let (width, height) = self.logical_size();
            // The window's own edge wins over everything else: a user reaching for the
            // border is not aiming at a tab that happens to be under it.
            if let Some(edge) = mouse::edge(x, y, width, height) {
                let _ = window.drag_resize_window(edge.direction());
                return;
            }
            match button {
                WinitButton::Left => {
                    // A click on an open menu is the menu's, wherever it lands: one that
                    // takes an item does that and nothing else, and one anywhere else is
                    // the click that puts the menu away. That is what every menu on the
                    // system does, and it is the reason a menu can be opened over the
                    // thing it is about without the first click doing two things at once.
                    if self.app.tab_menu().is_some() {
                        if let Hit::MenuItem(row) = self.chrome.hit(x as f32, y as f32) {
                            let commands = self.app.tab_menu_choose(row);
                            self.carry_out(loop_, commands);
                        } else {
                            self.app.close_tab_menu();
                        }
                        window.request_redraw();
                        return;
                    }
                    if self.chrome_press(x, y, titlebar, loop_) {
                        // Every control the chrome has that can end the window ends it the same
                        // way: the × on a tab, the settings mark, and the settings mark again
                        // when it closes the page while no shell is left. Asked here rather than
                        // in each arm, because the arms do not know what is left in the window
                        // and this is the caller that has just changed it.
                        if self.nothing_left() {
                            self.finish(loop_);
                        }
                        return;
                    }
                    if let Some(at) = self.grid_cell(x, y) {
                        // Ctrl+click follows the link under the pointer instead of
                        // starting a selection, which is what every terminal on this
                        // platform does and the only reason the modifier is read here at
                        // all. It is a deliberate gesture on purpose: a program chooses
                        // the URL, and a plain click on a link that opened it would hand
                        // a program the ability to open a page under the user's hand.
                        if self.held.control_key() {
                            let commands = self.app.open_link_at(at);
                            if !commands.is_empty() {
                                self.carry_out(loop_, commands);
                                window.request_redraw();
                                return;
                            }
                        }
                        self.pressed_at = Some(at);
                        self.app.select_from(at);
                        // A press starts a selection where a previous one was, so the
                        // thing on screen has changed even though the drag has not
                        // started yet.
                        window.request_redraw();
                    }
                }
                WinitButton::Right => {
                    // The tab strip's context menu, and only on a tab: a right-click
                    // anywhere else is the program's, which is what a program that asks
                    // for mouse reporting expects. An open menu anywhere goes first, so
                    // that a right-click is one of the things that dismisses it.
                    if self.app.tab_menu().is_some() {
                        self.app.close_tab_menu();
                        window.request_redraw();
                        return;
                    }
                    // The tab strip's menu, on a shell and on nothing else. The settings tab
                    // has no menu to open: every item a tab's menu offers is about a shell — its
                    // splits, its name, closing it — and a page that has one name and three ways
                    // to close it already has a menu with nothing in it. A right click there is
                    // the click that dismisses an open menu and is otherwise nothing at all.
                    if let Hit::Tab(TabId::Terminal(number)) = self.chrome.hit(x as f32, y as f32) {
                        self.app.open_tab_menu(number, (x as f32, y as f32));
                        window.request_redraw();
                        return;
                    }
                }
                WinitButton::Middle => {
                    // The chord and the × are the strip's two deliberate ways to close a tab;
                    // this is the one a user arrives at by habit, because it is what every other
                    // terminal on this platform does. Both cells answer it, and the cell's × is
                    // one of the things it lands on: the mark sits inside its own cell, so a
                    // middle click on it is a middle click on the tab.
                    if let Hit::Tab(id) | Hit::CloseTab(id) = self.chrome.hit(x as f32, y as f32) {
                        self.close_from_strip(id);
                        // Closing the last tab is closing the window, unless the settings page is
                        // open — a page is not a shell, and a shell's departure is not the
                        // window's. Dropping this left a window with no terminal in it: the shell
                        // was gone, the last frame was still painted, and nothing was listening
                        // for the exit that ends the loop.
                        if self.nothing_left() {
                            self.finish(loop_);
                        } else {
                            window.request_redraw();
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

    /// A press on a cell of the strip, and whether the cell took it.
    ///
    /// The two kinds of cell do different things to the page rather than the same thing to
    /// different tabs. A press on a terminal brings that shell forward and leaves the page where
    /// it is: still in the strip, still open, with its mark still lit — simply not the one being
    /// looked at. A press on the settings cell is the page asking to be looked at, which is the
    /// mark's own function and the chord's.
    fn tab_press(&mut self, id: TabId) -> bool {
        match id {
            TabId::Terminal(number) => {
                self.settings_tab = if self.settings_tab.open() {
                    SettingsTab::Behind
                } else {
                    SettingsTab::Shut
                };
                self.app.activate(number);
            }
            TabId::Settings => self.settings_tab = SettingsTab::Shown,
        }
        true
    }

    /// Hand a press to the chrome, and report whether it was the chrome's.
    ///
    /// `titlebar` is the last press on the strip's drag region, taken off the host before
    /// this was called, and it is handed back only by the arm that answers a press on that
    /// region — so the host is left holding a pair exactly when the two presses are two
    /// presses on the thing that pairs them.
    fn chrome_press(
        &mut self,
        x: f64,
        y: f64,
        titlebar: Option<(Instant, (f64, f64))>,
        loop_: &ActiveEventLoop,
    ) -> bool {
        let Some(window) = self.window.clone() else {
            return false;
        };
        let handled = match self.chrome.hit(x as f32, y as f32) {
            Hit::Tab(id) => self.tab_press(id),
            Hit::NewTab => {
                let (cols, rows) = self.grid_size();
                let commands = self.app.new_tab(cols.max(1), rows.max(1));
                // A new tab is a tab to look at, so it takes the screen from the page and the
                // page goes behind — still open, still in the strip, with its mark still lit.
                // The alternative is a mark that opens a tab you cannot see.
                if self.settings_tab.shows() {
                    self.settings_tab = SettingsTab::Behind;
                }
                self.carry_out(loop_, commands);
                true
            }
            Hit::SettingsButton => {
                // The chord's own function, because a button and a binding that did different
                // things would be one of them wrong.
                self.toggle_settings();
                true
            }
            Hit::Section(line) => self.show_section(line),
            Hit::Caption(caption) => {
                match caption {
                    Caption::Minimize => window.set_minimized(true),
                    Caption::Maximize => window.set_maximized(!window.is_maximized()),
                    // Closing the window, not the tab. A caption button that closed one
                    // terminal and left the window up would be a button that lies about
                    // what it does; `close-tab` is the key for that.
                    Caption::Close => self.finish(loop_),
                }
                true
            }
            Hit::Drag => {
                // The drag region, which is where DESIGN.md puts the promise: the gap
                // between the last thing the strip drew and the caption buttons maximizes
                // on a double-click. The interval and the slop are Windows' own, handed in
                // rather than read inside the test, so the rule stays a function of its
                // arguments and `mouse::double_click` stays unit-testable.
                let at = (x, y);
                let now = Instant::now();
                if mouse::double_click(
                    titlebar,
                    now,
                    at,
                    mouse::DOUBLE_CLICK,
                    mouse::DOUBLE_CLICK_SLOP,
                ) {
                    // A maximized window restores, which is what the same gesture does to
                    // every other window on the platform, and what a user who maximized by
                    // accident will reach for.
                    window.set_maximized(!window.is_maximized());
                } else {
                    // Nothing drags on the press that completed the gesture: the window has
                    // just changed size under the pointer, and a drag asked for from a
                    // point that only exists in the new layout drops the window where the
                    // user never aimed.
                    self.titlebar_press = Some((now, at));
                    let _ = window.drag_window();
                }
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
            Hit::CloseTab(id) => {
                // The mark is only published while it is drawn, so the user has aimed at it. It
                // closes its own tab rather than the tab on screen, and a settings cell closes
                // the page — the shell behind a page is not the tab the user pointed at.
                self.close_from_strip(id);
                true
            }
            Hit::Profile(row) => {
                // The chrome only draws a row while the picker is asking, so this
                // normally means exactly what it says. The exception is the frame in
                // which the picker has just closed and the redraw has not happened yet:
                // the popover is still on screen and the app is no longer asking, and a
                // click at that spot is a click on a terminal. Opening a tab there would
                // be acting on a question that has already been answered, so the press
                // falls through to the grid, which is what is under it now.
                if !self.app.picker_is_open() {
                    return false;
                }
                let commands = Command::unopened(self.app.picker_choose_at(row));
                self.carry_out(loop_, commands);
                true
            }
            // The popover's surface, between and around its rows. Swallowed for the same
            // reason the panel's is — a click on what is drawn is not a click on what it
            // covers — and passed through on the same stale frame.
            Hit::Picker => self.app.picker_is_open(),
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
            // The page's own surface, between and around its controls. Swallowed rather than
            // passed on: the page is what is on screen, and a click on a surface is a click on
            // the surface rather than on whatever the last frame drew behind it.
            //
            // A menu row and a menu's own surface are swallowed for the same reason and are
            // not reachable from here anyway: the press that lands on a menu is answered
            // before this is ever asked, in the left-button arm, because a menu takes the
            // whole click rather than sharing it with what it covers. The arms are here
            // because a `Hit` has to be answered somewhere, and answering "the menu's" with
            // "not the chrome's" would hand a click on a menu to the terminal underneath it.
            Hit::Settings | Hit::MenuItem(_) | Hit::Menu => true,
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
    ///
    /// Nothing at all while the settings page is up, and this is the one gate the mouse needs.
    /// The page occupies the grid's rectangle, so every point that would resolve to a cell — a
    /// press that starts a selection, a ctrl-click that follows a link, a hover report, a wheel
    /// — is over the page instead, and the answer "that is a cell of the terminal" would be a
    /// point the user is not pointing at. The shell keeps its own state while it is off screen:
    /// the viewport is where it was, the selection is what it was, and reporting resumes the
    /// frame the terminal is the tab on screen again.
    fn grid_cell(&self, x: f64, y: f64) -> Option<zet_vt::Pos> {
        if self.settings_tab.shows() {
            return None;
        }
        let renderer = self.renderer.as_ref()?;
        mouse::cell(x, y, self.placed.grid, renderer.metrics(), self.scale())
    }

    /// Whether a mouse event is the shell's to hear.
    ///
    /// A program in reporting mode asks for the mouse by its cell coordinates, and the page is not
    /// the shell: a click at a coordinate on the settings page is a click at a coordinate on a
    /// screen the program is not on, and the report would be a lie the program then acts on. So
    /// nothing is sent while the page is what the user is looking at.
    ///
    /// `Behind` is the other half of the rule and the reason this is asked of the state rather
    /// than of a flag: a page behind a terminal is not on screen at all, the shell in front of it
    /// is, and it gets its reports exactly as it did before the page was ever opened.
    const fn mouse_reaches_the_shell(&self) -> bool {
        !self.settings_tab.shows()
    }

    /// Send a button event to the program, if it asked for mouse reporting.
    fn forward_mouse(&self, state: ElementState, button: WinitButton) {
        self.send_mouse(mouse::button(button), mouse::action(state));
    }

    /// Tell the program the window gained or lost focus, if it asked to be told.
    ///
    /// `DECSET 1004` is the mode, and what it buys a full-screen program is the only way
    /// it has of knowing: without it a text editor keeps blinking its cursor and drawing
    /// its own status line into a window the user has walked away from, and a program
    /// that repaints on a timer keeps repainting. Only the focused tab is told, because
    /// only the focused tab's program has the keyboard.
    fn forward_focus(&self, focused: bool) {
        let Some(session) = self.app.active() else {
            return;
        };
        if let Some(bytes) = encode_focus(focused, &session.term().modes()) {
            let _ = session.write(&bytes);
        }
    }

    /// Tell the program the pointer moved, if it asked to be told.
    ///
    /// `DECSET 1002` wants a report while a button is held and `1003` wants every move,
    /// and which of those applies is the mode's business rather than this function's:
    /// the event goes out and the encoder answers `None` for a program that asked for
    /// neither. Both modes were confirmed to programs that then received nothing —
    /// only the press at one end of a drag and the release at the other — so a hover
    /// highlight never followed the pointer and a drag was drawn by the program as a
    /// jump with nothing in between.
    fn forward_motion(&self) {
        self.send_mouse(
            self.mouse_held.unwrap_or(zet_input::MouseButton::None),
            zet_input::MouseAction::Motion,
        );
    }

    /// One mouse report, or nothing.
    ///
    /// A point outside the grid is not a report at all: the chrome's own surfaces —
    /// the tab strip, the scrollbar, the settings page — are drawn over the grid and
    /// are not the terminal's to report on, and a program told about a move over the
    /// tab strip would be told the pointer was inside the text it is drawing.
    fn send_mouse(&self, button: zet_input::MouseButton, action: zet_input::MouseAction) {
        if !self.mouse_reaches_the_shell() {
            return;
        }
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
            button,
            action,
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
    ///
    /// Answers whether the window is owed a frame. Nothing draws on its own: the event
    /// loop draws when something asks it to, and a wheel that scrolled the view without
    /// asking is a scroll the user does not see until the shell next prints — which on an
    /// idle prompt is never. A wheel the program wants is the one case that owes nothing,
    /// because the program is about to print and that is what wakes the loop.
    fn wheel(&mut self, delta: MouseScrollDelta) -> bool {
        let cell = self
            .renderer
            .as_ref()
            .map_or(0.0, |r| f64::from(r.metrics().cell_height));
        let lines = mouse::wheel(delta, cell / f64::from(self.scale()));
        if lines == 0.0 {
            return false;
        }

        // The page is a list that can be longer than the window, and a list with no wheel is a
        // list whose last rows are unreachable on a laptop with no End key. The pointer decides
        // where it lands, so that the same gesture over the strip still belongs to the strip.
        if self.settings_tab.shows() && self.pointer.is_some_and(|(x, y)| self.over_page(x, y)) {
            self.settings_scroll = (self.settings_scroll - lines as f32 * PANEL_WHEEL).max(0.0);
            return true;
        }

        if self.wheel_to_program(lines) {
            return false;
        }

        // A terminal has no sub-line scrolling, so a trackpad's fraction of a line is
        // accumulated rather than dropped: at sixty events a second, each worth a tenth
        // of a line, dropping the remainder is a scroll that never happens at all.
        self.partial += lines;
        let whole = self.partial.trunc();
        if whole == 0.0 {
            return false;
        }
        self.partial -= whole;
        let Ok(step) = i32::try_from(whole as i64) else {
            return false;
        };
        if let Some(session) = self.app.active_mut() {
            session.scroll(step);
        }
        true
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

    /// Whether a point is inside the settings page.
    ///
    /// Asked of the chrome rather than worked out from the content rectangle, because the
    /// rectangle is the chrome's business and a second copy of it here is a second place for
    /// it to be wrong. The chrome hit-tests what it actually drew, which is also what
    /// the user is looking at.
    ///
    /// The strip's own settings control counts: the wheel over it belongs to the window's
    /// controls rather than to the terminal, and a wheel there that scrolled the shell would be
    /// the one pixel of the strip that scrolled something else. The page's own surface is the
    /// whole content rectangle, so every point below the strip that is not a control of it is
    /// still the page's to answer for.
    fn over_page(&self, x: f64, y: f64) -> bool {
        matches!(
            self.chrome.hit(x as f32, y as f32),
            Hit::Settings | Hit::Setting { .. } | Hit::SettingsButton | Hit::Section(_)
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
        // Every route to maximizing and back ends here, because they all produce a
        // `WM_SIZE` and winit updates its stored flag before dispatching: the caption
        // button, `Win+Up` and `Win+Down`, and the Aero Snap a titlebar drag gets from
        // the system. Asking the window rather than tracking the gesture is what keeps
        // this from being a second answer to a question Windows already answered.
        self.corners = set_corners(&window, self.corners, window.is_maximized());
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

    /// Ask for a frame, unless the program that just wrote is mid-repaint.
    ///
    /// Output arriving is what a redraw is for, and this is the one case where it is not:
    /// a program that has set `DECSET 2026` is telling the terminal that what it has
    /// written so far is half a picture. Drawing it would show the user the half — the
    /// cleared screen, the rows filled in as far as the program has got — and the write
    /// that ends the repaint would arrive a moment later and draw the whole thing, so the
    /// flicker that synchronization exists to remove is exactly what honoring the marker
    /// only half way would produce.
    ///
    /// The frame is postponed rather than dropped. [`App::frame_hold`] hands back the
    /// instant the hold lapses and `about_to_wait` wakes the loop for it, which is what
    /// keeps this from being a freeze: the program that will never send the write that
    /// ends the repaint is the program the budget is there for.
    ///
    /// Only this path is gated, and deliberately. A redraw asked for by a keystroke, a
    /// resize or the blink is the user's own business and not the program's, so a
    /// terminal that sat on those for the length of somebody else's repaint would be
    /// dropping the events it is most obliged to answer.
    fn draw_when_due(&mut self) {
        self.hold_until = self.app.frame_hold(Instant::now());
        if self.hold_until.is_none()
            && let Some(window) = self.window()
        {
            window.request_redraw();
        }
    }
}

/// The name of the section a line begins, if that line is a heading.
///
/// What a section is called, asked two ways: the panel answers with the line it drew and the
/// window turns that back into the name it holds, and a click on the rail arrives as a line and
/// has to become a name. Both go through here so that "a section is a heading" is written once,
/// which is the same reason the panel derives its rail from the headings rather than being
/// told about them.
fn heading_name(lines: &[zet_app::Line], line: usize) -> Option<&str> {
    match lines.get(line)? {
        zet_app::Line::Heading(name) => Some(name),
        _ => None,
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
                row: match line {
                    zet_app::Line::Heading(_) => zet_ui::Row::Heading,
                    zet_app::Line::Note(_) | zet_app::Line::Report { .. } => zet_ui::Row::Note,
                    zet_app::Line::Setting(setting) => {
                        zet_ui::Row::Control(control_of(setting.kind))
                    }
                },
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

/// Which tab the pointer is hovering, given what it hit.
///
/// A pointer on a cell's × is a pointer on that cell. The mark is a control inside the tab it
/// closes, so the two answers are one answer, and a strip that took the pointer off the tab the
/// moment it reached the tab's own × would fade the mark out from under the user as they went
/// for it.
///
/// A free function rather than a match inside `Host::tabs`, because that is what makes the rule
/// assertable: `tabs` reads a layout the chrome only produces with a window behind it.
fn hovered_tab(hit: Hit) -> Option<TabId> {
    match hit {
        Hit::Tab(id) | Hit::CloseTab(id) => Some(id),
        _ => None,
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
    /// The one place every way out of the loop passes through.
    ///
    /// `loop_.exit()` is called from [`Host::finish`] and nowhere else — the quit action, the
    /// caption's close button, the last tab's shell exiting, the window's own close request,
    /// and a failed start all go through it — and a position written down at each of them
    /// would be a position written down at four of them the day a sixth is added. This is
    /// winit's own last call, and it happens after the loop has stopped rather than during the
    /// event that stopped it, which is also the first moment the window's position is
    /// certainly its final one.
    fn exiting(&mut self, loop_: &ActiveEventLoop) {
        self.remember(loop_);
    }

    fn resumed(&mut self, loop_: &ActiveEventLoop) {
        // `resumed` fires again after a suspend on the platforms that have them, and a
        // second window is not what that means.
        if self.window.is_some() {
            return;
        }
        if let Err(error) = self.attach(loop_) {
            startup_failed(&error);
            self.finish(loop_);
            return;
        }
        // A terminal that opens with no terminal in it is a rectangle. The first tab is
        // opened here rather than by the app because the app cannot know how big the
        // window is until the window exists.
        let (cols, rows) = self.grid_size();
        if let Err(error) = self.app.open_tab(cols.max(1), rows.max(1)) {
            startup_failed(&error.into());
            self.finish(loop_);
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
            // A row that lists the families reads them, and a page that is not on screen is
            // not drawing that row: the frame is owed only while it is.
            if self.settings_tab.shows()
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

        // A tab whose shell exited is closed by the app, and the last one closing is what ends
        // the window — the same decision the close chord carries, made in the same place, for a
        // tab the user never touched. The settings page, again, is not a shell: a program
        // exiting is not the window's decision to make while there is still a page in it.
        let closed = self.app.reap();
        if closed.is_empty() {
            self.draw_when_due();
            return;
        }
        self.bring_the_page_forward();
        if self.nothing_left() {
            self.finish(loop_);
        } else {
            self.draw_when_due();
        }
    }

    fn window_event(&mut self, loop_: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.finish(loop_),
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::Resized(_) => self.resized(),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scaled(scale_factor as f32);
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                self.forward_focus(focused);
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
                // A pointer that has left the window is over nothing, so whatever was lit
                // fades out — and a frame is what makes that visible rather than a control
                // that stays lit for as long as nothing else happens to want one.
                if self.hover_at != Hit::None {
                    self.hover_at = Hit::None;
                    if let Some(window) = self.window() {
                        window.request_redraw();
                    }
                }
                // Same reason as losing focus, and it is the commoner case: a drag
                // taken off the edge of the window is a drag whose release happens
                // somewhere this window is not listening.
                self.end_drags();
            }
            WindowEvent::MouseInput { state, button, .. } => self.button(state, button, loop_),
            WindowEvent::MouseWheel { delta, .. } => {
                if self.wheel(delta)
                    && let Some(window) = self.window()
                {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, loop_: &ActiveEventLoop) {
        let now = Instant::now();
        // A frame a program is holding has nobody left to ask for it. The output that
        // would have drawn it has already been drained and deliberately not drawn, and a
        // program that is still repainting will write again and wake the loop by itself —
        // but the program that set the marker and stopped never will, so this deadline is
        // the loop's own appointment with it. It is a moment rather than an event, which
        // is why it has to be kept here, where the control flow is decided.
        //
        // The hold is cleared before the redraw is asked for, so that the frame which
        // follows is drawn rather than postponed a second time by a deadline that has
        // already passed.
        if let Some(due) = self.hold_until {
            if now < due {
                loop_.set_control_flow(ControlFlow::WaitUntil(due));
                return;
            }
            self.hold_until = None;
            if let Some(window) = self.window() {
                window.request_redraw();
            }
        }

        // A hover in flight is the other thing in zet that happens on a clock, and it is the
        // only one of them the *chrome* knows about: a pointer that has just arrived on a
        // control is a fade that needs the frames to run, and no event will arrive to ask for
        // them. This is the whole of the frame clock — while the chrome says something is
        // moving, the loop comes back for another frame, and once it has landed the deadline
        // is gone and the window is as idle as it was before.
        let mut due = if self.chrome.moving() {
            if let Some(window) = self.window() {
                window.request_redraw();
            }
            Some(now + HOVER_FRAME)
        } else {
            None
        };

        // The cursor's blink is the other thing in zet that happens on a clock, so the
        // loop is only ever woken for it when there is a cursor to blink. A terminal
        // sitting idle with nothing to flash — and with nothing fading — parks in `Wait`
        // and uses no power at all.
        if self.app.blinks() {
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
            due = Some(due.map_or(self.blink_at, |hover| hover.min(self.blink_at)));
        }

        match due {
            Some(at) => loop_.set_control_flow(ControlFlow::WaitUntil(at)),
            None => loop_.set_control_flow(ControlFlow::Wait),
        }
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
        host.settings_tab = SettingsTab::Shown;
        let tab = KeyEvent {
            key: Key::Tab,
            mods: Modifiers::empty(),
            text: None,
            kind: KeyKind::Press,
            base: None,
            unshifted: None,
        };
        let down = KeyEvent {
            key: Key::Down,
            mods: Modifiers::empty(),
            text: None,
            kind: KeyKind::Press,
            base: None,
            unshifted: None,
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
        host.settings_tab = SettingsTab::Shown;
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
            base: None,
            unshifted: None,
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
    fn a_wheel_over_the_grid_asks_for_the_frame_that_shows_what_it_moved() {
        // The viewport is read when a frame is drawn and nothing asks for one on its own:
        // an idle terminal has no output to wake the loop, and the blink comes round
        // twice a second. A wheel that scrolled the view and asked for no frame is a
        // scroll nobody sees until the shell next prints.
        let mut host = Host::new(app());
        let _ = host.app.open_tab(80, 24).expect("this machine has a shell");

        assert!(
            host.wheel(MouseScrollDelta::LineDelta(0.0, 3.0)),
            "the view moved, so a frame is owed"
        );
        // And a wheel with nothing in it is not a frame's worth of work.
        assert!(!host.wheel(MouseScrollDelta::LineDelta(0.0, 0.0)));
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

    /// A key press, which is the only kind of event the panel's own keys are.
    fn press(key: Key) -> KeyEvent {
        KeyEvent {
            key,
            mods: Modifiers::empty(),
            text: None,
            kind: KeyKind::Press,
            base: None,
            unshifted: None,
        }
    }

    /// A chord pressed, which is what the app's keymap answers to.
    fn chord(mods: Modifiers, key: Key) -> KeyEvent {
        KeyEvent { mods, ..press(key) }
    }

    #[test]
    fn the_settings_tab_opens_shows_and_closes() {
        // The three states, from the one control that moves between them. `Behind` is not a
        // fourth case to handle: it is what a terminal on top of the page leaves, and bringing the
        // page back has to be the same thing as opening it — except that where the user had got to
        // in the list is still there, because nothing about the page changed while they were away.
        let mut host = Host::new(app());
        assert_eq!(
            host.settings_tab,
            SettingsTab::Shut,
            "a fresh host has no tab"
        );

        host.toggle_settings();
        assert_eq!(
            host.settings_tab,
            SettingsTab::Shown,
            "the mark did not open the tab"
        );

        host.settings_scroll = 40.0;
        host.settings_focus = Some(3);
        host.settings_left = true;
        host.toggle_settings();
        assert_eq!(
            host.settings_tab,
            SettingsTab::Shut,
            "the mark did not close the tab"
        );
        assert_eq!(host.settings_scroll, 0.0, "the closed tab kept its scroll");
        assert_eq!(
            host.settings_focus, None,
            "the closed tab kept its highlight"
        );
        assert!(
            !host.settings_left,
            "the closed tab kept the focus it had walked off"
        );

        // A page behind a terminal comes forward where it was left, and the tab that put it there
        // is the one the user is on.
        let behind = SettingsTab::Behind;
        host.settings_tab = behind;
        host.settings_scroll = 40.0;
        host.settings_focus = Some(3);
        host.toggle_settings();
        assert_eq!(host.settings_tab, SettingsTab::Shown);
        assert_eq!(
            host.settings_scroll, 40.0,
            "the page came back at the top of its list"
        );
        assert_eq!(
            host.settings_focus,
            Some(3),
            "the page came back with no highlight"
        );

        // And opening from shut starts at the top however the last page was left.
        host.settings_tab = SettingsTab::Shut;
        host.settings_scroll = 40.0;
        host.settings_focus = Some(3);
        host.toggle_settings();
        assert_eq!(host.settings_scroll, 0.0);
        assert_eq!(host.settings_focus, None);
    }

    #[test]
    fn the_settings_follows_the_active_tab() {
        // The tab exists while the tab is open, whether or not it is the one on screen, and the
        // strip is told about it either way: the cell is in the run and the mark is lit, which is
        // what makes the page a tab rather than a mode the window is in.
        let mut host = Host::new(app());
        let terminals = host.tabs();
        assert!(
            !terminals
                .iter()
                .any(|tab| tab.id == zet_ui::TabId::Settings),
            "a shut tab is in the strip"
        );

        host.settings_tab = SettingsTab::Shown;
        let shown = host.tabs();
        assert_eq!(
            shown.len(),
            terminals.len() + 1,
            "the tab is not in the strip"
        );
        assert_eq!(
            shown.last().map(|tab| tab.id),
            Some(zet_ui::TabId::Settings),
            "the settings tab is not the last cell"
        );
        assert!(host.settings_tab.shows());

        // Behind a terminal: still a tab, not the one on screen.
        host.settings_tab = SettingsTab::Behind;
        let behind = host.tabs();
        assert_eq!(
            behind.len(),
            terminals.len() + 1,
            "the page behind lost its tab"
        );
        assert!(!host.settings_tab.shows(), "a page behind is on screen");

        host.close_settings();
        assert_eq!(
            host.tabs().len(),
            terminals.len(),
            "the closed tab is still there"
        );
    }

    #[test]
    fn the_close_chord_closes_the_settings_tab_and_not_the_shell_behind_it() {
        // `close-tab` means "close the tab I am looking at", and while the page is up that tab is
        // the page. The app's active tab is still the shell it was, because a shell that is not on
        // screen is still running — so a chord that reached the app would kill a shell the user
        // cannot see and leave the page up.
        let mut host = Host::new(app());
        // A host with no window has opened no shell: `Host::new` gets the app ready and the first
        // frame opens the tab. A test about a shell that survives the page has to have one.
        let _ = host
            .app
            .open_tab(EMPTY_GRID.0, EMPTY_GRID.1)
            .expect("a shell to leave alone");
        let shells = host.app.tab_numbers();
        assert!(
            !shells.is_empty(),
            "the host opened no shell to leave alone"
        );

        host.settings_tab = SettingsTab::Shown;
        let chord = chord(Modifiers::CTRL | Modifiers::SHIFT, Key::Char('W'));
        assert_eq!(
            host.app.action_for(&chord),
            Some(Action::CloseTab),
            "the chord under test is not the close chord"
        );
        assert!(host.page_key(&chord), "the close chord was not the page's");
        assert_eq!(
            host.settings_tab,
            SettingsTab::Shut,
            "the page did not close"
        );
        assert_eq!(
            host.app.tab_numbers(),
            shells,
            "the shell behind the page was closed with it"
        );

        // And with no page, the same chord is the app's, which is where it closes the shell in
        // front of the user.
        assert!(!host.page_key(&chord), "a shut page kept the close chord");
    }

    #[test]
    fn a_key_with_no_binding_does_not_reach_the_shell_while_the_page_is_up() {
        // The one place this window departs from the panel's rule. The panel let an unbound key
        // through because the shell was on screen beside it; a page has no prompt behind it, so a
        // letter typed there would reach a shell the user cannot see and the answer would appear on
        // a screen that is not being shown. A chord still goes through: a chord is the user asking
        // zet itself for something.
        let mut host = Host::new(app());
        let letter = press(Key::Char('a'));
        let new_tab = chord(Modifiers::CTRL | Modifiers::SHIFT, Key::Char('T'));
        assert!(
            !host.app.owns(&letter),
            "the letter under test is bound to something"
        );
        assert!(
            host.app.owns(&new_tab),
            "the chord under test is bound to nothing"
        );

        // With no page, both are the app's to decide, and the letter reaches the shell.
        assert!(!host.page_key(&letter), "a shut page swallowed a letter");
        assert!(!host.page_key(&new_tab), "a shut page swallowed a chord");

        host.settings_tab = SettingsTab::Shown;
        assert!(
            host.page_key(&letter),
            "a letter reached the shell behind the page"
        );
        assert!(
            !host.page_key(&new_tab),
            "the page swallowed a chord the user aimed at zet"
        );

        // Behind a terminal, the page is not what the user is looking at and the shell is: the
        // letter is the shell's again.
        host.settings_tab = SettingsTab::Behind;
        assert!(
            !host.page_key(&letter),
            "a page behind a terminal swallowed the shell's typing"
        );
    }

    #[test]
    fn the_mouse_is_not_forwarded_to_a_shell_that_is_not_on_screen() {
        // A program in reporting mode is told about the mouse by cell coordinates, and the page is
        // not the shell: a report from a click at a coordinate on the settings page would be a lie
        // the program acts on. `Behind` is the other half — the shell in front of it is what the
        // user is looking at, and it gets its reports as it did before the page was opened.
        let mut host = Host::new(app());
        assert!(
            host.mouse_reaches_the_shell(),
            "a host with no page kept the mouse"
        );

        host.settings_tab = SettingsTab::Shown;
        assert!(
            !host.mouse_reaches_the_shell(),
            "the mouse reached a shell that is not on screen"
        );

        host.settings_tab = SettingsTab::Behind;
        assert!(
            host.mouse_reaches_the_shell(),
            "a page behind a terminal took the shell's mouse reports"
        );

        host.close_settings();
        assert!(
            host.mouse_reaches_the_shell(),
            "a closed page kept the mouse"
        );
    }

    #[test]
    fn a_window_that_outlives_its_last_shell_is_a_window_with_a_page_in_it() {
        // The whole of the change, in three lines. Closing the last shell used to be closing the
        // window, which was the same thing while every tab was a shell. The settings page is a tab
        // of the window rather than a shell of the app, so the two came apart: the question the
        // ending asks is not "is there a tab" but "is there anything at all".
        let mut host = Host::new(app());
        let _ = host.app.open_tab(80, 24).expect("this machine has a shell");
        host.settings_tab = SettingsTab::Shown;

        let _ = host.app.close_tab(1);
        assert!(host.app.sessions().is_empty(), "the shell did not go");
        assert!(
            !host.nothing_left(),
            "the last terminal ended a window that still had a page in it"
        );
    }

    #[test]
    fn the_page_comes_to_the_front_when_the_last_terminal_goes() {
        // `Behind` means "a terminal is the tab on screen", so this is the state a terminal's
        // departure can leave behind without the user opening anything — a shell exiting is the
        // door nobody asked for, and the close chord is the door someone did. Both end in these
        // two calls. A page behind a terminal that is gone is a page the user cannot see on a
        // window that is still in front of them.
        let mut host = Host::new(app());
        let _ = host.app.open_tab(80, 24).expect("this machine has a shell");
        host.settings_tab = SettingsTab::Behind;

        let _ = host.app.close_tab(1);
        host.bring_the_page_forward();
        assert_eq!(
            host.settings_tab,
            SettingsTab::Shown,
            "the page stayed behind a terminal that is gone"
        );
        assert!(!host.nothing_left(), "the page went with the shell");
    }

    #[test]
    fn closing_the_last_terminal_with_no_page_ends_the_window() {
        // The ending the app always had, which is still the ending: with no page there is
        // nothing to hold the window open, and this is the one state that used to be the only
        // thing a last tab could do.
        let mut host = Host::new(app());
        let _ = host.app.open_tab(80, 24).expect("this machine has a shell");
        assert_eq!(host.settings_tab, SettingsTab::Shut);

        let _ = host.app.close_tab(1);
        assert!(
            host.nothing_left(),
            "a window with no shells and no page stayed up"
        );
    }

    #[test]
    fn closing_the_page_with_no_terminals_ends_the_window() {
        // The same ending from the other side, and the one that makes the settings tab a tab
        // rather than a way out of the rule: a page with no shell behind it is the only thing
        // left in the window, and closing it is closing the window. What the user is left with
        // otherwise is a strip with nothing in it, which is the state DESIGN.md says nobody can
        // reach.
        let mut host = Host::new(app());
        let _ = host.app.open_tab(80, 24).expect("this machine has a shell");
        host.settings_tab = SettingsTab::Shown;
        let _ = host.app.close_tab(1);

        host.close_from_strip(TabId::Settings);
        assert_eq!(host.settings_tab, SettingsTab::Shut, "the page did not go");
        assert!(
            host.nothing_left(),
            "the window outlived the last tab it had"
        );
    }

    #[test]
    fn closing_the_page_with_a_terminal_behind_it_leaves_the_window() {
        // The other answer to the same two acts, and the reason the ending is a question about
        // both kinds of tab rather than about the page: closing the page of a window with a
        // shell in it is closing one tab, and the shell behind it was never touched.
        let mut host = Host::new(app());
        let only = host.app.open_tab(80, 24).expect("this machine has a shell");
        host.settings_tab = SettingsTab::Shown;

        host.close_from_strip(TabId::Settings);
        assert_eq!(host.settings_tab, SettingsTab::Shut, "the page did not go");
        assert!(
            host.app.sessions().get(only).is_some(),
            "the shell behind the page went with it"
        );
        assert!(!host.nothing_left(), "a window with a shell in it ended");
    }

    #[test]
    fn the_strip_closes_the_tab_it_is_given_and_not_the_active_one() {
        // What the × and the middle click both mean, and it is the whole reason they are one
        // method: a user aiming at a tab's × has aimed at *that* tab, which is not always the tab
        // on screen. Closing the tab on screen instead would kill the shell the user was looking
        // at and leave the one they pointed at.
        let mut host = Host::new(app());
        let first = host.app.open_tab(80, 24).expect("this machine has a shell");
        let second = host.app.open_tab(80, 24).expect("this machine has a shell");
        assert_ne!(first, second, "two tabs are two numbers");
        host.app.activate(first);

        host.close_from_strip(TabId::Terminal(second));
        assert!(
            host.app.sessions().get(second).is_none(),
            "the tab that was aimed at is still open"
        );
        assert!(
            host.app.sessions().get(first).is_some(),
            "the active tab was closed instead of the one that was aimed at"
        );
        assert!(
            !host.nothing_left(),
            "closing one of two tabs ended the window"
        );
    }

    #[test]
    fn the_strip_closes_the_page_when_the_page_is_the_tab() {
        // The settings cell's × is the page's own way out, which is the same function the mark
        // and the chord go through — and the shell behind the page is not the tab that was
        // pointed at, so it is a shell that was never touched.
        let mut host = Host::new(app());
        let only = host.app.open_tab(80, 24).expect("this machine has a shell");
        host.settings_tab = SettingsTab::Shown;

        host.close_from_strip(TabId::Settings);
        assert_eq!(
            host.settings_tab,
            SettingsTab::Shut,
            "the page survived its own mark"
        );
        assert!(
            host.app.sessions().get(only).is_some(),
            "the shell behind the page was closed with it"
        );
    }

    #[test]
    fn a_pointer_on_a_close_mark_hovers_the_tab_it_closes() {
        // One hit answers for both, because the mark is a control inside the cell it closes. A
        // strip that took the pointer off the tab the moment it reached the tab's × would fade
        // the mark out from under the user as they went for it, and the preview bar under the
        // tab would go out with it.
        assert_eq!(
            hovered_tab(Hit::Tab(TabId::Terminal(2))),
            Some(TabId::Terminal(2))
        );
        assert_eq!(
            hovered_tab(Hit::CloseTab(TabId::Terminal(2))),
            Some(TabId::Terminal(2)),
            "a pointer on a mark was not a pointer on its tab"
        );
        assert_eq!(
            hovered_tab(Hit::CloseTab(TabId::Settings)),
            Some(TabId::Settings),
            "the settings cell's mark is the settings cell"
        );
        assert_eq!(hovered_tab(Hit::Drag), None);
        assert_eq!(hovered_tab(Hit::None), None);
    }

    #[test]
    fn the_panels_sections_are_the_apps_own_headings() {
        // The seam the rail stands on. `zet-ui` derives its sections from the heading rows it
        // is handed and knows nothing else about them, so a conversion that dropped them — or
        // turned them into something else on the way past — would leave a panel with no rail
        // at all and nothing anywhere to say why. These are the app's own names.
        let app = app();
        let rows = app.settings();
        let lines = panel_lines(&rows, None);
        let headings: Vec<&str> = lines
            .iter()
            .filter(|line| line.row == zet_ui::Row::Heading)
            .map(|line| line.text)
            .collect();
        assert_eq!(headings, ["Appearance", "Tabs", "Terminal", "Keys"]);
    }

    #[test]
    fn walking_past_a_sections_last_row_brings_the_next_one_to_the_page() {
        // The sections follow the keyboard rather than being a place the keyboard can be:
        // the rows are one list, and the row a `Tab` lands on has to be a row on screen. The
        // scroll goes back to the top with the page, because the position the old page was
        // scrolled to is a position in a list that is no longer being drawn.
        let mut host = Host::new(app());
        host.settings_tab = SettingsTab::Shown;
        let lines = host.app.settings();
        let next = lines
            .iter()
            .position(|line| matches!(line, zet_app::Line::Heading("Tabs")))
            .expect("the panel has a Tabs section");
        let last = (0..next)
            .rev()
            .find(|line| lines[*line].kind().is_some())
            .expect("the section before it has rows");
        host.settings_focus = Some(last);
        host.settings_scroll = 40.0;

        assert!(
            host.panel_key(&press(Key::Tab)),
            "a Tab on a row is the panel's"
        );
        assert_eq!(
            host.settings_section.as_deref(),
            Some("Tabs"),
            "the page did not follow the keyboard into the next section"
        );
        assert!(
            host.settings_scroll.abs() < f32::EPSILON,
            "the new page opened part-way down"
        );
        assert!(
            host.settings_focus.is_some_and(|line| line > last),
            "the focus did not move past the row it was on"
        );
    }

    #[test]
    fn clicking_a_section_shows_it_and_gives_the_keyboard_back() {
        // A click on the rail is a click on a name, not on a row: there is nowhere in the new
        // page for the old highlight to be, and the panel's rule is that it holds the keyboard
        // only when it has been asked to.
        let mut host = Host::new(app());
        host.settings_tab = SettingsTab::Shown;
        let lines = host.app.settings();
        let keys = lines
            .iter()
            .position(|line| matches!(line, zet_app::Line::Heading("Keys")))
            .expect("the panel has a Keys section");
        host.settings_focus = Some(0);
        host.settings_scroll = 40.0;

        assert!(
            host.show_section(keys),
            "a click on a heading names a section"
        );
        assert_eq!(host.settings_section.as_deref(), Some("Keys"));
        assert!(
            host.settings_focus.is_none(),
            "the keyboard was left on a row that is not on the new page"
        );
        assert!(host.settings_scroll.abs() < f32::EPSILON);
        assert!(!host.settings_left, "the next Tab enters the page again");

        // And a line that is not a heading names nothing.
        assert!(
            !host.show_section(keys + 1),
            "the row under a heading is not one"
        );
    }
}
