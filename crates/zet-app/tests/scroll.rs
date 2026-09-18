//! What the scroll actions do to a real terminal with real history in it.
//!
//! These need a live shell and a few hundred lines of output, which is why they are here
//! rather than beside the actions: every other test in `zet-app` drives state that a test
//! can set up by hand, and a scrollback is the one thing that cannot be faked without
//! inventing a second terminal. The session crate's own integration tests take the same
//! shape for the same reason.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use zet_app::{Action, App};
use zet_config::Config;
use zet_input::{Key, KeyEvent, KeyKind, Modifiers};
use zet_session::NoopWaker;

/// How long the shell gets to print two hundred lines before the test gives up.
///
/// Generous, and deliberately not a sleep: the loop below polls for the line it is
/// waiting for and leaves the moment it arrives, so this is a ceiling on a hang rather
/// than a cost every run pays.
const PATIENCE: Duration = Duration::from_secs(20);

fn app() -> App {
    App::new(
        Config::default(),
        PathBuf::from("test.toml"),
        Vec::new(),
        Arc::new(NoopWaker),
    )
    .expect("this machine has a shell")
}

fn chord(key: Key) -> KeyEvent {
    KeyEvent {
        key,
        mods: Modifiers::CTRL | Modifiers::SHIFT,
        text: None,
        kind: KeyKind::Press,
    }
}

/// An app with `cmd.exe` open on a screen short enough to fill up.
///
/// `cmd` rather than the default profile because the command below has to be one the
/// shell will actually run: the default on this machine is PowerShell, where
/// `for /l %i in ...` is a syntax error and the test would spend its patience waiting for
/// a line that was never going to be printed.
fn app_with_history(cols: u16, rows: u16) -> App {
    let mut app = app();
    let cmd = app
        .profiles()
        .iter()
        .find(|profile| {
            profile
                .program
                .file_name()
                .is_some_and(|name| name == "cmd.exe")
        })
        .expect("cmd.exe ships with Windows and discovery finds it before anything else");
    let id = cmd.id.clone();
    let _ = app.open_tab_with(&id, cols, rows).expect("cmd starts");
    let _ = app
        .active()
        .expect("a tab is open")
        .write(b"for /l %i in (1,1,200) do @echo zet-line-%i-of-200\r\n");

    let deadline = Instant::now() + PATIENCE;
    while !app.pump() || !on_screen(&app).contains("zet-line-200-of-200") {
        assert!(
            Instant::now() < deadline,
            "the shell never printed the lines the test needs:\n{}",
            on_screen(&app)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    app
}

/// What the active tab is showing, as one string.
///
/// The cells are read one at a time rather than through any `text` on a row: a row is a
/// fixed number of cells and the characters in it are whatever a program put there, which
/// is the same thing the renderer walks.
fn on_screen(app: &App) -> String {
    app.active()
        .map(|session| {
            session
                .visible_rows()
                .iter()
                .map(|row| row.cells().iter().map(|cell| cell.ch).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

#[test]
fn scroll_to_top_shows_the_oldest_line_and_scroll_to_bottom_comes_back() {
    // The two halves of one claim, asserted together because either alone is satisfied by
    // a terminal that does nothing: `Ctrl+Shift+Home` has to move the view off the live
    // screen and onto the oldest line, and `Ctrl+Shift+End` has to bring it back. The
    // failure this exists for is the two of them being the same keystroke, which is what
    // they were — `ScrollToTop` scrolled by `i32::MIN`, and a negative delta in the
    // session means "towards the live screen", so both went to the bottom and the oldest
    // line was unreachable by any key.
    let mut app = app_with_history(80, 10);
    assert_eq!(
        app.active().expect("a tab").scroll_offset(),
        0,
        "the view starts on the live screen"
    );

    let _ = app.key(&chord(Key::Home));
    let offset = app.active().expect("a tab").scroll_offset();
    assert!(offset > 0, "Ctrl+Shift+Home did not move the view");
    let screen = on_screen(&app);
    assert!(
        screen.contains("zet-line-1-of-200"),
        "the oldest line is not on screen:\n{screen}"
    );
    assert!(
        !screen.contains("zet-line-200-of-200"),
        "the newest line is still on screen, so the view did not go back:\n{screen}"
    );

    let _ = app.key(&chord(Key::End));
    assert_eq!(
        app.active().expect("a tab").scroll_offset(),
        0,
        "Ctrl+Shift+End did not come back to the live screen"
    );
    assert!(on_screen(&app).contains("zet-line-200-of-200"));
}

#[test]
fn the_actions_the_keys_run_are_the_ones_the_keymap_names() {
    // A guard on the test above rather than on the app: if the default keymap ever moves
    // scroll-to-top off `Ctrl+Shift+Home`, that test would quietly stop testing it.
    let app = app();
    for (key, action) in [
        (Key::Home, Action::ScrollToTop),
        (Key::End, Action::ScrollToBottom),
    ] {
        assert_eq!(
            app.bound(Modifiers::CTRL | Modifiers::SHIFT, key),
            Some(action),
            "{key:?} is no longer {action:?}"
        );
    }
}
