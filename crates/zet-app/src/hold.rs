//! Holding the frame while a program repaints.
//!
//! `DECSET 2026` is the mechanism that makes a full-screen program flicker-free: the
//! terminal holds the last complete frame until the program says the new one is finished.
//! A program that clears the screen and redraws it in forty writes is showing the user
//! thirty-nine states nobody asked to see otherwise, and a TUI repainting on a timer shows
//! them sixty times a second.
//!
//! [`zet_vt::Term::is_synchronized`] answers whether the program is mid-repaint. What is
//! left for this module is the part `Term` cannot know: the marker has no end that a
//! program that has crashed or wedged will send, so a terminal that held the frame for as
//! long as it was asked would be showing a picture of a program that is still printing.
//! The hold therefore expires.
//!
//! The clock is passed in rather than read, exactly as the cursor's blink is: a budget
//! that read `Instant::now` could not be asserted from either side of its own boundary.

use std::time::{Duration, Instant};

/// How long one repaint may hold the frame before the terminal stops waiting for it.
///
/// A repaint that is being done properly is a handful of writes into a pipe and takes a
/// fraction of a millisecond; the budget is not about the honest case. It is about the
/// program that set the marker and then stopped — hung, deadlocked, or killed between the
/// two writes — where the alternative is a window frozen on a stale frame for ever with
/// nothing to tell the user why. Four frames of a 60Hz display is short enough that a user
/// never sees the difference and long enough that no real repaint is cut off.
pub const BUDGET: Duration = Duration::from_millis(66);

/// The hold a program has asked for, and when it started.
#[derive(Default, Clone, Copy, Debug)]
pub struct Hold {
    /// When the repaint this hold belongs to began, while one is being held.
    since: Option<Instant>,
}

impl Hold {
    /// Whether to keep holding the frame, given whether the program is synchronized now.
    ///
    /// Called once per pump. The budget is measured from the first call that found the
    /// program synchronized rather than from each call, because a repaint arrives across
    /// several reads and a budget restarted on every one of them would never expire.
    pub fn holds(&mut self, synchronized: bool, now: Instant) -> bool {
        if !synchronized {
            // The program finished, or never started. Either way the next repaint gets a
            // budget of its own rather than inheriting the rest of this one's.
            self.since = None;
            return false;
        }
        let since = *self.since.get_or_insert(now);
        now.duration_since(since) < BUDGET
    }

    /// When the hold in force lapses, if one is in force.
    ///
    /// The deadline belongs to the repaint rather than to the call, so asking part-way
    /// through the budget gives the same instant as asking at the start of it. It is the
    /// start of the repaint plus the budget and not "now plus the budget": a caller that
    /// slept for the budget from wherever it happened to be asking would sleep past the
    /// end every time.
    ///
    /// `None` means no repaint has been seen since the last one finished. A hold whose
    /// budget has already been spent still has a deadline — that it has passed is what
    /// [`Self::holds`] answers, and the two are separate questions.
    #[must_use]
    pub fn deadline(&self) -> Option<Instant> {
        self.since.map(|since| since + BUDGET)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An instant to measure from, so no test depends on the wall clock.
    fn epoch() -> Instant {
        Instant::now()
    }

    #[test]
    fn a_program_that_did_not_ask_for_a_hold_does_not_get_one() {
        let mut hold = Hold::default();
        assert!(!hold.holds(false, epoch()));
    }

    #[test]
    fn a_program_mid_repaint_holds_the_frame() {
        let mut hold = Hold::default();
        let now = epoch();
        assert!(hold.holds(true, now));
        assert!(hold.holds(true, now + BUDGET / 2), "still repainting");
    }

    #[test]
    fn the_hold_lifts_the_moment_the_program_says_it_is_done() {
        let mut hold = Hold::default();
        let now = epoch();
        assert!(hold.holds(true, now));
        assert!(!hold.holds(false, now + Duration::from_millis(1)));
    }

    #[test]
    fn the_budget_is_measured_from_the_start_of_the_repaint_and_not_from_each_call() {
        // The failure this catches is the one that makes the budget useless: a repaint
        // that arrives in ten reads would push the deadline back ten times and hold the
        // frame for as long as the program kept writing, which is the freeze the budget
        // exists to prevent.
        let mut hold = Hold::default();
        let now = epoch();
        // Granted at the start, which is half the claim: a `holds` that answered "no" to
        // everything would satisfy the assertion at the end and every user of this module
        // would be back to a flickering terminal.
        assert!(hold.holds(true, now), "the first frame of the repaint");
        let mut at = now;
        for _ in 0..20 {
            // Every call is well inside the budget of the one before it, and the whole run
            // is well past the budget of the first.
            at += BUDGET / 8;
            let _ = hold.holds(true, at);
        }
        assert!(
            !hold.holds(true, at),
            "the hold outlived the repaint it was granted for"
        );
    }

    #[test]
    fn a_program_that_never_finishes_repainting_loses_the_frame() {
        let mut hold = Hold::default();
        let now = epoch();
        assert!(hold.holds(true, now));
        assert!(!hold.holds(true, now + BUDGET));
        assert!(!hold.holds(true, now + BUDGET * 10));
    }

    #[test]
    fn the_hold_says_when_it_lapses() {
        // The caller needs an instant to come back at, not only a yes. A program that sets
        // the marker and then stops — hung, deadlocked, killed between the two writes —
        // sends no further output, so a terminal that knew only "hold" would have nothing
        // to wake it and would sit on the held frame for ever, which is the freeze the
        // budget exists to prevent, arrived at from the other side.
        //
        // The instant belongs to the hold rather than to the call, so it does not move
        // when the budget is already spent: a deadline that followed the last call would
        // push the wake-up back on every read and never arrive.
        let mut hold = Hold::default();
        let now = epoch();
        assert_eq!(hold.deadline(), None, "nothing is being held");
        assert!(hold.holds(true, now));
        assert_eq!(hold.deadline(), Some(now + BUDGET));

        assert!(hold.holds(true, now + Duration::from_millis(10)));
        assert_eq!(
            hold.deadline(),
            Some(now + BUDGET),
            "the deadline moved with the call"
        );

        assert!(!hold.holds(true, now + BUDGET), "the budget ran out");
        assert_eq!(
            hold.deadline(),
            Some(now + BUDGET),
            "the deadline is the hold's own and does not move when the budget is spent"
        );

        assert!(!hold.holds(false, now + BUDGET));
        assert_eq!(hold.deadline(), None, "the program finished repainting");
    }

    #[test]
    fn a_second_repaint_gets_a_budget_of_its_own() {
        // A program that repaints on a timer holds and releases over and over. If the
        // released hold left its start instant behind, every repaint after the first
        // would be cut off immediately and the terminal would be back to flickering —
        // which is the bug this whole module exists to prevent, reintroduced by a stale
        // field.
        let mut hold = Hold::default();
        let now = epoch();
        assert!(hold.holds(true, now));
        let done = now + Duration::from_millis(5);
        assert!(!hold.holds(false, done));

        let later = now + BUDGET * 100;
        assert!(hold.holds(true, later), "the second repaint was not held");
        assert!(hold.holds(true, later + BUDGET / 2));
        assert!(!hold.holds(true, later + BUDGET));
    }
}
