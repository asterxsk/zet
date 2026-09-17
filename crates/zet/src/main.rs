//! zet: a terminal for Windows, built around the grid.
//!
//! This crate is the binary and nothing else. Every decision the terminal makes lives in
//! a library below it — the parser in `zet-vt`, the pseudoconsole in `zet-pty`, the
//! state machine in `zet-app`, the drawing in `zet-render` and `zet-ui` — and each of
//! those was built to be testable without a window, which is why the interesting parts
//! of zet have tests and this file has a `main`.
//!
//! What is here is the set of things that cannot be a library: the event loop, the
//! window, and the four modules that talk to Win32. [`host`] is the event loop's
//! callback and the only place the platform and the state machine meet.

// The crate's lints live in `Cargo.toml`, and `unsafe_code` is denied there rather than
// forbidden — `platform` and `clipboard` call Win32 directly and each says so at the top
// of its own file. Nothing else in the binary contains an `unsafe` block.

mod clipboard;
mod host;
mod keys;
mod mouse;
mod platform;
mod waker;

use std::process::ExitCode;
use std::sync::Arc;

use winit::event_loop::{ControlFlow, EventLoop};

use zet_app::App;
use zet_session::Waker;

use crate::host::Host;
use crate::waker::{ProxyWaker, Wake};

/// What `zet --version` prints.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What `zet --help` prints.
const USAGE: &str = "\
zet — a terminal for Windows, built around the grid

USAGE:
    zet

OPTIONS:
    -h, --help       Print this
    -V, --version    Print the version

There are no other options. Everything zet can be configured to do is in its config
file, which it writes on first run; the key bindings, the theme, the font, and the
window are all there.";

fn main() -> ExitCode {
    match parse(std::env::args().skip(1)) {
        Args::Help => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Args::Version => {
            println!("zet {VERSION}");
            ExitCode::SUCCESS
        }
        Args::Bad(argument) => {
            eprintln!("zet: unexpected argument `{argument}`");
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
        Args::Run => run(),
    }
}

/// What the command line said.
enum Args {
    /// Start the terminal.
    Run,
    /// Print the usage.
    Help,
    /// Print the version.
    Version,
    /// Something zet does not understand.
    Bad(String),
}

/// Read the command line.
///
/// Three answers and no arguments, which is the whole surface: a terminal is configured
/// by its config file, and a flag for every setting would be a second place for the same
/// truth. An unknown argument is refused rather than ignored, because a user who typed
/// `zet --maximised` deserves to be told that zet did not do that rather than to watch a
/// window open and wonder.
fn parse(mut arguments: impl Iterator<Item = String>) -> Args {
    let Some(argument) = arguments.next() else {
        return Args::Run;
    };
    match argument.as_str() {
        "-h" | "--help" => Args::Help,
        "-V" | "--version" => Args::Version,
        _ => Args::Bad(argument),
    }
}

/// Start the terminal, and say why if it cannot.
fn run() -> ExitCode {
    let Ok(loop_) = EventLoop::<Wake>::with_user_event().build() else {
        // Failing here means the platform refused to make an event loop at all, which
        // happens on a thread that is not the main one and almost nowhere else.
        platform::alert("zet could not start", "the platform refused an event loop");
        return ExitCode::FAILURE;
    };
    // `Wait` and not `Poll`: a terminal that is not being typed into should use no power
    // at all. The loop is woken by a session's output, by the window, and by the cursor's
    // blink, and `about_to_wait` is where the last of those arms itself.
    loop_.set_control_flow(ControlFlow::Wait);

    // The waker is what crosses the gap between the thread that reads a pseudoconsole
    // and the thread that draws. `zet-session` is handed one at construction and never
    // learns where it goes.
    let waker: Arc<dyn Waker> = Arc::new(ProxyWaker::new(loop_.create_proxy()));

    let app = match App::load(waker) {
        Ok(app) => app,
        Err(error) => {
            // A machine with no shell is a machine zet cannot do anything useful on, and
            // the message says which thing was missing rather than that something was.
            platform::alert("zet could not start", &error.to_string());
            return ExitCode::FAILURE;
        }
    };

    let mut host = Host::new(app);
    match loop_.run_app(&mut host) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("zet: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Args {
        parse(list.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn no_arguments_starts_the_terminal() {
        assert!(matches!(args(&[]), Args::Run));
    }

    #[test]
    fn both_spellings_of_help_and_version_work() {
        assert!(matches!(args(&["-h"]), Args::Help));
        assert!(matches!(args(&["--help"]), Args::Help));
        assert!(matches!(args(&["-V"]), Args::Version));
        assert!(matches!(args(&["--version"]), Args::Version));
    }

    #[test]
    fn an_unknown_argument_is_refused_rather_than_ignored() {
        // The failure this prevents: a user types a flag that looks plausible, zet
        // starts anyway, and nothing they asked for happens. Saying so is the difference
        // between a terminal with no options and a terminal that is broken.
        match args(&["--maximised"]) {
            Args::Bad(argument) => assert_eq!(argument, "--maximised"),
            _ => panic!("an unknown argument should be refused"),
        }
    }

    #[test]
    fn the_usage_names_both_options_it_has() {
        assert!(USAGE.contains("--help"));
        assert!(USAGE.contains("--version"));
    }
}
