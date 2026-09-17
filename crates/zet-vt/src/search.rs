//! Finding text in a grid.
//!
//! A search is a pure function of a [`Grid`] and a needle, which is the whole reason
//! it lives here rather than in the find bar: what the bar draws, where the match
//! count comes from, and where the viewport scrolls to are three questions the window
//! asks, and none of them is a question this module can answer.
//!
//! # A match is a logical line, not a row
//!
//! A terminal wraps a long line by marking the row it ran off and continuing on the
//! next one. Searching row by row would therefore fail to find a word that straddles
//! the fold — which is exactly the word a user is looking for, since they can see it
//! on screen while the search says it is not there. So the walk joins rows that a wrap
//! joined, and a match is reported as a pair of [`Pos`]itions that may be on two rows.
//!
//! # Case
//!
//! Insensitive, unless the needle has an uppercase character in it. Typing `usb` is
//! asking a loose question and typing `USB` is saying you meant it, and the count in
//! the find bar makes the difference visible the moment it changes.

use crate::cell::CellFlags;
use crate::grid::{Grid, Pos};
use crate::row::Row;

/// Where a match begins and ends, in history coordinates.
///
/// Both are inclusive, and `start` is always before `end`. The pair is what a
/// caller needs to paint it: `start` is the first cell of the first character and
/// `end` is the *last cell* of the last one, so a match ending in a double-width
/// character covers both of its columns.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Match {
    /// The first cell.
    pub start: Pos,
    /// The last cell.
    pub end: Pos,
}

impl Match {
    /// How many rows the match spans. One, unless a wrap fell inside it.
    #[must_use]
    pub const fn rows(&self) -> usize {
        self.end.row - self.start.row + 1
    }

    /// Whether a cell is inside it.
    ///
    /// The same test [`zet_render::Selection`] makes, for the same reason: this is the
    /// question a renderer asks once per cell of a grid it is already walking, so it
    /// has to be a comparison and not a search.
    ///
    /// [`zet_render::Selection`]: https://github.com/asterxsk/zet
    #[must_use]
    pub const fn contains(&self, row: usize, col: usize) -> bool {
        if row < self.start.row || row > self.end.row {
            return false;
        }
        if row == self.start.row && col < self.start.col {
            return false;
        }
        if row == self.end.row && col > self.end.col {
            return false;
        }
        true
    }
}

/// One character of a logical line, and where in the grid it came from.
///
/// `cells` is how many columns it occupies, and it is read off the grid's own flags
/// rather than recomputed from a width table: the grid already decided how wide the
/// character was when it laid the row out, and a second opinion could only disagree.
#[derive(Clone, Copy)]
struct Glyph {
    row: usize,
    col: usize,
    ch: char,
    cells: usize,
}

/// Find every occurrence of `needle` in the grid's screen and scrollback.
///
/// At most `limit` matches, for the caller that has to paint them: a search for a
/// single letter in ten thousand lines of scrollback is a search for tens of thousands
/// of rectangles, and a find bar that spends a second of frame time highlighting them
/// is worse than one that says `1000+` and stops.
///
/// The matches come back oldest first, so the one at the top of the scrollback is
/// `[0]` and "next match" walks towards the prompt.
#[must_use]
pub fn find(grid: &Grid, needle: &str, limit: usize) -> Vec<Match> {
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() || limit == 0 {
        return Vec::new();
    }
    let sensitive = needle.iter().any(|ch| ch.is_uppercase());

    let mut found = Vec::new();
    let mut line = Vec::new();
    let mut row = 0;

    while row < grid.total_rows() {
        line.clear();
        let mut last = row;
        while let Some(source) = grid.row_from_history(last) {
            if !source.is_wrapped() {
                collect(source, last, true, &mut line);
                break;
            }
            collect(source, last, false, &mut line);
            last += 1;
        }
        scan(&line, &needle, sensitive, limit, &mut found);
        if found.len() >= limit {
            break;
        }
        row = last + 1;
    }

    found
}

/// Append one row's characters to the logical line being built.
///
/// A row that the line continues past is taken whole; the row the line ends on is
/// taken down to its last non-blank cell, because a match that runs into the padding
/// at the end of a line is a match for whitespace the user cannot see and did not ask
/// for. Trailing blanks are the only thing trimmed — an unstyled space in the middle
/// of a line is real text and is kept.
fn collect(row: &Row, index: usize, ends_here: bool, out: &mut Vec<Glyph>) {
    let cells = row.cells();
    let end = if ends_here {
        row.content_len().min(cells.len())
    } else {
        cells.len()
    };
    for (col, cell) in cells[..end].iter().enumerate() {
        if cell.flags.contains(CellFlags::WIDE_CHAR_SPACER) {
            continue;
        }
        out.push(Glyph {
            row: index,
            col,
            ch: cell.ch,
            cells: if cell.flags.contains(CellFlags::WIDE_CHAR) {
                2
            } else {
                1
            },
        });
    }
}

/// Find every occurrence of `needle` within one logical line and append it.
///
/// Overlapping occurrences are not reported twice: a match consumes its own length,
/// so `aa` in `aaa` is one match and not two. The alternative counts things the user
/// would not, and the count is on screen.
fn scan(line: &[Glyph], needle: &[char], sensitive: bool, limit: usize, out: &mut Vec<Match>) {
    if needle.is_empty() || line.len() < needle.len() {
        return;
    }
    let mut at = 0;
    while at + needle.len() <= line.len() {
        if agrees(&line[at..at + needle.len()], needle, sensitive) {
            let first = line[at];
            let last = line[at + needle.len() - 1];
            out.push(Match {
                start: Pos::new(first.row, first.col),
                end: Pos::new(last.row, last.col + last.cells - 1),
            });
            if out.len() >= limit {
                return;
            }
            at += needle.len();
        } else {
            at += 1;
        }
    }
}

/// Whether a run of the line is the needle.
fn agrees(run: &[Glyph], needle: &[char], sensitive: bool) -> bool {
    run.iter().zip(needle).all(|(glyph, want)| {
        glyph.ch == *want
            || (!sensitive && glyph.ch.to_lowercase().eq(want.to_lowercase()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;
    use crate::term::Term;

    /// A terminal with `lines` fed to it through the real parser, one per row.
    ///
    /// Through the parser rather than by writing cells, because which rows a long line
    /// wraps onto, and which flag says so, is the terminal's decision — a test that
    /// made it itself would be testing the test.
    fn grid_of(width: usize, height: usize, lines: &[&str]) -> Term {
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

    fn spans(term: &Term, needle: &str) -> Vec<((usize, usize), (usize, usize))> {
        find(term.grid(), needle, 1000)
            .into_iter()
            .map(|m| ((m.start.row, m.start.col), (m.end.row, m.end.col)))
            .collect()
    }

    #[test]
    fn a_word_is_found_where_it_is() {
        let term = grid_of(20, 4, &["hello world", "nothing here"]);
        assert_eq!(spans(&term, "world"), vec![((0, 6), (0, 10))]);
    }

    #[test]
    fn every_occurrence_is_found_and_they_come_back_in_order() {
        let term = grid_of(20, 4, &["one one", "one"]);
        assert_eq!(
            spans(&term, "one"),
            vec![((0, 0), (0, 2)), ((0, 4), (0, 6)), ((1, 0), (1, 2))]
        );
    }

    #[test]
    fn a_needle_with_a_capital_in_it_is_case_sensitive_and_one_without_is_not() {
        let term = grid_of(20, 4, &["Usb usb USB"]);
        // No capitals at all, so all three.
        assert_eq!(spans(&term, "usb").len(), 3);
        // A leading capital, so only the one that agrees with it about case.
        assert_eq!(spans(&term, "Usb"), vec![((0, 0), (0, 2))]);
        // A capital that is not the first character still counts. Were the rule "the
        // first character decides", `uSb` would read as a lowercase needle and match
        // the `usb` in the middle; it does not.
        assert!(spans(&term, "uSb").is_empty());
    }

    #[test]
    fn a_match_that_straddles_a_wrap_is_found() {
        // Twelve columns, and a word that lands across the fold at column ten.
        let term = grid_of(10, 4, &["abcdefghijklmno"]);
        let found = spans(&term, "jklmn");
        assert_eq!(found, vec![((0, 9), (1, 3))]);
    }

    #[test]
    fn a_match_is_not_found_across_a_line_the_program_broke_itself() {
        // A newline the program sent is not a wrap, and the two rows are two lines.
        let term = grid_of(10, 4, &["abc", "def"]);
        assert!(spans(&term, "abcdef").is_empty());
    }

    #[test]
    fn trailing_padding_is_not_searchable_but_a_space_in_the_middle_is() {
        let term = grid_of(20, 4, &["ab   cd     "]);
        assert_eq!(spans(&term, "ab   cd"), vec![((0, 0), (0, 6))]);
        // The five blanks after `cd` are past the last non-blank cell.
        assert!(spans(&term, "cd     ").is_empty());
    }

    #[test]
    fn overlapping_occurrences_are_counted_once() {
        let term = grid_of(20, 4, &["aaa"]);
        assert_eq!(spans(&term, "aa"), vec![((0, 0), (0, 1))]);
    }

    #[test]
    fn a_wide_character_is_matched_by_its_own_character_and_covers_both_columns() {
        let term = grid_of(20, 4, &["日本語のテキスト"]);
        assert_eq!(spans(&term, "語"), vec![((0, 4), (0, 5))]);
        assert_eq!(spans(&term, "本語"), vec![((0, 2), (0, 5))]);
    }

    #[test]
    fn an_empty_needle_matches_nothing_rather_than_everything() {
        let term = grid_of(20, 4, &["hello"]);
        assert!(find(term.grid(), "", 1000).is_empty());
    }

    #[test]
    fn the_limit_is_the_number_of_matches_and_not_the_number_of_rows_searched() {
        let term = grid_of(20, 10, &["x"; 40]);
        assert_eq!(find(term.grid(), "x", 5).len(), 5);
    }

    #[test]
    fn a_match_answers_whether_a_cell_is_inside_it() {
        let found = Match {
            start: Pos::new(2, 5),
            end: Pos::new(3, 1),
        };
        assert!(!found.contains(1, 5));
        assert!(!found.contains(2, 4));
        assert!(found.contains(2, 5));
        assert!(found.contains(2, 79));
        assert!(found.contains(3, 0));
        assert!(found.contains(3, 1));
        assert!(!found.contains(3, 2));
        assert!(!found.contains(4, 0));
        assert_eq!(found.rows(), 2);
    }

    #[test]
    fn the_scrollback_is_searched_too() {
        // Two rows tall and four lines printed, so the first two are history.
        let term = grid_of(20, 2, &["first", "second", "third", "fourth"]);
        assert_eq!(term.grid().scrollback_len(), 2);
        // The oldest line is history row zero, and the whole history is searched.
        assert_eq!(spans(&term, "first"), vec![((0, 0), (0, 4))]);
        assert_eq!(spans(&term, "fourth"), vec![((3, 0), (3, 5))]);
    }

    #[test]
    fn an_empty_grid_is_searched_without_finding_anything_or_panicking() {
        let mut term = Term::new(4, 2);
        term.grid_mut().row_mut(0).resize(0);
        assert!(find(term.grid(), "a", 1000).is_empty());
    }
}
