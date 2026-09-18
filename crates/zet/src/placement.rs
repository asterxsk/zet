//! Where the window was the last time it was open.
//!
//! `window.remember-position` is on by default, and this is what keeps it: a window that
//! opens where you left it is the difference between a terminal you launch and a terminal
//! you arrange every morning.
//!
//! # Why this is not in `config.toml`
//!
//! The configuration file is the user's. It is read, edited by hand, written through by
//! the settings panel with its comments intact, and — the part that settles it — a file
//! that fails to parse produces defaults and diagnostics rather than an error, so a zet
//! that wrote a window position into it on the way out would be a zet that replaced a
//! file it had just failed to understand. A position is not a setting either: it is a
//! fact about this machine, and a roaming profile that carried it would carry a
//! coordinate on somebody else's monitor layout.
//!
//! So it is its own file, under the per-machine directory rather than the roaming one,
//! and it holds one line: two integers, which is what a window position is.
//!
//! # Why the file is text
//!
//! Because it is a file on a user's disk that they may open. `-1920 340` is readable,
//! fixable, and inspectable, where four little-endian bytes are none of those things,
//! and the only thing binary would save is a `parse` this module has to have anyway.
//!
//! # What a saved position is checked against
//!
//! A monitor that was unplugged yesterday takes its coordinates with it. A window
//! restored to a point on no screen is a window that is running and invisible, which is
//! the worst thing a terminal can do to someone, so a position is only used when it is
//! on a screen that is there now — and only written when it is on one, which is also
//! what keeps a minimized window's own idea of where it is out of the file.

use std::path::{Path, PathBuf};

/// Where the remembered position lives.
///
/// `%LOCALAPPDATA%\zet\window.txt`. `None` when `LOCALAPPDATA` is not set, which is a
/// machine with nowhere to remember anything; the caller does without rather than
/// failing, because a window that opens in the middle of the screen is a working window
/// and a terminal that refused to start over a missing environment variable is not.
#[must_use]
pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)?;
    Some(base.join("zet").join("window.txt"))
}

/// A window's outer corner, in physical pixels.
///
/// Physical and not logical, because that is what the platform reports and what it takes
/// back: a position converted to logical units on one monitor and back on another is a
/// window that drifts a little every time it is moved between displays with different
/// scaling.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Position {
    /// From the left edge of the primary display.
    pub x: i32,
    /// From its top edge. Negative on a display above the primary one.
    pub y: i32,
}

impl Position {
    /// A position from a platform's pair of integers.
    #[must_use]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// The line this is written as.
    ///
    /// One line, two integers, one space, and a newline: a file that ends without one is
    /// a file that some editor will add one to, and a file whose text changed for a
    /// reason that is not the window is a diff nobody can read.
    #[must_use]
    pub fn to_text(self) -> String {
        format!("{} {}\n", self.x, self.y)
    }

    /// The position a file's text holds, or `None` for anything else.
    ///
    /// Forgiving on the way in and exact on the way out: trailing whitespace, a missing
    /// newline, and a stray carriage return are all what a file that has been through a
    /// Windows editor looks like, and none of them change which position it names. What
    /// it will not do is guess — one number is not a position, three is not one, and a
    /// word is not a number.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split_whitespace();
        let x = parts.next()?.parse().ok()?;
        let y = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self { x, y })
    }
}

/// A display, as the window system describes it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Screen {
    /// Its outer corner, in the same coordinates as [`Position`].
    pub at: Position,
    /// Its size in physical pixels.
    pub size: (i32, i32),
}

impl Screen {
    /// Whether a point is on this screen.
    ///
    /// The half-open interval every rectangle test in this repository uses: the left edge
    /// is inside and the right edge is the first pixel of what is not.
    #[must_use]
    pub fn holds(self, point: Position) -> bool {
        point.x >= self.at.x
            && point.x < self.at.x.saturating_add(self.size.0)
            && point.y >= self.at.y
            && point.y < self.at.y.saturating_add(self.size.1)
    }
}

/// The position, if it is on one of these screens.
///
/// The corner and not the whole window: the corner is what the window manager is given,
/// what the user drags the window by, and the part that has to be reachable for the
/// window to be usable. A window restored so that most of it hangs off the right edge is
/// still a window with a titlebar on screen, which is what "where it was" means.
#[must_use]
pub fn on_a_screen(position: Position, screens: &[Screen]) -> Option<Position> {
    screens
        .iter()
        .any(|screen| screen.holds(position))
        .then_some(position)
}

/// The remembered position, or `None` when there is not one to use.
///
/// Every failure is the same answer: no file, an unreadable file, a file holding
/// something that is not a position. There is nothing a caller could do differently about
/// any of them, and a terminal that complained about a corrupt position file would be
/// complaining about a file the user never asked it to write.
#[must_use]
pub fn load(path: &Path) -> Option<Position> {
    Position::parse(&std::fs::read_to_string(path).ok()?)
}

/// Write the position down, for the next time the window opens.
///
/// Failing is not an error. A machine where `%LOCALAPPDATA%` cannot be written is a
/// machine where the window opens in the middle of the screen tomorrow, which is where it
/// would have opened anyway.
pub fn save(path: &Path, position: Position) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, position.to_text());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The screens most of these tests are written against: a primary display and one
    /// to its left, which is the arrangement that makes a negative coordinate ordinary
    /// rather than an error.
    const SCREENS: [Screen; 2] = [
        Screen {
            at: Position::new(0, 0),
            size: (2560, 1440),
        },
        Screen {
            at: Position::new(-1920, -200),
            size: (1920, 1080),
        },
    ];

    #[test]
    fn a_position_survives_being_written_and_read() {
        let position = Position::new(-1920, 340);
        assert_eq!(Position::parse(&position.to_text()), Some(position));
        assert_eq!(position.to_text(), "-1920 340\n");
    }

    #[test]
    fn the_edges_of_a_position_file_are_forgiven_and_nothing_else_is() {
        let wanted = Some(Position::new(8, -7));
        assert_eq!(Position::parse("8 -7"), wanted);
        assert_eq!(Position::parse("  8   -7  \r\n"), wanted);
        assert_eq!(Position::parse("8\n-7\n"), wanted);
        // What is not a position: a number short, a number too many, and a word.
        assert_eq!(Position::parse("8"), None);
        assert_eq!(Position::parse(""), None);
        assert_eq!(Position::parse("8 -7 3"), None);
        assert_eq!(Position::parse("eight -7"), None);
        assert_eq!(Position::parse("8 -7.5"), None);
    }

    #[test]
    fn a_screen_holds_its_own_corner_and_not_the_pixel_past_its_edge() {
        let screen = SCREENS[0];
        assert!(screen.holds(Position::new(0, 0)));
        assert!(screen.holds(Position::new(2559, 1439)));
        assert!(!screen.holds(Position::new(2560, 0)));
        assert!(!screen.holds(Position::new(0, 1440)));
        assert!(!screen.holds(Position::new(-1, 0)));
    }

    #[test]
    fn a_position_on_a_display_that_is_gone_is_not_used() {
        // Where the left-hand display used to be, with nothing there now.
        let unplugged = Position::new(-1920, 200);
        assert_eq!(on_a_screen(unplugged, &SCREENS[..1]), None);
        // And with the display back, the same coordinate is used as it was.
        assert_eq!(on_a_screen(unplugged, &SCREENS), Some(unplugged));
    }

    #[test]
    fn a_window_above_the_primary_display_keeps_its_negative_corner() {
        let above = Position::new(-1900, -200);
        assert_eq!(on_a_screen(above, &SCREENS), Some(above));
        assert_eq!(on_a_screen(above, &SCREENS[..1]), None);
    }

    #[test]
    fn a_minimized_windows_corner_is_not_a_screen_and_is_not_kept() {
        // What Windows reports for a window it has tucked away, at a coordinate no
        // display has ever been at. It is refused by the same rule that refuses a
        // monitor that has been unplugged, which is why there is no second rule for it.
        assert_eq!(on_a_screen(Position::new(-32000, -32000), &SCREENS), None);
    }

    #[test]
    fn a_file_that_is_not_there_is_not_a_position() {
        let path = Path::new("no-such-directory/window.txt");
        assert_eq!(load(path), None);
    }
}
