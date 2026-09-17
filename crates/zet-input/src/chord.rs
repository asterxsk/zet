//! Keys, modifiers, and the text form a keybinding is written in.

use std::fmt;
use std::str::FromStr;

use bitflags::bitflags;

bitflags! {
    /// The modifiers held when a key was pressed.
    ///
    /// The bit values are this crate's own and are assigned in the order a person
    /// writes them, so that reading the declaration gives the same order as
    /// [`Chord`]'s `Display`. They are *not* the values that go on the wire: xterm
    /// numbers its modifiers `Shift` 4, `Alt` 8, `Control` 16, off by two from
    /// anything natural, and that translation lives in `encode::modifier_param`
    /// where the sequence being built is visible next to it.
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
    pub struct Modifiers: u8 {
        /// Shift.
        const SHIFT = 1 << 0;
        /// Control.
        const CTRL = 1 << 1;
        /// Alt.
        const ALT = 1 << 2;
        /// The Windows key.
        const SUPER = 1 << 3;
    }
}

/// A key, as something a person can name.
///
/// This is the config file's vocabulary before it is the encoder's. It carries
/// [`Key::Char`] for the character a layout produced *and* the named punctuation
/// variants the layout could equally have produced, because the two arrive on
/// different paths: a keyboard event names `Minus`, an IME commit carries `-`. Both
/// have to round-trip through text, so `name`/`from_name` accept either spelling and
/// the encoder treats them identically — which is the only reason [`Key::Char`] and
/// [`Key::Minus`] can coexist without a rule for preferring one.
///
/// `F` holds a `u8` rather than listing twenty-four variants because twenty-four
/// variants would each need a doc comment, a match arm in `name`, and a match arm in
/// `from_name`, and would buy nothing: nothing matches on an individual function key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Key {
    /// The character the layout produced, already shifted and composed.
    Char(char),
    /// Return.
    Enter,
    /// Tab.
    Tab,
    /// Backspace.
    Backspace,
    /// Escape.
    Escape,
    /// The space bar.
    Space,
    /// Forward delete.
    Delete,
    /// Insert.
    Insert,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// The up arrow.
    Up,
    /// The down arrow.
    Down,
    /// The left arrow.
    Left,
    /// The right arrow.
    Right,
    /// A function key, `1` to `24`.
    F(u8),
    /// Caps lock itself, not the state it toggles.
    CapsLock,
    /// Num lock itself.
    NumLock,
    /// Scroll lock itself.
    ScrollLock,
    /// Print screen.
    PrintScreen,
    /// Pause.
    Pause,
    /// The context menu key.
    Menu,
    /// The `+` key.
    Plus,
    /// The `-` key.
    Minus,
    /// The `,` key.
    Comma,
    /// The `.` key.
    Period,
    /// The `/` key.
    Slash,
    /// The `;` key.
    Semicolon,
    /// The `'` key.
    Quote,
    /// The `` ` `` key.
    Backquote,
    /// The `\` key.
    Backslash,
    /// The `[` key.
    BracketLeft,
    /// The `]` key.
    BracketRight,
    /// The `=` key.
    Equal,
}

impl Key {
    /// The key's canonical name.
    ///
    /// A character names itself, which is what makes `Ctrl+Shift+T` readable, and
    /// every other key uses the spelling its variant does. `name` and
    /// `from_name` are inverses, so a key written into the config file and read back
    /// is the same key — see the round-trip test.
    #[must_use]
    pub fn name(self) -> String {
        match self {
            Key::Char(c) => c.to_string(),
            Key::Enter => "Enter".into(),
            Key::Tab => "Tab".into(),
            Key::Backspace => "Backspace".into(),
            Key::Escape => "Escape".into(),
            Key::Space => "Space".into(),
            Key::Delete => "Delete".into(),
            Key::Insert => "Insert".into(),
            Key::Home => "Home".into(),
            Key::End => "End".into(),
            Key::PageUp => "PageUp".into(),
            Key::PageDown => "PageDown".into(),
            Key::Up => "Up".into(),
            Key::Down => "Down".into(),
            Key::Left => "Left".into(),
            Key::Right => "Right".into(),
            Key::F(n) => format!("F{n}"),
            Key::CapsLock => "CapsLock".into(),
            Key::NumLock => "NumLock".into(),
            Key::ScrollLock => "ScrollLock".into(),
            Key::PrintScreen => "PrintScreen".into(),
            Key::Pause => "Pause".into(),
            Key::Menu => "Menu".into(),
            Key::Plus => "Plus".into(),
            Key::Minus => "Minus".into(),
            Key::Comma => "Comma".into(),
            Key::Period => "Period".into(),
            Key::Slash => "Slash".into(),
            Key::Semicolon => "Semicolon".into(),
            Key::Quote => "Quote".into(),
            Key::Backquote => "Backquote".into(),
            Key::Backslash => "Backslash".into(),
            Key::BracketLeft => "BracketLeft".into(),
            Key::BracketRight => "BracketRight".into(),
            Key::Equal => "Equal".into(),
        }
    }

    /// The key named by `name`, or `None` if nothing is named that.
    ///
    /// Names are matched without regard to case, and a single character is taken as
    /// that character, which is what lets `Ctrl+shift+t` and `Ctrl+Shift+T` both be
    /// written. Case does not imply Shift: `Shift` is written out, because a
    /// keybinding should say what it means and a capital letter in a config file is
    /// too easy to lose to a well-meaning editor.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let mut chars = name.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            return Some(Key::Char(c));
        }
        let lowered = name.to_ascii_lowercase();
        Some(match lowered.as_str() {
            "enter" => Key::Enter,
            "tab" => Key::Tab,
            "backspace" => Key::Backspace,
            "escape" => Key::Escape,
            "space" => Key::Space,
            "delete" => Key::Delete,
            "insert" => Key::Insert,
            "home" => Key::Home,
            "end" => Key::End,
            "pageup" => Key::PageUp,
            "pagedown" => Key::PageDown,
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            "capslock" => Key::CapsLock,
            "numlock" => Key::NumLock,
            "scrolllock" => Key::ScrollLock,
            "printscreen" => Key::PrintScreen,
            "pause" => Key::Pause,
            "menu" => Key::Menu,
            "plus" => Key::Plus,
            "minus" => Key::Minus,
            "comma" => Key::Comma,
            "period" => Key::Period,
            "slash" => Key::Slash,
            "semicolon" => Key::Semicolon,
            "quote" => Key::Quote,
            "backquote" => Key::Backquote,
            "backslash" => Key::Backslash,
            "bracketleft" => Key::BracketLeft,
            "bracketright" => Key::BracketRight,
            "equal" => Key::Equal,
            _ => {
                // `F1` through `F24`, and nothing else may start with an `f`: a name
                // that gets this far and parses as a number in range is a function
                // key, and anything else is a name this crate does not know.
                let n: u8 = lowered.strip_prefix('f')?.parse().ok()?;
                if n == 0 || n > 24 {
                    return None;
                }
                Key::F(n)
            }
        })
    }
}

/// A key together with the modifiers that must be held for it to fire.
///
/// The pair is stored rather than the two being asked for separately because the
/// whole point of the type is the text form: [`Chord::parse`] takes `Ctrl+Shift+T`
/// out of the config file and [`fmt::Display`] writes it back, so the app can show
/// a user what a binding is without a second vocabulary for reading them.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Chord {
    /// The modifiers that must be held.
    pub mods: Modifiers,
    /// The key that must be pressed.
    pub key: Key,
}

impl Chord {
    /// Read a chord out of its text form.
    ///
    /// Modifiers are `Ctrl`, `Shift`, `Alt` and `Super`, in any order and any case,
    /// joined to each other and to the key by `+`. Anything before the first token
    /// that is not a modifier is taken as the key, so the `+` key itself still works
    /// (`Ctrl++`), which splitting on every `+` would otherwise break.
    ///
    /// # Errors
    ///
    /// Returns [`ParseChordError`] when there is no key, when the key is not one this
    /// crate knows, or when the first token is neither — a misspelled modifier is
    /// reported as an unknown key, since at that position the two are the same
    /// mistake: `Foo+T` and `Ctrl+NotAKey` are both a typo somewhere.
    pub fn parse(text: &str) -> Result<Self, ParseChordError> {
        let parts: Vec<&str> = text.split('+').collect();
        let mut mods = Modifiers::empty();
        let mut at = 0;
        while at < parts.len() {
            let Some(flag) = modifier_named(parts[at].trim()) else {
                break;
            };
            mods |= flag;
            at += 1;
        }
        let rest = parts[at..].join("+");
        let key = Key::from_name(rest.trim()).ok_or_else(|| {
            ParseChordError::new(format!(
                "`{text}` does not name a key: `{}` is not one",
                rest.trim()
            ))
        })?;
        Ok(Chord { mods, key })
    }

    /// Whether this chord is what was just pressed.
    ///
    /// The comparison is exact, with one exception, and the exception is about which
    /// half of a key a binding names.
    ///
    /// A binding names a *key*, and the twelve punctuation keys have names of their
    /// own — `Comma`, `Period`, `Slash`. The event for `Shift+,` carries the character
    /// the layout put there, which is `<`, and the event for a chord without shift
    /// carries `,`. Both are the same key on the same board, so under shift a chord
    /// matches the character its key produces as well as the key itself. Without this
    /// `Ctrl+Shift+Comma` — which is what the shipped binding for the settings panel
    /// is called, and what anyone would write — is a binding that can never fire.
    ///
    /// Letters are deliberately not in that. `Ctrl+Shift+T` names
    /// [`Key::Char`]`('T')`, because that is the character the layout produces, and
    /// the event carries `T`; the exact comparison already gets it right. A chord
    /// written with the wrong case is a binding that never fires, which is a better
    /// failure than one that fires on a keystroke the user did not describe.
    #[must_use]
    pub fn matches(&self, mods: Modifiers, key: Key) -> bool {
        if self.mods != mods {
            return false;
        }
        self.key == key || (mods.contains(Modifiers::SHIFT) && self.matches_shifted(key))
    }

    /// Whether `key` is the character shift puts above this chord's key.
    fn matches_shifted(&self, key: Key) -> bool {
        let Key::Char(ch) = key else {
            return false;
        };
        // A letter is its own answer — see the rule in `matches` — and this crate has
        // no table for the rest of the world's layouts either, so what is compared is
        // the character with the shift taken back off it against the character the
        // named key produces. `<` comes back as `,` and meets `Key::Comma`; `1` with
        // shift is `!` and comes back to meet [`Key::Char`]`('1')`.
        if ch.is_alphabetic() {
            return false;
        }
        character_of(self.key).is_some_and(|base| unshifted(ch) == base)
    }
}

/// The character a key produces, for the keys that produce one.
///
/// The named punctuation is mapped back to its character here so that it and the
/// character it names are one fact: `Ctrl+[` and `Ctrl+BracketLeft` are the same
/// keystroke and must come out as the same thing, whether that is the 0x1b the
/// encoder sends or the `[` a binding is matched on.
pub(crate) fn character_of(key: Key) -> Option<char> {
    match key {
        Key::Char(c) => Some(c),
        Key::Space => Some(' '),
        Key::Plus => Some('+'),
        Key::Minus => Some('-'),
        Key::Comma => Some(','),
        Key::Period => Some('.'),
        Key::Slash => Some('/'),
        Key::Semicolon => Some(';'),
        Key::Quote => Some('\''),
        Key::Backquote => Some('`'),
        Key::Backslash => Some('\\'),
        Key::BracketLeft => Some('['),
        Key::BracketRight => Some(']'),
        Key::Equal => Some('='),
        _ => None,
    }
}

/// The unshifted form of a character, which is the one the protocol names the key by.
///
/// The protocol is explicit that the code is the key *before* shift: a program
/// matching a shortcut for `ctrl+shift+a` looks for the code of `a`, and a terminal
/// that sends the code of `A` hands it a chord it will never match. The host passes
/// on the key the layout produced, which under shift is the shifted character, so the
/// shift has to come back off here — and only here, because the bytes the legacy
/// encoding sends are the shifted ones and have to stay that way.
///
/// Letters are exact and the same on every layout: shift on a letter is its upper
/// case wherever there are letters. The rest is the pairing the protocol was defined
/// against, the digits and the punctuation above them on a PC-101 keyboard, which is
/// what every other terminal sends. A character in neither table is sent as itself,
/// which is the right answer for every key shift does not move.
pub(crate) fn unshifted(ch: char) -> char {
    if ch.is_ascii_uppercase() {
        return ch.to_ascii_lowercase();
    }
    match ch {
        '!' => '1',
        '@' => '2',
        '#' => '3',
        '$' => '4',
        '%' => '5',
        '^' => '6',
        '&' => '7',
        '*' => '8',
        '(' => '9',
        ')' => '0',
        '_' => '-',
        '+' => '=',
        '{' => '[',
        '}' => ']',
        '|' => '\\',
        ':' => ';',
        '"' => '\'',
        '<' => ',',
        '>' => '.',
        '?' => '/',
        '~' => '`',
        _ => ch,
    }
}

impl fmt::Display for Chord {
    /// Renders back to `Ctrl+Shift+T`.
    ///
    /// The modifier order here is fixed and is not the order they were written in,
    /// so that a binding edited by hand and the same binding set from the settings
    /// panel produce the same line in the file.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (flag, name) in [
            (Modifiers::CTRL, "Ctrl"),
            (Modifiers::SHIFT, "Shift"),
            (Modifiers::ALT, "Alt"),
            (Modifiers::SUPER, "Super"),
        ] {
            if self.mods.contains(flag) {
                f.write_str(name)?;
                f.write_str("+")?;
            }
        }
        f.write_str(&self.key.name())
    }
}

impl FromStr for Chord {
    type Err = ParseChordError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Chord::parse(text)
    }
}

/// The modifier a token names, or `None` if it names a key instead.
fn modifier_named(token: &str) -> Option<Modifiers> {
    Some(match token.to_ascii_lowercase().as_str() {
        "ctrl" => Modifiers::CTRL,
        "shift" => Modifiers::SHIFT,
        "alt" => Modifiers::ALT,
        "super" => Modifiers::SUPER,
        _ => return None,
    })
}

/// A keybinding that could not be read.
///
/// The message is the whole value of the type: this is shown to a user next to the
/// line in their config file that would not parse, so it names the text it was given
/// rather than describing a category of mistake.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ParseChordError {
    /// What was wrong, in the words the user will read.
    message: String,
}

impl ParseChordError {
    /// Build one. Private: the only producer is [`Chord::parse`].
    fn new(message: impl Into<String>) -> Self {
        ParseChordError {
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(mods: Modifiers, key: Key) -> Chord {
        Chord { mods, key }
    }

    #[test]
    fn parses_the_canonical_spelling() {
        let c = Chord::parse("Ctrl+Shift+T").unwrap();
        assert_eq!(c.mods, Modifiers::CTRL | Modifiers::SHIFT);
        assert_eq!(c.key, Key::Char('T'));
    }

    #[test]
    fn parses_a_lowercase_spelling_too() {
        let c = Chord::parse("ctrl+shift+t").unwrap();
        assert_eq!(c.mods, Modifiers::CTRL | Modifiers::SHIFT);
        assert_eq!(c.key, Key::Char('t'));
    }

    #[test]
    fn modifiers_may_be_written_in_any_order() {
        assert_eq!(
            Chord::parse("Shift+Ctrl+Alt+Super+T").unwrap(),
            Chord::parse("Super+Alt+Ctrl+Shift+T").unwrap()
        );
    }

    #[test]
    fn a_named_punctuation_key_parses_and_renders() {
        let c = Chord::parse("Ctrl+Plus").unwrap();
        assert_eq!(c, chord(Modifiers::CTRL, Key::Plus));
        assert_eq!(c.to_string(), "Ctrl+Plus");
    }

    #[test]
    fn a_function_key_parses_and_renders() {
        let c = Chord::parse("Alt+F4").unwrap();
        assert_eq!(c, chord(Modifiers::ALT, Key::F(4)));
        assert_eq!(c.to_string(), "Alt+F4");
    }

    #[test]
    fn a_digit_key_parses_and_renders() {
        let c = Chord::parse("Ctrl+0").unwrap();
        assert_eq!(c, chord(Modifiers::CTRL, Key::Char('0')));
        assert_eq!(c.to_string(), "Ctrl+0");
    }

    #[test]
    fn the_plus_key_can_be_written_between_pluses() {
        assert_eq!(
            Chord::parse("Ctrl++").unwrap(),
            chord(Modifiers::CTRL, Key::Char('+'))
        );
    }

    #[test]
    fn the_display_form_parses_back_to_the_same_chord() {
        for text in [
            "Ctrl+Shift+T",
            "ctrl+shift+t",
            "Ctrl+Plus",
            "Alt+F4",
            "Ctrl+0",
            "Ctrl++",
            "Super+Alt+Left",
            "F24",
            "Escape",
        ] {
            let parsed = Chord::parse(text).unwrap();
            let printed = parsed.to_string();
            assert_eq!(
                Chord::parse(&printed).unwrap(),
                parsed,
                "`{text}` printed as `{printed}` and did not come back"
            );
        }
    }

    #[test]
    fn every_key_round_trips_through_its_name() {
        let mut keys: Vec<Key> = "abcXYZ019".chars().map(Key::Char).collect();
        keys.extend([
            Key::Char('@'),
            Key::Char('+'),
            Key::Enter,
            Key::Tab,
            Key::Backspace,
            Key::Escape,
            Key::Space,
            Key::Delete,
            Key::Insert,
            Key::Home,
            Key::End,
            Key::PageUp,
            Key::PageDown,
            Key::Up,
            Key::Down,
            Key::Left,
            Key::Right,
            Key::CapsLock,
            Key::NumLock,
            Key::ScrollLock,
            Key::PrintScreen,
            Key::Pause,
            Key::Menu,
            Key::Plus,
            Key::Minus,
            Key::Comma,
            Key::Period,
            Key::Slash,
            Key::Semicolon,
            Key::Quote,
            Key::Backquote,
            Key::Backslash,
            Key::BracketLeft,
            Key::BracketRight,
            Key::Equal,
        ]);
        keys.extend((1..=24).map(Key::F));
        for key in keys {
            let name = key.name();
            assert_eq!(
                Key::from_name(&name),
                Some(key),
                "`{name}` did not come back"
            );
        }
    }

    #[test]
    fn function_keys_are_bounded() {
        assert_eq!(Key::from_name("F1"), Some(Key::F(1)));
        assert_eq!(Key::from_name("f24"), Some(Key::F(24)));
        assert_eq!(Key::from_name("F0"), None);
        assert_eq!(Key::from_name("F25"), None);
    }

    #[test]
    fn empty_text_is_an_error() {
        assert!(Chord::parse("").is_err());
    }

    #[test]
    fn a_trailing_plus_is_an_error() {
        assert!(Chord::parse("Ctrl+").is_err());
    }

    #[test]
    fn an_unknown_key_is_an_error() {
        assert!(Chord::parse("Ctrl+NotAKey").is_err());
    }

    #[test]
    fn an_unknown_modifier_is_an_error() {
        assert!(Chord::parse("Foo+T").is_err());
    }

    #[test]
    fn modifiers_with_no_key_are_an_error() {
        assert!(Chord::parse("Ctrl+Shift").is_err());
    }

    #[test]
    fn the_error_names_the_text_it_was_given() {
        let message = Chord::parse("Ctrl+Nope").unwrap_err().to_string();
        assert!(message.contains("Ctrl+Nope"), "{message}");
    }

    #[test]
    fn from_str_is_the_same_as_parse() {
        assert_eq!(
            "Ctrl+A".parse::<Chord>().unwrap(),
            Chord::parse("Ctrl+A").unwrap()
        );
    }

    #[test]
    fn matches_compares_both_halves() {
        let c = chord(Modifiers::CTRL, Key::Char('t'));
        assert!(c.matches(Modifiers::CTRL, Key::Char('t')));
        assert!(!c.matches(Modifiers::empty(), Key::Char('t')));
        assert!(!c.matches(Modifiers::CTRL, Key::Char('T')));
        assert!(!c.matches(Modifiers::CTRL, Key::Tab));
    }

    #[test]
    fn a_chord_naming_a_punctuation_key_fires_on_the_character_shift_puts_there() {
        // The shipped settings binding, and the reason this rule exists: on a US
        // layout the event for `Ctrl+Shift+,` carries `'<'`.
        let c = chord(Modifiers::CTRL | Modifiers::SHIFT, Key::Comma);
        assert!(c.matches(Modifiers::CTRL | Modifiers::SHIFT, Key::Char('<')));
        assert!(c.matches(Modifiers::CTRL | Modifiers::SHIFT, Key::Comma));
    }

    #[test]
    fn the_punctuation_rule_needs_shift_and_needs_the_same_key() {
        let c = chord(Modifiers::CTRL | Modifiers::SHIFT, Key::Comma);
        // Without shift the event carries `,`, and there is nothing to take back off.
        assert!(!c.matches(Modifiers::CTRL, Key::Char(',')));
        // Shift on some other key is some other key.
        assert!(!c.matches(Modifiers::CTRL | Modifiers::SHIFT, Key::Char('>')));
        // And the modifiers still have to be the ones the binding named.
        assert!(!c.matches(Modifiers::SHIFT, Key::Char('<')));
    }

    #[test]
    fn a_letter_is_never_matched_by_its_other_case() {
        // `Ctrl+Shift+T` names `'T'`; `'t'` is a different chord and stays one, and
        // the punctuation rule must not quietly widen it.
        let c = chord(Modifiers::CTRL | Modifiers::SHIFT, Key::Char('T'));
        assert!(!c.matches(Modifiers::CTRL | Modifiers::SHIFT, Key::Char('t')));
        let lower = chord(Modifiers::CTRL | Modifiers::SHIFT, Key::Char('t'));
        assert!(!lower.matches(Modifiers::CTRL | Modifiers::SHIFT, Key::Char('T')));
    }

    #[test]
    fn a_shifted_digit_meets_the_digit_it_sits_on() {
        let c = chord(Modifiers::CTRL | Modifiers::SHIFT, Key::Char('1'));
        assert!(c.matches(Modifiers::CTRL | Modifiers::SHIFT, Key::Char('!')));
        assert!(!c.matches(Modifiers::CTRL | Modifiers::SHIFT, Key::Char('@')));
    }
}
