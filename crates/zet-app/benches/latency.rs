//! The numbers the promise names, measured rather than assumed.
//!
//! PRODUCT.md says that cold start, keystroke latency, and throughput under heavy
//! output are the numbers that matter, "and they get measured rather than assumed".
//! This is the measuring:
//!
//! ```text
//! cargo bench -p zet-app --bench latency
//! ```
//!
//! It prints and does not fail. A timing threshold on a shared runner is a flaky test
//! with a longer name, and the one latency in the file that is a budget — criterion 1's
//! 300 ms to a first prompt — is written as a check on this machine, which a hosted
//! runner cannot make on your behalf. What the numbers are for is a person reading them
//! either side of a change.
//!
//! # What is not measured
//!
//! The last step of keystroke-to-pixel is the present, and it is not here because it
//! needs a device and a window and this runs on neither. Everything before it is: the
//! key to bytes at the pty, and bytes back into the grid. Reading the two together is
//! what says whether a latency you feel is zet's doing or the shell's, and the present
//! in between is a frame sized by the window rather than by anything in this file.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use zet_app::App;
use zet_config::Config;
use zet_input::{Key, KeyEvent, KeyKind, Modifiers};
use zet_session::NoopWaker;
use zet_vt::{Parser, Term};

/// How long the shell gets to print something before the run is called a failure.
const PATIENCE: Duration = Duration::from_secs(20);

/// The grid a tab is opened at. 80×24 is the size every terminal is measured at.
const COLS: u16 = 80;
const ROWS: u16 = 24;

/// How many times a per-keystroke measurement is taken, and how many the median is
/// reported from. One keystroke is far too fast to time on its own.
const KEYSTROKES: usize = 20_000;

/// How much output the throughput run feeds. A megabyte of text is on the order of a
/// large `cat`, and long enough that the per-call overhead stops being the number.
const THROUGHPUT_BYTES: usize = 1 << 20;

fn main() {
    println!("zet latency, on this machine\n");
    cold_start();
    keystroke();
    throughput();
}

/// Open a tab on the default profile and time it to the first character on screen.
///
/// The clock starts before `App::load`, because the configuration is read on the way to
/// the first prompt and a user waits for that too. The shell is the machine's own best
/// one — `App::open_tab`'s choice, not `cmd.exe` — because a cold start measured against
/// a shell nobody runs is a number about nothing.
///
/// The shell's name goes on the line, because a cold start is mostly the shell's and a
/// number without it says nothing about whose it is.
fn cold_start() {
    let started = Instant::now();
    let mut app = App::load(Arc::new(NoopWaker)).expect("this machine has a shell");
    let _ = app.open_tab(COLS, ROWS).expect("the default shell starts");
    let deadline = Instant::now() + PATIENCE;
    while !something_is_on_screen(&app) {
        app.pump();
        if Instant::now() > deadline {
            println!(
                "cold start to first prompt  {:>8}      (nothing was printed)",
                "-"
            );
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let name = app
        .sessions()
        .iter()
        .next()
        .map_or_else(|| "?".to_owned(), zet_session::Session::title);
    println!(
        "cold start to first prompt  {:>8.1} ms   ({name})",
        millis(started.elapsed())
    );
}

/// Whether any cell has been written to, which is the first prompt arriving.
///
/// A cell has no notion of empty — blank is a space with the default colours — so this
/// is the same test the grid makes when it decides whether anything is there.
fn something_is_on_screen(app: &App) -> bool {
    let Some(session) = app.active() else {
        return false;
    };
    let grid = session.term().grid();
    let Some(line) = grid.row_from_history(grid.history_top(session.scroll_offset())) else {
        return false;
    };
    (0..grid.cols()).any(|col| line.get(col).ch != ' ')
}

/// Time a keystroke from the event to the bytes at the pty.
///
/// This is `App::key`, which is the whole of zet's share of typing: the binding is
/// looked up, and what is left over is encoded and written. What the shell then does
/// with the byte is the shell's latency, and a benchmark that waited for the echo would
/// be measuring that instead.
fn keystroke() {
    let mut app = app();
    let _ = app.open_tab(COLS, ROWS).expect("the default shell starts");
    let event = KeyEvent {
        key: Key::Char('a'),
        mods: Modifiers::empty(),
        text: None,
        kind: KeyKind::Press,
    };
    let mut samples = Vec::with_capacity(KEYSTROKES);
    for _ in 0..KEYSTROKES {
        let started = Instant::now();
        let _ = app.key(&event);
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    println!(
        "keystroke to the pty        {:>8.1} us   (median of {KEYSTROKES})",
        micros(samples[KEYSTROKES / 2])
    );
}

/// Feed a megabyte of the output a terminal actually sees, and time the grid taking it.
///
/// The parser and the grid are the hot path of a build log or a `dir /s`, and this is
/// that path with nothing else in it. The payload is built rather than read so the
/// number does not move with the speed of the disk it came off.
fn throughput() {
    let payload = payload();
    let mut term = Term::new(COLS as usize, ROWS as usize);
    let mut parser = Parser::new();
    // Once to warm the allocator and the caches, so the timing is the work and not the
    // first touch of pages that are already resident on any real run.
    parser.advance_slice(&payload, &mut term);
    let started = Instant::now();
    parser.advance_slice(&payload, &mut term);
    let elapsed = started.elapsed();
    println!(
        "output into the grid       {:>8.1} MiB/s ({} KiB of mixed SGR and text)",
        mib_per_second(payload.len(), elapsed),
        payload.len() / 1024
    );
}

/// A megabyte of terminal output: coloured text, resets, and cursor placement.
///
/// The mix matters more than the size. Text alone is a fill loop over a row; the escape
/// sequences are what a build log is mostly made of, and a benchmark of one without the
/// other answers a question nobody asked.
fn payload() -> Vec<u8> {
    let mut out = Vec::with_capacity(THROUGHPUT_BYTES);
    let mut line = 0usize;
    while out.len() < THROUGHPUT_BYTES {
        // Four lines of a build log, with the shape a real one has: a timestamp that is
        // plain, a coloured status word, a message, and a carriage return.
        out.extend_from_slice(
            format!(
                "\x1b[90m{line:>6}\x1b[0m \x1b[32m  ok  \x1b[0m \
                 compiled zet-vt v0.1.0 in 0.{:03}s\r\n",
                line % 1000
            )
            .as_bytes(),
        );
        out.extend_from_slice(
            b"\x1b[33mwarning\x1b[0m: unused variable `x` \xe2\x80\x94 \
              the name is bound and never read\r\n",
        );
        out.extend_from_slice(b"\x1b[1;34m     Running\x1b[0m tests\\grid.rs\r\n");
        // A cursor move and an erase, because a progress line is what a long build
        // spends most of its output doing.
        out.extend_from_slice(b"\x1b[2K\x1b[1G     Building [=======>    ] 62%\r");
        line += 1;
    }
    out.truncate(THROUGHPUT_BYTES);
    out
}

/// A configured app with no tab open.
fn app() -> App {
    App::new(
        Config::default(),
        PathBuf::from("bench.toml"),
        Vec::new(),
        Arc::new(NoopWaker),
    )
    .expect("this machine has a shell")
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

fn micros(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e6
}

#[allow(clippy::cast_precision_loss)]
fn mib_per_second(bytes: usize, elapsed: Duration) -> f64 {
    (bytes as f64 / (1024.0 * 1024.0)) / elapsed.as_secs_f64()
}
