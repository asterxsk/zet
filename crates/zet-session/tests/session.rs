//! Drives real sessions through real pseudoconsoles.
//!
//! Everything in this crate that is bookkeeping is tested next to it, without a process.
//! This file is the part that only a real console can answer: does a spawned child
//! produce output that lands on the grid, does typing reach it, does a resize survive the
//! round trip to `ConPTY`, and does a session notice that its child is gone.
//!
//! Every test carries its own deadline. A failure in the pty layer arrives as a hang
//! rather than as an error, and a test suite that hangs is a test suite nobody runs.

#![cfg(windows)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use zet_pty::discovery::{Profile, Source};
use zet_session::{NoopWaker, Session, Sessions};

/// How long any single test may spend waiting for the child before it gives up.
///
/// Nothing here should take more than a second on a working machine. The margin is for a
/// loaded CI box, not for a slow terminal.
const PATIENCE: Duration = Duration::from_secs(20);

/// The directory cargo hands out for scratch files, emptied before each test.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the scratch directory should be creatable");
    dir
}

fn system32(executable: &str) -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| OsString::from(r"C:\Windows"));
    PathBuf::from(root).join("System32").join(executable)
}

/// A command-prompt profile carrying the arguments for one test.
///
/// Built here rather than discovered, so that a machine with an unusual set of installed
/// shells runs the same tests as any other.
fn cmd(args: &[&str]) -> Profile {
    Profile {
        id: "cmd-test".into(),
        name: "Command Prompt".into(),
        program: system32("cmd.exe"),
        args: args.iter().map(OsString::from).collect(),
        source: Source::KnownLocation,
    }
}

fn noop() -> Arc<dyn zet_session::Waker> {
    Arc::new(NoopWaker)
}

/// The whole screen as text, one line per row.
fn screen(session: &Session) -> String {
    let grid = session.term().grid();
    (0..grid.rows())
        .map(|row| {
            (0..grid.cols())
                .map(|col| grid.row(row).get(col).ch)
                .collect::<String>()
        })
        .collect::<Vec<String>>()
        .join("\n")
}

/// The rows the view is showing, one line each.
///
/// Not the same thing as [`screen`] when the view is scrolled back: this is what a
/// renderer would be handed, and the whole point of `visible_rows` is that it comes from
/// the history as well as the screen.
fn view(session: &Session) -> String {
    session
        .visible_rows()
        .iter()
        .map(|row| {
            (0..usize::from(session.cols()))
                .map(|col| row.get(col).ch)
                .collect::<String>()
        })
        .collect::<Vec<String>>()
        .join("\n")
}

/// Drain until a call comes back with nothing left to feed.
fn drain_to_rest(session: &mut Session) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let drained = session.drain();
        if drained.bytes == 0 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the child never stopped printing"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Drain until the screen holds `needle`, and report what the screen looked like.
fn drain_until(session: &mut Session, needle: &str) -> String {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let _drained = session.drain();
        let text = screen(session);
        if text.contains(needle) || Instant::now() >= deadline {
            return text;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_spawned_session_produces_output_that_reaches_the_grid() {
    let mut session = Session::spawn(&cmd(&["/c", "echo zet-marker"]), 80, 24, None, noop())
        .expect("the session should start");

    let text = drain_until(&mut session, "zet-marker");
    assert!(
        text.contains("zet-marker"),
        "the child's output should be on the grid, got:\n{text}"
    );

    session.close().expect("the session should close");
}

#[test]
fn drain_reports_the_bytes_it_fed_and_the_damage_they_caused() {
    let mut session = Session::spawn(&cmd(&["/c", "echo zet-marker"]), 80, 24, None, noop())
        .expect("the session should start");

    // Not every byte the child sends changes the grid: the pseudoconsole opens with
    // cursor and mode sequences that only move state. What has to be true is that the
    // batch which put text on the grid reported the damage it did.
    let deadline = Instant::now() + PATIENCE;
    let mut fed = 0usize;
    let mut damaged = false;
    loop {
        let drained = session.drain();
        fed += drained.bytes;
        damaged |= drained.damage;
        if screen(&session).contains("zet-marker") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the child never printed anything"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(fed > 0, "the child printed nothing at all");
    assert!(damaged, "the bytes that reached the grid went unreported");

    // ... and a drain with nothing in it reports nothing changed. This is the half that
    // a latched flag would fail: the screen has already changed once, so a tracker that
    // was never cleared would keep saying so.
    let deadline = Instant::now() + PATIENCE;
    loop {
        let idle = session.drain();
        if idle.bytes == 0 {
            assert!(
                !idle.damage,
                "an empty drain must not report damage left over from an earlier one"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the child never stopped printing"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    session.close().expect("the session should close");
}

#[test]
fn writing_reaches_the_child() {
    // `cmd /k` stays alive and reads its input, which is what a real shell session does.
    let mut session = Session::spawn(&cmd(&["/k", "prompt $g"]), 80, 24, None, noop())
        .expect("the session should start");

    // Wait for the prompt so that the write does not race the console attaching.
    drain_until(&mut session, ">");

    session
        .write(b"echo zet-typed\r\n")
        .expect("the write should succeed");

    let text = drain_until(&mut session, "zet-typed");
    assert!(
        text.contains("zet-typed"),
        "what was typed should come back from the child, got:\n{text}"
    );

    session.close().expect("the session should close");
}

#[test]
fn resize_is_accepted_and_the_grid_follows() {
    let mut session =
        Session::spawn(&cmd(&["/k"]), 80, 24, None, noop()).expect("the session should start");
    let _drained = session.drain();

    session
        .resize(120, 40)
        .expect("the resize should be accepted");
    assert_eq!(session.cols(), 120);
    assert_eq!(session.rows(), 40);
    assert_eq!(session.term().grid().cols(), 120);
    assert_eq!(session.term().grid().rows(), 40);
    assert_eq!(
        session.visible_rows().len(),
        40,
        "the view has to follow the terminal it is a view of"
    );

    session
        .resize(80, 24)
        .expect("the resize back should be accepted");
    assert_eq!(session.cols(), 80);
    assert_eq!(session.rows(), 24);

    session.close().expect("the session should close");
}

#[test]
fn a_session_whose_child_has_exited_reports_it_once() {
    let mut session = Session::spawn(&cmd(&["/c", "exit 3"]), 80, 24, None, noop())
        .expect("the session should start");

    let deadline = Instant::now() + PATIENCE;
    let mut announced = 0usize;
    loop {
        let drained = session.drain();
        if drained.exited {
            announced += 1;
        }
        if session.is_exited() && drained.bytes == 0 {
            break;
        }
        assert!(Instant::now() < deadline, "the child never exited");
        std::thread::sleep(Duration::from_millis(20));
    }

    assert!(session.is_exited());
    assert_eq!(session.exit_code(), Some(3), "the child's own exit code");
    assert_eq!(
        announced, 1,
        "the exit is an edge: a drain reports it once, and `is_exited` is the state"
    );

    // Writing into a session with no child is refused rather than silently accepted: the
    // console's input pipe is still open, so the write itself would succeed.
    assert!(session.write(b"x").is_err());

    session.close().expect("the session should close");
}

#[test]
fn a_session_starts_the_child_in_the_directory_it_was_given() {
    let dir = scratch("session-cwd");
    let mut session = Session::spawn(&cmd(&["/c", "cd"]), 80, 24, Some(dir), noop())
        .expect("the session should start");

    let text = drain_until(&mut session, "session-cwd");
    assert!(
        text.contains("session-cwd"),
        "the child should have started in the scratch directory, got:\n{text}"
    );

    session.close().expect("the session should close");
}

#[test]
fn scrolling_back_shows_the_history_and_coming_down_shows_the_screen_again() {
    // Two hundred lines on a ten-row screen is mostly scrollback, which is the only way
    // to tell a viewport that reads the history from one that does not.
    let command = "for /l %i in (1,1,200) do @echo zet-line-%i-of-200";
    let mut session = Session::spawn(&cmd(&["/c", command]), 80, 10, None, noop())
        .expect("the session should start");
    drain_until(&mut session, "zet-line-200-of-200");
    drain_to_rest(&mut session);

    let history = session.term().grid().scrollback_len();
    assert!(history > 0, "two hundred lines cannot fit on ten rows");

    // At the bottom, the view is the live screen, and the live screen ends with the
    // child's last line.
    assert_eq!(session.scroll_offset(), 0);
    assert_eq!(session.visible_rows().len(), 10);
    let bottom = screen(&session);
    assert!(bottom.contains("zet-line-200-of-200"), "got:\n{bottom}");

    // Scrolled all the way back, the newest line has to have left the window.
    session.scroll(i32::MAX);
    assert_eq!(session.scroll_offset(), history);
    let oldest = view(&session);
    assert_eq!(oldest.lines().count(), 10);
    assert!(
        !oldest.contains("zet-line-200-of-200"),
        "the newest line should have scrolled off the bottom of the view:\n{oldest}"
    );
    assert!(
        oldest.contains("zet-line-"),
        "the view should be showing the history:\n{oldest}"
    );

    // One row at a time, and down past the bottom clamps rather than panicking.
    session.scroll(-1);
    assert_eq!(session.scroll_offset(), history - 1);
    session.scroll(-i32::MAX);
    assert_eq!(session.scroll_offset(), 0);
    assert!(screen(&session).contains("zet-line-200-of-200"));

    session.close().expect("the session should close");
}

#[test]
fn output_does_not_move_a_view_that_is_scrolled_back() {
    // A program that logs while the user is reading history must not yank the view to the
    // bottom every time it prints. Typing does that — see `writing_reaches_the_child` —
    // and nothing here types after scrolling.
    //
    // The loop is deliberately far longer than the test can drain, so that the child is
    // still printing for as long as the assertion runs. A child that stopped first would
    // make this pass without ever testing anything.
    let command = "for /l %i in (1,1,100000) do @echo zet-fill-%i-of-100000";
    let mut session = Session::spawn(&cmd(&["/k", command]), 80, 10, None, noop())
        .expect("the session should start");

    let deadline = Instant::now() + PATIENCE;
    while session.term().grid().scrollback_len() == 0 {
        let _drained = session.drain();
        assert!(
            Instant::now() < deadline,
            "the child never filled the screen"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    session.scroll(i32::MAX);
    let offset = session.scroll_offset();
    assert!(offset > 0, "there should be history to scroll into");

    let mut printed_while_scrolled = 0usize;
    let mut idle = 0usize;
    let deadline = Instant::now() + Duration::from_secs(2);
    while idle < 2 && Instant::now() < deadline {
        let drained = session.drain();
        assert_eq!(
            session.scroll_offset(),
            offset,
            "output arriving while the view is scrolled back must leave the view alone"
        );
        if drained.bytes > 0 {
            printed_while_scrolled += drained.bytes;
            idle = 0;
        } else {
            idle += 1;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        printed_while_scrolled > 0,
        "the child should have printed while the view was scrolled back, or this test \
         proved nothing"
    );

    // ... and the way back to the live screen is the user's to ask for.
    session.scroll_to_bottom();
    assert_eq!(session.scroll_offset(), 0);

    session.close().expect("the session should close");
}

#[test]
fn tabs_are_numbered_from_one_and_a_number_is_never_reused() {
    let mut sessions = Sessions::new();
    assert!(sessions.is_empty());
    assert_eq!(sessions.active(), None);

    let first = sessions
        .open(&cmd(&["/k"]), 80, 24, None, noop())
        .expect("the first session should start");
    let second = sessions
        .open(&cmd(&["/k"]), 80, 24, None, noop())
        .expect("the second session should start");
    assert_eq!((first, second), (1, 2));
    assert_eq!(sessions.active(), Some(2), "a new tab is the active one");

    sessions
        .close(1)
        .expect("closing the first tab should work");
    assert_eq!(sessions.numbers(), vec![2]);

    let third = sessions
        .open(&cmd(&["/k"]), 80, 24, None, noop())
        .expect("the third session should start");
    assert_eq!(third, 3, "#1 must not be handed out again");

    assert!(
        sessions.close(1).is_err(),
        "closing a tab that is already gone has to be an error"
    );

    assert_eq!(sessions.iter().count(), 2);
    assert!(sessions.get(1).is_none());
    assert!(sessions.get_mut(2).is_some());
}

#[test]
fn reap_removes_a_session_whose_child_has_exited_and_keeps_the_rest() {
    let mut sessions = Sessions::new();
    let ended = sessions
        .open(&cmd(&["/c", "exit"]), 80, 24, None, noop())
        .expect("the first session should start");
    let running = sessions
        .open(&cmd(&["/k"]), 80, 24, None, noop())
        .expect("the second session should start");

    let deadline = Instant::now() + PATIENCE;
    while !sessions.get(ended).is_some_and(Session::is_exited) {
        assert!(Instant::now() < deadline, "the child never exited");
        std::thread::sleep(Duration::from_millis(20));
    }

    assert_eq!(sessions.reap(), vec![ended]);
    assert_eq!(sessions.numbers(), vec![running]);
    assert!(sessions.get(running).is_some());
    assert_eq!(sessions.reap(), Vec::<u32>::new(), "nothing left to reap");
}

#[test]
fn dropping_sessions_closes_them_without_hanging() {
    // The teardown path is where `ConPTY` deadlocks if the pump is not stopped first, and
    // a dropped collection is the path a host takes when its window closes.
    for _ in 0..3 {
        let mut sessions = Sessions::new();
        sessions
            .open(&cmd(&["/k"]), 80, 24, None, noop())
            .expect("the session should start");
        sessions
            .open(&cmd(&["/k"]), 80, 24, None, noop())
            .expect("the session should start");
    }
}

#[test]
fn the_scratch_directory_is_inside_the_repository() {
    // A guard on the test harness itself. If cargo ever moves `CARGO_TARGET_TMPDIR`
    // somewhere outside the checkout, the test above starts writing to a user profile
    // directory, and this is the assertion that notices.
    let tmp = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository = manifest
        .parent()
        .and_then(Path::parent)
        .expect("the repository root");
    assert!(
        tmp.starts_with(repository),
        "{tmp:?} is not inside {repository:?}"
    );
}
