//! Several running terminals, and the numbers that name them.
//!
//! # Why a tab number is a creation ordinal
//!
//! A number is handed out once and never reused. Close #2 of three and the tabs left are
//! #1 and #3 — not #1 and #2 — and the next tab opened is #4. The point of the rule is
//! that a number can be held on to: in a keybinding, in a log line, in the answer to "put
//! this in tab 3". Anything that renumbered would make every such reference silently
//! point at a different terminal the first time a tab in front of it was closed, which is
//! the kind of bug nobody reports because it never looks like one.
//!
//! # Where the bookkeeping lives
//!
//! `Tabs` holds the numbers, the counter, and the active mark, and it is deliberately
//! separate from the sessions themselves. Every rule above is a statement about `u32`s,
//! so it is tested as one, without spawning a process or needing a console to be
//! attached. [`Sessions`] is that bookkeeping plus the map of running terminals.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use zet_pty::discovery::Profile;

use crate::session::{Session, SessionError, Waker};

/// Every open session, under the number that names it.
///
/// Dropping this closes every session, which kills every child. There is no other way to
/// release the collection, and that is the point: a terminal whose tabs outlive it is a
/// process leak with a window in front of it.
#[derive(Default)]
pub struct Sessions {
    tabs: Tabs,
    sessions: HashMap<u32, Session>,
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
    /// The same ones [`Session::spawn`] reports. A profile that cannot be launched does
    /// not consume a number: the numbers are creation order, and a failure is not a
    /// creation.
    pub fn open(
        &mut self,
        profile: &Profile,
        cols: u16,
        rows: u16,
        cwd: Option<PathBuf>,
        waker: Arc<dyn Waker>,
    ) -> Result<u32, SessionError> {
        let session = Session::spawn(profile, cols, rows, cwd, waker)?;
        let number = self.tabs.open();
        self.sessions.insert(number, session);
        Ok(number)
    }

    /// Close a session and kill its child.
    ///
    /// # Errors
    ///
    /// Fails if there is no session with that number, or if the child could not be
    /// killed. The tab is gone either way; the error is about what happened to the
    /// process, not about the bookkeeping.
    pub fn close(&mut self, number: u32) -> Result<(), SessionError> {
        let Some(session) = self.sessions.remove(&number) else {
            return Err(SessionError::NoSession(number));
        };
        self.tabs.close(number);
        session.close()
    }

    /// The session with this number, if it is still open.
    #[must_use]
    pub fn get(&self, number: u32) -> Option<&Session> {
        self.sessions.get(&number)
    }

    /// The session with this number, mutably, if it is still open.
    #[must_use]
    pub fn get_mut(&mut self, number: u32) -> Option<&mut Session> {
        self.sessions.get_mut(&number)
    }

    /// Every open tab number, in creation order.
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

    /// The tab after `number` in creation order, wrapping around.
    ///
    /// `None` when there is only one session: wrapping would name the one already on
    /// screen, and a host that switched to it would be doing nothing at the cost of a
    /// frame.
    #[must_use]
    pub fn next_number(&self, number: u32) -> Option<u32> {
        self.tabs.next_number(number)
    }

    /// The tab before `number` in creation order, wrapping around.
    ///
    /// `None` under the same rule as [`Sessions::next_number`].
    #[must_use]
    pub fn previous_number(&self, number: u32) -> Option<u32> {
        self.tabs.previous_number(number)
    }

    /// Remove every session whose child has exited, and report the numbers that went
    /// away.
    ///
    /// Removing the session is what releases it: the pump stops, the pseudoconsole is
    /// closed, and the child — already gone — is reaped. Nothing here decides that an
    /// exited tab should disappear from under the user; that is a host's call, and this
    /// is the mechanism it calls once it has made it.
    pub fn reap(&mut self) -> Vec<u32> {
        let exited = self
            .tabs
            .reap(|number| self.sessions.get(&number).is_some_and(Session::is_exited));
        for number in &exited {
            self.sessions.remove(number);
        }
        exited
    }

    /// Every session, oldest tab first.
    pub fn iter(&self) -> impl Iterator<Item = &Session> + '_ {
        let sessions = &self.sessions;
        self.tabs
            .numbers()
            .into_iter()
            .filter_map(move |number| sessions.get(&number))
    }
}

/// Which tabs exist, in creation order, and which one is active.
///
/// None of this needs a running terminal to be right, which is why it is not tangled up
/// with one: the rules about numbers, order, and the active mark are the part of a tab
/// strip that is easy to get subtly wrong and cheap to test exhaustively.
#[derive(Debug, Default)]
struct Tabs {
    /// The open numbers, oldest first.
    numbers: Vec<u32>,
    /// The last number handed out.
    ///
    /// Numbers run from one and are never reused, so a counter that only ever goes up is
    /// the entire mechanism.
    last: u32,
    /// The tab the host should be showing.
    active: Option<u32>,
}

impl Tabs {
    /// Hand out the next number and make it the active tab.
    fn open(&mut self) -> u32 {
        self.last += 1;
        self.numbers.push(self.last);
        self.active = Some(self.last);
        self.last
    }

    /// Forget a number, reporting whether it was there.
    fn close(&mut self, number: u32) -> bool {
        let Some(index) = self.position(number) else {
            return false;
        };
        self.numbers.remove(index);
        if self.active == Some(number) {
            // The tab that was to the right slides into the gap, and the last tab closes
            // to the one on its left. Closing a tab showing a different one would be a
            // tab switch the user never asked for.
            self.active = self
                .numbers
                .get(index)
                .or_else(|| self.numbers.last())
                .copied();
        }
        true
    }

    /// Every open number, in creation order.
    fn numbers(&self) -> Vec<u32> {
        self.numbers.clone()
    }

    /// How many are open.
    fn len(&self) -> usize {
        self.numbers.len()
    }

    /// Whether none are open.
    fn is_empty(&self) -> bool {
        self.numbers.is_empty()
    }

    /// Where a number sits in creation order.
    fn position(&self, number: u32) -> Option<usize> {
        self.numbers.iter().position(|open| *open == number)
    }

    /// The active tab, if there is one.
    fn active(&self) -> Option<u32> {
        self.active
    }

    /// Activate a number, refusing one that is not open.
    fn set_active(&mut self, number: u32) {
        if self.position(number).is_some() {
            self.active = Some(number);
        }
    }

    /// The next number in creation order, wrapping, or `None` when there is only one tab.
    fn next_number(&self, number: u32) -> Option<u32> {
        let len = self.numbers.len();
        if len < 2 {
            return None;
        }
        let index = self.position(number)?;
        Some(self.numbers[(index + 1) % len])
    }

    /// The previous number in creation order, wrapping, or `None` when there is only one.
    fn previous_number(&self, number: u32) -> Option<u32> {
        let len = self.numbers.len();
        if len < 2 {
            return None;
        }
        let index = self.position(number)?;
        Some(self.numbers[(index + len - 1) % len])
    }

    /// Close every number `exited` reports true for, returning them in creation order.
    ///
    /// The predicate rather than a list of sessions is what keeps the rule testable: what
    /// is being decided here is *which numbers* go, and whether a given tab has exited is
    /// the caller's question.
    fn reap(&mut self, exited: impl Fn(u32) -> bool) -> Vec<u32> {
        let doomed: Vec<u32> = self
            .numbers
            .clone()
            .into_iter()
            .filter(|number| exited(*number))
            .collect();
        for number in &doomed {
            self.close(*number);
        }
        doomed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three tabs, numbered 1, 2, 3.
    fn three() -> Tabs {
        let mut tabs = Tabs::default();
        for _ in 0..3 {
            tabs.open();
        }
        tabs
    }

    #[test]
    fn numbers_start_at_one_and_climb_in_creation_order() {
        let mut tabs = Tabs::default();
        assert_eq!(tabs.open(), 1);
        assert_eq!(tabs.open(), 2);
        assert_eq!(tabs.open(), 3);
        assert_eq!(tabs.numbers(), vec![1, 2, 3]);
        assert_eq!(tabs.len(), 3);
        assert!(!tabs.is_empty());
    }

    #[test]
    fn nothing_is_open_to_begin_with() {
        let tabs = Tabs::default();
        assert!(tabs.is_empty());
        assert_eq!(tabs.len(), 0);
        assert_eq!(tabs.active(), None);
        assert_eq!(tabs.next_number(1), None);
        assert_eq!(tabs.previous_number(1), None);
    }

    #[test]
    fn closing_the_middle_of_three_leaves_the_other_two_numbered_as_they_were() {
        let mut tabs = three();
        assert!(tabs.close(2));
        assert_eq!(tabs.numbers(), vec![1, 3]);
        assert_eq!(tabs.len(), 2);
    }

    #[test]
    fn a_number_is_not_reused_after_a_close() {
        // The whole point of the rule: #2 is gone for good, and the next tab is #4, not
        // the #2 that just became free.
        let mut tabs = three();
        tabs.close(2);
        assert_eq!(tabs.open(), 4);
        assert_eq!(tabs.numbers(), vec![1, 3, 4]);
    }

    #[test]
    fn closing_everything_and_opening_again_still_does_not_reuse_a_number() {
        let mut tabs = three();
        for number in [1, 2, 3] {
            tabs.close(number);
        }
        assert!(tabs.is_empty());
        assert_eq!(tabs.open(), 4);
        assert_eq!(tabs.numbers(), vec![4]);
    }

    #[test]
    fn closing_a_number_that_is_not_open_changes_nothing() {
        let mut tabs = three();
        assert!(!tabs.close(9));
        assert!(!tabs.close(0));
        assert_eq!(tabs.numbers(), vec![1, 2, 3]);
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
    fn next_and_previous_skip_the_gap_a_close_left() {
        let mut tabs = three();
        tabs.close(2);
        assert_eq!(
            tabs.next_number(1),
            Some(3),
            "the order is creation order, not the order the numbers run in"
        );
        assert_eq!(tabs.previous_number(3), Some(1));
    }

    #[test]
    fn there_is_no_next_tab_when_only_one_is_open() {
        let mut tabs = Tabs::default();
        tabs.open();
        assert_eq!(tabs.next_number(1), None);
        assert_eq!(tabs.previous_number(1), None);
    }

    #[test]
    fn next_and_previous_from_a_number_that_is_not_open_are_none() {
        let tabs = three();
        assert_eq!(tabs.next_number(99), None);
        assert_eq!(tabs.previous_number(99), None);
    }

    #[test]
    fn the_newest_tab_is_the_active_one() {
        let mut tabs = Tabs::default();
        tabs.open();
        assert_eq!(tabs.active(), Some(1));
        tabs.open();
        assert_eq!(tabs.active(), Some(2), "opening a tab shows it");
    }

    #[test]
    fn the_active_tab_can_be_changed_and_a_closed_number_is_refused() {
        let mut tabs = three();
        tabs.set_active(1);
        assert_eq!(tabs.active(), Some(1));
        tabs.close(1);
        tabs.set_active(1);
        assert_eq!(
            tabs.active(),
            Some(2),
            "activating a closed number must leave the host with a tab that exists"
        );
        tabs.set_active(99);
        assert_eq!(tabs.active(), Some(2));
    }

    #[test]
    fn closing_the_active_tab_activates_the_one_that_took_its_place() {
        let mut tabs = three();
        tabs.set_active(2);
        tabs.close(2);
        assert_eq!(tabs.active(), Some(3));

        let mut tabs = three();
        tabs.close(3);
        assert_eq!(
            tabs.active(),
            Some(2),
            "the last tab closes to the one on its left"
        );
    }

    #[test]
    fn closing_a_tab_that_is_not_active_leaves_the_active_one_alone() {
        let mut tabs = three();
        tabs.set_active(1);
        tabs.close(3);
        assert_eq!(tabs.active(), Some(1));
    }

    #[test]
    fn closing_the_last_tab_leaves_nothing_active() {
        let mut tabs = Tabs::default();
        tabs.open();
        tabs.close(1);
        assert_eq!(tabs.active(), None);
    }

    #[test]
    fn reaping_removes_exactly_the_numbers_the_predicate_names() {
        let mut tabs = Tabs::default();
        for _ in 0..5 {
            tabs.open();
        }
        let gone = tabs.reap(|number| number == 2 || number == 4);
        assert_eq!(gone, vec![2, 4]);
        assert_eq!(tabs.numbers(), vec![1, 3, 5]);
    }

    #[test]
    fn reaping_nothing_removes_nothing() {
        let mut tabs = three();
        assert!(tabs.reap(|_| false).is_empty());
        assert_eq!(tabs.numbers(), vec![1, 2, 3]);
    }

    #[test]
    fn reaping_the_active_tab_moves_the_active_mark() {
        let mut tabs = three();
        tabs.set_active(2);
        assert_eq!(tabs.reap(|number| number == 2), vec![2]);
        assert_eq!(tabs.active(), Some(3));
    }

    #[test]
    fn reaping_everything_leaves_an_empty_collection() {
        let mut tabs = three();
        assert_eq!(tabs.reap(|_| true), vec![1, 2, 3]);
        assert!(tabs.is_empty());
        assert_eq!(tabs.active(), None);
    }
}
