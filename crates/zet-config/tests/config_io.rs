//! The parts of the configuration that touch a real file.
//!
//! These are integration tests rather than unit tests for one reason: they need a
//! scratch directory, and `CARGO_TARGET_TMPDIR` — the directory cargo hands out for
//! exactly this — is only defined for integration tests. It lives under `target/`
//! inside the repository, which is the only place these may write. The last test in
//! this file is the assertion that keeps that true.

use std::path::{Path, PathBuf};

use zet_config::{Background, Config, CursorShape, TabPosition, load, save};

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

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn a_config_written_out_reads_back_identically() {
    let dir = scratch("round-trip");
    let path = dir.join("config.toml");

    let mut config = Config {
        theme: "gruvbox-dark".to_owned(),
        ..Config::default()
    };
    config.cursor.shape = CursorShape::Bar;
    config.tabs.position = TabPosition::Left;
    config.font.size = 15.0;
    config.keys.insert("find".to_owned(), "Ctrl+F".to_owned());

    save(&config, &path).expect("saves");
    let back = load(&path).expect("loads");

    assert!(back.diagnostics.is_empty(), "{:?}", back.diagnostics);
    assert_eq!(back.config, config);
    assert!(back.existed);
}

#[test]
fn every_default_setting_survives_being_written_and_read() {
    // The list of keys the app can change is the list of keys that has to round-trip.
    // A default that does not is a setting that silently resets on restart.
    let dir = scratch("defaults");
    let path = dir.join("config.toml");
    save(&Config::default(), &path).expect("saves");
    let back = load(&path).expect("loads");
    assert!(back.diagnostics.is_empty(), "{:?}", back.diagnostics);
    assert_eq!(back.config, Config::default());
}

#[test]
fn saving_over_a_hand_written_file_keeps_its_comments() {
    // PRODUCT.md's sixth success criterion. The file a user has annotated is the one
    // they care about, and a settings-panel edit that strips the annotations is a
    // settings-panel edit they will stop making.
    let dir = scratch("comments");
    let path = write(
        &dir,
        "config.toml",
        "\
# zet, configured by hand.
theme = \"nord\" # the blue one

[font]
# I like it big
size = 13.0
",
    );

    let mut config = load(&path).expect("loads").config;
    config.font.size = 16.0;
    save(&config, &path).expect("saves");

    let written = read(&path);
    assert!(
        written.contains("# zet, configured by hand."),
        "the leading comment was lost:\n{written}"
    );
    assert!(
        written.contains("# the blue one"),
        "the trailing comment was lost:\n{written}"
    );
    assert!(
        written.contains("# I like it big"),
        "a comment above a value was lost:\n{written}"
    );
    assert!(
        written.contains("16.0"),
        "the edit was not written:\n{written}"
    );
    assert!(
        written.contains("theme = \"nord\""),
        "an untouched key changed:\n{written}"
    );
}

#[test]
fn saving_twice_writes_the_same_bytes_the_second_time() {
    // Idempotence, which is the property that actually matters. A save writes out every
    // key, so the first one on a hand-written three-line file produces a complete file
    // — that is deliberate, and it is how a user discovers what else there is to set.
    // What must not happen is the file drifting a little on every save, because that
    // turns a config file under version control into noise.
    let dir = scratch("idempotent");
    let path = write(
        &dir,
        "config.toml",
        "# a comment\ntheme = \"nord\"\n\n[font]\nsize = 15.0\n",
    );
    let config = load(&path).expect("loads").config;

    save(&config, &path).expect("saves");
    let first = read(&path);
    save(&config, &path).expect("saves again");
    let second = read(&path);

    assert_eq!(first, second, "a second save changed the file");
    assert!(first.contains("# a comment"), "the comment was lost");
    assert!(first.contains("size = 15.0"), "the user's value was lost");
}

#[test]
fn a_binding_the_config_no_longer_has_is_removed_from_the_file() {
    // Capturing a chord in the settings panel takes the key away from the action that
    // held it, so the map the panel saves is missing an entry the file still has. A save
    // that only copies forward leaves both actions holding the key. On the next load the
    // first in sort order wins, and the action the user displaced is back — holding the
    // chord they meant to give away.
    let dir = scratch("pruned-key");
    let path = write(
        &dir,
        "config.toml",
        "\
[keys]
close-tab = \"Ctrl+Shift+W\"
find = \"Ctrl+F\"
new-tab = \"Ctrl+Shift+T\"
",
    );

    let mut config = load(&path).expect("loads").config;
    config.keys.remove("close-tab");
    config
        .keys
        .insert("find".to_owned(), "Ctrl+Shift+W".to_owned());
    save(&config, &path).expect("saves");

    let written = read(&path);
    assert!(
        !written.contains("close-tab"),
        "a binding the config no longer has was left in the file:\n{written}"
    );
    let back = load(&path).expect("loads").config;
    assert_eq!(back, config, "the file says something the config does not");
    assert_eq!(
        back.keys.get("find").map(String::as_str),
        Some("Ctrl+Shift+W")
    );
    assert_eq!(
        back.keys.get("new-tab").map(String::as_str),
        Some("Ctrl+Shift+T"),
        "an untouched binding was lost"
    );
}

#[test]
fn a_comment_inside_a_value_survives_a_save_that_did_not_touch_it() {
    // A setting the save did not change must come out exactly as it went in. An array
    // with a comment between two of its elements renders differently from one without,
    // so a merge that decided "changed" by comparing renderings would replace the array
    // and take the annotation with it.
    let dir = scratch("inner-comment");
    let path = write(
        &dir,
        "config.toml",
        "\
theme = \"nord\"

[font]
fallback = [
    \"Consolas\", # my fallback
    \"Segoe UI Emoji\",
]
size = 15.0
",
    );

    let mut config = load(&path).expect("loads").config;
    config.theme = "gruvbox-dark".to_owned();
    save(&config, &path).expect("saves");

    let written = read(&path);
    assert!(
        written.contains("# my fallback"),
        "a comment inside an array was lost:\n{written}"
    );
    assert!(
        written.contains("\"Segoe UI Emoji\""),
        "an element of the array was lost:\n{written}"
    );
    assert!(
        written.contains("theme = \"gruvbox-dark\""),
        "the edit was not written:\n{written}"
    );
    assert_eq!(load(&path).expect("loads").config, config);
}

#[test]
fn a_comment_on_an_inline_table_survives_being_written_out_as_a_section() {
    // The writer spells this setting as a `[window.background]` section where the file
    // had it inline, so the setting changes shape on the way out. The comment is not
    // part of the setting and has to come across with it.
    let dir = scratch("inline-comment");
    let path = write(
        &dir,
        "config.toml",
        "\
[window]
background = { kind = \"gradient\", from = \"#0a0b0d\", to = \"#1a1030\", angle = 90.0 } # my background
",
    );

    let mut config = load(&path).expect("loads").config;
    config.font.size = 17.0;
    save(&config, &path).expect("saves");

    let written = read(&path);
    assert!(
        written.contains("# my background"),
        "the comment on the inline table was lost:\n{written}"
    );
    let back = load(&path).expect("loads");
    assert!(back.diagnostics.is_empty(), "{:?}", back.diagnostics);
    assert_eq!(back.config, config);
}

#[test]
fn a_key_the_background_does_not_have_is_reported_before_a_save_can_drop_it() {
    // `Background` is an internally tagged enum, and serde does not carry
    // `deny_unknown_fields` across one. That made it the single table in the schema that
    // accepted a key it does not have without saying so — and silence is what turned it
    // into a loss, because a save keeps only the shape a load accepts. The line went on
    // the next unrelated settings-panel edit with nothing having warned it would.
    let dir = scratch("background-key");
    let path = write(
        &dir,
        "config.toml",
        "\
[window.background]
kind = \"solid\"
# the colour I want
color = \"#101010\"
",
    );

    let loaded = load(&path).expect("loads");
    assert!(
        loaded
            .diagnostics
            .iter()
            .any(|it| it.message.contains("window.background.color")),
        "a key the solid background does not have was accepted in silence: {:?}",
        loaded.diagnostics
    );

    let mut config = loaded.config;
    config.font.size = 17.0;
    save(&config, &path).expect("saves");
    let written = read(&path);
    assert!(
        !written.contains("#101010"),
        "the reported key was kept and the diagnostic was wrong:\n{written}"
    );
}

#[test]
fn a_key_a_background_shape_does_have_is_not_reported() {
    // The check is against the keys of the kind the file named, not against a list of
    // every key any kind has. A gradient is four keys and none of them is a mistake.
    let dir = scratch("background-keys-ok");
    let path = write(
        &dir,
        "config.toml",
        "\
[window.background]
kind = \"gradient\"
from = \"#0a0b0d\"
to = \"#1a1030\"
angle = 90.0
",
    );
    let loaded = load(&path).expect("loads");
    assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
}

#[test]
fn a_hand_written_file_applies_without_being_rewritten_first() {
    let dir = scratch("hand-written");
    let path = write(
        &dir,
        "config.toml",
        "\
theme = \"tokyo-night\"

[window]
opacity = 0.9

[window.background]
kind = \"gradient\"
from = \"#0a0b0d\"
to = \"#1a1030\"
angle = 90.0

[tabs]
position = \"left\"
",
    );

    let loaded = load(&path).expect("loads");
    assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
    assert_eq!(loaded.config.theme, "tokyo-night");
    assert!((loaded.config.window.opacity - 0.9).abs() < f32::EPSILON);
    assert_eq!(loaded.config.tabs.position, TabPosition::Left);
    assert!(matches!(
        loaded.config.window.background,
        Background::Gradient { angle, .. } if (angle - 90.0).abs() < f32::EPSILON
    ));
}

#[test]
fn a_missing_file_loads_as_defaults_without_complaining() {
    let dir = scratch("missing");
    let loaded = load(&dir.join("config.toml")).expect("loads");
    assert!(!loaded.existed);
    assert!(loaded.diagnostics.is_empty());
    assert_eq!(loaded.config, Config::default());
}

#[test]
fn a_file_that_is_not_toml_at_all_is_reported_rather_than_replaced_silently() {
    let dir = scratch("garbage");
    let path = write(&dir, "config.toml", "this is not = = toml [[[\n");
    let error = load(&path).expect_err("garbage must be reported");
    let message = error.to_string();
    assert!(message.contains("config.toml"), "{message}");
}

#[test]
fn the_scratch_directory_is_inside_the_repository() {
    // The guard on the harness itself. If cargo ever moves `CARGO_TARGET_TMPDIR`
    // outside the checkout, every test above starts writing into a user profile
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
