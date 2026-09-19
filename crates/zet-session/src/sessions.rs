//! Several running terminals, and the numbers that name them.
//!
//! # Why a tab number is a position
//!
//! The numbers run `1..=N`, with no gaps and no reuse. Three tabs read `#1`, `#2`, `#3`;
//! close the first and what is left reads `#1`, `#2`; the next tab opened is `#3`. A number
//! is where a tab sits in the strip, and nothing else.
//!
//! The rule this replaced handed a number out at creation and never used it again, so the
//! same three tabs closed the same way were `#2` and `#3`: two tabs with a hole in front of
//! them. What that bought was a number that could be held on to, and it was paid for in the
//! one place the number is read — the strip, where a gap is indistinguishable from a tab
//! that failed to open. Nothing in zet addresses a tab by number: not a keybinding, not a
//! command, not a log line. There was nothing to hold on to.
//!
//! The cost is that a number is not an identity — a tab moves down a number when a tab in
//! front of it closes — and two things follow, both of them in this module. The active mark
//! is kept as a *place* rather than as a number, so that closing a tab in front of the one
//! on screen cannot quietly move the user to a different terminal; and [`Sessions::reap`]
//! answers with the numbers the tabs *were* shown under, because the caller comparing them
//! against a number it read before anything moved is the whole reason it asks.
//!
//! # Where the bookkeeping lives
//!
//! [`Tabs`] holds the order, the numbers, and the active mark, and it is generic over what
//! a tab is so that every rule above is tested against a handful of numbers: no pty, no
//! console, no child process. [`Sessions`] is that list with a [`Session`] in it.

use std::path::PathBuf;
use std::sync::Arc;

use zet_pty::discovery::Profile;

use crate::session::{Session, SessionError, Waker};

/// Every open session, in the order the tab strip shows them.
///
/// Dropping this closes every session, which kills every child. There is no other way to
/// release the collection, and that is the point: a terminal whose tabs outlive it is a
/// process leak with a window in front of it.
#[derive(Default)]
pub struct Sessions {
    tabs: Tabs<Session>,
}

impl Sessions {
    /// No sessions.
    #[must_use]
    pub fn new() -> Self {
        Sessions::default()
    }

    /// Open a session and return its tab number.
    ///
    /// The new tab becomes the active one, which is what a host would do next anyway and
    /// what the alternative — an `active` that is `None` until the host says otherwise —
    /// would only make it restate.
    ///
    /// # Errors
    ///
    /// The same ones [`Session::spawn`] reports. A profile that cannot be launched does not
    /// consume a number, and now cannot: the tab is added to the list once there is a
    /// session to put in it, and a number is a place in that list.
    pub fn open(
        &mut self,
        profile: &Profile,
        cols: u16,
        rows: u16,
        cwd: Option<PathBuf>,
        waker: Arc<dyn Waker>,
    ) -> Result<u32, SessionError> {
        let session = Session::spawn(profile, cols, rows, cwd, waker)?;
        Ok(self.tabs.open(session))
    }

    /// Close a session and kill its child.
    ///
    /// Every tab above it moves down a number, which is what keeps the strip reading
    /// `#1..=N`.
    ///
    /// # Errors
    ///
    /// Fails if there is no session with that number, or if the child could not be
    /// killed. The tab is gone either way; the error is about what happened to the
    /// process, not about the bookkeeping.
    pub fn close(&mut self, number: u32) -> Result<(), SessionError> {
        let Some(session) = self.tabs.close(number) else {
            return Err(SessionError::NoSession(number));
        };
        session.close()
    }

    /// The session with this number, if it is still open.
    #[must_use]
    pub fn get(&self, number: u32) -> Option<&Session> {
        self.tabs.get(number)
    }

    /// The session with this number, mutably, if it is still open.
    #[must_use]
    pub fn get_mut(&mut self, number: u32) -> Option<&mut Session> {
        self.tabs.get_mut(number)
    }

    /// Every open tab number, in the order the strip shows them.
    ///
    /// `1..=N`, always. That is the whole of what a positional number buys: there is no
    /// list of numbers that could fall out of step with the list of sessions.
    #[must_use]
    pub fn numbers(&self) -> Vec<u32> {
        self.tabs.numbers()
    }

    /// How many sessions are open.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    /// Whether there are no sessions at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// The tab the host should be showing, if any.
    #[must_use]
    pub fn active(&self) -> Option<u32> {
        self.tabs.active()
    }

    /// Make a tab the active one.
    ///
    /// A number that is not open is refused rather than remembered. Storing it would make
    /// `active` name a tab that is not there, and the host would have nothing to draw;
    /// keeping the current tab is the only answer that leaves the host with a valid one.
    pub fn set_active(&mut self, number: u32) {
        self.tabs.set_active(number);
    }

    /// The tab after `number` in the order the strip shows them, wrapping around.
    ///
    /// `None` when there is only one session: wrapping would name the one already on
    /// screen, and a host that switched to it would be doing nothing at the cost of a
    /// frame.
    #[must_use]
    pub fn next_number(&self, number: u32) -> Option<u32> {
        self.tabs.next_number(number)
    }

    /// The tab before `number` in the order the strip shows them, wrapping around.
    ///
    /// `None` under the same rule as [`Sessions::next_number`].
    #[must_use]
    pub fn previous_number(&self, number: u32) -> Option<u32> {
        self.tabs.previous_number(number)
    }

    /// Remove every session whose child has exited, and report the numbers that went
    /// away.
    ///
    /// The numbers are the ones those tabs were shown under *before* the reap. The tabs
    /// behind them are renumbered by it, so a caller asking "has the tab I was looking at
    /// gone?" is holding a number from before, and an answer in today's numbers would be
    /// an answer about a different terminal.
    ///
    /// Removing the session is what releases it: the pump stops, the pseudoconsole is
    /// closed, and the child — already gone — is reaped. Nothing here decides that an
    /// exited tab should disappear from under the user; that is a host's call, and this
    /// is the mechanism it calls once it has made it.
    pub fn reap(&mut self) -> Vec<u32> {
        self.tabs.reap(Session::is_exited)
    }

    /// Every session, in the order the strip shows them.
    pub fn iter(&self) -> impl Iterator<Item = &Session> + '_ {
        self.tabs.iter()
    }
}

/// A list of tabs: what is open, in the order they are shown, and which one is active.
///
/// Generic over what a tab is so that the rules can be tested against a `u8`. There is no
/// list of numbers here, because a tab's number is its place in this list; a struct holding
/// both would hold two answers to one question.
#[derive(Debug)]
struct Tabs<T> {
    /// The tabs, oldest first. A tab's number is its place here, plus one.
    open: Vec<T>,
    /// The tab the host should be showing, as a place in `open`.
    active: Option<usize>,
}

impl<T> Default for Tabs<T> {
    fn default() -> Self {
        Self {
            open: Vec::new(),
            active: None,
        }
    }
}

impl<T> Tabs<T> {
    /// Add a tab at the end, make it the active one, and answer with its number.
    fn open(&mut self, tab: T) -> u32 {
        self.open.push(tab);
        self.active = Some(self.open.len() - 1);
        number_at(self.open.len() - 1)
    }

    /// Take a tab out and answer with it, or `None` when no tab is shown under that
    /// number.
    ///
    /// The tabs above the gap move down a number, and the active mark follows the tab
    /// rather than the number: the host goes on showing the same terminal, under whatever
    /// number that terminal now has.
    fn close(&mut self, number: u32) -> Option<T> {
        let index = self.index(number)?;
        let tab = self.open.remove(index);
        self.active = match self.active {
            // The tab that went was the one on screen. The tab that slid into its place
            // takes the mark, and when there is no such tab the one on its left does —
            // which is the same thing said once: the place, clamped to the last there is.
            Some(active) if active == index => {
                self.open.len().checked_sub(1).map(|last| index.min(last))
            }
            // The tab on screen was above the gap and has moved down one.
            Some(active) if active > index => Some(active - 1),
            // Below the gap, so nothing about it changed — including there being no tab on
            // screen at all.
            other => other,
        };
        Some(tab)
    }

    /// The tab shown under a number, if there is one.
    fn get(&self, number: u32) -> Option<&T> {
        self.open.get(self.index(number)?)
    }

    /// The tab shown under a number, mutably, if there is one.
    fn get_mut(&mut self, number: u32) -> Option<&mut T> {
        let index = self.index(number)?;
        self.open.get_mut(index)
    }

    /// Every number, in the order the tabs are shown.
    fn numbers(&self) -> Vec<u32> {
        (0..self.open.len()).map(number_at).collect()
    }

    /// How many are open.
    fn len(&self) -> usize {
        self.open.len()
    }

    /// Whether none are open.
    fn is_empty(&self) -> bool {
        self.open.is_empty()
    }

    /// The number of the tab on screen, if there is one.
    fn active(&self) -> Option<u32> {
        self.active.map(number_at)
    }

    /// Put the active mark on a number, refusing one that names no tab.
    fn set_active(&mut self, number: u32) {
        if let Some(index) = self.index(number) {
            self.active = Some(index);
        }
    }

    /// The number after this one, wrapping, or `None` when there is nowhere to go.
    fn next_number(&self, number: u32) -> Option<u32> {
        self.wrapped(number, 1)
    }

    /// The number before this one, wrapping, or `None` when there is nowhere to go.
    fn previous_number(&self, number: u32) -> Option<u32> {
        self.wrapped(number, self.open.len().saturating_sub(1))
    }

    /// The tab `by` places on from a number, wrapping.
    ///
    /// One place forward and one place back are the same arithmetic with a different
    /// offset, and `len - 1` places forward is one place back, because places count from
    /// zero. `None` for a number that names no tab, and for a list with only one tab in it:
    /// the next tab there would be the one already on screen, at the cost of a frame.
    fn wrapped(&self, number: u32, by: usize) -> Option<u32> {
        let index = self.index(number)?;
        let len = self.open.len();
        (len > 1).then(|| number_at((index + by) % len))
    }

    /// Remove every tab the predicate names, and answer with the numbers they were shown
    /// under.
    ///
    /// The numbers as they were, not as they end up: a caller asking whether the tab it was
    /// looking at went away read that number before any of this moved.
    fn reap(&mut self, gone: impl Fn(&T) -> bool) -> Vec<u32> {
        let doomed: Vec<usize> = self
            .open
            .iter()
            .enumerate()
            .filter(|(_, tab)| gone(tab))
            .map(|(index, _)| index)
            .collect();
        let numbers = doomed.iter().copied().map(number_at).collect();
        // From the top down, so that the places of the tabs still to go do not move under
        // the loop as the ones above them are removed.
        for index in doomed.into_iter().rev() {
            self.close(number_at(index));
        }
        numbers
    }

    /// Every tab, in the order they are shown.
    fn iter(&self) -> impl Iterator<Item = &T> + '_ {
        self.open.iter()
    }

    /// The place a number names, if there is a tab there.
    fn index(&self, number: u32) -> Option<usize> {
        let place = usize::try_from(number.checked_sub(1)?).ok()?;
        (place < self.open.len()).then_some(place)
    }
}

/// The number a tab in this place is shown under.
///
/// One-based, because the strip draws `#1` for the first tab: a zero-based place would have
/// to be corrected at every place that names a tab rather than at the one place that makes
/// a number out of a place. Saturating rather than wrapping, because a window cannot hold
/// four billion tabs and a wrapped number would name a tab that is not there.
fn number_at(index: usize) -> u32 {
    u32::try_from(index + 1).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three tabs holding 10, 20, and 30, shown as `#1`, `#2`, and `#3`.
    ///
    /// A `u8` rather than a [`Session`], because everything these tests are about is the
    /// number a tab is shown under and which of them the mark is on — questions a child
    /// process, a pseudoconsole, and a read loop would add nothing to and a great deal of
    /// time to.
    fn three() -> Tabs<u8> {
        let mut tabs = Tabs::default();
        for tab in [10, 20, 30] {
            tabs.open(tab);
        }
        tabs
    }

    #[test]
    fn numbers_start_at_one_and_climb_in_creation_order() {
        let mut tabs = Tabs::default();
        assert_eq!(tabs.open(10), 1);
        assert_eq!(tabs.open(20), 2);
        assert_eq!(tabs.open(30), 3);
        assert_eq!(tabs.numbers(), vec![1, 2, 3]);
        assert_eq!(tabs.len(), 3);
        assert!(!tabs.is_empty());
        assert_eq!(tabs.get(1), Some(&10));
        assert_eq!(tabs.get(3), Some(&30));
    }

    #[test]
    fn nothing_is_open_to_begin_with() {
        let tabs: Tabs<u8> = Tabs::default();
        assert!(tabs.is_empty());
        assert_eq!(tabs.len(), 0);
        assert_eq!(tabs.active(), None);
        assert_eq!(tabs.numbers(), Vec::<u32>::new());
        assert_eq!(tabs.get(1), None);
        assert_eq!(tabs.next_number(1), None);
        assert_eq!(tabs.previous_number(1), None);
        assert_eq!(tabs.iter().count(), 0);
    }

    #[test]
    fn closing_the_first_tab_makes_the_second_the_first() {
        // The case the rule is written for: three tabs read #1, #2, #3, and after the first
        // one goes the strip has to read #1, #2 — not #2 and #3 with a gap in front.
        let mut tabs = three();
        assert_eq!(tabs.close(1), Some(10));
        assert_eq!(tabs.numbers(), vec![1, 2]);
        assert_eq!(tabs.get(1), Some(&20), "what was #2 is #1 now");
        assert_eq!(tabs.get(2), Some(&30), "and what was #3 is #2");
    }

    #[test]
    fn closing_from_the_middle_closes_the_gap_behind_it() {
        let mut tabs = three();
        assert_eq!(tabs.close(2), Some(20));
        assert_eq!(tabs.numbers(), vec![1, 2]);
        assert_eq!(tabs.get(1), Some(&10));
        assert_eq!(tabs.get(2), Some(&30));
    }

    #[test]
    fn closing_the_last_tab_moves_nothing() {
        let mut tabs = three();
        assert_eq!(tabs.close(3), Some(30));
        assert_eq!(tabs.numbers(), vec![1, 2]);
        assert_eq!(tabs.get(2), Some(&20));
    }

    #[test]
    fn a_number_names_the_tab_that_is_at_it_now() {
        // The cost of a positional number, written down as a test rather than left for
        // someone to be surprised by: after a close, #2 is whoever is second, which is not
        // the tab that was second before it.
        let mut tabs = three();
        tabs.close(1);
        assert_eq!(tabs.close(2), Some(30));
        assert_eq!(tabs.numbers(), vec![1]);
        assert_eq!(tabs.get(1), Some(&20));
    }

    #[test]
    fn closing_a_number_that_names_no_tab_changes_nothing() {
        let mut tabs = three();
        assert_eq!(tabs.close(4), None);
        assert_eq!(tabs.close(0), None);
        assert_eq!(tabs.close(99), None);
        assert_eq!(tabs.numbers(), vec![1, 2, 3]);
        assert_eq!(tabs.len(), 3);
    }

    #[test]
    fn the_active_mark_follows_the_terminal_and_not_the_number() {
        // The whole reason the mark is kept as a place rather than as a number. The third
        // tab is on screen and the first one closes: the terminal on screen is still the
        // third one, which the strip now calls #2. A mark kept as a number would have moved
        // to a different terminal without the user touching anything.
        let mut tabs = three();
        tabs.set_active(3);
        tabs.close(1);
        assert_eq!(tabs.active(), Some(2));
        assert_eq!(tabs.get(2), Some(&30));
    }

    #[test]
    fn closing_the_active_tab_activates_the_one_that_took_its_place() {
        let mut tabs = three();
        tabs.set_active(2);
        tabs.close(2);
        assert_eq!(tabs.active(), Some(2));
        assert_eq!(
            tabs.get(2),
            Some(&30),
            "the tab that slid in takes the mark"
        );

        let mut tabs = three();
        tabs.set_active(3);
        tabs.close(3);
        assert_eq!(
            tabs.active(),
            Some(2),
            "the last tab closes to the one on its left"
        );

        let mut tabs = three();
        tabs.set_active(1);
        tabs.close(1);
        assert_eq!(tabs.active(), Some(1));
    }

    #[test]
    fn closing_a_tab_that_is_not_active_leaves_the_active_one_alone() {
        let mut tabs = three();
        tabs.set_active(1);
        tabs.close(3);
        assert_eq!(tabs.active(), Some(1));
        assert_eq!(tabs.get(1), Some(&10));
    }

    #[test]
    fn the_active_tab_can_be_changed_and_a_number_that_names_none_is_refused() {
        let mut tabs = three();
        tabs.set_active(1);
        assert_eq!(tabs.active(), Some(1));
        tabs.set_active(99);
        assert_eq!(
            tabs.active(),
            Some(1),
            "a number that names no tab is refused"
        );
        tabs.set_active(0);
        assert_eq!(tabs.active(), Some(1));
    }

    #[test]
    fn closing_the_only_tab_leaves_nothing_active() {
        let mut tabs = Tabs::default();
        tabs.open(10);
        assert_eq!(tabs.close(1), Some(10));
        assert_eq!(tabs.active(), None);
        assert!(tabs.is_empty());
        assert_eq!(tabs.numbers(), Vec::<u32>::new());
    }

    #[test]
    fn next_and_previous_wrap_around_the_ends() {
        let tabs = three();
        assert_eq!(tabs.next_number(1), Some(2));
        assert_eq!(tabs.next_number(2), Some(3));
        assert_eq!(
            tabs.next_number(3),
            Some(1),
            "the last tab wraps to the first"
        );
        assert_eq!(tabs.previous_number(1), Some(3));
        assert_eq!(tabs.previous_number(2), Some(1));
        assert_eq!(tabs.previous_number(3), Some(2));
    }

    #[test]
    fn there_is_no_next_tab_when_only_one_is_open() {
        let mut tabs = Tabs::default();
        tabs.open(10);
        assert_eq!(tabs.next_number(1), None);
        assert_eq!(tabs.previous_number(1), None);
    }

    #[test]
    fn next_and_previous_from_a_number_that_names_no_tab_are_none() {
        let tabs = three();
        assert_eq!(tabs.next_number(99), None);
        assert_eq!(tabs.next_number(0), None);
        assert_eq!(tabs.previous_number(99), None);
        assert_eq!(tabs.previous_number(0), None);
    }

    #[test]
    fn reaping_answers_with_the_numbers_the_tabs_were_shown_under() {
        // The numbers as they were, not as they ended up. The caller compares them against
        // a number it read before the reap, so what it needs back is which of the numbers
        // it was looking at are gone.
        let mut tabs = three();
        assert_eq!(tabs.reap(|tab| *tab == 20), vec![2]);
        assert_eq!(tabs.numbers(), vec![1, 2]);
        assert_eq!(tabs.get(2), Some(&30));
    }

    #[test]
    fn reaping_more_than_one_does_not_shift_the_places_under_the_loop() {
        let mut tabs = Tabs::default();
        for tab in [10, 20, 30, 40] {
            tabs.open(tab);
        }
        assert_eq!(tabs.reap(|tab| *tab == 10 || *tab == 30), vec![1, 3]);
        assert_eq!(tabs.numbers(), vec![1, 2]);
        assert_eq!(tabs.get(1), Some(&20));
        assert_eq!(tabs.get(2), Some(&40));
    }

    #[test]
    fn reaping_nothing_removes_nothing() {
        let mut tabs = three();
        assert!(tabs.reap(|_| false).is_empty());
        assert_eq!(tabs.numbers(), vec![1, 2, 3]);
    }

    #[test]
    fn reaping_a_tab_in_front_of_the_active_one_leaves_the_mark_where_it_was() {
        let mut tabs = three();
        tabs.set_active(3);
        assert_eq!(tabs.reap(|tab| *tab == 10), vec![1]);
        assert_eq!(tabs.active(), Some(2));
        assert_eq!(tabs.get(2), Some(&30));
    }

    #[test]
    fn reaping_the_active_tab_moves_the_active_mark() {
        let mut tabs = three();
        tabs.set_active(2);
        assert_eq!(tabs.reap(|tab| *tab == 20), vec![2]);
        assert_eq!(tabs.active(), Some(2));
        assert_eq!(tabs.get(2), Some(&30));
    }

    #[test]
    fn reaping_everything_leaves_an_empty_collection() {
        let mut tabs = three();
        assert_eq!(tabs.reap(|_| true), vec![1, 2, 3]);
        assert!(tabs.is_empty());
        assert_eq!(tabs.active(), None);
    }

    #[test]
    fn a_tab_can_be_got_at_mutably_by_its_number() {
        let mut tabs = three();
        if let Some(tab) = tabs.get_mut(2) {
            *tab = 99;
        }
        assert_eq!(tabs.get(2), Some(&99));
        assert_eq!(tabs.get_mut(4), None);
    }

    #[test]
    fn the_tabs_iterate_in_the_order_they_are_shown() {
        let tabs = three();
        assert_eq!(tabs.iter().copied().collect::<Vec<u8>>(), vec![10, 20, 30]);
    }
}
