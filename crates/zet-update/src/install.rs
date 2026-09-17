//! Putting a new binary where the running one is.
//!
//! Windows will not let a running executable be written to or deleted. It will let one be
//! *renamed*, because the loader maps an image with `FILE_SHARE_DELETE`, and that is the
//! whole trick: move the running binary aside, move the new one into the name it vacated,
//! and delete the displaced copy on the next launch — by which point nothing is holding it.

use std::path::{Path, PathBuf};

use crate::Error;

/// The suffix given to a binary that has been displaced by an update.
const BACKUP_SUFFIX: &str = ".old";

/// Where a downloaded binary is staged before it is put in place.
///
/// It has to be beside the running executable and not in a cache directory. A rename across
/// volumes is a copy followed by a delete, which is neither atomic nor able to replace an
/// open file, and that failure arrives as a sharing violation with no useful message.
///
/// # Errors
///
/// Fails if the running executable's path cannot be read.
pub fn staged_path() -> Result<PathBuf, Error> {
    let running = std::env::current_exe()?;
    Ok(sibling(&running, ".new"))
}

/// The path a displaced binary is moved to.
///
/// # Errors
///
/// Fails if the running executable's path cannot be read.
pub fn backup_path() -> Result<PathBuf, Error> {
    let running = std::env::current_exe()?;
    Ok(sibling(&running, BACKUP_SUFFIX))
}

/// `path` with `suffix` appended to its whole file name, extension included.
///
/// `with_extension` would replace `.exe` rather than extend it, turning `zet.exe` into
/// `zet.old` — a name nothing would think to clean up, and one that no longer looks like
/// an executable to anything looking for one.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Move `new` over `running`, displacing whatever was there.
///
/// This is public because it is the part worth testing and [`replace_running_exe`] cannot
/// be tested at all: the running executable of a test process is the test binary, so a
/// test that calls it replaces itself with the fixture. Everything except the choice of
/// which file is the running one lives here.
///
/// # Errors
///
/// Fails if the directory cannot be written to, which is what an installation under
/// `Program Files` looks like without elevation.
pub fn replace_executable(running: &Path, new: &Path) -> Result<(), Error> {
    if !new.is_file() {
        return Err(Error::NothingToInstall(new.to_path_buf()));
    }
    let backup = sibling(running, BACKUP_SUFFIX);

    // A leftover from an earlier update. It is not running, so it can go.
    match std::fs::remove_file(&backup) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(Error::Io(error)),
    }

    std::fs::rename(running, &backup).map_err(|error| map_write_error(error, running))?;

    // From here the original is at `backup` and the install path is empty. If the second
    // move fails, put it back rather than leaving a user with no executable at all.
    if let Err(error) = std::fs::rename(new, running) {
        let restored = std::fs::rename(&backup, running);
        return Err(match restored {
            Ok(()) => map_write_error(error, running),
            Err(restore) => Error::Stranded {
                backup: backup.clone(),
                reason: format!("{error}; restoring the original also failed: {restore}"),
            },
        });
    }
    Ok(())
}

/// Replace the running executable with the file at `new`.
///
/// The update takes effect the next time zet starts. This process keeps running from the
/// image it already has, which is why the displaced copy cannot be deleted yet.
///
/// # Errors
///
/// Fails if the running executable's path cannot be read, or the replacement cannot be
/// written.
pub fn replace_running_exe(new: &Path) -> Result<(), Error> {
    replace_executable(&std::env::current_exe()?, new)
}

/// Delete the displaced binary from a previous update, if one is still there.
///
/// Returns the number removed. Called at startup: the file cannot be deleted by the update
/// that created it, because that update was running from it.
pub fn clean_backups() -> usize {
    let Ok(backup) = backup_path() else {
        return 0;
    };
    // A failure here is not worth reporting. The file is a leftover copy of a binary that
    // has already been replaced, and the only cost of keeping it is some disk.
    usize::from(std::fs::remove_file(backup).is_ok())
}

/// A write failure that means the directory is not ours to write.
fn map_write_error(error: std::io::Error, path: &Path) -> Error {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => Error::NotWritable(path.to_path_buf()),
        _ => Error::Io(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sibling_extends_the_name_rather_than_replacing_the_extension() {
        // `zet.exe` must become `zet.exe.old`, not `zet.old`: the backup still has to look
        // like the executable it is, and a cleanup step looking for `*.exe.old` has to be
        // able to find it.
        assert_eq!(
            sibling(Path::new(r"C:\apps\zet.exe"), ".old"),
            PathBuf::from(r"C:\apps\zet.exe.old")
        );
        assert_eq!(
            sibling(Path::new(r"C:\apps\zet.exe"), ".new"),
            PathBuf::from(r"C:\apps\zet.exe.new")
        );
    }

    #[test]
    fn a_sibling_of_a_name_without_an_extension_still_works() {
        assert_eq!(sibling(Path::new("zet"), ".old"), PathBuf::from("zet.old"));
    }

    #[test]
    fn a_missing_source_is_reported_before_anything_is_moved() {
        let missing = std::path::Path::new("no-such-file-anywhere.exe");
        assert!(matches!(
            replace_executable(Path::new("also-missing.exe"), missing),
            Err(Error::NothingToInstall(_))
        ));
    }

    #[test]
    fn the_displaced_binary_is_removed_once_and_only_once() {
        // The cleanup an update cannot do for itself. The file this makes is beside the
        // test binary, which is inside `target/` and is a sibling of nothing that matters:
        // the point is only that a leftover copy is there one moment and gone the next.
        let backup = backup_path().expect("the test binary has a path");
        std::fs::write(&backup, b"").expect("target should be writable");

        assert_eq!(clean_backups(), 1, "the leftover was there to be removed");
        assert!(!backup.exists(), "it is still there");
        assert_eq!(clean_backups(), 0, "a second sweep found nothing");
    }
}
