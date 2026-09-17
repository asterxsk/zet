//! Exercises the replacement against real files.
//!
//! [`zet_update::install::replace_running_exe`] is not called here. The running executable
//! of a test process is the test binary, so calling it would replace the test with the
//! fixture. The mechanism underneath it is what runs, on two ordinary files in a scratch
//! directory.
//!
//! The scratch directory is `CARGO_TARGET_TMPDIR`, which cargo places under `target/`
//! inside the repository. Nothing here writes outside the checkout.

#![cfg(windows)]

use std::path::{Path, PathBuf};

use zet_update::install;

/// The directory cargo hands out for scratch files, emptied before each test.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the scratch directory should be creatable");
    dir
}

fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, contents).expect("the file should be writable");
    path
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{path:?}: {error}"))
}

#[test]
fn the_new_binary_takes_the_place_of_the_old_one() {
    let dir = scratch("replace");
    let running = write(&dir, "zet.exe", "old build");
    let new = write(&dir, "zet.exe.new", "new build");

    install::replace_executable(&running, &new).expect("the replacement should succeed");

    assert_eq!(read(&running), "new build");
    assert_eq!(
        read(&dir.join("zet.exe.old")),
        "old build",
        "the displaced binary has to survive the swap, because the process that is running \
         it cannot be deleted until it exits"
    );
}

#[test]
fn a_leftover_backup_from_an_earlier_update_is_cleared() {
    let dir = scratch("leftover");
    let running = write(&dir, "zet.exe", "the version before last");
    // What a previous update left behind: it could not delete a file it was running from.
    write(&dir, "zet.exe.old", "an even older build");
    let new = write(&dir, "zet.exe.new", "the current build");

    install::replace_executable(&running, &new).expect("the replacement should succeed");

    assert_eq!(read(&running), "the current build");
    assert_eq!(read(&dir.join("zet.exe.old")), "the version before last");
}

#[test]
fn a_missing_download_changes_nothing() {
    let dir = scratch("missing");
    let running = write(&dir, "zet.exe", "the installed build");

    let error = install::replace_executable(&running, &dir.join("zet.exe.new"))
        .expect_err("a missing source should be refused");

    assert!(
        matches!(error, zet_update::Error::NothingToInstall(_)),
        "got {error:?}"
    );
    assert_eq!(
        read(&running),
        "the installed build",
        "a refused update must leave the installed binary exactly where it was"
    );
    assert!(
        !dir.join("zet.exe.old").exists(),
        "nothing should have been displaced"
    );
}

#[test]
fn the_staged_and_backup_paths_sit_beside_the_running_executable() {
    // Not under `%LOCALAPPDATA%`. A rename across volumes is a copy and a delete, which
    // cannot replace an open file, and the failure arrives as a bare sharing violation.
    let running = std::env::current_exe().expect("the running executable");
    for path in [
        install::staged_path().expect("a staged path"),
        install::backup_path().expect("a backup path"),
    ] {
        assert_eq!(path.parent(), running.parent(), "got {path:?}");
    }
}

#[test]
fn a_backup_is_named_so_that_it_still_looks_like_an_executable() {
    // `zet.exe` becomes `zet.exe.old`, not `zet.old`. The backup has to keep its extension:
    // it is a binary that has to stay runnable if the swap fails halfway, and a cleanup
    // step has to be able to recognise it.
    let backup = install::backup_path().expect("a backup path");
    let name = backup
        .file_name()
        .expect("a file name")
        .to_string_lossy()
        .into_owned();
    assert!(name.ends_with(".exe.old"), "got {name:?}");
}

#[test]
fn cleaning_up_is_safe_when_there_is_nothing_to_clean() {
    // Runs against the real test binary's directory. There is no backup there, and asking
    // to remove one that does not exist is an ordinary outcome rather than a failure.
    assert!(install::clean_backups() <= 1);
}

#[test]
fn the_scratch_directory_is_inside_the_repository() {
    // A guard on the test harness itself. If cargo ever moves `CARGO_TARGET_TMPDIR`
    // somewhere outside the checkout, every test above starts writing to a user profile
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
