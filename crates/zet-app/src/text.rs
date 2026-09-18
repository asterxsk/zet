//! Reading a selection out of the grid.
//!
//! Copying is the one thing a selection is for besides being painted, and it is the
//! only place a terminal has to decide what a row *means*. A row that soft-wrapped is
//! half of a longer line, and putting a newline where the window happened to end is how
//! a copied command comes out with a break in the middle of it.

use zet_render::Selection;
use zet_vt::{Pos, Term};

/// The text inside a selection, in a view scrolled `scroll_offset` rows back.
///
/// Rows are joined with a newline unless the row soft-wrapped into the next one, and
/// trailing spaces come off every line. Both are what every terminal does and both are
/// about the same thing: what the user selected is the text, not the rectangle.
///
/// A selection is made of window rows — the host turns a pointer position into a cell
/// and neither of them knows where the viewport is — so the offset is what turns them
/// into lines of history, and it is the same offset the renderer was handed. Without it
/// a selection made while scrolled back is drawn on the rows the user picked and reads
/// out the rows the shell happens to be on.
#[must_use]
pub fn selection_text(term: &Term, selection: Selection, scroll_offset: usize) -> String {
    let grid = term.grid();
    let cols = grid.cols();
    let top = grid.history_top(scroll_offset);
    let back = grid.scrollback_len();
    let ((first, _), (last, _)) = selection.bounds();
    let mut out = String::new();

    for row in first..=last.min(grid.rows().saturating_sub(1)) {
        let index = top + row;
        let Some(line) = grid.row_from_history(index) else {
            continue;
        };
        let span = selection.columns_in(row, cols);
        let start = out.len();
        for col in span {
            let cell = line.get(col);
            // The trailing half of a wide character carries a space for the renderer's
            // benefit and is not a character the user can see, let alone select.
            if cell.is_wide_spacer() {
                continue;
            }
            // Clusters are keyed by the screen rather than by the history, because that
            // is where they were built, so a row above the live screen has none and the
            // cell's own character is the whole of what there is to copy.
            match index
                .checked_sub(back)
                .and_then(|screen| term.cluster_at(Pos::new(screen, col)))
            {
                Some(cluster) => out.push_str(cluster),
                None if cell.ch == ' ' => out.push(' '),
                None => out.push(cell.ch),
            }
        }
        // A colour that reaches past the text is not something to copy.
        let trimmed = out[start..].trim_end().len() + start;
        out.truncate(trimmed);

        if row != last && !line.is_wrapped() {
            out.push('\n');
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use zet_vt::Parser;

    fn term(cols: usize, rows: usize, bytes: &[u8]) -> Term {
        let mut term = Term::new(cols, rows);
        let mut parser = Parser::new();
        parser.advance_slice(bytes, &mut term);
        term
    }

    fn select(rows: &Term, from: (usize, usize), to: (usize, usize)) -> String {
        selection_text(
            rows,
            Selection::new(Pos::new(from.0, from.1), Pos::new(to.0, to.1)),
            0,
        )
    }

    /// The same, in a view scrolled `offset` rows back.
    fn select_scrolled(
        rows: &Term,
        offset: usize,
        from: (usize, usize),
        to: (usize, usize),
    ) -> String {
        selection_text(
            rows,
            Selection::new(Pos::new(from.0, from.1), Pos::new(to.0, to.1)),
            offset,
        )
    }

    #[test]
    fn one_word_is_one_word() {
        let term = term(20, 3, b"hello world");
        assert_eq!(select(&term, (0, 0), (0, 4)), "hello");
    }

    #[test]
    fn a_selection_in_a_scrolled_view_reads_the_rows_that_are_drawn_there() {
        // `one` and `two` have scrolled off; the screen holds `three` and `four`.
        let term = term(20, 2, b"one\r\ntwo\r\nthree\r\nfour");
        assert_eq!(select(&term, (0, 0), (0, 4)), "three");
        // The window row the user dragged over is the same either way; what changes is
        // which line of history it is, and copying the screen's rows while the view is
        // scrolled is how a terminal hands back text nobody selected.
        assert_eq!(select_scrolled(&term, 1, (0, 0), (0, 4)), "two");
        assert_eq!(select_scrolled(&term, 2, (0, 0), (0, 4)), "one");
    }

    #[test]
    fn a_selection_across_rows_comes_back_with_newlines() {
        let term = term(20, 3, b"one\r\ntwo\r\nthree");
        assert_eq!(select(&term, (0, 0), (2, 4)), "one\ntwo\nthree");
    }

    #[test]
    fn a_soft_wrapped_line_is_not_broken_where_the_window_ended() {
        // Eight columns, twelve characters: the terminal wraps it, and a paste of the
        // result has to be the twelve characters.
        let mut term = Term::new(8, 4);
        let mut parser = Parser::new();
        parser.advance_slice(b"abcdefghijkl", &mut term);
        assert_eq!(select(&term, (0, 0), (1, 3)), "abcdefghijkl");
    }

    #[test]
    fn trailing_spaces_do_not_come_along() {
        let mut term = term(20, 2, b"code");
        // Paint a background past the text, which is what makes a row hold trailing
        // blanks at all.
        term.grid_mut().row_mut(0).get_mut(10).bg = zet_vt::Color::indexed(4);
        assert_eq!(select(&term, (0, 0), (0, 19)), "code");
    }

    #[test]
    fn half_of_a_wide_character_is_not_a_character() {
        let term = term(10, 2, "\u{4e2d}\u{6587}".as_bytes());
        assert_eq!(select(&term, (0, 0), (0, 3)), "\u{4e2d}\u{6587}");
    }

    #[test]
    fn a_combining_mark_stays_with_its_base() {
        let term = term(10, 2, "e\u{0301}x".as_bytes());
        assert_eq!(select(&term, (0, 0), (0, 1)), "e\u{0301}x");
    }

    #[test]
    fn a_selection_reaches_only_the_columns_it_names() {
        let term = term(20, 2, b"one two");
        assert_eq!(select(&term, (0, 4), (0, 6)), "two");
        assert_eq!(
            select(&term, (0, 4), (0, 99)),
            "two",
            "past the end is fine"
        );
    }

    #[test]
    fn a_selection_of_nothing_is_nothing() {
        let term = Term::new(20, 2);
        assert_eq!(select(&term, (0, 0), (0, 0)), "");
        assert_eq!(select(&term, (5, 0), (5, 9)), "", "past the last row");
    }
}
