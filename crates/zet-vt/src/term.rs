//! The terminal state machine.
//!
//! This is where the parser's events become screen changes. Cursor movement, the SGR
//! pen, the DEC private modes, the scrolling region, the alternate screen, and the
//! replies a program expects when it asks a question all live here.
//!
//! Four things in this file are easy to get wrong and are worth reading before editing:
//!
//! - **The pending wrap.** After printing in the last column the cursor does not move
//!   to the next line. It stays in the last column holding a wrap flag, and the line
//!   break happens when the *next* character arrives. Without this, every full-width
//!   line gains a spurious blank line, which is the bug that makes a terminal look
//!   broken to anyone who notices.
//! - **The alternate screen.** A program that switches to it expects the primary screen
//!   and its scrollback to be exactly as it left them, cursor included. Restoring only
//!   the grid and not the cursor loses the user's place in their shell session.
//! - **Combining marks.** A cell holds one `char`, so a cluster that is several code
//!   points lives in a side table keyed by position. Every operation that relocates
//!   cells has to throw that table away, because the keys are positions and the
//!   positions have just changed meaning.
//! - **Synchronized output.** `DECSET 2026` is the mechanism that makes a full-screen
//!   TUI flicker-free. It is reported to the host through [`Term::is_synchronized`], and
//!   the host must not present a frame while it is set.

use std::collections::HashMap;

use unicode_width::UnicodeWidthChar;

use crate::attrs::Attrs;
use crate::cell::Cell;
use crate::color::{Color, ColorSpec, NamedColor};
use crate::grid::{Grid, Pos};
use crate::parser::{Params, Perform, Private};

/// How the terminal should report mouse events.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum MouseMode {
    /// The program did not ask for the mouse.
    #[default]
    None,
    /// Press only. `DECSET 9`.
    X10,
    /// Press and release. `DECSET 1000`.
    Button,
    /// Press, release, and motion while a button is held. `DECSET 1002`.
    Drag,
    /// Every motion. `DECSET 1003`.
    Motion,
}

/// How to encode a mouse report.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum MouseEncoding {
    /// The original single-byte form, which cannot express coordinates past 223.
    #[default]
    X10,
    /// `DECSET 1005`, UTF-8 encoded coordinates.
    Utf8,
    /// `DECSET 1015`, decimal coordinates.
    Urxvt,
    /// `DECSET 1006`, the form every modern program uses.
    Sgr,
}

/// The modes that change how the terminal behaves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct Modes {
    /// `IRM`, `CSI 4 h`. Typing inserts rather than overwrites.
    pub insert: bool,
    /// `LNM`, `CSI 20 h`. A line feed also returns the carriage.
    pub newline: bool,
    /// `DECOM`, `DECSET 6`. Cursor addressing is relative to the scrolling region.
    pub origin: bool,
    /// `DECAWM`, `DECSET 7`. Printing past the last column wraps.
    pub autowrap: bool,
    /// `DECCKM`, `DECSET 1`. Arrow keys send `SS3` rather than `CSI`.
    pub app_cursor_keys: bool,
    /// `DECKPAM`, `ESC =`. Keypad keys send application sequences.
    pub app_keypad: bool,
    /// `DECTCEM`, `DECSET 25`. Whether the cursor is drawn.
    pub cursor_visible: bool,
    /// `DECSCNM`, `DECSET 5`. Reverse the whole screen.
    pub reverse_video: bool,
    /// `DECSET 2004`. Wrap pasted text in the paste markers.
    pub bracketed_paste: bool,
    /// `DECSET 1004`. Report focus changes.
    pub focus_events: bool,
    /// `DECSET 2026`. Hold the frame until the program says it is complete.
    pub synchronized: bool,
    /// What the program wants to know about the mouse.
    pub mouse: MouseMode,
    /// How the program wants mouse reports encoded.
    pub mouse_encoding: MouseEncoding,
    /// Whether the alternate screen is active.
    pub alt_screen: bool,
}

impl Default for Modes {
    fn default() -> Self {
        Modes {
            insert: false,
            newline: false,
            origin: false,
            autowrap: true,
            app_cursor_keys: false,
            app_keypad: false,
            cursor_visible: true,
            reverse_video: false,
            bracketed_paste: false,
            focus_events: false,
            synchronized: false,
            mouse: MouseMode::None,
            mouse_encoding: MouseEncoding::X10,
            alt_screen: false,
        }
    }
}

/// The current drawing state: colours, attributes, and the active hyperlink.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Pen {
    /// Foreground.
    pub fg: Color,
    /// Background.
    pub bg: Color,
    /// Attributes.
    pub attrs: Attrs,
    /// Index into the terminal's hyperlink table plus one, or zero for no link.
    pub link: u16,
}

/// Everything the alternate screen has to put back when it goes away.
struct AltScreen {
    grid: Grid,
    cursor: Pos,
    pen: Pen,
    cursor_visible: bool,
}

/// A cursor position remembered by `DECSC`.
#[derive(Clone, Copy)]
struct SavedCursor {
    pos: Pos,
    pen: Pen,
    origin: bool,
    autowrap: bool,
}

/// A terminal.
pub struct Term {
    grid: Grid,
    alt: Option<AltScreen>,
    cursor: Pos,
    saved: Option<SavedCursor>,
    pen: Pen,
    modes: Modes,
    /// True when the cursor sits in the last column with a wrap pending.
    pending_wrap: bool,
    /// Where the last character was drawn, so a combining mark has somewhere to go.
    last_print: Option<Pos>,
    /// Grapheme clusters that are more than one code point, keyed by position.
    clusters: HashMap<(usize, usize), String>,
    /// Bytes the terminal owes the program: device attributes, cursor reports, replies.
    responses: Vec<u8>,
    /// Hyperlink targets, indexed by the `link` field on a cell minus one.
    links: Vec<String>,
    /// The title the program set.
    title: String,
}

impl Term {
    /// A terminal of the given size.
    pub fn new(cols: usize, rows: usize) -> Self {
        Term {
            grid: Grid::new(cols, rows),
            alt: None,
            cursor: Pos::new(0, 0),
            saved: None,
            pen: Pen::default(),
            modes: Modes::default(),
            pending_wrap: false,
            last_print: None,
            clusters: HashMap::new(),
            responses: Vec::new(),
            links: Vec::new(),
            title: String::new(),
        }
    }

    /// The screen.
    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    /// The screen, mutably. For the host's own scrollback and selection work.
    pub fn grid_mut(&mut self) -> &mut Grid {
        &mut self.grid
    }

    /// Where the cursor is.
    pub fn cursor(&self) -> Pos {
        self.cursor
    }

    /// The current drawing state.
    pub fn pen(&self) -> Pen {
        self.pen
    }

    /// The current modes.
    pub fn modes(&self) -> Modes {
        self.modes
    }

    /// The title the program asked for, or an empty string.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Whether the program is holding the frame.
    ///
    /// A renderer must not present while this is set. That is the whole point of
    /// synchronized output: the program is mid-repaint, and showing the half-finished
    /// screen is exactly the flicker the mode exists to prevent.
    pub fn is_synchronized(&self) -> bool {
        self.modes.synchronized
    }

    /// The hyperlink target for a cell, if it has one.
    pub fn link_for(&self, cell: &Cell) -> Option<&str> {
        if cell.link == 0 {
            return None;
        }
        self.links.get((cell.link - 1) as usize).map(String::as_str)
    }

    /// The full grapheme cluster at a position, or `None` when the cell's own character
    /// is the whole cluster.
    pub fn cluster_at(&self, pos: Pos) -> Option<&str> {
        self.clusters.get(&(pos.row, pos.col)).map(String::as_str)
    }

    /// Take the bytes the terminal owes the program.
    ///
    /// The host writes these to the pty. Draining rather than reading means a program
    /// that never asks anything never makes the host do anything.
    pub fn take_responses(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.responses)
    }

    /// Resize, and put the cursor back inside the new screen.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.grid.resize(cols, rows, &mut self.cursor);
        self.pending_wrap = false;
        self.last_print = None;
        self.clusters.clear();
        if let Some(alt) = &mut self.alt {
            let mut cursor = alt.cursor;
            alt.grid.resize(cols, rows, &mut cursor);
            alt.cursor = cursor;
        }
    }

    /// Hard reset, as `ESC c` asks for.
    pub fn reset(&mut self) {
        let cols = self.grid.cols();
        let rows = self.grid.rows();
        let limit = self.grid.scrollback_limit();
        self.grid = Grid::new(cols, rows);
        self.grid.set_scrollback_limit(limit);
        self.alt = None;
        self.cursor = Pos::new(0, 0);
        self.saved = None;
        self.pen = Pen::default();
        self.modes = Modes::default();
        self.pending_wrap = false;
        self.last_print = None;
        self.clusters.clear();
        self.links.clear();
    }

    /// Drop every remembered cluster. Called by anything that moves cells around.
    fn invalidate_clusters(&mut self) {
        if !self.clusters.is_empty() {
            self.clusters.clear();
        }
    }

    // ---- writing -------------------------------------------------------------

    /// The cell the pen would draw right now, for a given character.
    fn pen_cell(&self, ch: char) -> Cell {
        Cell {
            ch,
            fg: self.pen.fg,
            bg: self.pen.bg,
            attrs: self.pen.attrs,
            link: self.pen.link,
            flags: crate::cell::CellFlags::empty(),
        }
    }

    /// Draw one character at the cursor and advance it.
    fn print_char(&mut self, ch: char) {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0);

        if width == 0 {
            self.combine(ch);
            return;
        }

        let cols = self.grid.cols();

        // The wrap happens now, not when the previous character filled the last column.
        // That covers both cases: the cursor left holding a pending wrap, and a
        // double-width character that cannot fit in what is left of the line and so
        // moves to the next one whole rather than being split or dropped.
        if self.modes.autowrap && (self.pending_wrap || self.cursor.col + width > cols) {
            // The row being left is half of a longer logical line, and this is the only
            // moment that knows it: a wrap the terminal performs leaves no trace on the
            // wire, unlike the newline a program sends. Marking it before the feed rather
            // than after is what makes this survive a scroll, where the row moves up
            // underneath the cursor but keeps its flags.
            self.grid.row_mut(self.cursor.row).set_wrapped(true);
            self.line_feed();
            self.carriage_return();
        }
        self.pending_wrap = false;

        let col = self.cursor.col;

        if self.modes.insert {
            let blank = self.grid.blank();
            self.grid.row_mut(self.cursor.row).insert_cells(
                cols,
                col,
                width.min(cols - col),
                blank,
            );
            self.invalidate_clusters();
        }

        let cell = self.pen_cell(ch);
        self.grid
            .row_mut(self.cursor.row)
            .write_at(cols, col, &cell, width);

        self.last_print = Some(self.cursor);
        self.clusters.remove(&(self.cursor.row, col));

        if col + width < cols {
            self.cursor.col = col + width;
        } else {
            self.cursor.col = cols - 1;
            self.pending_wrap = self.modes.autowrap;
        }
    }

    /// Attach a combining mark to the character before it.
    ///
    /// Dropping these is the difference between a terminal that renders Devanagari,
    /// Thai, and emoji skin tones and one that shows a trail of disconnected marks.
    fn combine(&mut self, ch: char) {
        let Some(pos) = self.last_print else {
            return;
        };
        let base = self.grid.row(pos.row).get(pos.col).ch;
        let entry = self
            .clusters
            .entry((pos.row, pos.col))
            .or_insert_with(|| base.to_string());
        entry.push(ch);
        self.grid.damage_mut().mark_row(pos.row);
    }

    fn line_feed(&mut self) {
        let (_, bottom) = self.grid.scroll_region();
        if self.cursor.row == bottom {
            self.grid.scroll_up(1);
            self.invalidate_clusters();
        } else if self.cursor.row + 1 < self.grid.rows() {
            self.cursor.row += 1;
        }
        self.pending_wrap = false;
        self.last_print = None;
    }

    fn reverse_index(&mut self) {
        let (top, _) = self.grid.scroll_region();
        if self.cursor.row == top {
            self.grid.scroll_down(1);
            self.invalidate_clusters();
        } else {
            self.cursor.row = self.cursor.row.saturating_sub(1);
        }
        self.pending_wrap = false;
        self.last_print = None;
    }

    fn carriage_return(&mut self) {
        self.cursor.col = 0;
        self.pending_wrap = false;
        self.last_print = None;
    }

    fn backspace(&mut self) {
        self.cursor.col = self.cursor.col.saturating_sub(1);
        self.pending_wrap = false;
        self.last_print = None;
    }

    fn tab(&mut self) {
        self.cursor.col = self.grid.next_tab(self.cursor.col);
        self.pending_wrap = false;
        self.last_print = None;
    }

    fn back_tab(&mut self) {
        self.cursor.col = self.grid.prev_tab(self.cursor.col);
        self.pending_wrap = false;
        self.last_print = None;
    }

    // ---- cursor movement -----------------------------------------------------

    /// `CUP` and `HVP`, which are one-based and clamp rather than wrap.
    fn set_cursor(&mut self, params: &Params, origin_relative: bool) {
        let row = params.value_or(0, 1).max(1) as usize - 1;
        let col = params.value_or(1, 1).max(1) as usize - 1;
        let (top, bottom) = self.grid.scroll_region();
        let row = if origin_relative && self.modes.origin {
            (top + row).min(bottom)
        } else {
            row.min(self.grid.rows() - 1)
        };
        self.cursor = Pos::new(row, col.min(self.grid.cols() - 1));
        self.pending_wrap = false;
        self.last_print = None;
    }

    /// Put the cursor at the home position, which origin mode moves to the region.
    fn home_cursor(&mut self) {
        let (top, _) = self.grid.scroll_region();
        self.cursor = Pos::new(if self.modes.origin { top } else { 0 }, 0);
        self.pending_wrap = false;
        self.last_print = None;
    }

    fn move_up(&mut self, n: usize) {
        let (top, _) = self.grid.scroll_region();
        let limit = if self.modes.origin { top } else { 0 };
        self.cursor.row = self.cursor.row.saturating_sub(n).max(limit);
        self.pending_wrap = false;
        self.last_print = None;
    }

    fn move_down(&mut self, n: usize) {
        let (_, bottom) = self.grid.scroll_region();
        let limit = if self.modes.origin {
            bottom
        } else {
            self.grid.rows() - 1
        };
        self.cursor.row = (self.cursor.row + n).min(limit);
        self.pending_wrap = false;
        self.last_print = None;
    }

    fn move_left(&mut self, n: usize) {
        self.cursor.col = self.cursor.col.saturating_sub(n);
        self.pending_wrap = false;
        self.last_print = None;
    }

    fn move_right(&mut self, n: usize) {
        self.cursor.col = (self.cursor.col + n).min(self.grid.cols() - 1);
        self.pending_wrap = false;
        self.last_print = None;
    }

    // ---- modes ---------------------------------------------------------------

    fn set_mode(&mut self, param: u16, private: bool, enable: bool) {
        if !private {
            match param {
                4 => self.modes.insert = enable,
                20 => self.modes.newline = enable,
                _ => {}
            }
            return;
        }
        match param {
            1 => self.modes.app_cursor_keys = enable,
            5 => self.modes.reverse_video = enable,
            6 => {
                self.modes.origin = enable;
                // Setting or clearing origin mode homes the cursor, which is what keeps
                // a program from drawing at a stale position when it flips the mode.
                self.home_cursor();
            }
            7 => self.modes.autowrap = enable,
            9 => self.set_mouse(enable, MouseMode::X10),
            25 => self.modes.cursor_visible = enable,
            47 | 1047 => {
                if enable {
                    self.enter_alt_screen();
                } else {
                    self.exit_alt_screen();
                }
            }
            1000 => self.set_mouse(enable, MouseMode::Button),
            1002 => self.set_mouse(enable, MouseMode::Drag),
            1003 => self.set_mouse(enable, MouseMode::Motion),
            1004 => self.modes.focus_events = enable,
            1005 => self.modes.mouse_encoding = MouseEncoding::Utf8,
            1006 => self.modes.mouse_encoding = MouseEncoding::Sgr,
            1015 => self.modes.mouse_encoding = MouseEncoding::Urxvt,
            1048 => {
                if enable {
                    self.save_cursor();
                } else {
                    self.restore_cursor();
                }
            }
            1049 => {
                // The save has to happen before the switch. The alternate screen has
                // its own cursor, so saving after entering it would remember the wrong
                // position and lose the user's place in the shell.
                if enable {
                    self.save_cursor();
                    self.enter_alt_screen();
                } else {
                    self.exit_alt_screen();
                    self.restore_cursor();
                }
            }
            2004 => self.modes.bracketed_paste = enable,
            2026 => self.modes.synchronized = enable,
            _ => {}
        }
    }

    /// Turn one mouse mode on, or off when it is the one currently in force.
    ///
    /// The reporting modes replace one another rather than stack. A program that
    /// upgrades from `1000` to `1002` and later sends a bare `1000l` on the way out
    /// must not end up with the mouse switched off, because the mode it is turning off
    /// is not the one it is running.
    fn set_mouse(&mut self, enable: bool, mode: MouseMode) {
        if enable {
            self.modes.mouse = mode;
        } else if self.modes.mouse == mode {
            self.modes.mouse = MouseMode::None;
        }
    }

    fn enter_alt_screen(&mut self) {
        if self.alt.is_some() {
            return;
        }
        let cols = self.grid.cols();
        let rows = self.grid.rows();
        self.alt = Some(AltScreen {
            grid: std::mem::replace(&mut self.grid, Grid::new(cols, rows)),
            cursor: self.cursor,
            pen: self.pen,
            cursor_visible: self.modes.cursor_visible,
        });
        self.modes.alt_screen = true;
        self.cursor = Pos::new(0, 0);
        self.pending_wrap = false;
        self.last_print = None;
        self.invalidate_clusters();
    }

    fn exit_alt_screen(&mut self) {
        let Some(alt) = self.alt.take() else {
            return;
        };
        self.grid = alt.grid;
        self.cursor = alt.cursor;
        self.pen = alt.pen;
        self.modes.cursor_visible = alt.cursor_visible;
        self.modes.alt_screen = false;
        self.pending_wrap = false;
        self.last_print = None;
        self.invalidate_clusters();
        self.grid.damage_mut().mark_all();
    }

    fn save_cursor(&mut self) {
        self.saved = Some(SavedCursor {
            pos: self.cursor,
            pen: self.pen,
            origin: self.modes.origin,
            autowrap: self.modes.autowrap,
        });
    }

    fn restore_cursor(&mut self) {
        if let Some(saved) = self.saved {
            self.cursor = Pos::new(
                saved.pos.row.min(self.grid.rows() - 1),
                saved.pos.col.min(self.grid.cols() - 1),
            );
            self.pen = saved.pen;
            self.modes.origin = saved.origin;
            self.modes.autowrap = saved.autowrap;
            self.pending_wrap = false;
            self.last_print = None;
        }
    }

    // ---- SGR -----------------------------------------------------------------

    fn apply_sgr(&mut self, params: &Params) {
        // `CSI m` with no parameters is `CSI 0 m`. The active hyperlink is not part of
        // the pen an SGR reset clears; only `OSC 8` closes it.
        if params.is_empty() {
            self.pen = Pen {
                link: self.pen.link,
                ..Pen::default()
            };
            return;
        }

        let mut i = 0;
        while i < params.len() {
            let Some(values) = params.get(i) else { break };
            let param = values.first().copied().unwrap_or(0);
            match param {
                0 => {
                    self.pen = Pen {
                        link: self.pen.link,
                        ..Pen::default()
                    };
                }
                1 => self.pen.attrs.insert(Attrs::BOLD),
                2 => self.pen.attrs.insert(Attrs::DIM),
                3 => self.pen.attrs.insert(Attrs::ITALIC),
                4 => {
                    self.pen.attrs.clear_underline();
                    // `4:0` is "no underline"; `4:2` through `4:5` select a style.
                    match values.get(1).copied().unwrap_or(1) {
                        0 => {}
                        2 => self.pen.attrs.insert(Attrs::DOUBLE_UNDERLINE),
                        3 => self.pen.attrs.insert(Attrs::CURLY_UNDERLINE),
                        4 => self.pen.attrs.insert(Attrs::DOTTED_UNDERLINE),
                        5 => self.pen.attrs.insert(Attrs::DASHED_UNDERLINE),
                        _ => self.pen.attrs.insert(Attrs::UNDERLINE),
                    }
                }
                5 | 6 => self.pen.attrs.insert(Attrs::BLINK),
                7 => self.pen.attrs.insert(Attrs::REVERSE),
                8 => self.pen.attrs.insert(Attrs::HIDDEN),
                9 => self.pen.attrs.insert(Attrs::STRIKETHROUGH),
                21 => {
                    self.pen.attrs.clear_underline();
                    self.pen.attrs.insert(Attrs::DOUBLE_UNDERLINE);
                }
                22 => self.pen.attrs.remove(Attrs::BOLD | Attrs::DIM),
                23 => self.pen.attrs.remove(Attrs::ITALIC),
                24 => self.pen.attrs.clear_underline(),
                25 => self.pen.attrs.remove(Attrs::BLINK),
                27 => self.pen.attrs.remove(Attrs::REVERSE),
                28 => self.pen.attrs.remove(Attrs::HIDDEN),
                29 => self.pen.attrs.remove(Attrs::STRIKETHROUGH),
                30..=37 | 90..=97 => {
                    if let Some(named) = NamedColor::from_sgr_fg(param) {
                        self.pen.fg = Color::indexed(named.index());
                    }
                }
                39 => self.pen.fg = Color::DEFAULT,
                40..=47 | 100..=107 => {
                    if let Some(named) = NamedColor::from_sgr_bg(param) {
                        self.pen.bg = Color::indexed(named.index());
                    }
                }
                49 => self.pen.bg = Color::DEFAULT,
                38 | 48 | 58 => {
                    let (color, consumed) = extended_color(params, i);
                    match param {
                        38 => {
                            if let Some(c) = color {
                                self.pen.fg = c;
                            }
                        }
                        48 => {
                            if let Some(c) = color {
                                self.pen.bg = c;
                            }
                        }
                        // 58 is the underline colour. zet draws underlines in the
                        // foreground, so the sequence is consumed and ignored rather
                        // than stored somewhere the renderer would never read.
                        _ => {}
                    }
                    i += consumed - 1;
                }
                _ => {}
            }
            i += 1;
        }
    }

    // ---- responses -----------------------------------------------------------

    fn respond(&mut self, bytes: &[u8]) {
        self.responses.extend_from_slice(bytes);
    }

    fn report_cursor(&mut self) {
        // The report is one-based. A cursor holding a pending wrap sits in the last
        // column and reports that column, not one past it, which is what a program
        // reading its own position back expects to see.
        let report = format!("\x1b[{};{}R", self.cursor.row + 1, self.cursor.col + 1);
        self.respond(report.as_bytes());
    }

    fn set_hyperlink(&mut self, uri: &[u8]) {
        if uri.is_empty() {
            self.pen.link = 0;
            return;
        }
        let uri = String::from_utf8_lossy(uri).into_owned();
        // Reuse an existing entry, so a program that prints a thousand links to the
        // same target does not grow the table a thousand times.
        // The table is capped below `u16::MAX` entries, so neither of these truncates;
        // `try_from` states that rather than relying on the cap being noticed.
        if let Some(index) = self.links.iter().position(|link| *link == uri) {
            self.pen.link = u16::try_from(index + 1).unwrap_or(u16::MAX);
        } else if self.links.len() < u16::MAX as usize {
            self.links.push(uri);
            self.pen.link = u16::try_from(self.links.len()).unwrap_or(u16::MAX);
        }
    }
}

/// Read the extended colour form beginning at parameter `i`, after `38`, `48`, or `58`.
///
/// Two spellings are in the wild and both are common. The semicolon form `38;2;r;g;b`
/// puts every component in its own parameter; the colon form `38:2::r:g:b` puts them
/// all in sub-parameters of one. A program emits whichever its library happens to use,
/// so a terminal has to take both, and the difference shows up here only as how many
/// parameters the sequence consumed.
///
/// Returns the colour and how many parameters it used, which is always at least one.
fn extended_color(params: &Params, i: usize) -> (Option<Color>, usize) {
    let Some(values) = params.get(i) else {
        return (None, 1);
    };

    // One parameter carrying more than one value means the colon form.
    let colon = values.len() > 1;
    let kind = if colon {
        values[1]
    } else {
        params.value_or(i + 1, u16::MAX)
    };

    match kind {
        // `38;5;n` and `38:5:n`
        5 => {
            if colon {
                (values.get(2).map(|&n| Color::indexed(n.min(255) as u8)), 1)
            } else {
                (
                    Some(Color::indexed(params.value_or(i + 2, 0).min(255) as u8)),
                    3,
                )
            }
        }
        // `38;2;r;g;b`, `38:2:r:g:b`, and `38:2::r:g:b` with the colour-space slot.
        2 => {
            let rgb = if colon {
                // Six values means the colour-space identifier was written; five means
                // it was left off. A real identifier is never a plausible red channel,
                // so counting is the only reliable way to tell the two apart.
                let start = if values.len() >= 6 { 3 } else { 2 };
                (
                    values.get(start).copied(),
                    values.get(start + 1).copied(),
                    values.get(start + 2).copied(),
                )
            } else {
                (
                    params.get(i + 2).and_then(|v| v.first().copied()),
                    params.get(i + 3).and_then(|v| v.first().copied()),
                    params.get(i + 4).and_then(|v| v.first().copied()),
                )
            };
            let used = if colon { 1 } else { 5 };
            match rgb {
                (Some(r), Some(g), Some(b)) => (
                    Some(Color::rgb(
                        r.min(255) as u8,
                        g.min(255) as u8,
                        b.min(255) as u8,
                    )),
                    used,
                ),
                _ => (None, used),
            }
        }
        _ => (None, if colon { 1 } else { 2 }),
    }
}

impl Perform for Term {
    fn print(&mut self, ch: char) {
        self.print_char(ch);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x08 => self.backspace(),
            0x09 => self.tab(),
            // LF, VT, and FF all move down a line. CR is handled separately below.
            0x0a..=0x0c => {
                self.line_feed();
                if self.modes.newline {
                    self.carriage_return();
                }
            }
            0x0d => self.carriage_return(),
            // 0x07 is the bell; the host decides whether to make a sound. 0x0e and 0x0f
            // select the alternate character set, which UTF-8 makes moot.
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], byte: u8) {
        if !intermediates.is_empty() {
            // `ESC ( B` and friends select a character set, which UTF-8 makes moot.
            return;
        }
        match byte {
            // `ESC 7` and `ESC 8`: save and restore the cursor.
            b'7' => self.save_cursor(),
            b'8' => self.restore_cursor(),
            // `ESC D`: index, a line feed that does not return the carriage.
            b'D' => self.line_feed(),
            // `ESC M`: reverse index.
            b'M' => self.reverse_index(),
            // `ESC E`: next line.
            b'E' => {
                self.carriage_return();
                self.line_feed();
            }
            // `ESC H`: set a tab stop at the cursor.
            b'H' => {
                let col = self.cursor.col;
                self.grid.set_tab(col);
            }
            // `ESC c`: full reset.
            b'c' => self.reset(),
            // `ESC =` and `ESC >`: application and normal keypad.
            b'=' => self.modes.app_keypad = true,
            b'>' => self.modes.app_keypad = false,
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &Params,
        intermediates: &[u8],
        private: Option<Private>,
        action: char,
    ) {
        let count = |i: usize, default: u16| params.value_or(i, default).max(1) as usize;

        if let Some(marker) = private {
            match (marker, action) {
                (Private::Question, 'h') => {
                    for i in 0..params.len().max(1) {
                        self.set_mode(params.value_or(i, 0), true, true);
                    }
                }
                (Private::Question, 'l') => {
                    for i in 0..params.len().max(1) {
                        self.set_mode(params.value_or(i, 0), true, false);
                    }
                }
                // `DECDSR`: the private form of the status report.
                (Private::Question, 'n') => {
                    if params.value_or(0, 0) == 6 {
                        self.report_cursor();
                    }
                }
                // `DA2`: the secondary device attributes, which report a version.
                (Private::Greater, 'c') => self.respond(b"\x1b[>0;1;0c"),
                _ => {}
            }
            return;
        }

        if !intermediates.is_empty() {
            // `CSI ! p` is `DECSTR`, the soft terminal reset. It puts the pen and the
            // modes back but leaves the screen and the scrollback alone.
            if intermediates == b"!" && action == 'p' {
                self.pen = Pen {
                    link: self.pen.link,
                    ..Pen::default()
                };
                self.modes.insert = false;
                self.modes.newline = false;
                self.modes.origin = false;
                self.modes.autowrap = true;
                self.modes.app_cursor_keys = false;
                self.modes.app_keypad = false;
                self.modes.cursor_visible = true;
                self.modes.reverse_video = false;
                self.grid.reset_scroll_region();
                self.pending_wrap = false;
            }
            return;
        }

        match action {
            'A' => self.move_up(count(0, 1)),
            'B' | 'e' => self.move_down(count(0, 1)),
            'C' | 'a' => self.move_right(count(0, 1)),
            'D' => self.move_left(count(0, 1)),
            'E' => {
                self.move_down(count(0, 1));
                self.carriage_return();
            }
            'F' => {
                self.move_up(count(0, 1));
                self.carriage_return();
            }
            'G' | '`' => {
                self.cursor.col = (count(0, 1) - 1).min(self.grid.cols() - 1);
                self.pending_wrap = false;
                self.last_print = None;
            }
            'd' => {
                let row = count(0, 1) - 1;
                let (top, bottom) = self.grid.scroll_region();
                self.cursor.row = if self.modes.origin {
                    (top + row).min(bottom)
                } else {
                    row.min(self.grid.rows() - 1)
                };
                self.pending_wrap = false;
                self.last_print = None;
            }
            'H' | 'f' => self.set_cursor(params, true),
            'I' => {
                for _ in 0..count(0, 1) {
                    self.tab();
                }
            }
            'Z' => {
                for _ in 0..count(0, 1) {
                    self.back_tab();
                }
            }
            'J' => {
                let mode = params.value_or(0, 0);
                self.grid.erase_in_display(mode, self.cursor);
                self.invalidate_clusters();
            }
            'K' => {
                let mode = params.value_or(0, 0);
                self.grid.erase_in_line(mode, self.cursor);
                self.invalidate_clusters();
            }
            'L' => {
                let (row, n) = (self.cursor.row, count(0, 1));
                self.grid.insert_lines(row, n);
                self.invalidate_clusters();
            }
            'M' => {
                let (row, n) = (self.cursor.row, count(0, 1));
                self.grid.delete_lines(row, n);
                self.invalidate_clusters();
            }
            // `ECH`: erase characters, leaving the cursor where it is.
            'X' => {
                let (cols, start, n) = (self.grid.cols(), self.cursor.col, count(0, 1));
                let end = (start + n).min(cols);
                let bg = self.grid.blank().bg;
                self.grid
                    .row_mut(self.cursor.row)
                    .reset_range(cols, start..end, bg);
                self.invalidate_clusters();
            }
            // `ICH`
            '@' => {
                let (cols, start, n) = (self.grid.cols(), self.cursor.col, count(0, 1));
                let blank = self.grid.blank();
                self.grid
                    .row_mut(self.cursor.row)
                    .insert_cells(cols, start, n, blank);
                self.invalidate_clusters();
            }
            // `DCH`
            'P' => {
                let (cols, start, n) = (self.grid.cols(), self.cursor.col, count(0, 1));
                let blank = self.grid.blank();
                self.grid
                    .row_mut(self.cursor.row)
                    .delete_cells(cols, start, n, blank);
                self.invalidate_clusters();
            }
            // `SU` and `SD`, which scroll the region without moving the cursor.
            'S' => {
                self.grid.scroll_up(count(0, 1));
                self.invalidate_clusters();
            }
            'T' => {
                self.grid.scroll_down(count(0, 1));
                self.invalidate_clusters();
            }
            // `DECSTBM`: the scrolling region, which homes the cursor.
            'r' => {
                if params.is_empty() {
                    self.grid.reset_scroll_region();
                } else {
                    let top = count(0, 1) - 1;
                    let rows = u16::try_from(self.grid.rows()).unwrap_or(u16::MAX);
                    let bottom = params.value_or(1, rows) as usize;
                    if top < bottom && bottom <= self.grid.rows() {
                        self.grid.set_scroll_region(top, bottom - 1);
                    }
                }
                self.home_cursor();
            }
            'g' => match params.value_or(0, 0) {
                0 => {
                    let col = self.cursor.col;
                    self.grid.clear_tab(col);
                }
                3 => self.grid.clear_tabs(),
                _ => {}
            },
            'h' => {
                for i in 0..params.len().max(1) {
                    self.set_mode(params.value_or(i, 0), false, true);
                }
            }
            'l' => {
                for i in 0..params.len().max(1) {
                    self.set_mode(params.value_or(i, 0), false, false);
                }
            }
            'm' => self.apply_sgr(params),
            // `DSR`
            'n' => match params.value_or(0, 0) {
                5 => self.respond(b"\x1b[0n"),
                6 => self.report_cursor(),
                _ => {}
            },
            // `DA1`. The claims are the ones zet can keep: a VT220 with ANSI colour.
            // Reporting an option that is not implemented is how a program ends up
            // sending a sequence the terminal silently eats.
            'c' => self.respond(b"\x1b[?62;22c"),
            's' => self.save_cursor(),
            'u' => self.restore_cursor(),
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]]) {
        let Some((&first, rest)) = params.split_first() else {
            return;
        };
        match first {
            // `OSC 0` and `OSC 2` set the window title. `OSC 1` sets the icon name,
            // which nothing on Windows shows, so it is treated as the title.
            b"0" | b"1" | b"2" => {
                if let Some(title) = rest.first() {
                    self.title = String::from_utf8_lossy(title).into_owned();
                }
            }
            // `OSC 8`: hyperlinks, spelled `8;params;uri`.
            b"8" => {
                if let Some(uri) = rest.get(1) {
                    self.set_hyperlink(uri);
                }
            }
            // `OSC 10` and `OSC 11` ask for the foreground and background. Answering is
            // what lets a shell theme itself to match the terminal; a program that gets
            // no answer falls back to its own defaults, so silence is survivable.
            b"10" | b"11" if rest.first().is_some_and(|p| *p == b"?") => {
                let color = if first == b"10" {
                    self.pen.fg
                } else {
                    self.pen.bg
                };
                let reply = match color.spec() {
                    ColorSpec::Rgb(r, g, b) => format!(
                        "\x1b]{};rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}\x07",
                        if first == b"10" { "10" } else { "11" }
                    ),
                    // Indices below sixteen and the default both belong to the theme,
                    // which this layer does not know. Staying silent is correct;
                    // inventing a colour is not.
                    _ => String::new(),
                };
                if !reply.is_empty() {
                    self.respond(reply.as_bytes());
                }
            }
            _ => {}
        }
    }

    fn dcs_hook(
        &mut self,
        _params: &Params,
        intermediates: &[u8],
        _private: Option<Private>,
        action: char,
    ) {
        // `DECRQSS`: `DCS $ q Pt ST`. Answering the SGR query tells a program what the
        // pen is without it having to guess from what it last sent.
        if intermediates == b"$" && action == 'q' {
            self.respond(b"\x1bP1$r0m\x1b\\");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attrs::UnderlineStyle;
    use crate::parser::Parser;

    /// A terminal and the parser feeding it.
    ///
    /// A sequence split across two writes is still one sequence, and testing through a
    /// fresh parser each time would quietly hide every state that lives between bytes.
    struct Session {
        term: Term,
        parser: Parser,
    }

    impl core::ops::Deref for Session {
        type Target = Term;
        fn deref(&self) -> &Term {
            &self.term
        }
    }

    impl core::ops::DerefMut for Session {
        fn deref_mut(&mut self) -> &mut Term {
            &mut self.term
        }
    }

    fn open(cols: usize, rows: usize) -> Session {
        Session {
            term: Term::new(cols, rows),
            parser: Parser::new(),
        }
    }

    fn feed(session: &mut Session, bytes: &[u8]) {
        session.parser.advance_slice(bytes, &mut session.term);
    }

    fn at(term: &Term, row: usize, col: usize) -> char {
        term.grid().row(row).get(col).ch
    }

    fn line(term: &Term, row: usize) -> String {
        (0..term.grid().cols())
            .map(|col| match term.cluster_at(Pos::new(row, col)) {
                Some(cluster) => cluster.to_string(),
                None => at(term, row, col).to_string(),
            })
            .collect()
    }

    fn trimmed(term: &Term, row: usize) -> String {
        line(term, row).trim_end().to_string()
    }

    #[test]
    fn plain_text_lands_on_the_first_line() {
        let mut t = open(20, 4);
        feed(&mut t, b"hello");
        assert_eq!(trimmed(&t, 0), "hello");
        assert_eq!(t.cursor(), Pos::new(0, 5));
    }

    #[test]
    fn a_line_feed_moves_down_and_a_carriage_return_comes_back() {
        let mut t = open(20, 4);
        feed(&mut t, b"one\r\ntwo");
        assert_eq!(trimmed(&t, 0), "one");
        assert_eq!(trimmed(&t, 1), "two");
        assert_eq!(t.cursor(), Pos::new(1, 3));
    }

    #[test]
    fn a_bare_line_feed_keeps_the_column() {
        let mut t = open(20, 4);
        feed(&mut t, b"ab\ncd");
        assert_eq!(trimmed(&t, 1), "  cd");
    }

    #[test]
    fn printing_past_the_last_column_does_not_wrap_until_the_next_character() {
        let mut t = open(5, 3);
        feed(&mut t, b"abcde");
        assert_eq!(
            t.cursor(),
            Pos::new(0, 4),
            "the cursor waits in the last column"
        );
        assert_eq!(trimmed(&t, 1), "", "there is no spurious blank line yet");
        feed(&mut t, b"f");
        assert_eq!(trimmed(&t, 0), "abcde");
        assert_eq!(trimmed(&t, 1), "f", "the wrap happens now");
    }

    #[test]
    fn a_carriage_return_cancels_a_pending_wrap() {
        let mut t = open(5, 3);
        feed(&mut t, b"abcde\rX");
        assert_eq!(trimmed(&t, 0), "Xbcde");
        assert_eq!(trimmed(&t, 1), "");
    }

    #[test]
    fn a_row_the_terminal_wrapped_is_marked_as_wrapped() {
        // Nothing on the wire distinguishes a wrap the terminal performed from one the
        // program typed, so this flag is the whole record of it. Copying a line and
        // reflowing one on a resize both read it, and both are wrong without it.
        let mut t = open(5, 3);
        feed(&mut t, b"abcde");
        assert!(!t.grid().row(0).is_wrapped(), "nothing has wrapped yet");
        feed(&mut t, b"f");
        assert!(t.grid().row(0).is_wrapped(), "the row the cursor left");
        assert!(!t.grid().row(1).is_wrapped(), "the row it moved to");
    }

    #[test]
    fn a_newline_the_program_sent_is_not_a_wrap() {
        let mut t = open(5, 3);
        feed(&mut t, b"abc\r\ndef");
        assert!(!t.grid().row(0).is_wrapped());
    }

    #[test]
    fn a_wide_character_that_does_not_fit_wraps_and_marks_the_row() {
        // The other way into the wrap branch: nothing is pending, but two columns will
        // not fit in the one that is left.
        let mut t = open(5, 3);
        feed(&mut t, "abcd\u{4e2d}".as_bytes());
        assert!(t.grid().row(0).is_wrapped());
        assert_eq!(t.cursor(), Pos::new(1, 2));
    }

    #[test]
    fn a_wrapped_row_keeps_its_mark_when_the_screen_scrolls() {
        let mut t = open(5, 2);
        feed(&mut t, b"abcde");
        feed(&mut t, b"f");
        assert!(t.grid().row(0).is_wrapped());
        // Two more wraps fill what is left of a two-row screen, so the first one scrolls
        // off the top and the rest move up. The mark has to move with its row.
        feed(&mut t, b"\r\nghijkl");
        assert!(t.grid().row(0).is_wrapped(), "rode up with its row");
        assert!(
            !t.grid().row(1).is_wrapped(),
            "and did not spread to the next"
        );
    }

    #[test]
    fn autowrap_off_keeps_everything_on_one_line() {
        let mut t = open(5, 3);
        feed(&mut t, b"\x1b[?7labcdefgh");
        assert_eq!(
            trimmed(&t, 0),
            "abcdh",
            "with no wrap, characters past the margin land on the last column"
        );
        assert_eq!(trimmed(&t, 1), "");
    }

    #[test]
    fn cursor_addressing_is_one_based() {
        let mut t = open(20, 10);
        feed(&mut t, b"\x1b[3;5HX");
        assert_eq!(at(&t, 2, 4), 'X');
        assert_eq!(t.cursor(), Pos::new(2, 5));
    }

    #[test]
    fn cursor_home_with_no_parameters() {
        let mut t = open(20, 10);
        feed(&mut t, b"\x1b[5;5H\x1b[HX");
        assert_eq!(at(&t, 0, 0), 'X');
    }

    #[test]
    fn an_empty_first_parameter_means_the_first_row() {
        // `CSI ;5H` is the sequence the parser's open-parameter flag exists for: two
        // parameters, the first of them empty and therefore row 1, column 5.
        let mut t = open(20, 10);
        feed(&mut t, b"\x1b[9;9H\x1b[;5H");
        assert_eq!(t.cursor(), Pos::new(0, 4));
    }

    #[test]
    fn cursor_movement_commands() {
        let mut t = open(20, 10);
        feed(&mut t, b"\x1b[5;5H\x1b[2A");
        assert_eq!(t.cursor(), Pos::new(2, 4));
        feed(&mut t, b"\x1b[3B");
        assert_eq!(t.cursor(), Pos::new(5, 4));
        feed(&mut t, b"\x1b[2C");
        assert_eq!(t.cursor(), Pos::new(5, 6));
        feed(&mut t, b"\x1b[10D");
        assert_eq!(t.cursor(), Pos::new(5, 0), "movement clamps at the edge");
    }

    #[test]
    fn cursor_movement_clamps_at_the_screen_edges() {
        let mut t = open(10, 4);
        feed(&mut t, b"\x1b[99;99H");
        assert_eq!(t.cursor(), Pos::new(3, 9));
        feed(&mut t, b"\x1b[99A\x1b[99D");
        assert_eq!(t.cursor(), Pos::new(0, 0));
    }

    #[test]
    fn next_line_and_previous_line() {
        let mut t = open(20, 10);
        feed(&mut t, b"\x1b[5;5H\x1b[2E");
        assert_eq!(t.cursor(), Pos::new(6, 0));
        feed(&mut t, b"\x1b[3F");
        assert_eq!(t.cursor(), Pos::new(3, 0));
    }

    #[test]
    fn column_and_row_addressing() {
        let mut t = open(20, 10);
        feed(&mut t, b"\x1b[4d\x1b[7G");
        assert_eq!(t.cursor(), Pos::new(3, 6));
    }

    #[test]
    fn a_split_sequence_survives_being_fed_one_byte_at_a_time() {
        let mut t = open(20, 4);
        for byte in b"\x1b[1;31mX" {
            feed(&mut t, &[*byte]);
        }
        let cell = t.grid().row(0).get(0);
        assert_eq!(cell.ch, 'X');
        assert_eq!(cell.fg, Color::indexed(NamedColor::Red.index()));
    }

    #[test]
    fn sgr_sets_colours_and_attributes() {
        let mut t = open(20, 4);
        feed(&mut t, b"\x1b[1;31;44mX");
        let cell = t.grid().row(0).get(0);
        assert!(cell.attrs.contains(Attrs::BOLD));
        assert_eq!(cell.fg, Color::indexed(NamedColor::Red.index()));
        assert_eq!(cell.bg, Color::indexed(NamedColor::Blue.index()));
    }

    #[test]
    fn sgr_reset_clears_the_pen() {
        let mut t = open(20, 4);
        feed(&mut t, b"\x1b[1;31m\x1b[mX");
        let cell = t.grid().row(0).get(0);
        assert!(cell.attrs.is_plain());
        assert!(cell.fg.is_default());
    }

    #[test]
    fn bright_colours_map_to_the_high_half_of_the_palette() {
        let mut t = open(20, 4);
        feed(&mut t, b"\x1b[92mX");
        assert_eq!(
            t.grid().row(0).get(0).fg,
            Color::indexed(NamedColor::BrightGreen.index())
        );
    }

    #[test]
    fn truecolor_semicolon_form_spans_five_parameters() {
        let mut t = open(20, 4);
        feed(&mut t, b"\x1b[38;2;255;128;0mX");
        assert_eq!(t.grid().row(0).get(0).fg, Color::rgb(255, 128, 0));
        assert_eq!(
            t.cursor(),
            Pos::new(0, 1),
            "the character still lands at column 0"
        );
    }

    #[test]
    fn a_truecolor_background_in_the_semicolon_form() {
        let mut t = open(20, 4);
        feed(&mut t, b"\x1b[48;2;1;2;3mX");
        assert_eq!(t.grid().row(0).get(0).bg, Color::rgb(1, 2, 3));
    }

    #[test]
    fn truecolor_colon_form_with_and_without_the_colour_space_slot() {
        let mut t = open(20, 4);
        feed(&mut t, b"\x1b[38:2::10:20:30mX");
        assert_eq!(t.grid().row(0).get(0).fg, Color::rgb(10, 20, 30));

        let mut t = open(20, 4);
        feed(&mut t, b"\x1b[38:2:10:20:30mX");
        assert_eq!(t.grid().row(0).get(0).fg, Color::rgb(10, 20, 30));
    }

    #[test]
    fn sgr_after_an_extended_colour_still_applies() {
        // The consumed parameter count is what makes this work. Reading only the first
        // value of `38;2;...` would leave `2;255;128;0` to be interpreted as four more
        // SGR codes, which is nothing but garbage attributes.
        let mut t = open(20, 4);
        feed(&mut t, b"\x1b[38;2;255;128;0;1mX");
        let cell = t.grid().row(0).get(0);
        assert_eq!(cell.fg, Color::rgb(255, 128, 0));
        assert!(cell.attrs.contains(Attrs::BOLD));
    }

    #[test]
    fn indexed_colour_forms() {
        let mut t = open(20, 4);
        feed(&mut t, b"\x1b[48;5;200mX");
        assert_eq!(t.grid().row(0).get(0).bg, Color::indexed(200));

        let mut t = open(20, 4);
        feed(&mut t, b"\x1b[48:5:200mX");
        assert_eq!(t.grid().row(0).get(0).bg, Color::indexed(200));
    }

    #[test]
    fn underline_styles_round_trip() {
        let mut t = open(20, 4);
        feed(&mut t, b"\x1b[4:3mX");
        assert_eq!(
            t.grid().row(0).get(0).attrs.underline_style(),
            UnderlineStyle::Curly
        );
        feed(&mut t, b"\x1b[24mY");
        assert_eq!(
            t.grid().row(0).get(1).attrs.underline_style(),
            UnderlineStyle::None
        );
    }

    #[test]
    fn a_plain_underline_is_the_default_style() {
        let mut t = open(20, 4);
        feed(&mut t, b"\x1b[4mX");
        assert_eq!(
            t.grid().row(0).get(0).attrs.underline_style(),
            UnderlineStyle::Single
        );
    }

    #[test]
    fn a_colour_set_after_a_reset_attribute_still_applies() {
        // The parameter list is walked in order, so `1;0;31` must leave red set.
        let mut t = open(20, 4);
        feed(&mut t, b"\x1b[1;0;31mX");
        let cell = t.grid().row(0).get(0);
        assert!(!cell.attrs.contains(Attrs::BOLD));
        assert_eq!(cell.fg, Color::indexed(NamedColor::Red.index()));
    }

    #[test]
    fn erase_in_line_from_the_cursor() {
        let mut t = open(6, 2);
        feed(&mut t, b"abcdef\x1b[1;3H\x1b[K");
        assert_eq!(trimmed(&t, 0), "ab");
    }

    #[test]
    fn erase_in_line_to_the_cursor_keeps_what_follows() {
        let mut t = open(6, 2);
        feed(&mut t, b"abcdef\x1b[1;3H\x1b[1K");
        assert_eq!(trimmed(&t, 0), "   def");
    }

    #[test]
    fn insert_and_delete_characters() {
        let mut t = open(6, 2);
        // Six columns hold "abcde" plus one blank, so inserting two pushes `d` and `e`
        // right and drops what no longer fits.
        feed(&mut t, b"abcde\x1b[1;2H\x1b[2@");
        assert_eq!(trimmed(&t, 0), "a  bcd");

        let mut t = open(6, 2);
        feed(&mut t, b"abcde\x1b[1;2H\x1b[2P");
        assert_eq!(trimmed(&t, 0), "ade");
    }

    #[test]
    fn erase_characters_leaves_the_cursor_alone() {
        let mut t = open(6, 2);
        feed(&mut t, b"abcdef\x1b[1;2H\x1b[2X");
        assert_eq!(trimmed(&t, 0), "a  def");
        assert_eq!(t.cursor(), Pos::new(0, 1));
    }

    #[test]
    fn delete_lines_pulls_the_rest_of_the_screen_up() {
        let mut t = open(6, 4);
        feed(&mut t, b"a\r\nb\r\nc\r\nd\x1b[2;1H\x1b[M");
        assert_eq!(trimmed(&t, 0), "a");
        assert_eq!(trimmed(&t, 1), "c");
        assert_eq!(trimmed(&t, 2), "d");
        assert_eq!(trimmed(&t, 3), "");
    }

    #[test]
    fn insert_lines_pushes_the_rest_of_the_screen_down() {
        let mut t = open(6, 4);
        feed(&mut t, b"a\r\nb\r\nc\r\nd\x1b[2;1H\x1b[L");
        assert_eq!(trimmed(&t, 0), "a");
        assert_eq!(trimmed(&t, 1), "");
        assert_eq!(trimmed(&t, 2), "b");
        assert_eq!(trimmed(&t, 3), "c");
    }

    #[test]
    fn a_scrolling_region_keeps_lines_outside_it_pinned() {
        let mut t = open(6, 5);
        feed(&mut t, b"\x1b[1;1Htop");
        feed(&mut t, b"\x1b[5;1Hbot");
        // Region rows 2..4, then print enough to scroll inside it.
        feed(&mut t, b"\x1b[2;4r\x1b[2;1Ha\r\nb\r\nc\r\n");
        assert_eq!(trimmed(&t, 0), "top", "row 1 is outside the region");
        assert_eq!(trimmed(&t, 4), "bot", "row 5 is outside the region");
    }

    #[test]
    fn setting_the_scroll_region_homes_the_cursor() {
        let mut t = open(20, 10);
        feed(&mut t, b"\x1b[5;5H\x1b[2;8r");
        assert_eq!(t.cursor(), Pos::new(0, 0));
    }

    #[test]
    fn a_scrolling_region_with_no_parameters_resets_to_the_whole_screen() {
        let mut t = open(20, 10);
        feed(&mut t, b"\x1b[2;8r\x1b[r");
        assert_eq!(t.grid().scroll_region(), (0, 9));
    }

    #[test]
    fn origin_mode_makes_addressing_relative_to_the_region() {
        let mut t = open(20, 10);
        feed(&mut t, b"\x1b[3;8r\x1b[?6h\x1b[1;1HX");
        assert_eq!(at(&t, 2, 0), 'X', "row 1 of the region is screen row 3");
        feed(&mut t, b"\x1b[?6l");
        assert_eq!(t.cursor(), Pos::new(0, 0));
    }

    #[test]
    fn scrolling_produces_scrollback() {
        let mut t = open(6, 3);
        feed(&mut t, b"a\r\nb\r\nc\r\nd\r\ne");
        assert!(t.grid().scrollback_len() >= 2);
    }

    #[test]
    fn the_alternate_screen_preserves_the_primary_one() {
        let mut t = open(10, 4);
        feed(&mut t, b"primary\x1b[?1049h");
        assert_eq!(trimmed(&t, 0), "", "the alternate screen starts blank");
        feed(&mut t, b"alt");
        assert_eq!(trimmed(&t, 0), "alt");
        feed(&mut t, b"\x1b[?1049l");
        assert_eq!(trimmed(&t, 0), "primary");
        assert_eq!(t.cursor(), Pos::new(0, 7), "the cursor comes back too");
        assert!(!t.modes().alt_screen);
    }

    #[test]
    fn the_alternate_screen_keeps_itself_out_of_the_primary_scrollback() {
        let mut t = open(10, 3);
        feed(&mut t, b"\x1b[?1049h");
        for i in 0..20 {
            feed(&mut t, format!("alt {i}\r\n").as_bytes());
        }
        feed(&mut t, b"\x1b[?1049l");
        assert_eq!(
            t.grid().scrollback_len(),
            0,
            "the primary history must be untouched"
        );
    }

    #[test]
    fn the_alternate_screen_survives_resize() {
        let mut t = open(10, 4);
        feed(&mut t, b"\x1b[?1049halt");
        t.resize(20, 8);
        assert_eq!(trimmed(&t, 0), "alt");
        feed(&mut t, b"\x1b[?1049l");
        assert_eq!(t.grid().cols(), 20);
        assert_eq!(t.grid().rows(), 8);
    }

    #[test]
    fn entering_the_alternate_screen_twice_does_not_stack() {
        let mut t = open(10, 4);
        feed(&mut t, b"primary\x1b[?1049h\x1b[?1049halt\x1b[?1049l");
        assert_eq!(trimmed(&t, 0), "primary");
    }

    #[test]
    fn a_cursor_hidden_by_a_full_screen_program_comes_back_on_the_way_out() {
        let mut t = open(10, 4);
        feed(&mut t, b"\x1b[?1049h\x1b[?25l");
        assert!(!t.modes().cursor_visible);
        feed(&mut t, b"\x1b[?1049l");
        assert!(t.modes().cursor_visible, "the shell's cursor comes back");
    }

    #[test]
    fn save_and_restore_the_cursor_including_the_pen() {
        let mut t = open(20, 6);
        feed(&mut t, b"\x1b[3;5H\x1b[31m\x1b7");
        feed(&mut t, b"\x1b[1;1H\x1b[0m");
        feed(&mut t, b"\x1b8X");
        assert_eq!(t.cursor(), Pos::new(2, 5));
        assert_eq!(
            t.grid().row(2).get(4).fg,
            Color::indexed(NamedColor::Red.index()),
            "the pen comes back with the cursor"
        );
    }

    #[test]
    fn save_and_restore_also_work_as_csi_s_and_csi_u() {
        let mut t = open(20, 6);
        feed(&mut t, b"\x1b[2;3H\x1b[s\x1b[1;1H\x1b[uX");
        assert_eq!(t.cursor(), Pos::new(1, 3));
    }

    #[test]
    fn restoring_with_nothing_saved_is_harmless() {
        let mut t = open(20, 6);
        feed(&mut t, b"\x1b[3;3H\x1b8X");
        assert_eq!(at(&t, 2, 2), 'X', "the cursor stays where it was");
    }

    #[test]
    fn cursor_visibility_is_a_mode_not_a_drawing_decision() {
        let mut t = open(10, 4);
        assert!(t.modes().cursor_visible);
        feed(&mut t, b"\x1b[?25l");
        assert!(!t.modes().cursor_visible);
        feed(&mut t, b"\x1b[?25h");
        assert!(t.modes().cursor_visible);
    }

    #[test]
    fn bracketed_paste_and_focus_events_are_tracked() {
        let mut t = open(10, 4);
        feed(&mut t, b"\x1b[?2004h\x1b[?1004h");
        assert!(t.modes().bracketed_paste);
        assert!(t.modes().focus_events);
        feed(&mut t, b"\x1b[?2004l\x1b[?1004l");
        assert!(!t.modes().bracketed_paste);
        assert!(!t.modes().focus_events);
    }

    #[test]
    fn mouse_modes_and_encodings_are_tracked_independently() {
        let mut t = open(10, 4);
        feed(&mut t, b"\x1b[?1002h\x1b[?1006h");
        assert_eq!(t.modes().mouse, MouseMode::Drag);
        assert_eq!(t.modes().mouse_encoding, MouseEncoding::Sgr);
        feed(&mut t, b"\x1b[?1003h");
        assert_eq!(t.modes().mouse, MouseMode::Motion);
        feed(&mut t, b"\x1b[?1002l");
        assert_eq!(
            t.modes().mouse,
            MouseMode::Motion,
            "turning off a mode the program has already replaced must not disable the mouse"
        );
        feed(&mut t, b"\x1b[?1003l");
        assert_eq!(t.modes().mouse, MouseMode::None);
        assert_eq!(
            t.modes().mouse_encoding,
            MouseEncoding::Sgr,
            "the encoding is a separate mode"
        );
    }

    #[test]
    fn synchronized_output_is_reported_to_the_host() {
        let mut t = open(10, 4);
        assert!(!t.is_synchronized());
        feed(&mut t, b"\x1b[?2026h");
        assert!(t.is_synchronized(), "the host must hold the frame");
        feed(&mut t, b"\x1b[?2026l");
        assert!(!t.is_synchronized());
    }

    #[test]
    fn insert_mode_shifts_existing_text() {
        let mut t = open(8, 2);
        feed(&mut t, b"abcde\x1b[1;1H\x1b[4hXY");
        assert_eq!(trimmed(&t, 0), "XYabcde");
    }

    #[test]
    fn a_wide_character_typed_in_insert_mode_shifts_by_two() {
        let mut t = open(8, 2);
        feed(&mut t, b"abcdef\x1b[1;1H\x1b[4h");
        feed(&mut t, "中".as_bytes());
        let row = t.grid().row(0);
        assert_eq!(row.get(0).ch, '中');
        assert!(
            row.get(1).is_wide_spacer(),
            "the second half is a spacer cell"
        );
        assert_eq!(at(&t, 0, 2), 'a');
        assert_eq!(at(&t, 0, 7), 'f', "the row is exactly full");
        assert_eq!(
            t.cursor(),
            Pos::new(0, 2),
            "a wide character takes two columns"
        );
    }

    #[test]
    fn newline_mode_makes_a_line_feed_return_the_carriage() {
        let mut t = open(10, 3);
        feed(&mut t, b"ab\x1b[20h\ncd");
        assert_eq!(trimmed(&t, 0), "ab");
        assert_eq!(trimmed(&t, 1), "cd", "the carriage should have returned");
    }

    #[test]
    fn tab_stops_default_to_every_eight_columns_and_can_be_cleared() {
        let mut t = open(24, 2);
        feed(&mut t, b"a\tb");
        assert_eq!(at(&t, 0, 8), 'b');
        feed(&mut t, b"\x1b[3g");
        assert!(t.grid().tabs().iter().all(|&set| !set));
        feed(&mut t, b"\x1b[1;1H\x1b[2I");
        assert_eq!(
            t.cursor(),
            Pos::new(0, 23),
            "with no stops left, the tab runs to the edge"
        );
    }

    #[test]
    fn a_tab_stop_can_be_set_and_cleared_at_the_cursor() {
        let mut t = open(24, 2);
        feed(&mut t, b"\x1b[1;4H\x1bH\x1b[1;1H\t");
        assert_eq!(t.cursor(), Pos::new(0, 3));
        feed(&mut t, b"\x1b[g\x1b[1;1H\t");
        assert_eq!(
            t.cursor(),
            Pos::new(0, 8),
            "the stop is gone, so the next one wins"
        );
    }

    #[test]
    fn back_tab_steps_to_the_previous_stop() {
        let mut t = open(24, 2);
        feed(&mut t, b"\x1b[1;20H\x1b[Z");
        assert_eq!(t.cursor(), Pos::new(0, 16));
    }

    #[test]
    fn a_combining_mark_attaches_to_the_character_before_it() {
        let mut t = open(10, 2);
        feed(&mut t, "e\u{0301}".as_bytes());
        assert_eq!(
            t.cursor(),
            Pos::new(0, 1),
            "a combining mark takes no column"
        );
        assert_eq!(
            t.cluster_at(Pos::new(0, 0)),
            Some("e\u{0301}"),
            "the cluster must survive for the renderer"
        );
    }

    #[test]
    fn several_combining_marks_accumulate_on_one_cell() {
        let mut t = open(10, 2);
        feed(&mut t, "a\u{0301}\u{0327}".as_bytes());
        assert_eq!(t.cluster_at(Pos::new(0, 0)), Some("a\u{0301}\u{0327}"));
        assert_eq!(t.cursor(), Pos::new(0, 1));
    }

    #[test]
    fn a_combining_mark_with_nothing_before_it_is_dropped() {
        let mut t = open(10, 2);
        feed(&mut t, "\u{0301}".as_bytes());
        assert_eq!(t.cursor(), Pos::new(0, 0));
        assert_eq!(t.cluster_at(Pos::new(0, 0)), None);
    }

    #[test]
    fn a_combining_mark_after_a_line_feed_does_not_find_a_stale_cell() {
        let mut t = open(10, 3);
        feed(&mut t, b"ab\r\n");
        feed(&mut t, "\u{0301}".as_bytes());
        assert_eq!(
            t.cluster_at(Pos::new(1, 0)),
            None,
            "a newline ends the character a mark could attach to"
        );
    }

    #[test]
    fn a_wide_character_advances_two_columns() {
        let mut t = open(10, 2);
        feed(&mut t, "中a".as_bytes());
        assert_eq!(t.cursor(), Pos::new(0, 3));
        assert_eq!(at(&t, 0, 0), '中');
        assert_eq!(at(&t, 0, 2), 'a');
    }

    #[test]
    fn a_wide_character_at_the_last_column_waits_for_the_next_line() {
        let mut t = open(5, 2);
        feed(&mut t, "abcd".as_bytes());
        feed(&mut t, "中".as_bytes());
        assert_eq!(at(&t, 0, 4), ' ');
        assert_eq!(
            at(&t, 1, 0),
            '中',
            "the whole character moves to the next line"
        );
    }

    #[test]
    fn erasing_drops_the_combining_marks_with_the_cells_they_sat_on() {
        let mut t = open(10, 2);
        feed(&mut t, "e\u{0301}".as_bytes());
        feed(&mut t, b"\x1b[2K");
        assert_eq!(t.cluster_at(Pos::new(0, 0)), None);
    }

    #[test]
    fn scrolling_drops_the_combining_marks_because_their_positions_moved() {
        let mut t = open(10, 2);
        feed(&mut t, b"\x1b[2;1H");
        feed(&mut t, "e\u{0301}".as_bytes());
        feed(&mut t, b"\n\n");
        assert_eq!(t.cluster_at(Pos::new(1, 0)), None);
    }

    #[test]
    fn device_status_reports() {
        let mut t = open(10, 4);
        feed(&mut t, b"\x1b[3;7H\x1b[6n");
        assert_eq!(t.take_responses(), b"\x1b[3;7R");
        feed(&mut t, b"\x1b[5n");
        assert_eq!(t.take_responses(), b"\x1b[0n");
    }

    #[test]
    fn the_private_cursor_report_answers_too() {
        let mut t = open(10, 4);
        feed(&mut t, b"\x1b[2;2H\x1b[?6n");
        assert_eq!(t.take_responses(), b"\x1b[2;2R");
    }

    #[test]
    fn a_cursor_report_at_the_right_edge_reports_the_last_column() {
        // The cursor is physically in column 5 holding a pending wrap, and that is what
        // a program reading its position back has to be told. Reporting 6 would put the
        // cursor outside a screen that is only five columns wide.
        let mut t = open(5, 2);
        feed(&mut t, b"abcde\x1b[6n");
        assert_eq!(t.take_responses(), b"\x1b[1;5R");
    }

    #[test]
    fn device_attributes_requests_get_answers() {
        let mut t = open(10, 4);
        feed(&mut t, b"\x1b[c");
        let response = t.take_responses();
        assert!(
            response.starts_with(b"\x1b[?"),
            "got {:?}",
            String::from_utf8_lossy(&response)
        );
        assert!(response.ends_with(b"c"));

        feed(&mut t, b"\x1b[>c");
        assert_eq!(t.take_responses(), b"\x1b[>0;1;0c");
    }

    #[test]
    fn responses_are_drained_not_repeated() {
        let mut t = open(10, 4);
        feed(&mut t, b"\x1b[5n");
        assert!(!t.take_responses().is_empty());
        assert!(t.take_responses().is_empty());
    }

    #[test]
    fn the_title_is_captured() {
        let mut t = open(10, 4);
        feed(&mut t, b"\x1b]0;my title\x07");
        assert_eq!(t.title(), "my title");
        assert_eq!(trimmed(&t, 0), "", "an OSC must not print anything");
    }

    #[test]
    fn the_title_can_be_set_with_st_termination_too() {
        let mut t = open(10, 4);
        feed(&mut t, b"\x1b]2;other\x1b\\");
        assert_eq!(t.title(), "other");
    }

    #[test]
    fn hyperlinks_attach_to_cells_and_are_deduplicated() {
        let mut t = open(40, 2);
        let link = b"\x1b]8;;https://example.com\x07";
        feed(&mut t, link);
        feed(&mut t, b"one ");
        feed(&mut t, link);
        feed(&mut t, b"two");
        feed(&mut t, b"\x1b]8;;\x07");

        let first = t.grid().row(0).get(0);
        let second = t.grid().row(0).get(4);
        assert_eq!(t.link_for(&first), Some("https://example.com"));
        assert_eq!(
            first.link, second.link,
            "the same target must reuse one table entry"
        );

        feed(&mut t, b"plain");
        assert_eq!(t.link_for(&t.grid().row(0).get(9)), None);
    }

    #[test]
    fn a_hyperlink_survives_a_colour_reset() {
        let mut t = open(40, 2);
        feed(&mut t, b"\x1b]8;;https://example.com\x07\x1b[31mx\x1b[0my");
        assert_eq!(
            t.link_for(&t.grid().row(0).get(1)),
            Some("https://example.com"),
            "SGR 0 resets the pen, not the hyperlink"
        );
    }

    #[test]
    fn a_background_colour_query_is_answered() {
        let mut t = open(10, 4);
        feed(&mut t, b"\x1b[48;2;16;32;48m\x1b]11;?\x07");
        assert_eq!(t.take_responses(), b"\x1b]11;rgb:1010/2020/3030\x07");
    }

    #[test]
    fn a_query_for_a_default_colour_stays_silent() {
        let mut t = open(10, 4);
        feed(&mut t, b"\x1b]10;?\x07");
        assert!(
            t.take_responses().is_empty(),
            "the theme owns the default colours, so this layer has no answer"
        );
    }

    #[test]
    fn decrqss_gets_an_answer_so_a_program_stops_guessing() {
        let mut t = open(10, 4);
        feed(&mut t, b"\x1bP$qm\x1b\\");
        assert_eq!(t.take_responses(), b"\x1bP1$r0m\x1b\\");
    }

    #[test]
    fn a_soft_reset_clears_the_pen_but_keeps_the_screen() {
        let mut t = open(10, 4);
        feed(&mut t, b"\x1b[1;31mtext\x1b[!p");
        assert_eq!(trimmed(&t, 0), "text");
        assert!(t.pen().fg.is_default());
        assert!(!t.pen().attrs.contains(Attrs::BOLD));
    }

    #[test]
    fn a_soft_reset_restores_the_modes_without_touching_the_screen() {
        let mut t = open(10, 4);
        feed(&mut t, b"\x1b[2;8r\x1b[?6h\x1b[?7l\x1b[!p");
        assert!(!t.modes().origin);
        assert!(t.modes().autowrap);
        assert_eq!(t.grid().scroll_region(), (0, 3));
    }

    #[test]
    fn a_hard_reset_clears_everything() {
        let mut t = open(10, 4);
        feed(&mut t, b"text\x1b[?1049h\x1b[1;31m\x1bc");
        assert_eq!(trimmed(&t, 0), "");
        assert!(t.pen().fg.is_default());
        assert!(!t.modes().alt_screen);
    }

    #[test]
    fn a_hard_reset_keeps_the_scrollback_limit_the_host_chose() {
        let mut t = open(10, 4);
        t.grid_mut().set_scrollback_limit(3);
        feed(&mut t, b"\x1bc");
        assert_eq!(t.grid().scrollback_limit(), 3);
    }

    #[test]
    fn reverse_index_at_the_top_scrolls_down() {
        let mut t = open(6, 3);
        feed(&mut t, b"a\r\nb\r\nc\x1b[1;1H\x1bM");
        assert_eq!(trimmed(&t, 0), "");
        assert_eq!(trimmed(&t, 1), "a");
    }

    #[test]
    fn reverse_video_is_a_mode() {
        let mut t = open(10, 4);
        feed(&mut t, b"\x1b[?5h");
        assert!(t.modes().reverse_video);
        feed(&mut t, b"\x1b[?5l");
        assert!(!t.modes().reverse_video);
    }

    #[test]
    fn application_cursor_keys_and_keypad_are_modes() {
        let mut t = open(10, 4);
        feed(&mut t, b"\x1b[?1h\x1b=");
        assert!(t.modes().app_cursor_keys);
        assert!(t.modes().app_keypad);
        feed(&mut t, b"\x1b[?1l\x1b>");
        assert!(!t.modes().app_cursor_keys);
        assert!(!t.modes().app_keypad);
    }

    #[test]
    fn erase_does_not_disturb_the_scrollback() {
        let mut t = open(6, 2);
        feed(&mut t, b"a\r\nb\r\nc");
        let before = t.grid().scrollback_len();
        feed(&mut t, b"\x1b[2J");
        assert_eq!(t.grid().scrollback_len(), before);
    }

    #[test]
    fn a_full_screen_of_output_scrolls_and_keeps_history() {
        let mut t = open(10, 4);
        for i in 0..20 {
            feed(&mut t, format!("line {i}\r\n").as_bytes());
        }
        assert!(
            t.grid().scrollback_len() >= 16,
            "got {}",
            t.grid().scrollback_len()
        );
    }

    #[test]
    fn the_screen_survives_a_resize_without_losing_the_cursor_line() {
        let mut t = open(20, 6);
        for i in 0..6 {
            feed(&mut t, format!("row{i}\r\n").as_bytes());
        }
        t.resize(40, 12);
        let all: String = (0..t.grid().rows())
            .map(|row| line(&t, row))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            all.contains("row5"),
            "the last line printed should still be visible"
        );
        assert!(t.cursor().row < t.grid().rows());
        assert!(t.cursor().col < t.grid().cols());
    }

    #[test]
    fn feeding_random_control_bytes_never_panics() {
        let mut t = open(20, 6);
        for round in 0..4u8 {
            for byte in 0..=u8::MAX {
                feed(&mut t, &[byte.wrapping_add(round)]);
            }
        }
        assert!(t.cursor().row < t.grid().rows());
        assert!(t.cursor().col < t.grid().cols());
    }

    #[test]
    fn feeding_a_realistic_session_leaves_a_sane_screen() {
        let mut t = open(80, 24);
        feed(
            &mut t,
            b"\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H\x1b[1;32m$\x1b[0m cargo build\r\n",
        );
        feed(
            &mut t,
            b"   Compiling zet-vt v0.1.0\r\n    Finished in 0.34s\r\n",
        );
        feed(&mut t, b"\x1b[?25h\x1b[?1049l");
        assert!(!t.modes().alt_screen);
        assert!(t.modes().cursor_visible);
        assert_eq!(t.grid().cols(), 80);
    }
}
