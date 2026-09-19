//! The terminal as a state machine, with no window attached.
//!
//! Everything a terminal does between a key press and a repaint lives here: which tabs
//! are open, which one is active, what the user has selected, where the view is
//! scrolled, and whether the cursor is in the lit half of its blink. None of it needs a
//! window, a GPU, or a font, which is the point — this is the part of the program with
//! the most decisions in it and it is the part that can be tested by hand.
//!
//! What is *not* here is anything the platform owns. The clipboard, a second window, the
//! event loop's exit, and the update check are all things this crate cannot do, so it
//! says it wants them with a [`Command`] and the host carries them out. The one rule
//! that keeps that honest: nothing in this file may assume a command has happened.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use zet_config::{
    Config, Diagnostic, Severity, Theme, by_slug, contrast_theme, default_path, load,
};
use zet_input::{Chord, Key, KeyEvent, KeyKind, Modifiers, encode_key, encode_paste};
use zet_pty::discovery::{self, Profile};
use zet_render::Selection;
use zet_session::{Session, SessionError, Sessions, Waker};
use zet_vt::search::Match;
use zet_vt::{Modes, Pos};

use crate::action::Action;
use crate::find::Find;
use crate::hold::Hold;
use crate::picker::Picker;
use crate::settings;
use crate::text::selection_text;

/// How long each half of a cursor blink lasts.
///
/// DESIGN.md fixes it and it is not a setting: the number is the one that has been the
/// same since hardware terminals, and a slider for it would be a slider for how annoying
/// the cursor is.
pub const BLINK_INTERVAL: Duration = Duration::from_millis(530);

/// How many lines one page of scrolling covers when the host has not said.
const PAGE: i32 = 20;

/// Something the app needs the window to do.
///
/// The app has already decided *what*; these are the parts only the host can carry out,
/// because they need a platform, a second window, or the event loop itself.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Command {
    /// Put this text on the system clipboard.
    Copy(String),
    /// Read the clipboard and hand it back through [`App::paste`].
    Paste,
    /// Open a second window at the same size.
    NewWindow,
    /// Open this URL in whatever the system opens URLs with.
    OpenUrl(String),
    /// Tell the user this, because nothing else can.
    Report(String),
    /// The last tab closed. The host decides whether that closes the window.
    Quit,
}

impl Command {
    /// What a tab that did not open has to say, as the command that says it.
    ///
    /// A constructor rather than a print at each call site, because a shell that will
    /// not start has three doors into it — the `+` in the strip, the chord, and a row of
    /// the profile picker — and this is the half of the program with no console to print
    /// to. The failure's whole symptom otherwise is that nothing happened, which is the
    /// one symptom a user cannot tell from a keystroke that missed.
    #[must_use]
    pub fn unopened(result: Result<u32, AppError>) -> Vec<Self> {
        match result {
            Ok(_) => Vec::new(),
            Err(error) => vec![Command::Report(error.to_string())],
        }
    }
}

/// Something went wrong that the user has to be told about.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// No shell could be started.
    #[error("no shell could be started: {0}")]
    NoShell(String),

    /// The session could not be created.
    #[error(transparent)]
    Session(#[from] SessionError),

    /// No shell was found on this machine at all.
    #[error("no shell was found on this machine")]
    NoProfiles,
}

/// The cursor's blink, as a phase rather than a timer.
///
/// A timer would be a second source of truth about the time. This is derived from one
/// instant, so a frame that arrives late is late by the same amount the blink is, and
/// nothing drifts.
#[derive(Clone, Copy, Debug)]
struct Blink {
    epoch: Instant,
    phase_shifted: bool,
}

impl Blink {
    fn new(now: Instant) -> Self {
        Blink {
            epoch: now,
            phase_shifted: false,
        }
    }

    fn on(&self, now: Instant) -> bool {
        let elapsed = now.duration_since(self.epoch);
        let half = (elapsed.as_millis() / BLINK_INTERVAL.as_millis()).is_multiple_of(2);
        half != self.phase_shifted
    }

    /// Restart the cycle, so that a cursor which has just moved is lit rather than
    /// arriving mid-blink.
    fn restart(&mut self, now: Instant, shift: bool) {
        self.epoch = now;
        self.phase_shifted = shift;
    }
}

/// A selection being dragged out.
#[derive(Clone, Copy, Debug)]
struct Drag {
    anchor: Pos,
    head: Pos,
}

/// One terminal's worth of state, and as many terminals as are open.
pub struct App {
    config: Config,
    config_path: PathBuf,
    diagnostics: Vec<Diagnostic>,
    theme: &'static Theme,
    profiles: Vec<Profile>,
    sessions: Sessions,
    bindings: Vec<(Chord, Action)>,
    drag: Option<Drag>,
    waker: Arc<dyn Waker>,
    blink: Blink,
    /// A contrast theme was forced by the system, which outranks the user's choice.
    forced_contrast: bool,
    /// The system has asked for as little movement as possible, which stops the blink.
    reduce_motion: bool,
    /// What the system last reported about accessibility, before the configuration had
    /// its say: whether motion should be reduced, and whether colours are forced.
    ///
    /// Kept rather than folded in because both answers are re-gated every time the
    /// configuration changes. A user who turns "Reduce motion" off in the panel is
    /// changing their mind about a system setting zet already knows, and re-reading the
    /// system to find that out again would be a second source of truth about it.
    reported: (bool, bool),
    /// How much the font size has been nudged from the configured one, in points.
    font_nudge: f32,
    /// The directory every tab in this window starts in, if the command line named one.
    start_directory: Option<PathBuf>,
    /// Every family the settings panel may offer for the grid, as the host found them.
    ///
    /// Empty until the host has looked, which is why the panel's font row falls back to
    /// the family the file names: a row offering one choice is a row that says the truth
    /// about the file and nothing about the machine.
    families: Vec<String>,
    /// The find bar: what was typed, what it found, and which find it is on.
    find: Find,
    /// The profile picker: whether a new tab is asking which shell to open.
    pick: Picker,
    /// The hold a program has asked for on the frame while it repaints.
    hold: Hold,
}

impl App {
    /// Build an app around an already-loaded configuration.
    ///
    /// Takes the config rather than reading it so that a test can drive a whole app
    /// without a file on disk, which is the difference between testing the state machine
    /// and testing the file loader for the second time.
    ///
    /// # Errors
    ///
    /// [`AppError::NoProfiles`] when nothing on this machine can be run as a shell.
    pub fn new(
        config: Config,
        config_path: PathBuf,
        diagnostics: Vec<Diagnostic>,
        waker: Arc<dyn Waker>,
    ) -> Result<Self, AppError> {
        let profiles = discovery::discover();
        if profiles.is_empty() {
            return Err(AppError::NoProfiles);
        }
        let theme = by_slug(&config.theme).unwrap_or_else(|| zet_config::default_theme());
        let bindings = parse_bindings(&config);
        let diagnostics = diagnostics_for(&config, diagnostics);
        let now = Instant::now();
        Ok(App {
            config,
            config_path,
            diagnostics,
            theme,
            profiles,
            sessions: Sessions::new(),
            bindings,
            drag: None,
            waker,
            blink: Blink::new(now),
            forced_contrast: false,
            reduce_motion: false,
            reported: (false, false),
            font_nudge: 0.0,
            start_directory: None,
            families: Vec::new(),
            find: Find::default(),
            pick: Picker::default(),
            hold: Hold::default(),
        })
    }

    /// Start every tab this window opens in `directory`.
    ///
    /// A property of the window rather than of its first tab, because the window is what
    /// the directory was chosen for: zet launched from "Open zet here" belongs to that
    /// folder, and a new tab in it that opened somewhere else would be the surprising
    /// thing rather than the consistent one.
    ///
    /// Nothing is checked here. A directory that does not exist fails when the first
    /// shell is started, which is where the error can name the program that would not
    /// start as well as the place it was told to start in.
    pub fn set_start_directory(&mut self, directory: Option<PathBuf>) {
        self.start_directory = directory;
    }

    /// Read the config from its usual place and build an app around it.
    ///
    /// Never fails on a bad file. A terminal that refuses to start because a config file
    /// has a typo is a terminal the user cannot use to fix the typo, and
    /// [`zet_config::load`] already repairs what it can and reports the rest.
    ///
    /// # Errors
    ///
    /// [`AppError::NoProfiles`] when nothing on this machine can be run as a shell.
    pub fn load(waker: Arc<dyn Waker>) -> Result<Self, AppError> {
        let Ok(path) = default_path() else {
            return Self::new(
                Config::default(),
                PathBuf::from("config.toml"),
                Vec::new(),
                waker,
            );
        };
        match load(&path) {
            Ok(loaded) => Self::new(loaded.config, loaded.path, loaded.diagnostics, waker),
            // A file that cannot be read at all is the one case the loader cannot repair
            // — a directory where the file should be, a permission the process does not
            // have. Starting on the defaults is still better than not starting.
            Err(error) => Self::new(
                Config::default(),
                path,
                vec![Diagnostic {
                    severity: Severity::Error,
                    message: error.to_string(),
                }],
                waker,
            ),
        }
    }

    /// The active configuration.
    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// Where the configuration was read from, and will be written back to.
    #[must_use]
    pub fn config_path(&self) -> &std::path::Path {
        &self.config_path
    }

    /// What was wrong with the configuration file, if anything.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// The colours the grid is drawn in.
    #[must_use]
    pub const fn theme(&self) -> &'static Theme {
        self.theme
    }

    /// The shells this machine can open a tab with, in the order the picker shows them.
    #[must_use]
    pub fn profiles(&self) -> &[Profile] {
        &self.profiles
    }

    /// Every open session.
    #[must_use]
    pub const fn sessions(&self) -> &Sessions {
        &self.sessions
    }

    /// The tab numbers, in the order the strip shows them: `1..=N`, with no gaps.
    #[must_use]
    pub fn tab_numbers(&self) -> Vec<u32> {
        self.sessions.numbers()
    }

    /// The tab the keyboard goes to.
    #[must_use]
    pub fn active_number(&self) -> Option<u32> {
        self.sessions.active()
    }

    /// The active terminal.
    #[must_use]
    pub fn active(&self) -> Option<&Session> {
        self.sessions.active().and_then(|n| self.sessions.get(n))
    }

    /// The active terminal, mutably.
    pub fn active_mut(&mut self) -> Option<&mut Session> {
        let number = self.sessions.active()?;
        self.sessions.get_mut(number)
    }

    /// Whether the cursor should be drawn lit this frame.
    #[must_use]
    pub fn blink_on(&self, now: Instant) -> bool {
        self.blink.on(now)
    }

    /// When the blink next changes, so the host can wake exactly then.
    #[must_use]
    pub fn next_blink(&self, now: Instant) -> Instant {
        let elapsed = now.duration_since(self.blink.epoch);
        let half = BLINK_INTERVAL.as_millis();
        let spent = Duration::from_millis((elapsed.as_millis() % half) as u64);
        let left = BLINK_INTERVAL.saturating_sub(spent);
        now + left
    }

    /// Whether the cursor should blink at all, which reduce-motion turns off.
    #[must_use]
    pub fn blinks(&self) -> bool {
        self.config.cursor.blink && !self.reduce_motion()
    }

    /// Whether the system has asked for as little movement as possible.
    #[must_use]
    pub const fn reduce_motion(&self) -> bool {
        self.reduce_motion
    }

    /// What the user has selected, if anything.
    #[must_use]
    pub fn selection(&self) -> Option<Selection> {
        self.drag.map(|drag| Selection::new(drag.anchor, drag.head))
    }

    /// How much the font size has been nudged with the keyboard, in points.
    #[must_use]
    pub const fn font_nudge(&self) -> f32 {
        self.font_nudge
    }

    /// The size the grid font should be drawn at, including any keyboard nudge.
    ///
    /// The nudge is applied before the range is, so a nudge can never leave the range and
    /// the size the panel shows is the size the grid is drawn at.
    #[must_use]
    pub fn font_size(&self) -> f32 {
        zet_config::clamp_or(
            self.config.font.size + self.font_nudge,
            settings::MIN_SIZE,
            settings::MAX_SIZE,
            zet_config::FontSettings::default().size,
        )
    }

    // ---------------------------------------------------------------------------
    // Settings
    // ---------------------------------------------------------------------------

    /// Tell the app which families the machine has, so the font row can offer them.
    ///
    /// The host owns this because the app depends on no font library and rasterises
    /// nothing; enumerating what is installed is a question for the layer that already
    /// reads font files.
    pub fn set_families(&mut self, families: Vec<String>) {
        self.families = families;
    }

    /// The rows the settings panel draws.
    ///
    /// What the loader found wrong with the file comes first, and the settings follow it.
    /// First because it is the only section about the file rather than in it, and a
    /// problem the user has to scroll past forty settings to find is a problem they will
    /// not find — which, until this, was the whole of how a `cursor.thickness = 99` was
    /// reported.
    #[must_use]
    pub fn settings(&self) -> Vec<settings::Line> {
        let mut lines = settings::problems(&self.diagnostics);
        lines.extend(settings::lines(&self.config, &self.bindings));
        lines
    }

    /// Carry out a click on a settings row.
    ///
    /// The panel is a view of the configuration and never a copy of it, so this is the
    /// whole of the round trip: read the rows, click one, and the file changes. What the
    /// caller does with the answer is write it back and rebuild whatever reads it.
    pub fn adjust(&mut self, id: settings::Id, back: bool) -> settings::Effect {
        let effect = settings::adjust(&mut self.config, id, back, &self.families);
        if effect == settings::Effect::Changed {
            // The panel's size is the configured size, so a nudge from the keyboard is
            // dropped the moment the panel is used: two numbers for one thing would
            // otherwise leave the row reading 14 pt while the grid drew 16.
            if id == settings::Id::FontSize {
                self.font_nudge = 0.0;
            }
            self.apply();
        }
        effect
    }

    /// Write the configuration back to the file it was read from, and take in what the
    /// loader makes of what was written.
    ///
    /// The Problems section is about the file rather than in it, which is what makes it
    /// worth reading and also what makes it expire: the panel writes the file on every
    /// click, so a complaint about a value the user has just changed through the rows
    /// below it is the panel reporting a problem it fixed itself, with no way to be rid
    /// of it short of a restart. Reading the file back asks the question of the authority
    /// that answered it at launch, so the list cannot drift from the file it describes.
    ///
    /// The configuration is *not* taken from the re-read. A file something else has
    /// edited since, or one that half landed, must not walk back a click made here.
    ///
    /// # Errors
    ///
    /// Fails if the file cannot be written, or if what is already there is not TOML —
    /// writing would destroy something the user typed.
    pub fn save(&mut self) -> Result<(), zet_config::ConfigError> {
        zet_config::save(&self.config, &self.config_path)?;
        if let Ok(loaded) = load(&self.config_path) {
            self.diagnostics = diagnostics_for(&self.config, loaded.diagnostics);
        }
        Ok(())
    }

    /// Bind a chord to an action, replacing whatever ran it before.
    pub fn bind(&mut self, action: Action, chord: Chord) {
        settings::bind(&mut self.config, action, chord);
        self.bindings = parse_bindings(&self.config);
    }

    /// Re-derive everything the configuration decides.
    ///
    /// Called after a settings change rather than by the host, because the theme and the
    /// keymap are this crate's own reading of the file and nothing outside it can put
    /// them back in step.
    fn apply(&mut self) {
        let (reduce_motion, high_contrast) = self.reported;
        self.reduce_motion = reduce_motion && self.config.appearance.follow_reduce_motion;
        self.forced_contrast = high_contrast && self.config.appearance.follow_forced_colors;
        self.theme = if self.forced_contrast {
            contrast_theme()
        } else {
            by_slug(&self.config.theme).unwrap_or_else(|| zet_config::default_theme())
        };
        self.bindings = parse_bindings(&self.config);
    }

    // ---------------------------------------------------------------------------
    // Tabs
    // ---------------------------------------------------------------------------

    /// Whether a new tab is asking which shell to open.
    #[must_use]
    pub const fn picker_is_open(&self) -> bool {
        self.pick.is_open()
    }

    /// Which row of the profile list the question is on, if it is being asked.
    ///
    /// An index into [`App::profiles`] rather than a profile, because the window paints
    /// the whole list and highlights one row of it: the row is what it needs and a
    /// profile here would be a second way to ask the same question.
    #[must_use]
    pub fn picker_at(&self) -> Option<usize> {
        self.pick.is_open().then(|| self.pick.at())
    }

    /// Open the shell the question is on, and put the question away.
    ///
    /// # Errors
    ///
    /// Whatever [`App::picker_choose_at`] returns.
    pub fn picker_choose(&mut self) -> Result<u32, AppError> {
        self.picker_choose_at(self.pick.at())
    }

    /// Open the shell on a given row, and put the question away.
    ///
    /// One function for the keyboard and the pointer, because they are two ways to the
    /// same answer: a click that opened a different shell from the row it landed on would
    /// be a popover that lies about what it is offering.
    ///
    /// The picker closes whatever the answer turns out to be. A shell that will not start
    /// is an error the host reports, not a question that stays up offering the same shell
    /// to fail again — the user asked once and the answer was no.
    ///
    /// # Errors
    ///
    /// [`AppError::NoProfiles`] when the list has no such row, and whatever
    /// [`App::open_tab_with`] reports when the shell will not start.
    pub fn picker_choose_at(&mut self, row: usize) -> Result<u32, AppError> {
        self.pick.close();
        let id = self.profiles.get(row).map(|profile| profile.id.clone());
        let (cols, rows) = self.grid_size();
        match id {
            Some(id) => self.open_tab_with(&id, cols, rows),
            None => Err(AppError::NoProfiles),
        }
    }

    /// A key aimed at the profile picker, and whether it took it.
    ///
    /// The caller asks this before [`App::key`], the way it asks [`App::find_key`], so an
    /// answer is read once and a key the picker took never reaches the keymap.
    ///
    /// Four keys, and only while the question is up: the picker is a popover over a live
    /// terminal rather than a modal, so every letter and every chord still reaches the
    /// shell behind it. That is the same rule the settings panel follows, and the reason
    /// a chord is excluded here rather than matched on: matching one would take
    /// `Ctrl+Shift+T` away from the action that closes the picker.
    ///
    /// The release is excluded for the reason [`App::action_for`] excludes it — a key-up
    /// is the other half of a press, and an answer that ran twice would open two tabs.
    pub fn picker_key(&mut self, event: &KeyEvent) -> bool {
        if !self.pick.is_open() || event.kind == KeyKind::Release || !event.mods.is_empty() {
            return false;
        }
        match event.key {
            Key::Down => self.pick.move_by(1, self.profiles.len()),
            Key::Up => self.pick.move_by(-1, self.profiles.len()),
            Key::Enter => {
                let _ = self.picker_choose();
            }
            Key::Escape => self.pick.close(),
            _ => return false,
        }
        true
    }

    /// Open a tab with the first profile, which is the machine's best shell.
    ///
    /// # Errors
    ///
    /// Whatever [`App::open_tab_with`] returns.
    pub fn open_tab(&mut self, cols: u16, rows: u16) -> Result<u32, AppError> {
        let id = self.profiles.first().map(|p| p.id.clone());
        match id {
            Some(id) => self.open_tab_with(&id, cols, rows),
            None => Err(AppError::NoProfiles),
        }
    }

    /// Open a tab with the first profile, and answer with what has to be said about it.
    ///
    /// The form the doors into a new tab want, all three of them: `Ctrl+Shift+T` with the
    /// picker turned off, the `+` in the tab strip, and a row of the picker itself. None
    /// of the three has anywhere to say why nothing happened, and the command does.
    pub fn new_tab(&mut self, cols: u16, rows: u16) -> Vec<Command> {
        Command::unopened(self.open_tab(cols, rows))
    }

    /// Open a tab with a named profile.
    ///
    /// # Errors
    ///
    /// [`AppError::NoShell`] when no profile has that id, and whatever the session layer
    /// reports when the shell will not start.
    pub fn open_tab_with(&mut self, id: &str, cols: u16, rows: u16) -> Result<u32, AppError> {
        let profile = self
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .ok_or_else(|| AppError::NoShell(id.to_owned()))?
            .clone();
        let number = self.sessions.open(
            &profile,
            cols,
            rows,
            self.start_directory.clone(),
            Arc::clone(&self.waker),
        )?;
        self.blink.restart(Instant::now(), false);
        Ok(number)
    }

    /// Close a tab, and report whether that was the last one.
    ///
    /// # Errors
    ///
    /// [`SessionError`] when the shell could not be torn down. The tab is closed either
    /// way: a child that will not die is not a reason to leave a tab the user asked to
    /// be rid of.
    pub fn close_tab(&mut self, number: u32) -> Result<bool, SessionError> {
        let was_active = self.sessions.active() == Some(number);
        let result = self.sessions.close(number);
        self.drag = None;
        if was_active {
            // Closing the tab that was showing is a tab switch, whether or not the user
            // asked for one: the strip slides another tab into the gap, and the match
            // list, the count and the marks all belong to the terminal that just went.
            // Closing a tab showing a different one moves nothing under the query and is
            // not a reason to search again.
            self.find.touch();
        }
        result.map(|()| self.sessions.is_empty())
    }

    /// Make a tab active.
    pub fn activate(&mut self, number: u32) {
        if self.sessions.get(number).is_some() {
            self.sessions.set_active(number);
            // A selection belongs to the terminal it was made in.
            self.drag = None;
            // So does a find. The match list, the count, and the marks were all built
            // out of the terminal that was active a moment ago, and the marks are the
            // ones that hurt: they are painted by row, so they land on whatever the new
            // tab has on those rows, and the bar reads "3 of 3" about a terminal that has
            // never contained the query. A tab switch is the same event as a terminal
            // printing — the grid under the query is a different one — so it is the same
            // note that says so.
            self.find.touch();
            self.blink.restart(Instant::now(), false);
        }
    }

    /// Drop the tabs whose shells have exited, and report which numbers went.
    ///
    /// Called after a drain. A tab is not closed the moment its shell exits — a terminal
    /// shows what a program printed on its way out, and the user is the one who decides
    /// the output has been read.
    pub fn reap(&mut self) -> Vec<u32> {
        let active = self.sessions.active();
        let reaped = self.sessions.reap();
        if active.is_some_and(|number| reaped.contains(&number)) {
            // The same door as `close_tab`, and not the one `pump` already covers: an
            // exiting shell usually prints, and `pump` touches the bar for that, but a
            // shell that exits saying nothing does not, and the tab going is on its own
            // enough to leave the matches sitting over a different terminal.
            self.find.touch();
        }
        reaped
    }

    // ---------------------------------------------------------------------------
    // Input
    // ---------------------------------------------------------------------------

    /// Send a key event, and report what the host has to do about it.
    ///
    /// A bound chord runs its action; anything else is encoded and written to the active
    /// terminal, which is the default and the reason the keymap is a short list rather
    /// than a complete description of the keyboard.
    pub fn key(&mut self, event: &KeyEvent) -> Vec<Command> {
        if let Some(action) = self.action_for(event) {
            return self.run(action);
        }
        // A bound chord belongs to zet even on the events `action_for` declined to run it
        // for: the release of the key, and the auto-repeat of a toggle. Falling through to
        // the program there would type the chord into the shell — holding `Ctrl+Shift+F`
        // past the first repeat would send the find bar's own binding to the prompt, and
        // the release is worse, because a program under the kitty keyboard protocol is
        // sent key-up events and would be told about the release of a key it never saw go
        // down.
        if !self.owns(event) {
            self.type_into_session(event);
        }
        Vec::new()
    }

    /// Whether this event's chord is one zet has claimed for itself.
    ///
    /// A bound chord stays zet's on every event, including the ones no action runs for:
    /// the release of the key, and the auto-repeat of a toggle. It is the question [`App::key`]
    /// asks before typing, and it is asked here rather than inline so that "what does zet
    /// keep for itself" has one answer.
    #[must_use]
    pub fn owns(&self, event: &KeyEvent) -> bool {
        self.bound(event.mods, event.key).is_some()
    }

    /// The action a key event runs, if it runs one.
    ///
    /// A binding runs on the way down and on auto-repeat, and never on the way
    /// up. [`App::bound`] answers for a chord and not for an event, so asking *it*
    /// would run the action twice for one press — once when the key went down and
    /// again when it came up, which is two tabs for one `Ctrl+Shift+T`.
    ///
    /// Holding a bound chord down repeats it when the action is a step, which is what
    /// a user holding `Ctrl+Shift+T` is asking for, and does not when the action is a
    /// toggle — see [`Action::repeats`]. `encode_key` already answers `None` for a
    /// release, so the two halves of this agree about what a release is.
    ///
    /// This is a method rather than a condition written out twice because the host
    /// asks the same question for a different reason: it wants to know whether it owes
    /// a frame, since an action changes the screen and then stops talking while typing
    /// is answered by the program's echo. A host that answered it differently from the
    /// app — by forgetting the release, say — would draw twice for one press.
    #[must_use]
    pub fn action_for(&self, event: &KeyEvent) -> Option<Action> {
        if event.kind == KeyKind::Release {
            return None;
        }
        let action = self.bound(event.mods, event.key)?;
        if event.kind == KeyKind::Repeat && !self.repeats(action) {
            return None;
        }
        Some(action)
    }

    /// Whether holding this action's chord down should run it again.
    ///
    /// [`Action::repeats`] is the answer for the action on its own, and it is the whole
    /// answer except for `new-tab`. That one is a step — holding `Ctrl+Shift+T` to fill
    /// the window with tabs is what a user holding it is asking for — until the profile
    /// picker is up, at which point the same chord *closes* the picker and is a toggle
    /// for as long as it is open. A toggle that ran on auto-repeat would open and close
    /// the question on alternate frames and leave the user looking at whichever state
    /// the repeat rate happened to stop on.
    ///
    /// It is a method on the app rather than a rule inside the action because the action
    /// cannot see whether the picker is open, and it is asked here rather than at the
    /// call site because the host asks [`App::action_for`] the same question for its own
    /// reason and the two answers have to agree.
    #[must_use]
    fn repeats(&self, action: Action) -> bool {
        if self.pick.is_open() && action == Action::NewTab {
            return false;
        }
        action.repeats()
    }

    /// Send typed text to the active terminal, as though it had been pasted.
    ///
    /// Used for an IME commit, which arrives as composed text rather than as a key, and
    /// by [`App::paste`].
    pub fn paste(&mut self, text: &str) {
        let modes = self
            .active()
            .map_or_else(Modes::default, |s| s.term().modes());
        let bytes = encode_paste(text, &modes);
        if let Some(session) = self.active_mut() {
            let _ = session.write(&bytes);
        }
    }

    /// Find the action a key is bound to, if any.
    ///
    /// This is asked before the encoder is, so a chord zet has a binding for never
    /// reaches the program — and that is deliberate even for a program that has
    /// negotiated the kitty keyboard protocol. The protocol's `CSI u` changes what the
    /// terminal *sends*; it does not say the terminal stops having a keyboard of its
    /// own, and a terminal that gave its own shortcuts up the moment a program asked
    /// for all keys would be one where the settings panel could be unreachable from
    /// inside whichever full-screen program happened to be running. kitty, which wrote
    /// the protocol, keeps its own bindings the same way.
    ///
    /// What the flags would change is the *matching*, and they are not consulted here:
    /// a chord is matched on the key and the modifiers the host reports, and a program
    /// that wanted `Ctrl+Shift+Comma` for itself would need the user to unbind it. The
    /// panel has a row for every action, so that is a thing the user can do.
    #[must_use]
    pub fn bound(&self, mods: zet_input::Modifiers, key: zet_input::Key) -> Option<Action> {
        self.bindings
            .iter()
            .find(|(chord, _)| chord.matches(mods, key))
            .map(|(_, action)| *action)
    }

    fn type_into_session(&mut self, event: &KeyEvent) {
        let Some(session) = self.active_mut() else {
            return;
        };
        let bytes = encode_key(event, &session.term().modes());
        if let Some(bytes) = bytes {
            // Typing is the user asking to see the prompt.
            let _ = session.write(&bytes);
        }
        self.blink.restart(Instant::now(), false);
    }

    /// Carry out an action.
    fn run(&mut self, action: Action) -> Vec<Command> {
        match action {
            Action::NewTab => {
                if self.pick.is_open() {
                    // One chord, both ways, as the find bar and the settings panel have
                    // it: the key that asked the question is the one a user who did not
                    // mean to ask it will press next.
                    self.pick.close();
                } else if self.config.tabs.open_default_without_asking {
                    let (cols, rows) = self.grid_size();
                    return self.new_tab(cols, rows);
                } else {
                    self.pick.open();
                }
                Vec::new()
            }
            Action::CloseTab => {
                let Some(number) = self.sessions.active() else {
                    return Vec::new();
                };
                match self.close_tab(number) {
                    Ok(true) => vec![Command::Quit],
                    _ => Vec::new(),
                }
            }
            Action::NextTab | Action::PreviousTab => {
                if let Some(number) = self.sessions.active() {
                    let next = if action == Action::NextTab {
                        self.sessions.next_number(number)
                    } else {
                        self.sessions.previous_number(number)
                    };
                    if let Some(next) = next {
                        self.activate(next);
                    }
                }
                Vec::new()
            }
            Action::NewWindow => vec![Command::NewWindow],
            Action::Copy => match self.selection_text() {
                Some(text) if !text.is_empty() => vec![Command::Copy(text)],
                _ => Vec::new(),
            },
            Action::Paste => vec![Command::Paste],
            Action::Quit => vec![Command::Quit],
            Action::ToggleTabPosition => {
                self.config.tabs.position = self.config.tabs.position.flipped();
                Vec::new()
            }
            Action::ScrollPageUp => self.scroll(PAGE),
            Action::ScrollPageDown => self.scroll(-PAGE),
            Action::ScrollToTop => {
                // Up into the history, which is a *positive* delta — `Session::scroll` reads
                // a negative one as "come back down towards the live screen". This was
                // `i32::MIN`, so the largest possible step in the wrong direction, and
                // `scroll_to` clamps to what there is: `Ctrl+Shift+Home` landed on the live
                // screen, exactly where `Ctrl+Shift+End` goes, and the oldest line was
                // unreachable by any key. The clamp is what makes a step this large the
                // right way to say "as far as it goes".
                self.scroll(i32::MAX);
                Vec::new()
            }
            Action::ScrollToBottom => {
                if let Some(session) = self.active_mut() {
                    session.scroll_to_bottom();
                }
                Vec::new()
            }
            Action::FontLarger => {
                self.font_nudge =
                    (self.font_nudge + 1.0).min(settings::MAX_SIZE - self.config.font.size);
                Vec::new()
            }
            Action::FontSmaller => {
                self.font_nudge =
                    (self.font_nudge - 1.0).max(settings::MIN_SIZE - self.config.font.size);
                Vec::new()
            }
            Action::FontReset => {
                self.font_nudge = 0.0;
                Vec::new()
            }
            Action::Find => {
                // One chord, both ways: the key that opens the bar closes it, which is
                // what a user who pressed it by accident will press next.
                if self.find.is_open() {
                    self.find.close();
                } else {
                    self.find.open();
                    self.refresh_find();
                    self.reveal_find();
                }
                Vec::new()
            }
            // The settings panel is drawn by the window rather than by this crate, and
            // the state that says whether it is open is the window's too.
            Action::Settings => Vec::new(),
        }
    }

    fn scroll(&mut self, delta: i32) -> Vec<Command> {
        if let Some(session) = self.active_mut() {
            session.scroll(delta);
        }
        Vec::new()
    }

    // ---------------------------------------------------------------------------
    // Find
    // ---------------------------------------------------------------------------

    /// The find bar's state.
    #[must_use]
    pub const fn find(&self) -> &Find {
        &self.find
    }

    /// Run the query again over the active terminal, if there is a reason to.
    ///
    /// Called once per frame while the bar is open. It is a no-op unless the query just
    /// changed or the terminal has printed since the last look — see [`Find::search`] —
    /// because the walk allocates a glyph per cell of the history and a terminal that
    /// is streaming output would otherwise pay for all of it sixty times a second.
    pub fn refresh_find(&mut self) {
        let (sessions, find) = (&self.sessions, &mut self.find);
        let Some(session) = sessions.active().and_then(|number| sessions.get(number)) else {
            return;
        };
        let grid = session.term().grid();
        let from = grid.history_top(session.scroll_offset());
        find.search(grid, from);
    }

    /// Take a key for the find bar, and say whether it took it.
    ///
    /// The bar owns the keys a text field owns — what you type, `Backspace`, `Escape`,
    /// and `Enter` for the next match — and nothing else. Everything with a modifier on
    /// it is left alone, so a chord still reaches the keymap: `Ctrl+Shift+F` closes the
    /// bar it opened, and `Ctrl+Shift+T` opens a tab whether the bar is up or not.
    pub fn find_key(&mut self, event: &KeyEvent) -> bool {
        if !self.find.is_open() || event.kind == KeyKind::Release {
            return false;
        }
        let typed = event.mods.is_empty() || event.mods == Modifiers::SHIFT;
        match event.key {
            Key::Escape if typed => {
                self.find.close();
                return true;
            }
            Key::Enter if typed => {
                // Plain Enter goes forward and Shift+Enter goes back, which is what
                // every find bar does. `typed` admits exactly those two, so the shift
                // is the whole of the difference between them.
                self.find_step(event.mods.is_empty());
                return true;
            }
            Key::Backspace if typed => {
                self.find.pop();
                self.refresh_find();
                return true;
            }
            _ if typed => {}
            _ => return false,
        }
        // The platform's own text, which is what an IME commits and what a layout
        // produces. `Key::Char` would do for a US keyboard and would be wrong for the
        // three quarters of the world that do not have one.
        let Some(text) = event.text.as_deref() else {
            return false;
        };
        let mut took = false;
        for ch in text.chars() {
            if !ch.is_control() {
                self.find.push(ch);
                took = true;
            }
        }
        if took {
            self.refresh_find();
        }
        took
    }

    /// Move to the next or the previous match, and bring it into view.
    pub fn find_step(&mut self, forward: bool) {
        if forward {
            self.find.next_match();
        } else {
            self.find.previous_match();
        }
        self.reveal_find();
    }

    /// Scroll the active terminal so that the match the arrows are on is on screen.
    ///
    /// Only when it is not already there, so that walking matches that are all visible
    /// does not move the view under the user. A match above the fold becomes the top
    /// row; one below becomes the bottom row; and one taller than the window is shown
    /// from its first row, because a match you can see the start of is a match you can
    /// read.
    fn reveal_find(&mut self) {
        let (sessions, find) = (&mut self.sessions, &self.find);
        let Some(found) = find.active() else {
            return;
        };
        let Some(number) = sessions.active() else {
            return;
        };
        let Some(session) = sessions.get_mut(number) else {
            return;
        };
        let want = {
            let grid = session.term().grid();
            let top = grid.history_top(session.scroll_offset());
            let Some(want) = reveal_row(found, top, grid.rows()) else {
                return;
            };
            want
        };
        // The offset counts from the live screen rather than from the top of the
        // history, so the row wanted at the top and the number of rows behind it are
        // the same number read from opposite ends.
        let history = session.term().grid().scrollback_len();
        session.scroll_to(history.saturating_sub(want));
    }

    /// The find bar's matches that are on screen now, in the renderer's coordinates.
    ///
    /// Returns the marks to paint and which of them the arrows are on. The conversion
    /// is a subtraction, and it is the host's because only the session knows where the
    /// view is scrolled to — the find bar deals in positions in a history that is
    /// growing underneath it.
    #[must_use]
    pub fn find_marks(&self) -> (Vec<Selection>, Option<usize>) {
        let Some(session) = self.active().filter(|_| self.find.is_open()) else {
            return (Vec::new(), None);
        };
        let grid = session.term().grid();
        let rows = grid.rows();
        let cols = grid.cols();
        let top = grid.history_top(session.scroll_offset());
        let mut marks = Vec::new();
        let mut active = None;
        for (index, found) in self.find.matches().iter().enumerate() {
            if found.end.row < top || found.start.row >= top + rows {
                continue;
            }
            if self.find.is_active(index) {
                active = Some(marks.len());
            }
            marks.push(clip_match(*found, top, rows, cols));
        }
        (marks, active)
    }

    /// The text the user has selected, if there is one.
    #[must_use]
    pub fn selection_text(&self) -> Option<String> {
        let selection = self.selection()?;
        let session = self.active()?;
        Some(selection_text(
            session.term(),
            selection,
            session.scroll_offset(),
        ))
    }

    // ---------------------------------------------------------------------------
    // Pointer
    // ---------------------------------------------------------------------------

    /// Begin a selection at a cell.
    pub fn select_from(&mut self, at: Pos) {
        self.drag = Some(Drag {
            anchor: at,
            head: at,
        });
    }

    /// Extend a selection to a cell.
    ///
    /// Does nothing when no drag is in progress, so a host that forwards every motion
    /// event does not have to know whether the button is down.
    pub fn select_to(&mut self, at: Pos) {
        if let Some(drag) = &mut self.drag {
            drag.head = at;
        }
    }

    /// End a selection. The highlight is kept; only the gesture is over.
    pub fn select_end(&mut self) {
        if let Some(drag) = self.drag
            && drag.anchor == drag.head
        {
            // A click with no drag is a click, not a one-cell selection, and a
            // one-cell highlight left behind after every click is noise.
            self.drag = None;
        }
    }

    /// Throw the selection away.
    pub fn select_none(&mut self) {
        self.drag = None;
    }

    /// Open the hyperlink on this screen cell, if it has one.
    ///
    /// What is decided here is only what the click landed on; whether the click *is* the
    /// gesture is the host's, because the modifier that makes it one is the host's to
    /// read. A cell with no link answers with nothing at all rather than with a command
    /// the host would have to know to ignore, which is the rule the rest of this file
    /// follows: every `Command` that comes back is one to carry out.
    ///
    /// The row is a screen row, counted from the top of the visible grid, because that is
    /// what a pointer is in and what a mouse report counts in. Turning it into a history
    /// index is the same arithmetic the renderer does, and it is the only place the scroll
    /// offset is allowed to matter.
    #[must_use]
    pub fn open_link_at(&mut self, at: Pos) -> Vec<Command> {
        let Some(session) = self.active() else {
            return Vec::new();
        };
        let term = session.term();
        let grid = term.grid();
        let index = grid.history_top(session.scroll_offset()) + at.row;
        let Some(line) = grid.row_from_history(index) else {
            return Vec::new();
        };
        let Some(url) = term.link_for(&line.get(at.col)) else {
            return Vec::new();
        };
        // Only the schemes that mean "a page". The system's open call does not only open:
        // handed a path it runs it, and this string was chosen by a program rather than by
        // the user, so anything that is not a URL is dropped here rather than being handed
        // to the shell. A program that wants to run something can print a command for the
        // user to read; one that wants to run something without it being read is the case
        // this exists for.
        if openable(url) {
            vec![Command::OpenUrl(url.to_owned())]
        } else {
            Vec::new()
        }
    }

    // ---------------------------------------------------------------------------
    // The rest of the window's job
    // ---------------------------------------------------------------------------

    /// Take in whatever the shells have written, and report whether anything changed.
    ///
    /// Returns true when at least one session produced output that changed its grid, so
    /// the host knows a repaint is worth doing.
    pub fn pump(&mut self) -> bool {
        let numbers = self.sessions.numbers();
        let mut changed = false;
        for number in numbers {
            if let Some(session) = self.sessions.get_mut(number) {
                let drained = session.drain();
                changed |= drained.damage;
            }
        }
        if changed {
            // What was found in a grid that has been printing since is a list of
            // positions that have moved, and the count in the bar would be a count of
            // where things used to be.
            self.find.touch();
        }
        changed
    }

    /// When the frame may next be drawn, if the active program is holding it.
    ///
    /// `DECSET 2026` is the one thing in the protocol that makes a full-screen program
    /// flicker-free, and it only works if the terminal honors it: a program that clears
    /// the screen and redraws it in forty writes is showing thirty-nine states nobody
    /// asked to see. The marker has no end that a program which has crashed will send, so
    /// the hold expires — see [`crate::hold`] for the budget and why it is not optional.
    ///
    /// Only the active tab is asked. A background tab repainting is not something the
    /// window is showing, and holding the frame for it would be holding the frame for
    /// output nobody can see.
    ///
    /// The instant is supplied by the caller rather than read here, which is what makes a
    /// budget measurable from either side of its own boundary.
    ///
    /// The deadline comes back rather than a bare "yes" because the caller has to be woken
    /// to draw the frame it is declining to draw now, and the one program that will not
    /// wake it is the one the budget is for: a program that set the marker and stopped
    /// sends no further output, so a terminal that only knew "hold" would wait for an
    /// event that never comes and hold the frame for ever. `None` means draw now.
    pub fn frame_hold(&mut self, now: Instant) -> Option<Instant> {
        let synchronized = self
            .active()
            .is_some_and(|session| session.term().is_synchronized());
        self.hold
            .holds(synchronized, now)
            .then(|| self.hold.deadline())
            .flatten()
    }

    /// Tell every session how big the grid is.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let resized_any = !self.sessions.is_empty();
        for number in self.sessions.numbers() {
            if let Some(session) = self.sessions.get_mut(number) {
                let _ = session.resize(cols, rows);
            }
        }
        if resized_any {
            // A resize re-wraps every logical line, so rows move and the positions the
            // bar found are positions in a grid that no longer exists. A resize with
            // nothing open changes nothing to have found anything in.
            self.find.touch();
        }
    }

    /// The grid size of the active session, or a sensible default when none is open.
    fn grid_size(&self) -> (u16, u16) {
        self.active()
            .map_or((80, 24), |session| (session.cols(), session.rows()))
    }

    /// Apply what the system reports about accessibility, as far as the configuration
    /// lets it.
    ///
    /// Both of these are the system's news and the user's decision. Windows says whether
    /// motion should be reduced and whether colours are forced; `[appearance]` says
    /// whether zet acts on either, and both default to yes because a machine that has
    /// asked for less movement has asked for a reason.
    ///
    /// Called again whenever the configuration changes, because the two switches behind
    /// it are rows in the settings panel: a user turning "Reduce motion" off is changing
    /// their mind about exactly this, and a decision read once at startup would leave the
    /// row doing nothing until the next launch.
    pub fn system_accessibility(&mut self, reduce_motion: bool, high_contrast: bool) {
        self.reported = (reduce_motion, high_contrast);
        self.apply();
    }
}

/// Where the view has to be scrolled to so that `found` is on screen, or `None` when
/// it already is.
///
/// `top` is the history row at the top of the viewport and `rows` is how many the
/// viewport has. A match above the fold becomes the top row; one below becomes the
/// bottom row; and one taller than the window is shown from its first row, because a
/// match you can see the start of is a match you can read — scroll far enough to put
/// its last row on the bottom row and its first row is off the top, which is the one
/// case where following the rule blindly hides the whole point of going there.
///
/// Pure, and separated from the session it scrolls, because the arithmetic is where
/// this was wrong and the arithmetic is what a test can hold down.
fn reveal_row(found: Match, top: usize, rows: usize) -> Option<usize> {
    let bottom = top + rows - 1;
    if found.start.row < top {
        Some(found.start.row)
    } else if found.end.row > bottom {
        Some(
            (found.end.row + 1)
                .saturating_sub(rows)
                .min(found.start.row),
        )
    } else {
        None
    }
}

/// One of the find bar's matches, in the coordinates the renderer draws in.
///
/// A match with a row above the viewport is cut off at the top, and what is left of it
/// starts at the beginning of the first visible row: the column it began at belongs to
/// a line nobody can see. Carrying it over would bound the visible part by a column
/// from another line, or — when the whole match maps onto one row — give the selection
/// corners the wrong way round and paint everything between them. The same at the
/// other end.
fn clip_match(found: Match, top: usize, rows: usize, cols: usize) -> Selection {
    let start = Pos::new(
        found.start.row.saturating_sub(top),
        if found.start.row < top {
            0
        } else {
            found.start.col
        },
    );
    let end = Pos::new(
        found.end.row.saturating_sub(top).min(rows - 1),
        if found.end.row >= top + rows {
            cols - 1
        } else {
            found.end.col
        },
    );
    Selection::new(start, end)
}

/// Whether a hyperlink is something to open rather than something to run.
///
/// A program chooses the string, and the call that opens a URL is the same call that
/// starts a program: `C:\Windows\System32\calc.exe` is a perfectly good argument to it.
/// The three schemes that mean "a page" are the ones that get through, and everything
/// else — including a bare path and a scheme nobody here has heard of — is refused by
/// being absent from this list rather than by being on a list of things to refuse.
///
/// Case is not significant: the URL specification says schemes are case-insensitive, and
/// a program that writes `HTTPS://` means what every other program means by `https://`.
fn openable(url: &str) -> bool {
    let scheme = url.split_once(':').map(|(scheme, _)| scheme);
    matches!(
        scheme,
        Some(scheme)
            if scheme.eq_ignore_ascii_case("http")
                || scheme.eq_ignore_ascii_case("https")
                || scheme.eq_ignore_ascii_case("mailto")
    )
}

/// Turn the config file's `[keys]` table into chords.
///
/// An entry that cannot be parsed is dropped rather than fatal — a keybinding is not worth
/// refusing to start over — and [`report_unparseable_bindings`] is where the dropping is
/// said out loud.
pub(crate) fn parse_bindings(config: &Config) -> Vec<(Chord, Action)> {
    config
        .keys
        .iter()
        .filter_map(|(name, text)| {
            let action = Action::from_name(name)?;
            let chord = text.parse::<Chord>().ok()?;
            Some((chord, action))
        })
        .collect()
}

/// Report the `[keys]` values that are not chords.
///
/// The loader does not parse chords — `zet-config` has no business knowing what one is,
/// and says so where it decides what it checks — so this is the only place in the
/// workspace where `new-tab = "Ctrl+Banana"` is looked at. Without it the entry is dropped
/// by [`parse_bindings`] in silence, which is the one thing a misspelled binding must not
/// do: the key simply stops working and nothing says why.
///
/// Two values are skipped, and both are deliberate rather than misspelled. An action zet
/// does not know is the loader's to report, and it already does. An empty value is how the
/// settings panel writes an action it has taken a chord away from.
fn report_unparseable_bindings(config: &Config, diagnostics: &mut Vec<Diagnostic>) {
    for (name, text) in &config.keys {
        if text.is_empty() || Action::from_name(name).is_none() {
            continue;
        }
        if text.parse::<Chord>().is_err() {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                message: format!("keys.{name} = {text:?} is not a chord zet knows"),
            });
        }
    }
}

/// Everything wrong with `config`: what the loader found, and what only this crate can.
///
/// One function because there are two ways in — a launch and a save — and the panel must
/// not be able to tell which of them it is looking at. The loader's come first, so the
/// rows read in the order they were found: what is wrong with the file's shape, and then
/// what is wrong with its bindings.
fn diagnostics_for(config: &Config, mut diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    report_unparseable_bindings(config, &mut diagnostics);
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;
    use zet_input::{Key, KeyEvent, KeyKind, Modifiers};
    use zet_session::NoopWaker;

    fn app() -> App {
        App::new(
            Config::default(),
            PathBuf::from("test.toml"),
            Vec::new(),
            Arc::new(NoopWaker),
        )
        .expect("this machine has a shell")
    }

    fn key(key: Key, mods: Modifiers) -> KeyEvent {
        KeyEvent {
            key,
            mods,
            text: None,
            kind: KeyKind::Press,
        }
    }

    #[test]
    fn a_fresh_app_has_no_tabs_and_no_configuration_problems() {
        let app = app();
        assert!(app.sessions().is_empty());
        assert_eq!(app.active_number(), None);
        assert!(app.diagnostics().is_empty());
    }

    #[test]
    fn a_machine_has_at_least_one_shell_to_open() {
        assert!(!app().profiles().is_empty());
    }

    #[test]
    fn what_the_file_got_wrong_is_the_first_thing_the_panel_says() {
        // The loader has always produced these and `App::diagnostics` has always held
        // them, and nothing read either, so the promise in the configuration reference —
        // that a value which parses and cannot be used is reported — was kept by a field
        // no user could see. The panel is the surface: it is where somebody who is
        // changing settings is standing.
        let app = App::new(
            Config::default(),
            PathBuf::from("test.toml"),
            vec![Diagnostic {
                severity: zet_config::Severity::Warning,
                message: "cursor.thickness is outside 1 to 8, using 2".to_owned(),
            }],
            Arc::new(NoopWaker),
        )
        .expect("this machine has a shell");

        let rows = app.settings();
        assert_eq!(rows.first().map(settings::Line::text), Some("Problems"));
        let problem = rows
            .iter()
            .find(|row| matches!(row, settings::Line::Note(_)))
            .expect("the diagnostic is a row");
        assert_eq!(
            problem.text(),
            "cursor.thickness is outside 1 to 8, using 2"
        );
        assert_eq!(problem.value(), "warning");
        assert!(
            problem.id().is_none(),
            "a problem is not a setting and has nothing to adjust"
        );
    }

    #[test]
    fn a_file_with_nothing_wrong_says_nothing_about_it() {
        // The guard on the test above: a panel that opened with an empty "Problems" every
        // time would be a heading over nothing, and the one launch where it mattered would
        // read as decoration.
        assert_eq!(
            app().settings().first().map(settings::Line::text),
            Some("Appearance")
        );
        assert!(
            app()
                .settings()
                .iter()
                .all(|row| !matches!(row, settings::Line::Note(_)))
        );
    }

    #[test]
    fn every_default_binding_names_an_action_the_app_has() {
        // The shipped keymap and the action list are two lists of the same thing, and a
        // key that silently does nothing is the failure this catches.
        let config = Config::default();
        let bound = parse_bindings(&config);
        assert_eq!(bound.len(), config.keys.len());
        for (_, action) in &bound {
            assert!(Action::ALL.contains(action), "{action:?} is not an action");
        }
    }

    #[test]
    fn a_binding_that_does_not_parse_is_dropped_rather_than_fatal() {
        let mut config = Config::default();
        config.keys.insert("new-tab".into(), "Ctrl+Banana".into());
        let bound = parse_bindings(&config);
        assert!(!bound.iter().any(|(_, a)| *a == Action::NewTab));
        assert!(bound.iter().any(|(_, a)| *a == Action::CloseTab));
    }

    #[test]
    fn only_the_schemes_that_mean_a_page_are_openable() {
        for url in [
            "https://example.com",
            "http://example.com",
            "mailto:someone@example.com",
            // The specification says a scheme is not case-sensitive, and a program that
            // writes it in capitals means what every other program means.
            "HTTPS://example.com",
        ] {
            assert!(openable(url), "{url} should be openable");
        }
        for url in [
            // The one that matters: the system's open call runs a path handed to it.
            r"C:\Windows\System32\calc.exe",
            r"Z:\nowhere\evil.exe",
            "file:///C:/Windows/System32/calc.exe",
            "javascript:alert(1)",
            "",
            "no-scheme-at-all",
        ] {
            assert!(!openable(url), "{url} should not be openable");
        }
    }

    #[test]
    fn a_binding_that_does_not_parse_is_reported_as_well_as_dropped() {
        // The keybindings reference promises "a misspelled modifier is reported as an
        // unknown key", and `parse_bindings`'s own comment claimed "the config loader has
        // already reported the ones it could see". Neither was true: zet-config does not
        // parse chords and says so in its own comment, so nothing in the workspace ever
        // looked at `new-tab = "Ctrl+Banana"` and the entry simply vanished.
        let mut config = Config::default();
        config.keys.insert("new-tab".into(), "Ctrl+Banana".into());
        let app = App::new(
            config,
            PathBuf::from("test.toml"),
            Vec::new(),
            Arc::new(NoopWaker),
        )
        .expect("this machine has a shell");
        let reported: Vec<(&Severity, &str)> = app
            .diagnostics()
            .iter()
            .map(|diagnostic| (&diagnostic.severity, diagnostic.message.as_str()))
            .collect();
        assert_eq!(
            reported,
            vec![(
                &Severity::Error,
                "keys.new-tab = \"Ctrl+Banana\" is not a chord zet knows"
            )]
        );
    }

    #[test]
    fn an_action_the_panel_unbound_is_not_a_chord_it_cannot_read() {
        // The empty value is the documented spelling for "unbound" — the settings panel
        // writes it when a chord is taken away from an action — so it is the one value in
        // the table that is deliberately not a chord and must not be complained about.
        let mut config = Config::default();
        config.keys.insert("new-tab".into(), String::new());
        let app = App::new(
            config,
            PathBuf::from("test.toml"),
            Vec::new(),
            Arc::new(NoopWaker),
        )
        .expect("this machine has a shell");
        assert!(app.diagnostics().is_empty(), "{:?}", app.diagnostics());
    }

    #[test]
    fn a_bound_chord_runs_its_action_instead_of_reaching_the_program() {
        let mut app = app();
        let commands = app.key(&key(Key::F(4), Modifiers::ALT));
        assert_eq!(commands, vec![Command::Quit]);
    }

    #[test]
    fn an_unbound_chord_goes_to_the_terminal_when_one_is_open() {
        let mut app = app();
        let _ = app.open_tab(80, 24).expect("a shell starts");
        assert!(app.key(&key(Key::Char('a'), Modifiers::empty())).is_empty());
        // The shell has been told about the key, not about the binding.
        assert!(app.active().is_some());
    }

    #[test]
    fn ctrl_shift_t_opens_a_tab_and_it_becomes_the_active_one() {
        let mut app = app();
        let _ = app.open_tab(80, 24).expect("a shell starts");
        let _ = app.key(&key(Key::Char('T'), Modifiers::CTRL | Modifiers::SHIFT));
        assert_eq!(app.tab_numbers().len(), 2);
        assert_eq!(app.active_number(), Some(2));
    }

    /// An app whose file says to ask which shell a new tab opens.
    fn app_that_asks() -> App {
        let mut config = Config::default();
        config.tabs.open_default_without_asking = false;
        App::new(
            config,
            PathBuf::from("test.toml"),
            Vec::new(),
            Arc::new(NoopWaker),
        )
        .expect("this machine has a shell")
    }

    /// The chord the shipped keymap gives `new-tab`.
    fn new_tab_chord() -> KeyEvent {
        key(Key::Char('T'), Modifiers::CTRL | Modifiers::SHIFT)
    }

    #[test]
    fn a_new_tab_opens_the_shell_on_the_machine_with_nothing_in_the_way() {
        // The default, and it has to stay the default: `open-default-without-asking` is
        // the settings panel's row and a user who never opens that file gets a tab from
        // one keystroke and no question.
        let mut app = app();
        let _ = app.key(&new_tab_chord());
        assert_eq!(app.tab_numbers().len(), 1);
        assert!(!app.picker_is_open(), "nothing was asked");
    }

    #[test]
    fn a_new_tab_asks_which_shell_when_the_file_says_to_ask() {
        // `tabs.open-default-without-asking` was documented in docs/configuration.html
        // from the day it was written and read by nothing: setting it to false gave the
        // user the same tab as leaving it alone, with no error and nothing to explain it.
        let mut app = app_that_asks();
        let _ = app.key(&new_tab_chord());
        assert!(app.picker_is_open());
        assert_eq!(app.tab_numbers().len(), 0, "asking opens no tab by itself");
        assert_eq!(app.picker_at(), Some(0));
    }

    /// The name a tab reports before its shell has set a title of its own, which is the
    /// profile's name. Nothing drains the terminal in these tests, so this is the
    /// profile that actually ran rather than whatever the shell has printed since.
    fn opened_profile(app: &App) -> String {
        app.active().expect("a tab is open").title()
    }

    #[test]
    fn the_question_starts_on_the_shell_that_would_have_opened_anyway() {
        // Enter, pressed the moment the picker appears, has to be the same tab the key
        // would have opened with the setting off — otherwise turning the setting on is a
        // tax on everyone who wanted the default after all.
        let mut asking = app_that_asks();
        let _ = asking.key(&new_tab_chord());
        let _ = asking.picker_key(&key(Key::Enter, Modifiers::empty()));
        let mut plain = app();
        let _ = plain.key(&new_tab_chord());
        assert_eq!(opened_profile(&asking), opened_profile(&plain));
    }

    #[test]
    fn walking_the_question_and_answering_it_opens_the_row_that_was_lit() {
        let mut app = app_that_asks();
        let _ = app.key(&new_tab_chord());
        // A machine with one shell has one row, and it is the only one there is to be on.
        // The walk below is written for both shapes rather than skipped on one of them.
        let last = app.profiles().len() - 1;
        let wanted = app.profiles()[last].name.clone();
        for _ in 0..last {
            let _ = app.picker_key(&key(Key::Down, Modifiers::empty()));
        }
        assert_eq!(app.picker_at(), Some(last));
        let _ = app.picker_key(&key(Key::Enter, Modifiers::empty()));
        assert!(!app.picker_is_open(), "the question is put away");
        assert_eq!(app.tab_numbers().len(), 1);
        assert_eq!(opened_profile(&app), wanted);
    }

    #[test]
    fn the_question_survives_walking_off_the_end_of_the_list() {
        let mut app = app_that_asks();
        let _ = app.key(&new_tab_chord());
        let rows = app.profiles().len();
        for _ in 0..=rows {
            let _ = app.picker_key(&key(Key::Down, Modifiers::empty()));
        }
        assert_eq!(
            app.picker_at(),
            Some(1 % rows),
            "walking off the end comes back round"
        );
        let _ = app.picker_key(&key(Key::Up, Modifiers::empty()));
        assert_eq!(app.picker_at(), Some(0), "and back the other way");
    }

    #[test]
    fn the_row_a_click_landed_on_is_the_shell_that_opens() {
        // The keyboard and the pointer are two ways to the same answer, so they go through
        // one function: a click that opened a different shell from the row it was on would
        // be a popover that lies about what it is offering.
        let mut app = app_that_asks();
        let _ = app.key(&new_tab_chord());
        let last = app.profiles().len() - 1;
        let wanted = app.profiles()[last].name.clone();
        let _ = app.picker_choose_at(last);
        assert!(!app.picker_is_open());
        assert_eq!(app.tab_numbers().len(), 1);
        assert_eq!(opened_profile(&app), wanted);
    }

    #[test]
    fn a_row_that_is_not_on_the_list_opens_nothing() {
        let mut app = app_that_asks();
        let _ = app.key(&new_tab_chord());
        let beyond = app.profiles().len();
        assert!(app.picker_choose_at(beyond).is_err());
        assert!(!app.picker_is_open(), "the question was still answered");
        assert_eq!(app.tab_numbers().len(), 0);
    }

    #[test]
    fn changing_your_mind_opens_no_tab() {
        let mut app = app_that_asks();
        let _ = app.key(&new_tab_chord());
        let _ = app.picker_key(&key(Key::Escape, Modifiers::empty()));
        assert!(!app.picker_is_open());
        assert_eq!(app.tab_numbers().len(), 0);
    }

    #[test]
    fn the_chord_that_asked_the_question_takes_it_back() {
        // One chord, both ways, which is what the find bar and the settings panel do and
        // the only way out that does not depend on the user guessing a second key.
        let mut app = app_that_asks();
        let _ = app.key(&new_tab_chord());
        let _ = app.key(&new_tab_chord());
        assert!(!app.picker_is_open());
        assert_eq!(app.tab_numbers().len(), 0, "closing it opened a tab");
    }

    #[test]
    fn holding_the_chord_does_not_flap_the_question() {
        // While the picker is up this chord is a toggle rather than a step, and a toggle
        // that ran on auto-repeat would open and close it on alternate frames and leave
        // the user looking at whichever state the repeat rate happened to stop on.
        let mut app = app_that_asks();
        let _ = app.key(&new_tab_chord());
        for _ in 0..5 {
            let _ = app.key(&KeyEvent {
                kind: KeyKind::Repeat,
                ..new_tab_chord()
            });
        }
        assert!(app.picker_is_open());
        assert_eq!(app.tab_numbers().len(), 0);
    }

    #[test]
    fn the_question_is_not_asked_again_once_it_has_been_answered() {
        let mut app = app_that_asks();
        let _ = app.key(&new_tab_chord());
        let _ = app.picker_key(&key(Key::Enter, Modifiers::empty()));
        assert!(!app.picker_is_open());
        let _ = app.key(&new_tab_chord());
        assert!(app.picker_is_open(), "the next new tab asks again");
        assert_eq!(app.tab_numbers().len(), 1);
    }

    #[test]
    fn a_binding_runs_on_the_way_down_and_not_on_the_way_up() {
        // The failure this prevents is quiet and doubled: `bound` answers for a chord,
        // not for an event, so a release that reached it would run the action a second
        // time and one press of `Ctrl+Shift+T` would open two tabs.
        let mut app = app();
        let _ = app.open_tab(80, 24).expect("a shell starts");
        let chord = Modifiers::CTRL | Modifiers::SHIFT;
        let _ = app.key(&KeyEvent {
            kind: KeyKind::Release,
            ..key(Key::Char('T'), chord)
        });
        assert_eq!(app.tab_numbers().len(), 1, "letting go opened a tab");

        let _ = app.key(&key(Key::Char('T'), chord));
        assert_eq!(app.tab_numbers().len(), 2);
        let _ = app.key(&KeyEvent {
            kind: KeyKind::Repeat,
            ..key(Key::Char('T'), chord)
        });
        assert_eq!(
            app.tab_numbers().len(),
            3,
            "holding a bound chord should repeat it, which is the whole point of holding it"
        );
    }

    #[test]
    fn a_held_toggle_runs_once_and_does_not_reach_the_program_afterwards() {
        // The bar is opened by the press. A repeat that ran the action again would shut
        // it, and letting the repeat fall through to the shell instead would type the
        // chord into the prompt — so the key has to stay zet's while doing nothing.
        let mut app = app();
        let _ = app.open_tab(80, 24).expect("a shell starts");
        let chord = Modifiers::CTRL | Modifiers::SHIFT;
        assert_eq!(
            app.action_for(&key(Key::Char('F'), chord)),
            Some(Action::Find)
        );
        assert_eq!(
            app.action_for(&KeyEvent {
                kind: KeyKind::Repeat,
                ..key(Key::Char('F'), chord)
            }),
            None,
            "a toggle must not run again on the repeat"
        );

        let commands = app.key(&KeyEvent {
            kind: KeyKind::Repeat,
            ..key(Key::Char('F'), chord)
        });
        assert!(
            commands.is_empty(),
            "the repeat ran something: {commands:?}"
        );
    }

    #[test]
    fn the_release_of_a_bound_chord_is_zet_and_not_the_shell() {
        // The press runs the action, so the release has to stay zet's. A program under
        // the kitty keyboard protocol is sent key-up events, and the release of a chord
        // the app swallowed on the way down is a key-up for a press the program never
        // saw. It is also the half that a guard which simply swallowed every release
        // would get wrong, so the key nobody claimed is checked alongside it.
        let mut app = app();
        let _ = app.open_tab(80, 24).expect("a shell starts");
        let chord = Modifiers::CTRL | Modifiers::SHIFT;
        let release = KeyEvent {
            kind: KeyKind::Release,
            ..key(Key::Char('F'), chord)
        };

        assert!(app.owns(&release), "a bound chord is zet's on the way up");
        assert_eq!(
            app.action_for(&release),
            None,
            "and it runs nothing on the way up"
        );
        assert!(
            !app.owns(&KeyEvent {
                kind: KeyKind::Release,
                ..key(Key::Char('a'), Modifiers::empty())
            }),
            "an unbound key is still the shell's to receive"
        );

        // And a key nobody claimed does reach the program. Sending anything to it is what
        // returns the view to the bottom — that is `Session::write`'s own rule — so a view
        // that was scrolled back is the observation.
        app.active_mut().expect("a tab").scroll_to(3);
        let _ = app.key(&key(Key::Char('a'), Modifiers::empty()));
        assert_eq!(
            app.active().expect("a tab").scroll_offset(),
            0,
            "typing did not reach the program"
        );
    }

    #[test]
    fn tab_keys_wrap_around() {
        let mut app = app();
        let _ = app.open_tab(80, 24).expect("a shell starts");
        let _ = app.open_tab(80, 24).expect("a shell starts");
        assert_eq!(app.active_number(), Some(2));
        let _ = app.key(&key(Key::Tab, Modifiers::CTRL));
        assert_eq!(app.active_number(), Some(1), "next wraps forward");
        let _ = app.key(&key(Key::Tab, Modifiers::CTRL | Modifiers::SHIFT));
        assert_eq!(app.active_number(), Some(2), "previous wraps back");
    }

    #[test]
    fn closing_the_last_tab_asks_the_host_to_quit() {
        let mut app = app();
        let _ = app.open_tab(80, 24).expect("a shell starts");
        let commands = app.key(&key(Key::Char('W'), Modifiers::CTRL | Modifiers::SHIFT));
        assert_eq!(commands, vec![Command::Quit]);
        assert!(app.sessions().is_empty());
    }

    #[test]
    fn closing_one_of_two_tabs_does_not_ask_to_quit() {
        let mut app = app();
        let _ = app.open_tab(80, 24).expect("a shell starts");
        let _ = app.open_tab(80, 24).expect("a shell starts");
        let commands = app.key(&key(Key::Char('W'), Modifiers::CTRL | Modifiers::SHIFT));
        assert!(commands.is_empty());
        assert_eq!(app.tab_numbers(), vec![1]);
    }

    #[test]
    fn copy_with_nothing_selected_asks_for_nothing() {
        let mut app = app();
        let _ = app.open_tab(80, 24).expect("a shell starts");
        let commands = app.key(&key(Key::Char('C'), Modifiers::CTRL | Modifiers::SHIFT));
        assert!(commands.is_empty());
    }

    #[test]
    fn copying_a_blank_selection_leaves_the_clipboard_alone() {
        // A fresh shell's grid is blank. Putting the empty string on the clipboard would
        // throw away whatever was there, on the strength of a drag across nothing.
        let mut app = app();
        let _ = app.open_tab(80, 24).expect("a shell starts");
        app.select_from(Pos::new(0, 0));
        app.select_to(Pos::new(0, 2));
        let commands = app.key(&key(Key::Char('C'), Modifiers::CTRL | Modifiers::SHIFT));
        assert!(commands.is_empty());
    }

    #[test]
    fn a_click_with_no_drag_leaves_no_selection_behind() {
        let mut app = app();
        let _ = app.open_tab(80, 24).expect("a shell starts");
        app.select_from(Pos::new(2, 2));
        app.select_end();
        assert_eq!(app.selection(), None);
    }

    #[test]
    fn a_drag_leaves_a_selection_behind() {
        let mut app = app();
        let _ = app.open_tab(80, 24).expect("a shell starts");
        app.select_from(Pos::new(2, 2));
        app.select_to(Pos::new(3, 4));
        app.select_end();
        let selection = app.selection().expect("still selected");
        assert_eq!(selection.bounds(), ((2, 2), (3, 4)));
    }

    #[test]
    fn switching_tabs_throws_the_selection_away() {
        // A selection is a set of cells in one terminal, and the same coordinates in
        // another terminal are a different selection.
        let mut app = app();
        let _ = app.open_tab(80, 24).expect("a shell starts");
        let _ = app.open_tab(80, 24).expect("a shell starts");
        app.select_from(Pos::new(1, 1));
        app.select_to(Pos::new(2, 2));
        app.activate(1);
        assert_eq!(app.selection(), None);
    }

    #[test]
    fn the_tab_strip_flips_and_flips_back() {
        let mut app = app();
        assert_eq!(app.config().tabs.position, zet_config::TabPosition::Top);
        let _ = app.key(&key(Key::Char('P'), Modifiers::CTRL | Modifiers::SHIFT));
        assert_eq!(app.config().tabs.position, zet_config::TabPosition::Left);
        let _ = app.key(&key(Key::Char('P'), Modifiers::CTRL | Modifiers::SHIFT));
        assert_eq!(app.config().tabs.position, zet_config::TabPosition::Top);
    }

    #[test]
    fn the_font_size_nudge_moves_one_point_at_a_time_and_stops_at_the_limits() {
        let mut app = app();
        assert!((app.font_size() - 13.0).abs() < f32::EPSILON);
        let _ = app.key(&key(Key::Plus, Modifiers::CTRL));
        assert!((app.font_size() - 14.0).abs() < f32::EPSILON);
        let _ = app.key(&key(Key::Char('0'), Modifiers::CTRL));
        assert!((app.font_size() - 13.0).abs() < f32::EPSILON);

        for _ in 0..200 {
            let _ = app.key(&key(Key::Plus, Modifiers::CTRL));
        }
        assert!((app.font_size() - 72.0).abs() < f32::EPSILON);
        for _ in 0..200 {
            let _ = app.key(&key(Key::Minus, Modifiers::CTRL));
        }
        assert!((app.font_size() - 4.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_font_size_that_is_not_a_number_is_still_a_size() {
        // `nan` is a float TOML can spell, so `size = nan` is a file a user can write.
        // It is reported as out of range and it passes through `f32::clamp` unchanged —
        // NaN is not below the range, it is not above it, and clamp has no answer for
        // it but itself. From here it multiplies into the text scale and the font
        // loader refuses the face, so one line of the configuration is a terminal that
        // will not open, in a version whose own documentation says a bad file never
        // stops the app from starting.
        let mut app = app();
        app.config.font.size = f32::NAN;
        let size = app.font_size();
        assert!(
            size.is_finite(),
            "a NaN size reached the font loader: {size}"
        );
        assert!((settings::MIN_SIZE..=settings::MAX_SIZE).contains(&size));
        assert!((size - 13.0).abs() < f32::EPSILON, "{size}");
    }

    #[test]
    fn forced_contrast_overrides_the_configured_theme_and_can_be_turned_off_again() {
        let mut app = app();
        assert_eq!(app.theme().slug, "zet-dark");
        app.system_accessibility(false, true);
        assert_eq!(app.theme().slug, "zet-contrast");
        app.system_accessibility(false, false);
        assert_eq!(app.theme().slug, "zet-dark");
    }

    #[test]
    fn the_two_appearance_switches_are_what_stops_the_system_having_its_way() {
        // Each row in the panel that covers a system setting has to actually cover it,
        // or the panel is offering a choice it does not honour.
        let mut app = app();
        app.system_accessibility(true, true);
        assert!(app.reduce_motion(), "reduce motion is honoured by default");
        assert_eq!(app.theme().slug, "zet-contrast");

        app.adjust(settings::Id::ReduceMotion, false);
        assert!(!app.reduce_motion(), "the row turned it off");
        app.adjust(settings::Id::ForcedColors, false);
        assert_eq!(
            app.theme().slug,
            "zet-dark",
            "with forced colours switched off the user's theme comes back"
        );

        // And they stay off when the system reports the same thing again, which is the
        // case a value read once and cached would get wrong.
        app.system_accessibility(true, true);
        assert!(!app.reduce_motion());
        assert_eq!(app.theme().slug, "zet-dark");
    }

    #[test]
    fn the_contrast_floor_of_the_forced_theme_is_the_reason_it_exists() {
        let mut app = app();
        app.system_accessibility(false, true);
        let theme = app.theme();
        assert!(
            theme
                .foreground
                .contrasts_with(theme.background, zet_config::theme::ANSI_CONTRAST_FLOOR)
        );
    }

    #[test]
    fn a_blink_is_on_for_half_of_its_period_and_off_for_the_other_half() {
        let mut blink = Blink::new(Instant::now());
        let start = blink.epoch;
        // Hard transitions, no fade in between: the phase is a comparison rather than
        // an interpolation.
        assert!(blink.on(start));
        assert!(blink.on(start + Duration::from_millis(529)));
        assert!(!blink.on(start + Duration::from_millis(530)));
        assert!(!blink.on(start + Duration::from_millis(1059)));
        assert!(blink.on(start + Duration::from_millis(1060)));

        blink.restart(start, true);
        assert!(!blink.on(start), "a restart can start in the dark half");
    }

    #[test]
    fn the_next_blink_is_never_more_than_one_interval_away() {
        let app = app();
        let now = Instant::now();
        let next = app.next_blink(now);
        assert!(next > now);
        assert!(next - now <= BLINK_INTERVAL);
    }

    #[test]
    fn reduce_motion_turns_the_blink_off_without_touching_the_setting() {
        let mut app = app();
        assert!(app.blinks());
        app.reduce_motion = true;
        assert!(!app.blinks());
        assert!(
            app.config().cursor.blink,
            "the setting is the user's, not ours"
        );
    }

    #[test]
    fn a_drain_of_nothing_reports_nothing_changed() {
        let mut app = app();
        let _ = app.open_tab(80, 24).expect("a shell starts");
        assert!(!app.pump());
    }

    #[test]
    fn a_tab_switch_throws_the_find_results_away() {
        let mut app = app();
        let first = app.open_tab(80, 24).expect("a shell starts");
        app.find.open();
        app.find.push('a');
        // One search, so the bar holds a settled match list. The grid is a local one for
        // the same reason the fixture below uses one: the bar is being driven directly
        // and a shell that prints on demand is a race.
        let mut term = zet_vt::Term::new(20, 4);
        let mut parser = zet_vt::Parser::new();
        parser.advance_slice(b"alpha", &mut term);
        app.find.search(term.grid(), 0);
        assert!(!app.find.is_stale(), "a search settles the bar");

        let _ = app.open_tab(80, 24).expect("a shell starts");
        app.activate(first);

        assert!(
            app.find.is_stale(),
            "the matches and the count are the previous terminal's"
        );
    }

    #[test]
    fn closing_a_tab_throws_the_find_results_away_too() {
        // The same event as a tab switch, through the other door: the matches are
        // positions in the history of the terminal that just went away, and `find_marks`
        // paints them by row onto whatever is active now. A highlight and a "1 of 1" over
        // a terminal that never contained the query is the thing the switch guard exists
        // to prevent, and closing the tab reaches the same state without going through it.
        let mut app = app();
        let _first = app.open_tab(80, 24).expect("a shell starts");
        let second = app.open_tab(80, 24).expect("a shell starts");
        let mut app = settled_find(app);
        assert_eq!(
            app.active_number(),
            Some(second),
            "the bar is over this one"
        );

        let _ = app.close_tab(second);

        assert!(
            app.find.is_stale(),
            "the matches belong to the tab that closed"
        );
    }

    #[test]
    fn closing_a_tab_the_user_is_not_looking_at_leaves_the_find_alone() {
        // The other half of the rule above, and the reason `close_tab` asks which tab
        // went rather than touching the bar for every close: a tab in the background is
        // not the grid the marks were found in, and searching the whole history again
        // because a tab somewhere else was closed is work with nothing behind it.
        let mut app = app();
        let first = app.open_tab(80, 24).expect("a shell starts");
        let second = app.open_tab(80, 24).expect("a shell starts");
        let mut app = settled_find(app);
        assert_eq!(app.active_number(), Some(second));

        let _ = app.close_tab(first);

        assert!(
            !app.find.is_stale(),
            "the terminal under the query did not move"
        );
    }

    #[test]
    fn a_reaped_tab_throws_the_find_results_away_as_well() {
        // The third door: a shell that exits is removed by the host's reap, and the tab
        // that becomes active is not the one the match list was built from.
        let mut app = app();
        let _first = app.open_tab(80, 24).expect("a shell starts");
        let _second = app.open_tab(80, 24).expect("a shell starts");
        let mut app = settled_find(app);

        app.active()
            .expect("a tab is open")
            .write(b"exit\r\n")
            .expect("the shell is told to go");
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut reaped = Vec::new();
        while reaped.is_empty() {
            app.pump();
            // Settle the bar again every time round, because the shell echoing the
            // command it is given is output, and `pump` is right to touch the bar for
            // it. Without this the assertion below would be reading the echo's touch
            // rather than reap's, and would pass on a `reap` that had done nothing.
            app = settled_find(app);
            reaped = app.reap();
            assert!(Instant::now() < deadline, "the shell never exited");
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(
            app.find.is_stale(),
            "the grid under the query went with the tab"
        );
    }

    #[test]
    fn a_resize_throws_the_find_results_away_as_well() {
        // And the fourth: a narrowing resize re-splits every logical line, so every row
        // below one that changed shape moves and the positions the bar is holding are
        // positions in a grid that no longer exists.
        let mut app = app();
        let _ = app.open_tab(80, 24).expect("a shell starts");
        let mut app = settled_find(app);

        app.resize(40, 24);

        assert!(
            app.find.is_stale(),
            "every row below a re-wrapped line has moved"
        );
    }

    /// An app with the find bar open, a query typed, and one search run over a grid the
    /// test owns.
    ///
    /// A local grid for the reason `a_tab_switch_throws_the_find_results_away` gives: the
    /// bar is being driven directly, and a shell that prints on demand is a race.
    fn settled_find(mut app: App) -> App {
        app.find.open();
        app.find.push('a');
        let mut term = zet_vt::Term::new(20, 4);
        let mut parser = zet_vt::Parser::new();
        parser.advance_slice(b"alpha", &mut term);
        app.find.search(term.grid(), 0);
        assert!(!app.find.is_stale(), "a search settles the bar");
        app
    }

    #[test]
    fn resizing_tells_every_session() {
        let mut app = app();
        let _ = app.open_tab(80, 24).expect("a shell starts");
        let _ = app.open_tab(80, 24).expect("a shell starts");
        app.resize(100, 30);
        for number in app.tab_numbers() {
            let session = app.sessions().get(number).expect("open");
            assert_eq!((session.cols(), session.rows()), (100, 30));
        }
    }

    #[test]
    fn a_tab_that_will_not_open_is_reported_rather_than_dropped() {
        // The failure whose whole symptom is that nothing happened. There are three
        // doors into it — the `+`, the chord, and a row of the picker — and all three
        // dropped the answer, which leaves the user pressing the button again with
        // nothing to say whether that is worth doing.
        let mut app = app();
        assert!(
            app.new_tab(80, 24).is_empty(),
            "a tab that opens says nothing"
        );

        let commands = Command::unopened(app.open_tab_with("no-such-profile", 80, 24));
        match commands.as_slice() {
            [Command::Report(message)] => {
                assert!(message.contains("no-such-profile"), "{message}");
            }
            other => panic!("expected one report, got {other:?}"),
        }
    }

    #[test]
    fn a_new_tab_is_the_size_the_last_one_was() {
        let mut app = app();
        let _ = app.open_tab(120, 40).expect("a shell starts");
        let _ = app.key(&key(Key::Char('T'), Modifiers::CTRL | Modifiers::SHIFT));
        let session = app.active().expect("the new tab");
        assert_eq!((session.cols(), session.rows()), (120, 40));
    }

    // ---------------------------------------------------------------------------
    // The find bar
    // ---------------------------------------------------------------------------

    /// The bar, with a query typed into it and a grid searched for matches.
    ///
    /// The grid is fed straight into the bar rather than through a session: what is
    /// being tested is the bar's half of the pair — the query, the cursor, and the
    /// keys it takes — and a real shell printing `beta` on demand is a race.
    fn finding(query: &str, lines: &[&str], rows: usize) -> App {
        let mut app = app();
        let mut term = zet_vt::Term::new(20, rows);
        let mut parser = zet_vt::Parser::new();
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                parser.advance_slice(b"\r\n", &mut term);
            }
            parser.advance_slice(line.as_bytes(), &mut term);
        }
        app.find.open();
        for ch in query.chars() {
            app.find.push(ch);
        }
        app.find.search(term.grid(), 0);
        app
    }

    fn match_at(start: (usize, usize), end: (usize, usize)) -> Match {
        Match {
            start: Pos::new(start.0, start.1),
            end: Pos::new(end.0, end.1),
        }
    }

    #[test]
    fn enter_goes_forward_and_shift_enter_goes_back() {
        // Both are one key, and the shipped bug had them the wrong way round, which is
        // the kind of thing that reads as working until you are on the third match.
        let mut app = finding("beta", &["beta one", "beta two", "beta three"], 8);
        assert!(app.find_key(&key(Key::Enter, Modifiers::empty())));
        assert_eq!(app.find().position(), Some(2), "plain Enter went backwards");
        assert!(app.find_key(&key(Key::Enter, Modifiers::SHIFT)));
        assert_eq!(app.find().position(), Some(1), "Shift+Enter went forwards");
    }

    #[test]
    fn the_find_bar_takes_the_space_bar_as_text() {
        // The space bar is a named key rather than a character on this platform, so a
        // bar that only reads `event.text` from character keys cannot hold a space and
        // the key falls through to the shell underneath.
        let mut app = finding("", &["hello world"], 8);
        let event = KeyEvent {
            text: Some(" ".to_string()),
            ..key(Key::Space, Modifiers::empty())
        };
        assert!(app.find_key(&event));
        assert_eq!(app.find().query(), " ");
    }

    #[test]
    fn a_match_that_starts_above_the_viewport_starts_at_the_first_visible_column() {
        // The bug this holds down: the start column was carried over from a row nobody
        // can see, and when the whole match mapped onto one row the corners came out
        // reversed and the highlight painted the gap between them instead.
        let found = match_at((1, 12), (2, 3));
        let mark = clip_match(found, 2, 10, 80);
        assert_eq!(mark.bounds(), ((0, 0), (0, 3)));
    }

    #[test]
    fn a_match_that_ends_below_the_viewport_stops_at_the_last_visible_column() {
        let found = match_at((8, 4), (12, 6));
        let mark = clip_match(found, 0, 10, 80);
        assert_eq!(mark.bounds(), ((8, 4), (9, 79)));
    }

    #[test]
    fn a_match_inside_the_viewport_is_moved_and_not_clipped() {
        let found = match_at((5, 2), (5, 6));
        let mark = clip_match(found, 3, 10, 80);
        assert_eq!(mark.bounds(), ((2, 2), (2, 6)));
    }

    #[test]
    fn a_match_already_on_screen_is_not_a_reason_to_scroll() {
        let found = match_at((4, 2), (4, 6));
        assert_eq!(reveal_row(found, 0, 10), None);
        assert_eq!(reveal_row(found, 4, 10), None, "it is on the top row");
        assert_eq!(reveal_row(found, 4, 1), None, "it is the only row");
    }

    #[test]
    fn a_match_above_the_fold_is_put_on_the_top_row() {
        let found = match_at((2, 2), (3, 6));
        assert_eq!(reveal_row(found, 5, 10), Some(2));
    }

    #[test]
    fn a_match_below_the_fold_is_put_on_the_bottom_row() {
        let found = match_at((20, 2), (21, 6));
        // Row 12 at the top puts row 21 — the match's last — on the bottom of ten.
        assert_eq!(reveal_row(found, 5, 10), Some(12));
    }

    #[test]
    fn a_match_taller_than_the_window_is_shown_from_its_first_row() {
        // The bug this holds down: `(end + 1) - rows` with no case for a match taller
        // than the window scrolls its start off the top, so following a match the user
        // can only half see hides the half they were following.
        let found = match_at((10, 0), (40, 0));
        assert_eq!(reveal_row(found, 0, 10), Some(10));
    }
}
