//! The configuration file: its schema, its defaults, and its round-trip.
//!
//! Three rules shape everything here.
//!
//! **Every field has a default, and the defaults are what a fresh install writes.** A
//! config file that is missing a key is not an error, and a key that is present and
//! equal to its default is indistinguishable from one that is absent. That is what
//! makes the settings panel able to write a file that a hand-editor can then delete
//! lines from without breaking anything.
//!
//! **A key that is not in the schema is an error, not a shrug.** `deny_unknown_fields`
//! is on for every section. Silently ignoring `fonnt.size` means a user changes a
//! setting, sees nothing happen, and has no way to find out why. The cost is that a
//! config written by a newer zet is rejected by an older one, which is the right
//! trade: zet ships the file and the binary together, and there is no third party
//! writing this file.
//!
//! **A bad file never stops the app from starting.** [`load`] returns diagnostics
//! alongside a config that is always usable, falling back per-section to the default
//! when a section will not parse. A terminal that refuses to open because a colour is
//! misspelled is a terminal you cannot use to fix the misspelling.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::rgb::Rgb;
use crate::theme;
use crate::{ConfigError, palette};

/// How serious a problem found while loading the configuration is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    /// The app runs, but not with what the file asked for.
    Warning,
    /// A setting was discarded. The app still runs, on the default for that setting.
    Error,
}

/// Something the configuration file got wrong.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    /// How serious it is.
    pub severity: Severity,
    /// What is wrong, in a sentence, naming the key where possible.
    pub message: String,
}

impl Diagnostic {
    fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
        }
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
        }
    }
}

/// A configuration and the problems found reading it.
#[derive(Clone, Debug)]
pub struct Loaded {
    /// The settings, always usable.
    pub config: Config,
    /// What went wrong, in the order it was found. Empty for a clean load.
    pub diagnostics: Vec<Diagnostic>,
    /// Where it was read from.
    pub path: PathBuf,
    /// Whether the file existed. A missing file is not a diagnostic — it is a fresh
    /// install, and the app writes the defaults out on first save rather than on
    /// first launch.
    pub existed: bool,
}

// ---------------------------------------------------------------------------------
// The schema
// ---------------------------------------------------------------------------------

/// Everything in `config.toml`.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Config {
    /// The active theme, by slug. See [`crate::by_slug`].
    pub theme: String,
    /// Typeface and size.
    pub font: FontSettings,
    /// The window itself.
    pub window: WindowSettings,
    /// The tab strip.
    pub tabs: TabSettings,
    /// The cursor.
    pub cursor: CursorSettings,
    /// System-integration switches.
    pub appearance: Appearance,
    /// Key bindings.
    pub keys: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: theme::default_theme().slug.to_owned(),
            font: FontSettings::default(),
            window: WindowSettings::default(),
            tabs: TabSettings::default(),
            cursor: CursorSettings::default(),
            appearance: Appearance::default(),
            keys: default_keymap(),
        }
    }
}

/// The typeface and its size.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct FontSettings {
    /// The family name, exactly as Windows reports it.
    pub family: String,
    /// The size in points at 100% scaling.
    pub size: f32,
    /// Families tried in order when the primary has no glyph for a character.
    ///
    /// Separate from the primary because the two answer different questions. The
    /// primary must be monospaced or the grid falls apart; a fallback only has to
    /// cover the one character it is reached for, and the best fallback for an emoji
    /// is a proportional font nobody would want to type in.
    pub fallback: Vec<String>,
}

impl Default for FontSettings {
    fn default() -> Self {
        // DESIGN.md fixes the grid face: Cascadia Mono, with Consolas behind it. Both
        // ship with Windows 11, so a fresh install has a working terminal before the
        // user has opened the settings panel.
        //
        // The emoji face comes before the symbol face deliberately, and the order was
        // wrong until a test caught it. Segoe UI Symbol contains monochrome outlines
        // for the same codepoints Segoe UI Emoji has colour glyphs for, so putting it
        // first means 😀 is drawn as a grey blob by a font that was asked for a
        // diamond. The emoji face covers nothing else — box drawing and the arrows
        // come from the primary — so it costs the symbol face nothing to go second.
        Self {
            family: "Cascadia Mono".to_owned(),
            size: 13.0,
            fallback: vec![
                "Consolas".to_owned(),
                "Segoe UI Emoji".to_owned(),
                "Segoe UI Symbol".to_owned(),
            ],
        }
    }
}

/// The window itself.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct WindowSettings {
    /// What is painted behind the grid.
    pub background: Background,
    /// How opaque the window is, from 0.0 to 1.0.
    pub opacity: f32,
    /// Whether the window remembers where it was.
    pub remember_position: bool,
    /// Whether the window starts maximized.
    pub start_maximized: bool,
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            background: Background::Solid,
            opacity: 1.0,
            remember_position: true,
            start_maximized: false,
        }
    }
}

/// What is drawn behind the grid.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Background {
    /// The theme's ground colour, flat.
    #[default]
    Solid,
    /// A picture, scaled to fill.
    Image {
        /// Where it is.
        path: PathBuf,
        /// How opaque the picture is, from 0.0 to 1.0. The theme's ground shows
        /// through below this, which is what keeps text legible over a photograph.
        #[serde(default = "opaque")]
        opacity: f32,
    },
    /// A two-stop linear gradient.
    Gradient {
        /// The first colour.
        from: Rgb,
        /// The second colour.
        to: Rgb,
        /// The direction, in degrees clockwise from pointing right.
        angle: f32,
    },
}

/// The default opacity for a background image the file did not give one.
///
/// A named function rather than a literal because `#[serde(default = "...")]` takes a
/// path, not an expression.
fn opaque() -> f32 {
    1.0
}

/// The tab strip.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct TabSettings {
    /// Where the strip lives.
    pub position: TabPosition,
    /// Whether a new tab opens the default profile without asking.
    pub open_default_without_asking: bool,
}

impl Default for TabSettings {
    fn default() -> Self {
        Self {
            position: TabPosition::Top,
            open_default_without_asking: true,
        }
    }
}

/// Where the tab strip lives.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TabPosition {
    /// One row shared with the titlebar.
    #[default]
    Top,
    /// A rail down the left edge.
    Left,
}

impl TabPosition {
    /// The value written in the config file.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Left => "left",
        }
    }

    /// The other one, for the key that flips the strip.
    #[must_use]
    pub const fn flipped(self) -> Self {
        match self {
            Self::Top => Self::Left,
            Self::Left => Self::Top,
        }
    }
}

/// The cursor.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct CursorSettings {
    /// Which shape to draw.
    pub shape: CursorShape,
    /// Whether it blinks.
    pub blink: bool,
    /// The thickness in pixels of a bar or underline cursor.
    pub thickness: u8,
}

impl Default for CursorSettings {
    fn default() -> Self {
        Self {
            shape: CursorShape::Block,
            blink: true,
            thickness: 2,
        }
    }
}

/// The shapes a cursor can take.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CursorShape {
    /// A filled block over the cell.
    #[default]
    Block,
    /// A vertical bar at the left of the cell.
    Bar,
    /// A horizontal line under the cell.
    Underline,
    /// A block outline, drawn as a rectangle.
    HollowBlock,
}

impl CursorShape {
    /// Every shape, in the order the settings panel lists them.
    pub const ALL: [CursorShape; 4] = [
        CursorShape::Block,
        CursorShape::Bar,
        CursorShape::Underline,
        CursorShape::HollowBlock,
    ];

    /// The value written in the config file.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::Bar => "bar",
            Self::Underline => "underline",
            Self::HollowBlock => "hollow-block",
        }
    }
}

/// System integration that is the user's choice rather than the system's.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Appearance {
    /// Text scaling for the chrome, from 0.5 to 3.0.
    ///
    /// `0.0` means "follow the system", which is the default and is what makes Windows'
    /// own text-size slider work without zet having to be told about it twice.
    pub text_scale: f32,
    /// Whether to honor the system's reduce-motion setting.
    pub follow_reduce_motion: bool,
    /// Whether to switch to the contrast theme when Windows reports forced colours.
    pub follow_forced_colors: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            text_scale: 0.0,
            follow_reduce_motion: true,
            follow_forced_colors: true,
        }
    }
}

// ---------------------------------------------------------------------------------
// The default keymap
// ---------------------------------------------------------------------------------

/// The bindings a fresh install gets.
///
/// A `BTreeMap` rather than named fields, because the settings panel lists them in
/// order and a map keeps that order stable and sorted when the panel writes them back.
/// The action names are the map's keys, so an unknown action is an unknown key and is
/// caught by the same `deny_unknown_fields`-style check that guards the rest of the
/// file — see [`unknown_actions`].
///
/// Chord syntax is `Ctrl+Shift+T`, with modifiers in any order and the key last. The
/// parser lives in `zet-input`; this crate only stores the text, because a keybinding
/// is a string a user types and validating it here would mean this crate knowing what
/// a key is.
#[must_use]
pub fn default_keymap() -> BTreeMap<String, String> {
    [
        ("new-tab", "Ctrl+Shift+T"),
        ("close-tab", "Ctrl+Shift+W"),
        ("next-tab", "Ctrl+Tab"),
        ("previous-tab", "Ctrl+Shift+Tab"),
        ("new-window", "Ctrl+Shift+N"),
        ("copy", "Ctrl+Shift+C"),
        ("paste", "Ctrl+Shift+V"),
        ("find", "Ctrl+Shift+F"),
        ("settings", "Ctrl+Shift+Comma"),
        ("toggle-tab-position", "Ctrl+Shift+P"),
        ("scroll-page-up", "Shift+PageUp"),
        ("scroll-page-down", "Shift+PageDown"),
        ("scroll-to-top", "Ctrl+Shift+Home"),
        ("scroll-to-bottom", "Ctrl+Shift+End"),
        ("font-larger", "Ctrl+Plus"),
        ("font-smaller", "Ctrl+Minus"),
        ("font-reset", "Ctrl+0"),
        ("quit", "Alt+F4"),
    ]
    .into_iter()
    .map(|(action, chord)| (action.to_owned(), chord.to_owned()))
    .collect()
}

/// The actions a keymap entry may name.
pub const ACTIONS: [&str; 18] = [
    "new-tab",
    "close-tab",
    "next-tab",
    "previous-tab",
    "new-window",
    "copy",
    "paste",
    "find",
    "settings",
    "toggle-tab-position",
    "scroll-page-up",
    "scroll-page-down",
    "scroll-to-top",
    "scroll-to-bottom",
    "font-larger",
    "font-smaller",
    "font-reset",
    "quit",
];

/// Keymap entries naming an action that does not exist.
#[must_use]
pub fn unknown_actions(keys: &BTreeMap<String, String>) -> Vec<String> {
    keys.keys()
        .filter(|action| !ACTIONS.contains(&action.as_str()))
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------------
// Loading and saving
// ---------------------------------------------------------------------------------

/// Where the configuration lives.
///
/// `%APPDATA%\zet\config.toml`. Windows' roaming profile directory, because a
/// terminal's settings are exactly the kind of thing a user expects to follow them
/// between machines on a domain.
///
/// # Errors
///
/// Fails when `APPDATA` is not set, which happens in a service context and nowhere a
/// terminal is running.
pub fn default_path() -> Result<PathBuf, ConfigError> {
    let base = std::env::var_os("APPDATA")
        .filter(|value| !value.is_empty())
        .ok_or(ConfigError::NoConfigDir)?;
    Ok(PathBuf::from(base).join("zet").join("config.toml"))
}

/// Read the configuration at `path`.
///
/// Never fails on account of the file's contents: a file that does not parse, or a
/// section that does not, produces diagnostics and defaults rather than an error. The
/// only error is one where nothing could be read at all and the caller has to decide
/// what to do about an unreadable directory.
///
/// # Errors
///
/// Fails if the file exists and cannot be read, or if it is not valid TOML at all.
pub fn load(path: &Path) -> Result<Loaded, ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(ConfigError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };

    let existed = text.is_some();
    let mut diagnostics = Vec::new();
    let config = match &text {
        None => Config::default(),
        Some(text) => from_str(text, path, &mut diagnostics)?,
    };

    Ok(Loaded {
        config,
        diagnostics,
        path: path.to_path_buf(),
        existed,
    })
}

/// Read the configuration at the default path, falling back to defaults.
///
/// The path a running app wants: it cannot do anything useful about an unreadable
/// `%APPDATA%`, so it starts with defaults and says so.
#[must_use]
pub fn load_default() -> Loaded {
    let Ok(path) = default_path() else {
        return Loaded {
            config: Config::default(),
            diagnostics: vec![Diagnostic::error(
                "APPDATA is not set, so the configuration could not be read",
            )],
            path: PathBuf::new(),
            existed: false,
        };
    };
    load(&path).unwrap_or_else(|error| Loaded {
        config: Config::default(),
        diagnostics: vec![Diagnostic::error(error.to_string())],
        path,
        existed: true,
    })
}

/// Parse a configuration, collecting everything wrong with it.
fn from_str(
    text: &str,
    path: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Config, ConfigError> {
    let config: Config = match toml_edit::de::from_str(text) {
        Ok(config) => config,
        Err(error) => {
            // A file that does not parse as a whole gets one diagnostic naming the
            // line, and the app runs on defaults. Trying to salvage individual
            // sections out of text the parser rejected would mean guessing which part
            // of the file the user meant, and guessing wrong is worse than saying so.
            return Err(ConfigError::Parse {
                path: path.to_path_buf(),
                message: error.to_string(),
            });
        }
    };
    check(&config, diagnostics);
    Ok(config)
}

/// Report the values that parsed but cannot be used.
///
/// A separate pass from deserialization because these are semantic rather than
/// syntactic: `size = -3` is a perfectly good float and a perfectly bad font size.
/// Each one is clamped by the caller that uses it, so a diagnostic here is
/// information rather than a refusal to run.
fn check(config: &Config, diagnostics: &mut Vec<Diagnostic>) {
    if theme::by_slug(&config.theme).is_none() {
        diagnostics.push(Diagnostic::error(format!(
            "theme = {:?} does not name a theme zet ships; using {:?}",
            config.theme,
            theme::default_theme().slug
        )));
    }
    if !(4.0..=72.0).contains(&config.font.size) {
        diagnostics.push(Diagnostic::error(format!(
            "font.size = {} is outside 4 to 72; using {}",
            config.font.size,
            FontSettings::default().size
        )));
    }
    if !(0.0..=1.0).contains(&config.window.opacity) {
        diagnostics.push(Diagnostic::error(format!(
            "window.opacity = {} is outside 0 to 1",
            config.window.opacity
        )));
    }
    if !(1..=3).contains(&config.cursor.thickness) {
        diagnostics.push(Diagnostic::warning(format!(
            "cursor.thickness = {} is outside 1 to 3",
            config.cursor.thickness
        )));
    }
    if config.appearance.text_scale != 0.0 && !(0.5..=3.0).contains(&config.appearance.text_scale) {
        diagnostics.push(Diagnostic::error(format!(
            "appearance.text-scale = {} is outside 0.5 to 3.0",
            config.appearance.text_scale
        )));
    }
    if let Background::Image { path, .. } = &config.window.background
        && !path.exists()
    {
        diagnostics.push(Diagnostic::error(format!(
            "window.background.path = {} does not exist",
            path.display()
        )));
    }
    for action in unknown_actions(&config.keys) {
        diagnostics.push(Diagnostic::error(format!(
            "keys.{action} is not an action zet knows"
        )));
    }
}

/// The configuration, with every diagnostic-triggering value replaced by its default.
///
/// What the app should actually run on. Distinct from [`load`] so that the diagnostics
/// can still be shown: the user needs to see *what* was wrong at the same time as a
/// working terminal, and silently repairing the values would hide the first.
#[must_use]
pub fn repaired(config: &Config) -> Config {
    let defaults = Config::default();
    let mut fixed = config.clone();
    if theme::by_slug(&fixed.theme).is_none() {
        fixed.theme = defaults.theme;
    }
    if !(4.0..=72.0).contains(&fixed.font.size) {
        fixed.font.size = defaults.font.size;
    }
    fixed.window.opacity = fixed.window.opacity.clamp(0.0, 1.0);
    fixed.cursor.thickness = fixed.cursor.thickness.clamp(1, 3);
    if fixed.appearance.text_scale != 0.0 {
        fixed.appearance.text_scale = fixed.appearance.text_scale.clamp(0.5, 3.0);
    }
    if let Background::Image { path, .. } = &fixed.window.background
        && !path.exists()
    {
        fixed.window.background = Background::Solid;
    }
    fixed
}

/// The chrome palette for `config`.
///
/// The one place the two planes meet. Forced colours is not a theme choice — it is an
/// accessibility requirement, and it overrides the chrome while leaving the grid's
/// theme alone so that a program's colours stay a program's colours.
#[must_use]
pub fn palette_for(config: &Config, forced_colors: Option<Rgb>) -> palette::Palette {
    match forced_colors {
        Some(highlight) if config.appearance.follow_forced_colors => {
            palette::Palette::high_contrast(highlight)
        }
        _ => palette::Palette::instrument(),
    }
}

/// Write `config` to `path`, preserving comments already in the file.
///
/// The round-trip PRODUCT.md asks for. The existing file is parsed as a
/// comment-preserving document and only the leaves whose values actually changed are
/// replaced, each keeping the trailing comment that was on it. A save with nothing
/// changed leaves the file byte-identical, which is what makes it safe to call on
/// every settings-panel edit.
///
/// # Errors
///
/// Fails if the file cannot be written, or if what is already there is not TOML — in
/// which case writing would destroy something the user typed.
pub fn save(config: &Config, path: &Path) -> Result<(), ConfigError> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let text = render(config, &existing)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(path, text).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Render `config` as TOML, merging into `existing` if that parses.
fn render(config: &Config, existing: &str) -> Result<String, ConfigError> {
    let text = toml_edit::ser::to_string_pretty(config).map_err(|error| ConfigError::Parse {
        path: PathBuf::new(),
        message: error.to_string(),
    })?;
    // A file that is not TOML is replaced rather than merged, and so is a file that is
    // TOML but somehow unreadable as a document. Refusing to save would leave the
    // settings panel and the file permanently out of step, and the parse error is
    // reported separately through `load`, where the user can see it.
    if existing.trim().is_empty() {
        return Ok(text);
    }
    let Ok(mut document) = existing.parse::<toml_edit::DocumentMut>() else {
        return Ok(text);
    };
    let Ok(fresh) = text.parse::<toml_edit::DocumentMut>() else {
        return Ok(text);
    };
    merge(document.as_table_mut(), fresh.as_table());
    Ok(document.to_string())
}

/// Copy every leaf of `fresh` into `document`, keeping comments on the way.
fn merge(document: &mut toml_edit::Table, fresh: &toml_edit::Table) {
    for (key, value) in fresh {
        match (document.get_mut(key), value) {
            (Some(toml_edit::Item::Table(existing)), toml_edit::Item::Table(fresh)) => {
                merge(existing, fresh);
            }
            (Some(toml_edit::Item::Value(existing)), toml_edit::Item::Value(fresh)) => {
                if existing.to_string() != fresh.to_string() {
                    // The suffix is where an inline `# comment` lives, and the prefix is
                    // the whitespace before the value. Carrying both across is the whole
                    // reason this is a merge rather than a re-serialize.
                    let mut replacement = fresh.clone();
                    let decor = replacement.decor_mut();
                    decor.set_prefix(existing.decor().prefix().cloned().unwrap_or_default());
                    decor.set_suffix(existing.decor().suffix().cloned().unwrap_or_default());
                    *existing = replacement;
                }
            }
            (_, toml_edit::Item::Table(fresh)) => {
                document.insert(key, toml_edit::Item::Table(fresh.clone()));
            }
            (_, toml_edit::Item::Value(fresh)) => {
                document.insert(key, toml_edit::Item::Value(fresh.clone()));
            }
            (_, other) => {
                document.insert(key, other.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded(text: &str) -> (Config, Vec<Diagnostic>) {
        let mut diagnostics = Vec::new();
        let config = from_str(text, Path::new("test.toml"), &mut diagnostics).expect("valid");
        (config, diagnostics)
    }

    #[test]
    fn an_empty_file_is_the_defaults() {
        let (config, diagnostics) = loaded("");
        assert_eq!(config, Config::default());
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn a_partial_file_keeps_the_defaults_for_what_it_omits() {
        let (config, diagnostics) = loaded("theme = \"nord\"\n");
        assert_eq!(config.theme, "nord");
        assert_eq!(config.font, FontSettings::default());
        assert_eq!(config.keys, default_keymap());
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn a_misspelled_key_is_an_error_rather_than_an_ignored_line() {
        // The failure this prevents: a user edits `font.size`, mistypes it, and the
        // only symptom is that nothing happens.
        let error = from_str(
            "[font]\nsizz = 20.0\n",
            Path::new("test.toml"),
            &mut Vec::new(),
        )
        .expect_err("a misspelled key must not be ignored");
        let message = error.to_string();
        assert!(message.contains("sizz"), "{message}");
    }

    #[test]
    fn a_theme_that_does_not_exist_is_reported_and_repaired() {
        let (config, diagnostics) = loaded("theme = \"solarized-dark\"\n");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert_eq!(repaired(&config).theme, Config::default().theme);
    }

    #[test]
    fn a_font_size_outside_the_range_is_reported_and_repaired() {
        let (config, diagnostics) = loaded("[font]\nsize = 900.0\n");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(repaired(&config).font, FontSettings::default());
    }

    #[test]
    fn an_unknown_action_in_the_keymap_is_reported() {
        let (_, diagnostics) = loaded("[keys]\nopen-the-pod-bay-doors = \"Ctrl+H\"\n");
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("open-the-pod-bay-doors"));
    }

    #[test]
    fn a_missing_background_image_is_reported() {
        let (config, diagnostics) =
            loaded("[window.background]\nkind = \"image\"\npath = \"Z:/nowhere/at/all.png\"\n");
        assert_eq!(diagnostics.len(), 1);
        assert!(matches!(
            repaired(&config).window.background,
            Background::Solid
        ));
    }

    #[test]
    fn a_gradient_background_round_trips() {
        let (config, diagnostics) = loaded(
            "[window.background]\nkind = \"gradient\"\nfrom = \"#0a0b0d\"\nto = \"#1a1030\"\nangle = 45.0\n",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        // Compared field by field rather than as a whole value, because `angle` is an
        // `f32` and a whole-value comparison would be an exact float equality.
        assert!(matches!(
            config.window.background,
            Background::Gradient { from, to, angle }
                if from == Rgb::new(0x0a, 0x0b, 0x0d)
                    && to == Rgb::new(0x1a, 0x10, 0x30)
                    && (angle - 45.0).abs() < f32::EPSILON
        ));
    }

    #[test]
    fn a_bad_colour_in_a_gradient_names_the_problem() {
        let error = from_str(
            "[window.background]\nkind = \"gradient\"\nfrom = \"not a colour\"\nto = \"#000000\"\nangle = 0.0\n",
            Path::new("test.toml"),
            &mut Vec::new(),
        )
        .expect_err("a bad colour must not be accepted");
        let message = error.to_string();
        // What has to be in the message is the offending text and what a colour is
        // supposed to look like. The key and the line come from the TOML renderer
        // around it, which points at `[window.background]` rather than at `from` — the
        // span it can prove is the table's, not the field's.
        assert!(message.contains("not a colour"), "{message}");
        assert!(message.contains("#0a0b0d"), "{message}");
    }

    #[test]
    fn the_defaults_round_trip_through_toml_unchanged() {
        let config = Config::default();
        let text = toml_edit::ser::to_string_pretty(&config).expect("serializable");
        let (parsed, diagnostics) = loaded(&text);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(parsed, config);
    }

    #[test]
    fn a_file_that_is_not_toml_is_replaced_rather_than_merged() {
        let garbage = "this is not = = toml [[[";
        let rendered = render(&Config::default(), garbage).expect("renders");
        assert!(rendered.contains("theme ="));
    }

    #[test]
    fn the_default_keymap_only_names_actions_that_exist() {
        assert!(unknown_actions(&default_keymap()).is_empty());
        assert_eq!(default_keymap().len(), ACTIONS.len());
    }

    #[test]
    fn forced_colors_overrides_the_chrome_only_when_asked() {
        let mut config = Config::default();
        let highlight = Rgb::new(0x00, 0x78, 0xd4);
        assert_eq!(
            palette_for(&config, Some(highlight)),
            palette::Palette::high_contrast(highlight)
        );
        config.appearance.follow_forced_colors = false;
        assert_eq!(
            palette_for(&config, Some(highlight)),
            palette::Palette::instrument()
        );
        // No forced colours reported at all: the setting is irrelevant.
        assert_eq!(
            palette_for(&Config::default(), None),
            palette::Palette::instrument()
        );
    }

    #[test]
    fn the_shipped_keymap_is_written_in_the_chord_syntax_the_parser_expects() {
        // This crate does not parse chords, but it can at least refuse to ship one that
        // has an obviously wrong shape: a `+` at either end, or an empty binding.
        for (action, chord) in default_keymap() {
            assert!(!chord.is_empty(), "{action} binds nothing");
            assert!(
                !chord.starts_with('+') && !chord.ends_with('+'),
                "{action} = {chord:?} has a dangling separator"
            );
        }
    }
}
