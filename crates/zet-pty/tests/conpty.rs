//! Drives real child processes through real pseudoconsoles.
//!
//! These are the only tests that prove the FFI works. Everything else in the crate is
//! pure logic that a unit test can reach; this file is the part where a wrong handle
//! order, a missing attribute, or a misplaced close shows up as a hang instead of a
//! failure, which is why every test here carries its own deadline and shuts its session
//! down through [`Pty::shutdown`] rather than dropping it.

#![cfg(windows)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use zet_pty::{Pty, SpawnConfig};

/// How long any single test may spend waiting for output before it gives up.
///
/// Nothing here should take more than a second on a working machine. The margin is for a
/// loaded CI box, not for a slow terminal.
const PATIENCE: Duration = Duration::from_secs(20);

fn system32(executable: &str) -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| OsString::from(r"C:\Windows"));
    PathBuf::from(root).join("System32").join(executable)
}

fn cmd() -> PathBuf {
    system32("cmd.exe")
}

/// Run `program` to completion and collect everything it printed.
fn collect(config: &SpawnConfig) -> Vec<u8> {
    let pty = Pty::spawn(config).expect("the session should start");
    let mut collected = Vec::new();
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        match pty.next_chunk(Duration::from_millis(200)) {
            Some(Ok(chunk)) => collected.extend_from_slice(&chunk),
            // The reader reports the session ending, which is the normal way this loop
            // finishes for a short-lived child.
            Some(Err(_)) => break,
            None => {}
        }
    }
    pty.shutdown().expect("the session should shut down");
    collected
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn a_child_process_runs_and_what_it_prints_comes_back() {
    let config = SpawnConfig::new(cmd(), 80, 24).args(["/c", "echo zet-marker"]);
    let output = collect(&config);
    assert!(
        contains(&output, b"zet-marker"),
        "expected the child's output, got {:?}",
        String::from_utf8_lossy(&output)
    );
}

#[test]
fn the_output_arrives_as_vt_rather_than_as_plain_text() {
    // ConPTY renders the child's console into escape sequences. If this ever returns the
    // raw text with no escapes, the pseudoconsole was not actually attached and the
    // whole architecture is wrong.
    let config = SpawnConfig::new(cmd(), 80, 24).args(["/c", "echo hello"]);
    let output = collect(&config);
    assert!(
        output.contains(&0x1b),
        "expected an escape byte in {:?}",
        String::from_utf8_lossy(&output)
    );
}

#[test]
fn a_size_change_is_accepted_while_the_child_is_running() {
    let pty = Pty::spawn(&SpawnConfig::new(cmd(), 80, 24).args(["/k", "prompt"]))
        .expect("the session should start");
    // Give the child a moment to attach before changing its window.
    std::thread::sleep(Duration::from_millis(500));
    pty.resize(120, 40).expect("the resize should be accepted");
    pty.resize(80, 24).expect("the resize back should be accepted");
    pty.shutdown().expect("the session should shut down");
}

#[test]
fn typing_reaches_the_child() {
    // `cmd /k` stays alive and reads its input, which is what a real shell session does.
    let mut pty = Pty::spawn(&SpawnConfig::new(cmd(), 80, 24).args(["/k", "prompt $g"]))
        .expect("the session should start");

    // Wait for the banner so the write does not race the console attaching.
    let mut seen = Vec::new();
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline && !contains(&seen, b">") {
        if let Some(Ok(chunk)) = pty.next_chunk(Duration::from_millis(200)) {
            seen.extend_from_slice(&chunk);
        }
    }

    pty.write(b"echo zet-typed\r\n").expect("the write should succeed");

    let mut after = Vec::new();
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        match pty.next_chunk(Duration::from_millis(200)) {
            Some(Ok(chunk)) => {
                after.extend_from_slice(&chunk);
                if contains(&after, b"zet-typed") {
                    break;
                }
            }
            Some(Err(_)) => break,
            None => {}
        }
    }
    pty.shutdown().expect("the session should shut down");
    assert!(
        contains(&after, b"zet-typed"),
        "the shell should have echoed its own output back, got {:?}",
        String::from_utf8_lossy(&after)
    );
}

#[test]
fn the_exit_code_of_the_child_is_reported() {
    let pty = Pty::spawn(&SpawnConfig::new(cmd(), 80, 24).args(["/c", "exit 7"]))
        .expect("the session should start");
    let code = pty.wait(PATIENCE);
    pty.shutdown().expect("the session should shut down");
    assert_eq!(code, Some(7), "the child's own exit code should come through");
}

#[test]
fn killing_the_session_ends_a_child_that_would_otherwise_run_forever() {
    let pty = Pty::spawn(&SpawnConfig::new(cmd(), 80, 24).args(["/c", "ping -n 60 127.0.0.1"]))
        .expect("the session should start");
    std::thread::sleep(Duration::from_millis(500));
    assert!(pty.wait(Duration::from_millis(50)).is_none(), "it should still be running");

    pty.kill().expect("the kill should succeed");
    assert!(
        pty.wait(PATIENCE).is_some(),
        "the child should be gone after the kill"
    );
    pty.shutdown().expect("the session should shut down");
}

#[test]
fn a_shell_session_can_be_opened_and_closed_repeatedly() {
    // The close path is where ConPTY deadlocks if the ordering is wrong. Ten in a row
    // catches the version of that bug which only shows up on the second or third open.
    for attempt in 0..10 {
        let pty = Pty::spawn(&SpawnConfig::new(cmd(), 80, 24).args(["/c", "echo", "x"]))
            .unwrap_or_else(|error| panic!("attempt {attempt} failed to start: {error}"));
        pty.shutdown()
            .unwrap_or_else(|error| panic!("attempt {attempt} failed to shut down: {error}"));
    }
}

#[test]
fn a_program_that_writes_a_lot_does_not_deadlock_the_reader() {
    // The reader blocks when its channel is full, and the channel here is never drained
    // until the end. If backpressure is wired up wrongly, this hangs instead of failing.
    // Five thousand lines is far more than the channel's sixty-four slots.
    let config = SpawnConfig::new(cmd(), 80, 24).args([
        "/c",
        "for /l %i in (1,1,5000) do @echo line %i of filler text to make the buffer work",
    ]);
    let pty = Pty::spawn(&config).expect("the session should start");

    // Deliberately do not read for a moment, so the channel fills and the reader parks.
    std::thread::sleep(Duration::from_secs(2));

    let mut total = 0usize;
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        match pty.next_chunk(Duration::from_millis(200)) {
            Some(Ok(chunk)) => total += chunk.len(),
            Some(Err(_)) => break,
            None => {
                if pty.wait(Duration::from_millis(50)).is_some() {
                    break;
                }
            }
        }
    }
    pty.shutdown().expect("the session should shut down");
    assert!(
        total > 10_000,
        "expected a substantial amount of output, got {total} bytes"
    );
}
