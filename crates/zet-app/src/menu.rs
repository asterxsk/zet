//! What a right-click on a tab offers.

/// What pressing an item does.
///
/// Not a [`Command`](crate::Command), because most of these are not one thing the window
/// does: closing a tab is a change to the session list, and the answer to it is a command
/// only when the last tab went. The app turns an action into whatever it turns out to be,
/// which is where the knowledge of what a tab is already lives.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Open a tab with the default profile.
    NewTab,
    /// Open a second window.
    NewWindow,
    /// Close the tab the menu is about.
    Close,
    /// Close every tab but that one.
    CloseOthers,
}

/// One item of a tab's context menu: what it says, and what pressing it does.
///
/// The two together rather than two lists, because a menu drawn from one list and answered
/// from another is a menu where the row under the pointer and the thing that happens are
/// kept in step by hand. The words are here and not in the painter for the reason every
/// other word in the chrome is: a list of settings is the app's, and a rectangle is not.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Item {
    /// What the row says.
    pub label: &'static str,
    /// What pressing it does.
    pub action: Action,
}

/// The items a right-click on a tab offers, given how many tabs are open.
///
/// `Close others` is offered only when there are others. A menu is a list of things the
/// user can have, and an item that would do nothing is an item that teaches the user the
/// menu does not work — the same argument the panel's thickness row is built on.
///
/// No item is ever offered in a state where it does nothing: every one of these is
/// possible from any tab of any window, which is what keeps the answer to "which item is
/// this index" the same list the menu was drawn from.
#[must_use]
pub fn items(tabs: usize) -> Vec<Item> {
    let mut items = vec![
        Item {
            label: "New tab",
            action: Action::NewTab,
        },
        Item {
            label: "New window",
            action: Action::NewWindow,
        },
        Item {
            label: "Close tab",
            action: Action::Close,
        },
    ];
    if tabs > 1 {
        items.push(Item {
            label: "Close other tabs",
            action: Action::CloseOthers,
        });
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four things a tab can be told to do, in the order they are offered.
    ///
    /// Order matters twice over: it is what the user reads, and it is what an index means.
    /// Closing is last of the three that are always there because it is the one that takes
    /// something away, and a menu whose destructive item is in the middle is a menu where
    /// a mis-aimed click lands on it.
    #[test]
    fn a_tab_offers_the_four_things_it_can_do() {
        let labels: Vec<&str> = items(3).into_iter().map(|item| item.label).collect();
        assert_eq!(
            labels,
            ["New tab", "New window", "Close tab", "Close other tabs"]
        );
        let actions: Vec<Action> = items(3).into_iter().map(|item| item.action).collect();
        assert_eq!(
            actions,
            [
                Action::NewTab,
                Action::NewWindow,
                Action::Close,
                Action::CloseOthers
            ]
        );
    }

    /// A window with one tab is a window with no others to close.
    ///
    /// The item would do nothing, and the panel's rule is that a row which changes nothing
    /// is a row that makes the user doubt the panel rather than the setting. The same
    /// menu in the same window has to say the same thing every time it is opened, which is
    /// why this is a function of the tab count rather than of the tab.
    #[test]
    fn closing_the_others_is_not_offered_when_there_are_none() {
        let labels: Vec<&str> = items(1).into_iter().map(|item| item.label).collect();
        assert_eq!(labels, ["New tab", "New window", "Close tab"]);
    }

    /// Every item says something, and no two say the same thing.
    ///
    /// Two rows with one word each are two rows a user cannot tell apart, and the second
    /// of them would be unreachable however many times it was drawn.
    #[test]
    fn every_item_has_its_own_words() {
        let mut labels: Vec<&str> = items(4).into_iter().map(|item| item.label).collect();
        assert!(labels.iter().all(|label| !label.is_empty()));
        labels.sort_unstable();
        let count = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), count, "two items share a label");
    }
}
