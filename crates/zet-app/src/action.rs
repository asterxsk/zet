//! The things a key can be bound to.
//!
//! The names are the ones in the config file's `[keys]` table, so this enum and
//! [`zet_config::ACTIONS`] are two spellings of one list. They are kept in step by a
//! test rather than by a comment, because the failure mode is a user binding a key to an
//! action the app has never heard of and getting nothing with no error.

/// Something a key can be bound to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Open a tab with the default profile.
    NewTab,
    /// Close the active tab.
    CloseTab,
    /// Move to the next tab, wrapping.
    NextTab,
    /// Move to the previous tab, wrapping.
    PreviousTab,
    /// Open a second window.
    NewWindow,
    /// Put the selection on the clipboard.
    Copy,
    /// Take the clipboard and send it to the active session.
    Paste,
    /// Open the find bar over the grid.
    Find,
    /// Open the settings panel.
    Settings,
    /// Move the tab strip between the titlebar and the left rail.
    ToggleTabPosition,
    /// Scroll up by a screen.
    ScrollPageUp,
    /// Scroll down by a screen.
    ScrollPageDown,
    /// Scroll to the oldest line still kept.
    ScrollToTop,
    /// Return to the live screen.
    ScrollToBottom,
    /// Grow the terminal font.
    FontLarger,
    /// Shrink the terminal font.
    FontSmaller,
    /// Return the terminal font to the configured size.
    FontReset,
    /// Close the window.
    Quit,
}

impl Action {
    /// Every action, in the order the settings panel lists them.
    pub const ALL: [Action; 18] = [
        Action::NewTab,
        Action::CloseTab,
        Action::NextTab,
        Action::PreviousTab,
        Action::NewWindow,
        Action::Copy,
        Action::Paste,
        Action::Find,
        Action::Settings,
        Action::ToggleTabPosition,
        Action::ScrollPageUp,
        Action::ScrollPageDown,
        Action::ScrollToTop,
        Action::ScrollToBottom,
        Action::FontLarger,
        Action::FontSmaller,
        Action::FontReset,
        Action::Quit,
    ];

    /// The name written in the config file.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Action::NewTab => "new-tab",
            Action::CloseTab => "close-tab",
            Action::NextTab => "next-tab",
            Action::PreviousTab => "previous-tab",
            Action::NewWindow => "new-window",
            Action::Copy => "copy",
            Action::Paste => "paste",
            Action::Find => "find",
            Action::Settings => "settings",
            Action::ToggleTabPosition => "toggle-tab-position",
            Action::ScrollPageUp => "scroll-page-up",
            Action::ScrollPageDown => "scroll-page-down",
            Action::ScrollToTop => "scroll-to-top",
            Action::ScrollToBottom => "scroll-to-bottom",
            Action::FontLarger => "font-larger",
            Action::FontSmaller => "font-smaller",
            Action::FontReset => "font-reset",
            Action::Quit => "quit",
        }
    }

    /// The action a config file named, or `None` for a name nothing knows.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.name() == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_round_trips() {
        for action in Action::ALL {
            assert_eq!(Action::from_name(action.name()), Some(action));
        }
    }

    #[test]
    fn the_names_are_the_ones_the_config_file_validates_against() {
        // Two lists, one meaning. If they drift, a user can bind a key to an action that
        // is accepted by the file and unknown to the app, and nothing says so.
        let mut mine: Vec<&str> = Action::ALL.iter().map(|action| action.name()).collect();
        let mut theirs: Vec<&str> = zet_config::ACTIONS.to_vec();
        mine.sort_unstable();
        theirs.sort_unstable();
        assert_eq!(mine, theirs);
    }

    #[test]
    fn an_unknown_name_is_not_an_action() {
        assert_eq!(Action::from_name("split-pane"), None);
        assert_eq!(
            Action::from_name("New-Tab"),
            None,
            "names are case-sensitive"
        );
    }
}
