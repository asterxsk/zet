//! The kitty keyboard protocol's progressive enhancement flags, and their stack.
//!
//! A program asks the terminal for more than the legacy key encoding can say by
//! sending `CSI = flags ; mode u`, and the terminal keeps the answer here. The flags
//! are a bit field with five bits defined so far, and every one of them is a promise
//! about what [`crate::Term`] will *report* rather than about how it behaves — which
//! is why none of this changes the screen and all of it is read by the host.
//!
//! # Why there is a stack
//!
//! A program that puts the terminal into its own key mode has to put it back, and it
//! is not the only program that will ever run. `CSI > flags u` saves what was in
//! force and installs new flags; `CSI < n u` restores what was saved. Without it, a
//! program that turned on enhanced keys and then crashed would leave every program
//! after it reading escape codes it never asked for, and the fix would be to close
//! the terminal.
//!
//! The spec asks for two more things that the shape here follows: the stack is
//! bounded, because it is grown by a program rather than by the user and a
//! never-popped push loop is a way to make a terminal allocate without limit; and
//! the main and alternate screens have separate stacks, so that a full-screen editor
//! can change the key mode without knowing or caring what the shell's mode was. That
//! second one is why [`crate::Term`] moves a whole `Keyboard` in and out with the
//! alternate screen rather than keeping one here and saving nothing.

/// One of the progressive enhancement flags, or a set of several.
///
/// A bit field rather than five booleans, because that is how it travels: the program
/// sends the number and the terminal replies with the number, and a struct of bools
/// would need a conversion in both directions that nothing else in this crate needs.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct KeyboardFlags(u8);

impl KeyboardFlags {
    /// No enhancements. The legacy encoding, which is what a program that never asked
    /// for anything gets and expects.
    pub const NONE: Self = Self(0);

    /// `0b1`. Report `Esc`, `alt+key`, `ctrl+key`, `ctrl+alt+key`, and `shift+alt+key`
    /// as `CSI u` sequences instead of the legacy bytes, which are ambiguous with the
    /// beginnings of escape codes.
    ///
    /// This is the one that earns its keep. Under it, `Escape` is `CSI 27 u` rather
    /// than a bare `0x1b`, so a program no longer has to guess whether the byte it just
    /// read is a key or the start of a sequence — the fragile timing hack every
    /// terminal program of the last forty years has had to write.
    pub const DISAMBIGUATE: Self = Self(0b1);

    /// `0b10`. Report key repeat and key release as well as key press.
    ///
    /// This is the flag `zet-input` could not implement without this module: the legacy
    /// encoding has no byte for either, so a repeat had to be sent as a press and a
    /// release had to be dropped.
    pub const EVENT_TYPES: Self = Self(0b10);

    /// `0b100`. Report the shifted key and the key at the same position on the base
    /// layout, so that a shortcut bound to a character works on a layout where that
    /// character needs a different key.
    ///
    /// **Not implemented**, and it is the only one of the five that is not. It wants
    /// two code points beside every key — the shifted one and the one at the same
    /// position on a PC-101 layout — and the host carries neither: a key event
    /// arrives as the character the layout produced and the text it composed, and
    /// the physical key the base-layout one would come from is dropped on the way.
    /// See [`Self::SUPPORTED`] for what happens to a program that asks.
    pub const ALTERNATE_KEYS: Self = Self(0b100);

    /// `0b1000`. Report *every* key as an escape code, including the ones that would
    /// otherwise just be their text.
    ///
    /// This is what a game wants: under it, `w` is a key event rather than the letter
    /// `w`, and it has a release. It implies [`Self::DISAMBIGUATE`], since a key
    /// reported as an escape code is unambiguous by construction.
    pub const ALL_KEYS: Self = Self(0b1000);

    /// `0b10000`. Send the text a key produced as code points inside the escape code,
    /// so that a program can have both the key event and the text.
    ///
    /// Undefined without [`Self::ALL_KEYS`], because a key that is not being reported
    /// as an escape code has nowhere to carry the text.
    pub const ASSOCIATED_TEXT: Self = Self(0b10000);

    /// The flags this terminal implements.
    ///
    /// The protocol is built to be implemented a piece at a time: a program sets the
    /// flags it wants and then queries to find out which it got, and the specification
    /// says that is exactly how a program is meant to discover a terminal that does
    /// only some of them. So the flag this terminal cannot honour — see
    /// [`Self::ALTERNATE_KEYS`] — is dropped from every set that arrives and is never
    /// reported back in the reply to `CSI ? u`. The alternative, echoing a bit nothing
    /// acts on, is a promise the program then relies on and the keys then break.
    pub const SUPPORTED: Self = Self(
        Self::DISAMBIGUATE.0 | Self::EVENT_TYPES.0 | Self::ALL_KEYS.0 | Self::ASSOCIATED_TEXT.0,
    );

    /// The flags a set of bits names, less the ones this terminal does not implement.
    /// Bits above the five defined ones are dropped for the same reason: a program
    /// that sets one is asking for something nothing here does.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits & Self::SUPPORTED.0)
    }

    /// The flags as the number a program sends and reads back.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether every flag in `other` is set here.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether nothing is set, which means the legacy encoding and no stack entry
    /// worth keeping.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// This set with `other` taken away.
    #[must_use]
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

impl core::ops::BitOr for KeyboardFlags {
    type Output = Self;

    /// Two flag sets together, which is how a program asks for two enhancements at
    /// once and how a test says so: `DISAMBIGUATE | ALL_KEYS`.
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// How `CSI = flags ; mode u` asks for its flags to be applied.
///
/// The three modes are the reason a program can turn one enhancement off without
/// disturbing the four it cannot see: it sends `CSI = 2 ; 3 u` and everything else
/// stays as it was.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Apply {
    /// `1`. Every bit in the flags is set and every bit outside them is cleared.
    Set,
    /// `2`. Every bit in the flags is set and the rest are left alone.
    Or,
    /// `3`. Every bit in the flags is cleared and the rest are left alone. The spec
    /// calls this "all set bits are reset", which is the same thing said from the
    /// other side.
    AndNot,
}

impl Apply {
    /// The mode a parameter names, or `None` for one that is not a mode.
    ///
    /// The spec says the parameter is optional and defaults to 1, so an absent
    /// parameter is [`Apply::Set`]; a present one that is not 1, 2, or 3 is not
    /// something to guess at, and the sequence is ignored.
    #[must_use]
    pub const fn from_param(mode: u16) -> Option<Self> {
        match mode {
            1 => Some(Self::Set),
            2 => Some(Self::Or),
            3 => Some(Self::AndNot),
            _ => None,
        }
    }
}

/// How many saved flag sets a program may stack up.
///
/// The spec asks for a limit and does not name one. Sixty-four is more nesting than
/// any program has ever needed — the deepest real use is a multiplexer inside an
/// editor inside a shell — and small enough that the eviction the spec asks for can
/// never be reached by a program that is behaving.
pub const STACK_LIMIT: usize = 64;

/// The keyboard flags in force, and the ones a program has saved.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Keyboard {
    /// What the program last asked for, and what the encoder reads.
    flags: KeyboardFlags,
    /// Values saved by `CSI > u`, oldest first, bounded by [`STACK_LIMIT`].
    saved: Vec<KeyboardFlags>,
}

impl Keyboard {
    /// The flags in force.
    #[must_use]
    pub const fn flags(&self) -> KeyboardFlags {
        self.flags
    }

    /// How many entries are saved. For tests and for a caller that wants to know
    /// whether a program left anything behind.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.saved.len()
    }

    /// Save the flags in force and install `flags`.
    ///
    /// `CSI > flags u`. An omitted parameter is zero rather than "keep what is there",
    /// which is the one reading under which the default in the spec makes sense: the
    /// program is naming what it wants, not what it had.
    pub fn push(&mut self, flags: KeyboardFlags) {
        if self.saved.len() >= STACK_LIMIT {
            // The spec says the *oldest* entry is evicted. Dropping the newest would be
            // the obvious ring-buffer answer and the wrong one: the entry a program is
            // most likely to pop back to is the one it pushed most recently.
            self.saved.remove(0);
        }
        self.saved.push(self.flags);
        self.flags = flags;
    }

    /// Restore what `n` pushes ago saved, and answer the flags now in force.
    ///
    /// `CSI < n u`. A pop that empties the stack resets everything, which the spec asks
    /// for by name: it is what a program that pushed once and popped twice gets, and
    /// resetting is the only answer that does not leave the terminal in a mode nobody
    /// asked for and nobody can turn off.
    pub fn pop(&mut self, n: usize) -> KeyboardFlags {
        for _ in 0..n.max(1) {
            if let Some(restored) = self.saved.pop() {
                self.flags = restored;
            } else {
                self.flags = KeyboardFlags::NONE;
                break;
            }
        }
        self.flags
    }

    /// Apply `flags` by `mode`. `CSI = flags ; mode u`.
    pub fn apply(&mut self, flags: KeyboardFlags, mode: Apply) {
        self.flags = match mode {
            Apply::Set => flags,
            Apply::Or => self.flags | flags,
            Apply::AndNot => self.flags.without(flags),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: KeyboardFlags = KeyboardFlags::ALL_KEYS;

    #[test]
    fn a_terminal_nobody_has_asked_anything_of_is_in_legacy_mode() {
        let keyboard = Keyboard::default();
        assert_eq!(keyboard.flags(), KeyboardFlags::NONE);
        assert!(keyboard.flags().is_empty());
        assert_eq!(keyboard.depth(), 0);
    }

    #[test]
    fn the_bits_are_the_numbers_the_protocol_puts_on_the_wire() {
        // These five numbers are the whole of the protocol's vocabulary and they are
        // not ours to choose: a program sends `CSI = 5 u` and means disambiguate and
        // alternate keys. A typo here is invisible until a program misbehaves.
        assert_eq!(KeyboardFlags::DISAMBIGUATE.bits(), 1);
        assert_eq!(KeyboardFlags::EVENT_TYPES.bits(), 2);
        assert_eq!(KeyboardFlags::ALTERNATE_KEYS.bits(), 4);
        assert_eq!(KeyboardFlags::ALL_KEYS.bits(), 8);
        assert_eq!(KeyboardFlags::ASSOCIATED_TEXT.bits(), 16);
    }

    #[test]
    fn a_bit_the_terminal_does_not_implement_is_dropped_rather_than_echoed() {
        // `CSI ? u` is a promise about what this terminal will report. Repeating back a
        // bit that nothing here acts on would be a lie the program then relies on, and
        // the protocol is built so that a program can find out: it sets what it wants,
        // queries, and reads the answer.
        let asked = KeyboardFlags::from_bits(0xff);
        assert_eq!(asked.bits(), 0b1_1011);
        assert!(asked.contains(KeyboardFlags::ASSOCIATED_TEXT));
        assert!(!asked.contains(KeyboardFlags::ALTERNATE_KEYS));

        // And the same bit arriving on its own is nothing at all, rather than a set
        // that claims to have it.
        assert_eq!(
            KeyboardFlags::from_bits(KeyboardFlags::ALTERNATE_KEYS.bits()),
            KeyboardFlags::NONE
        );
    }

    #[test]
    fn the_three_apply_modes_do_three_different_things() {
        let mut keyboard = Keyboard::default();
        keyboard.apply(KeyboardFlags::DISAMBIGUATE, Apply::Set);
        assert_eq!(keyboard.flags(), KeyboardFlags::DISAMBIGUATE);

        // Or adds without disturbing.
        keyboard.apply(KeyboardFlags::EVENT_TYPES, Apply::Or);
        assert_eq!(
            keyboard.flags(),
            KeyboardFlags::DISAMBIGUATE | KeyboardFlags::EVENT_TYPES
        );

        // And-not removes without disturbing.
        keyboard.apply(KeyboardFlags::DISAMBIGUATE, Apply::AndNot);
        assert_eq!(keyboard.flags(), KeyboardFlags::EVENT_TYPES);

        // Set clears what is not named, which is the whole difference from Or.
        keyboard.apply(KeyboardFlags::ASSOCIATED_TEXT, Apply::Set);
        assert_eq!(keyboard.flags(), KeyboardFlags::ASSOCIATED_TEXT);
    }

    #[test]
    fn a_mode_that_is_not_one_of_the_three_is_not_guessed_at() {
        assert_eq!(Apply::from_param(1), Some(Apply::Set));
        assert_eq!(Apply::from_param(2), Some(Apply::Or));
        assert_eq!(Apply::from_param(3), Some(Apply::AndNot));
        assert_eq!(Apply::from_param(0), None);
        assert_eq!(Apply::from_param(4), None);
        assert_eq!(Apply::from_param(65535), None);
    }

    #[test]
    fn a_push_saves_what_was_in_force_and_a_pop_puts_it_back() {
        let mut keyboard = Keyboard::default();
        keyboard.apply(KeyboardFlags::EVENT_TYPES, Apply::Set);

        keyboard.push(ALL);
        assert_eq!(keyboard.flags(), ALL);
        assert_eq!(keyboard.depth(), 1);

        keyboard.push(KeyboardFlags::ASSOCIATED_TEXT);
        assert_eq!(keyboard.flags(), KeyboardFlags::ASSOCIATED_TEXT);
        assert_eq!(keyboard.depth(), 2);

        assert_eq!(keyboard.pop(1), ALL, "back to the first push");
        assert_eq!(keyboard.pop(1), KeyboardFlags::EVENT_TYPES);
        assert_eq!(keyboard.depth(), 0);
    }

    #[test]
    fn popping_more_than_was_pushed_resets_rather_than_underflows() {
        // The spec, by name: "If a pop request is received that empties the stack, all
        // flags are reset." The alternative is a terminal stuck in a mode the program
        // that set it has forgotten about.
        let mut keyboard = Keyboard::default();
        keyboard.push(ALL);
        assert_eq!(keyboard.pop(9), KeyboardFlags::NONE);
        assert_eq!(keyboard.depth(), 0);
        assert_eq!(keyboard.pop(1), KeyboardFlags::NONE, "and it stays reset");
    }

    #[test]
    fn a_pop_of_zero_still_pops_one() {
        // The spec gives the parameter a default of 1, and a program that sends an
        // explicit zero means "the default" rather than "nothing" — a pop of nothing
        // is a sequence with no effect, which is not a thing to send.
        let mut keyboard = Keyboard::default();
        keyboard.push(ALL);
        assert_eq!(keyboard.pop(0), KeyboardFlags::NONE);
        assert_eq!(keyboard.depth(), 0);
    }

    #[test]
    fn the_stack_is_bounded_and_evicts_the_oldest() {
        // A program that pushes without popping is a program that would otherwise make
        // the terminal allocate without limit. The spec asks for a limit and this is
        // the one it gets.
        let mut keyboard = Keyboard::default();
        for _ in 0..(STACK_LIMIT * 3) {
            keyboard.push(ALL);
        }
        assert_eq!(
            keyboard.depth(),
            STACK_LIMIT,
            "three times the limit leaves the limit, not three times the limit"
        );

        // Every entry holds `ALL` but the very first, which held `NONE` and is the one
        // the pushes evicted. So every pop restores `ALL` — and the flag that was in
        // force before any of this, which is what the first push saved, is gone.
        for _ in 0..STACK_LIMIT {
            assert_eq!(keyboard.pop(1), ALL);
        }
        assert_eq!(keyboard.flags(), ALL, "there is nothing left to restore");
    }

    #[test]
    fn the_flags_a_push_evicted_are_the_oldest_ones() {
        // The other half of the bound: which end goes. Pushing 1, then 2, ... means the
        // entry saved by the first push holds zero; after overflowing, popping the
        // whole stack must not arrive back at zero, because that entry was evicted.
        let mut keyboard = Keyboard::default();
        keyboard.apply(KeyboardFlags::DISAMBIGUATE, Apply::Set);
        for _ in 0..STACK_LIMIT {
            keyboard.push(ALL);
        }
        keyboard.push(KeyboardFlags::EVENT_TYPES);
        for _ in 0..STACK_LIMIT {
            keyboard.pop(1);
        }
        assert_eq!(
            keyboard.flags(),
            ALL,
            "the entry holding DISAMBIGUATE was the oldest and went first"
        );
    }
}
