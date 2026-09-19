//! Finding the shells this machine has.
//!
//! The brief asks for the thing Windows Terminal does: a list of what is installed, and
//! the user picks. That list has to be built by looking rather than by guessing, because
//! there is no registry key or well-known location that enumerates a machine's shells.
//! Four sources are consulted, in the order a user would expect to see them:
//!
//! 1. The well-known absolute locations. `cmd.exe` and Windows PowerShell are always in
//!    `%SystemRoot%`, and pwsh 7 is always under `%ProgramFiles%`. These are found even
//!    on a machine whose `PATH` has been mangled, which is common.
//! 2. `PATH`, honouring `PATHEXT`. This is what catches an unusual install location,
//!    Chocolatey, Scoop, or a user's own build.
//! 3. The Windows app execution aliases under `%LOCALAPPDATA%\Microsoft\WindowsApps`.
//!    These are consulted last because a stale alias is a dead end: the entry exists but
//!    launching it opens the Store.
//! 4. The WSL distributions, which are enumerated by asking `wsl.exe`.
//!
//! Nothing here reads the registry. App Paths would add a few entries that `PATH` and
//! the well-known locations already cover, and it is not worth a dependency to catch a
//! case that does not exist on a stock machine.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where a profile was found, which decides how much it can be trusted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// A location that is correct on every Windows install.
    KnownLocation,
    /// Found by walking `PATH`.
    Path,
    /// A Windows app execution alias. It may point at nothing.
    AppAlias,
    /// A distribution reported by `wsl.exe`.
    Wsl,
}

/// One shell the user can open a tab with.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Profile {
    /// A stable identifier, safe to write into a config file. Never translated, never
    /// reordered, and it must not change when the display name does.
    pub id: String,
    /// What to show in the profile picker.
    pub name: String,
    /// The executable.
    pub program: PathBuf,
    /// Arguments that go before the user's.
    pub args: Vec<OsString>,
    /// Where it came from.
    pub source: Source,
}

impl Profile {
    /// A profile with no arguments.
    fn new(id: &str, name: &str, program: PathBuf, source: Source) -> Self {
        Profile {
            id: id.to_owned(),
            name: name.to_owned(),
            program,
            args: Vec::new(),
            source,
        }
    }

    /// The same profile with starting arguments.
    fn with_args(mut self, args: impl IntoIterator<Item = impl Into<OsString>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }
}

/// Every shell found on this machine, in the order the picker should show them.
///
/// The first entry is the one a new tab opens with when the user has expressed no
/// preference. On this platform that is PowerShell 7 if it is installed, because it is
/// what a terminal's own users install first, and Windows PowerShell or Command Prompt
/// otherwise, because those are the ones guaranteed to be there. [`order`] is what decides
/// that, so that it is a fact about the shells rather than about which of the four sources
/// below happened to reach one first.
pub fn discover() -> Vec<Profile> {
    let mut found = Vec::new();
    let system_root =
        std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    let system32 = system_root.join("System32");

    // 1. The locations that do not depend on PATH.
    let pwsh_known = program_files().map(|dir| dir.join(r"PowerShell\7\pwsh.exe"));
    if let Some(profile) = pwsh_known
        .filter(|path| path.is_file())
        .map(|path| Profile::new("pwsh", "PowerShell 7", path, Source::KnownLocation))
    {
        found.push(profile);
    }

    let windows_powershell = system32.join(r"WindowsPowerShell\v1.0\powershell.exe");
    if windows_powershell.is_file() {
        found.push(Profile::new(
            "powershell",
            "Windows PowerShell",
            windows_powershell,
            Source::KnownLocation,
        ));
    }

    let cmd = system32.join("cmd.exe");
    if cmd.is_file() {
        found.push(Profile::new(
            "cmd",
            "Command Prompt",
            cmd,
            Source::KnownLocation,
        ));
    }

    // 2. PATH, for installs in places the well-known list does not know about.
    let path_entries = [
        ("pwsh", "PowerShell 7", "pwsh.exe"),
        ("nu", "Nushell", "nu.exe"),
        ("bash", "Bash", "bash.exe"),
    ];
    for (id, name, executable) in path_entries {
        if !found.iter().any(|profile| profile.id == id)
            && let Some(program) = find_on_path(executable)
        {
            found.push(Profile::new(id, name, program, Source::Path));
        }
    }

    if let Some(bash) = find_git_bash() {
        found.push(Profile::new("git-bash", "Git Bash", bash, Source::Path));
    }

    // WSL goes before the app aliases. A distribution is a real shell; an alias is a
    // shortcut that may lead nowhere.
    found.extend(wsl_profiles());

    // 3. App execution aliases, last.
    if let Some(aliases) = app_aliases_dir() {
        for (id, name, executable) in [
            ("pwsh-alias", "PowerShell 7 (Store)", "pwsh.exe"),
            ("wsl-alias", "WSL", "wsl.exe"),
        ] {
            let candidate = aliases.join(executable);
            if candidate.is_file() && !found.iter().any(|profile| profile.program == candidate) {
                found.push(Profile::new(id, name, candidate, Source::AppAlias));
            }
        }
    }

    let mut kept = dedupe(found);
    order(&mut kept);
    kept
}

/// Put a found set of profiles into the order the picker shows them.
///
/// The one rule with a reason of its own is the top of the list, because the first entry is
/// the shell a new tab opens. PowerShell 7 leads *wherever it was found* — the well-known
/// location, `PATH`, a package manager's directory — since which of those a machine's
/// install used is not something the user chose, and the sources below reach it in an order
/// that has nothing to do with which shell it is. Windows PowerShell and `cmd.exe` follow,
/// because both are certain to be present and neither is what someone who installed pwsh 7
/// wants by default. Everything else is left in the order the sources found it.
///
/// The Store alias is deliberately not promoted with `pwsh`: it is a shortcut that opens
/// the Store when the app behind it is not installed, and ranking it above two shells that
/// are known to work would be ranking a maybe above two certainties.
fn order(profiles: &mut [Profile]) {
    profiles.sort_by_key(rank);
}

/// How early a profile is shown, lowest first. See [`order`] for what the ranks mean.
fn rank(profile: &Profile) -> u8 {
    match profile.id.as_str() {
        "pwsh" => 0,
        "powershell" => 1,
        "cmd" => 2,
        _ => 3,
    }
}

/// Drop profiles that would launch the same thing twice.
///
/// `PATH` frequently contains the same directory more than once, and pwsh 7 is regularly
/// both under `%ProgramFiles%` and on `PATH`. Showing it twice in the picker looks like a
/// bug because it is one.
fn dedupe(profiles: Vec<Profile>) -> Vec<Profile> {
    let mut kept: Vec<Profile> = Vec::with_capacity(profiles.len());
    for profile in profiles {
        let duplicate = kept.iter().any(|existing| {
            existing.args == profile.args
                && same_file::equivalently(&existing.program, &profile.program)
        });
        if !duplicate {
            kept.push(profile);
        }
    }
    kept
}

/// Path comparison that does not care about case or a trailing separator.
mod same_file {
    use std::path::Path;

    /// Whether two paths name the same file on a case-insensitive filesystem.
    pub(super) fn equivalently(left: &Path, right: &Path) -> bool {
        if left == right {
            return true;
        }
        let normalise = |path: &Path| {
            path.to_string_lossy()
                .trim_end_matches(['\\', '/'])
                .to_lowercase()
        };
        normalise(left) == normalise(right)
    }
}

/// The first executable named `name` on `PATH`, honouring `PATHEXT`.
#[must_use]
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let extensions = search_extensions(name);
    std::env::split_paths(&path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .find_map(|dir| {
            extensions
                .iter()
                .map(|extension| dir.join(format!("{name}{extension}")))
                .find(|candidate| candidate.is_file())
        })
}

/// The file names to try for `name`, in order.
///
/// A name that already carries an extension is tried as written. Otherwise it is tried
/// bare first and then with each of `PATHEXT`, because `createprocess` does the same and
/// a `.exe` appended to something that is already a valid file name changes what runs.
fn search_extensions(name: &str) -> Vec<String> {
    if Path::new(name).extension().is_some() {
        return vec![String::new()];
    }
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
    let mut extensions = vec![String::new()];
    extensions.extend(
        pathext
            .split(';')
            .filter(|part| !part.is_empty())
            .map(str::to_lowercase),
    );
    extensions
}

/// `%ProgramFiles%`, falling back to the path that is correct on every install.
fn program_files() -> Option<PathBuf> {
    std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from(r"C:\Program Files")))
}

/// The directory holding the Windows app execution aliases.
fn app_aliases_dir() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(local).join(r"Microsoft\WindowsApps"))
}

/// Git for Windows, wherever it was installed.
///
/// The installer offers both a per-user and a per-machine location and does not put
/// `bash.exe` on `PATH` unless asked, so both are checked and then `PATH` is tried.
fn find_git_bash() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = program_files() {
        candidates.push(dir.join(r"Git\bin\bash.exe"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join(r"Programs\Git\bin\bash.exe"));
    }
    if let Some(found) = candidates.into_iter().find(|path| path.is_file()) {
        return Some(found);
    }
    find_on_path("bash.exe").filter(|path| {
        // Git Bash is the one that ships next to a `usr\bin`, which distinguishes it
        // from an MSYS2 or Cygwin bash on the same PATH.
        path.parent()
            .and_then(Path::parent)
            .is_some_and(|root| root.join("usr").is_dir())
    })
}

/// Every WSL distribution, as a profile that opens a shell in it.
///
/// `wsl.exe --list --verbose` writes UTF-16, not UTF-8, and does not say so. Reading it
/// as UTF-8 produces a string with a null between every character, which parses into
/// exactly one nonsense distribution.
fn wsl_profiles() -> Vec<Profile> {
    let wsl = find_on_path("wsl.exe")
        .or_else(|| find_on_path("wsl"))
        .or_else(|| {
            let system = std::env::var_os("SystemRoot")?;
            let candidate = PathBuf::from(system).join(r"System32\wsl.exe");
            candidate.is_file().then_some(candidate)
        });
    let Some(wsl) = wsl else {
        return Vec::new();
    };

    let Ok(output) = Command::new(&wsl)
        .args(["--list", "--verbose"])
        .creation_flags_no_window()
        .output()
    else {
        return Vec::new();
    };
    // A machine with WSL installed but no distributions reports this on stderr and
    // nothing on stdout, which is not an error worth surfacing.
    if !output.status.success() && output.stdout.is_empty() {
        return Vec::new();
    }

    parse_wsl_list(&output.stdout)
        .into_iter()
        .map(|distro| {
            Profile::new(
                &format!("wsl:{}", distro.name),
                &format!("{} (WSL)", distro.name),
                wsl.clone(),
                Source::Wsl,
            )
            .with_args(["-d", &distro.name])
        })
        .collect()
}

/// One line of `wsl --list --verbose`.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Distro {
    name: String,
    /// Whether this is the distribution WSL opens by default.
    default: bool,
}

/// Parse `wsl.exe --list --verbose` output, whatever encoding it arrived in.
fn parse_wsl_list(bytes: &[u8]) -> Vec<Distro> {
    let text = decode_console_output(bytes);
    let mut distros = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // The header is the one line that is entirely column names.
        let first = trimmed.trim_start_matches('*').trim_start();
        if first.starts_with("NAME") {
            continue;
        }
        let default = trimmed.starts_with('*');
        // Columns are whitespace separated: name, state, version. A name cannot contain
        // whitespace, so splitting is enough and the rest of the line is discarded.
        let Some(name) = first.split_whitespace().next() else {
            continue;
        };
        // A distribution WSL cannot start is still listed, with no state column.
        if name.is_empty() {
            continue;
        }
        distros.push(Distro {
            name: name.to_owned(),
            default,
        });
    }
    distros
}

/// Decode output that may be UTF-16 or UTF-8.
///
/// `wsl.exe` writes UTF-16LE. Other tools on the same code path do not. Deciding by
/// looking at the bytes rather than by which command produced them means this keeps
/// working if that ever changes.
fn decode_console_output(bytes: &[u8]) -> String {
    let looks_utf16 = bytes.len() >= 4
        && bytes.len().is_multiple_of(2)
        // Every second byte of ASCII text in UTF-16 is a null, and enough of them in a row
        // is not something UTF-8 text produces.
        && bytes
            .as_chunks::<2>()
            .0
            .iter()
            .take(64)
            .filter(|pair| pair[1] == 0)
            .count()
            > 16;
    if looks_utf16 {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// Keep a console window from flashing when zet shells out to ask a question.
trait NoWindow {
    /// Run the command without a console window.
    fn creation_flags_no_window(&mut self) -> &mut Self;
}

impl NoWindow for Command {
    fn creation_flags_no_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        /// `CREATE_NO_WINDOW`. Not in `std`, and only ever used here.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        self.creation_flags(CREATE_NO_WINDOW)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16le(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn a_name_with_an_extension_is_not_expanded() {
        assert_eq!(search_extensions("cmd.exe"), vec![String::new()]);
    }

    #[test]
    fn a_bare_name_is_tried_with_pathext() {
        let extensions = search_extensions("pwsh");
        assert_eq!(extensions[0], "", "the bare name comes first");
        assert!(extensions.len() > 1);
        assert!(extensions.iter().all(|e| e == &e.to_lowercase()));
    }

    #[test]
    fn wsl_output_is_decoded_from_utf16() {
        let listing = "  NAME            STATE           VERSION\n* Ubuntu-22.04    Running         2\n  Debian          Stopped         2\n";
        let distros = parse_wsl_list(utf16le(listing).as_slice());
        assert_eq!(
            distros,
            vec![
                Distro {
                    name: "Ubuntu-22.04".into(),
                    default: true
                },
                Distro {
                    name: "Debian".into(),
                    default: false
                },
            ]
        );
    }

    #[test]
    fn wsl_output_that_arrives_as_utf8_still_parses() {
        // Some builds write UTF-8, and the decoder has to notice rather than produce a
        // string with a null between every character.
        let listing = "NAME  STATE  VERSION\nAlpine  Stopped  2\n";
        let distros = parse_wsl_list(listing.as_bytes());
        assert_eq!(distros.len(), 1);
        assert_eq!(distros[0].name, "Alpine");
    }

    #[test]
    fn the_wsl_header_is_not_a_distribution() {
        let distros = parse_wsl_list(utf16le("NAME STATE VERSION\n").as_slice());
        assert!(distros.is_empty());
    }

    #[test]
    fn an_empty_wsl_listing_produces_nothing() {
        assert!(parse_wsl_list(&[]).is_empty());
        assert!(parse_wsl_list(utf16le("").as_slice()).is_empty());
    }

    #[test]
    fn the_default_marker_is_read_from_the_first_column() {
        let distros = parse_wsl_list(utf16le("* Kali  Running  2\n").as_slice());
        assert_eq!(distros[0].name, "Kali");
        assert!(distros[0].default);
    }

    #[test]
    fn a_distribution_name_is_taken_whole_up_to_the_first_space() {
        let distros = parse_wsl_list(utf16le("Ubuntu-24.04  Stopped  2\n").as_slice());
        assert_eq!(distros[0].name, "Ubuntu-24.04");
    }

    #[test]
    fn a_path_with_a_different_case_or_a_trailing_separator_is_the_same_file() {
        assert!(same_file::equivalently(
            Path::new(r"C:\Windows\System32\cmd.exe"),
            Path::new(r"c:\windows\system32\CMD.EXE")
        ));
        assert!(same_file::equivalently(
            Path::new(r"C:\Windows"),
            Path::new(r"C:\Windows\")
        ));
        assert!(!same_file::equivalently(
            Path::new(r"C:\Windows\System32\cmd.exe"),
            Path::new(r"C:\Windows\System32\wsl.exe")
        ));
    }

    #[test]
    fn duplicates_are_dropped_but_different_arguments_are_not() {
        let cmd = PathBuf::from(r"C:\Windows\System32\cmd.exe");
        let profiles = vec![
            Profile::new("cmd", "Command Prompt", cmd.clone(), Source::KnownLocation),
            Profile::new("cmd-again", "Command Prompt", cmd.clone(), Source::Path),
            Profile::new(
                "wsl:Ubuntu",
                "Ubuntu (WSL)",
                PathBuf::from("wsl.exe"),
                Source::Wsl,
            )
            .with_args(["-d", "Ubuntu"]),
            Profile::new(
                "wsl:Debian",
                "Debian (WSL)",
                PathBuf::from("wsl.exe"),
                Source::Wsl,
            )
            .with_args(["-d", "Debian"]),
        ];
        let kept = dedupe(profiles);
        assert_eq!(kept.len(), 3, "the second cmd.exe is the same shell");
        assert_eq!(
            kept[2].args,
            vec![OsString::from("-d"), OsString::from("Debian")]
        );
    }

    #[test]
    fn a_real_machine_always_has_a_command_prompt() {
        // `cmd.exe` ships with Windows. If this fails, the discovery order is broken in a
        // way that would leave a fresh install with no shells at all.
        let profiles = discover();
        assert!(
            profiles.iter().any(|profile| profile.id == "cmd"),
            "found {profiles:#?}"
        );
        assert!(
            std::env::var_os("SystemRoot").is_some(),
            "SystemRoot is set on every Windows install"
        );
    }

    #[test]
    fn every_discovered_profile_can_be_launched() {
        // An alias that points at nothing is the failure this guards against: the entry
        // exists in the picker and opening it does nothing.
        for profile in discover() {
            assert!(
                profile.program.is_file(),
                "{} points at {:?}, which is not a file",
                profile.id,
                profile.program
            );
            assert!(!profile.id.is_empty());
            assert!(!profile.name.is_empty());
        }
    }

    #[test]
    fn powershell_7_is_the_shell_a_new_tab_opens_whichever_source_found_it() {
        // A machine whose pwsh is not at the well-known location — Scoop, a user-scope
        // install, a package manager's directory — used to open Windows PowerShell 5.1 in
        // a new tab, because the `PATH` search ran after the fallbacks and the first entry
        // is the one a new tab opens. Which source reached the shell first is not something
        // the user chose; which shell it is, is.
        let mut found = vec![
            Profile::new(
                "powershell",
                "Windows PowerShell",
                PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"),
                Source::KnownLocation,
            ),
            Profile::new(
                "cmd",
                "Command Prompt",
                PathBuf::from(r"C:\Windows\System32\cmd.exe"),
                Source::KnownLocation,
            ),
            Profile::new(
                "pwsh",
                "PowerShell 7",
                PathBuf::from(r"C:\Users\someone\scoop\shims\pwsh.exe"),
                Source::Path,
            ),
            Profile::new(
                "nu",
                "Nushell",
                PathBuf::from(r"C:\bin\nu.exe"),
                Source::Path,
            ),
        ];
        order(&mut found);
        assert_eq!(
            found
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            vec!["pwsh", "powershell", "cmd", "nu"],
            "a new tab opens the first profile"
        );
    }

    #[test]
    fn everything_that_is_not_one_of_the_three_shells_keeps_the_order_it_was_found_in() {
        // The rank is about which shell a new tab opens, and past `cmd` there is nothing to
        // rank: the sources are consulted in the order a user would expect to see them, and
        // a sort that reshuffled them would be a second opinion about that order.
        let mut found = vec![
            Profile::new(
                "nu",
                "Nushell",
                PathBuf::from(r"C:\bin\nu.exe"),
                Source::Path,
            ),
            Profile::new(
                "git-bash",
                "Git Bash",
                PathBuf::from(r"C:\bin\bash.exe"),
                Source::Path,
            ),
            Profile::new(
                "wsl:Ubuntu",
                "Ubuntu (WSL)",
                PathBuf::from(r"C:\bin\wsl.exe"),
                Source::Wsl,
            ),
            Profile::new(
                "pwsh-alias",
                "PowerShell 7 (Store)",
                PathBuf::from(r"C:\alias\pwsh.exe"),
                Source::AppAlias,
            ),
        ];
        order(&mut found);
        assert_eq!(
            found
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            vec!["nu", "git-bash", "wsl:Ubuntu", "pwsh-alias"]
        );
    }

    #[test]
    fn a_machine_with_powershell_7_opens_a_tab_with_it() {
        // The machine-level half of the same rule: whatever `discover` found, if it found
        // PowerShell 7 then that is what a new tab opens. The profile is looked for by id
        // and not by position, because where discover put it is the thing under test.
        let profiles = discover();
        if profiles.iter().any(|profile| profile.id == "pwsh") {
            assert_eq!(
                profiles[0].id,
                "pwsh",
                "found {:#?}",
                profiles
                    .iter()
                    .map(|profile| &profile.id)
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn discovered_ids_are_unique() {
        let profiles = discover();
        let mut ids: Vec<&str> = profiles.iter().map(|profile| profile.id.as_str()).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(
            ids.len(),
            before,
            "a duplicate id would collide in the config file"
        );
    }
}
