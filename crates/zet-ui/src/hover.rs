//! How lit each control is, and the transition that carried it there.
//!
//! Hover used to be a comparison against the pointer, made inside the painter once per
//! control: a rectangle either contained the point or it did not, and a control flipped
//! its colour on the frame the answer changed. That is a hard cut, and a pointer crossing
//! a row of tabs is a row of hard cuts.
//!
//! So the comparison happens once, in [`crate::Chrome`], and what the painter is given is a
//! *number*: nothing, half-lit, lit. Everything that has a hover today reads that number
//! instead of the rectangle, which is why this is one mechanism rather than ten. The same
//! number carries the arrival of a tab and of a surface, where it is the coverage the thing
//! is drawn at rather than how lit it is — one piece of arithmetic with three keys, because
//! the shape of "where it started, when, and which way" is the same for all three.
//!
//! # A set, and why it is usually one
//!
//! [`crate::Chrome::hit`] answers with the first region holding the point, so the pointer is
//! over exactly one control by construction — and the list below therefore holds one entry
//! rising and at most one falling. There is one exception, and it is the × on a tab: the mark
//! sits inside its cell, so a pointer on the mark names *both* the mark and the cell it is in,
//! and the set is sometimes two. A cell and its mark are one place seen at two distances rather
//! than two controls, which is what makes keeping them both lit the right answer. An entry that
//! has landed at nothing is dropped — a pointer swept across the window does not leave a heap
//! of half-finished fades behind it.
//!
//! # The clock is the chrome's
//!
//! Nothing here reads one. [`Fades::step`] is handed the time the caller already gives
//! [`crate::Chrome::set_time`], which is what makes a hover assertable halfway through
//! itself rather than only at its ends.

use crate::Hit;
use crate::geometry::HOVER;
use crate::strip::progress;

/// One key's lit state, and the transition carrying it there.
///
/// The same shape as the indicator's travel: where it started from, when, and which way it
/// is going. A transition is a function of those three and the clock, so there is no value
/// to step and no drift to accumulate.
struct Lit<K> {
    /// What is lit. A control the user can point at, or — for an arrival — the thing whose
    /// arrival is being measured. Both are a name for a key, so a control that can be hit and a
    /// tab that can arrive are filed the same way.
    key: K,
    /// How lit it was when the transition started, which is where it *is* rather than where
    /// it was heading: a pointer swept across three tabs is three half-finished fades, not
    /// three restarts.
    from: f32,
    /// When it started, on the caller's clock.
    start: f32,
    /// Whether it is on its way up.
    on: bool,
}

/// Every key that is lit or on its way there, and the clock and span they are measured against.
///
/// Generic over its key, so one transition carries the pointer's hover, a tab's arrival, and a
/// surface's — the three things in the chrome that fade. There is one [`Fades::step`] and no
/// single-key wrapper beside it, because a wrapper for the same type would be a second
/// definition of the same transition.
pub(crate) struct Fades<K> {
    entries: Vec<Lit<K>>,
    /// The time of the frame being drawn. Set by [`Fades::step`], read by [`Fades::of`],
    /// which is a question about *this* frame and takes no argument of its own.
    now: f32,
    /// How long a transition takes, in seconds. A field rather than a constant because the
    /// three keys do not share a span: a hover is shorter than the arrival of a surface.
    span: f32,
}

/// The pointer's hover: which controls are lit, and the transition carrying each there.
///
/// The alias is not decoration: it is the crate's name for the one `Fades` whose span is the
/// hover's, and it keeps the painters asking for a [`Hover`] rather than for a `Fades<Hit>`.
pub(crate) type Hover = Fades<Hit>;

impl<K> Fades<K> {
    /// A set of transitions that take `span` seconds each.
    pub(crate) const fn new(span: f32) -> Self {
        Self {
            entries: Vec::new(),
            now: 0.0,
            span,
        }
    }
}

/// A `Fades` over the hover's span, which is what [`Hover`] and the crate's tests want.
///
/// Written by hand rather than derived: `#[derive(Default)]` would demand `K: Default` for
/// every key, and neither [`Hit`] nor a tab's id has a default — there is no such thing as the
/// control you are hovering when you are hovering nothing.
impl<K> Default for Fades<K> {
    fn default() -> Self {
        Self::new(HOVER)
    }
}

impl<K: Copy + PartialEq> Fades<K> {
    /// How lit `key` is: zero at rest, one present, and the curve between them while a
    /// transition is in flight.
    pub(crate) fn of(&self, key: K) -> f32 {
        self.entries
            .iter()
            .find(|entry| entry.key == key)
            .map_or(0.0, |entry| lit(entry, self.now, self.span))
    }

    /// Step every entry toward `present`, starting and turning around what has to.
    ///
    /// `present` is everything the pointer is on this frame — or, for an arrival, everything
    /// that is on screen. A key that is not in it fades out, which is the whole lifetime rule:
    /// an entry that has reached nothing is dropped.
    ///
    /// `instant` is reduce-motion: a machine that has asked for less movement has asked for
    /// the answer rather than the animation, so the transition is started already past the
    /// end of its span and every entry lands on its target in this frame.
    pub(crate) fn step(&mut self, now: f32, present: &[K], instant: bool) {
        self.now = now;
        // The whole of the reduce-motion rule, and it is one line because a transition here
        // is a start time rather than a queue of values.
        let start = if instant { now - self.span } else { now };

        for entry in &mut self.entries {
            let towards = present.contains(&entry.key);
            if entry.on != towards {
                // From where it is, not from where it began.
                entry.from = lit(entry, now, self.span);
                entry.start = start;
                entry.on = towards;
            }
        }

        // Every key that has just been reached or has just appeared starts at nothing, because
        // nothing was lit for it: a fade only makes sense from where the thing already was.
        for key in present {
            if !self.entries.iter().any(|entry| entry.key == *key) {
                self.entries.push(Lit {
                    key: *key,
                    from: 0.0,
                    start,
                    on: true,
                });
            }
        }

        // A fade that has reached nothing is over. This is what keeps the list from growing
        // with everything the pointer has ever crossed, and it is the only place an entry
        // leaves other than by being turned around.
        self.entries
            .retain(|entry| entry.on || lit(entry, now, self.span) > 0.0);
    }

    /// Whether anything is still moving.
    ///
    /// The host's frame clock is this and nothing else: while it is true the loop wakes for
    /// another frame, and once every transition has landed it stops. A transition that never
    /// ends would spin the loop, so what makes this terminate is worth stating — every entry
    /// was started at a finite time and [`lit`] is constant once a span has passed.
    pub(crate) fn moving(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| progress(self.now - entry.start, self.span) < 1.0)
    }
}

/// How lit an entry is at `now`, measured against `span`.
///
/// Exponential ease-out, the curve the indicator's travel and the panel's slide already use.
/// The shape is not new and neither is the arithmetic: only the span is.
fn lit<K>(entry: &Lit<K>, now: f32, span: f32) -> f32 {
    let progress = progress(now - entry.start, span);
    if entry.on {
        entry.from + (1.0 - entry.from) * progress
    } else {
        entry.from * (1.0 - progress)
    }
}
