//! Whether the app notices a program that has asked it to hold the frame.
//!
//! [`zet_app::hold`] proves the budget and `zet-vt` proves the marker is parsed, and
//! neither of them proves the two are connected: `holds_frame` could read the wrong
//! session, or the wrong bit, or nothing at all, and every unit test in the workspace
//! would still pass. The one thing that needs a real program is the program asking, so
//! that is where this lives — beside `scroll.rs`, for the same reason and in the same
//! shape.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use zet_app::App;
use zet_app::hold::BUDGET;
use zet_config::Config;
use zet_session::NoopWaker;

/// How long the shell gets to do as it is told before the test gives up.
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

#[test]
fn a_program_mid_repaint_holds_the_frame_and_the_hold_expires() {
    // `cmd` rather than the default profile because the marker has to be emitted by
    // something with no opinion of its own: `cmd` never sends `DECSET 2026`, so the only
    // thing in this window that can set it is the line this test types. A shell that used
    // synchronized output for its own prompt — PowerShell's line editor does — would make
    // the assertion below pass whether or not the app was reading anything.
    //
    // The marker is set through `prompt`, which is the one place `cmd` expands `$E` into
    // an escape byte. A prompt is printed once and the shell then waits for ever, so the
    // mode stays set for as long as the assertions need it and neither side is racing the
    // other.
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

    let idle = Instant::now();
    assert_eq!(
        app.frame_hold(idle),
        None,
        "the frame was held for a program that had not asked for it"
    );

    let _ = app
        .active()
        .expect("a tab is open")
        .write(b"prompt $E[?2026h$G\r\n");
    let deadline = Instant::now() + PATIENCE;
    while !app.pump() || !synchronized(&app) {
        assert!(
            Instant::now() < deadline,
            "the shell never printed the marker the test typed at it"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // The deadline is asserted exactly, and not merely as "some instant": it is the start
    // of the repaint plus the budget, so a version that measured the budget from each
    // call would still answer `Some` here and still leave the window frozen. This is the
    // one place the two are told apart by something other than a stopwatch.
    let now = Instant::now();
    assert_eq!(
        app.frame_hold(now),
        Some(now + BUDGET),
        "the app drew over a program that is mid-repaint"
    );
    assert_eq!(
        app.frame_hold(now + Duration::from_millis(10)),
        Some(now + BUDGET),
        "the hold did not survive the repaint it was granted for"
    );
    assert_eq!(
        app.frame_hold(now + BUDGET),
        None,
        "the hold outlived its budget, which is the freeze the budget exists to prevent"
    );
}

/// Whether the active tab's program has asked for the frame to be held.
///
/// Read through the session rather than through the app so the assertion above tests the
/// app's reading of it rather than restating it.
fn synchronized(app: &App) -> bool {
    app.active()
        .is_some_and(|session| session.term().is_synchronized())
}
