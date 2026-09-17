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

use zet_vt::{Modes, MouseEncoding, MouseMode};

use crate::chord::{Key, Modifiers};
use crate::key::{KeyEvent, KeyKind};
use crate::mouse::{MouseAction, MouseButton, MouseEvent};

/// `ESC`, which starts every sequence this crate emits.
const ESC: u8 = 0x1b;

/// The bytes a program should be given for a key event, or `None` for none at all.
///
/// `None` is a real answer and not a failure: it means "write nothing to the pty",
/// which is what a key that this encoding cannot describe has to produce. There are
/// three such keys or events — a release, a repeat, and the handful of keys in
/// [`Key`] that exist so a user can name them (`CapsLock`, `PrintScreen`, `Pause`,
/// `Menu`, the three locks) and that never had a byte of their own. A host that
/// treats `None` as an error will drop the keystroke twice.
///
/// The modifiers are read from the event and the text is not; [`KeyEvent`] says why.
#[must_use]
pub fn encode_key(event: &KeyEvent, modes: &Modes) -> Option<Vec<u8>> {
    // Legacy VT has no byte for a key going up or for one repeating. That is the
    // gap the kitty protocol exists to fill, and this crate does not implement it,
    // so both kinds are dropped rather than approximated by a press — a repeat sent
    // as a press is indistinguishable from the user holding the key down, which is
    // a different thing to a program counting keystrokes.
    if event.kind != KeyKind::Press {
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
fn parameterised_key(event: &KeyEvent, modes: &Modes) -> Option<Vec<u8>> {
    let m = modifier_param(event.mods);
    match event.key {
        Key::Up => Some(cursor(b'A', m, modes)),
        Key::Down => Some(cursor(b'B', m, modes)),
        Key::Right => Some(cursor(b'C', m, modes)),
        Key::Left => Some(cursor(b'D', m, modes)),
        Key::Home => Some(cursor(b'H', m, modes)),
        Key::End => Some(cursor(b'F', m, modes)),
        Key::Insert => Some(tilde(2, m)),
        Key::Delete => Some(tilde(3, m)),
        Key::PageUp => Some(tilde(5, m)),
        Key::PageDown => Some(tilde(6, m)),
        Key::Tab if event.mods.contains(Modifiers::SHIFT) => Some(back_tab(m)),
        Key::F(n) => function(n, m),
        _ => None,
    }
}

/// The value of xterm's `<m>` parameter for a set of modifiers.
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

/// `CSI 1;<m><final>`, the parameter form of a cursor key or a shifted tab.
fn parameterised(lead: u16, m: u8, final_byte: u8) -> Vec<u8> {
    let mut out = vec![ESC, b'['];
    out.extend_from_slice(lead.to_string().as_bytes());
    out.push(b';');
    out.extend_from_slice(m.to_string().as_bytes());
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
fn cursor(final_byte: u8, m: u8, modes: &Modes) -> Vec<u8> {
    if m == 1 {
        let lead = if modes.app_cursor_keys { b'O' } else { b'[' };
        vec![ESC, lead, final_byte]
    } else {
        parameterised(1, m, final_byte)
    }
}

/// `CSI <n>~` and its parameter form, for the editing keypad and F5 upward.
fn tilde(n: u16, m: u8) -> Vec<u8> {
    if m == 1 {
        let mut out = vec![ESC, b'['];
        out.extend_from_slice(n.to_string().as_bytes());
        out.push(b'~');
        out
    } else {
        parameterised(n, m, b'~')
    }
}

/// Shift+Tab.
///
/// Plain Shift keeps xterm's short `CSI Z`, which is what readline's `backward-tab`
/// and every completion menu in existence look for, even though it sits oddly next
/// to `CSI 1;2Z` for Shift+Alt+Tab. The short form is the one in the wild.
fn back_tab(m: u8) -> Vec<u8> {
    if m == 2 {
        vec![ESC, b'[', b'Z']
    } else {
        parameterised(1, m, b'Z')
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
fn function(n: u8, m: u8) -> Option<Vec<u8>> {
    match n {
        1 => Some(function_ss3(b'P', m)),
        2 => Some(function_ss3(b'Q', m)),
        3 => Some(function_ss3(b'R', m)),
        4 => Some(function_ss3(b'S', m)),
        5 => Some(tilde(15, m)),
        6 => Some(tilde(17, m)),
        7 => Some(tilde(18, m)),
        8 => Some(tilde(19, m)),
        9 => Some(tilde(20, m)),
        10 => Some(tilde(21, m)),
        11 => Some(tilde(23, m)),
        12 => Some(tilde(24, m)),
        13 => Some(tilde(25, m)),
        14 => Some(tilde(26, m)),
        15 => Some(tilde(28, m)),
        16 => Some(tilde(29, m)),
        17 => Some(tilde(31, m)),
        18 => Some(tilde(32, m)),
        19 => Some(tilde(33, m)),
        20 => Some(tilde(34, m)),
        _ => None,
    }
}

/// `SS3 <final>`, or `CSI 1;<m><final>` once a modifier has to be carried.
fn function_ss3(final_byte: u8, m: u8) -> Vec<u8> {
    if m == 1 {
        vec![ESC, b'O', final_byte]
    } else {
        parameterised(1, m, final_byte)
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

/// The character a key produces, for the keys that produce one.
///
/// The named punctuation is mapped back to its character here so that the control
/// codes above can be applied to it: `Ctrl+[` and `Ctrl+BracketLeft` are the same
/// keystroke and must produce the same 0x1b.
fn character_of(key: Key) -> Option<char> {
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
            out.push(one_byte(u32::from(32 + x10)));
            out.push(one_byte(32 + col));
            out.push(one_byte(32 + row));
            out
        }
        MouseEncoding::Utf8 => {
            let mut out = vec![ESC, b'[', b'M'];
            // The button stays a single byte here. xterm widens it as well, but no
            // value this crate emits needs it — the largest is back at 160 — and a
            // program reading a UTF-8 stream only has to be right about the
            // coordinates it actually compares.
            out.push(one_byte(u32::from(32 + x10)));
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
    let value = (coordinate + 32).min(2047);
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
    fn release_and_repeat_encode_nothing() {
        for kind in [KeyKind::Release, KeyKind::Repeat] {
            let event = KeyEvent {
                key: Key::Char('a'),
                mods: Modifiers::empty(),
                text: None,
                kind,
            };
            assert_eq!(encode_key(&event, &Modes::default()), None, "{kind:?}");
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
}
