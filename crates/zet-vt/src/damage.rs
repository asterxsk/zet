//! What changed since the last frame.
//!
//! A terminal that repaints its whole grid every frame is a terminal that burns a
//! laptop battery to display a blinking cursor. The renderer asks this type what
//! changed and touches only those rows.
//!
//! The granularity is a row, not a cell. A row is a contiguous run of cells that the
//! renderer has to walk in order anyway, so tracking finer would cost more in bookkeeping
//! than it saves in fill. Programs that emit a single changed cell are rare next to
//! programs that repaint a line of status output, which is the case this is built for.
//!
//! # The row-level readers are waiting for a retained frame
//!
//! Everything here that names a *row* — [`Damage::is_dirty`], [`Damage::is_full`],
//! [`Damage::dirty_rows`], [`Damage::bounding_rows`] — is written, tested, and called by
//! nothing in production. The one reader the app uses is [`Damage::is_empty`], to decide
//! whether to draw at all.
//!
//! That is deliberate rather than forgotten. Drawing only the rows that changed means the
//! previous frame's rectangles have to survive, because a row omitted from a frame that
//! is rebuilt from scratch and drawn over a cleared surface is a row that gets erased —
//! so it needs a retained buffer and an explicit erase of wherever the cursor used to be.
//! The renderer's grid module carries the same reasoning from the drawing side and is
//! where that work would start. The readers are the vocabulary it will need, and they are
//! kept rather than deleted and rewritten, because the marking discipline they read
//! (`mark_row`, `mark_rows`, `mark_all`) is maintained by production code either way.

/// The rows that changed since the last frame was drawn.
#[derive(Clone, Debug)]
pub struct Damage {
    /// One flag per screen row.
    rows: Vec<bool>,
    /// Set when everything must be redrawn regardless of the flags, which is what a
    /// palette change, a font change, or a resize does.
    full: bool,
    /// How many rows are flagged, so emptiness is a comparison rather than a scan.
    count: usize,
}

impl Damage {
    /// A tracker for a screen `rows` tall, with nothing dirty.
    pub fn new(rows: usize) -> Self {
        Damage {
            rows: vec![false; rows],
            full: false,
            count: 0,
        }
    }

    /// Resize the tracker. A row-count change dirties everything, because every row
    /// that survives has moved.
    pub fn resize(&mut self, rows: usize) {
        if self.rows.len() == rows {
            return;
        }
        self.rows.clear();
        self.rows.resize(rows, true);
        self.full = true;
        self.count = rows;
    }

    /// Flag one row.
    pub fn mark_row(&mut self, row: usize) {
        if let Some(slot) = self.rows.get_mut(row)
            && !*slot
        {
            *slot = true;
            self.count += 1;
        }
    }

    /// Flag a half-open range of rows.
    pub fn mark_rows(&mut self, rows: core::ops::Range<usize>) {
        for row in rows {
            self.mark_row(row);
        }
    }

    /// Flag every row.
    pub fn mark_all(&mut self) {
        self.full = true;
        self.rows.fill(true);
        self.count = self.rows.len();
    }

    /// Whether anything changed.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Whether a specific row changed.
    pub fn is_dirty(&self, row: usize) -> bool {
        self.rows.get(row).copied().unwrap_or(false)
    }

    /// How many rows are flagged.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Whether the whole screen needs redrawing.
    pub fn is_full(&self) -> bool {
        self.full
    }

    /// Forget everything. Called once the frame has been drawn.
    pub fn clear(&mut self) {
        self.full = false;
        if self.count == 0 {
            return;
        }
        self.rows.fill(false);
        self.count = 0;
    }

    /// The flagged rows, in order.
    pub fn dirty_rows(&self) -> impl Iterator<Item = usize> + '_ {
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(i, &d)| if d { Some(i) } else { None })
    }

    /// The smallest row range that covers everything flagged.
    ///
    /// A renderer that has to submit work per row will often do better submitting one
    /// range, since the rows between two changes usually cost less to redraw than the
    /// second draw call costs to issue.
    pub fn bounding_rows(&self) -> Option<(usize, usize)> {
        let first = self.rows.iter().position(|&d| d)?;
        let last = self.rows.iter().rposition(|&d| d)?;
        Some((first, last))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_tracker_is_clean() {
        let d = Damage::new(24);
        assert!(d.is_empty());
        assert_eq!(d.count(), 0);
        assert!(!d.is_full());
        assert_eq!(d.bounding_rows(), None);
    }

    #[test]
    fn marking_a_row_twice_counts_once() {
        let mut d = Damage::new(24);
        d.mark_row(3);
        d.mark_row(3);
        assert_eq!(d.count(), 1);
        assert!(d.is_dirty(3));
        assert!(!d.is_dirty(4));
    }

    #[test]
    fn marking_out_of_range_is_ignored() {
        let mut d = Damage::new(4);
        d.mark_row(99);
        assert!(d.is_empty());
    }

    #[test]
    fn clear_resets_everything() {
        let mut d = Damage::new(4);
        d.mark_all();
        assert!(d.is_full());
        d.clear();
        assert!(d.is_empty());
        assert!(!d.is_full());
        assert_eq!(d.bounding_rows(), None);
    }

    #[test]
    fn clear_on_a_clean_tracker_is_a_no_op() {
        let mut d = Damage::new(4);
        d.clear();
        assert!(d.is_empty());
    }

    #[test]
    fn dirty_rows_lists_only_the_flagged_ones_in_order() {
        let mut d = Damage::new(8);
        d.mark_row(5);
        d.mark_row(1);
        assert_eq!(d.dirty_rows().collect::<Vec<_>>(), vec![1, 5]);
    }

    #[test]
    fn the_bounding_range_covers_the_gaps() {
        let mut d = Damage::new(24);
        d.mark_row(2);
        d.mark_row(20);
        assert_eq!(d.bounding_rows(), Some((2, 20)));
    }

    #[test]
    fn resizing_the_screen_dirties_every_row() {
        let mut d = Damage::new(24);
        d.clear();
        d.resize(30);
        assert!(d.is_full());
        assert_eq!(d.count(), 30);
        assert!(d.is_dirty(29));
    }

    #[test]
    fn resizing_to_the_same_height_changes_nothing() {
        let mut d = Damage::new(24);
        d.clear();
        d.resize(24);
        assert!(d.is_empty());
    }

    #[test]
    fn mark_rows_marks_a_half_open_range() {
        let mut d = Damage::new(8);
        d.mark_rows(2..5);
        assert_eq!(d.dirty_rows().collect::<Vec<_>>(), vec![2, 3, 4]);
    }
}
