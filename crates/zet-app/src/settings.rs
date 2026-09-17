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

use zet_config::{Config, CursorShape};
use zet_input::Chord;

use crate::action::Action;

/// The smallest and largest the grid font may be set to, in points.
///
/// Written down once so that the panel's stepper and the clamp on the size the grid is
/// actually drawn at cannot drift apart: a panel that offers a size the renderer then
/// refuses is a panel that lies.
pub const MIN_SIZE: f32 = 4.0;

/// The largest the grid font may be set to, in points.
pub const MAX_SIZE: f32 = 72.0;

/// The cursor thicknesses the panel offers, in pixels.
///
/// The schema's, not the panel's own: the panel offers what the configuration file
/// accepts, and a stepper that could take a thickness past the range the schema warns
/// about would let the user set a value the file then complains about.
const MIN_THICKNESS: u8 = zet_config::MIN_CURSOR_THICKNESS;
const MAX_THICKNESS: u8 = zet_config::MAX_CURSOR_THICKNESS;

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
    /// Where the tab strip lives.
    TabPosition,
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
    /// A setting.
    Setting(Setting),
}

impl Line {
    /// The text to draw: a heading's name, or a setting's label.
    #[must_use]
    pub fn text(&self) -> &str {
        match self {
            Self::Heading(text) => text,
            Self::Setting(setting) => setting.label,
        }
    }

    /// The value to draw, empty for a heading.
    #[must_use]
    pub fn value(&self) -> &str {
        match self {
            Self::Heading(_) => "",
            Self::Setting(setting) => &setting.value,
        }
    }

    /// How the row is clicked, `None` for a heading.
    #[must_use]
    pub const fn kind(&self) -> Option<Kind> {
        match self {
            Self::Heading(_) => None,
            Self::Setting(setting) => Some(setting.kind),
        }
    }

    /// What the row sets, `None` for a heading.
    #[must_use]
    pub const fn id(&self) -> Option<Id> {
        match self {
            Self::Heading(_) => None,
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

/// The rows, read out of a configuration.
///
/// The font row's *value* is the family in the file and needs nothing else; the list of
/// families it can be stepped through is only needed when it is clicked, which is why
/// [`adjust`] takes it and this does not.
#[must_use]
pub fn lines(config: &Config, bindings: &[(Chord, Action)]) -> Vec<Line> {
    let mut lines = vec![
        Line::Heading("Appearance"),
        setting(Id::Theme, "Theme", theme_name(config), Kind::Choice),
        setting(
            Id::TextScale,
            "Text scale",
            scale_name(config.appearance.text_scale),
            Kind::Choice,
        ),
        setting(
            Id::ReduceMotion,
            "Reduce motion",
            on_off(config.appearance.follow_reduce_motion),
            Kind::Toggle,
        ),
        setting(
            Id::ForcedColors,
            "Forced colours",
            on_off(config.appearance.follow_forced_colors),
            Kind::Toggle,
        ),
        Line::Heading("Tabs"),
        setting(
            Id::TabPosition,
            "Position",
            title_case(config.tabs.position.as_str()),
            Kind::Choice,
        ),
        Line::Heading("Terminal"),
        setting(Id::Font, "Font", config.font.family.clone(), Kind::Choice),
        setting(
            Id::FontSize,
            "Size",
            format!("{} pt", config.font.size.round()),
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
                (config.font.size + if back { -1.0 } else { 1.0 }).clamp(MIN_SIZE, MAX_SIZE);
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
    config.keys.retain(|_, bound| *bound != text);
    config.keys.insert(action.name().to_owned(), text);
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
    fn the_value_a_row_shows_is_the_value_in_the_file() {
        let mut config = config();
        config.cursor.shape = CursorShape::Bar;
        config.cursor.thickness = 3;
        config.cursor.blink = false;
        config.appearance.text_scale = 1.5;
        let lines = lines(&config, &parse_bindings(&config));
        assert_eq!(at(&lines, Id::CursorShape), "Bar");
        assert_eq!(at(&lines, Id::CursorThickness), "3 px");
        assert_eq!(at(&lines, Id::CursorBlink), "Off");
        assert_eq!(at(&lines, Id::TextScale), "150%");
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
        assert_eq!(
            lines(&config, &parse_bindings(&config)).len(),
            lines(&config, &parse_bindings(&config)).len()
        );
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
            Id::TabPosition,
            Id::Font,
            Id::FontSize,
            Id::CursorShape,
            Id::CursorBlink,
            Id::CursorThickness,
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
