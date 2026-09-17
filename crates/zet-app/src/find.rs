//! The find bar's state: what was typed, what it found, and which find it is on.
//!
//! The bar itself is the window's — the 32-pixel row, the field, the count — and this
//! is everything behind it. The split is the same one the settings panel uses: this
//! decides what a query means and which match the arrows are on, the window decides
//! what that looks like, and neither of them knows about the other.
//!
//! The searching is [`zet_vt::search`]'s, which is a pure function of a grid. What is
//! left here is the part that has state: a query being typed one character at a time,
//! and a cursor that walks the matches it turns up.

use zet_vt::Grid;
use zet_vt::search::{self, Match};

/// How many matches a search reports before it stops counting.
///
/// A single letter in a full scrollback matches tens of thousands of times, and past
/// a few hundred the count is a number nobody reads and the highlight is a screen
/// nobody can see through. The bar says "1000+" rather than pretending otherwise.
pub const MATCH_LIMIT: usize = 1000;

/// What the find bar is doing.
#[derive(Default, Clone)]
pub struct Find {
    open: bool,
    query: String,
    matches: Vec<Match>,
    at: Option<usize>,
    /// Where the arrows go looking after the query changes: the row the last active
    /// match was on.
    ///
    /// Without it, every keystroke would start from wherever the view had just been
    /// scrolled to, and typing `beta` one letter at a time would walk off down the
    /// scrollback looking for `b` before it ever got to look for `beta`. With it, each
    /// character narrows the search around the match the one before it found.
    from: Option<usize>,
    capped: bool,
    /// Whether the query changed since the last search, which is the only reason to
    /// run one: the walk allocates a glyph per cell of the history, and doing it on
    /// every frame of a busy terminal is a cost with nothing on the other side of it.
    stale: bool,
}

impl Find {
    /// Whether the bar is showing.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.open
    }

    /// Show the bar. What was searched for last time is still there.
    ///
    /// Opening is also what makes the bar search: closing it threw the matches away,
    /// and the same query over a terminal that has been printing for a week is not the
    /// same list of positions.
    pub fn open(&mut self) {
        self.open = true;
        self.stale = true;
    }

    /// Hide the bar.
    ///
    /// The query is kept so that reopening it is one keystroke rather than a retype,
    /// and the matches are dropped because they are positions in a grid that has been
    /// scrolling the whole time the bar was shut.
    pub fn close(&mut self) {
        self.open = false;
        self.matches.clear();
        self.at = None;
        self.from = None;
        self.capped = false;
        self.stale = false;
    }

    /// What has been typed.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Every match the last search found, oldest first.
    #[must_use]
    pub fn matches(&self) -> &[Match] {
        &self.matches
    }

    /// How many matches there are, at least.
    #[must_use]
    pub fn count(&self) -> usize {
        self.matches.len()
    }

    /// Whether the search stopped at [`MATCH_LIMIT`] rather than running out.
    #[must_use]
    pub const fn capped(&self) -> bool {
        self.capped
    }

    /// Which match the arrows are on, counted from one for the readout.
    #[must_use]
    pub fn position(&self) -> Option<usize> {
        self.at.map(|at| at + 1)
    }

    /// Which match the arrows are on and how many there are, for the bar to read out.
    ///
    /// `None` when there is nothing to be on, which is a different thing from being on
    /// the zeroth of nothing and is the difference between a bar that says `No results`
    /// and one that says `0 of 0`.
    #[must_use]
    pub fn tally(&self) -> Option<(usize, usize)> {
        self.at.map(|at| (at + 1, self.matches.len()))
    }

    /// The match the arrows are on.
    #[must_use]
    pub fn active(&self) -> Option<Match> {
        self.at.and_then(|at| self.matches.get(at)).copied()
    }

    /// Whether the match at `index` is the one the arrows are on.
    #[must_use]
    pub fn is_active(&self, index: usize) -> bool {
        self.at == Some(index)
    }

    /// Whether the terminal has moved under the query since the last search.
    #[must_use]
    pub const fn is_stale(&self) -> bool {
        self.stale
    }

    /// Note that the terminal's contents have changed.
    pub const fn touch(&mut self) {
        self.stale = true;
    }

    /// Add a character to the query.
    ///
    /// A control character is refused: `Tab` and `Escape` are the bar's own keys, and
    /// a query holding a newline is a query nothing can match.
    pub fn push(&mut self, ch: char) {
        if ch.is_control() {
            return;
        }
        self.query.push(ch);
        self.remember();
    }

    /// Remove the last character. Whether there was one to remove.
    pub fn pop(&mut self) -> bool {
        if self.query.pop().is_none() {
            return false;
        }
        self.remember();
        true
    }

    /// Throw the query away.
    pub fn clear(&mut self) {
        if self.query.is_empty() {
            return;
        }
        self.query.clear();
        self.remember();
    }

    /// Note where the arrows are before the query changes under them.
    fn remember(&mut self) {
        self.from = self.active().map(|found| found.start.row).or(self.from);
        self.stale = true;
        self.at = None;
    }

    /// Run the query again over `grid`, if there is any reason to.
    ///
    /// `from` is the history row at the top of the viewport. It only decides where the
    /// arrows start when the query has just changed: a search that ran again because
    /// the terminal printed something must not move the match the user is looking at,
    /// or holding down `Enter` on a busy terminal would chase its own tail.
    ///
    /// The caller's `from` only answers the first search after the bar opens: after
    /// that the bar remembers where the last match it found was and narrows around it.
    pub fn search(&mut self, grid: &Grid, from: usize) {
        if !self.stale {
            return;
        }
        self.stale = false;
        let found = search::find(grid, &self.query, MATCH_LIMIT);
        self.capped = found.len() >= MATCH_LIMIT;
        self.matches = found;
        if self.matches.is_empty() {
            self.at = None;
        } else if self.at.is_none_or(|at| at >= self.matches.len()) {
            // The first match at or below the place the search is anchored to. Past
            // the last one, the top of the scrollback is the next place to go.
            let start = self.from.unwrap_or(from);
            let first = self
                .matches
                .partition_point(|found| found.start.row < start);
            self.at = Some(if first < self.matches.len() { first } else { 0 });
        }
    }

    /// Move to the next match, wrapping at the end.
    pub fn next_match(&mut self) {
        self.step(true);
    }

    /// Move to the previous match, wrapping at the start.
    pub fn previous_match(&mut self) {
        self.step(false);
    }

    fn step(&mut self, forward: bool) {
        let count = self.matches.len();
        if count == 0 {
            return;
        }
        let at = self.at.unwrap_or(0);
        self.at = Some(if forward {
            (at + 1) % count
        } else {
            (at + count - 1) % count
        });
        self.from = self.active().map(|found| found.start.row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zet_vt::{Parser, Term};

    /// A terminal with `lines` fed to it, one per row, from the top.
    fn term_of(width: usize, height: usize, lines: &[&str]) -> Term {
        let mut term = Term::new(width, height);
        let mut parser = Parser::new();
        for (row, text) in lines.iter().enumerate() {
            if row > 0 {
                parser.advance_slice(b"\r\n", &mut term);
            }
            parser.advance_slice(text.as_bytes(), &mut term);
        }
        term
    }

    /// A bar that has been given `query` and searched a grid from row zero.
    fn searching(term: &Term, query: &str) -> Find {
        let mut find = Find::default();
        find.open();
        for ch in query.chars() {
            find.push(ch);
        }
        find.search(term.grid(), 0);
        find
    }

    #[test]
    fn a_closed_bar_is_doing_nothing_and_says_so() {
        let find = Find::default();
        assert!(!find.is_open());
        assert_eq!(find.count(), 0);
        assert_eq!(find.position(), None);
        assert_eq!(find.active(), None);
    }

    #[test]
    fn typing_builds_a_query_and_the_search_finds_what_it_says() {
        let term = term_of(20, 4, &["alpha beta", "beta gamma"]);
        let find = searching(&term, "beta");
        assert_eq!(find.query(), "beta");
        assert_eq!(find.count(), 2);
    }

    #[test]
    fn backspace_takes_a_character_back_and_says_whether_there_was_one() {
        let mut find = Find::default();
        assert!(!find.pop());
        find.push('a');
        assert!(find.pop());
        assert_eq!(find.query(), "");
        assert!(!find.pop());
    }

    #[test]
    fn a_control_character_is_not_part_of_a_query() {
        // `Tab` and `Escape` are the bar's own keys, and a query holding a newline is
        // a query nothing can match.
        let mut find = Find::default();
        find.push('\t');
        find.push('\n');
        find.push('\u{1b}');
        assert_eq!(find.query(), "");
        find.push('a');
        assert_eq!(find.query(), "a");
    }

    #[test]
    fn a_search_that_has_no_reason_to_run_again_does_not() {
        let mut term = term_of(20, 4, &["alpha"]);
        let mut find = Find::default();
        find.open();
        for ch in "alpha".chars() {
            find.push(ch);
        }
        find.search(term.grid(), 0);
        assert_eq!(find.count(), 1);
        // Print another `alpha` without telling the bar anything, and the search
        // still owes the old answer — which is what "the terminal scrolled but you
        // did not type" looks like from here.
        let mut parser = Parser::new();
        parser.advance_slice(b"\r\nalpha", &mut term);
        find.search(term.grid(), 0);
        assert_eq!(find.count(), 1);
        // Told, it looks again.
        find.touch();
        find.search(term.grid(), 0);
        assert_eq!(find.count(), 2);
    }

    #[test]
    fn the_arrows_walk_the_matches_and_wrap_at_both_ends() {
        let term = term_of(20, 4, &["one one one"]);
        let mut find = searching(&term, "one");
        assert_eq!(find.count(), 3);
        assert_eq!(find.position(), Some(1));
        find.next_match();
        assert_eq!(find.position(), Some(2));
        find.next_match();
        assert_eq!(find.position(), Some(3));
        find.next_match();
        assert_eq!(find.position(), Some(1), "wraps forward");
        find.previous_match();
        assert_eq!(find.position(), Some(3), "wraps backward");
    }

    #[test]
    fn the_arrows_start_at_the_first_match_below_the_top_of_the_view_and_wrap_past_the_end() {
        let term = term_of(20, 10, &["a", "b", "a", "b", "a"]);
        let mut find = Find::default();
        find.open();
        find.push('a');
        // Nothing has been scrolled off, so history row 4 is the third `a`.
        find.search(term.grid(), 4);
        assert_eq!(find.position(), Some(3));
        // Asked for a row past every match, the search starts again at the top
        // rather than sitting on nothing.
        find.at = None;
        find.stale = true;
        find.search(term.grid(), 99);
        assert_eq!(find.position(), Some(1));
    }

    #[test]
    fn a_search_that_runs_again_under_a_moving_terminal_leaves_the_match_alone() {
        let mut term = term_of(20, 10, &["a", "b", "a"]);
        let mut find = searching(&term, "a");
        find.next_match();
        assert_eq!(find.position(), Some(2));
        let mut parser = Parser::new();
        parser.advance_slice(b"\r\na", &mut term);
        find.touch();
        find.search(term.grid(), 0);
        assert_eq!(find.count(), 3, "the new match was found");
        assert_eq!(find.position(), Some(2), "and the cursor did not move");
    }

    #[test]
    fn typing_another_character_narrows_around_where_you_already_are() {
        // `b` is on rows 0 and 2, `be` on rows 1 and 3. Searching `b` lands on row 0.
        // The next keystroke is told the view is at row 2 — which it would be, if the
        // caller passed the viewport top and the view had moved — and the answer is
        // still the `be` on row 1, because the search is anchored where the last match
        // was rather than where the view happens to be.
        let term = term_of(20, 10, &["b", "be", "b", "be"]);
        let mut find = Find::default();
        find.open();
        find.push('b');
        find.search(term.grid(), 0);
        assert_eq!(find.active().map(|found| found.start.row), Some(0));
        find.push('e');
        find.search(term.grid(), 2);
        assert_eq!(find.query(), "be");
        assert_eq!(find.active().map(|found| found.start.row), Some(1));
    }

    #[test]
    fn a_query_that_matches_nothing_has_no_match_to_be_on() {
        let term = term_of(20, 4, &["alpha"]);
        let find = searching(&term, "omega");
        assert_eq!(find.count(), 0);
        assert_eq!(find.position(), None);
        assert_eq!(find.active(), None);
        assert!(!find.capped());
    }

    #[test]
    fn the_limit_is_reported_rather_than_hid() {
        // Two thousand matches, against a limit of one thousand.
        let row = "x".repeat(100);
        let lines: Vec<&str> = (0..20).map(|_| row.as_str()).collect();
        let term = term_of(120, 20, &lines);
        let find = searching(&term, "x");
        assert_eq!(find.count(), MATCH_LIMIT);
        assert!(find.capped());
    }

    #[test]
    fn clearing_the_query_takes_the_matches_with_it() {
        let term = term_of(20, 4, &["alpha"]);
        let mut find = searching(&term, "alpha");
        assert_eq!(find.count(), 1);
        find.clear();
        find.search(term.grid(), 0);
        assert_eq!(find.query(), "");
        assert_eq!(find.count(), 0);
        assert_eq!(find.position(), None);
    }

    #[test]
    fn closing_keeps_the_query_and_throws_the_positions_away() {
        let term = term_of(20, 4, &["alpha"]);
        let mut find = searching(&term, "alpha");
        find.close();
        assert!(!find.is_open());
        assert_eq!(find.query(), "alpha");
        assert_eq!(find.count(), 0, "positions in a grid that has been moving");
        assert_eq!(find.position(), None);
    }

    #[test]
    fn a_closed_bar_reopened_finds_the_same_thing_again() {
        let term = term_of(20, 4, &["alpha beta"]);
        let mut find = searching(&term, "beta");
        find.close();
        find.open();
        find.search(term.grid(), 0);
        assert_eq!(find.count(), 1);
        assert_eq!(find.position(), Some(1));
    }

    #[test]
    fn the_arrows_do_nothing_when_there_is_nothing_to_walk() {
        let mut find = Find::default();
        find.next_match();
        find.previous_match();
        assert_eq!(find.position(), None);
    }
}
