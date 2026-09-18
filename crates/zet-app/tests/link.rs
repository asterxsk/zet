//! Whether a click on a hyperlink reaches the URL the program published.
//!
//! `zet-vt` proves the sequence is parsed and kept, and `zet-render` proves a cell that
//! carries a link is drawn as one. Neither proves the two ends meet: the app could read
//! the wrong cell, or the wrong session, or nothing at all, and every unit test in the
//! workspace would still pass. What needs a real program is a program publishing a link,
//! so that is where this lives — beside `scroll.rs` and `hold.rs`, in the same shape.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use zet_app::{App, Command};
use zet_config::Config;
use zet_session::NoopWaker;
use zet_vt::{Cell, Pos};

/// How long the shell gets to do as it is told before the test gives up.
const PATIENCE: Duration = Duration::from_secs(20);

/// The URL the shell is told to publish.
const URL: &str = "https://example.com/zet";

fn app() -> App {
    App::new(
        Config::default(),
        PathBuf::from("test.toml"),
        Vec::new(),
        Arc::new(NoopWaker),
    )
    .expect("this machine has a shell")
}

/// Open a `cmd` tab and make it print a hyperlink.
///
/// `cmd` rather than the default profile for the reason `hold.rs` gives: the sequence has
/// to come from something with no opinion of its own. The prompt is the one place `cmd`
/// expands `$E` into an escape byte, and OSC 8 may be closed with `ST` rather than `BEL`
/// — which matters here, because a prompt can type a backslash and cannot type a bell. A
/// prompt is printed once and the shell then waits, so the link is on screen for as long
/// as the assertions need it and neither side is racing the other.
fn app_with_a_link(url: &str) -> App {
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
    let _ = app.open_tab_with(&id, 80, 10).expect("cmd starts");
    let _ = app
        .active()
        .expect("a tab is open")
        .write(format!("prompt $E]8;;{url}$E\\LINK$E]8;;$E\\\r\n").as_bytes());
    let deadline = Instant::now() + PATIENCE;
    while linked_cell(&app).is_none() {
        app.pump();
        assert!(
            Instant::now() < deadline,
            "the shell never printed the link"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    app
}

/// The first cell on screen matching `want`, and where it is.
fn find(app: &App, want: impl Fn(&Cell) -> bool) -> Option<Pos> {
    let session = app.active()?;
    let grid = session.term().grid();
    let top = grid.history_top(session.scroll_offset());
    for row in 0..grid.rows() {
        let line = grid.row_from_history(top + row)?;
        for col in 0..grid.cols() {
            if want(&line.get(col)) {
                return Some(Pos::new(row, col));
            }
        }
    }
    None
}

/// A cell that carries a hyperlink, and one that does not, both on screen.
fn linked_cell(app: &App) -> Option<Pos> {
    find(app, |cell| cell.link != 0)
}

fn plain_cell(app: &App) -> Option<Pos> {
    find(app, |cell| cell.link == 0 && cell.ch != ' ')
}

#[test]
fn a_click_on_a_linked_cell_asks_the_host_to_open_the_url() {
    let mut app = app_with_a_link(URL);
    let at = linked_cell(&app).expect("the shell printed a link");
    assert_eq!(
        app.open_link_at(at),
        vec![Command::OpenUrl(URL.to_owned())],
        "clicking a link did not ask the host to open it"
    );
}

#[test]
fn a_click_elsewhere_asks_for_nothing() {
    // The gesture is Ctrl+click on a link and nothing else. Every other cell is text,
    // and a click on text is how a selection starts — a cell next to a link that opened
    // it too would make the link's own run impossible to select.
    let mut app = app_with_a_link(URL);
    let at = plain_cell(&app).expect("the shell printed its own prompt too");
    assert!(
        !app.open_link_at(at)
            .iter()
            .any(|command| matches!(command, Command::OpenUrl(_))),
        "a cell with no link opened one"
    );
}

#[test]
fn a_link_that_names_something_to_run_rather_than_open_is_ignored() {
    // A hyperlink is text a *program* chose, and the system's "open this" call does not
    // only open things: handed a path it runs it. So the schemes that mean "a page" are
    // the ones that get through, and anything else is dropped here rather than being
    // handed to the shell — a program that wants to run something can print a command
    // for the user to read, and one that wants to run something *without* being read is
    // the case this is for.
    let mut app = app_with_a_link(r"Z:\nowhere\zet-should-not-run-this.exe");
    let at = linked_cell(&app).expect("the shell printed a link");
    assert_eq!(
        app.open_link_at(at),
        Vec::new(),
        "a link the system would run rather than open was opened"
    );
}
