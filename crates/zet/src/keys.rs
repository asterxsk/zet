//! The keyboard, as `winit` reports it and as zet names it.
//!
//! Two vocabularies meet here and neither is wrong. `winit` describes a *platform
//! event*: which physical switch moved, what the layout made of it, whether it is a
//! repeat. zet describes a *key*, in the vocabulary a config file is written in —
//! [`Key`] has both a `Plus` and a `Char('+')` because a keyboard event names `Plus` and
//! an IME commit carries `'+'`, and the two have to round-trip through the same text.
//!
//! # The one thing this file has to get right
//!
//! A binding is written `Ctrl+Plus` and a press of the `+` key arrives from `winit` as
//! `Character("+")`. If the translation left that alone, `font-larger` would be a key
//! that never fires — silently, with no error, which is the worst kind of broken
//! binding, because the config file looks right and the app looks broken. So every
//! punctuation character is mapped to its *named* variant, and `zet_input::encode`'s
//! character lookup maps it straight back to the character on the way out. The encoder
//! treats the two spellings identically by design; this file is what makes that design
//! reachable.
//!
//! # What is deliberately dropped
//!
//! A key zet cannot name returns [`None`] and never reaches the session. `Unidentified`
//! and `Dead` are the platform saying "this keystroke is not a character", and the
//! modifiers themselves — Shift, Control, Alt — are carried in [`Modifiers`] rather
//! than being keys of their own.
//!
//! # Where the modifiers come from
//!
//! Not from the key event. `winit` reports modifier state as its own event —
//! `WindowEvent::ModifiersChanged` — and every `KeyboardInput` in between carries no
//! copy of it. So the host keeps the last one it saw and hands it in, which is also why
//! [`translate`] takes an argument rather than reading one.

use winit::event::KeyEvent as WinitKeyEvent;
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey, PhysicalKey};
use zet_input::{Key, KeyEvent, KeyKind, Modifiers};

/// Translate a key event, or [`None`] for one zet has no name for.
///
/// `held` is the modifier state as of the last `ModifiersChanged` event, which is what
/// `winit` requires a caller to track. The logical key is tried first; the physical one
/// is the fallback for the keys that produce no text, which is how the navigation
/// cluster and the function row come through on layouts that map them to nothing.
#[must_use]
pub fn translate(event: &WinitKeyEvent, held: ModifiersState) -> Option<KeyEvent> {
    translate_key(
        &event.logical_key,
        event.physical_key,
        event.state,
        event.repeat,
        held,
    )
}

/// The same translation, over the parts of an event rather than the event.
///
/// `winit`'s `KeyEvent` cannot be built outside `winit` — its `platform_specific` field
/// is crate-private — so a test of [`translate`] would have to wait for a real keyboard.
/// This is the seam that makes the file testable, and it is not a hypothetical one: the
/// punctuation mapping below is the single thing in zet whose failure is silent, so it
/// is the last thing that should be untestable.
#[must_use]
pub fn translate_key(
    logical: &WinitKey,
    physical: PhysicalKey,
    state: winit::event::ElementState,
    repeat: bool,
    held: ModifiersState,
) -> Option<KeyEvent> {
    let key = key(logical).or_else(|| named_keycode(physical))?;
    Some(KeyEvent {
        key,
        mods: translate_modifiers(held),
        text: text_of(logical),
        kind: kind(state, repeat),
    })
}

/// What a `winit` logical key is, in zet's vocabulary.
fn key(logical: &WinitKey) -> Option<Key> {
    match logical {
        WinitKey::Named(named) => named_key(*named),
        WinitKey::Character(text) => character_key(text),
        // A dead key is the first half of a composition and `Unidentified` is the
        // platform declining to say. Neither is a press zet can send: the character the
        // composition eventually produces arrives as a later event, carrying text.
        WinitKey::Dead(_) | WinitKey::Unidentified(_) => None,
    }
}

/// A named key, which is everything that is not a character.
fn named_key(named: NamedKey) -> Option<Key> {
    Some(match named {
        NamedKey::Enter => Key::Enter,
        NamedKey::Tab => Key::Tab,
        NamedKey::Backspace => Key::Backspace,
        NamedKey::Escape => Key::Escape,
        NamedKey::Space => Key::Space,
        NamedKey::Delete => Key::Delete,
        NamedKey::Insert => Key::Insert,
        NamedKey::Home => Key::Home,
        NamedKey::End => Key::End,
        NamedKey::PageUp => Key::PageUp,
        NamedKey::PageDown => Key::PageDown,
        NamedKey::ArrowUp => Key::Up,
        NamedKey::ArrowDown => Key::Down,
        NamedKey::ArrowLeft => Key::Left,
        NamedKey::ArrowRight => Key::Right,
        NamedKey::CapsLock => Key::CapsLock,
        NamedKey::NumLock => Key::NumLock,
        NamedKey::ScrollLock => Key::ScrollLock,
        NamedKey::PrintScreen => Key::PrintScreen,
        NamedKey::Pause => Key::Pause,
        NamedKey::ContextMenu => Key::Menu,
        // Everything else — the modifiers themselves, the browser and media keys, and
        // whatever a future `winit` adds — is a key zet has no name for.
        _ => return function(named),
    })
}

/// A function key, by name.
fn function(named: NamedKey) -> Option<Key> {
    use NamedKey as N;
    Some(match named {
        N::F1 => Key::F(1),
        N::F2 => Key::F(2),
        N::F3 => Key::F(3),
        N::F4 => Key::F(4),
        N::F5 => Key::F(5),
        N::F6 => Key::F(6),
        N::F7 => Key::F(7),
        N::F8 => Key::F(8),
        N::F9 => Key::F(9),
        N::F10 => Key::F(10),
        N::F11 => Key::F(11),
        N::F12 => Key::F(12),
        N::F13 => Key::F(13),
        N::F14 => Key::F(14),
        N::F15 => Key::F(15),
        N::F16 => Key::F(16),
        N::F17 => Key::F(17),
        N::F18 => Key::F(18),
        N::F19 => Key::F(19),
        N::F20 => Key::F(20),
        N::F21 => Key::F(21),
        N::F22 => Key::F(22),
        N::F23 => Key::F(23),
        N::F24 => Key::F(24),
        _ => return None,
    })
}

/// A character the layout produced, as a key.
///
/// A printable character is a `Char`, except for the twelve punctuation characters that
/// have names of their own, which are the named variants. The exception is the whole
/// point of the function: `Ctrl+Plus` in a config file is the `+` key, and arriving here
/// as [`Key::Plus`] is what lets the binding match.
fn character_key(text: &str) -> Option<Key> {
    let mut characters = text.chars();
    let first = characters.next()?;
    // A `Character` holding more than one character is a composition the platform
    // committed, not a key press. It reaches the session as text or not at all.
    if characters.next().is_some() {
        return None;
    }
    Some(match first {
        '+' => Key::Plus,
        '-' => Key::Minus,
        ',' => Key::Comma,
        '.' => Key::Period,
        '/' => Key::Slash,
        ';' => Key::Semicolon,
        '\'' => Key::Quote,
        '`' => Key::Backquote,
        '\\' => Key::Backslash,
        '[' => Key::BracketLeft,
        ']' => Key::BracketRight,
        '=' => Key::Equal,
        // A control character is what the platform reports when a modifier was held and
        // the layout translated the combination — `Ctrl+A` is `\u{1}`. That is not the
        // key the user pressed, and the encoder derives the byte from the letter, so
        // declining here costs nothing and inventing a `Char('\u{1}')` would send the
        // wrong thing.
        c if c.is_control() => return None,
        c => Key::Char(c),
    })
}

/// The key behind a physical switch, for events whose logical key named nothing.
///
/// The navigation cluster and the function row are the cases this exists for: they are
/// keys rather than characters, and a layout that produces no text for them should still
/// let them through.
fn named_keycode(code: PhysicalKey) -> Option<Key> {
    use winit::keyboard::KeyCode as C;
    let PhysicalKey::Code(code) = code else {
        return None;
    };
    Some(match code {
        C::Enter | C::NumpadEnter => Key::Enter,
        C::Tab => Key::Tab,
        C::Backspace => Key::Backspace,
        C::Escape => Key::Escape,
        C::Space => Key::Space,
        C::Delete => Key::Delete,
        C::Insert => Key::Insert,
        C::Home => Key::Home,
        C::End => Key::End,
        C::PageUp => Key::PageUp,
        C::PageDown => Key::PageDown,
        C::ArrowUp => Key::Up,
        C::ArrowDown => Key::Down,
        C::ArrowLeft => Key::Left,
        C::ArrowRight => Key::Right,
        C::CapsLock => Key::CapsLock,
        C::NumLock => Key::NumLock,
        C::ScrollLock => Key::ScrollLock,
        C::PrintScreen => Key::PrintScreen,
        C::Pause => Key::Pause,
        C::ContextMenu => Key::Menu,
        C::F1 => Key::F(1),
        C::F2 => Key::F(2),
        C::F3 => Key::F(3),
        C::F4 => Key::F(4),
        C::F5 => Key::F(5),
        C::F6 => Key::F(6),
        C::F7 => Key::F(7),
        C::F8 => Key::F(8),
        C::F9 => Key::F(9),
        C::F10 => Key::F(10),
        C::F11 => Key::F(11),
        C::F12 => Key::F(12),
        C::F13 => Key::F(13),
        C::F14 => Key::F(14),
        C::F15 => Key::F(15),
        C::F16 => Key::F(16),
        C::F17 => Key::F(17),
        C::F18 => Key::F(18),
        C::F19 => Key::F(19),
        C::F20 => Key::F(20),
        C::F21 => Key::F(21),
        C::F22 => Key::F(22),
        C::F23 => Key::F(23),
        C::F24 => Key::F(24),
        _ => return None,
    })
}

/// The text the platform composed for this event, if it composed any.
///
/// Handed to the session as text so that a layout's own shift state reaches the shell's
/// line editor. A control character is dropped: it encodes a modifier rather than
/// something the user typed, and echoing `\u{1}` into a prompt is visible garbage.
fn text_of(logical: &WinitKey) -> Option<String> {
    // The space bar is a named key on this platform and is still text: it is what the
    // user typed, and a consumer that reads the text to find out what was typed — the
    // find bar is the one — wants it. Leaving it out meant a query could hold every
    // character except the commonest separator there is.
    if matches!(logical, WinitKey::Named(NamedKey::Space)) {
        return Some(" ".to_string());
    }
    let WinitKey::Character(text) = logical else {
        return None;
    };
    if text.chars().any(char::is_control) {
        return None;
    }
    Some(text.to_string())
}

/// Whether this is a press, a repeat, or a release.
///
/// `winit` spells a repeat by leaving the state at `Pressed` and setting a flag, which
/// is a distinction a terminal needs and cannot reconstruct afterwards: a program that
/// negotiated the kitty protocol sees the repeat as its own event, and one that did not
/// sees the same byte either way.
const fn kind(state: winit::event::ElementState, repeat: bool) -> KeyKind {
    if repeat {
        KeyKind::Repeat
    } else {
        match state {
            winit::event::ElementState::Pressed => KeyKind::Press,
            winit::event::ElementState::Released => KeyKind::Release,
        }
    }
}

/// The modifiers held when the event fired.
///
/// The left/right distinction `winit` keeps is dropped, because nothing downstream can
/// act on it: a terminal protocol encodes "control is held", and the two keys are the
/// same modifier everywhere it matters.
///
/// Public because the mouse needs it too, and for the same reason. A mouse event carries
/// no modifier state of its own, so the host keeps the last one it saw and both paths
/// read it — one translation, so a `Ctrl+click` means the same thing to the chrome and
/// to the program underneath it.
#[must_use]
pub fn translate_modifiers(held: ModifiersState) -> Modifiers {
    let mut out = Modifiers::empty();
    if held.shift_key() {
        out |= Modifiers::SHIFT;
    }
    if held.control_key() {
        out |= Modifiers::CTRL;
    }
    if held.alt_key() {
        out |= Modifiers::ALT;
    }
    if held.super_key() {
        out |= Modifiers::SUPER;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::event::ElementState;
    use winit::keyboard::{KeyCode, NativeKey, PhysicalKey};

    /// A press, built from the parts a test can actually construct.
    fn press(key: &WinitKey, code: KeyCode, held: ModifiersState) -> Option<KeyEvent> {
        translate_key(
            key,
            PhysicalKey::Code(code),
            ElementState::Pressed,
            false,
            held,
        )
    }

    /// A press with nothing held, which is most of them.
    fn plain(key: &WinitKey, code: KeyCode) -> Option<KeyEvent> {
        press(key, code, ModifiersState::empty())
    }

    /// The logical key a layout produces for a one-character string.
    fn character(text: &str) -> WinitKey {
        WinitKey::Character(text.into())
    }

    #[test]
    fn the_punctuation_that_has_a_name_is_named() {
        // The reason this file exists. Each of these is what winit reports for a key the
        // config file calls by name, and a binding on any of them is dead unless the
        // translation produces the named variant.
        let cases = [
            ("+", Key::Plus, KeyCode::Equal),
            ("-", Key::Minus, KeyCode::Minus),
            (",", Key::Comma, KeyCode::Comma),
            (".", Key::Period, KeyCode::Period),
            ("/", Key::Slash, KeyCode::Slash),
            (";", Key::Semicolon, KeyCode::Semicolon),
            ("'", Key::Quote, KeyCode::Quote),
            ("`", Key::Backquote, KeyCode::Backquote),
            ("\\", Key::Backslash, KeyCode::Backslash),
            ("[", Key::BracketLeft, KeyCode::BracketLeft),
            ("]", Key::BracketRight, KeyCode::BracketRight),
            ("=", Key::Equal, KeyCode::Equal),
        ];
        for (text, expected, code) in cases {
            let translated =
                plain(&character(text), code).unwrap_or_else(|| panic!("{text} should translate"));
            assert_eq!(translated.key, expected, "{text} should be {expected:?}");
        }
    }

    #[test]
    fn a_binding_written_for_the_named_key_fires() {
        // The end-to-end version of the test above, and the one that would have caught
        // the bug it describes: a chord parsed out of the config file has to equal the
        // key this module produces for the same physical press.
        let chord: zet_input::Chord = "Ctrl+Plus".parse().expect("a valid chord");
        let translated = press(&character("+"), KeyCode::Equal, ModifiersState::CONTROL)
            .expect("the + key translates");
        assert_eq!(chord.key, translated.key);
        assert_eq!(chord.mods, translated.mods);
    }

    #[test]
    fn an_ordinary_letter_stays_a_character() {
        let translated =
            press(&character("A"), KeyCode::KeyA, ModifiersState::SHIFT).expect("A translates");
        assert_eq!(translated.key, Key::Char('A'));
        assert_eq!(translated.mods, Modifiers::SHIFT);
        assert_eq!(translated.text.as_deref(), Some("A"));
    }

    #[test]
    fn a_control_character_is_not_a_key() {
        // Ctrl+A arrives as U+0001 on some platforms. Sending a `Char` holding it would
        // be a byte the shell cannot read; the encoder builds the real byte from the
        // letter, so declining here loses nothing.
        assert!(plain(&character("\u{1}"), KeyCode::KeyA).is_none());
    }

    #[test]
    fn a_composition_that_committed_several_characters_is_not_a_key() {
        assert!(plain(&character("ab"), KeyCode::KeyA).is_none());
    }

    #[test]
    fn a_named_key_and_its_character_are_both_named_consistently() {
        // Space has a named spelling and can also arrive as text. The two are different
        // keys on purpose — the encoder treats them identically — and what matters is
        // that neither is dropped.
        let named =
            plain(&WinitKey::Named(NamedKey::Space), KeyCode::Space).expect("space translates");
        assert_eq!(named.key, Key::Space);
        assert_eq!(
            named.text.as_deref(),
            Some(" "),
            "and it carries its text, which is the only thing a text field reads"
        );
        assert_eq!(
            plain(&character(" "), KeyCode::Space)
                .expect("the space character translates")
                .key,
            Key::Char(' ')
        );
    }

    #[test]
    fn every_function_key_survives_the_round_trip() {
        // F13 and up exist on no keyboard anyone owns, which is exactly why they are the
        // ones a terminal is asked to send: a program can bind them without stealing a
        // key the user has.
        const NAMED: [NamedKey; 24] = [
            NamedKey::F1,
            NamedKey::F2,
            NamedKey::F3,
            NamedKey::F4,
            NamedKey::F5,
            NamedKey::F6,
            NamedKey::F7,
            NamedKey::F8,
            NamedKey::F9,
            NamedKey::F10,
            NamedKey::F11,
            NamedKey::F12,
            NamedKey::F13,
            NamedKey::F14,
            NamedKey::F15,
            NamedKey::F16,
            NamedKey::F17,
            NamedKey::F18,
            NamedKey::F19,
            NamedKey::F20,
            NamedKey::F21,
            NamedKey::F22,
            NamedKey::F23,
            NamedKey::F24,
        ];
        for (index, named) in NAMED.into_iter().enumerate() {
            let number = u8::try_from(index + 1).expect("twenty-four fits in a byte");
            let translated = plain(&WinitKey::Named(named), KeyCode::F1)
                .unwrap_or_else(|| panic!("F{number} should translate"));
            assert_eq!(translated.key, Key::F(number));
        }
    }

    #[test]
    fn a_key_that_produced_no_text_falls_back_to_the_switch() {
        // The case the fallback exists for: the logical key names nothing zet can use
        // and the physical one still identifies the switch.
        let translated = plain(
            &WinitKey::Unidentified(NativeKey::Unidentified),
            KeyCode::ArrowLeft,
        )
        .expect("the physical key names an arrow");
        assert_eq!(translated.key, Key::Left);
    }

    #[test]
    fn a_key_zet_cannot_name_is_dropped_rather_than_guessed() {
        assert!(
            plain(
                &WinitKey::Named(NamedKey::BrowserBack),
                KeyCode::BrowserBack
            )
            .is_none()
        );
    }

    #[test]
    fn a_repeat_is_a_repeat_and_not_a_second_press() {
        let repeated = translate_key(
            &character("a"),
            PhysicalKey::Code(KeyCode::KeyA),
            ElementState::Pressed,
            true,
            ModifiersState::empty(),
        )
        .expect("a repeat still names a key");
        assert_eq!(repeated.kind, KeyKind::Repeat);
    }

    #[test]
    fn a_release_is_reported_as_one() {
        let released = translate_key(
            &character("a"),
            PhysicalKey::Code(KeyCode::KeyA),
            ElementState::Released,
            false,
            ModifiersState::empty(),
        )
        .expect("a release still names a key");
        assert_eq!(released.kind, KeyKind::Release);
    }

    #[test]
    fn every_modifier_reaches_the_event() {
        let translated = press(
            &character("a"),
            KeyCode::KeyA,
            ModifiersState::CONTROL
                | ModifiersState::SHIFT
                | ModifiersState::ALT
                | ModifiersState::SUPER,
        )
        .expect("a translates");
        assert_eq!(
            translated.mods,
            Modifiers::CTRL | Modifiers::SHIFT | Modifiers::ALT | Modifiers::SUPER
        );
    }

    #[test]
    fn a_dead_key_is_not_sent_as_a_keystroke() {
        // The first half of a composition is not a key press. The character the
        // composition eventually produces arrives as its own event, carrying text.
        assert!(plain(&WinitKey::Dead(None), KeyCode::Backquote).is_none());
    }
}
