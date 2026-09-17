//! A single line of cells.

use crate::cell::{Cell, CellFlags};
use crate::color::Color;

/// One row of the screen or of the scrollback.
///
/// A row's storage may be shorter than the terminal is wide. On-screen rows grow as
/// they are written; rows that have been pushed into the scrollback are trimmed of
/// their trailing blanks first, because a scrollback that keeps full-width storage
/// for ten thousand mostly-empty lines costs tens of megabytes for nothing.
///
/// Reading past the end of a row yields a blank cell rather than panicking, so the
/// renderer and the reflow code never have to check the length.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Row {
    cells: Vec<Cell>,
    /// True when the row below this one is a continuation of it.
    ///
    /// This is the only record that a logical line was wrapped by the terminal rather
    /// than by the program. Reflow depends on it entirely: without it, resizing a
    /// window re-wraps each physical row instead of each logical line, which is how
    /// terminals mangle long command lines on resize.
    wrapped: bool,
}

impl Row {
    /// An empty row with room for `cols` cells.
    pub fn new(cols: usize) -> Self {
        Row {
            cells: Vec::with_capacity(cols),
            wrapped: false,
        }
    }

    /// A row filled with blank cells, exactly `cols` wide.
    pub fn blank(cols: usize) -> Self {
        Row {
            cells: vec![Cell::blank(); cols],
            wrapped: false,
        }
    }

    /// A blank row whose cells carry `bg`, for an erase that must leave a colour behind.
    pub fn blank_with(cols: usize, bg: Color) -> Self {
        Row {
            cells: vec![
                Cell {
                    bg,
                    ..Cell::blank()
                };
                cols
            ],
            wrapped: false,
        }
    }

    /// The number of cells this row stores.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Whether this row stores no cells at all.
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Whether the row below continues this one.
    pub fn is_wrapped(&self) -> bool {
        self.wrapped
    }

    /// Mark whether the row below continues this one.
    pub fn set_wrapped(&mut self, wrapped: bool) {
        self.wrapped = wrapped;
    }

    /// The cell at `col`, or a blank if `col` is past the stored end.
    pub fn get(&self, col: usize) -> Cell {
        self.cells.get(col).copied().unwrap_or_default()
    }

    /// A mutable reference to the cell at `col`, growing the row to reach it.
    pub fn get_mut(&mut self, col: usize) -> &mut Cell {
        if col >= self.cells.len() {
            self.cells.resize(col + 1, Cell::blank());
        }
        &mut self.cells[col]
    }

    /// Every stored cell, in column order.
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// Every cell, mutably, widened to at least `cols`.
    ///
    /// Erases and clears run over the whole width of the terminal even on a row that
    /// was trimmed for the scrollback, so this grows the row rather than handing back a
    /// short slice. Handing back the short slice is how a cleared row keeps a stripe of
    /// old text past the point where the program thought it had erased.
    pub fn cells_mut(&mut self, cols: usize) -> &mut [Cell] {
        if self.cells.len() < cols {
            self.cells.resize(cols, Cell::blank());
        }
        &mut self.cells
    }

    /// Force the row to exactly `cols` cells, filling with blanks or dropping the tail.
    pub fn resize(&mut self, cols: usize) {
        self.cells.resize(cols, Cell::blank());
    }

    /// Drop stored cells past `len`, keeping the allocation.
    pub fn truncate(&mut self, len: usize) {
        self.cells.truncate(len);
    }

    /// Reset every stored cell to a blank carrying `bg`.
    ///
    /// The row is widened to `cols` first so that an erase always covers the full
    /// width of the terminal, even on a row that was trimmed for the scrollback.
    pub fn reset(&mut self, cols: usize, bg: Color) {
        self.cells.clear();
        self.cells.resize(
            cols,
            Cell {
                bg,
                ..Cell::blank()
            },
        );
        self.wrapped = false;
    }

    /// Reset the cells in `range` to a blank carrying `bg`, clamping to `cols`.
    pub fn reset_range(&mut self, cols: usize, range: core::ops::Range<usize>, bg: Color) {
        let start = range.start.min(cols);
        let end = range.end.min(cols);
        if start >= end {
            return;
        }
        if self.cells.len() < end {
            self.cells.resize(end, Cell::blank());
        }
        let blank = Cell {
            bg,
            ..Cell::blank()
        };
        for cell in &mut self.cells[start..end] {
            *cell = blank;
        }
    }

    /// Write the character in `template` at `col`, returning how many columns it used.
    ///
    /// The styling travels as a whole [`Cell`] rather than as four loose arguments,
    /// which keeps this down to one argument per independent thing it needs: where, what
    /// shape, and how wide.
    ///
    /// Writing over the trailing half of a wide character also clears its leading half,
    /// and writing over the leading half clears the trailing spacer. Skipping that is
    /// what leaves half-glyphs and stray spaces behind the cursor in terminals that
    /// treat a cell as independent of its neighbour.
    pub fn write_at(&mut self, cols: usize, col: usize, template: &Cell, width: usize) -> usize {
        if col >= cols {
            return 0;
        }

        // Clear whichever half of a wide character this write lands on, and its partner.
        self.clear_wide_partner(cols, col);
        if width == 2 {
            if col + 1 < cols {
                self.clear_wide_partner(cols, col + 1);
            } else {
                // A wide character does not fit in the last column. Terminals drop it
                // rather than splitting it, and the cursor does not advance.
                return 0;
            }
        }

        {
            let cell = self.get_mut(col);
            cell.set_char(
                template.ch,
                template.fg,
                template.bg,
                template.attrs,
                template.link,
            );
            if width == 2 {
                cell.flags.insert(CellFlags::WIDE_CHAR);
            }
        }

        if width == 2 {
            let spacer = self.get_mut(col + 1);
            *spacer = Cell {
                ch: ' ',
                fg: template.fg,
                bg: template.bg,
                attrs: template.attrs,
                link: template.link,
                flags: CellFlags::WIDE_CHAR_SPACER,
            };
        }

        width
    }

    /// If `col` holds either half of a wide character, blank both halves.
    fn clear_wide_partner(&mut self, cols: usize, col: usize) {
        let cell = self.get(col);
        if cell.flags.contains(CellFlags::WIDE_CHAR_SPACER) {
            if col > 0 {
                let lead = self.get_mut(col - 1);
                *lead = Cell {
                    bg: lead.bg,
                    ..Cell::blank()
                };
            }
        } else if cell.flags.contains(CellFlags::WIDE_CHAR) && col + 1 < cols {
            let trail = self.get_mut(col + 1);
            *trail = Cell {
                bg: trail.bg,
                ..Cell::blank()
            };
        }
    }

    /// Shift cells in `[start, end)` right by `count`, filling the gap with `blank`.
    pub fn insert_cells(&mut self, cols: usize, start: usize, count: usize, blank: Cell) {
        if start >= cols || count == 0 {
            return;
        }
        self.resize(cols);
        let count = count.min(cols - start);
        self.cells[start..].rotate_right(count);
        for cell in &mut self.cells[start..start + count] {
            *cell = blank;
        }
    }

    /// Shift cells in `[start, end)` left by `count`, filling the tail with `blank`.
    pub fn delete_cells(&mut self, cols: usize, start: usize, count: usize, blank: Cell) {
        if start >= cols || count == 0 {
            return;
        }
        self.resize(cols);
        let count = count.min(cols - start);
        self.cells[start..].rotate_left(count);
        let tail = cols - count;
        for cell in &mut self.cells[tail..] {
            *cell = blank;
        }
    }

    /// The index one past the last cell that would draw something.
    ///
    /// A cell draws something if it is not a blank, or if it is a blank carrying a
    /// background, a link, or a wide-character spacer. Those all have to survive
    /// trimming or the scrollback loses coloured backgrounds and half of every wide
    /// character.
    pub fn content_len(&self) -> usize {
        self.cells
            .iter()
            .rposition(|c| !c.is_blank() || c.flags.contains(CellFlags::WIDE_CHAR_SPACER))
            .map_or(0, |i| i + 1)
    }

    /// Drop trailing cells that would draw nothing. Used before a row enters the scrollback.
    pub fn trim(&mut self) {
        let len = self.content_len();
        self.cells.truncate(len);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::NamedColor;

    fn write(row: &mut Row, cols: usize, col: usize, ch: char, width: usize) -> usize {
        row.write_at(
            cols,
            col,
            &Cell {
                ch,
                ..Cell::blank()
            },
            width,
        )
    }

    #[test]
    fn reading_past_the_end_is_blank_not_a_panic() {
        let row = Row::new(4);
        assert_eq!(row.get(99), Cell::blank());
        assert_eq!(row.len(), 0);
    }

    #[test]
    fn writing_grows_the_row_to_reach_the_column() {
        let mut row = Row::new(10);
        write(&mut row, 10, 5, 'x', 1);
        assert_eq!(row.len(), 6);
        assert_eq!(row.get(5).ch, 'x');
        assert_eq!(row.get(4).ch, ' ');
    }

    #[test]
    fn a_wide_character_occupies_two_cells() {
        let mut row = Row::new(10);
        let used = write(&mut row, 10, 0, '\u{4e2d}', 2);
        assert_eq!(used, 2);
        assert!(row.get(0).flags.contains(CellFlags::WIDE_CHAR));
        assert!(row.get(1).is_wide_spacer());
    }

    #[test]
    fn a_wide_character_is_dropped_at_the_last_column_not_split() {
        let mut row = Row::new(4);
        let used = write(&mut row, 4, 3, '\u{4e2d}', 2);
        assert_eq!(
            used, 0,
            "a wide character must not be split across the edge"
        );
        assert_eq!(row.get(3).ch, ' ');
    }

    #[test]
    fn overwriting_a_wide_spacer_clears_the_character_to_its_left() {
        let mut row = Row::new(10);
        write(&mut row, 10, 0, '\u{4e2d}', 2);
        // Overwrite the spacer half with a narrow character.
        write(&mut row, 10, 1, 'a', 1);
        assert_eq!(row.get(1).ch, 'a');
        assert_eq!(row.get(0).ch, ' ', "the orphaned lead half must be cleared");
        assert!(!row.get(0).flags.contains(CellFlags::WIDE_CHAR));
    }

    #[test]
    fn overwriting_a_wide_lead_clears_the_spacer_to_its_right() {
        let mut row = Row::new(10);
        write(&mut row, 10, 0, '\u{4e2d}', 2);
        write(&mut row, 10, 0, 'a', 1);
        assert_eq!(row.get(0).ch, 'a');
        assert!(
            !row.get(1).is_wide_spacer(),
            "the orphaned spacer must be cleared"
        );
        assert_eq!(row.get(1).ch, ' ');
    }

    #[test]
    fn writing_a_wide_character_over_half_of_another_cleans_both_sides() {
        let mut row = Row::new(10);
        write(&mut row, 10, 0, '\u{4e2d}', 2);
        write(&mut row, 10, 2, '\u{4e2d}', 2);
        // Now overwrite columns 1..3 with a wide character, straddling both.
        write(&mut row, 10, 1, '\u{4e2d}', 2);
        assert_eq!(row.get(0).ch, ' ');
        assert!(row.get(1).flags.contains(CellFlags::WIDE_CHAR));
        assert!(row.get(2).is_wide_spacer());
        assert_eq!(row.get(3).ch, ' ');
    }

    #[test]
    fn trailing_blanks_trim_but_a_coloured_background_does_not() {
        let mut row = Row::new(8);
        write(&mut row, 8, 0, 'a', 1);
        assert_eq!(row.content_len(), 1);
        row.trim();
        assert_eq!(row.len(), 1);

        let mut row = Row::new(8);
        write(&mut row, 8, 0, 'a', 1);
        row.reset_range(8, 1..5, Color::indexed(NamedColor::Blue.index()));
        assert_eq!(
            row.content_len(),
            5,
            "a coloured background must survive trimming"
        );
    }

    #[test]
    fn a_trailing_wide_spacer_survives_trimming() {
        // The spacer is a blank cell, so trimming by blankness alone would drop it and
        // the wide character would render at half width.
        let mut row = Row::new(8);
        write(&mut row, 8, 0, '\u{4e2d}', 2);
        assert_eq!(row.content_len(), 2);
        row.trim();
        assert_eq!(row.len(), 2);
        assert!(row.get(1).is_wide_spacer());
    }

    #[test]
    fn insert_shifts_right_and_fills_the_gap() {
        let mut row = Row::blank(5);
        for (i, ch) in "abcde".chars().enumerate() {
            write(&mut row, 5, i, ch, 1);
        }
        row.insert_cells(5, 1, 2, Cell::blank());
        let s: String = (0..5).map(|i| row.get(i).ch).collect();
        assert_eq!(s, "a  bc");
    }

    #[test]
    fn delete_shifts_left_and_pads_the_tail() {
        let mut row = Row::blank(5);
        for (i, ch) in "abcde".chars().enumerate() {
            write(&mut row, 5, i, ch, 1);
        }
        row.delete_cells(5, 1, 2, Cell::blank());
        let s: String = (0..5).map(|i| row.get(i).ch).collect();
        assert_eq!(s, "ade  ");
    }

    #[test]
    fn insert_wider_than_the_row_clears_it_without_overflowing() {
        let mut row = Row::blank(4);
        for (i, ch) in "abcd".chars().enumerate() {
            write(&mut row, 4, i, ch, 1);
        }
        row.insert_cells(4, 0, 99, Cell::blank());
        assert_eq!(row.len(), 4);
        let s: String = (0..4).map(|i| row.get(i).ch).collect();
        assert_eq!(
            s, "    ",
            "inserting past the width pushes every cell off the row"
        );
    }

    #[test]
    fn insert_at_the_last_column_only_affects_that_column() {
        let mut row = Row::blank(4);
        for (i, ch) in "abcd".chars().enumerate() {
            write(&mut row, 4, i, ch, 1);
        }
        row.insert_cells(4, 3, 99, Cell::blank());
        let s: String = (0..4).map(|i| row.get(i).ch).collect();
        assert_eq!(s, "abc ");
    }

    #[test]
    fn insert_past_the_right_edge_drops_the_overflow() {
        let mut row = Row::blank(4);
        for (i, ch) in "abcd".chars().enumerate() {
            write(&mut row, 4, i, ch, 1);
        }
        row.insert_cells(4, 1, 2, Cell::blank());
        let s: String = (0..4).map(|i| row.get(i).ch).collect();
        assert_eq!(s, "a  b", "d and c must fall off the right edge");
    }

    #[test]
    fn reset_range_clamps_past_the_width() {
        let mut row = Row::blank(4);
        row.reset_range(4, 2..100, Color::indexed(1));
        assert_eq!(row.len(), 4);
        assert_eq!(row.get(3).bg, Color::indexed(1));
    }

    #[test]
    fn reset_widens_a_trimmed_row_back_to_the_full_width() {
        let mut row = Row::new(20);
        write(&mut row, 20, 0, 'a', 1);
        row.trim();
        assert_eq!(row.len(), 1);
        row.reset(20, Color::DEFAULT);
        assert_eq!(row.len(), 20);
        assert_eq!(row.get(19).ch, ' ');
        assert!(!row.is_wrapped());
    }
}
