//! The byte sequences themselves.
//!
//! Every table here is xterm's, because xterm is what the programs on the other end
//! of the pty were written against: a TUI reads its terminfo entry, gets `kcuu1`,
//! and expects the bytes in this file. Where the sequence is not obvious the comment
//! says where it comes from, because a table copied without its source is a table
//! nobody can check.
//!
//! Three things in here are worth knowing before reading the rest.
//!
//! **The modifier parameter.** A modified special key becomes `CSI 1;<m><final>` or
//! `CSI <n>;<m>~`, where `<m>` is `1 + Shift(1) + Alt(2) + Control(4) + Super(8)`.
//! That is a plain decimal CSI parameter, not a packed field: the four-bit limit in
//! xterm's documentation is the size of the *bit set* being counted, so the widest
//! value, all four modifiers down, is the ordinary number 16 and needs no
//! sub-parameter encoding to write. Nothing above 16 can arise here.
//!
//! **Shift is always transmitted.** Shift+Left could be argued either way — the
//! parameter adds nothing a program could not work out, and some terminals sent the
//! bare `CSI D` for years — but `CSI 1;2D` is the form xterm's default
//! configuration produces and the form every current program is tested against, so
//! it is the form this crate sends. A program that only knows the bare sequence is
//! one that predates modifier keys on the cursor pad, and it is already broken by
//! the rest of this table.
//!
//! **Function keys are not contiguous.** xterm's PC-style table skips `CSI 16~` and
//! `CSI 22~` on the way from F5 to F12, and `CSI 27~` and `CSI 30~` on the way from
//! F13 to F20. The numbers below are transcribed from that table rather than counted
//! out, because the gaps are real: `CSI 16~` is not F6 and no terminal sends it.
//! F21 through F24 have no entry in it at all — see [`function`].

use zet_vt::{KeyboardFlags, Modes, MouseEncoding, MouseMode};

use crate::chord::{Key, Modifiers, character_of, unshifted};
use crate::key::{KeyEvent, KeyKind};
use crate::mouse::{MouseAction, MouseButton, MouseEvent};

/// `ESC`, which starts every sequence this crate emits.
const ESC: u8 = 0x1b;

/// The bytes a program should be given for a key event, or `None` for none at all.
///
/// `None` is a real answer and not a failure: it means "write nothing to the pty",
/// which is what a key that this encoding cannot describe has to produce. There are
/// two such things — a release, and the handful of keys in [`Key`] that exist so a
/// user can name them (`CapsLock`, `PrintScreen`, `Pause`, `Menu`, the three locks)
/// and that never had a byte of their own. A host that treats `None` as an error
/// will drop the keystroke twice.
///
/// The text is read for one thing and one thing only, and only when a program has
/// asked for it by name: the kitty protocol's associated-text field. [`KeyEvent`]
/// says why it is otherwise kept off this path.
#[must_use]
pub fn encode_key(event: &KeyEvent, modes: &Modes) -> Option<Vec<u8>> {
    // A program that asked for the kitty keyboard protocol gets what it asked for.
    // This is not a second spelling of the same bytes: under the protocol a release
    // exists at all, a repeat is told apart from a press, and Escape stops being a
    // byte a program has to guess about with a timer.
    if let Some(bytes) = kitty_key(event, modes) {
        return Some(bytes);
    }

    // A key going up has no byte in legacy VT, so a release produces none. A key
    // repeating does, and it is the byte its press produces, because a repeat *is* a
    // press as far as the wire is concerned: the keyboard is held down and the
    // autorepeat rate fired again, which the program is meant to see as another
    // keystroke. Withholding it is not caution, it is a keyboard that stops working
    // the moment a key is held — holding an arrow key walks nowhere and holding
    // Backspace deletes exactly one character.
    //
    // The kind is still kept distinct in [`KeyKind`], because the kitty protocol can
    // express it and is the reason the host tracks it at all; the branch above is the
    // one that does, and this one is the encoding that cannot.
    if event.kind == KeyKind::Release {
        return None;
    }

    // The keys whose sequence carries the modifiers as a CSI parameter. These come
    // first because Alt must not also be prefixed to them: `ESC ESC [ A` is not an
    // up arrow, it is Escape followed by an up arrow.
    if let Some(bytes) = parameterised_key(event, modes) {
        return Some(bytes);
    }

    // Everything left produces a character or a control byte, and for those Alt is a
    // literal ESC in front of the result — which is how Alt has been transmitted
    // since before there was anywhere else to put it.
    let mut out = Vec::new();
    if event.mods.contains(Modifiers::ALT) {
        out.push(ESC);
    }
    out.extend_from_slice(&literal(event.key, event.mods, modes)?);
    Some(out)
}

/// The keys whose modifiers travel inside the sequence.
///
/// This is also exactly the set the kitty protocol reports in that form rather than
/// in its `CSI u` form, which is not a coincidence: the protocol's own table gives
/// the up arrow as `CSI 1 A` and Insert as `CSI 2 ~`, the sequences these keys have
/// had for forty years. See [`kitty_key`].
fn parameterised_key(event: &KeyEvent, modes: &Modes) -> Option<Vec<u8>> {
    let p = Params::plain(modifier_param(event.mods));
    match event.key {
        Key::Up => Some(cursor(b'A', p, modes)),
        Key::Down => Some(cursor(b'B', p, modes)),
        Key::Right => Some(cursor(b'C', p, modes)),
        Key::Left => Some(cursor(b'D', p, modes)),
        Key::Home => Some(cursor(b'H', p, modes)),
        Key::End => Some(cursor(b'F', p, modes)),
        Key::Insert => Some(tilde(2, p)),
        Key::Delete => Some(tilde(3, p)),
        Key::PageUp => Some(tilde(5, p)),
        Key::PageDown => Some(tilde(6, p)),
        Key::Tab if event.mods.contains(Modifiers::SHIFT) => Some(back_tab(p)),
        Key::F(n) => function(n, p),
        _ => None,
    }
}

/// The value of xterm's `<m>` parameter for a set of modifiers.
///
/// The kitty protocol's modifier bit field is this number minus one — the two agree
/// on the encoding down to the bit assignments — so this is also what the protocol's
/// sequences carry, and the two callers do not need their own copies.
fn modifier_param(mods: Modifiers) -> u8 {
    let mut m = 1;
    if mods.contains(Modifiers::SHIFT) {
        m += 1;
    }
    if mods.contains(Modifiers::ALT) {
        m += 2;
    }
    if mods.contains(Modifiers::CTRL) {
        m += 4;
    }
    if mods.contains(Modifiers::SUPER) {
        m += 8;
    }
    m
}

/// The parameter a key's sequence carries, and the event type when one is reported.
///
/// The kitty protocol puts its event type in a sub-field of the modifier parameter,
/// so a sequence that has to say "this key came up" is the same sequence with a
/// colon and a 3 in it. Everything that writes one of those sequences takes this
/// rather than the bare number, because there is one place that decides when the
/// parameter can be left out and it is not the same place for every key.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Params {
    /// `1 +` the modifiers held.
    m: u8,
    /// The kitty event type: 2 for a repeat, 3 for a release. `None` for a press,
    /// which the protocol makes the default and leaves out of the sequence.
    event: Option<u8>,
}

impl Params {
    /// The parameter for the legacy encoding, which has no event types at all.
    const fn plain(m: u8) -> Self {
        Self { m, event: None }
    }

    /// Whether the parameter can be left out of the sequence entirely.
    ///
    /// Several of these keys have a shorter form when it can be: `SS3 A` for an
    /// unmodified up arrow, `CSI Z` for Shift+Tab. A sequence that has to carry an
    /// event type has nowhere to put it in the short form, so it takes the long one.
    const fn is_default(self) -> bool {
        self.m == 1 && self.event.is_none()
    }
}

/// `CSI 1;<m><final>`, the parameter form of a cursor key or a shifted tab.
fn parameterised(lead: u16, p: Params, final_byte: u8) -> Vec<u8> {
    let mut out = vec![ESC, b'['];
    out.extend_from_slice(lead.to_string().as_bytes());
    out.push(b';');
    out.extend_from_slice(p.m.to_string().as_bytes());
    if let Some(event) = p.event {
        out.push(b':');
        out.extend_from_slice(event.to_string().as_bytes());
    }
    out.push(final_byte);
    out
}

/// An arrow, Home, or End.
///
/// The unmodified form is the only one `DECCKM` touches: `SS3` in application cursor
/// mode, `CSI` otherwise. A modified one is always `CSI 1;<m><final>` — the
/// application-mode prefix has no room for a parameter, and xterm does not try to
/// keep it, which is why a program that enabled application cursor keys still
/// receives `CSI` for Shift+Left.
fn cursor(final_byte: u8, p: Params, modes: &Modes) -> Vec<u8> {
    if p.is_default() {
        let lead = if modes.app_cursor_keys { b'O' } else { b'[' };
        vec![ESC, lead, final_byte]
    } else {
        parameterised(1, p, final_byte)
    }
}

/// `CSI <n>~` and its parameter form, for the editing keypad and F5 upward.
fn tilde(n: u16, p: Params) -> Vec<u8> {
    if p.is_default() {
        let mut out = vec![ESC, b'['];
        out.extend_from_slice(n.to_string().as_bytes());
        out.push(b'~');
        out
    } else {
        parameterised(n, p, b'~')
    }
}

/// Shift+Tab.
///
/// Plain Shift keeps xterm's short `CSI Z`, which is what readline's `backward-tab`
/// and every completion menu in existence look for, even though it sits oddly next
/// to `CSI 1;2Z` for Shift+Alt+Tab. The short form is the one in the wild.
fn back_tab(p: Params) -> Vec<u8> {
    if p.m == 2 && p.event.is_none() {
        vec![ESC, b'[', b'Z']
    } else {
        parameterised(1, p, b'Z')
    }
}

/// A function key.
///
/// F1 through F4 are `SS3 P` through `SS3 S`, and xterm switches that `SS3` to `CSI`
/// the moment a modifier has to be carried — it is the same rule [`cursor`] follows,
/// for the same reason.
///
/// F5 through F20 come from xterm's PC-style table, gaps included. F21 through F24
/// are **not** in it, and this returns `None` for them rather than continuing the
/// arithmetic: the table stops at `CSI 34~`, and the only thing past it that exists
/// anywhere is the reading where F13 is really Shift+F1 and F24 is Shift+F12, which
/// would collide with this crate's own modifier handling and has no terminfo entry
/// behind it. Named keys a user cannot bind are a gap worth stating; invented bytes
/// are a bug worth not shipping.
fn function(n: u8, p: Params) -> Option<Vec<u8>> {
    match n {
        1 => Some(function_ss3(b'P', p)),
        2 => Some(function_ss3(b'Q', p)),
        3 => Some(function_ss3(b'R', p)),
        4 => Some(function_ss3(b'S', p)),
        5 => Some(tilde(15, p)),
        6 => Some(tilde(17, p)),
        7 => Some(tilde(18, p)),
        8 => Some(tilde(19, p)),
        9 => Some(tilde(20, p)),
        10 => Some(tilde(21, p)),
        11 => Some(tilde(23, p)),
        12 => Some(tilde(24, p)),
        13 => Some(tilde(25, p)),
        14 => Some(tilde(26, p)),
        15 => Some(tilde(28, p)),
        16 => Some(tilde(29, p)),
        17 => Some(tilde(31, p)),
        18 => Some(tilde(32, p)),
        19 => Some(tilde(33, p)),
        20 => Some(tilde(34, p)),
        _ => None,
    }
}

/// `SS3 <final>`, or `CSI 1;<m><final>` once a modifier has to be carried.
fn function_ss3(final_byte: u8, p: Params) -> Vec<u8> {
    if p.is_default() {
        vec![ESC, b'O', final_byte]
    } else {
        parameterised(1, p, final_byte)
    }
}

/// The keys that come down to a character or a single control byte.
fn literal(key: Key, held: Modifiers, modes: &Modes) -> Option<Vec<u8>> {
    if let Some(ch) = character_of(key) {
        if held.contains(Modifiers::CTRL)
            && let Some(byte) = control_byte(ch)
        {
            return Some(vec![byte]);
        }
        // Control has no code for this character, so it does not travel: the
        // character itself is sent. Sending nothing instead would swallow a
        // keystroke the user pressed, and a shell that receives `-` is easier to
        // explain than a shell that receives silence.
        let mut buf = [0u8; 4];
        return Some(ch.encode_utf8(&mut buf).as_bytes().to_vec());
    }
    match key {
        Key::Enter => Some(vec![if modes.newline { b'\n' } else { b'\r' }]),
        Key::Backspace => Some(vec![0x7f]),
        Key::Escape => Some(vec![ESC]),
        Key::Tab => Some(vec![b'\t']),
        _ => None,
    }
}

/// The control byte for a character, if it has one.
///
/// Ctrl+A through Ctrl+Z are the letters' positions in the alphabet, and the
/// punctuation is the ASCII control range an ordinary terminal keyboard can still
/// reach: `@` and Space are NUL, `[` `\` `]` `^` `_` are 0x1b through 0x1f, and `?`
/// is DEL, which is why Backspace and Ctrl+? are the same byte on the wire and why a
/// terminal that maps Backspace to 0x08 breaks both.
fn control_byte(ch: char) -> Option<u8> {
    match ch {
        'a'..='z' | 'A'..='Z' => {
            let lower = u8::try_from(ch.to_ascii_lowercase()).ok()?;
            Some(lower - b'a' + 1)
        }
        ' ' | '@' => Some(0x00),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' => Some(0x1f),
        '?' => Some(0x7f),
        _ => None,
    }
}

// ---- the kitty keyboard protocol ---------------------------------------------
//
// Everything below is one protocol, and it is worth saying at the top what it is
// for, because the escape codes it produces look arbitrary until you know. A
// terminal has to send keys as bytes, and the bytes it has to send them as were
// chosen when there was no way to say anything else: Escape *is* the byte that
// starts an escape sequence, `Ctrl+I` is the byte for Tab, and a key going up has
// no byte at all. Every terminal program of the last forty years has had to guess
// around that, usually with a timer that waits to see whether anything follows a
// lone 0x1b.
//
// The protocol fixes it by giving every key a canonical escape code of its own.
// A program opts in by name, in stages, and a terminal that speaks it reports what
// it was asked for and nothing more — which is why every function here has a
// condition on it rather than a single branch at the top.

/// The escape code a key event has under the kitty keyboard protocol, or `None` when
/// the protocol leaves the key to the legacy encoding.
///
/// `None` is the common answer and not a failure. Under this protocol the legacy
/// bytes are the *correct* bytes for most keys: the protocol's own table gives the
/// up arrow `CSI 1 A` and Insert `CSI 2 ~`, which is what the legacy encoder already
/// sends, and the specification says a terminal should use whatever its terminfo
/// entry says for those keys. What the protocol adds is a way to say the things the
/// legacy bytes cannot — a release, a repeat, an Escape that is not the start of a
/// sequence — and those are what this produces.
fn kitty_key(event: &KeyEvent, modes: &Modes) -> Option<Vec<u8>> {
    let flags = modes.keyboard;
    if flags.is_empty() {
        return None;
    }

    let typed = flags.contains(KeyboardFlags::EVENT_TYPES);
    // A key going up has no legacy byte and never had one, so a release exists only
    // where a program asked for event types. A repeat does have one — it is the
    // byte its press produces — but only inside the protocol is it told apart from
    // one, so without the flag it falls through and is sent as a press.
    if event.kind == KeyKind::Release && !typed {
        return None;
    }

    let p = Params {
        m: modifier_param(event.mods),
        event: event_type(event.kind, typed),
    };

    // A key the protocol reports in a sequence — an arrow, the editing keypad, a
    // function key up to F12 — keeps the one it already has, because that sequence is
    // the protocol's own table entry for it and not a legacy stand-in: the table
    // gives the up arrow `CSI 1 A` and Insert `CSI 2 ~`, which is what every terminal
    // has sent for forty years. So the bytes do not change, and the only thing that
    // can bring us here is an event type that has to be written into them.
    if sequence_form(event.key) {
        return p.event.map(|_| parameterised_form(event, p));
    }

    // Everything else is a key that produces text, or one the protocol numbers. It is
    // reported as an escape code when a flag moves it there — and a key with no
    // legacy bytes at all is already there, which is why `numbered` is a reason on
    // its own rather than a condition on the others.
    let all = flags.contains(KeyboardFlags::ALL_KEYS);
    let numbered = key_code(event.key).is_some() && !has_legacy_bytes(event.key);
    let escaped =
        all || numbered || (flags.contains(KeyboardFlags::DISAMBIGUATE) && ambiguous(event));

    if !escaped && p.event.is_none() {
        return None;
    }

    Some(csi_u(
        key_code(event.key)?,
        p,
        kitty_text(event, flags, all),
    ))
}

/// Whether the protocol reports a key in a sequence rather than giving it a code
/// point of its own.
///
/// This is the protocol's functional-key table read the other way round: the keys it
/// lists with a `CSI` sequence are these. F13 upward are deliberately not among them,
/// even though [`function`] has tilde sequences for F13 through F20 out of xterm's
/// table — the protocol's table numbers them in the private use area instead, and a
/// program that opted into the protocol is reading that table and not xterm's.
fn sequence_form(key: Key) -> bool {
    matches!(
        key,
        Key::Up
            | Key::Down
            | Key::Right
            | Key::Left
            | Key::Home
            | Key::End
            | Key::Insert
            | Key::Delete
            | Key::PageUp
            | Key::PageDown
            | Key::F(1..=12)
    )
}

/// Whether the legacy encoding has bytes of its own for a key the protocol numbers.
///
/// The four the protocol keeps the control codes of, and every key that produces a
/// character: those are what the legacy encoder sends, and it takes a flag to move
/// them off it. The rest of the numbered keys — the locks, Print Screen, Pause, Menu,
/// F13 upward — have nothing behind them, so the protocol's number is the only thing
/// there is to send and no flag is needed to prefer it.
fn has_legacy_bytes(key: Key) -> bool {
    matches!(key, Key::Escape | Key::Enter | Key::Tab | Key::Backspace)
        || character_of(key).is_some()
}

/// Whether the protocol says a key must be reported as an escape code rather than as
/// the bytes it would produce on its own.
///
/// The protocol names five cases: Escape, alt+key, ctrl+key, ctrl+alt+key, and
/// shift+alt+key. What they have in common is that the bytes the key would otherwise
/// produce are bytes that mean something else — `alt+[` produces the two bytes that
/// begin a CSI sequence, and Escape produces the one byte that *is* a sequence's
/// first byte. Shift is not among them: `A` cannot be mistaken for anything, so it
/// is still sent as text.
///
/// Enter, Tab, and Backspace are excepted by name on top of that, and the reason is
/// worth keeping: a program that set the flag and then crashed without clearing it
/// would otherwise leave a terminal whose Enter key does not produce a newline, and
/// the user could not type `reset` to get out.
fn ambiguous(event: &KeyEvent) -> bool {
    match event.key {
        Key::Enter | Key::Tab | Key::Backspace => false,
        Key::Escape => true,
        _ => event.mods.contains(Modifiers::ALT) || event.mods.contains(Modifiers::CTRL),
    }
}

/// The event type sub-field's value, or `None` for a press.
///
/// A press is the protocol's default and is left out rather than spelled as `1`,
/// which is what makes a sequence from a program that asked for event types and one
/// from a program that did not byte-identical for the common case.
const fn event_type(kind: KeyKind, typed: bool) -> Option<u8> {
    if !typed {
        return None;
    }
    match kind {
        KeyKind::Press => None,
        KeyKind::Repeat => Some(2),
        KeyKind::Release => Some(3),
    }
}

/// The sequence for a key that carries its modifiers as a CSI parameter, with the
/// event type written into it.
///
/// The parameter is always present here even when nothing is held, because the event
/// type is a sub-field of it and a sub-field needs a field to be a sub-field of.
/// That is also why this does not go through [`cursor`] or [`tilde`]: their short
/// forms — `SS3 A`, `CSI 2 ~` — have nowhere to put it.
fn parameterised_form(event: &KeyEvent, p: Params) -> Vec<u8> {
    let (lead, final_byte) = match event.key {
        Key::Up => (1, b'A'),
        Key::Down => (1, b'B'),
        Key::Right => (1, b'C'),
        Key::Left => (1, b'D'),
        Key::Home => (1, b'H'),
        Key::End => (1, b'F'),
        Key::Insert => (2, b'~'),
        Key::Delete => (3, b'~'),
        Key::PageUp => (5, b'~'),
        Key::PageDown => (6, b'~'),
        // F1 through F4 are the `SS3` keys, and `CSI 1;<m>P` is what their parameter
        // form has always been. F5 upward are the tilde table.
        Key::F(n) if n <= 4 => (1, b"PQRS"[usize::from(n) - 1]),
        Key::F(n) => (function_tilde(n).unwrap_or(1), b'~'),
        // `sequence_form` is the only caller and it admits nothing else, so this is
        // unreachable rather than a case to guess at.
        _ => (1, b'~'),
    };
    parameterised(lead, p, final_byte)
}

/// The number in `CSI <n>~` for a function key from F5 upward, or `None` for one the
/// table does not reach.
///
/// The same table [`function`] uses, and the reason it is a function rather than a
/// second match: the two must agree, and a key reported under the protocol with a
/// different number from the one reported without it is a key two programs disagree
/// about.
fn function_tilde(n: u8) -> Option<u16> {
    Some(match n {
        5 => 15,
        6 => 17,
        7 => 18,
        8 => 19,
        9 => 20,
        10 => 21,
        11 => 23,
        12 => 24,
        13 => 25,
        14 => 26,
        15 => 28,
        16 => 29,
        17 => 31,
        18 => 32,
        19 => 33,
        20 => 34,
        _ => return None,
    })
}

/// The code point the protocol names a key by.
///
/// Most of these are in the Unicode private use area, because a key like Print
/// Screen has no character and the protocol needs a number for it that cannot
/// collide with one. The four that are not are Escape, Enter, Tab, and Backspace,
/// which keep the control codes they have always had — the same exception
/// [`ambiguous`] makes, for the same reason, and the protocol's table lists them
/// that way too.
///
/// `None` for a key the protocol has no number for. That is a key whose sequence
/// carries its modifiers instead, and the arithmetic of *that* is
/// [`parameterised_form`]'s; it is also the handful of named keys a user can bind and
/// nothing can type.
fn key_code(key: Key) -> Option<u32> {
    Some(match key {
        Key::Escape => 27,
        Key::Enter => 13,
        Key::Tab => 9,
        Key::Backspace => 127,
        // F1 through F12 have a sequence of their own and never reach here. F13
        // upward have no sequence in the legacy table at all — which is the gap
        // [`function`] documents — and the protocol is where they become usable:
        // its table numbers them from here.
        Key::F(n) => match n {
            13..=35 => 57_376 + (u32::from(n) - 13),
            _ => return None,
        },
        Key::CapsLock => 57_358,
        Key::ScrollLock => 57_359,
        Key::NumLock => 57_360,
        Key::PrintScreen => 57_361,
        Key::Pause => 57_362,
        Key::Menu => 57_363,
        _ => u32::from(unshifted(character_of(key)?)),
    })
}

/// The code points of the text a key produced, when the program asked for them.
///
/// The flag is only meaningful alongside [`KeyboardFlags::ALL_KEYS`], and asking for
/// it is the one thing that makes this crate read [`KeyEvent::text`] at all: the text
/// rides inside the escape code, which a program only parses because it asked, rather
/// than being sent as bytes that a shell would take for typing. A key with no text —
/// an arrow, a function key — gets no field rather than an empty one.
fn kitty_text(event: &KeyEvent, flags: KeyboardFlags, all: bool) -> Option<Vec<u32>> {
    if !all || !flags.contains(KeyboardFlags::ASSOCIATED_TEXT) {
        return None;
    }
    let text = event.text.as_deref()?;
    let points: Vec<u32> = text.chars().map(u32::from).collect();
    (!points.is_empty()).then_some(points)
}

/// `CSI <code> ; <mods> ; <text> u`, with each field left out when it has nothing to
/// say.
fn csi_u(code: u32, p: Params, text: Option<Vec<u32>>) -> Vec<u8> {
    let mut out = vec![ESC, b'['];
    out.extend_from_slice(code.to_string().as_bytes());
    if text.is_some() && p.is_default() {
        // A field cannot be written without the one before it. `;1` says what an
        // absent modifier field says — no modifiers, a press — so the text has a
        // place to sit.
        out.push(b';');
        out.push(b'1');
    } else if !p.is_default() {
        out.push(b';');
        out.extend_from_slice(p.m.to_string().as_bytes());
        if let Some(event) = p.event {
            out.push(b':');
            out.extend_from_slice(event.to_string().as_bytes());
        }
    }
    if let Some(text) = text {
        out.push(b';');
        for (i, point) in text.iter().enumerate() {
            if i > 0 {
                out.push(b':');
            }
            out.extend_from_slice(point.to_string().as_bytes());
        }
    }
    out.push(b'u');
    out
}

/// The bytes a program should be given for a mouse event, or `None` for none at all.
///
/// The mode decides what is reportable at all — see [`reportable`] — and the encoding
/// decides how it is written. Nothing here is sent unless the program asked for it,
/// which is the rule that makes mouse reporting usable: a program that wants the
/// mouse selects text with it, and one that does not gets the pointer back.
#[must_use]
pub fn encode_mouse(event: MouseEvent, modes: &Modes) -> Option<Vec<u8>> {
    if !reportable(event, modes.mouse) {
        return None;
    }

    // xterm's `Cb`: the button, plus 32 while the pointer is moving, plus the
    // modifiers, and `3` for a release in the encodings that have nowhere else to say
    // it. SGR is the exception and the reason it exists: it keeps the button number
    // and marks the release with its final byte, so a program can tell *which*
    // button came up. Reporting `3` there instead would throw away the only thing
    // SGR adds.
    let body = match event.action {
        MouseAction::Motion => button_code(event.button) + 32,
        MouseAction::Release => 3,
        MouseAction::Press => button_code(event.button),
    };
    let x10 = body + modifier_bits(event.mods);
    let sgr = if event.action == MouseAction::Release {
        button_code(event.button) + modifier_bits(event.mods)
    } else {
        x10
    };

    // The wire is one-based; the host's grid is not.
    let col = u32::from(event.col) + 1;
    let row = u32::from(event.row) + 1;
    let release = event.action == MouseAction::Release;

    Some(match modes.mouse_encoding {
        MouseEncoding::Sgr => {
            let mut out = vec![ESC, b'[', b'<'];
            push_decimal(&mut out, u32::from(sgr));
            out.push(b';');
            push_decimal(&mut out, col);
            out.push(b';');
            push_decimal(&mut out, row);
            out.push(if release { b'm' } else { b'M' });
            out
        }
        MouseEncoding::Urxvt => {
            // The same value the X10 form packs into a byte, printed in decimal
            // instead. The `+32` is not decoration: it is what makes an application's
            // parser subtract 32 and land on the button, and dropping it reports
            // button `-32`.
            let mut out = vec![ESC, b'['];
            push_decimal(&mut out, u32::from(32 + x10));
            out.push(b';');
            push_decimal(&mut out, col);
            out.push(b';');
            push_decimal(&mut out, row);
            out.push(b'M');
            out
        }
        MouseEncoding::X10 => {
            let mut out = vec![ESC, b'[', b'M'];
            push_wide(&mut out, u32::from(32 + x10));
            out.push(one_byte(32 + col));
            out.push(one_byte(32 + row));
            out
        }
        MouseEncoding::Utf8 => {
            let mut out = vec![ESC, b'[', b'M'];
            // The button is widened like the two coordinates beside it, and the reason
            // is the two thumb buttons rather than any position on a wide screen: back
            // and forward are buttons 8 and 9, which put the report's first value at
            // 160 and 161. Written as a raw byte those are `0xA0` and `0xA1`, which are
            // not UTF-8 at all — a program reading the stream the way this encoding
            // exists to let it has a malformed sequence on its hands and no way to
            // recover the button. xterm sends the two-byte form here and so does zet.
            push_wide(&mut out, u32::from(32 + x10));
            push_position(&mut out, col);
            push_position(&mut out, row);
            out
        }
    })
}

/// Whether the mode wants this event reported at all.
///
/// The five modes are cumulative, and the difference between the last three is the
/// only part worth stating: `Drag` reports movement *while a button is held*, so a
/// motion with nothing down is not a report but the pointer crossing the window, and
/// `Motion` reports everything. Sending the pointer's every twitch to a program that
/// asked for drags is not a small mistake — it is a flood of input the program did
/// not ask for and will try to interpret.
fn reportable(event: MouseEvent, mode: MouseMode) -> bool {
    match mode {
        MouseMode::None => false,
        MouseMode::X10 => event.action == MouseAction::Press,
        MouseMode::Button => event.action != MouseAction::Motion,
        MouseMode::Drag => match event.action {
            MouseAction::Motion => event.button != MouseButton::None,
            _ => true,
        },
        MouseMode::Motion => true,
    }
}

/// The button's code on the wire.
fn button_code(button: MouseButton) -> u16 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::None => 3,
        MouseButton::WheelUp => 64,
        MouseButton::WheelDown => 65,
        MouseButton::WheelLeft => 66,
        MouseButton::WheelRight => 67,
        MouseButton::Back => 128,
        MouseButton::Forward => 129,
    }
}

/// The modifiers, added into a mouse report.
///
/// These are the same three bits as xterm's key modifiers with the same weights, but
/// they are written separately because the value is not the same parameter: a mouse
/// report has no `1 +` bias, so the crate's two encodings cannot share a function
/// without one of them lying about its base.
fn modifier_bits(mods: Modifiers) -> u16 {
    let mut bits = 0;
    if mods.contains(Modifiers::SHIFT) {
        bits += 4;
    }
    if mods.contains(Modifiers::ALT) {
        bits += 8;
    }
    if mods.contains(Modifiers::CTRL) {
        bits += 16;
    }
    bits
}

/// Squeeze a value into the one byte the X10 form has for it.
///
/// Clamping, not wrapping. The X10 form spends its bytes directly, so the last
/// representable position is 223 and a wider terminal has to report the edge rather
/// than a column that has wrapped around to the middle of the screen — a click that
/// lands somewhere the user did not click is worse than one that lands at the border.
fn one_byte(value: u32) -> u8 {
    u8::try_from(value.min(u32::from(u8::MAX))).unwrap_or(u8::MAX)
}

/// One coordinate in the UTF-8 form.
///
/// Below 128 this is the single byte the X10 form would have sent, which is why the
/// two encodings are identical on a small terminal and programs can detect neither.
/// Above it the coordinate becomes a two-byte character, and the ceiling at 2015 is
/// where 32 + 2015 reaches 2047 and the two-byte form runs out.
fn push_position(out: &mut Vec<u8>, coordinate: u32) {
    push_wide(out, (coordinate + 32).min(2047));
}

/// One value of the UTF-8 form, as however many bytes it takes to be a character.
///
/// A surrogate is not a `char` and is dropped rather than encoded, which is the only
/// value the two callers can produce that has no UTF-8 spelling; the button's own
/// ceiling is where `char::from_u32` starts refusing, and no button code or position
/// reaches it.
fn push_wide(out: &mut Vec<u8>, value: u32) {
    let mut buf = [0u8; 4];
    if let Some(ch) = char::from_u32(value) {
        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
    }
}

/// Append a CSI parameter in decimal.
fn push_decimal(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(value.to_string().as_bytes());
}

/// The bytes a program should be given for pasted text.
///
/// With bracketed paste on, the text is wrapped in the markers so the program knows
/// the difference between text arriving and a user typing it — which is the whole
/// point, since it is what lets an editor stop auto-indenting a paste. With it off
/// there is nothing to mark and the text goes as it came.
///
/// The terminator is removed from the text in either case, and that is the one thing
/// here that is not about fidelity. A pasted string containing `ESC[201~` verbatim
/// would close the bracket early, and everything after it in the same paste would be
/// read as keystrokes — a paste could run a command. Stripping it unconditionally
/// costs a few bytes out of a paste that had no business containing them.
#[must_use]
pub fn encode_paste(text: &str, modes: &Modes) -> Vec<u8> {
    const END: &str = "\x1b[201~";
    let mut out = Vec::with_capacity(text.len() + 12);
    if modes.bracketed_paste {
        out.extend_from_slice(b"\x1b[200~");
    }
    for piece in text.split(END) {
        out.extend_from_slice(piece.as_bytes());
    }
    if modes.bracketed_paste {
        out.extend_from_slice(END.as_bytes());
    }
    out
}

/// The bytes a program should be given when the window gains or loses focus, or
/// `None` when it did not ask to be told.
#[must_use]
pub fn encode_focus(gained: bool, modes: &Modes) -> Option<Vec<u8>> {
    if !modes.focus_events {
        return None;
    }
    Some(if gained {
        b"\x1b[I".to_vec()
    } else {
        b"\x1b[O".to_vec()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: Key, mods: Modifiers) -> KeyEvent {
        KeyEvent {
            key,
            mods,
            text: None,
            kind: KeyKind::Press,
        }
    }

    fn encode(key: Key, mods: Modifiers) -> Option<Vec<u8>> {
        encode_key(&press(key, mods), &Modes::default())
    }

    fn bytes(key: Key, mods: Modifiers) -> Vec<u8> {
        encode(key, mods).expect("this key has an encoding")
    }

    /// The encoding as a string, for the tests whose expected bytes are all ASCII.
    fn text(key: Key, mods: Modifiers) -> String {
        String::from_utf8(bytes(key, mods)).expect("this sequence is valid UTF-8")
    }

    fn modes_with(modify: impl FnOnce(&mut Modes)) -> Modes {
        let mut modes = Modes::default();
        modify(&mut modes);
        modes
    }

    fn mouse(
        button: MouseButton,
        action: MouseAction,
        col: u16,
        row: u16,
        mods: Modifiers,
    ) -> MouseEvent {
        MouseEvent {
            button,
            action,
            col,
            row,
            mods,
        }
    }

    fn mouse_modes(mode: MouseMode, encoding: MouseEncoding) -> Modes {
        Modes {
            mouse: mode,
            mouse_encoding: encoding,
            ..Modes::default()
        }
    }

    #[test]
    fn every_ctrl_letter_is_its_position_in_the_alphabet() {
        for (at, ch) in ('a'..='z').enumerate() {
            let expected = vec![u8::try_from(at).unwrap() + 1];
            assert_eq!(bytes(Key::Char(ch), Modifiers::CTRL), expected, "Ctrl+{ch}");
            assert_eq!(
                bytes(Key::Char(ch.to_ascii_uppercase()), Modifiers::CTRL),
                expected,
                "Ctrl+{}",
                ch.to_ascii_uppercase()
            );
        }
    }

    #[test]
    fn ctrl_a_and_ctrl_z_are_the_ends_of_the_range() {
        assert_eq!(bytes(Key::Char('a'), Modifiers::CTRL), vec![0x01]);
        assert_eq!(bytes(Key::Char('z'), Modifiers::CTRL), vec![0x1a]);
    }

    #[test]
    fn the_punctuation_control_codes() {
        for (ch, byte) in [
            (' ', 0x00),
            ('@', 0x00),
            ('[', 0x1b),
            ('\\', 0x1c),
            (']', 0x1d),
            ('^', 0x1e),
            ('_', 0x1f),
            ('?', 0x7f),
        ] {
            assert_eq!(
                bytes(Key::Char(ch), Modifiers::CTRL),
                vec![byte],
                "Ctrl+{ch}"
            );
        }
    }

    #[test]
    fn the_named_punctuation_controls_the_same_way_as_the_characters() {
        assert_eq!(bytes(Key::Space, Modifiers::CTRL), vec![0x00]);
        assert_eq!(bytes(Key::BracketLeft, Modifiers::CTRL), vec![0x1b]);
        assert_eq!(bytes(Key::Backslash, Modifiers::CTRL), vec![0x1c]);
        assert_eq!(bytes(Key::BracketRight, Modifiers::CTRL), vec![0x1d]);
    }

    #[test]
    fn shift_does_not_change_a_character() {
        // The host already applied the layout, so a shifted `a` arrives as `A`.
        assert_eq!(text(Key::Char('A'), Modifiers::SHIFT), "A");
        assert_eq!(text(Key::Char('a'), Modifiers::empty()), "a");
    }

    #[test]
    fn alt_is_an_escape_in_front_of_the_character() {
        assert_eq!(text(Key::Char('a'), Modifiers::ALT), "\x1ba");
        assert_eq!(text(Key::Char('1'), Modifiers::ALT), "\x1b1");
        assert_eq!(
            bytes(Key::Char('a'), Modifiers::ALT | Modifiers::CTRL),
            vec![ESC, 0x01]
        );
    }

    #[test]
    fn alt_prefixes_the_control_keys_too() {
        assert_eq!(bytes(Key::Enter, Modifiers::ALT), vec![ESC, b'\r']);
        assert_eq!(bytes(Key::Backspace, Modifiers::ALT), vec![ESC, 0x7f]);
        assert_eq!(bytes(Key::Tab, Modifiers::ALT), vec![ESC, b'\t']);
    }

    #[test]
    fn a_non_ascii_character_is_utf8() {
        assert_eq!(bytes(Key::Char('é'), Modifiers::empty()), "é".as_bytes());
        assert_eq!(bytes(Key::Char('中'), Modifiers::empty()), "中".as_bytes());
    }

    #[test]
    fn arrows_in_the_normal_cursor_mode() {
        assert_eq!(text(Key::Up, Modifiers::empty()), "\x1b[A");
        assert_eq!(text(Key::Down, Modifiers::empty()), "\x1b[B");
        assert_eq!(text(Key::Right, Modifiers::empty()), "\x1b[C");
        assert_eq!(text(Key::Left, Modifiers::empty()), "\x1b[D");
    }

    #[test]
    fn arrows_in_the_application_cursor_mode() {
        let modes = modes_with(|m| m.app_cursor_keys = true);
        for (key, final_byte) in [
            (Key::Up, b'A'),
            (Key::Down, b'B'),
            (Key::Right, b'C'),
            (Key::Left, b'D'),
        ] {
            assert_eq!(
                encode_key(&press(key, Modifiers::empty()), &modes).unwrap(),
                vec![ESC, b'O', final_byte]
            );
        }
    }

    #[test]
    fn home_and_end_in_both_cursor_modes() {
        assert_eq!(text(Key::Home, Modifiers::empty()), "\x1b[H");
        assert_eq!(text(Key::End, Modifiers::empty()), "\x1b[F");
        let modes = modes_with(|m| m.app_cursor_keys = true);
        assert_eq!(
            encode_key(&press(Key::Home, Modifiers::empty()), &modes).unwrap(),
            b"\x1bOH"
        );
        assert_eq!(
            encode_key(&press(Key::End, Modifiers::empty()), &modes).unwrap(),
            b"\x1bOF"
        );
    }

    #[test]
    fn a_modified_cursor_key_is_csi_even_in_application_mode() {
        let modes = modes_with(|m| m.app_cursor_keys = true);
        assert_eq!(
            encode_key(&press(Key::Up, Modifiers::SHIFT), &modes).unwrap(),
            b"\x1b[1;2A"
        );
    }

    #[test]
    fn the_editing_keypad() {
        assert_eq!(text(Key::Insert, Modifiers::empty()), "\x1b[2~");
        assert_eq!(text(Key::Delete, Modifiers::empty()), "\x1b[3~");
        assert_eq!(text(Key::PageUp, Modifiers::empty()), "\x1b[5~");
        assert_eq!(text(Key::PageDown, Modifiers::empty()), "\x1b[6~");
    }

    #[test]
    fn every_function_key_xterm_defines() {
        let table = [
            (1, "\x1bOP"),
            (2, "\x1bOQ"),
            (3, "\x1bOR"),
            (4, "\x1bOS"),
            (5, "\x1b[15~"),
            (6, "\x1b[17~"),
            (7, "\x1b[18~"),
            (8, "\x1b[19~"),
            (9, "\x1b[20~"),
            (10, "\x1b[21~"),
            (11, "\x1b[23~"),
            (12, "\x1b[24~"),
            (13, "\x1b[25~"),
            (14, "\x1b[26~"),
            (15, "\x1b[28~"),
            (16, "\x1b[29~"),
            (17, "\x1b[31~"),
            (18, "\x1b[32~"),
            (19, "\x1b[33~"),
            (20, "\x1b[34~"),
        ];
        for (n, expected) in table {
            assert_eq!(text(Key::F(n), Modifiers::empty()), expected, "F{n}");
        }
    }

    #[test]
    fn f21_through_f24_have_no_sequence() {
        // xterm's table stops at F20. Returning `None` is deliberate; see `function`.
        for n in 21..=24 {
            assert_eq!(encode(Key::F(n), Modifiers::empty()), None, "F{n}");
        }
    }

    #[test]
    fn a_modified_function_key_carries_the_parameter() {
        assert_eq!(text(Key::F(1), Modifiers::SHIFT), "\x1b[1;2P");
        assert_eq!(text(Key::F(4), Modifiers::CTRL), "\x1b[1;5S");
        assert_eq!(text(Key::F(5), Modifiers::SHIFT), "\x1b[15;2~");
        assert_eq!(text(Key::F(12), Modifiers::ALT), "\x1b[24;3~");
        assert_eq!(text(Key::F(13), Modifiers::SHIFT), "\x1b[25;2~");
    }

    #[test]
    fn the_modifier_parameter_arithmetic() {
        for (mods, m) in [
            (Modifiers::empty(), 1),
            (Modifiers::SHIFT, 2),
            (Modifiers::ALT, 3),
            (Modifiers::SHIFT | Modifiers::ALT, 4),
            (Modifiers::CTRL, 5),
            (Modifiers::SHIFT | Modifiers::CTRL, 6),
            (Modifiers::ALT | Modifiers::CTRL, 7),
            (Modifiers::SHIFT | Modifiers::ALT | Modifiers::CTRL, 8),
            (Modifiers::SUPER, 9),
            (Modifiers::SHIFT | Modifiers::SUPER, 10),
            (Modifiers::SUPER | Modifiers::CTRL, 13),
            (
                Modifiers::SHIFT | Modifiers::ALT | Modifiers::CTRL | Modifiers::SUPER,
                16,
            ),
        ] {
            let expected = if m == 1 {
                "\x1b[A".to_string()
            } else {
                format!("\x1b[1;{m}A")
            };
            assert_eq!(text(Key::Up, mods), expected, "modifiers {mods:?}");
        }
    }

    #[test]
    fn shift_arrow_carries_its_parameter() {
        // The value this crate chose over the bare `CSI A`; see the module docs.
        assert_eq!(text(Key::Left, Modifiers::SHIFT), "\x1b[1;2D");
    }

    #[test]
    fn a_modified_editing_key_carries_its_parameter() {
        assert_eq!(text(Key::Home, Modifiers::SHIFT), "\x1b[1;2H");
        assert_eq!(text(Key::Delete, Modifiers::CTRL), "\x1b[3;5~");
        assert_eq!(text(Key::PageUp, Modifiers::ALT), "\x1b[5;3~");
    }

    #[test]
    fn shift_tab_is_the_short_form() {
        assert_eq!(text(Key::Tab, Modifiers::SHIFT), "\x1b[Z");
    }

    #[test]
    fn shift_tab_with_anything_else_carries_the_parameter() {
        assert_eq!(
            text(Key::Tab, Modifiers::SHIFT | Modifiers::ALT),
            "\x1b[1;4Z"
        );
        assert_eq!(
            text(Key::Tab, Modifiers::SHIFT | Modifiers::CTRL),
            "\x1b[1;6Z"
        );
    }

    #[test]
    fn tab_without_shift_is_a_tab() {
        assert_eq!(text(Key::Tab, Modifiers::empty()), "\t");
        assert_eq!(text(Key::Tab, Modifiers::CTRL), "\t");
    }

    #[test]
    fn enter_is_a_carriage_return() {
        assert_eq!(text(Key::Enter, Modifiers::empty()), "\r");
    }

    #[test]
    fn newline_mode_makes_enter_a_line_feed() {
        let modes = modes_with(|m| m.newline = true);
        assert_eq!(
            encode_key(&press(Key::Enter, Modifiers::empty()), &modes).unwrap(),
            b"\n"
        );
    }

    #[test]
    fn space_with_only_shift_is_a_space() {
        assert_eq!(text(Key::Space, Modifiers::SHIFT), " ");
        assert_eq!(text(Key::Space, Modifiers::empty()), " ");
    }

    #[test]
    fn backspace_is_delete() {
        assert_eq!(bytes(Key::Backspace, Modifiers::empty()), vec![0x7f]);
    }

    #[test]
    fn escape_alone_is_escape() {
        assert_eq!(bytes(Key::Escape, Modifiers::empty()), vec![ESC]);
    }

    #[test]
    fn a_release_encodes_nothing() {
        let event = KeyEvent {
            key: Key::Char('a'),
            mods: Modifiers::empty(),
            text: None,
            kind: KeyKind::Release,
        };
        assert_eq!(encode_key(&event, &Modes::default()), None);
    }

    #[test]
    fn a_repeat_encodes_exactly_what_its_press_does() {
        // The test the whole held-key behaviour rests on: whatever a key sends when
        // it goes down, it sends again for every repeat. Written as an equality
        // against the press rather than against a byte, so it cannot drift — the
        // keys worth checking are the ones whose encoding is complicated enough to
        // have a special case in it.
        for key in [
            Key::Char('a'),
            Key::Backspace,
            Key::Up,
            Key::Down,
            Key::Delete,
            Key::F(5),
        ] {
            for mods in [Modifiers::empty(), Modifiers::CTRL, Modifiers::ALT] {
                let press = KeyEvent {
                    key,
                    mods,
                    text: None,
                    kind: KeyKind::Press,
                };
                let repeat = KeyEvent {
                    kind: KeyKind::Repeat,
                    ..press.clone()
                };
                let modes = Modes::default();
                assert_eq!(
                    encode_key(&repeat, &modes),
                    encode_key(&press, &modes),
                    "{key:?} with {mods:?}: holding the key sends something else than \
                     pressing it"
                );
                assert!(
                    encode_key(&press, &modes).is_some(),
                    "{key:?} with {mods:?} encodes nothing at all, so this case proves \
                     nothing"
                );
            }
        }
    }

    #[test]
    fn the_keys_with_no_sequence_encode_nothing() {
        for key in [
            Key::CapsLock,
            Key::NumLock,
            Key::ScrollLock,
            Key::PrintScreen,
            Key::Pause,
            Key::Menu,
        ] {
            assert_eq!(encode(key, Modifiers::empty()), None, "{key:?}");
        }
    }

    #[test]
    fn an_unmapped_ctrl_combination_still_sends_its_character() {
        assert_eq!(text(Key::Minus, Modifiers::CTRL), "-");
        assert_eq!(text(Key::Char('-'), Modifiers::CTRL), "-");
    }

    #[test]
    fn sgr_press_release_and_motion() {
        let modes = mouse_modes(MouseMode::Button, MouseEncoding::Sgr);
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Press,
                    4,
                    6,
                    Modifiers::empty()
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[<0;5;7M"
        );
        // SGR keeps the button number on release, which is the whole reason for its
        // own final character.
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Release,
                    4,
                    6,
                    Modifiers::empty()
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[<0;5;7m"
        );
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Right,
                    MouseAction::Release,
                    0,
                    0,
                    Modifiers::empty()
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[<2;1;1m"
        );
    }

    #[test]
    fn sgr_motion_and_wheels() {
        let modes = mouse_modes(MouseMode::Motion, MouseEncoding::Sgr);
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Motion,
                    1,
                    1,
                    Modifiers::empty()
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[<32;2;2M"
        );
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::None,
                    MouseAction::Motion,
                    1,
                    1,
                    Modifiers::empty()
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[<35;2;2M"
        );
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::WheelUp,
                    MouseAction::Press,
                    1,
                    1,
                    Modifiers::empty()
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[<64;2;2M"
        );
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::WheelDown,
                    MouseAction::Press,
                    1,
                    1,
                    Modifiers::empty()
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[<65;2;2M"
        );
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Back,
                    MouseAction::Press,
                    1,
                    1,
                    Modifiers::empty()
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[<128;2;2M"
        );
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Forward,
                    MouseAction::Press,
                    1,
                    1,
                    Modifiers::empty()
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[<129;2;2M"
        );
    }

    #[test]
    fn sgr_modifier_bits() {
        let modes = mouse_modes(MouseMode::Button, MouseEncoding::Sgr);
        // Shift 4, Alt 8, Control 16, added to the button.
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Press,
                    0,
                    0,
                    Modifiers::SHIFT
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[<4;1;1M"
        );
        assert_eq!(
            encode_mouse(
                mouse(MouseButton::Left, MouseAction::Press, 0, 0, Modifiers::ALT),
                &modes
            )
            .unwrap(),
            b"\x1b[<8;1;1M"
        );
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Middle,
                    MouseAction::Press,
                    0,
                    0,
                    Modifiers::CTRL
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[<17;1;1M"
        );
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Press,
                    0,
                    0,
                    Modifiers::SHIFT | Modifiers::CTRL
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[<20;1;1M"
        );
    }

    #[test]
    fn the_x10_form_packs_everything_into_bytes() {
        let modes = mouse_modes(MouseMode::Button, MouseEncoding::X10);
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Press,
                    0,
                    0,
                    Modifiers::empty()
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[M\x20\x21\x21"
        );
        // 32 + 3 for the release, and no way to say which button it was.
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Release,
                    0,
                    0,
                    Modifiers::empty()
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[M\x23\x21\x21"
        );
        // 32 + 0 + 32 for motion with the button still down.
        let dragging = mouse_modes(MouseMode::Drag, MouseEncoding::X10);
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Motion,
                    2,
                    3,
                    Modifiers::empty()
                ),
                &dragging
            )
            .unwrap(),
            b"\x1b[M\x40\x23\x24"
        );
    }

    #[test]
    fn the_urxvt_form_is_the_x10_value_in_decimal() {
        let modes = mouse_modes(MouseMode::Button, MouseEncoding::Urxvt);
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Press,
                    4,
                    6,
                    Modifiers::empty()
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[32;5;7M"
        );
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Release,
                    4,
                    6,
                    Modifiers::empty()
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[35;5;7M"
        );
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Press,
                    4,
                    6,
                    Modifiers::SHIFT
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[36;5;7M"
        );
    }

    #[test]
    fn the_utf8_form_widens_only_what_it_has_to() {
        let modes = mouse_modes(MouseMode::Button, MouseEncoding::Utf8);
        // Below 95 the coordinate is still one byte, so the two forms agree.
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Press,
                    0,
                    0,
                    Modifiers::empty()
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[M\x20\x21\x21"
        );
        // At 94 the 1-based position is 95, so 32 + 95 = 127 and it still fits.
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Press,
                    94,
                    0,
                    Modifiers::empty()
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[M\x20\x7f\x21"
        );
        // At 95 it becomes the two-byte UTF-8 form: 32 + 96 = 128.
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Press,
                    95,
                    0,
                    Modifiers::empty()
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[M\x20\xc2\x80\x21"
        );
    }

    #[test]
    fn the_utf8_form_widens_the_thumb_buttons_too() {
        // Back and forward are buttons 8 and 9, which put the report's first value at
        // 32 + 128 and 32 + 129 — 160 and 161, both past the one-byte range. Sent as
        // raw bytes those are `0xA0` and `0xA1`, which are not UTF-8 at all: a program
        // decoding the stream the way this encoding exists to let it has a malformed
        // sequence and no way to recover the button. Only the coordinates were widened.
        let modes = mouse_modes(MouseMode::Button, MouseEncoding::Utf8);
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Back,
                    MouseAction::Press,
                    0,
                    0,
                    Modifiers::empty()
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[M\xc2\xa0\x21\x21"
        );
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Forward,
                    MouseAction::Press,
                    0,
                    0,
                    Modifiers::empty()
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[M\xc2\xa1\x21\x21"
        );
    }

    #[test]
    fn every_value_of_a_utf8_report_is_a_character() {
        // The button, the column and the row all come from different arithmetic and
        // only the last two were ever widened, so the assertion is on the whole report
        // rather than on any one of the three.
        let modes = mouse_modes(MouseMode::Motion, MouseEncoding::Utf8);
        for button in [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Back,
            MouseButton::Forward,
            MouseButton::WheelUp,
            MouseButton::WheelRight,
        ] {
            for mods in [
                Modifiers::empty(),
                Modifiers::SHIFT | Modifiers::CTRL | Modifiers::ALT,
            ] {
                // A column and a row past 95, which is where the widening starts, and a
                // wide terminal's worth of columns on top of that.
                for (col, row) in [(0, 0), (95, 95), (200, 60), (5000, 5000)] {
                    let encoded =
                        encode_mouse(mouse(button, MouseAction::Press, col, row, mods), &modes)
                            .expect("a report is written for every button");
                    assert!(
                        core::str::from_utf8(&encoded).is_ok(),
                        "{button:?} with {mods:?} at ({col}, {row}) produced \
                         {encoded:02x?}, which is not a UTF-8 string"
                    );
                }
            }
        }
    }

    #[test]
    fn coordinates_are_clamped_rather_than_wrapped() {
        let modes = mouse_modes(MouseMode::Button, MouseEncoding::X10);
        let encoded = encode_mouse(
            mouse(
                MouseButton::Left,
                MouseAction::Press,
                900,
                900,
                Modifiers::empty(),
            ),
            &modes,
        )
        .unwrap();
        assert_eq!(encoded.len(), 6);
        assert_eq!(encoded[3], 32, "the button is not a coordinate");
        assert_eq!(&encoded[4..], &[u8::MAX, u8::MAX]);
    }

    #[test]
    fn the_utf8_form_clamps_at_its_own_ceiling() {
        let modes = mouse_modes(MouseMode::Button, MouseEncoding::Utf8);
        let encoded = encode_mouse(
            mouse(
                MouseButton::Left,
                MouseAction::Press,
                60000,
                0,
                Modifiers::empty(),
            ),
            &modes,
        )
        .unwrap();
        // 32 + 2015 = 2047, the top of the two-byte form.
        assert_eq!(encoded[3], 32, "the button is not a coordinate");
        assert_eq!(&encoded[4..6], "\u{7ff}".as_bytes());
    }

    #[test]
    fn the_sgr_form_does_not_clamp_its_coordinates() {
        // Decimal parameters have room, so a wide terminal gets its real column.
        let modes = mouse_modes(MouseMode::Button, MouseEncoding::Sgr);
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Press,
                    999,
                    0,
                    Modifiers::empty()
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[<0;1000;1M"
        );
    }

    #[test]
    fn no_mode_reports_nothing_when_the_program_did_not_ask() {
        for action in [
            MouseAction::Press,
            MouseAction::Release,
            MouseAction::Motion,
        ] {
            let event = mouse(MouseButton::Left, action, 1, 1, Modifiers::empty());
            for encoding in [
                MouseEncoding::X10,
                MouseEncoding::Utf8,
                MouseEncoding::Urxvt,
                MouseEncoding::Sgr,
            ] {
                let modes = mouse_modes(MouseMode::None, encoding);
                assert_eq!(encode_mouse(event, &modes), None, "{action:?} {encoding:?}");
            }
        }
    }

    #[test]
    fn x10_reports_presses_only() {
        let modes = mouse_modes(MouseMode::X10, MouseEncoding::Sgr);
        assert!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Press,
                    1,
                    1,
                    Modifiers::empty()
                ),
                &modes
            )
            .is_some()
        );
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Release,
                    1,
                    1,
                    Modifiers::empty()
                ),
                &modes
            ),
            None
        );
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Motion,
                    1,
                    1,
                    Modifiers::empty()
                ),
                &modes
            ),
            None
        );
    }

    #[test]
    fn button_reports_presses_and_releases_but_no_motion() {
        let modes = mouse_modes(MouseMode::Button, MouseEncoding::Sgr);
        assert!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Release,
                    1,
                    1,
                    Modifiers::empty()
                ),
                &modes
            )
            .is_some()
        );
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Motion,
                    1,
                    1,
                    Modifiers::empty()
                ),
                &modes
            ),
            None
        );
    }

    #[test]
    fn drag_reports_motion_only_while_a_button_is_held() {
        let modes = mouse_modes(MouseMode::Drag, MouseEncoding::Sgr);
        assert!(
            encode_mouse(
                mouse(
                    MouseButton::Left,
                    MouseAction::Motion,
                    1,
                    1,
                    Modifiers::empty()
                ),
                &modes
            )
            .is_some()
        );
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::None,
                    MouseAction::Motion,
                    1,
                    1,
                    Modifiers::empty()
                ),
                &modes
            ),
            None
        );
    }

    #[test]
    fn motion_reports_a_bare_motion() {
        let modes = mouse_modes(MouseMode::Motion, MouseEncoding::Sgr);
        assert_eq!(
            encode_mouse(
                mouse(
                    MouseButton::None,
                    MouseAction::Motion,
                    0,
                    0,
                    Modifiers::empty()
                ),
                &modes
            )
            .unwrap(),
            b"\x1b[<35;1;1M"
        );
    }

    #[test]
    fn bracketed_paste_wraps_the_text() {
        let modes = modes_with(|m| m.bracketed_paste = true);
        assert_eq!(encode_paste("hi", &modes), b"\x1b[200~hi\x1b[201~");
    }

    #[test]
    fn plain_paste_is_the_text_alone() {
        assert_eq!(encode_paste("hi", &Modes::default()), b"hi");
    }

    #[test]
    fn a_paste_cannot_close_its_own_bracket() {
        let modes = modes_with(|m| m.bracketed_paste = true);
        assert_eq!(encode_paste("a\x1b[201~b", &modes), b"\x1b[200~ab\x1b[201~");
    }

    #[test]
    fn the_terminator_is_stripped_from_a_plain_paste_too() {
        // Unconditional by design; `encode_paste` says why.
        assert_eq!(encode_paste("a\x1b[201~b", &Modes::default()), b"ab");
    }

    #[test]
    fn every_occurrence_of_the_terminator_goes() {
        let modes = modes_with(|m| m.bracketed_paste = true);
        assert_eq!(
            encode_paste("\x1b[201~\x1b[201~", &modes),
            b"\x1b[200~\x1b[201~"
        );
    }

    #[test]
    fn focus_events_are_reported_only_when_asked_for() {
        let modes = modes_with(|m| m.focus_events = true);
        assert_eq!(encode_focus(true, &modes).unwrap(), b"\x1b[I");
        assert_eq!(encode_focus(false, &modes).unwrap(), b"\x1b[O");
    }

    #[test]
    fn focus_events_are_silent_when_not_asked_for() {
        assert_eq!(encode_focus(true, &Modes::default()), None);
        assert_eq!(encode_focus(false, &Modes::default()), None);
    }

    // ---- the kitty keyboard protocol -----------------------------------------

    /// A terminal a program has asked for `flags`.
    fn kitty(flags: KeyboardFlags) -> Modes {
        modes_with(|m| m.keyboard = flags)
    }

    /// The sequence a key produces under `flags`, or a panic saying it produced none.
    fn kitty_bytes(event: &KeyEvent, flags: KeyboardFlags) -> String {
        let bytes = encode_key(event, &kitty(flags)).expect("this key has an encoding");
        String::from_utf8(bytes).expect("this sequence is valid UTF-8")
    }

    fn with_kind(key: Key, mods: Modifiers, kind: KeyKind) -> KeyEvent {
        KeyEvent {
            kind,
            ..press(key, mods)
        }
    }

    #[test]
    fn a_program_that_has_asked_for_nothing_gets_the_legacy_bytes() {
        // The whole point of the flags being a set of opt-ins: every test in this file
        // above this line is a program that never sent `CSI = u`, and none of them
        // changed. This is the test that says so from the inside.
        for (key, mods) in [
            (Key::Char('a'), Modifiers::empty()),
            (Key::Char('a'), Modifiers::CTRL),
            (Key::Up, Modifiers::empty()),
            (Key::Escape, Modifiers::empty()),
            (Key::F(5), Modifiers::empty()),
        ] {
            assert_eq!(
                encode(key, mods),
                encode_key(&press(key, mods), &kitty(KeyboardFlags::NONE)),
                "{key:?} with {mods:?}"
            );
        }
    }

    #[test]
    fn the_protocols_own_examples_come_out_right() {
        // Straight from the specification's table, because a sequence that is merely
        // self-consistent is worth nothing here: the program on the other end has
        // these numbers written down.
        let disambiguate = KeyboardFlags::DISAMBIGUATE;
        // `shift+a -> CSI 97 ; 2 ; 65 u`, from the section on associated text — which
        // is a flag on top of the all-keys flag, so that is where the example lives.
        // Without the text the same key is `CSI 97;2u`, which the next test checks.
        assert_eq!(
            kitty_bytes(
                &KeyEvent {
                    text: Some("A".into()),
                    ..press(Key::Char('A'), Modifiers::SHIFT)
                },
                KeyboardFlags::ALL_KEYS | KeyboardFlags::ASSOCIATED_TEXT
            ),
            "\x1b[97;2;65u"
        );
        // `ctrl+shift+tab should be CSI 9 ; 6 u`, which the specification lists as a
        // correction: fixterms sent `CSI 1 ; 5 Z`, and Tab is one of the three the
        // disambiguate flag exempts — so this is the protocol's answer under the
        // flag that has no exemptions, which the next test is about.
        assert_eq!(
            kitty_bytes(
                &press(Key::Tab, Modifiers::CTRL | Modifiers::SHIFT),
                KeyboardFlags::ALL_KEYS
            ),
            "\x1b[9;6u"
        );
        // The table's own rows for the letters, all with Ctrl held so that they are
        // escape codes rather than text.
        assert_eq!(
            kitty_bytes(&press(Key::Char('i'), Modifiers::CTRL), disambiguate),
            "\x1b[105;5u"
        );
        assert_eq!(
            kitty_bytes(&press(Key::Char('3'), Modifiers::CTRL), disambiguate),
            "\x1b[51;5u"
        );
        assert_eq!(
            kitty_bytes(&press(Key::Semicolon, Modifiers::CTRL), disambiguate),
            "\x1b[59;5u"
        );
    }

    #[test]
    fn the_code_is_the_key_before_shift_and_never_after_it() {
        // The protocol is explicit about this and gives a wrong example to make the
        // point: `ctrl+shift+a` is `CSI 97;modifiers u` and "must not be CSI 65;
        // modifiers u". A program matching a shortcut looks for the code of the
        // unshifted key, so a terminal that sends the shifted one hands it a chord it
        // will never match.
        let flags = KeyboardFlags::ALL_KEYS;
        assert_eq!(
            kitty_bytes(&press(Key::Char('A'), Modifiers::SHIFT), flags),
            "\x1b[97;2u"
        );
        assert_eq!(
            kitty_bytes(
                &press(Key::Char('A'), Modifiers::CTRL | Modifiers::SHIFT),
                flags
            ),
            "\x1b[97;6u"
        );
        // And the punctuation shift moves, which is the other half of the same fact:
        // Shift+1 arrives as `!` and is still reported as the key `1`.
        assert_eq!(
            kitty_bytes(&press(Key::Char('!'), Modifiers::SHIFT), flags),
            "\x1b[49;2u"
        );
    }

    #[test]
    fn a_repeat_and_a_release_are_told_apart_from_a_press_only_with_event_types() {
        // Without the flag, a repeat is a press — the bytes are the same, and the
        // program asked for nothing better. With it, the type goes in a sub-field of
        // the modifier parameter, which is why the parameter appears even with
        // nothing held: a sub-field needs a field to be a sub-field of.
        let arrow = press(Key::Up, Modifiers::empty());
        assert_eq!(
            kitty_bytes(
                &with_kind(Key::Up, Modifiers::empty(), KeyKind::Repeat),
                KeyboardFlags::ALL_KEYS
            ),
            "\x1b[A"
        );
        assert_eq!(
            kitty_bytes(
                &with_kind(Key::Up, Modifiers::empty(), KeyKind::Repeat),
                KeyboardFlags::ALL_KEYS | KeyboardFlags::EVENT_TYPES
            ),
            "\x1b[1;1:2A"
        );
        assert_eq!(
            kitty_bytes(
                &with_kind(Key::Up, Modifiers::empty(), KeyKind::Release),
                KeyboardFlags::ALL_KEYS | KeyboardFlags::EVENT_TYPES
            ),
            "\x1b[1;1:3A",
            "the release of an unmodified arrow, which legacy has no byte for"
        );
        // A press carries no type at all, which is what makes a sequence from a
        // program that asked for event types identical to one from a program that did
        // not, in the case they share.
        assert_eq!(
            kitty_bytes(&arrow, KeyboardFlags::ALL_KEYS | KeyboardFlags::EVENT_TYPES),
            "\x1b[A"
        );
        // And an empty event type is not enough: the type lands after the modifiers.
        assert_eq!(
            kitty_bytes(
                &with_kind(Key::Up, Modifiers::CTRL, KeyKind::Release),
                KeyboardFlags::ALL_KEYS | KeyboardFlags::EVENT_TYPES
            ),
            "\x1b[1;5:3A"
        );
    }

    #[test]
    fn a_release_is_nothing_at_all_until_a_program_asks_for_event_types() {
        // Not a new rule, the old one seen from the other side: legacy has no byte for
        // a key going up, so without the flag there is nothing to send.
        assert_eq!(
            encode_key(
                &with_kind(Key::Char('a'), Modifiers::empty(), KeyKind::Release),
                &kitty(KeyboardFlags::ALL_KEYS)
            ),
            None
        );
        assert_eq!(
            kitty_bytes(
                &with_kind(Key::Char('a'), Modifiers::empty(), KeyKind::Release),
                KeyboardFlags::ALL_KEYS | KeyboardFlags::EVENT_TYPES
            ),
            "\x1b[97;1:3u"
        );
    }

    #[test]
    fn the_disambiguate_flag_moves_only_the_keys_that_were_ambiguous() {
        // The five the protocol names: Escape, alt+key, ctrl+key, ctrl+alt+key, and
        // shift+alt+key. Shift alone is not among them — `A` cannot be mistaken for
        // the start of anything — so it is still sent as the text it is.
        let flags = KeyboardFlags::DISAMBIGUATE;
        assert_eq!(
            kitty_bytes(&press(Key::Escape, Modifiers::empty()), flags),
            "\x1b[27u"
        );
        assert_eq!(
            kitty_bytes(&press(Key::Char('a'), Modifiers::CTRL), flags),
            "\x1b[97;5u"
        );
        assert_eq!(
            kitty_bytes(&press(Key::Char('a'), Modifiers::ALT), flags),
            "\x1b[97;3u"
        );
        assert_eq!(
            kitty_bytes(
                &press(Key::Char('a'), Modifiers::CTRL | Modifiers::ALT),
                flags
            ),
            "\x1b[97;7u"
        );
        assert_eq!(
            kitty_bytes(
                &press(Key::Char('A'), Modifiers::SHIFT | Modifiers::ALT),
                flags
            ),
            "\x1b[97;4u"
        );
        // And the ones it leaves alone.
        assert_eq!(
            kitty_bytes(&press(Key::Char('A'), Modifiers::SHIFT), flags),
            "A",
            "shift on its own still produces text"
        );
        assert_eq!(
            kitty_bytes(&press(Key::Char('a'), Modifiers::empty()), flags),
            "a"
        );
    }

    #[test]
    fn enter_tab_and_backspace_keep_their_bytes_under_disambiguate() {
        // The protocol carves these three out by name, and the reason is the shell: a
        // program that set the flag and crashed without clearing it would otherwise
        // leave a terminal whose Enter key produces no newline, and the user could not
        // type `reset` to get out of it. So ctrl+Enter is still a carriage return.
        let flags = KeyboardFlags::DISAMBIGUATE;
        for (key, want) in [
            (Key::Enter, "\r"),
            (Key::Tab, "\t"),
            (Key::Backspace, "\u{7f}"),
        ] {
            assert_eq!(
                kitty_bytes(&press(key, Modifiers::CTRL), flags),
                want,
                "{key:?} with Ctrl"
            );
        }
        // Under the all-keys flag there is no exception: every key is an escape code,
        // and the protocol says so in as many words.
        let all = KeyboardFlags::ALL_KEYS;
        assert_eq!(
            kitty_bytes(&press(Key::Enter, Modifiers::empty()), all),
            "\x1b[13u"
        );
        assert_eq!(
            kitty_bytes(&press(Key::Tab, Modifiers::empty()), all),
            "\x1b[9u"
        );
        assert_eq!(
            kitty_bytes(&press(Key::Backspace, Modifiers::empty()), all),
            "\x1b[127u"
        );
    }

    #[test]
    fn the_all_keys_flag_reports_every_key_including_the_ones_that_were_text() {
        let flags = KeyboardFlags::ALL_KEYS;
        assert_eq!(
            kitty_bytes(&press(Key::Char('a'), Modifiers::empty()), flags),
            "\x1b[97u"
        );
        assert_eq!(
            kitty_bytes(&press(Key::Char('a'), Modifiers::SHIFT), flags),
            "\x1b[97;2u"
        );
        assert_eq!(
            kitty_bytes(&press(Key::Space, Modifiers::empty()), flags),
            "\x1b[32u"
        );
        // A lock key is not text and has no character, so the protocol gives it a
        // number in the private use area.
        assert_eq!(
            kitty_bytes(&press(Key::CapsLock, Modifiers::empty()), flags),
            "\x1b[57358u"
        );
        assert_eq!(
            kitty_bytes(&press(Key::PrintScreen, Modifiers::empty()), flags),
            "\x1b[57361u"
        );
    }

    #[test]
    fn the_function_keys_the_legacy_table_stops_at_are_reachable_under_the_protocol() {
        // F21 through F24 have no entry in xterm's table — `function` documents that
        // gap and returns nothing for them — and the protocol is what fills it: its
        // table numbers them from F13's 57376 upward.
        let flags = KeyboardFlags::ALL_KEYS;
        assert_eq!(
            kitty_bytes(&press(Key::F(13), Modifiers::empty()), flags),
            "\x1b[57376u"
        );
        assert_eq!(
            kitty_bytes(&press(Key::F(21), Modifiers::empty()), flags),
            "\x1b[57384u"
        );
        assert_eq!(
            kitty_bytes(&press(Key::F(24), Modifiers::empty()), flags),
            "\x1b[57387u"
        );
        // F1 through F12 have a sequence of their own and keep it, because that is the
        // sequence the protocol's table gives them too.
        assert_eq!(
            kitty_bytes(&press(Key::F(1), Modifiers::empty()), flags),
            "\x1bOP"
        );
        assert_eq!(
            kitty_bytes(&press(Key::F(5), Modifiers::empty()), flags),
            "\x1b[15~"
        );
    }

    #[test]
    fn a_function_key_keeps_its_sequence_under_the_protocol_and_only_gains_a_type() {
        // The protocol's table gives the arrow keys `CSI 1 A` and the editing keypad
        // `CSI 2 ~`, which is what they have always been. Nothing about them changes
        // under the protocol, which is why a program that opts in does not stop
        // understanding its own terminfo entry.
        let flags = KeyboardFlags::DISAMBIGUATE | KeyboardFlags::ALL_KEYS;
        assert_eq!(
            kitty_bytes(&press(Key::Up, Modifiers::empty()), flags),
            "\x1b[A"
        );
        assert_eq!(
            kitty_bytes(&press(Key::Up, Modifiers::CTRL), flags),
            "\x1b[1;5A"
        );
        assert_eq!(
            kitty_bytes(&press(Key::Insert, Modifiers::empty()), flags),
            "\x1b[2~"
        );
        assert_eq!(
            kitty_bytes(&press(Key::Delete, Modifiers::SHIFT), flags),
            "\x1b[3;2~"
        );
        assert_eq!(
            kitty_bytes(&press(Key::PageDown, Modifiers::ALT), flags),
            "\x1b[6;3~"
        );
    }

    #[test]
    fn the_associated_text_rides_inside_the_escape_code_and_nowhere_else() {
        // This is the one thing in the crate that reads `KeyEvent::text`, and the
        // reason it is safe is that the text goes *inside* a sequence the program
        // asked for rather than being written to the pty as bytes a shell would take
        // for typing. The protocol's own example is the first one.
        let flags = KeyboardFlags::ALL_KEYS | KeyboardFlags::ASSOCIATED_TEXT;
        let shift_a = KeyEvent {
            text: Some("A".into()),
            ..press(Key::Char('A'), Modifiers::SHIFT)
        };
        assert_eq!(kitty_bytes(&shift_a, flags), "\x1b[97;2;65u");

        // Without the flag, or without the all-keys flag it is defined against, the
        // text is not read at all.
        assert_eq!(kitty_bytes(&shift_a, KeyboardFlags::ALL_KEYS), "\x1b[97;2u");
        assert_eq!(
            kitty_bytes(&shift_a, KeyboardFlags::ASSOCIATED_TEXT),
            "A",
            "the associated-text flag alone is undefined, so the key stays text"
        );

        // A key with no text gets no field rather than an empty one, and a key with
        // no modifiers gets the modifier field written out so the text has somewhere
        // to sit.
        assert_eq!(
            kitty_bytes(&press(Key::Up, Modifiers::empty()), flags),
            "\x1b[A"
        );
        let plain = KeyEvent {
            text: Some("a".into()),
            ..press(Key::Char('a'), Modifiers::empty())
        };
        assert_eq!(kitty_bytes(&plain, flags), "\x1b[97;1;97u");
    }

    #[test]
    fn a_key_the_protocol_has_no_number_for_is_left_to_the_legacy_encoding() {
        // Nothing here is invented to fill a gap in the table. The protocol numbers
        // F1 through F35 and the keys with characters, and a key outside that is one
        // no program can be told about in this encoding either.
        let flags = KeyboardFlags::ALL_KEYS;
        assert_eq!(
            encode_key(&press(Key::F(40), Modifiers::empty()), &kitty(flags)),
            None
        );
    }
}
