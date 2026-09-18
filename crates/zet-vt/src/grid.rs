//! The screen, the scrollback, and the operations that move lines around.
//!
//! The grid does not know what a cursor is. It knows how tall it is, which lines have
//! scrolled off the top, and how to move rows when a program asks. The terminal state
//! machine above it owns the cursor and decides which grid operation a control
//! sequence means.

use std::collections::VecDeque;

use crate::attrs::Attrs;
use crate::cell::{Cell, CellFlags};
use crate::color::Color;
use crate::damage::Damage;
use crate::row::Row;

/// A position on the screen, in cells from the top left.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct Pos {
    /// Zero-based row.
    pub row: usize,
    /// Zero-based column.
    pub col: usize,
}

impl Pos {
    /// A position.
    pub const fn new(row: usize, col: usize) -> Self {
        Pos { row, col }
    }
}

/// How much scrollback to keep, in lines.
pub const DEFAULT_SCROLLBACK: usize = 10_000;

/// The screen and the lines that have scrolled off it.
pub struct Grid {
    cols: usize,
    rows: usize,
    /// The visible screen, top to bottom. Always exactly `rows` long.
    screen: Vec<Row>,
    /// Lines that scrolled off the top, oldest first.
    scrollback: VecDeque<Row>,
    scrollback_limit: usize,
    /// The inclusive top and bottom of the scrolling region. Programs set this to keep
    /// a status line pinned at the top or bottom of the screen.
    scroll_region: (usize, usize),
    /// The background a blank cell gets when the grid erases. This is the `BCE` rule:
    /// after `SGR 41`, an erase leaves red behind rather than the theme's background.
    erase_bg: Color,
    /// Which rows changed since the last frame.
    damage: Damage,
    /// Tab stops, one per column.
    tabs: Vec<bool>,
}

impl Grid {
    /// A grid with every cell blank and a tab stop every eight columns.
    pub fn new(cols: usize, rows: usize) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        Grid {
            cols,
            rows,
            screen: (0..rows).map(|_| Row::blank(cols)).collect(),
            scrollback: VecDeque::new(),
            scrollback_limit: DEFAULT_SCROLLBACK,
            scroll_region: (0, rows - 1),
            erase_bg: Color::DEFAULT,
            damage: Damage::new(rows),
            tabs: default_tabs(cols),
        }
    }

    /// How wide the grid is, in cells.
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// How tall the visible screen is, in rows.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// The visible screen, top to bottom.
    pub fn screen(&self) -> &[Row] {
        &self.screen
    }

    /// One row of the visible screen.
    pub fn row(&self, row: usize) -> &Row {
        &self.screen[row]
    }

    /// One row of the visible screen, mutably.
    pub fn row_mut(&mut self, row: usize) -> &mut Row {
        self.damage.mark_row(row);
        &mut self.screen[row]
    }

    /// The damage tracker.
    pub fn damage(&self) -> &Damage {
        &self.damage
    }

    /// The damage tracker, mutably, for the terminal to flag cursor movement.
    pub fn damage_mut(&mut self) -> &mut Damage {
        &mut self.damage
    }

    /// How many lines are in the scrollback.
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// A line from the scrollback, oldest first.
    pub fn scrollback_row(&self, index: usize) -> Option<&Row> {
        self.scrollback.get(index)
    }

    /// How many lines the grid can show, scrollback included.
    pub fn total_rows(&self) -> usize {
        self.scrollback.len() + self.rows
    }

    /// The history index of the top of a view scrolled `offset` rows back.
    ///
    /// An offset is measured from the live screen and a history index from the oldest
    /// line, so this is the one subtraction between the two, and it is one function
    /// because everything that draws or hit-tests a scrolled viewport has to agree on
    /// the answer: the renderer asks it which rows to draw, the find bar asks it where
    /// its matches are, and the selection asks it whose text was copied. Computed
    /// separately at each of those, a view scrolled four rows starts four rows above the
    /// live screen in one of them and somewhere else in another — which draws a
    /// highlight over text nobody selected.
    ///
    /// Saturating, because an offset past the top of the history is a view of the oldest
    /// line rather than a panic.
    #[must_use]
    pub fn history_top(&self, offset: usize) -> usize {
        self.scrollback.len().saturating_sub(offset)
    }

    /// A row anywhere in the history, scrollback first.
    ///
    /// This is what a renderer walking a scrolled-back viewport uses, so it never has
    /// to know where the scrollback ends and the screen begins.
    pub fn row_from_history(&self, index: usize) -> Option<&Row> {
        if index < self.scrollback.len() {
            self.scrollback.get(index)
        } else {
            self.screen.get(index - self.scrollback.len())
        }
    }

    /// The inclusive top and bottom of the scrolling region.
    pub fn scroll_region(&self) -> (usize, usize) {
        self.scroll_region
    }

    /// Set the scrolling region, clamped to the screen.
    pub fn set_scroll_region(&mut self, top: usize, bottom: usize) {
        let bottom = bottom.min(self.rows - 1);
        let top = top.min(bottom);
        self.scroll_region = (top, bottom);
    }

    /// Reset the scrolling region to the whole screen.
    pub fn reset_scroll_region(&mut self) {
        self.scroll_region = (0, self.rows - 1);
    }

    /// How many lines of scrollback are kept.
    pub fn scrollback_limit(&self) -> usize {
        self.scrollback_limit
    }

    /// Set how many lines of scrollback to keep, dropping the oldest if it shrank.
    pub fn set_scrollback_limit(&mut self, limit: usize) {
        self.scrollback_limit = limit;
        while self.scrollback.len() > limit {
            self.scrollback.pop_front();
        }
    }

    /// The background a blank cell gets on an erase.
    pub fn erase_bg(&self) -> Color {
        self.erase_bg
    }

    /// Set the background a blank cell gets on an erase.
    pub fn set_erase_bg(&mut self, bg: Color) {
        self.erase_bg = bg;
    }

    /// A blank cell carrying the current erase background.
    pub fn blank(&self) -> Cell {
        Cell {
            bg: self.erase_bg,
            ..Cell::blank()
        }
    }

    /// The pen colours and attributes a blank cell should carry, for `ECH` and friends.
    pub fn blank_with(&self, fg: Color, attrs: Attrs) -> Cell {
        Cell {
            fg,
            bg: self.erase_bg,
            attrs,
            ..Cell::blank()
        }
    }

    /// Push a line into the scrollback, dropping the oldest if the limit is reached.
    fn push_scrollback(&mut self, mut row: Row) {
        if self.scrollback_limit == 0 {
            return;
        }
        // Trailing blanks are most of a typical terminal line. Trimming before storing
        // is what keeps ten thousand lines of scrollback affordable.
        row.trim();
        self.scrollback.push_back(row);
        while self.scrollback.len() > self.scrollback_limit {
            self.scrollback.pop_front();
        }
    }

    /// Move the contents of the scrolling region up by `n`, filling from the bottom.
    ///
    /// When the region is the whole screen the evicted lines go to the scrollback.
    /// When it is not, they are discarded, because a program that set a scrolling
    /// region is managing that region itself and does not want its status line
    /// preserved as history.
    pub fn scroll_up(&mut self, n: usize) {
        let (top, bottom) = self.scroll_region;
        let height = bottom - top + 1;
        let n = n.min(height);
        if n == 0 {
            return;
        }
        let full_screen = top == 0 && bottom + 1 == self.rows;
        let blank = Row::blank_with(self.cols, self.erase_bg);
        for _ in 0..n {
            let row = self.screen.remove(top);
            if full_screen {
                self.push_scrollback(row);
            }
            self.screen.insert(bottom, blank.clone());
        }
        self.damage.mark_rows(top..bottom + 1);
    }

    /// Move the contents of the scrolling region down by `n`, filling from the top.
    pub fn scroll_down(&mut self, n: usize) {
        let (top, bottom) = self.scroll_region;
        let height = bottom - top + 1;
        let n = n.min(height);
        if n == 0 {
            return;
        }
        let blank = Row::blank_with(self.cols, self.erase_bg);
        for _ in 0..n {
            self.screen.remove(bottom);
            self.screen.insert(top, blank.clone());
        }
        self.damage.mark_rows(top..bottom + 1);
    }

    /// Insert `n` blank lines at `row`, pushing lines below down and out of the region.
    pub fn insert_lines(&mut self, row: usize, n: usize) {
        let (top, bottom) = self.scroll_region;
        if row < top || row > bottom {
            return;
        }
        let n = n.min(bottom - row + 1);
        let blank = Row::blank_with(self.cols, self.erase_bg);
        for _ in 0..n {
            self.screen.remove(bottom);
            self.screen.insert(row, blank.clone());
        }
        self.break_wrap_at(row, bottom);
        self.damage.mark_rows(top..bottom + 1);
    }

    /// Delete `n` lines at `row`, pulling lines below up and filling from the bottom.
    pub fn delete_lines(&mut self, row: usize, n: usize) {
        let (top, bottom) = self.scroll_region;
        if row < top || row > bottom {
            return;
        }
        let n = n.min(bottom - row + 1);
        let blank = Row::blank_with(self.cols, self.erase_bg);
        for _ in 0..n {
            self.screen.remove(row);
            self.screen.insert(bottom, blank.clone());
        }
        self.break_wrap_at(row, bottom);
        self.damage.mark_rows(top..bottom + 1);
    }

    /// Drop the soft-wrap flags an insert or delete has just invalidated.
    ///
    /// Both operations move rows sideways past each other, so whatever the row above
    /// the edit was continuing into is not what now follows it, and the row pushed to
    /// the bottom of the region has had its own continuation pushed out with it. Both
    /// flags would otherwise survive and claim a logical line spanning rows that the
    /// user can see are separate — which a copy then joins and a search matches across.
    fn break_wrap_at(&mut self, row: usize, bottom: usize) {
        if row > 0 {
            self.screen[row - 1].set_wrapped(false);
        }
        self.screen[bottom].set_wrapped(false);
    }

    /// Erase the whole screen.
    ///
    /// `history` decides whether this is `ED 3`, which also throws away the scrollback.
    /// Programs that clear the screen before drawing an interface mean `ED 2` and would
    /// be furious to lose the user's history, so the distinction matters.
    pub fn clear_screen(&mut self, history: bool) {
        let blank = self.blank();
        let cols = self.cols;
        for row in &mut self.screen {
            for cell in row.cells_mut(cols) {
                *cell = blank;
            }
            row.set_wrapped(false);
        }
        if history {
            self.scrollback.clear();
        }
        self.damage.mark_all();
    }

    /// Erase from the cursor to the end of the screen, or the start, or everything.
    pub fn erase_in_display(&mut self, mode: u16, cursor: Pos) {
        let blank = self.blank();
        match mode {
            // Below: cursor to the end of the screen.
            0 => {
                self.erase_row_range(cursor.row, cursor.col, self.cols, blank);
                for row in cursor.row + 1..self.rows {
                    self.erase_row_range(row, 0, self.cols, blank);
                }
            }
            // Above: start of the screen to the cursor.
            1 => {
                for row in 0..cursor.row {
                    self.erase_row_range(row, 0, self.cols, blank);
                }
                self.erase_row_range(cursor.row, 0, cursor.col + 1, blank);
            }
            // Everything.
            2 => {
                for row in 0..self.rows {
                    self.erase_row_range(row, 0, self.cols, blank);
                }
            }
            // The saved lines only, which is what `ED 3` means and what a program
            // sends when it wants to drop its history without disturbing what the
            // user is looking at. Nothing on screen changes, so nothing is damaged.
            3 => {
                self.scrollback.clear();
            }
            _ => {}
        }
    }

    /// Erase part of one line, relative to the cursor.
    pub fn erase_in_line(&mut self, mode: u16, cursor: Pos) {
        let blank = self.blank();
        match mode {
            // Cursor to the end of the line.
            0 => self.erase_row_range(cursor.row, cursor.col, self.cols, blank),
            // Start of the line to the cursor.
            1 => self.erase_row_range(cursor.row, 0, cursor.col + 1, blank),
            // The whole line.
            2 => self.erase_row_range(cursor.row, 0, self.cols, blank),
            _ => {}
        }
    }

    fn erase_row_range(&mut self, row: usize, start: usize, end: usize, blank: Cell) {
        if row >= self.rows {
            return;
        }
        let cols = self.cols;
        let r = &mut self.screen[row];
        if r.len() < cols {
            r.resize(cols);
        }
        let (start, end) = (start.min(cols), end.min(cols));
        for cell in &mut r.cells_mut(cols)[start..end] {
            *cell = blank;
        }
        // An erase that ends between the two halves of a wide character leaves the half
        // outside the range without its partner. `EL` and `ED` are where a user meets
        // this most: clearing from the cursor to the end of a line that holds CJK text
        // puts the cursor in the middle of a character about half the time.
        r.repair_wide(cols, start.saturating_sub(1)..=end, &blank);
        // A row cleared from end to end holds no first half of anything, so it is no
        // longer the continuation of the row above it. Leaving the flag set makes a
        // later reflow or copy treat a blank row as part of a logical line, which
        // shows up as a line that gains a screenful of spaces when the window widens.
        if start == 0 && end >= cols {
            r.set_wrapped(false);
        }
        self.damage.mark_row(row);
    }

    /// The tab stops, one per column.
    pub fn tabs(&self) -> &[bool] {
        &self.tabs
    }

    /// Clear every tab stop.
    pub fn clear_tabs(&mut self) {
        self.tabs.fill(false);
    }

    /// Set a tab stop at `col`.
    pub fn set_tab(&mut self, col: usize) {
        if col < self.cols {
            self.tabs[col] = true;
        }
    }

    /// Clear the tab stop at `col`.
    pub fn clear_tab(&mut self, col: usize) {
        if col < self.cols {
            self.tabs[col] = false;
        }
    }

    /// The next tab stop to the right of `col`, or the last column.
    pub fn next_tab(&self, col: usize) -> usize {
        ((col + 1)..self.cols)
            .find(|&c| self.tabs[c])
            .unwrap_or(self.cols.saturating_sub(1))
    }

    /// The next tab stop to the left of `col`, or column zero.
    pub fn prev_tab(&self, col: usize) -> usize {
        (0..col).rev().find(|&c| self.tabs[c]).unwrap_or(0)
    }

    /// Resize the grid, re-wrapping the content and keeping the cursor on the same
    /// character.
    pub fn resize(&mut self, cols: usize, rows: usize, cursor: &mut Pos) {
        let cols = cols.max(1);
        let rows = rows.max(1);
        if cols == self.cols && rows == self.rows {
            return;
        }
        if cols != self.cols {
            self.reflow(cols, cursor);
        }
        self.cols = cols;
        self.resize_rows(rows, cursor);
        self.tabs = default_tabs(cols);
        self.damage.resize(rows);
        self.damage.mark_all();
    }

    /// Change the screen height, anchoring the bottom of the content.
    ///
    /// Growing pulls lines back out of the scrollback, which is what makes a terminal
    /// feel like it did the right thing when you maximize it: the prompt stays at the
    /// bottom and older output reappears above it.
    fn resize_rows(&mut self, rows: usize, cursor: &mut Pos) {
        let old_rows = self.rows;
        if rows == old_rows {
            return;
        }
        if rows < old_rows {
            let removed = old_rows - rows;
            // Lines fall off the top, and the cursor moves up with them.
            for _ in 0..removed {
                let row = self.screen.remove(0);
                self.push_scrollback(row);
            }
            cursor.row = cursor.row.saturating_sub(removed);
        } else {
            // Only lines that actually exist in the scrollback come back on screen, and
            // they push the content down with them. Once the scrollback runs dry the
            // remaining new rows are blank and go at the *bottom*. Padding the top
            // instead would leave a freshly opened terminal's prompt floating halfway
            // down the window after the user maximized it.
            let mut pulled = 0;
            for _ in 0..(rows - old_rows) {
                let Some(row) = self.scrollback.pop_back() else {
                    break;
                };
                self.screen.insert(0, row);
                pulled += 1;
            }
            cursor.row += pulled;
            while self.screen.len() < rows {
                self.screen.push(Row::blank(self.cols));
            }
        }
        self.rows = rows;
        cursor.row = cursor.row.min(rows - 1);
        self.scroll_region = (0, rows - 1);
    }

    /// Re-wrap every line to `new_cols`, keeping the cursor on the same character.
    ///
    /// Wrapping is about logical lines, not physical rows. A shell command the user
    /// typed that happened to be longer than the window is one logical line spanning
    /// several rows, and resizing has to re-flow it as a unit. Treating each row as its
    /// own line is what makes a terminal shred a long command across the screen when
    /// you resize, which is the single most visible resize bug there is.
    fn reflow(&mut self, new_cols: usize, cursor: &mut Pos) {
        let old_cols = self.cols;
        let screen_start = self.scrollback.len();

        // Everything the grid holds, scrollback first.
        let mut all: Vec<Row> = self.scrollback.drain(..).collect();
        all.append(&mut self.screen);
        if all.is_empty() {
            return;
        }

        let cursor_abs = screen_start + cursor.row;

        // Join soft-wrapped rows into logical lines, remembering where the cursor is.
        let mut logical: Vec<Vec<Cell>> = Vec::new();
        let mut cursor_logical = 0usize;
        let mut cursor_offset = 0usize;
        let mut current: Vec<Cell> = Vec::new();
        for (i, row) in all.iter().enumerate() {
            if i == cursor_abs {
                cursor_logical = logical.len();
                cursor_offset = current.len() + cursor.col;
            }
            let width = if row.is_wrapped() {
                old_cols
            } else {
                row.content_len()
            };
            for col in 0..width {
                current.push(row.get(col));
            }
            if !row.is_wrapped() {
                logical.push(core::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            logical.push(current);
        }
        if cursor_logical >= logical.len() {
            cursor_logical = logical.len().saturating_sub(1);
        }

        // Re-split each logical line into rows of the new width.
        let mut rebuilt: Vec<Row> = Vec::new();
        let mut cursor_new_row = 0usize;
        let mut cursor_new_col = 0usize;
        for (li, line) in logical.iter().enumerate() {
            let start = rebuilt.len();
            let mut chunk = Row::new(new_cols);
            if line.is_empty() {
                chunk.resize(new_cols);
                rebuilt.push(chunk);
                if li == cursor_logical {
                    cursor_new_row = start;
                    cursor_new_col = 0;
                }
                continue;
            }
            // A cursor can sit one past the last character, holding a pending wrap.
            // There is no cell there, so it lands on the last character instead.
            let offset = cursor_offset.min(line.len() - 1);
            let mut col = 0usize;
            for (i, cell) in line.iter().enumerate() {
                // A wide character occupies two cells, and the second one is a
                // spacer that draws nothing on its own. If the leading half landed in
                // the last column of the new width, the spacer would open the next
                // row and the glyph would be drawn cut in half at the right edge, so
                // the pair moves to the next row whole instead.
                let wide = cell.flags.contains(CellFlags::WIDE_CHAR);
                if col > 0 && (col >= new_cols || (wide && col + 1 >= new_cols)) {
                    chunk.set_wrapped(true);
                    rebuilt.push(core::mem::replace(&mut chunk, Row::new(new_cols)));
                    col = 0;
                }
                if li == cursor_logical && i == offset {
                    cursor_new_row = rebuilt.len();
                    cursor_new_col = col;
                }
                *chunk.get_mut(col) = *cell;
                col += 1;
            }
            chunk.set_wrapped(false);
            rebuilt.push(chunk);
        }

        // Anchor the bottom of the screen to the bottom of the content, and never let
        // the window be pulled off that anchor.
        //
        // Pulling it up to keep the cursor on screen is the obvious thing and it loses
        // text: the cursor reflows above the top of the new screen exactly when the
        // content grew below it, so following the cursor puts the screen's last row
        // short of the content's end, and every row in between is dropped — the
        // scrollback was already cut at the anchor and holds nothing below it, so there
        // is no copy left to restore from. The cursor is clamped into the screen a few
        // lines down instead, which is a cursor drawn on the wrong row until the program
        // writes again, and a terminal's own screen is not the place to lose a program's
        // output for.
        let total = rebuilt.len();
        let start = total.saturating_sub(self.rows);

        self.scrollback = rebuilt[..start].iter().cloned().collect();
        self.screen = rebuilt[start..].to_vec();
        while self.screen.len() < self.rows {
            self.screen.push(Row::blank(new_cols));
        }
        cursor.row = cursor_new_row.saturating_sub(start).min(self.rows - 1);
        cursor.col = cursor_new_col.min(new_cols - 1);

        while self.scrollback.len() > self.scrollback_limit {
            self.scrollback.pop_front();
        }
    }
}

/// The default tab stops: every eight columns, starting at column eight.
fn default_tabs(cols: usize) -> Vec<bool> {
    (0..cols).map(|c| c > 0 && c % 8 == 0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::NamedColor;

    fn text(grid: &Grid, row: usize) -> String {
        let r = grid.row(row);
        (0..grid.cols()).map(|c| r.get(c).ch).collect()
    }

    fn write(grid: &mut Grid, row: usize, col: usize, s: &str) {
        let cols = grid.cols();
        let bg = grid.erase_bg();
        for (i, ch) in s.chars().enumerate() {
            let cell = Cell {
                ch,
                bg,
                ..Cell::blank()
            };
            grid.row_mut(row).write_at(cols, col + i, &cell, 1);
        }
    }

    /// Write one character of the width it actually draws at, so that a wide character
    /// gets its `WIDE_CHAR` flag and its spacer the way the terminal's own output path
    /// gives it one.
    fn write_wide(grid: &mut Grid, row: usize, col: usize, ch: char) -> usize {
        let cols = grid.cols();
        let bg = grid.erase_bg();
        let cell = Cell {
            ch,
            bg,
            ..Cell::blank()
        };
        grid.row_mut(row).write_at(cols, col, &cell, 2)
    }

    /// Whether `row` continues into the row below it.
    fn wrapped(grid: &Grid, row: usize) -> bool {
        grid.row(row).is_wrapped()
    }

    #[test]
    fn a_new_grid_is_blank() {
        let g = Grid::new(10, 3);
        assert_eq!(g.cols(), 10);
        assert_eq!(g.rows(), 3);
        assert_eq!(g.total_rows(), 3);
        for row in 0..3 {
            assert_eq!(text(&g, row), " ".repeat(10));
        }
    }

    #[test]
    fn default_tab_stops_are_every_eight_columns() {
        let g = Grid::new(24, 2);
        assert_eq!(g.next_tab(0), 8);
        assert_eq!(g.next_tab(8), 16);
        assert_eq!(
            g.next_tab(16),
            23,
            "past the last stop the cursor goes to the edge"
        );
        assert_eq!(g.prev_tab(20), 16);
        assert_eq!(g.prev_tab(8), 0);
    }

    #[test]
    fn scrolling_the_whole_screen_pushes_lines_into_history() {
        let mut g = Grid::new(5, 3);
        write(&mut g, 0, 0, "one");
        write(&mut g, 1, 0, "two");
        write(&mut g, 2, 0, "three");
        g.scroll_up(1);
        assert_eq!(g.scrollback_len(), 1);
        let first = g
            .scrollback_row(0)
            .expect("a line should have scrolled off");
        assert_eq!((0..3).map(|c| first.get(c).ch).collect::<String>(), "one");
        assert_eq!(text(&g, 0), "two  ");
        assert_eq!(text(&g, 1), "three");
    }

    #[test]
    fn scrollback_keeps_its_colours_but_drops_trailing_blanks() {
        let mut g = Grid::new(20, 2);
        let red = Color::indexed(NamedColor::Red.index());
        let cols = g.cols();
        let cell = Cell {
            ch: 'x',
            fg: red,
            bg: red,
            ..Cell::blank()
        };
        g.row_mut(0).write_at(cols, 0, &cell, 1);
        g.scroll_up(1);
        let row = g.scrollback_row(0).unwrap();
        assert_eq!(row.len(), 1, "the nineteen trailing blanks should be gone");
        assert_eq!(row.get(0).fg, red);
    }

    #[test]
    fn scrollback_respects_its_limit() {
        let mut g = Grid::new(4, 2);
        g.set_scrollback_limit(2);
        for i in 0..5 {
            write(&mut g, 0, 0, &format!("{i}"));
            g.scroll_up(1);
        }
        assert_eq!(g.scrollback_len(), 2);
    }

    #[test]
    fn a_scrollback_limit_of_zero_discards_history() {
        let mut g = Grid::new(4, 2);
        g.set_scrollback_limit(0);
        write(&mut g, 0, 0, "x");
        g.scroll_up(1);
        assert_eq!(g.scrollback_len(), 0);
    }

    #[test]
    fn scrolling_inside_a_region_does_not_touch_history() {
        let mut g = Grid::new(5, 4);
        g.set_scroll_region(1, 2);
        write(&mut g, 0, 0, "top");
        write(&mut g, 1, 0, "mid");
        write(&mut g, 3, 0, "bot");
        g.scroll_up(1);
        assert_eq!(g.scrollback_len(), 0, "a region scroll is not history");
        assert_eq!(
            text(&g, 0),
            "top  ",
            "the line outside the region must not move"
        );
        assert_eq!(text(&g, 3), "bot  ");
        assert_eq!(text(&g, 2), "     ", "the region scrolled");
    }

    #[test]
    fn scroll_down_fills_from_the_top() {
        let mut g = Grid::new(5, 3);
        write(&mut g, 0, 0, "a");
        write(&mut g, 2, 0, "c");
        g.scroll_down(1);
        assert_eq!(text(&g, 0), "     ");
        assert_eq!(text(&g, 1), "a    ");
        assert_eq!(text(&g, 2), "     ", "c fell off the bottom");
    }

    #[test]
    fn insert_lines_pushes_content_down_within_the_region() {
        let mut g = Grid::new(5, 4);
        for (i, s) in ["a", "b", "c", "d"].iter().enumerate() {
            write(&mut g, i, 0, s);
        }
        g.insert_lines(1, 1);
        assert_eq!(text(&g, 0), "a    ");
        assert_eq!(text(&g, 1), "     ");
        assert_eq!(text(&g, 2), "b    ");
        assert_eq!(text(&g, 3), "c    ", "d fell off the bottom");
    }

    #[test]
    fn delete_lines_pulls_content_up() {
        let mut g = Grid::new(5, 4);
        for (i, s) in ["a", "b", "c", "d"].iter().enumerate() {
            write(&mut g, i, 0, s);
        }
        g.delete_lines(1, 1);
        assert_eq!(text(&g, 0), "a    ");
        assert_eq!(text(&g, 1), "c    ");
        assert_eq!(text(&g, 2), "d    ");
        assert_eq!(text(&g, 3), "     ");
    }

    #[test]
    fn insert_lines_outside_the_region_does_nothing() {
        let mut g = Grid::new(5, 4);
        g.set_scroll_region(1, 2);
        write(&mut g, 0, 0, "a");
        g.insert_lines(0, 1);
        assert_eq!(text(&g, 0), "a    ", "row 0 is outside the region");
    }

    #[test]
    fn erase_below_the_cursor_clears_the_rest_of_the_screen() {
        let mut g = Grid::new(4, 3);
        for i in 0..3 {
            write(&mut g, i, 0, "abcd");
        }
        g.erase_in_display(0, Pos::new(1, 1));
        assert_eq!(text(&g, 0), "abcd");
        assert_eq!(text(&g, 1), "a   ", "the cursor column itself is erased");
        assert_eq!(text(&g, 2), "    ");
    }

    #[test]
    fn erase_above_the_cursor_clears_up_to_and_including_it() {
        let mut g = Grid::new(4, 3);
        for i in 0..3 {
            write(&mut g, i, 0, "abcd");
        }
        g.erase_in_display(1, Pos::new(1, 2));
        assert_eq!(text(&g, 0), "    ");
        assert_eq!(
            text(&g, 1),
            "   d",
            "the erase stops at the cursor, not at the line"
        );
        assert_eq!(text(&g, 2), "abcd");
    }

    #[test]
    fn erase_in_line_modes() {
        let mut g = Grid::new(4, 2);
        write(&mut g, 0, 0, "abcd");
        g.erase_in_line(1, Pos::new(0, 1));
        assert_eq!(text(&g, 0), "  cd");

        write(&mut g, 1, 0, "abcd");
        g.erase_in_line(0, Pos::new(1, 2));
        assert_eq!(text(&g, 1), "ab  ");
    }

    #[test]
    fn an_erase_uses_the_current_background() {
        let mut g = Grid::new(4, 2);
        let red = Color::indexed(NamedColor::Red.index());
        g.set_erase_bg(red);
        g.erase_in_line(2, Pos::new(0, 0));
        assert_eq!(g.row(0).get(0).bg, red, "BCE must leave the colour behind");
    }

    #[test]
    fn clearing_the_screen_keeps_history_unless_asked() {
        let mut g = Grid::new(4, 2);
        write(&mut g, 0, 0, "x");
        g.scroll_up(1);
        assert_eq!(g.scrollback_len(), 1);
        g.clear_screen(false);
        assert_eq!(
            g.scrollback_len(),
            1,
            "ED 2 must not eat the user's history"
        );
        g.clear_screen(true);
        assert_eq!(g.scrollback_len(), 0, "ED 3 is the one that clears history");
    }

    #[test]
    fn growing_the_screen_pulls_lines_back_out_of_history() {
        let mut g = Grid::new(4, 2);
        write(&mut g, 0, 0, "old");
        g.scroll_up(1);
        assert_eq!(g.scrollback_len(), 1);
        let mut cursor = Pos::new(1, 0);
        g.resize(4, 3, &mut cursor);
        assert_eq!(g.scrollback_len(), 0, "the line should come back on screen");
        assert_eq!(text(&g, 0), "old ");
        assert_eq!(g.rows(), 3);
    }

    #[test]
    fn shrinking_the_screen_pushes_lines_into_history() {
        let mut g = Grid::new(4, 4);
        for (i, s) in ["a", "b", "c", "d"].iter().enumerate() {
            write(&mut g, i, 0, s);
        }
        let mut cursor = Pos::new(3, 0);
        g.resize(4, 2, &mut cursor);
        assert_eq!(g.scrollback_len(), 2);
        assert_eq!(text(&g, 0), "c   ");
        assert_eq!(text(&g, 1), "d   ");
        assert_eq!(cursor.row, 1, "the cursor rides up with its line");
    }

    #[test]
    fn narrowing_re_wraps_a_long_logical_line_instead_of_truncating_it() {
        let mut g = Grid::new(10, 4);
        // One logical line that fills the width, so it soft-wraps.
        write(&mut g, 0, 0, "abcdefghij");
        g.row_mut(0).set_wrapped(true);
        write(&mut g, 1, 0, "klmno");

        let mut cursor = Pos::new(1, 5);
        g.resize(5, 4, &mut cursor);

        let joined: String = (0..g.total_rows())
            .filter_map(|i| g.row_from_history(i))
            .map(|r| (0..r.len().min(5)).map(|c| r.get(c).ch).collect::<String>())
            .collect::<Vec<_>>()
            .join("|");
        assert!(
            joined.contains("abcde|fghij|klmno"),
            "the logical line should re-wrap, got {joined}"
        );
    }

    #[test]
    fn widening_joins_a_wrapped_line_back_together() {
        let mut g = Grid::new(5, 4);
        write(&mut g, 0, 0, "abcde");
        g.row_mut(0).set_wrapped(true);
        write(&mut g, 1, 0, "fghij");

        // The cursor sits on the 'h', the third character of the second row.
        let mut cursor = Pos::new(1, 2);
        g.resize(10, 4, &mut cursor);

        assert_eq!(text(&g, 0), "abcdefghij");
        assert!(!g.row(0).is_wrapped());
        assert_eq!(cursor.row, 0, "the cursor's line merged with the one above");
        assert_eq!(cursor.col, 7);
        assert_eq!(g.row(cursor.row).get(cursor.col).ch, 'h');
    }

    #[test]
    fn the_cursor_stays_on_the_same_character_through_a_reflow() {
        let mut g = Grid::new(10, 4);
        write(&mut g, 0, 0, "abcdefghij");
        g.row_mut(0).set_wrapped(true);
        write(&mut g, 1, 0, "klmno");
        let mut cursor = Pos::new(1, 3); // sitting on the 'n'
        g.resize(5, 4, &mut cursor);
        assert_eq!(
            g.row(cursor.row).get(cursor.col).ch,
            'n',
            "the cursor must still be on the character it was on"
        );
    }

    #[test]
    fn a_narrowing_reflow_keeps_the_rows_that_would_fall_below_the_screen() {
        // A cursor homed to the top of a full screen is what makes a reflow pull the
        // window up. Allowed past the bottom anchor, that pull puts the last row of the
        // screen above the end of the content, and the rows between there and the end
        // are dropped from the screen without ever reaching the scrollback — from where
        // nothing can bring them back, because nothing else holds a copy.
        let mut g = Grid::new(20, 6);
        for row in 0..5 {
            write(&mut g, row, 0, &format!("line{row}"));
        }
        let mut cursor = Pos::new(0, 0);
        g.resize(4, 6, &mut cursor);

        let mut all = String::new();
        for index in 0..g.total_rows() {
            let Some(row) = g.row_from_history(index) else {
                continue;
            };
            // No separator between rows, because there is none on the screen: a logical
            // line that was wrapped is a run of characters that ends at the right margin
            // and continues on the next row, and it is the characters being present that
            // this is asking about.
            for col in 0..g.cols() {
                all.push(row.get(col).ch);
            }
        }
        for row in 0..5 {
            assert!(
                all.contains(&format!("line{row}")),
                "line{row} is nowhere in the history: {all:?}"
            );
        }
    }

    #[test]
    fn resizing_to_the_same_size_is_a_no_op() {
        let mut g = Grid::new(10, 4);
        write(&mut g, 0, 0, "keep");
        let mut cursor = Pos::new(2, 3);
        g.resize(10, 4, &mut cursor);
        assert_eq!(text(&g, 0), "keep      ");
        assert_eq!(cursor, Pos::new(2, 3));
    }

    #[test]
    fn a_zero_sized_resize_is_clamped_rather_than_panicking() {
        let mut g = Grid::new(10, 4);
        let mut cursor = Pos::new(1, 1);
        g.resize(0, 0, &mut cursor);
        assert!(g.cols() >= 1);
        assert!(g.rows() >= 1);
    }

    #[test]
    fn the_cursor_never_lands_outside_the_screen_after_a_resize() {
        let mut g = Grid::new(20, 10);
        for i in 0..10 {
            write(&mut g, i, 0, "0123456789");
        }
        let mut cursor = Pos::new(9, 10);
        g.resize(3, 2, &mut cursor);
        assert!(cursor.row < g.rows(), "row {} of {}", cursor.row, g.rows());
        assert!(cursor.col < g.cols(), "col {} of {}", cursor.col, g.cols());
    }

    #[test]
    fn erasing_out_of_range_does_not_panic() {
        let mut g = Grid::new(4, 2);
        g.erase_in_display(0, Pos::new(99, 99));
        g.erase_in_line(0, Pos::new(99, 99));
        g.erase_in_display(7, Pos::new(0, 0));
    }

    #[test]
    fn damage_is_flagged_by_every_mutating_operation() {
        let mut g = Grid::new(4, 4);
        g.damage_mut().clear();
        write(&mut g, 1, 0, "x");
        assert!(g.damage().is_dirty(1));

        g.damage_mut().clear();
        g.scroll_up(1);
        assert!(!g.damage().is_empty());

        g.damage_mut().clear();
        g.insert_lines(0, 1);
        assert!(!g.damage().is_empty());
    }

    #[test]
    fn history_access_spans_the_scrollback_and_the_screen() {
        let mut g = Grid::new(4, 2);
        write(&mut g, 0, 0, "old");
        write(&mut g, 1, 0, "new");
        g.scroll_up(1);
        let all: Vec<String> = (0..g.total_rows())
            .map(|i| {
                let r = g.row_from_history(i).unwrap();
                (0..4).map(|c| r.get(c).ch).collect()
            })
            .collect();
        assert_eq!(all, vec!["old ", "new ", "    "]);
    }

    #[test]
    fn ed_three_drops_the_history_and_leaves_the_screen_alone() {
        let mut g = Grid::new(4, 2);
        write(&mut g, 0, 0, "old");
        g.scroll_up(1);
        write(&mut g, 0, 0, "new");
        assert_eq!(g.scrollback_len(), 1);

        g.erase_in_display(3, Pos::new(0, 0));

        assert_eq!(g.scrollback_len(), 0, "ED 3 is the history eraser");
        assert_eq!(
            text(&g, 0),
            "new ",
            "a program clearing its scrollback must not blank the screen"
        );
    }

    #[test]
    fn erasing_a_whole_row_clears_its_wrap_flag() {
        let mut g = Grid::new(5, 3);
        write(&mut g, 0, 0, "abcde");
        g.row_mut(0).set_wrapped(true);
        write(&mut g, 1, 0, "f");

        g.erase_in_display(2, Pos::new(0, 0));

        assert!(!wrapped(&g, 0), "a blank row continues into nothing");
        assert_eq!(text(&g, 0), "     ");
    }

    #[test]
    fn erasing_part_of_a_row_leaves_its_wrap_flag_alone() {
        // Only a whole-row erase unlinks a row. Erasing a prefix leaves the tail,
        // which is still the first half of what runs onto the next row.
        let mut g = Grid::new(5, 3);
        write(&mut g, 0, 0, "abcde");
        g.row_mut(0).set_wrapped(true);

        g.erase_in_line(1, Pos::new(0, 2));

        assert_eq!(text(&g, 0), "   de");
        assert!(wrapped(&g, 0));
    }

    #[test]
    fn deleting_lines_breaks_the_wrap_flag_above_the_edit() {
        let mut g = Grid::new(5, 4);
        write(&mut g, 0, 0, "abcde");
        g.row_mut(0).set_wrapped(true);
        write(&mut g, 1, 0, "1");
        write(&mut g, 2, 0, "2");

        g.delete_lines(1, 1);

        assert!(
            !wrapped(&g, 0),
            "row 0 no longer continues into what the delete pulled up"
        );
        assert_eq!(text(&g, 1), "2    ");
    }

    #[test]
    fn inserting_lines_breaks_the_wrap_flag_above_the_edit() {
        let mut g = Grid::new(5, 4);
        write(&mut g, 0, 0, "abcde");
        g.row_mut(0).set_wrapped(true);
        write(&mut g, 1, 0, "1");

        g.insert_lines(1, 1);

        assert!(!wrapped(&g, 0), "row 0 continues into a blank line now");
        assert_eq!(text(&g, 1), "     ");
        assert_eq!(text(&g, 2), "1    ");
    }

    #[test]
    fn inserting_lines_breaks_the_wrap_flag_on_the_row_pushed_to_the_bottom() {
        let mut g = Grid::new(5, 4);
        write(&mut g, 2, 0, "abcde");
        g.row_mut(2).set_wrapped(true);
        write(&mut g, 3, 0, "1");

        g.insert_lines(0, 1);

        // The line that was at row 2 is now at row 3, and its continuation fell off
        // the bottom of the region rather than following it.
        assert!(!wrapped(&g, 3));
    }

    #[test]
    fn reflow_moves_a_wide_character_whole_rather_than_splitting_it() {
        // Joining the two rows gives `abcd`, a blank, then the wide character. Split
        // every three cells and the leading half lands in the last column of a row
        // with its spacer opening the next one, so the renderer draws a half glyph at
        // the right edge and a stray blank below it.
        let mut g = Grid::new(5, 4);
        write(&mut g, 0, 0, "abcd");
        g.row_mut(0).set_wrapped(true);
        assert_eq!(write_wide(&mut g, 1, 0, '中'), 2, "it fits at the start");

        let mut cursor = Pos::new(1, 1);
        g.resize(3, 4, &mut cursor);

        let rows: Vec<String> = (0..g.total_rows())
            .map(|i| {
                let r = g.row_from_history(i).unwrap();
                (0..g.cols()).map(|c| r.get(c).ch).collect()
            })
            .collect();
        assert_eq!(rows[0], "abc");
        assert_eq!(
            rows[1], "d  ",
            "one column left is not room for a wide character"
        );
        assert_eq!(rows[2], "中  ", "so it moves to the next row instead");
        let wide = g.row_from_history(2).unwrap();
        assert!(
            wide.get(0).flags.contains(CellFlags::WIDE_CHAR) && wide.get(1).is_wide_spacer(),
            "the leading half and its spacer must stay side by side"
        );
    }
}
