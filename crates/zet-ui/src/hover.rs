//! How lit each control is, and the transition that carried it there.
//!
//! Hover used to be a comparison against the pointer, made inside the painter once per
//! control: a rectangle either contained the point or it did not, and a control flipped
//! its colour on the frame the answer changed. That is a hard cut, and a pointer crossing
//! a row of tabs is a row of hard cuts.
//!
//! So the comparison happens once, in [`crate::Chrome`], and what the painter is given is a
//! *number*: nothing, half-lit, lit. Everything that has a hover today reads that number
//! instead of the rectangle, which is why this is one mechanism rather than ten.
//!
//! # One at a time, and why that is not an assumption
//!
//! [`crate::Chrome::hit`] answers with the first region holding the point, so the pointer is
//! over exactly one control by construction. The list below therefore holds one entry rising
//! and at most one falling, and an entry that has landed at nothing is dropped — a pointer
//! swept across the window does not leave a heap of half-finished fades behind it.
//!
//! # The clock is the chrome's
//!
//! Nothing here reads one. [`Hover::step`] is handed the time the caller already gives
//! [`crate::Chrome::set_time`], which is what makes a hover assertable halfway through
//! itself rather than only at its ends.

use crate::Hit;
use crate::geometry::HOVER;
use crate::strip::progress;

/// One control's lit state, and the transition carrying it there.
///
/// The same shape as the indicator's travel: where it started from, when, and which way it
/// is going. A transition is a function of those three and the clock, so there is no value
/// to step and no drift to accumulate.
struct Lit {
    /// What is lit. [`Hit`] is the crate's own name for a thing the user can point at, so a
    /// control that can be hit is a control that can be hovered, with no second vocabulary.
    hit: Hit,
    /// How lit it was when the transition started, which is where it *is* rather than where
    /// it was heading: a pointer swept across three tabs is three half-finished fades, not
    /// three restarts.
    from: f32,
    /// When it started, on the caller's clock.
    start: f32,
    /// Whether it is on its way up.
    on: bool,
}

/// Every control that is lit or on its way there, and the clock they are measured against.
#[derive(Default)]
pub(crate) struct Hover {
    entries: Vec<Lit>,
    /// The time of the frame being drawn. Set by [`Hover::step`], read by [`Hover::of`],
    /// which is a question about *this* frame and takes no argument of its own.
    now: f32,
}

impl Hover {
    /// How lit `hit` is: zero at rest, one under the pointer, and the curve between them
    /// while a transition is in flight.
    pub(crate) fn of(&self, hit: Hit) -> f32 {
        self.entries
            .iter()
            .find(|entry| entry.hit == hit)
            .map_or(0.0, |entry| lit(entry, self.now))
    }

    /// Step every entry toward `target`, starting and turning around what has to.
    ///
    /// `instant` is reduce-motion: a machine that has asked for less movement has asked for
    /// the answer rather than the animation, so the transition is started already past the
    /// end of its span and every entry lands on its target in this frame.
    pub(crate) fn step(&mut self, now: f32, target: Hit, instant: bool) {
        self.now = now;
        // The whole of the reduce-motion rule, and it is one line because a transition here
        // is a start time rather than a queue of values.
        let start = if instant { now - HOVER } else { now };

        let mut arriving = false;
        for entry in &mut self.entries {
            let towards_target = entry.hit == target;
            arriving |= towards_target;
            if entry.on != towards_target {
                // From where it is, not from where it began.
                entry.from = lit(entry, now);
                entry.start = start;
                entry.on = towards_target;
            }
        }

        if !arriving && target != Hit::None {
            // Nothing was lit for the pointer's new target, so it starts at nothing.
            self.entries.push(Lit {
                hit: target,
                from: 0.0,
                start,
                on: true,
            });
        }

        // A fade that has reached nothing is over. This is what keeps the list from growing
        // with everything the pointer has ever crossed, and it is the only place an entry
        // leaves other than by being turned around.
        self.entries
            .retain(|entry| entry.on || lit(entry, now) > 0.0);
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
            .any(|entry| progress(self.now - entry.start, HOVER) < 1.0)
    }
}

/// How lit an entry is at `now`.
///
/// Exponential ease-out, the curve the indicator's travel and the panel's slide already use.
/// The shape is not new and neither is the arithmetic: only the span is.
fn lit(entry: &Lit, now: f32) -> f32 {
    let progress = progress(now - entry.start, HOVER);
    if entry.on {
        entry.from + (1.0 - entry.from) * progress
    } else {
        entry.from * (1.0 - progress)
    }
}
