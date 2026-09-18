//! The profile picker: which shell a new tab opens, when the file says to ask.
//!
//! `tabs.open-default-without-asking = true` is the default and means `Ctrl+Shift+T`
//! opens the machine's best shell with nothing in the way. Setting it to `false` is a
//! user saying they have more than one shell and they want to choose, and this is the
//! state behind that question: whether it is being asked, and which row the answer is
//! on.
//!
//! The list itself is [`crate::App`]'s — it is the profiles the machine was discovered
//! to have, and the app already holds them for the tab it opens when nothing is asked.
//! What lives here is the part that is not a list: a highlight that moves and wraps, and
//! an open flag that a key press and a paint both read.

/// Which row of the profile list is lit, while the question is being asked.
///
/// Copies, because a picker is three words of state and a borrow of it would outlive the
/// borrow of the app that a paint already holds.
#[derive(Default, Clone, Copy, Debug)]
pub struct Picker {
    open: bool,
    at: usize,
}

impl Picker {
    /// Whether the picker is on screen.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.open
    }

    /// Start asking.
    ///
    /// The highlight goes back to the top every time, which is the row that would have
    /// opened with no picker at all. A user who opens the question and immediately presses
    /// `Enter` gets what they would have got before, and the second shell is one `Down`
    /// away rather than the first thing under the cursor.
    pub fn open(&mut self) {
        self.open = true;
        self.at = 0;
    }

    /// Stop asking, whatever the answer was.
    pub fn close(&mut self) {
        self.open = false;
        self.at = 0;
    }

    /// Which row is lit.
    #[must_use]
    pub const fn at(&self) -> usize {
        self.at
    }

    /// Move the highlight `delta` rows, wrapping at both ends.
    ///
    /// Wrapping rather than stopping, which is what the find bar does and what a choice
    /// does: the list is short, the user is looking at it, and a key that does nothing at
    /// the end of a visible list reads as a key that is broken. `count` is taken as an
    /// argument rather than stored because the list belongs to the app; a list of no rows
    /// leaves the highlight where it is rather than dividing by it.
    pub fn move_by(&mut self, delta: i32, count: usize) {
        if count == 0 {
            return;
        }
        let count = i32::try_from(count).unwrap_or(i32::MAX);
        let at = i32::try_from(self.at).unwrap_or(0);
        // `rem_euclid` and not `%`: up from the first row has to land on the last one,
        // and `%` in Rust keeps the sign of the left operand, which would leave it at
        // zero and make the key look dead.
        self.at = usize::try_from((at + delta).rem_euclid(count)).unwrap_or(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picker_nobody_asked_for_is_not_on_screen() {
        let picker = Picker::default();
        assert!(!picker.is_open());
        assert_eq!(picker.at(), 0);
    }

    #[test]
    fn opening_lights_the_row_that_would_have_opened_anyway() {
        // The point of the first row being lit: a user who set the key to ask, opened the
        // question, and pressed `Enter` without reading it gets the tab they would have
        // got with the key not set at all.
        let mut picker = Picker::default();
        picker.move_by(1, 4);
        picker.open();
        assert!(picker.is_open());
        assert_eq!(picker.at(), 0, "the highlight starts at the top");
    }

    #[test]
    fn down_walks_the_list_and_wraps_past_the_end() {
        let mut picker = Picker::default();
        picker.open();
        for row in [1, 2, 0] {
            picker.move_by(1, 3);
            assert_eq!(picker.at(), row);
        }
    }

    #[test]
    fn up_from_the_first_row_wraps_to_the_last() {
        // The case `%` gets wrong: with a plain remainder this stays at zero, and the
        // only way up from the top of the list stops working.
        let mut picker = Picker::default();
        picker.open();
        picker.move_by(-1, 3);
        assert_eq!(picker.at(), 2);
        picker.move_by(-1, 3);
        assert_eq!(picker.at(), 1);
    }

    #[test]
    fn choosing_and_changing_your_mind_leave_the_highlight_at_the_top() {
        // Both ways out of the picker put the state back, so the next `Ctrl+Shift+T`
        // starts from the same place rather than wherever the last answer was left.
        for leaving in [Picker::close, Picker::open] {
            let mut picker = Picker::default();
            picker.open();
            picker.move_by(2, 5);
            leaving(&mut picker);
            assert_eq!(picker.at(), 0);
        }
    }

    #[test]
    fn a_machine_with_one_shell_does_not_move_off_it() {
        let mut picker = Picker::default();
        picker.open();
        picker.move_by(1, 1);
        assert_eq!(picker.at(), 0);
        picker.move_by(-1, 1);
        assert_eq!(picker.at(), 0);
    }

    #[test]
    fn a_list_with_no_rows_has_nowhere_to_go() {
        // Not reachable from the window, because a picker only opens when a profile was
        // found. It is here because the arithmetic divides by the count, and a division
        // by zero is a panic rather than a wrong answer.
        let mut picker = Picker::default();
        picker.open();
        picker.move_by(1, 0);
        picker.move_by(-1, 0);
        assert_eq!(picker.at(), 0);
    }
}
