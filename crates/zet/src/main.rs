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

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use winit::event_loop::{ControlFlow, EventLoop};

use zet_app::App;
use zet_font::FontLibrary;
use zet_session::Waker;

use crate::host::Host;
use crate::waker::{ProxyWaker, Wake};

/// What `zet --version` prints.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What `zet --help` prints.
const USAGE: &str = "\
zet — a terminal for Windows, built around the grid

USAGE:
    zet [--directory <path>]

OPTIONS:
    -d, --directory <path>    Start the shell in this directory
    -h, --help                Print this
    -V, --version             Print the version

`--directory` is not a setting and nothing else here is one either. Everything zet can
be configured to do is in its config file — the key bindings, the theme, the font, and
the window — and the settings panel (Ctrl+Shift+Comma) edits the same file in place,
comment by comment, so the two can never disagree. The directory is not a setting, it
is where you were standing when you typed the command: it is what the folder right-click
menu passes, and it applies to every tab the window opens.";

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
        Args::Bad(reason) => {
            eprintln!("zet: {reason}");
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
        Args::Run { directory } => run(directory),
    }
}

/// What the command line said.
enum Args {
    /// Start the terminal, in this directory if one was named.
    Run { directory: Option<PathBuf> },
    /// Print the usage.
    Help,
    /// Print the version.
    Version,
    /// Something zet does not understand, and why.
    Bad(String),
}

/// Read the command line.
///
/// Two answers and one argument, which is nearly the whole surface: a terminal is
/// configured by its config file, and a flag for every setting would be a second place
/// for the same truth. `--directory` earns its place because it is not a setting — it is
/// where the user was standing when they typed the command, and the folder right-click
/// menu has nowhere else to put it.
///
/// An unknown argument is refused rather than ignored, because a user who typed
/// `zet --maximised` deserves to be told that zet did not do that rather than to watch a
/// window open and wonder.
fn parse(mut arguments: impl Iterator<Item = String>) -> Args {
    let mut directory = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "-h" | "--help" => return Args::Help,
            "-V" | "--version" => return Args::Version,
            "-d" | "--directory" => {
                let Some(path) = arguments.next() else {
                    return Args::Bad(format!("`{argument}` needs a directory after it"));
                };
                directory = Some(PathBuf::from(path));
            }
            _ => return Args::Bad(format!("unexpected argument `{argument}`")),
        }
    }
    Args::Run { directory }
}

/// Start the terminal, and say why if it cannot.
fn run(directory: Option<PathBuf>) -> ExitCode {
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
        Ok(mut app) => {
            app.set_start_directory(directory);
            app
        }
        Err(error) => {
            // A machine with no shell is a machine zet cannot do anything useful on, and
            // the message says which thing was missing rather than that something was.
            platform::alert("zet could not start", &error.to_string());
            return ExitCode::FAILURE;
        }
    };

    // The settings panel offers every monospaced family the machine has, and finding out
    // which ones those are means resolving all of them — a registry walk and a face load
    // per family, which is longer than a frame and would be felt on every launch. It gets
    // its own thread, and the list arrives as a user event; a window opened before the
    // thread has finished simply has a font row that names the configured family and
    // nothing else, which is the truth until the machine has been asked.
    //
    // The library lives and dies on that thread. What crosses back is a list of names,
    // not the blobs it had to materialise to produce it.
    {
        let proxy = loop_.create_proxy();
        std::thread::spawn(move || {
            let mut library = FontLibrary::new();
            let installed = library.families().to_vec();
            let families: Vec<String> = installed
                .into_iter()
                .filter(|family| library.is_monospace(family))
                .collect();
            let _ = proxy.send_event(Wake::Families(families));
        });
    }

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

    /// The directory a `Run` carries, or a panic if it is not one.
    fn directory_of(parsed: Args) -> Option<PathBuf> {
        match parsed {
            Args::Run { directory } => directory,
            _ => panic!("expected the terminal to start"),
        }
    }

    #[test]
    fn no_arguments_starts_the_terminal() {
        assert_eq!(directory_of(args(&[])), None);
    }

    #[test]
    fn both_spellings_of_directory_are_read() {
        for flag in ["-d", "--directory"] {
            assert_eq!(
                directory_of(args(&[flag, r"D:\somewhere"])),
                Some(PathBuf::from(r"D:\somewhere")),
                "{flag}"
            );
        }
    }

    #[test]
    fn a_directory_that_is_not_there_is_still_taken_at_its_word() {
        // Nothing here touches the filesystem. The path is what the folder right-click
        // menu passed, and whether it exists is a question for the moment a shell is
        // started in it — which is where the error can say both what would not start and
        // where it was told to start.
        assert_eq!(
            directory_of(args(&["--directory", r"D:\gone"])),
            Some(PathBuf::from(r"D:\gone"))
        );
    }

    #[test]
    fn a_directory_with_nothing_after_it_says_so() {
        // Not "unexpected argument `--directory`", which is what a reader would conclude
        // means the flag does not exist.
        match args(&["--directory"]) {
            Args::Bad(reason) => assert!(reason.contains("needs a directory"), "{reason}"),
            _ => panic!("a flag with no value should be refused"),
        }
    }

    #[test]
    fn help_wins_over_a_directory_that_follows_it() {
        // `zet --help -d x` prints the usage rather than opening a window in `x`, which
        // is what every other program does and what the reader asked for by typing it.
        assert!(matches!(
            args(&["--help", "-d", r"D:\somewhere"]),
            Args::Help
        ));
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
            Args::Bad(reason) => assert_eq!(reason, "unexpected argument `--maximised`"),
            _ => panic!("an unknown argument should be refused"),
        }
    }

    #[test]
    fn the_usage_names_every_option_it_has() {
        // Kept in step by hand, so the list of flags is written down twice in one file
        // and the second copy is a test. A flag that exists but is undocumented is the
        // one a user cannot find.
        for flag in ["--help", "--version", "--directory"] {
            assert!(USAGE.contains(flag), "{flag} is missing from the usage");
        }
    }
}
