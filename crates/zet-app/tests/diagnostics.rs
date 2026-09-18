//! The Problems section, over a real file.
//!
//! Integration rather than unit for the reason `zet-config`'s own `config_io.rs` gives:
//! these need a scratch directory, and `CARGO_TARGET_TMPDIR` — the directory cargo hands
//! out for exactly this — is only defined for integration tests. It lives under `target/`
//! inside the repository, which is the only place these may write.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use zet_app::{App, Id};
use zet_config::load;
use zet_session::NoopWaker;

/// A clean directory for one test.
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

/// An app built the way the host builds one, from a real file.
fn app(path: &Path) -> App {
    let loaded = load(path).expect("the file parses");
    App::new(
        loaded.config,
        loaded.path,
        loaded.diagnostics,
        Arc::new(NoopWaker),
    )
    .expect("this machine has a shell")
}

#[test]
fn a_problem_the_panel_fixes_stops_being_reported() {
    // The Problems section is about the file rather than in it, which is what makes it
    // worth reading and also what makes it expire: the panel writes the file on every
    // click, and a complaint about a value the user has just changed is the panel
    // reporting a problem it fixed itself, with no way to be rid of it short of a
    // restart.
    let dir = scratch("a-problem-the-panel-fixes");
    let path = write(&dir, "config.toml", "theme = \"nope\"\n");

    let mut app = app(&path);
    assert_eq!(app.diagnostics().len(), 1, "the loader complains about `nope`");

    // What a click on the theme row does: change the value, then write the file.
    app.adjust(Id::Theme, true);
    app.save().expect("the file it just read is writable");

    assert!(
        app.diagnostics().is_empty(),
        "the file no longer names a theme that does not exist: {:?}",
        app.diagnostics()
    );
    assert!(
        load(&path).expect("parses").diagnostics.is_empty(),
        "and the file on disk is the file the panel is describing"
    );
}

#[test]
fn a_problem_the_panel_did_not_fix_is_still_reported() {
    // The other half: a save is not a way to make the panel stop complaining. A key
    // naming an action zet does not have survives every write, because the file is
    // edited leaf by leaf and nothing here removes a line the user put there.
    let dir = scratch("a-problem-the-panel-does-not-fix");
    let path = write(
        &dir,
        "config.toml",
        "[keys]\nbanana = \"Ctrl+Shift+E\"\nnew-tab = \"Ctrl+Shift+T\"\n",
    );

    let mut app = app(&path);
    assert_eq!(app.diagnostics().len(), 1, "one unknown action");

    app.adjust(Id::Theme, true);
    app.save().expect("the file it just read is writable");

    assert_eq!(
        app.diagnostics().len(),
        1,
        "the binding is still not an action zet knows: {:?}",
        app.diagnostics()
    );
}
