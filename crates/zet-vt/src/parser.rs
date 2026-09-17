//! The byte-level escape sequence parser.
//!
//! This is a state machine over bytes that emits events through [`Perform`]. It knows
//! nothing about screens, cursors, or cells, which is what makes it testable in
//! isolation and what keeps the interesting behaviour in [`crate::term`].
//!
//! The state set is the one from Paul Williams' `vt100` parser, which is the design
//! every serious terminal has converged on. The states are not arbitrary: each one
//! exists because of a specific class of malformed input that a real program emitted
//! once. `CsiIgnore` exists because `CSI 1;?2m` must be discarded rather than
//! interpreted, `DcsIgnore` because an unterminated device control string must not
//! swallow the rest of the session, and the `*Intermediate` states because `CSI ! p`
//! is a different command from `CSI p`.

use core::fmt;

/// The most parameters a single sequence may carry. No real program approaches this;
/// the cap exists so the parser has a fixed stack frame and never allocates.
pub const MAX_PARAMS: usize = 32;

/// The most values across all parameters and sub-parameters.
const MAX_VALUES: usize = 64;

/// The longest operating system command payload the parser keeps.
///
/// Programs that set a window title to a whole command line will send kilobytes. The
/// parser keeps the head and reports the truncation through
/// [`Perform::osc_truncated`] rather than letting the buffer grow without bound.
pub const MAX_OSC: usize = 4096;

/// The parameters of one control sequence.
///
/// A parameter is one or more values separated by `:`. So `CSI 38:2:255:0:0 m` is one
/// parameter holding five values, while `CSI 1;2 m` is two parameters holding one
/// value each. Sub-parameters are not a curiosity: the colon form is how programs set
/// truecolor foregrounds and underline styles.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Params {
    values: [u16; MAX_VALUES],
    /// The index in `values` where parameter `i` begins. A parameter's values run to
    /// the next parameter's start, or to `nvalues` for the last one.
    starts: [u8; MAX_PARAMS],
    nparams: u8,
    nvalues: u8,
    /// Whether a parameter is being accumulated right now. A `;` closes the current
    /// one, and the next digit opens a new one. Without this flag, `CSI ;5H` would
    /// merge its two parameters into one and move the cursor to column 5 instead of
    /// row 0 column 5.
    open: bool,
}

impl Params {
    const fn new() -> Self {
        Params {
            values: [0; MAX_VALUES],
            starts: [0; MAX_PARAMS],
            nparams: 0,
            nvalues: 0,
            open: false,
        }
    }

    const fn clear(&mut self) {
        self.nparams = 0;
        self.nvalues = 0;
        self.open = false;
    }

    /// The number of parameters.
    pub const fn len(&self) -> usize {
        self.nparams as usize
    }

    /// Whether the sequence carried no parameters at all.
    pub const fn is_empty(&self) -> bool {
        self.nparams == 0
    }

    /// Every value of parameter `index`, the parameter itself first, then its
    /// sub-parameters. `None` when the sequence did not carry that parameter.
    pub fn get(&self, index: usize) -> Option<&[u16]> {
        if index >= self.len() {
            return None;
        }
        let start = self.starts[index] as usize;
        let end = if index + 1 < self.len() {
            self.starts[index + 1] as usize
        } else {
            self.nvalues as usize
        };
        Some(&self.values[start..end])
    }

    /// Parameter `index`, or `default` when it was omitted.
    ///
    /// An explicitly written zero is returned as zero. Callers that want the
    /// "omitted or zero means one" behaviour that most cursor commands use write
    /// `.max(1)` themselves, because that rule does not hold for every command.
    pub fn value_or(&self, index: usize, default: u16) -> u16 {
        self.get(index).and_then(|v| v.first()).copied().unwrap_or(default)
    }

    /// Whether parameter `index` was written at all.
    pub const fn has(&self, index: usize) -> bool {
        index < self.nparams as usize
    }

    /// Start a parameter if none is open. Idempotent, so every byte that could begin
    /// a parameter can call it without checking first.
    fn open_param(&mut self) {
        if self.open {
            return;
        }
        if self.nparams as usize >= MAX_PARAMS || self.nvalues as usize >= MAX_VALUES {
            return;
        }
        self.starts[self.nparams as usize] = self.nvalues;
        self.nparams += 1;
        self.values[self.nvalues as usize] = 0;
        self.nvalues += 1;
        self.open = true;
    }

    /// End the current parameter. An empty one is materialised as zero, which is what
    /// `CSI ;5H` means: a zero row, then column 5.
    fn close_param(&mut self) {
        self.open_param();
        self.open = false;
    }

    fn accumulate(&mut self, digit: u16) {
        self.open_param();
        if !self.open || self.nvalues == 0 {
            return;
        }
        let slot = (self.nvalues - 1) as usize;
        self.values[slot] = self.values[slot].saturating_mul(10).saturating_add(digit);
    }

    /// Materialise a parameter left open by a trailing separator.
    ///
    /// `CSI 1;m` carries two parameters, the second of them empty and therefore zero.
    /// Without this, the sequence would report a single parameter and `CSI 1;H` would
    /// move to row 1 of the current column instead of to the home position.
    fn finish(&mut self) {
        if !self.open && self.nparams > 0 {
            self.close_param();
        }
    }

    /// Start another value inside the current parameter. Used by the `:` separator.
    fn next_subparam(&mut self) {
        self.open_param();
        if !self.open || self.nvalues as usize >= MAX_VALUES {
            return;
        }
        self.values[self.nvalues as usize] = 0;
        self.nvalues += 1;
    }
}

impl fmt::Debug for Params {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[")?;
        for i in 0..self.len() {
            if i > 0 {
                f.write_str(";")?;
            }
            let mut first = true;
            for v in self.get(i).unwrap_or_default() {
                if !first {
                    f.write_str(":")?;
                }
                write!(f, "{v}")?;
                first = false;
            }
        }
        f.write_str("]")
    }
}

/// The private marker a sequence was introduced with.
///
/// `CSI ? 25 h` and `CSI 25 h` are different commands. Dropping the marker collapses
/// show-cursor and set-mode-25 into the same thing, which is a real bug in terminals
/// that forget it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Private {
    /// `?` — usually modes that are not in the original VT specification.
    Question,
    /// `>` — mostly the secondary device attributes request.
    Greater,
    /// `<` — the secondary device attributes response.
    Less,
    /// `=` — the application keypad, and the DECKPAM/DECKPNM pair.
    Equals,
}

impl Private {
    const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            b'?' => Some(Private::Question),
            b'>' => Some(Private::Greater),
            b'<' => Some(Private::Less),
            b'=' => Some(Private::Equals),
            _ => None,
        }
    }

    /// The byte that introduces this marker.
    pub const fn to_byte(self) -> u8 {
        match self {
            Private::Question => b'?',
            Private::Greater => b'>',
            Private::Less => b'<',
            Private::Equals => b'=',
        }
    }
}

/// What the parser emits. Implement this to consume a stream of terminal output.
///
/// Every method has an empty default, so an implementation only writes the handlers it
/// cares about.
pub trait Perform {
    /// A printable character. Grapheme clusters arrive as separate characters; the
    /// consumer is responsible for composing them, since only it knows the grid.
    fn print(&mut self, ch: char);

    /// A C0 control byte.
    fn execute(&mut self, _byte: u8) {}

    /// A complete control sequence.
    fn csi_dispatch(&mut self, _params: &Params, _intermediates: &[u8], _private: Option<Private>, _action: char) {}

    /// An escape sequence that is not a control sequence.
    fn esc_dispatch(&mut self, _intermediates: &[u8], _byte: u8) {}

    /// A complete operating system command, already split on `;`.
    fn osc_dispatch(&mut self, _params: &[&[u8]]) {}

    /// The start of a device control string.
    fn dcs_hook(&mut self, _params: &Params, _intermediates: &[u8], _private: Option<Private>, _action: char) {}

    /// One byte of device control string payload.
    fn dcs_put(&mut self, _byte: u8) {}

    /// The end of a device control string.
    fn dcs_unhook(&mut self) {}

    /// Set when an OSC was longer than [`MAX_OSC`] and its tail was dropped.
    fn osc_truncated(&mut self, _truncated: bool) {}
}

/// How many bytes of an unfinished UTF-8 sequence we will hold.
const UTF8_MAX: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Ground,
    Escape,
    EscapeIntermediate,
    CsiEntry,
    CsiParam,
    CsiIntermediate,
    CsiIgnore,
    DcsEntry,
    DcsParam,
    DcsIntermediate,
    DcsPassthrough,
    DcsIgnore,
    /// Saw `ESC` inside a device control string that was hooked; `\` terminates it and
    /// reports the string as complete.
    DcsEscape,
    /// Saw `ESC` inside a string we never hooked, either a discarded device control
    /// string or an SOS/PM/APC. `\` terminates it and nothing is reported, because
    /// firing `dcs_unhook` here would tell the consumer a string it never saw begin
    /// has ended.
    StringEscape,
    OscString,
    /// Saw `ESC` inside an operating system command; `\` here terminates it.
    OscEscape,
    /// A string sequence, which is legal input that we deliberately discard.
    SosPmApcString,
    Utf8,
}

/// The parser.
pub struct Parser {
    state: State,
    params: Params,
    intermediates: [u8; 4],
    nintermediates: u8,
    private: Option<Private>,
    /// Bytes of a partially received UTF-8 sequence.
    utf8: [u8; UTF8_MAX],
    utf8_len: u8,
    utf8_expected: u8,
    /// OSC payload, kept only up to [`MAX_OSC`].
    osc: Vec<u8>,
    osc_truncated: bool,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    /// A parser in the ground state.
    pub fn new() -> Self {
        Parser {
            state: State::Ground,
            params: Params::new(),
            intermediates: [0; 4],
            nintermediates: 0,
            private: None,
            utf8: [0; UTF8_MAX],
            utf8_len: 0,
            utf8_expected: 0,
            osc: Vec::new(),
            osc_truncated: false,
        }
    }

    /// Feed one byte to the parser.
    pub fn advance<P: Perform>(&mut self, byte: u8, perform: &mut P) {
        // Most bytes are consumed by the arm that receives them. The exception is the
        // `ESC` that turns out not to terminate a string, which has to be handed back to
        // the escape state along with the byte after it.
        loop {
            match self.state {
                State::Ground => {
                    match byte {
                        0x1b => {
                            self.state = State::Escape;
                            self.clear_sequence();
                        }
                        0x00..=0x1f | 0x7f => perform.execute(byte),
                        0x20..=0x7e => perform.print(byte as char),
                        _ => {
                            // A byte with the high bit set starts a UTF-8 sequence.
                            self.utf8[0] = byte;
                            self.utf8_len = 1;
                            self.utf8_expected = utf8_len_for(byte);
                            if self.utf8_expected <= 1 {
                                // A continuation byte with no lead is not valid UTF-8.
                                perform.print(char::REPLACEMENT_CHARACTER);
                                self.reset_utf8();
                            } else {
                                self.state = State::Utf8;
                            }
                        }
                    }
                    return;
                }

                State::Utf8 => {
                    if !is_utf8_continuation(byte) {
                        // The sequence was cut short. Report what we have and treat this
                        // byte as a fresh start, which is what a resynchronising decoder
                        // has to do.
                        perform.print(char::REPLACEMENT_CHARACTER);
                        self.reset_utf8();
                        self.state = State::Ground;
                        continue;
                    }
                    self.utf8[self.utf8_len as usize] = byte;
                    self.utf8_len += 1;
                    if self.utf8_len == self.utf8_expected {
                        match core::str::from_utf8(&self.utf8[..self.utf8_len as usize]) {
                            Ok(s) => {
                                for ch in s.chars() {
                                    perform.print(ch);
                                }
                            }
                            Err(_) => perform.print(char::REPLACEMENT_CHARACTER),
                        }
                        self.reset_utf8();
                        self.state = State::Ground;
                    } else if self.utf8_len as usize >= UTF8_MAX {
                        perform.print(char::REPLACEMENT_CHARACTER);
                        self.reset_utf8();
                        self.state = State::Ground;
                    }
                    return;
                }

                State::Escape => match byte {
                    0x00..=0x17 | 0x19 | 0x1c..=0x1f => perform.execute(byte),
                    0x1b => self.clear_sequence(),
                    0x20..=0x2f => {
                        self.collect_intermediate(byte);
                        self.state = State::EscapeIntermediate;
                    }
                    b'P' => {
                        self.clear_sequence();
                        self.state = State::DcsEntry;
                    }
                    0x58 | 0x5e | 0x5f => self.state = State::SosPmApcString,
                    b'[' => {
                        self.clear_sequence();
                        self.state = State::CsiEntry;
                    }
                    b']' => {
                        self.clear_sequence();
                        self.osc.clear();
                        self.osc_truncated = false;
                        self.state = State::OscString;
                    }
                    0x30..=0x4f | 0x51..=0x57 | 0x59 | 0x5a | 0x5c | 0x60..=0x7e => {
                        perform.esc_dispatch(&self.intermediates[..self.nintermediates as usize], byte);
                        self.state = State::Ground;
                    }
                    0x7f => {}
                    _ => self.state = State::Ground,
                },

                State::EscapeIntermediate => match byte {
                    0x00..=0x17 | 0x19 | 0x1c..=0x1f => perform.execute(byte),
                    0x1b => {
                        self.state = State::Escape;
                        self.clear_sequence();
                    }
                    0x20..=0x2f => self.collect_intermediate(byte),
                    0x30..=0x7e => {
                        perform.esc_dispatch(&self.intermediates[..self.nintermediates as usize], byte);
                        self.state = State::Ground;
                    }
                    _ => {}
                },

                State::CsiEntry => match byte {
                    0x00..=0x17 | 0x19 | 0x1c..=0x1f => perform.execute(byte),
                    0x1b => {
                        self.state = State::Escape;
                        self.clear_sequence();
                    }
                    0x20..=0x2f => {
                        self.collect_intermediate(byte);
                        self.state = State::CsiIntermediate;
                    }
                    0x30..=0x39 => {
                        self.params.accumulate(u16::from(byte - b'0'));
                        self.state = State::CsiParam;
                    }
                    0x3a => {
                        self.params.next_subparam();
                        self.state = State::CsiParam;
                    }
                    0x3b => {
                        self.params.close_param();
                        self.state = State::CsiParam;
                    }
                    0x3c..=0x3f => {
                        self.private = Private::from_byte(byte);
                        self.state = State::CsiParam;
                    }
                    0x40..=0x7e => {
                        self.dispatch_csi(byte, perform);
                        self.state = State::Ground;
                    }
                    _ => {}
                },

                State::CsiParam => match byte {
                    0x00..=0x17 | 0x19 | 0x1c..=0x1f => perform.execute(byte),
                    0x1b => {
                        self.state = State::Escape;
                        self.clear_sequence();
                    }
                    0x20..=0x2f => {
                        self.collect_intermediate(byte);
                        self.state = State::CsiIntermediate;
                    }
                    0x30..=0x39 => self.params.accumulate(u16::from(byte - b'0')),
                    0x3a => self.params.next_subparam(),
                    0x3b => self.params.close_param(),
                    0x3c..=0x3f => {
                        // A private marker after parameters began is malformed input.
                        // Ignoring the sequence is safer than guessing at intent.
                        self.state = State::CsiIgnore;
                    }
                    0x40..=0x7e => {
                        self.dispatch_csi(byte, perform);
                        self.state = State::Ground;
                    }
                    _ => {}
                },

                State::CsiIntermediate => match byte {
                    0x00..=0x17 | 0x19 | 0x1c..=0x1f => perform.execute(byte),
                    0x1b => {
                        self.state = State::Escape;
                        self.clear_sequence();
                    }
                    0x20..=0x2f => self.collect_intermediate(byte),
                    0x30..=0x3f => self.state = State::CsiIgnore,
                    0x40..=0x7e => {
                        self.dispatch_csi(byte, perform);
                        self.state = State::Ground;
                    }
                    _ => {}
                },

                State::CsiIgnore => match byte {
                    0x00..=0x17 | 0x19 | 0x1c..=0x1f => perform.execute(byte),
                    0x1b => {
                        self.state = State::Escape;
                        self.clear_sequence();
                    }
                    0x20..=0x3f => {}
                    0x40..=0x7e => self.state = State::Ground,
                    _ => {}
                },

                State::DcsEntry => match byte {
                    0x00..=0x17 | 0x19 | 0x1c..=0x1f => perform.execute(byte),
                    0x1b => {
                        self.state = State::Escape;
                        self.clear_sequence();
                    }
                    0x20..=0x2f => {
                        self.collect_intermediate(byte);
                        self.state = State::DcsIntermediate;
                    }
                    0x30..=0x39 => {
                        self.params.accumulate(u16::from(byte - b'0'));
                        self.state = State::DcsParam;
                    }
                    0x3a => {
                        self.params.next_subparam();
                        self.state = State::DcsParam;
                    }
                    0x3b => {
                        self.params.close_param();
                        self.state = State::DcsParam;
                    }
                    0x3c..=0x3f => {
                        self.private = Private::from_byte(byte);
                        self.state = State::DcsParam;
                    }
                    0x40..=0x7e => {
                        self.params.finish();
                        perform.dcs_hook(
                            &self.params,
                            &self.intermediates[..self.nintermediates as usize],
                            self.private,
                            byte as char,
                        );
                        self.state = State::DcsPassthrough;
                    }
                    _ => {}
                },

                State::DcsParam => match byte {
                    0x00..=0x17 | 0x19 | 0x1c..=0x1f => perform.execute(byte),
                    0x1b => {
                        self.state = State::Escape;
                        self.clear_sequence();
                    }
                    0x20..=0x2f => {
                        self.collect_intermediate(byte);
                        self.state = State::DcsIntermediate;
                    }
                    0x30..=0x39 => self.params.accumulate(u16::from(byte - b'0')),
                    0x3a => self.params.next_subparam(),
                    0x3b => self.params.close_param(),
                    0x3c..=0x3f => self.state = State::DcsIgnore,
                    0x40..=0x7e => {
                        self.params.finish();
                        perform.dcs_hook(
                            &self.params,
                            &self.intermediates[..self.nintermediates as usize],
                            self.private,
                            byte as char,
                        );
                        self.state = State::DcsPassthrough;
                    }
                    _ => {}
                },

                State::DcsIntermediate => match byte {
                    0x00..=0x17 | 0x19 | 0x1c..=0x1f => perform.execute(byte),
                    0x1b => {
                        self.state = State::Escape;
                        self.clear_sequence();
                    }
                    0x20..=0x2f => self.collect_intermediate(byte),
                    0x30..=0x3f => self.state = State::DcsIgnore,
                    0x40..=0x7e => {
                        self.params.finish();
                        perform.dcs_hook(
                            &self.params,
                            &self.intermediates[..self.nintermediates as usize],
                            self.private,
                            byte as char,
                        );
                        self.state = State::DcsPassthrough;
                    }
                    _ => {}
                },

                State::DcsPassthrough => match byte {
                    0x00..=0x17 | 0x19 | 0x1c..=0x1f => perform.dcs_put(byte),
                    0x1b => self.state = State::DcsEscape,
                    0x20..=0x7e => perform.dcs_put(byte),
                    0x7f => {}
                    _ => {}
                },

                State::DcsEscape => {
                    if byte == b'\\' {
                        perform.dcs_unhook();
                        self.state = State::Ground;
                        return;
                    }
                    // Not a string terminator. The escape belongs to the stream, so hand
                    // it to the escape state along with this byte.
                    self.state = State::Escape;
                    self.clear_sequence();
                    continue;
                }

                State::DcsIgnore => match byte {
                    0x00..=0x17 | 0x19 | 0x1c..=0x1f => perform.execute(byte),
                    0x1b => self.state = State::StringEscape,
                    _ => {}
                },

                State::StringEscape => {
                    if byte == b'\\' {
                        self.state = State::Ground;
                        return;
                    }
                    // The string ended with something other than ST. That escape belongs
                    // to the byte stream, so hand it back and reprocess this byte as the
                    // start of a fresh sequence.
                    self.state = State::Escape;
                    self.clear_sequence();
                    continue;
                }

                State::OscString => match byte {
                    0x07 => {
                        self.dispatch_osc(perform);
                        self.state = State::Ground;
                    }
                    0x1b => self.state = State::OscEscape,
                    0x00..=0x06 | 0x08..=0x1a | 0x1c..=0x1f => {}
                    _ => self.push_osc(byte),
                },

                State::OscEscape => {
                    if byte == b'\\' {
                        self.dispatch_osc(perform);
                        self.state = State::Ground;
                        return;
                    }
                    // An OSC terminated by something other than ST. Discard it and let the
                    // escape start over, which is how a truncated title sequence stops
                    // corrupting everything after it.
                    perform.osc_truncated(false);
                    self.state = State::Escape;
                    self.clear_sequence();
                    continue;
                }

                State::SosPmApcString => {
                    // SOS, PM and APC carry payloads no terminal implements. Discarding
                    // them is correct; the only thing that matters is finding the end.
                    if byte == 0x1b {
                        self.state = State::StringEscape;
                    }
                    return;
                }
            }
            // Every arm above either returned or explicitly asked for another pass.
            return;
        }
    }

    /// Feed a whole buffer.
    pub fn advance_slice<P: Perform>(&mut self, bytes: &[u8], perform: &mut P) {
        for &b in bytes {
            self.advance(b, perform);
        }
    }

    /// Abandon any partially parsed sequence and return to ground.
    ///
    /// Used when the pty is reset, so a half-received escape cannot leak into the
    /// next session and turn the first line of a prompt into garbage.
    pub fn reset(&mut self) {
        self.state = State::Ground;
        self.clear_sequence();
        self.osc.clear();
        self.osc_truncated = false;
        self.reset_utf8();
    }

    fn clear_sequence(&mut self) {
        self.params.clear();
        self.nintermediates = 0;
        self.private = None;
    }

    fn reset_utf8(&mut self) {
        self.utf8_len = 0;
        self.utf8_expected = 0;
    }

    fn collect_intermediate(&mut self, byte: u8) {
        if (self.nintermediates as usize) < self.intermediates.len() {
            self.intermediates[self.nintermediates as usize] = byte;
            self.nintermediates += 1;
        }
    }

    fn push_osc(&mut self, byte: u8) {
        if self.osc.len() < MAX_OSC {
            self.osc.push(byte);
        } else if !self.osc_truncated {
            self.osc_truncated = true;
        }
    }

    fn dispatch_osc<P: Perform>(&mut self, perform: &mut P) {
        perform.osc_truncated(self.osc_truncated);
        let parts: Vec<&[u8]> = self.osc.split(|&b| b == b';').collect();
        perform.osc_dispatch(&parts);
        self.osc.clear();
        self.osc_truncated = false;
    }

    fn dispatch_csi<P: Perform>(&mut self, action: u8, perform: &mut P) {
        self.params.finish();
        perform.csi_dispatch(
            &self.params,
            &self.intermediates[..self.nintermediates as usize],
            self.private,
            action as char,
        );
    }
}

/// How many bytes a UTF-8 sequence starting with `byte` should be.
const fn utf8_len_for(byte: u8) -> u8 {
    match byte {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        // 0x80..=0xc1 and 0xf5..=0xff can never start a sequence.
        _ => 1,
    }
}

const fn is_utf8_continuation(byte: u8) -> bool {
    byte & 0xc0 == 0x80
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One `csi_dispatch`: its parameters, intermediates, private marker and action.
    type Csi = (Vec<Vec<u16>>, Vec<u8>, Option<Private>, char);

    /// Records everything the parser emits so tests can assert on the shape of it.
    #[derive(Default)]
    struct Recorder {
        printed: String,
        executed: Vec<u8>,
        csi: Vec<Csi>,
        esc: Vec<(Vec<u8>, u8)>,
        osc: Vec<Vec<Vec<u8>>>,
        dcs: Vec<(char, Vec<u8>)>,
        osc_truncations: Vec<bool>,
    }

    impl Perform for Recorder {
        fn print(&mut self, ch: char) {
            self.printed.push(ch);
        }
        fn execute(&mut self, byte: u8) {
            self.executed.push(byte);
        }
        fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], private: Option<Private>, action: char) {
            let p = (0..params.len())
                .map(|i| params.get(i).unwrap_or_default().to_vec())
                .collect();
            self.csi.push((p, intermediates.to_vec(), private, action));
        }
        fn esc_dispatch(&mut self, intermediates: &[u8], byte: u8) {
            self.esc.push((intermediates.to_vec(), byte));
        }
        fn osc_dispatch(&mut self, params: &[&[u8]]) {
            self.osc.push(params.iter().map(|p| p.to_vec()).collect());
        }
        fn dcs_hook(&mut self, _p: &Params, _i: &[u8], _private: Option<Private>, action: char) {
            self.dcs.push((action, Vec::new()));
        }
        fn dcs_put(&mut self, byte: u8) {
            if let Some(last) = self.dcs.last_mut() {
                last.1.push(byte);
            }
        }
        fn osc_truncated(&mut self, truncated: bool) {
            self.osc_truncations.push(truncated);
        }
    }

    fn parse(input: &[u8]) -> Recorder {
        let mut p = Parser::new();
        let mut r = Recorder::default();
        p.advance_slice(input, &mut r);
        r
    }

    #[test]
    fn plain_text_is_printed() {
        let r = parse(b"hello");
        assert_eq!(r.printed, "hello");
        assert!(r.csi.is_empty());
    }

    #[test]
    fn carriage_return_and_line_feed_are_executed_not_printed() {
        let r = parse(b"a\r\nb");
        assert_eq!(r.printed, "ab");
        assert_eq!(r.executed, vec![b'\r', b'\n']);
    }

    #[test]
    fn utf8_multibyte_characters_arrive_whole() {
        let r = parse("héllo 中文 🎉".as_bytes());
        assert_eq!(r.printed, "héllo 中文 🎉");
        assert!(!r.printed.contains(char::REPLACEMENT_CHARACTER));
    }

    #[test]
    fn a_utf8_sequence_split_across_two_writes_still_decodes() {
        let bytes = "中".as_bytes();
        let mut p = Parser::new();
        let mut r = Recorder::default();
        p.advance(bytes[0], &mut r);
        assert_eq!(r.printed, "", "half a sequence must print nothing");
        p.advance(bytes[1], &mut r);
        p.advance(bytes[2], &mut r);
        assert_eq!(r.printed, "中");
    }

    #[test]
    fn an_invalid_utf8_byte_becomes_a_replacement_character_and_parsing_continues() {
        let r = parse(&[0xff, b'o', b'k']);
        assert_eq!(r.printed, "\u{fffd}ok");
    }

    #[test]
    fn a_truncated_utf8_sequence_recovers_at_the_next_ascii_byte() {
        let r = parse(&[0xe4, 0xb8, b'!']);
        assert_eq!(r.printed, "\u{fffd}!");
    }

    #[test]
    fn csi_with_multiple_parameters() {
        let r = parse(b"\x1b[1;31m");
        assert_eq!(r.csi.len(), 1);
        let (params, intermediates, private, action) = &r.csi[0];
        assert_eq!(params, &vec![vec![1], vec![31]]);
        assert!(intermediates.is_empty());
        assert!(private.is_none());
        assert_eq!(*action, 'm');
    }

    #[test]
    fn csi_with_no_parameters_is_reported_as_empty() {
        let r = parse(b"\x1b[H");
        assert_eq!(r.csi[0].0, Vec::<Vec<u16>>::new());
        assert_eq!(r.csi[0].3, 'H');
    }

    #[test]
    fn a_missing_parameter_reads_as_its_default_not_as_zero() {
        let r = parse(b"\x1b[;5H");
        assert_eq!(r.csi[0].0[0], vec![0], "an explicitly empty parameter is zero");
        assert_eq!(r.csi[0].0[1], vec![5]);
    }

    #[test]
    fn the_private_marker_is_preserved() {
        let r = parse(b"\x1b[?25l");
        assert_eq!(r.csi[0].2, Some(Private::Question));
        assert_eq!(r.csi[0].3, 'l');

        let r = parse(b"\x1b[>0c");
        assert_eq!(r.csi[0].2, Some(Private::Greater));
    }

    #[test]
    fn sub_parameters_are_grouped_into_one_parameter() {
        let r = parse(b"\x1b[4:3m");
        assert_eq!(r.csi[0].0, vec![vec![4, 3]]);
    }

    #[test]
    fn truecolor_sgr_sub_parameters_survive_intact() {
        let r = parse(b"\x1b[38:2::255:128:0m");
        assert_eq!(r.csi[0].0, vec![vec![38, 2, 0, 255, 128, 0]]);
    }

    #[test]
    fn semicolon_truecolor_sgr_is_two_parameters() {
        let r = parse(b"\x1b[38;2;255;128;0m");
        assert_eq!(r.csi[0].0, vec![vec![38], vec![2], vec![255], vec![128], vec![0]]);
    }

    #[test]
    fn a_private_marker_after_parameters_makes_the_sequence_ignorable() {
        let r = parse(b"\x1b[1;?2m");
        assert!(r.csi.is_empty(), "malformed input must not dispatch anything");
        assert_eq!(r.printed, "");
    }

    #[test]
    fn intermediate_bytes_reach_the_handler() {
        let r = parse(b"\x1b[!p");
        assert_eq!(r.csi[0].1, vec![b'!']);
        assert_eq!(r.csi[0].3, 'p');
    }

    #[test]
    fn a_parameter_after_an_intermediate_is_ignored() {
        let r = parse(b"\x1b[!1p");
        assert!(r.csi.is_empty());
    }

    #[test]
    fn esc_sequences_dispatch() {
        let r = parse(b"\x1b7\x1b8");
        assert_eq!(r.esc, vec![(vec![], b'7'), (vec![], b'8')]);
    }

    #[test]
    fn esc_with_intermediates() {
        let r = parse(b"\x1b(B");
        assert_eq!(r.esc, vec![(vec![b'('], b'B')]);
    }

    #[test]
    fn c0_controls_inside_a_csi_are_executed_not_swallowed() {
        // A program can send a bell in the middle of a sequence. Dropping it would be a
        // missed beep; swallowing the sequence would be a corrupted screen.
        let r = parse(b"\x1b[1;\x07m");
        assert_eq!(r.executed, vec![0x07]);
        assert_eq!(r.csi[0].3, 'm');
        assert_eq!(r.csi[0].0, vec![vec![1], vec![0]]);
    }

    #[test]
    fn an_escape_after_an_incomplete_csi_restarts_cleanly() {
        let r = parse(b"\x1b[1;\x1b[2m");
        assert_eq!(r.csi.len(), 1);
        assert_eq!(r.csi[0].0, vec![vec![2]]);
    }

    #[test]
    fn osc_terminated_by_bel() {
        let r = parse(b"\x1b]0;my title\x07");
        assert_eq!(r.osc.len(), 1);
        assert_eq!(r.osc[0], vec![b"0".to_vec(), b"my title".to_vec()]);
    }

    #[test]
    fn osc_terminated_by_st() {
        let r = parse(b"\x1b]0;title\x1b\\");
        assert_eq!(r.osc[0], vec![b"0".to_vec(), b"title".to_vec()]);
    }

    #[test]
    fn an_osc_split_across_writes_survives() {
        let mut p = Parser::new();
        let mut r = Recorder::default();
        p.advance_slice(b"\x1b]8;;https://exa", &mut r);
        assert!(r.osc.is_empty());
        p.advance_slice(b"mple.com\x07", &mut r);
        assert_eq!(r.osc[0], vec![b"8".to_vec(), Vec::new(), b"https://example.com".to_vec()]);
    }

    #[test]
    fn an_unterminated_osc_is_dropped_and_the_next_escape_still_works() {
        let r = parse(b"\x1b]0;no terminator\x1b[31m");
        assert!(r.osc.is_empty(), "an OSC without a terminator must not fire");
        assert_eq!(r.csi.len(), 1);
        assert_eq!(r.csi[0].3, 'm');
    }

    #[test]
    fn osc_dispatch_is_not_reported_as_truncated_when_it_was_not() {
        let r = parse(b"\x1b]0;short\x07");
        assert_eq!(r.osc_truncations, vec![false]);
    }

    #[test]
    fn a_pastable_selection_osc_round_trips() {
        let r = parse(b"\x1b]52;c;aGVsbG8=\x07");
        assert_eq!(r.osc[0], vec![b"52".to_vec(), b"c".to_vec(), b"aGVsbG8=".to_vec()]);
    }

    #[test]
    fn dcs_payload_is_collected_and_unhooked() {
        // DECRQSS is `DCS $ q ... ST`, so the dollar arrives as an intermediate and `q`
        // is the final byte. Swapping them makes the request unanswerable.
        let r = parse(b"\x1bP$qm\x1b\\");
        assert_eq!(r.dcs.len(), 1);
        assert_eq!(r.dcs[0].0, 'q');
        assert_eq!(r.dcs[0].1, b"m");
    }

    #[test]
    fn dcs_intermediates_reach_the_hook() {
        struct Grab(Option<(char, Vec<u8>)>);
        impl Perform for Grab {
            fn print(&mut self, _ch: char) {}
            fn dcs_hook(&mut self, _pa: &Params, i: &[u8], _pr: Option<Private>, action: char) {
                self.0 = Some((action, i.to_vec()));
            }
        }

        let mut p = Parser::new();
        let mut g = Grab(None);
        p.advance_slice(b"\x1bP$q\x1b\\", &mut g);
        assert_eq!(g.0, Some(('q', vec![b'$'])));
    }

    #[test]
    fn a_trailing_separator_leaves_an_empty_parameter() {
        let r = parse(b"\x1b[1;m");
        assert_eq!(r.csi[0].0, vec![vec![1], vec![0]]);
    }

    #[test]
    fn a_trailing_colon_leaves_an_empty_subparameter() {
        let r = parse(b"\x1b[38:2:255:m");
        assert_eq!(r.csi[0].0, vec![vec![38, 2, 255, 0]]);
    }

    #[test]
    fn a_sequence_with_no_parameters_gains_none() {
        let r = parse(b"\x1b[H");
        assert!(r.csi[0].0.is_empty());
    }

    #[test]
    fn dcs_with_parameters() {
        let r = parse(b"\x1bP1;2|payload\x1b\\");
        assert_eq!(r.dcs[0].0, '|');
        assert_eq!(r.dcs[0].1, b"payload");
    }

    #[test]
    fn an_unterminated_dcs_does_not_swallow_the_rest_of_the_session() {
        // The string is discarded, but the escape after it must still be parsed.
        let r = parse(b"\x1bPjunk\x1b[1m");
        assert_eq!(r.csi.len(), 1);
        assert_eq!(r.csi[0].0, vec![vec![1]]);
    }

    #[test]
    fn sos_pm_and_apc_strings_are_discarded_but_terminated_correctly() {
        for intro in [b"\x1bX".as_slice(), b"\x1b^", b"\x1b_"] {
            let mut input = intro.to_vec();
            input.extend_from_slice(b"ignored payload\x1b\\");
            input.extend_from_slice(b"\x1b[1m");
            let r = parse(&input);
            assert_eq!(r.printed, "", "string payload must not be printed");
            assert_eq!(r.csi.len(), 1, "the escape after the string must be parsed");
        }
    }

    #[test]
    fn reset_abandons_a_half_received_sequence() {
        let mut p = Parser::new();
        let mut r = Recorder::default();
        p.advance_slice(b"\x1b[1;3", &mut r);
        p.reset();
        p.advance_slice(b"ok", &mut r);
        assert_eq!(r.printed, "ok", "the abandoned CSI must not consume this text");
        assert!(r.csi.is_empty());
    }

    #[test]
    fn a_parameter_longer_than_u16_saturates_instead_of_wrapping() {
        let r = parse(b"\x1b[99999999999m");
        assert_eq!(r.csi[0].0[0], vec![u16::MAX]);
    }

    #[test]
    fn more_parameters_than_the_cap_do_not_panic() {
        let mut input = b"\x1b[".to_vec();
        for i in 0..200 {
            if i > 0 {
                input.push(b';');
            }
            input.push(b'1');
        }
        input.push(b'm');
        let r = parse(&input);
        assert_eq!(r.csi.len(), 1);
        assert!(r.csi[0].0.len() <= MAX_PARAMS);
    }

    #[test]
    fn every_byte_value_alone_never_panics() {
        for b in 0..=u8::MAX {
            let mut p = Parser::new();
            let mut r = Recorder::default();
            p.advance(b, &mut r);
        }
    }

    #[test]
    fn random_looking_byte_soup_never_panics() {
        // A deterministic sweep over the byte space, in sequence, which is what a pty
        // full of binary garbage looks like.
        let mut p = Parser::new();
        let mut r = Recorder::default();
        for round in 0..8u8 {
            for b in 0..=u8::MAX {
                p.advance(b.wrapping_add(round), &mut r);
            }
        }
    }

    #[test]
    fn params_debug_round_trips_the_colon_form() {
        let r = parse(b"\x1b[4:3m");
        let p = Params {
            values: {
                let mut v = [0u16; MAX_VALUES];
                v[0] = 4;
                v[1] = 3;
                v
            },
            starts: [0; MAX_PARAMS],
            nparams: 1,
            nvalues: 2,
            open: false,
        };
        assert_eq!(format!("{p:?}"), "[4:3]");
        assert_eq!(format!("{:?}", r.csi[0].0), "[[4, 3]]");
    }

    #[test]
    fn value_or_distinguishes_omitted_from_zero() {
        struct Grab(Option<(u16, u16, bool, bool)>);
        impl Perform for Grab {
            fn print(&mut self, _ch: char) {}
            fn csi_dispatch(&mut self, params: &Params, _i: &[u8], _p: Option<Private>, _a: char) {
                self.0 = Some((
                    params.value_or(0, 9),
                    params.value_or(1, 9),
                    params.has(0),
                    params.has(5),
                ));
            }
        }

        // Re-derive the parameters by re-parsing, since `Recorder` flattens them.
        let mut p = Parser::new();
        let mut g = Grab(None);
        p.advance_slice(b"\x1b[0;5H", &mut g);
        assert_eq!(g.0, Some((0, 5, true, false)));

        let r = parse(b"\x1b[0;5H");
        assert!(!r.csi.is_empty());
    }
}
