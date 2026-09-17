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

/// What `zet --help` prints.
const USAGE: &str = "\
zet — a terminal for Windows, built around the grid

USAGE:
    zet [--directory <path>]
    zet --check-update
    zet --update

OPTIONS:
    -d, --directory <path>    Start the shell in this directory
    -h, --help                Print this
    -V, --version             Print the version
        --check-update        Ask GitHub whether a newer version has been published
        --update              Download it, verify it, and put it where this binary is

`--directory` is not a setting and nothing else here is one either. Everything zet can
be configured to do is in its config file — the key bindings, the theme, the font, and
the window — and the settings panel (Ctrl+Shift+Comma) edits the same file in place,
comment by comment, so the two can never disagree. The directory is not a setting, it
is where you were standing when you typed the command: it is what the folder right-click
menu passes, and it applies to every tab the window opens.

`--check-update` and `--update` are the only things here that touch the network, and the
only flags that do not start a terminal. Nothing else in zet makes a request on its own:
the launch check that `[update] check-on-launch` is written for is not wired in yet, so
typing one of these is the only way to make zet ask. `--update` is the same check plus
the download, the digest check, and the replacement — nothing is written until the
download has matched the digest the release published, and the new version takes effect
the next time zet starts rather than now. See PRIVACY.md for what the requests disclose.";

fn main() -> ExitCode {
    match parse(std::env::args().skip(1)) {
        Args::Help => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Args::Version => {
            // The commit and the target as well as the version, because a version on its
            // own does not identify a build: two binaries can both call themselves
            // `0.1.0` and differ, and the one a bug report came from is the one that
            // matters.
            println!("{}", zet_update::version_line());
            ExitCode::SUCCESS
        }
        Args::CheckUpdate => check_update(),
        Args::Update => update(),
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
    /// Ask whether a newer version has been published.
    CheckUpdate,
    /// Download the newest version and put it where the running binary is.
    Update,
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
            "--check-update" => return Args::CheckUpdate,
            "--update" => return Args::Update,
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

/// Ask GitHub whether a newer version has been published, and say what came back.
///
/// The config file is deliberately not read. `[update] check-on-launch` is about the
/// check zet makes *on its own*, and this is not that: a user who turned the automatic
/// one off and then typed this asked for the request by name. Not reading the file also
/// means this works on a machine whose config will not parse, which is exactly when
/// someone might be running it.
///
/// A check that cannot be completed exits non-zero, so a script can tell "there is
/// nothing newer" from "there is no answer". Whether there *is* a newer version does not
/// change the exit code — this reports, it does not decide.
fn check_update() -> ExitCode {
    let checker = zet_update::Checker::new(zet_update::Http::default(), zet_update::DEFAULT_REPO);
    match checker.check() {
        Ok(found) => {
            // The version and not the whole `Update`: what to print is the one number,
            // and taking it here is what keeps the wording testable without a server —
            // which matters, because everything else in this function is a socket.
            let newer = found.as_ref().map(|update| update.version().to_string());
            print!("{}", check_report(newer.as_deref()));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("zet: {error}");
            ExitCode::FAILURE
        }
    }
}

/// What to print after a check, given the version it found, if it found one.
fn check_report(newer: Option<&str>) -> String {
    let this = zet_update::version_line();
    match newer {
        None => format!("{this} is the newest release\n"),
        Some(version) => format!(
            "zet {version} is available; you have {this}\n\
             https://github.com/{}/releases/latest\n",
            zet_update::DEFAULT_REPO,
        ),
    }
}

/// Download the newest release, verify it, and put it where the running binary is.
///
/// Unlike [`check_update`], this one does change something on disk, and what it changes is
/// the executable that is currently running. Windows will not let a running image be
/// written to or deleted but will let it be *renamed*, which is the whole mechanism: the
/// old binary is moved aside, the new one takes the name, and the displaced copy is
/// deleted on the next launch, by which point nothing is holding it. This process keeps
/// running from the image it already has, so **the new version takes effect the next time
/// zet starts**.
///
/// Typing the flag is the confirmation. Nothing is downloaded until the check has said
/// there is a newer version for this target, and nothing is written until the download has
/// matched the digest the release published — so the worst case of a build that has gone
/// wrong is a message and no file.
fn update() -> ExitCode {
    let checker = zet_update::Checker::new(zet_update::Http::default(), zet_update::DEFAULT_REPO);
    let found = match checker.check() {
        Ok(found) => found,
        Err(error) => {
            eprintln!("zet: {error}");
            return ExitCode::FAILURE;
        }
    };
    let Some(update) = found else {
        print!("{}", check_report(None));
        return ExitCode::SUCCESS;
    };

    // Reported before the download rather than after it, because it is a megabyte and a
    // half of someone else's bandwidth and a line saying what is being fetched is the
    // difference between waiting and wondering.
    println!("zet {} is available; downloading it", update.version());
    match update.install(&zet_update::Http::default()) {
        Ok(_) => {
            print!("{}", install_report(&update.version().to_string()));
            ExitCode::SUCCESS
        }
        Err(error) => {
            // The old binary is still in place unless the error says otherwise: the
            // install moves it aside only after the new one is verified, and puts it back
            // if the second move fails.
            eprintln!("zet: {error}");
            ExitCode::FAILURE
        }
    }
}

/// What to print once a new binary is in place.
///
/// The path is printed because this is the one operation in the program that rewrites a
/// file the user owns, and where it landed is the first thing anyone would want to check.
fn install_report(version: &str) -> String {
    let at = std::env::current_exe().map_or_else(
        |_| "where zet is".to_owned(),
        |path| path.display().to_string(),
    );
    format!("zet {version} is installed at {at}; it takes effect the next time zet starts\n")
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
    fn check_update_is_its_own_flag_and_starts_nothing() {
        // It has no short form on purpose: `-c` reads like `--config` or `--continue`,
        // and this is the one flag here that makes a network request.
        assert!(matches!(args(&["--check-update"]), Args::CheckUpdate));
        assert!(matches!(args(&["--update"]), Args::Update));
        assert!(matches!(args(&["-c"]), Args::Bad(_)));
    }

    #[test]
    fn installing_says_where_it_put_the_binary() {
        // The path is the point of this line. An update rewrites a file the user owns, and
        // "where did it go" is the first question anyone would ask of a command that did.
        let line = install_report("9.9.9");
        assert!(line.contains("zet 9.9.9 is installed at"), "{line}");
        assert!(line.contains("next time zet starts"), "{line}");
        assert_eq!(line.lines().count(), 1);
    }

    #[test]
    fn a_check_that_finds_nothing_says_which_build_is_current() {
        // The build and not just the version, for the same reason `--version` names the
        // commit: `0.1.0` is a range of binaries and someone reading this needs to know
        // which one they are on.
        let line = check_report(None);
        assert_eq!(line.lines().count(), 1);
        assert!(line.contains(&zet_update::version_line()), "{line}");
        assert!(line.ends_with("newest release\n"), "{line}");
    }

    #[test]
    fn a_check_that_finds_something_names_it_and_says_where_to_get_it() {
        let line = check_report(Some("9.9.9"));
        assert!(line.contains("zet 9.9.9 is available"), "{line}");
        assert!(line.contains(&zet_update::version_line()), "{line}");
        assert!(
            line.contains("github.com/asterxsk/zet/releases/latest"),
            "{line}"
        );
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
    fn the_version_line_identifies_a_build_and_not_just_a_release() {
        // A version on its own does not identify a build, and the reason this is asserted
        // here rather than left to `zet-update`'s own test is that the wiring is the part
        // that was missing: `version_line` existed, was tested, and nothing called it.
        let line = zet_update::version_line();
        assert!(line.starts_with("zet "), "{line}");
        assert!(line.contains(env!("CARGO_PKG_VERSION")), "{line}");
        assert!(line.contains(zet_update::TARGET), "{line}");
        assert_eq!(
            line.lines().count(),
            1,
            "a --version that wraps gets truncated"
        );
    }

    #[test]
    fn the_usage_names_every_option_it_has() {
        // Kept in step by hand, so the list of flags is written down twice in one file
        // and the second copy is a test. A flag that exists but is undocumented is the
        // one a user cannot find.
        for flag in [
            "--help",
            "--version",
            "--directory",
            "--check-update",
            "--update",
        ] {
            assert!(USAGE.contains(flag), "{flag} is missing from the usage");
        }
    }
}
