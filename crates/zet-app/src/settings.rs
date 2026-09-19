//! What the settings panel contains, and what clicking it does.
//!
//! The panel is drawn by `zet-ui`, which knows a row is a label, a value and a way of
//! being clicked and knows nothing else about it. What a row *means* is here, because
//! every one of them is a field of [`Config`] and a second copy of the configuration
//! living next to the painter is exactly how a panel and a file start disagreeing.
//!
//! The split is a list in and a list out. [`App::settings`] reads the configuration and
//! answers with the rows to draw; [`App::adjust`] takes the row that was clicked and
//! changes the configuration. Nothing in between holds state, so the panel cannot show a
//! value the file does not have.
//!
//! Two rows are not fields at all. [`Id::Binding`] is the `[keys]` table, one row per
//! action, and it is *captured* rather than clicked: the caller is told to wait for a
//! chord and hands it back through [`App::bind`].

use zet_config::{Background, Config, CursorShape, Diagnostic, Severity};
use zet_input::Chord;

use crate::action::Action;

/// The smallest and largest the grid font may be set to, in points.
///
/// The schema's, for the same reason as the cursor thickness below: the panel's stepper,
/// the clamp on the size the grid is drawn at, and the range the file is checked against
/// are one number, and a panel that offered a size the next launch warned about would be
/// a panel that lies about what it can set.
pub const MIN_SIZE: f32 = zet_config::MIN_FONT_SIZE;

/// The largest the grid font may be set to, in points.
pub const MAX_SIZE: f32 = zet_config::MAX_FONT_SIZE;

/// The cursor thicknesses the panel offers, in pixels.
///
/// The schema's, not the panel's own: the panel offers what the configuration file
/// accepts, and a stepper that could take a thickness past the range the schema warns
/// about would let the user set a value the file then complains about.
const MIN_THICKNESS: u8 = zet_config::MIN_CURSOR_THICKNESS;
const MAX_THICKNESS: u8 = zet_config::MAX_CURSOR_THICKNESS;

/// The window opacities the panel offers, as whole percents.
///
/// The floor is the panel's and not the schema's, which accepts anything from nothing to
/// all of it: a window at zero is a ghost, and the panel that would raise it again is
/// drawn *in the window*, so a user who stepped down to it could not see the control that
/// undoes it. Below twenty percent the terminal is unreadable anyway, so the range the
/// stepper moves through is the range that leaves a window someone can still use, and the
/// file still says what it says.
///
/// Whole percents, and stepped in them, so that going up and back down lands on the value
/// it started from: adding a tenth at a time accumulates the error of a number that is not
/// a tenth and drifts off the value the file was read with.
const MIN_OPACITY: u32 = 20;
const OPACITY_STEP: u32 = 10;

/// The opacity floor for a background picture, and the step the row moves in.
///
/// Lower than the window's, because the two are different questions: a window at nothing
/// takes the terminal with it, while a picture at nothing leaves the theme's own ground
/// behind it — which is a setting someone might want for a moment and would not want to be
/// unable to leave. Ten percent rather than none for the same reason the window has a floor
/// at all: the row that would raise it is drawn over the thing it is behind.
const MIN_IMAGE_OPACITY: u32 = 10;
const IMAGE_OPACITY_STEP: u32 = 10;

/// The angles the gradient row moves through, in degrees clockwise from pointing right.
///
/// A stepper rather than a list of compass names, because the renderer takes a number and
/// DESIGN.md gives a `Step` to a number. Fifteen degrees is a sixteenth of the circle: fine
/// enough that a gradient can be pointed where it looks right, coarse enough that getting
/// from one side to the other is a handful of clicks rather than a scroll.
const ANGLE_STEP: f32 = 15.0;

/// The largest angle the row will step to.
///
/// Short of a full turn on purpose. A stepper stops at its ends rather than wrapping, so an
/// angle of 360 would be a value the row could show and never reach again; 345 is the last
/// one before the direction it started from.
const MAX_ANGLE: f32 = 345.0;

/// Text scales, as the system's own slider offers them.
///
/// `0.0` is "follow the system" and is not a percentage, which is why this is a table of
/// pairs rather than a list of numbers.
const TEXT_SCALES: [(&str, f32); 6] = [
    ("System", 0.0),
    ("100%", 1.0),
    ("125%", 1.25),
    ("150%", 1.5),
    ("175%", 1.75),
    ("200%", 2.0),
];

/// What a row's control does when it is clicked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Two states. Either half flips it.
    Toggle,
    /// One of several values. The two halves step through the list.
    Choice,
    /// A number. The left half goes down, the right half goes up.
    Step,
    /// A key. Either half asks for the next chord the user presses.
    Chord,
}

/// Which setting a row is.
///
/// An identity rather than an index, because the rows a click can land on are not fixed:
/// the list changes length when the cursor shape makes the thickness row apply, and a
/// click resolved by position would then change the setting below the one that was hit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Id {
    /// The colour scheme, by slug.
    Theme,
    /// The text scale, `0.0` for the system's.
    TextScale,
    /// Whether the system's reduce-motion setting is honoured.
    ReduceMotion,
    /// Whether the system's forced-colours setting is honoured.
    ForcedColors,
    /// How opaque the window is.
    WindowOpacity,
    /// What is painted behind the grid.
    Background,
    /// How much of a background picture shows.
    ///
    /// A row of its own rather than a second half of the background row, because it is a
    /// second number: the kind and how much of it there is are two settings that the
    /// control the panel has can each carry, and one of them is a picture.
    ImageOpacity,
    /// Which way a background gradient runs.
    GradientAngle,
    /// Whether the window remembers where it was.
    RememberPosition,
    /// Whether the window starts maximized.
    StartMaximized,
    /// Where the tab strip lives.
    TabPosition,
    /// Whether a new tab opens the default profile without asking.
    OpenWithoutAsking,
    /// The grid's family.
    Font,
    /// The grid's size, in points.
    FontSize,
    /// The cursor's shape.
    CursorShape,
    /// Whether the cursor blinks.
    CursorBlink,
    /// The cursor's thickness, for the shapes that have one.
    CursorThickness,
    /// The chord bound to an action.
    Binding(Action),
}

/// One row the panel can draw.
pub struct Setting {
    /// What the row is.
    pub id: Id,
    /// What the row is called.
    pub label: &'static str,
    /// What it is set to, as it should be read.
    pub value: String,
    /// How it is clicked.
    pub kind: Kind,
}

/// A row of the panel: a section heading, or something you can set.
pub enum Line {
    /// A section's name, and the rule under it.
    Heading(&'static str),
    /// Something the configuration file got wrong.
    ///
    /// A row with nothing to click, because there is nothing here to set: the file is the
    /// setting, and this is what the loader said about it. The two are drawn differently —
    /// a heading is a section, and this is a line of text under one.
    Note(Problem),
    /// A setting.
    Setting(Setting),
}

/// One thing the configuration file got wrong.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Problem {
    /// What is wrong, in the loader's own words.
    pub text: String,
    /// How serious it is.
    pub severity: Severity,
}

impl Line {
    /// The text to draw: a heading's name, a problem's message, or a setting's label.
    #[must_use]
    pub fn text(&self) -> &str {
        match self {
            Self::Heading(text) => text,
            Self::Note(problem) => &problem.text,
            Self::Setting(setting) => setting.label,
        }
    }

    /// The value to draw, empty for a heading.
    #[must_use]
    pub fn value(&self) -> &str {
        match self {
            Self::Heading(_) => "",
            // The severity, which is the column the configuration reference documents and
            // the only thing on the row that is not the message itself.
            Self::Note(problem) => match problem.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            },
            Self::Setting(setting) => &setting.value,
        }
    }

    /// How the row is clicked, `None` for a heading or a problem.
    #[must_use]
    pub const fn kind(&self) -> Option<Kind> {
        match self {
            Self::Heading(_) | Self::Note(_) => None,
            Self::Setting(setting) => Some(setting.kind),
        }
    }

    /// What the row sets, `None` for a heading or a problem.
    #[must_use]
    pub const fn id(&self) -> Option<Id> {
        match self {
            Self::Heading(_) | Self::Note(_) => None,
            Self::Setting(setting) => Some(setting.id),
        }
    }
}

/// What a click asked the caller to do about it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effect {
    /// The row does not move that way, or the click changed nothing.
    None,
    /// The configuration changed. Write it back, and rebuild whatever reads it.
    Changed,
    /// The row wants the next chord the user presses, for this action.
    Capture(Action),
}

/// What the loader found wrong with the file, as rows.
///
/// Separate from [`lines`] because it is the one section that is about the file rather
/// than in it: nothing here can be clicked, nothing here is a field of [`Config`], and the
/// whole section goes away when there is nothing to say. [`App::settings`] puts it first,
/// because a problem the user has to scroll to find is a problem they will not find.
///
/// An empty list is no rows at all and not an empty heading: a panel that opened with
/// "Problems" over nothing would teach the user to ignore the one time it mattered.
#[must_use]
pub fn problems(diagnostics: &[Diagnostic]) -> Vec<Line> {
    if diagnostics.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::with_capacity(diagnostics.len() + 1);
    lines.push(Line::Heading("Problems"));
    lines.extend(diagnostics.iter().map(|diagnostic| {
        Line::Note(Problem {
            text: diagnostic.message.clone(),
            severity: diagnostic.severity,
        })
    }));
    lines
}

/// The Theme row, labelled with whether the palette is zet's own.
///
/// PRODUCT.md's seventh criterion promises it — imported palettes "ship unmodified so they
/// look like themselves, and the theme picker says so" — and DESIGN.md gives the reason the
/// sentence exists at all: a picker that showed them beside zet's own without a word would
/// imply they had been checked for contrast. They have not been, and could not be without
/// changing them, which is the whole point of importing a palette. `Theme::published` has
/// been set correctly on all eight since the themes were written and read by nothing, so
/// the promise was being kept by a field no user could see.
///
/// The marker is on the label rather than on the value, which is where the name goes: that
/// column is 118px — wide enough for a chord, no wider — and the painter draws a run of
/// text into the frame without clipping it, so `Solarized Light (as published)` would not
/// be cut off at the control, it would run out of the panel and over the grid.
#[must_use]
fn theme_row(config: &Config) -> Line {
    let imported = zet_config::by_slug(&config.theme).is_some_and(|theme| theme.published);
    let label = if imported {
        "Theme (as published)"
    } else {
        "Theme"
    };
    setting(Id::Theme, label, theme_name(config), Kind::Choice)
}

/// The rows, read out of a configuration.
///
/// The font row's *value* is the family in the file and needs nothing else; the list of
/// families it can be stepped through is only needed when it is clicked, which is why
/// [`adjust`] takes it and this does not.
#[must_use]
pub fn lines(config: &Config, bindings: &[(Chord, Action)]) -> Vec<Line> {
    let mut lines = appearance_rows(config);
    lines.extend(tabs_rows(config));
    lines.extend(terminal_rows(config));
    lines.extend(binding_rows(bindings));
    lines
}

/// The Appearance section: what the window is painted with, and how it reads.
#[must_use]
fn appearance_rows(config: &Config) -> Vec<Line> {
    let mut lines = vec![
        Line::Heading("Appearance"),
        theme_row(config),
        setting(
            Id::TextScale,
            "Text scale",
            scale_name(config.appearance.text_scale),
            Kind::Choice,
        ),
        setting(
            Id::ReduceMotion,
            "Reduce motion",
            follow(config.appearance.follow_reduce_motion),
            Kind::Toggle,
        ),
        setting(
            Id::ForcedColors,
            "Forced colours",
            follow(config.appearance.follow_forced_colors),
            Kind::Toggle,
        ),
        setting(
            Id::Background,
            "Background",
            title_case(kind_word(&config.window.background)),
            Kind::Choice,
        ),
    ];

    // The row that belongs to the kind the file is in, directly under the row that chooses
    // it: a picture's opacity and a gradient's angle are numbers about the thing named a
    // line above, and a row that appeared at the end of the list would be a row about
    // something the user has scrolled away from. The same rule as the cursor's thickness.
    match &config.window.background {
        Background::Image { opacity, .. } => lines.push(setting(
            Id::ImageOpacity,
            "Image opacity",
            format!("{}%", configured_image_opacity(*opacity)),
            Kind::Step,
        )),
        Background::Gradient { angle, .. } => lines.push(setting(
            Id::GradientAngle,
            "Angle",
            format!("{}°", configured_angle(*angle).round()),
            Kind::Step,
        )),
        Background::Solid => {}
    }

    lines.push(setting(
        Id::WindowOpacity,
        "Window opacity",
        format!("{}%", configured_opacity(config)),
        Kind::Step,
    ));
    lines.push(setting(
        Id::RememberPosition,
        "Remember position",
        on_off(config.window.remember_position),
        Kind::Toggle,
    ));
    lines.push(setting(
        Id::StartMaximized,
        "Start maximized",
        on_off(config.window.start_maximized),
        Kind::Toggle,
    ));
    lines
}

/// The Tabs section: where the strip lives, and what a new tab does.
#[must_use]
fn tabs_rows(config: &Config) -> Vec<Line> {
    vec![
        Line::Heading("Tabs"),
        setting(
            Id::TabPosition,
            "Position",
            title_case(config.tabs.position.as_str()),
            Kind::Choice,
        ),
        setting(
            Id::OpenWithoutAsking,
            "Open without asking",
            on_off(config.tabs.open_default_without_asking),
            Kind::Toggle,
        ),
    ]
}

/// The Terminal section: the grid's own type and cursor.
#[must_use]
fn terminal_rows(config: &Config) -> Vec<Line> {
    let mut lines = vec![
        Line::Heading("Terminal"),
        setting(Id::Font, "Font", config.font.family.clone(), Kind::Choice),
        setting(
            Id::FontSize,
            "Size",
            format!("{} pt", configured_size(config).round()),
            Kind::Step,
        ),
        setting(
            Id::CursorShape,
            "Cursor",
            title_case(config.cursor.shape.as_str()),
            Kind::Choice,
        ),
        setting(
            Id::CursorBlink,
            "Blink",
            on_off(config.cursor.blink),
            Kind::Toggle,
        ),
    ];

    // A thickness is a property of a bar and an underline. A block is the whole cell and
    // a hollow block is a border on it, so neither has one, and a row that changes
    // nothing is a row that makes the user doubt the panel rather than the shape.
    if matches!(
        config.cursor.shape,
        CursorShape::Bar | CursorShape::Underline
    ) {
        lines.push(setting(
            Id::CursorThickness,
            "Thickness",
            format!("{} px", config.cursor.thickness),
            Kind::Step,
        ));
    }

    lines
}

/// The Keys section: one row per action, reading the chord it is bound to.
#[must_use]
fn binding_rows(bindings: &[(Chord, Action)]) -> Vec<Line> {
    let mut lines = Vec::with_capacity(Action::ALL.len() + 1);
    lines.push(Line::Heading("Keys"));
    for action in Action::ALL {
        lines.push(Line::Setting(Setting {
            id: Id::Binding(action),
            label: action_name(action),
            value: bindings
                .iter()
                .find(|(_, bound)| *bound == action)
                .map_or_else(|| "Unbound".to_owned(), |(chord, _)| chord.to_string()),
            kind: Kind::Chord,
        }));
    }
    lines
}

/// The name a family is offered under.
///
/// `families` is what the machine has; the configured family is put first whether or not
/// it is in that list, because a family that has been uninstalled since it was set is
/// still what the file says, and a row that hid it would be a row that lied about the
/// file.
#[must_use]
pub fn candidates(config: &Config, families: &[String]) -> Vec<String> {
    let mut list = Vec::with_capacity(families.len() + 1);
    list.push(config.font.family.clone());
    list.extend(
        families
            .iter()
            .filter(|family| **family != config.font.family)
            .cloned(),
    );
    list
}

/// Carry out a click on a row.
///
/// `back` is the left half of a control and `forward` is the right one, which for a
/// toggle and a chord are the same thing: a two-state row has no direction, so both
/// halves flip it and both halves ask for a key.
pub fn adjust(config: &mut Config, id: Id, back: bool, families: &[String]) -> Effect {
    match id {
        Id::Theme => {
            let themes = zet_config::builtin();
            let at = themes.iter().position(|theme| theme.slug == config.theme);
            let next = step(themes.len(), at, back);
            themes[next].slug.clone_into(&mut config.theme);
            Effect::Changed
        }
        Id::TextScale => {
            let at = TEXT_SCALES.iter().position(|(_, value)| {
                (*value - config.appearance.text_scale).abs() < f32::EPSILON
            });
            config.appearance.text_scale = TEXT_SCALES[step(TEXT_SCALES.len(), at, back)].1;
            Effect::Changed
        }
        Id::ReduceMotion => {
            config.appearance.follow_reduce_motion = !config.appearance.follow_reduce_motion;
            Effect::Changed
        }
        Id::ForcedColors => {
            config.appearance.follow_forced_colors = !config.appearance.follow_forced_colors;
            Effect::Changed
        }
        Id::WindowOpacity => {
            let percent = configured_opacity(config);
            let next = if back {
                percent.saturating_sub(OPACITY_STEP)
            } else {
                percent.saturating_add(OPACITY_STEP)
            };
            config.window.opacity = next.clamp(MIN_OPACITY, 100) as f32 / 100.0;
            Effect::Changed
        }
        Id::Background => {
            let kinds = background_kinds(config);
            let at = kinds
                .iter()
                .position(|kind| *kind == kind_word(&config.window.background));
            let wanted = background_for(kinds[step(kinds.len(), at, back)], config);
            config.window.background = wanted;
            Effect::Changed
        }
        // The two rows that belong to a kind of background, and so can be clicked while the
        // file is in the other one: the row is drawn only while its kind is set, but a click
        // already in flight when the row above it changes the kind lands here, and a
        // `Effect::None` is the honest answer to a click on a background that has no opacity
        // to step.
        Id::ImageOpacity => step_image_opacity(config, back).unwrap_or(Effect::None),
        Id::GradientAngle => step_gradient_angle(config, back).unwrap_or(Effect::None),
        Id::RememberPosition => {
            config.window.remember_position = !config.window.remember_position;
            Effect::Changed
        }
        Id::StartMaximized => {
            config.window.start_maximized = !config.window.start_maximized;
            Effect::Changed
        }
        Id::OpenWithoutAsking => {
            config.tabs.open_default_without_asking = !config.tabs.open_default_without_asking;
            Effect::Changed
        }
        Id::TabPosition => {
            config.tabs.position = config.tabs.position.flipped();
            Effect::Changed
        }
        Id::Font => {
            let list = candidates(config, families);
            let at = list.iter().position(|family| *family == config.font.family);
            list[step(list.len(), at, back)].clone_into(&mut config.font.family);
            Effect::Changed
        }
        Id::FontSize => {
            config.font.size =
                (configured_size(config) + if back { -1.0 } else { 1.0 }).clamp(MIN_SIZE, MAX_SIZE);
            Effect::Changed
        }
        Id::CursorShape => {
            let at = CursorShape::ALL
                .iter()
                .position(|shape| *shape == config.cursor.shape);
            config.cursor.shape = CursorShape::ALL[step(CursorShape::ALL.len(), at, back)];
            Effect::Changed
        }
        Id::CursorBlink => {
            config.cursor.blink = !config.cursor.blink;
            Effect::Changed
        }
        Id::CursorThickness => {
            let next = i32::from(config.cursor.thickness) + if back { -1 } else { 1 };
            config.cursor.thickness =
                next.clamp(i32::from(MIN_THICKNESS), i32::from(MAX_THICKNESS)) as u8;
            Effect::Changed
        }
        Id::Binding(action) => Effect::Capture(action),
    }
}

/// Move a picture's transparency one step, `None` if the background is not a picture.
///
/// Stepped in whole percents for the same reason the window's is: going up and back down
/// lands on the value it started from rather than beside it.
#[must_use]
fn step_image_opacity(config: &mut Config, back: bool) -> Option<Effect> {
    let Background::Image { opacity, .. } = &mut config.window.background else {
        return None;
    };
    let percent = configured_image_opacity(*opacity);
    let next = if back {
        percent.saturating_sub(IMAGE_OPACITY_STEP)
    } else {
        percent.saturating_add(IMAGE_OPACITY_STEP)
    };
    *opacity = next.clamp(MIN_IMAGE_OPACITY, 100) as f32 / 100.0;
    Some(Effect::Changed)
}

/// Point a gradient one step round, `None` if the background is not a gradient.
///
/// The end of the range is a stop and not a wrap: [`MAX_ANGLE`] is where the stepper gives
/// up rather than turning over into the direction it began at.
#[must_use]
fn step_gradient_angle(config: &mut Config, back: bool) -> Option<Effect> {
    let Background::Gradient { angle, .. } = &mut config.window.background else {
        return None;
    };
    let step = if back { -ANGLE_STEP } else { ANGLE_STEP };
    *angle = zet_config::clamp_or(configured_angle(*angle) + step, 0.0, MAX_ANGLE, 0.0);
    Some(Effect::Changed)
}

/// Bind a chord to an action.
///
/// The `[keys]` table is actions in the file's words against chords, so binding an action
/// is one insert and the reverse question — which chord runs this action — is one read.
///
/// A chord can only run one thing, and that is the case worth being deliberate about. If
/// the chord was already spoken for, the action that had it is left `Unbound` rather than
/// the two of them being resolved by whichever the map happened to yield first. The panel
/// shows the action that lost reading `Unbound`, so the cost of the steal is visible
/// where the user is standing, which is the only place it can be undone.
pub fn bind(config: &mut Config, action: Action, chord: Chord) {
    let text = chord.to_string();
    // Parsed and compared as a keystroke rather than as text. One key has more than
    // one spelling — `Chord::parse` reads case and modifier order freely, and the
    // punctuation shift moves is one keystroke under two names — so a comparison of
    // strings leaves every other spelling in the file and one chord ends up running
    // two actions, resolved by whichever key the map reached first and shown in the
    // panel as both of them bound. `matches` is the crate's own answer to "is this the
    // same keystroke", and it is the one the keymap is dispatched through, so a chord
    // this takes away is exactly a chord that would have fired it.
    //
    // A line that does not parse at all is kept. It is already reported, and deleting
    // a user's line is not what pressing a key in the panel asked for.
    //
    // A line the chord was taken from is emptied and left where it is. Deleting it would
    // un-ask the question the panel just answered: the `[keys]` table is a patch on the
    // shipped bindings and an action the file does not name is an action that keeps its
    // default, so the row would read `Unbound` until the next start and the default chord
    // after it. The empty value is the file's way of saying unbound, and it is the only
    // one it has — `Chord::parse` refuses it, `parse_bindings` drops the entry, and the
    // panel's row is built from that list rather than from the table.
    for bound in config.keys.values_mut() {
        if Chord::parse(bound).is_ok_and(|parsed| parsed.matches(chord.mods, chord.key)) {
            bound.clear();
        }
    }
    config.keys.insert(action.name().to_owned(), text);
}

/// The configured font size, brought into the range the panel offers.
///
/// The panel is one of the two readers of the file's size, and the file's can be
/// anything: below the range, above it, or not a number at all. Reading the size the
/// grid is actually drawn at is what stops the row showing "NaN pt" and, worse, writing
/// the NaN back the next time either arrow is pressed.
fn configured_size(config: &Config) -> f32 {
    zet_config::clamp_or(
        config.font.size,
        MIN_SIZE,
        MAX_SIZE,
        zet_config::FontSettings::default().size,
    )
}

/// The window's opacity as a whole percent.
///
/// The row shows what the file says and the stepper moves from there, which is why this
/// rounds rather than clamping to the range the panel offers: a file set to 0.05 by hand
/// says 5%, and pressing the key that raises it goes to 20% — the first value the panel
/// can offer above it — rather than jumping from a floor of its own making. The clamp is
/// only here so that a number the file should not have, and the panel would draw as
/// nonsense, still draws as something.
fn configured_opacity(config: &Config) -> u32 {
    (config.window.opacity * 100.0).round().clamp(0.0, 100.0) as u32
}

/// The word the configuration file uses for a kind of background.
fn kind_word(background: &Background) -> &'static str {
    match background {
        Background::Solid => "solid",
        Background::Image { .. } => "image",
        Background::Gradient { .. } => "gradient",
    }
}

/// The kinds a click on the background row steps through.
///
/// `solid` and `gradient` are always in the list: the panel can write both of them whole.
/// `image` is in it only while the file already names a picture, because the path is the
/// half of that setting the four controls cannot type — a panel that offered the kind
/// without a path would write a configuration that is broken the moment it is clicked, and
/// the row that reported it afterwards would be reporting a mistake the user did not make
/// and cannot fix from where they are standing. A picture is therefore set in the file and
/// shown here, and stepping away from it is a change the panel is allowed to make.
///
/// The current kind is put first so that neither half of the control can land back on
/// `image`: the two halves are the two kinds the panel can author, whichever kind the file
/// is in.
fn background_kinds(config: &Config) -> &'static [&'static str] {
    if matches!(config.window.background, Background::Image { .. }) {
        &["image", "solid", "gradient"]
    } else {
        &["solid", "gradient"]
    }
}

/// The background a named kind means.
fn background_for(kind: &str, config: &Config) -> Background {
    match kind {
        "gradient" => seeded_gradient(config),
        // Only ever reached when the file is already in it, since that is the one way
        // `image` gets into the list: this is the picture that is already there, kept as it
        // is rather than reconstructed without the path it is made of.
        "image" => config.window.background.clone(),
        _ => Background::Solid,
    }
}

/// A gradient made of the theme's own colours.
///
/// The two stops are the half of a gradient the panel cannot pick — there is no control
/// that chooses a colour in the four DESIGN.md allows — so this is what they are until the
/// file says otherwise, and the file's pair is what the row goes on showing. They come from
/// the theme because a background is the theme's: a panel that invented a pair of colours
/// could produce a terminal disagreeing with its own palette, and the fix would be a trip
/// into the file the panel exists to save the user from.
///
/// The pair is the colour the grid paints by default and the colour it paints a selection
/// with, which are the theme's two answers to "behind the text" and "behind the text, but
/// somewhere the user is looking". Ninety degrees, because a wash that runs top to bottom
/// reads as a background where one that runs across reads as a decoration.
fn seeded_gradient(config: &Config) -> Background {
    let theme = zet_config::by_slug(&config.theme).unwrap_or_else(zet_config::default_theme);
    Background::Gradient {
        from: theme.background,
        to: theme.selection,
        angle: 90.0,
    }
}

/// A gradient's angle as a direction, in the range the row shows.
///
/// Turned back into the circle rather than clamped, because an angle is a direction and not
/// a quantity: a file set to 700 degrees points the way 340 points, and a row reading `700°`
/// would be showing a number the stepper could never produce again. A number that is not a
/// number — `nan` is a float TOML can spell — reads as zero, which is the one direction a
/// value with no direction can be said to have.
fn configured_angle(angle: f32) -> f32 {
    zet_config::clamp_or(angle.rem_euclid(360.0), 0.0, 360.0, 0.0)
}

/// A background picture's opacity as a whole percent, as the file has it.
///
/// Shown rather than brought into the range the row steps through, for the same reason as
/// the window's: a file set to five percent says five percent, and the first press of the
/// key that raises it goes to the floor rather than to a floor of the panel's own making.
fn configured_image_opacity(opacity: f32) -> u32 {
    (opacity * 100.0).round().clamp(0.0, 100.0) as u32
}

/// The next index in a ring, where `None` means "not in the list" and enters at the top.
fn step(len: usize, at: Option<usize>, back: bool) -> usize {
    if len == 0 {
        return 0;
    }
    match (at, back) {
        // A value that is not in the list at all — a family that was uninstalled, a
        // scale written by hand — starts at the end the key is coming from, so the first
        // press moves it into the list rather than to the far side of it.
        (None, false) => 0,
        (None | Some(0), true) => len - 1,
        (Some(at), true) => at - 1,
        (Some(at), false) => (at + 1) % len,
    }
}

/// The display name of the configured theme.
fn theme_name(config: &Config) -> String {
    zet_config::by_slug(&config.theme)
        .map_or_else(|| config.theme.clone(), |theme| theme.name.into())
}

/// The display name of a text scale.
fn scale_name(scale: f32) -> String {
    TEXT_SCALES
        .iter()
        .find(|(_, value)| (*value - scale).abs() < f32::EPSILON)
        .map_or_else(
            || format!("{}%", (scale * 100.0).round()),
            |(name, _)| (*name).into(),
        )
}

/// `On` or `Off`.
fn on_off(value: bool) -> String {
    if value { "On" } else { "Off" }.to_owned()
}

/// What a setting the system has an answer to reads as: `Follow system`, or `Off`.
///
/// These two are not `on_off`, because `On` beside the label `Reduce motion` says motion
/// is being reduced, and the flag says something else: that zet will do whatever the
/// system asked. On a machine where Windows has asked for nothing those come apart, and
/// the panel would claim the cursor was still while it blinked. The value names the
/// policy instead, which is the thing the toggle actually chooses between.
fn follow(value: bool) -> String {
    if value { "Follow system" } else { "Off" }.to_owned()
}

/// A config value's spelling with its first letter raised.
///
/// The file's spellings are kebab-case and lowercase, which is right for a file and
/// wrong for a label: `hollow-block` is not a cursor shape anyone reads.
fn title_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut raise = true;
    for ch in text.chars() {
        match ch {
            '-' => {
                out.push(' ');
                raise = true;
            }
            _ if raise => {
                out.extend(ch.to_uppercase());
                raise = false;
            }
            _ => out.push(ch),
        }
    }
    out
}

/// The label for a binding row, from the action's file name.
fn action_name(action: Action) -> &'static str {
    match action {
        Action::NewTab => "New tab",
        Action::CloseTab => "Close tab",
        Action::NextTab => "Next tab",
        Action::PreviousTab => "Previous tab",
        Action::NewWindow => "New window",
        Action::Copy => "Copy",
        Action::Paste => "Paste",
        Action::Find => "Find",
        Action::Settings => "Settings",
        Action::ToggleTabPosition => "Tab position",
        Action::ScrollPageUp => "Scroll up",
        Action::ScrollPageDown => "Scroll down",
        Action::ScrollToTop => "Scroll to top",
        Action::ScrollToBottom => "Scroll to bottom",
        Action::FontLarger => "Font larger",
        Action::FontSmaller => "Font smaller",
        Action::FontReset => "Font reset",
        Action::Quit => "Quit",
    }
}

/// A setting row.
fn setting(id: Id, label: &'static str, value: String, kind: Kind) -> Line {
    Line::Setting(Setting {
        id,
        label,
        value,
        kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::parse_bindings;

    fn config() -> Config {
        Config::default()
    }

    fn at(lines: &[Line], id: Id) -> &str {
        lines
            .iter()
            .find(|line| line.id() == Some(id))
            .map(Line::value)
            .expect("the row is in the panel")
    }

    #[test]
    fn every_action_has_a_row_and_every_row_names_an_action() {
        let config = config();
        let lines = lines(&config, &parse_bindings(&config));
        for action in Action::ALL {
            let row = lines
                .iter()
                .find(|line| line.id() == Some(Id::Binding(action)));
            let row = row.unwrap_or_else(|| panic!("{action:?} has no row"));
            assert!(
                !row.text().is_empty(),
                "{action:?}'s row has no label, so it draws as a blank line"
            );
        }
        let rows = lines
            .iter()
            .filter(|line| matches!(line.id(), Some(Id::Binding(_))))
            .count();
        assert_eq!(rows, Action::ALL.len());
    }

    #[test]
    fn an_imported_palette_says_it_ships_as_published() {
        // PRODUCT.md's seventh criterion: imported palettes "ship unmodified so they look
        // like themselves, and the theme picker says so". DESIGN.md gives the reason the
        // sentence exists at all — a picker that showed them beside zet's own without a
        // word would imply they had been checked for contrast, and they have not been,
        // and could not be without changing them.
        //
        // `Theme::published` has been set correctly on all eight since the themes were
        // written and read by nothing, so the promise was kept by a field no user saw.
        fn theme_row(config: &Config) -> (String, String) {
            let rows = lines(config, &parse_bindings(config));
            let row = rows
                .iter()
                .find(|line| line.id() == Some(Id::Theme))
                .expect("the row is in the panel");
            (row.text().to_owned(), row.value().to_owned())
        }

        let mut imported = config();
        imported.theme = "nord".into();
        assert_eq!(
            theme_row(&imported),
            (
                "Theme (as published)".to_owned(),
                "Nord".to_owned(),
                // The name is still the value and still readable: the marker is on the
                // label because the value's column is 118px — wide enough for a chord —
                // and the painter draws a run of text without clipping it, so a suffix
                // here would run out of the panel and over the grid.
            )
        );

        // And one of zet's own carries nothing, because there is nothing to say: zet
        // checked these, and DESIGN.md's contrast table is where that is written down.
        assert_eq!(
            theme_row(&config()),
            ("Theme".to_owned(), "zet dark".to_owned())
        );
    }

    #[test]
    fn the_value_a_row_shows_is_the_value_in_the_file() {
        let mut config = config();
        config.cursor.shape = CursorShape::Bar;
        config.cursor.thickness = 3;
        config.cursor.blink = false;
        config.appearance.text_scale = 1.5;
        config.window.opacity = 0.85;
        let lines = lines(&config, &parse_bindings(&config));
        assert_eq!(at(&lines, Id::WindowOpacity), "85%");
        assert_eq!(at(&lines, Id::CursorShape), "Bar");
        assert_eq!(at(&lines, Id::CursorThickness), "3 px");
        assert_eq!(at(&lines, Id::CursorBlink), "Off");
        assert_eq!(at(&lines, Id::TextScale), "150%");
    }

    #[test]
    fn a_setting_the_system_has_an_answer_to_says_whose_answer_it_is() {
        // `On` here would be a claim about the machine rather than about zet: the flag
        // says "do what the system said", and a system that asked for nothing would
        // leave the panel saying motion was reduced while the cursor blinked.
        let mut config = config();
        let following = lines(&config, &parse_bindings(&config));
        assert_eq!(at(&following, Id::ReduceMotion), "Follow system");
        assert_eq!(at(&following, Id::ForcedColors), "Follow system");

        config.appearance.follow_reduce_motion = false;
        config.appearance.follow_forced_colors = false;
        let ignoring = lines(&config, &parse_bindings(&config));
        assert_eq!(at(&ignoring, Id::ReduceMotion), "Off");
        assert_eq!(at(&ignoring, Id::ForcedColors), "Off");
    }

    #[test]
    fn a_block_cursor_has_no_thickness_row() {
        let mut config = config();
        config.cursor.shape = CursorShape::Block;
        let block = lines(&config, &parse_bindings(&config));
        assert!(
            !block
                .iter()
                .any(|line| line.id() == Some(Id::CursorThickness))
        );

        config.cursor.shape = CursorShape::Underline;
        let underline = lines(&config, &parse_bindings(&config));
        assert!(
            underline
                .iter()
                .any(|line| line.id() == Some(Id::CursorThickness))
        );
    }

    #[test]
    fn a_toggle_flips_either_way_and_a_choice_does_not() {
        let mut config = config();
        let before = config.cursor.blink;
        assert_eq!(
            adjust(&mut config, Id::CursorBlink, false, &[]),
            Effect::Changed
        );
        assert_ne!(config.cursor.blink, before);
        adjust(&mut config, Id::CursorBlink, true, &[]);
        assert_eq!(config.cursor.blink, before, "a toggle has no direction");

        let shape = config.cursor.shape;
        adjust(&mut config, Id::CursorShape, false, &[]);
        let forward = config.cursor.shape;
        assert_ne!(forward, shape);
        adjust(&mut config, Id::CursorShape, true, &[]);
        assert_eq!(config.cursor.shape, shape, "back undoes forward");
    }

    #[test]
    fn a_stepper_stops_at_its_ends_rather_than_wrapping() {
        let mut config = config();
        config.font.size = MAX_SIZE;
        for _ in 0..10 {
            adjust(&mut config, Id::FontSize, false, &[]);
        }
        assert!((config.font.size - MAX_SIZE).abs() < f32::EPSILON);
        config.font.size = MIN_SIZE;
        for _ in 0..10 {
            adjust(&mut config, Id::FontSize, true, &[]);
        }
        assert!((config.font.size - MIN_SIZE).abs() < f32::EPSILON);
    }

    #[test]
    fn the_opacity_stepper_stops_short_of_a_window_nobody_can_see() {
        // The floor is the interesting end. A window at zero is invisible, and the panel
        // that would raise it is drawn in the window: the value the stepper will not go
        // below is what keeps the control reachable, which is a fact about the panel and
        // not about the schema. The file may still say what it likes.
        let mut config = config();
        for _ in 0..20 {
            adjust(&mut config, Id::WindowOpacity, true, &[]);
        }
        assert!((config.window.opacity - 0.2).abs() < f32::EPSILON);

        // And from a value the file set below the floor, the way out is up.
        config.window.opacity = 0.05;
        adjust(&mut config, Id::WindowOpacity, false, &[]);
        assert!((config.window.opacity - 0.2).abs() < f32::EPSILON);

        for _ in 0..20 {
            adjust(&mut config, Id::WindowOpacity, false, &[]);
        }
        assert!((config.window.opacity - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn the_opacity_stepper_lands_on_the_value_it_started_from() {
        // Which is what whole percents are for. A tenth is not a tenth in binary, and a
        // stepper that added one at a time would come back to 0.7999999 and write that
        // into the file — a value that reads as a bug in the panel.
        let mut config = config();
        config.window.opacity = 0.9;
        for _ in 0..3 {
            adjust(&mut config, Id::WindowOpacity, true, &[]);
        }
        for _ in 0..3 {
            adjust(&mut config, Id::WindowOpacity, false, &[]);
        }
        assert!((config.window.opacity - 0.9).abs() < f32::EPSILON);
        assert_eq!(configured_opacity(&config), 90);
    }

    #[test]
    fn every_choice_answers_both_ways_from_every_value_it_offers() {
        // A ring is the one place an off-by-one hides: a list entered at the wrong end
        // still looks right until the user presses the other key.
        for id in [Id::Theme, Id::TextScale, Id::CursorShape, Id::TabPosition] {
            let mut config = config();
            let start = lines(&config, &parse_bindings(&config))
                .iter()
                .find(|line| line.id() == Some(id))
                .map(|line| line.value().to_owned())
                .expect("the row exists");
            adjust(&mut config, id, true, &[]);
            let back = lines(&config, &parse_bindings(&config))
                .iter()
                .find(|line| line.id() == Some(id))
                .map(|line| line.value().to_owned())
                .expect("the row exists");
            assert_ne!(back, start, "{id:?} did not move backwards");
            adjust(&mut config, id, false, &[]);
            let forward = lines(&config, &parse_bindings(&config))
                .iter()
                .find(|line| line.id() == Some(id))
                .map(|line| line.value().to_owned())
                .expect("the row exists");
            assert_eq!(forward, start, "{id:?} did not come back");
        }
    }

    #[test]
    fn a_family_the_machine_does_not_have_is_still_the_one_in_the_file() {
        let mut config = config();
        config.font.family = "No Such Mono".into();
        let list = candidates(&config, &["Cascadia Mono".to_owned()]);
        assert_eq!(list[0], "No Such Mono");
        // And the row reads what the file says rather than what the machine has. The
        // family is offered first *and* shown, because the two are the same question: a
        // panel that listed the installed families under a name the file does not contain
        // would be describing a setting that is not the one in force.
        let row = lines(&config, &parse_bindings(&config))
            .into_iter()
            .find(|line| line.id() == Some(Id::Font))
            .expect("the font row");
        assert_eq!(row.value(), "No Such Mono");
    }

    #[test]
    fn a_problem_with_the_file_is_a_row_with_nothing_to_set() {
        // The loader has always found these and had nowhere to put them: `App::diagnostics`
        // was public, held the answers, and was called by nothing, so a user who typed
        // `cursor.thickness = 99` got the default value back with no way at all to find out
        // why the row they set did nothing. This is the surface — the panel is where
        // someone who is changing settings is standing, and the file is the setting.
        let rows = problems(&[
            Diagnostic {
                severity: Severity::Warning,
                message: "cursor.thickness is outside 1 to 8, using 2".to_owned(),
            },
            Diagnostic {
                severity: Severity::Error,
                message: "theme \"nordd\" is not a theme, using zet-dark".to_owned(),
            },
        ]);

        assert_eq!(rows.first().map(Line::text), Some("Problems"));
        let notes: Vec<&Line> = rows
            .iter()
            .filter(|row| matches!(row, Line::Note(_)))
            .collect();
        for note in &notes {
            assert!(
                note.kind().is_none() && note.id().is_none(),
                "a problem is a row to read, not a row to set"
            );
        }
        assert_eq!(
            notes
                .iter()
                .map(|note| (note.text(), note.value()))
                .collect::<Vec<_>>(),
            vec![
                ("cursor.thickness is outside 1 to 8, using 2", "warning"),
                ("theme \"nordd\" is not a theme, using zet-dark", "error"),
            ],
            "the severity is the column the configuration reference documents"
        );
    }

    #[test]
    fn a_file_with_nothing_wrong_gets_no_problems_section() {
        // An empty heading is a section that says nothing and takes a row to say it, and a
        // panel that always opened with "Problems" over nothing would train the user to
        // ignore the one time it mattered.
        assert!(problems(&[]).is_empty());
    }

    #[test]
    fn binding_a_chord_moves_it_off_whatever_ran_it_before() {
        let mut config = config();
        let chord = Chord::parse("Ctrl+Shift+K").expect("a parseable chord");
        bind(&mut config, Action::NewTab, chord);
        let bound = parse_bindings(&config);
        let mine: Vec<Action> = bound
            .iter()
            .filter(|(c, _)| *c == chord)
            .map(|(_, a)| *a)
            .collect();
        assert_eq!(mine, vec![Action::NewTab]);
        assert_eq!(
            bound.iter().filter(|(_, a)| *a == Action::NewTab).count(),
            1,
            "an action bound twice runs from whichever chord the parse reached first"
        );
    }

    #[test]
    fn an_action_a_chord_was_taken_from_is_unbound_in_the_file_and_not_merely_absent() {
        // The file's `[keys]` table is a patch on the defaults: an action it does not
        // name is an action that keeps the shipped chord. So dropping the line is not
        // taking the key away, it is taking it away until the next start — the panel says
        // `Unbound`, the user restarts, and the action they took the key from is holding
        // a chord again. The line has to stay and say nothing.
        let mut config = config();
        let chord = Chord::parse("Ctrl+Shift+W").expect("a parseable chord");
        bind(&mut config, Action::Find, chord);

        assert_eq!(
            config.keys.get("close-tab").map(String::as_str),
            Some(""),
            "the displaced action was dropped from the table rather than emptied"
        );
        assert_eq!(
            config.keys.get("find").map(String::as_str),
            Some("Ctrl+Shift+W")
        );
        // And the panel the user is looking at says the same thing the file will.
        let shown = lines(&config, &parse_bindings(&config));
        let row = shown
            .iter()
            .find_map(|line| match line {
                Line::Setting(setting) if setting.id == Id::Binding(Action::CloseTab) => {
                    Some(setting.value.clone())
                }
                _ => None,
            })
            .expect("the panel has a row for every action");
        assert_eq!(row, "Unbound");
    }

    #[test]
    fn binding_a_chord_takes_it_off_a_line_spelled_differently() {
        // The file is hand-editable and a chord has more than one spelling. The copy
        // entry below names the same chord the panel is about to bind, so the paste
        // entry has to take it: leaving both in place gives one chord two actions, and
        // which of them runs is decided by the order of a map rather than by the user.
        // Three spellings of two keystrokes: the letter's case, the order of the
        // modifiers, and the punctuation the comma key produces under shift. Each is
        // the keystroke the panel is about to bind — `Chord::matches` says so and the
        // keymap dispatches on it — and each has to lose the entry.
        for (spelling, chosen) in [
            ("ctrl+shift+p", "Ctrl+Shift+P"),
            ("Shift+Ctrl+P", "Ctrl+Shift+P"),
            ("Ctrl+Shift+Comma", "Ctrl+Shift+<"),
        ] {
            let mut config = config();
            let chord = Chord::parse(chosen).expect("a parseable chord");
            config.keys.insert("copy".into(), spelling.into());
            bind(&mut config, Action::Paste, chord);

            let bound = parse_bindings(&config);
            let mine: Vec<Action> = bound
                .iter()
                .filter(|(c, _)| *c == chord)
                .map(|(_, a)| *a)
                .collect();
            assert_eq!(
                mine,
                vec![Action::Paste],
                "`copy = \"{spelling}\"` and `paste = \"{chosen}\"` are one keystroke, \
                 and both of them were left in the file"
            );
            assert_eq!(
                bound.iter().filter(|(_, a)| *a == Action::Copy).count(),
                0,
                "copy was left holding the chord the panel just gave to paste"
            );
        }
    }

    #[test]
    fn a_binding_row_shows_the_chord_that_actually_runs_it() {
        let mut config = config();
        bind(
            &mut config,
            Action::Quit,
            Chord::parse("Ctrl+Q").expect("a parseable chord"),
        );
        let lines = lines(&config, &parse_bindings(&config));
        assert_eq!(at(&lines, Id::Binding(Action::Quit)), "Ctrl+Q");
    }

    #[test]
    fn the_switches_the_panel_shows_are_the_switches_the_file_gets() {
        // The panel's promise is that it is a view over the file, and a field with no row
        // is a field the user can only reach by hand. That is the right answer for a value
        // that is a path — the four controls cannot type one — and the wrong answer for a
        // switch, which is the one kind of setting the controls are exactly shaped for.
        let mut config = config();
        config.window.remember_position = false;
        config.window.start_maximized = true;
        config.tabs.open_default_without_asking = false;
        let rows = lines(&config, &parse_bindings(&config));
        assert_eq!(at(&rows, Id::RememberPosition), "Off");
        assert_eq!(at(&rows, Id::StartMaximized), "On");
        assert_eq!(at(&rows, Id::OpenWithoutAsking), "Off");

        let before = (
            config.window.remember_position,
            config.window.start_maximized,
            config.tabs.open_default_without_asking,
        );
        for id in [
            Id::RememberPosition,
            Id::StartMaximized,
            Id::OpenWithoutAsking,
        ] {
            assert_eq!(adjust(&mut config, id, false, &[]), Effect::Changed);
        }
        assert_eq!(
            (
                config.window.remember_position,
                config.window.start_maximized,
                config.tabs.open_default_without_asking,
            ),
            (!before.0, !before.1, !before.2),
            "every switch moved, and moved the way the file would show it"
        );
    }

    #[test]
    fn a_toggle_has_no_direction() {
        let mut config = config();
        let before = config.window.start_maximized;
        adjust(&mut config, Id::StartMaximized, false, &[]);
        assert_ne!(config.window.start_maximized, before);
        adjust(&mut config, Id::StartMaximized, true, &[]);
        assert_eq!(config.window.start_maximized, before);
    }

    #[test]
    fn a_picture_the_file_names_is_a_row_the_panel_shows_and_can_step_away_from() {
        // The panel cannot type a path, so it does not offer the kind — but a file that
        // names one is a file the panel has to describe, and stepping away from it is a
        // change the panel is allowed to make.
        let mut config = config();
        config.window.background = Background::Image {
            path: "Z:/pictures/terminal.png".into(),
            opacity: 0.5,
        };
        let shown = lines(&config, &parse_bindings(&config));
        assert_eq!(at(&shown, Id::Background), "Image");
        assert_eq!(at(&shown, Id::ImageOpacity), "50%");

        assert_eq!(
            adjust(&mut config, Id::Background, false, &[]),
            Effect::Changed
        );
        assert_eq!(
            config.window.background,
            Background::Solid,
            "forward from a picture is the flat ground the theme paints"
        );
        let after = lines(&config, &parse_bindings(&config));
        assert!(
            !after.iter().any(|line| line.id() == Some(Id::ImageOpacity)),
            "the row about a picture goes with the picture"
        );
    }

    #[test]
    fn the_kinds_the_background_row_offers_are_the_ones_the_panel_can_write_whole() {
        // Every value the row can step to has to be a configuration the loader will not
        // complain about on the next start. `Image` needs a path, a path is not one of the
        // four controls, and a panel that offered the kind anyway would write a file whose
        // one problem row is a problem the user did not cause — and cannot fix from where
        // they are standing.
        let mut config = config();
        for _ in 0..6 {
            adjust(&mut config, Id::Background, false, &[]);
            assert!(
                !matches!(config.window.background, Background::Image { .. }),
                "the panel wrote a picture it cannot name: {:?}",
                config.window.background
            );
        }
    }

    #[test]
    fn a_gradient_the_panel_makes_is_made_of_the_themes_own_colours() {
        // A gradient is a background, and a background is the theme's. The panel seeds the
        // two stops from the theme the file names rather than inventing a pair of colours,
        // so clicking the row cannot produce a terminal that disagrees with its own theme.
        // The pair is a starting point rather than a decision: both are the file's to
        // change, and a hand-written pair is what the row will show.
        let mut config = config();
        config.theme = "nord".into();
        assert_eq!(
            adjust(&mut config, Id::Background, false, &[]),
            Effect::Changed
        );
        let theme = zet_config::by_slug("nord").expect("zet ships nord");
        assert_eq!(
            config.window.background,
            Background::Gradient {
                from: theme.background,
                to: theme.selection,
                angle: 90.0
            }
        );
    }

    #[test]
    fn stepping_back_from_the_flat_ground_reaches_the_gradient_and_not_the_other_way_round() {
        let mut config = config();
        adjust(&mut config, Id::Background, true, &[]);
        assert!(matches!(
            config.window.background,
            Background::Gradient { .. }
        ));
        adjust(&mut config, Id::Background, false, &[]);
        assert_eq!(
            config.window.background,
            Background::Solid,
            "and back again"
        );
    }

    #[test]
    fn a_row_that_belongs_to_one_kind_is_not_drawn_for_another() {
        let mut config = config();
        let solid = lines(&config, &parse_bindings(&config));
        assert_eq!(at(&solid, Id::Background), "Solid");
        for id in [Id::ImageOpacity, Id::GradientAngle] {
            assert!(
                !solid.iter().any(|line| line.id() == Some(id)),
                "{id:?} is about a kind the file is not in"
            );
        }

        config.window.background = Background::Gradient {
            from: zet_config::Rgb::new(0, 0, 0),
            to: zet_config::Rgb::new(255, 255, 255),
            angle: 90.0,
        };
        let gradient = lines(&config, &parse_bindings(&config));
        assert_eq!(at(&gradient, Id::Background), "Gradient");
        assert_eq!(at(&gradient, Id::GradientAngle), "90°");
        assert!(
            !gradient
                .iter()
                .any(|line| line.id() == Some(Id::ImageOpacity))
        );
    }

    #[test]
    fn the_angle_stepper_stops_at_its_ends_rather_than_wrapping() {
        let mut config = config();
        adjust(&mut config, Id::Background, false, &[]);
        for _ in 0..40 {
            adjust(&mut config, Id::GradientAngle, false, &[]);
        }
        let Background::Gradient { angle, .. } = config.window.background else {
            panic!("the row exists, so the background is a gradient");
        };
        assert!((angle - MAX_ANGLE).abs() < f32::EPSILON, "got {angle}");

        for _ in 0..40 {
            adjust(&mut config, Id::GradientAngle, true, &[]);
        }
        let Background::Gradient { angle, .. } = config.window.background else {
            panic!("the row exists, so the background is a gradient");
        };
        assert!(angle.abs() < f32::EPSILON, "got {angle}");
    }

    #[test]
    fn an_angle_the_file_could_not_have_written_is_still_a_row_that_can_be_stepped() {
        // The file is hand-editable and a number in it can be anything, `nan` included —
        // TOML spells it. A row that read `NaN°` and whose stepper then wrote the NaN back
        // is a row the user cannot get out of.
        let mut config = config();
        config.window.background = Background::Gradient {
            from: zet_config::Rgb::new(0, 0, 0),
            to: zet_config::Rgb::new(255, 255, 255),
            angle: f32::NAN,
        };
        let rows = lines(&config, &parse_bindings(&config));
        assert_eq!(at(&rows, Id::GradientAngle), "0°");
        adjust(&mut config, Id::GradientAngle, false, &[]);
        let Background::Gradient { angle, .. } = config.window.background else {
            panic!("the row exists, so the background is a gradient");
        };
        assert!(
            angle.is_finite(),
            "the stepper wrote a number back: {angle}"
        );
    }

    #[test]
    fn the_picture_opacity_stepper_stops_short_of_a_picture_nobody_can_see() {
        // The same floor as the window's, for the same reason: a picture at nothing is a
        // picture the user cannot find again, and the row that would raise it is drawn over
        // the terminal the picture is behind.
        let mut config = config();
        config.window.background = Background::Image {
            path: "Z:/pictures/terminal.png".into(),
            opacity: 0.5,
        };
        for _ in 0..30 {
            adjust(&mut config, Id::ImageOpacity, true, &[]);
        }
        let Background::Image { opacity, .. } = config.window.background else {
            panic!("the row exists, so the background is a picture");
        };
        assert!((opacity - MIN_IMAGE_OPACITY as f32 / 100.0).abs() < f32::EPSILON);

        for _ in 0..30 {
            adjust(&mut config, Id::ImageOpacity, false, &[]);
        }
        let Background::Image { opacity, .. } = config.window.background else {
            panic!("the row exists, so the background is a picture");
        };
        assert!((opacity - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn every_id_has_a_row_that_can_be_found_again() {
        // The panel is clicked by identity. An id with no row is a setting the user
        // cannot reach, and a row whose id is missing is a click that goes nowhere.
        let mut config = config();
        config.cursor.shape = CursorShape::Bar;
        let lines = lines(&config, &parse_bindings(&config));
        for id in [
            Id::Theme,
            Id::TextScale,
            Id::ReduceMotion,
            Id::ForcedColors,
            Id::WindowOpacity,
            Id::TabPosition,
            Id::Font,
            Id::FontSize,
            Id::CursorShape,
            Id::CursorBlink,
            Id::CursorThickness,
            Id::Background,
            Id::RememberPosition,
            Id::StartMaximized,
            Id::OpenWithoutAsking,
        ] {
            assert!(
                lines.iter().any(|line| line.id() == Some(id)),
                "{id:?} has no row"
            );
        }
    }

    #[test]
    fn the_heading_text_is_not_a_value_that_could_be_clicked() {
        let config = config();
        for line in lines(&config, &parse_bindings(&config)) {
            if let Line::Heading(text) = line {
                assert!(line.kind().is_none());
                assert!(line.id().is_none());
                assert_eq!(line.value(), "");
                assert!(!text.is_empty());
            }
        }
    }
}
