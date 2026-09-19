//! A key event, as the host hands it over.

use crate::chord::{Key, Modifiers};

/// What the host is reporting about a key.
///
/// The kind is here because the two encodings disagree about what a repeat is. Kitty
/// can spell one as itself, so a program that negotiated for it can tell a held key
/// from a tapped one; the legacy encoding cannot, and sends a repeat as the press it
/// repeats. Only a release means the same thing to both, which is "nothing". The
/// host has to pass the distinction along or it is gone by the time the encoding that
/// can use it gets there.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum KeyKind {
    /// The key went down.
    Press,
    /// The key is still down and the keyboard's repeat rate fired again.
    Repeat,
    /// The key came up.
    Release,
}

/// A key event.
///
/// `text` is the one field [`crate::encode_key`] does not read, and that is
/// deliberate. It carries whatever the platform composed for the event — a dead key
/// resolved to an accent, an IME candidate committed — and it belongs to the host,
/// which needs it to echo into the grid and to decide whether a composition is still
/// in progress. What goes to the pty is decided by [`KeyEvent::key`] and
/// [`KeyEvent::mods`] alone, because a program reads the byte for the key that was
/// pressed and has no way to make sense of a composed string arriving unannounced.
/// Keeping it off the encoding path is what stops a half-finished IME candidate from
/// being sent to a shell as though the user had typed it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct KeyEvent {
    /// The key itself. For [`Key::Char`] this is the character the layout produced,
    /// already shifted, so a capital `A` is `Char('A')` and not `Char('a')` with
    /// Shift held.
    pub key: Key,
    /// The modifiers held when the event fired.
    pub mods: Modifiers,
    /// The text the platform composed for this event, if any. For the host, not for
    /// the wire — see the type's own documentation.
    pub text: Option<String>,
    /// Whether this is a press, a repeat, or a release.
    pub kind: KeyKind,
    /// The key at the same position on the standard PC-101 layout, when the host knows
    /// it. This is what the kitty protocol's `Report alternate keys` flag calls the
    /// base-layout key, and it is the reason the flag needs the physical switch: a
    /// shortcut bound to `Ctrl+Z` on a QWERTZ keyboard is a press of `y`'s position,
    /// and only the base-layout key says so. `None` for a key the layout has no such
    /// character for, and the encoder then writes no base sub-field rather than
    /// guessing one.
    pub base: Option<char>,
    /// The key the current layout produces with no modifiers held, when the host knows
    /// it. This is what names the key on the wire: the protocol's main code point is
    /// the unshifted key, so `Shift+2` on a German layout is `2` (0x32) and not the
    /// `"` its key cap shows. `None` falls back to taking the shift back off
    /// [`KeyEvent::key`] by the US pairing, which is right on a US layout and an
    /// approximation elsewhere.
    pub unshifted: Option<char>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_event_carries_its_own_text_without_the_encoder_reading_it() {
        let event = KeyEvent {
            key: Key::Char('a'),
            mods: Modifiers::empty(),
            text: Some("a".into()),
            kind: KeyKind::Press,
            base: None,
            unshifted: None,
        };
        assert_eq!(event.key, Key::Char('a'));
        assert_eq!(event.text.as_deref(), Some("a"));
        assert_eq!(event.kind, KeyKind::Press);
        assert_eq!(event.base, None);
        assert_eq!(event.unshifted, None);
    }
}
