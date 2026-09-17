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
use zet_input::{Chord, KeyEvent, encode_key, encode_paste};
use zet_pty::discovery::{self, Profile};
use zet_render::Selection;
use zet_session::{Session, SessionError, Sessions, Waker};
use zet_vt::{Modes, Pos};

use crate::action::Action;
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
    /// The last tab closed. The host decides whether that closes the window.
    Quit,
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
    /// How much the font size has been nudged from the configured one, in points.
    font_nudge: f32,
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
            font_nudge: 0.0,
        })
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

    /// The tab numbers, in creation order.
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
    #[must_use]
    pub fn font_size(&self) -> f32 {
        (self.config.font.size + self.font_nudge).clamp(4.0, 72.0)
    }

    // ---------------------------------------------------------------------------
    // Tabs
    // ---------------------------------------------------------------------------

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
        let number = self
            .sessions
            .open(&profile, cols, rows, None, Arc::clone(&self.waker))?;
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
        let result = self.sessions.close(number);
        self.drag = None;
        result.map(|()| self.sessions.is_empty())
    }

    /// Make a tab active.
    pub fn activate(&mut self, number: u32) {
        if self.sessions.get(number).is_some() {
            self.sessions.set_active(number);
            // A selection belongs to the terminal it was made in.
            self.drag = None;
            self.blink.restart(Instant::now(), false);
        }
    }

    /// Drop the tabs whose shells have exited, and report which numbers went.
    ///
    /// Called after a drain. A tab is not closed the moment its shell exits — a terminal
    /// shows what a program printed on its way out, and the user is the one who decides
    /// the output has been read.
    pub fn reap(&mut self) -> Vec<u32> {
        self.sessions.reap()
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
        if let Some(action) = self.bound(event.mods, event.key) {
            return self.run(action);
        }
        self.type_into_session(event);
        Vec::new()
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
    /// The kitty keyboard protocol's `CSI u` disambiguation belongs here rather than in
    /// the encoder, because whether a chord is a zet shortcut or a byte for the program
    /// is zet's decision and not the terminal's. It is not implemented: a binding is
    /// matched on the legacy encoding alone, so a program that negotiates for the
    /// protocol sees the same shortcut behaviour as one that does not.
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
                let (cols, rows) = self.grid_size();
                let _ = self.open_tab(cols, rows);
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
                self.scroll(i32::MIN);
                Vec::new()
            }
            Action::ScrollToBottom => {
                if let Some(session) = self.active_mut() {
                    session.scroll_to_bottom();
                }
                Vec::new()
            }
            Action::FontLarger => {
                self.font_nudge = (self.font_nudge + 1.0).min(72.0 - self.config.font.size);
                Vec::new()
            }
            Action::FontSmaller => {
                self.font_nudge = (self.font_nudge - 1.0).max(4.0 - self.config.font.size);
                Vec::new()
            }
            Action::FontReset => {
                self.font_nudge = 0.0;
                Vec::new()
            }
            // The two overlays are drawn by the window rather than by this crate, and
            // the state that says whether they are open is theirs too. Until they exist,
            // the binding does nothing rather than pretending.
            Action::Find | Action::Settings => Vec::new(),
        }
    }

    fn scroll(&mut self, delta: i32) -> Vec<Command> {
        if let Some(session) = self.active_mut() {
            session.scroll(delta);
        }
        Vec::new()
    }

    /// The text the user has selected, if there is one.
    #[must_use]
    pub fn selection_text(&self) -> Option<String> {
        let selection = self.selection()?;
        let term = self.active()?.term();
        Some(selection_text(term, selection))
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
        changed
    }

    /// Tell every session how big the grid is.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        for number in self.sessions.numbers() {
            if let Some(session) = self.sessions.get_mut(number) {
                let _ = session.resize(cols, rows);
            }
        }
    }

    /// The grid size of the active session, or a sensible default when none is open.
    fn grid_size(&self) -> (u16, u16) {
        self.active()
            .map_or((80, 24), |session| (session.cols(), session.rows()))
    }

    /// Switch to the contrast theme and stay there, because the system asked for it.
    ///
    /// Forced colours are an accessibility requirement rather than a preference, so this
    /// is not a setting the user can have set against them at runtime: it overrides
    /// whatever the config says for as long as it is on.
    pub fn force_contrast(&mut self, forced: bool) {
        self.forced_contrast = forced;
        self.theme = if forced {
            contrast_theme()
        } else {
            by_slug(&self.config.theme).unwrap_or_else(|| zet_config::default_theme())
        };
    }
}

/// Turn the config file's `[keys]` table into chords.
///
/// An entry that cannot be parsed is dropped rather than fatal. The config loader has
/// already reported the ones it could see, and a keybinding is not worth refusing to
/// start over.
fn parse_bindings(config: &Config) -> Vec<(Chord, Action)> {
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
    fn forced_contrast_overrides_the_configured_theme_and_can_be_turned_off_again() {
        let mut app = app();
        assert_eq!(app.theme().slug, "zet-dark");
        app.force_contrast(true);
        assert_eq!(app.theme().slug, "zet-contrast");
        app.force_contrast(false);
        assert_eq!(app.theme().slug, "zet-dark");
    }

    #[test]
    fn the_contrast_floor_of_the_forced_theme_is_the_reason_it_exists() {
        let mut app = app();
        app.force_contrast(true);
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
    fn a_new_tab_is_the_size_the_last_one_was() {
        let mut app = app();
        let _ = app.open_tab(120, 40).expect("a shell starts");
        let _ = app.key(&key(Key::Char('T'), Modifiers::CTRL | Modifiers::SHIFT));
        let session = app.active().expect("the new tab");
        assert_eq!((session.cols(), session.rows()), (120, 40));
    }
}
