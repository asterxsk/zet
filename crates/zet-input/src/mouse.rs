//! A mouse event, as the host hands it over.

use crate::chord::Modifiers;

/// A mouse button, including the four scroll directions.
///
/// The wheel directions are buttons because that is what they are on the wire: the
/// legacy protocols have no "scroll" concept and report a wheel tick as a press of
/// button 4 or 5, which is why every encoder here has to special-case them rather
/// than treating them as three buttons and a motion.
///
/// [`MouseButton::None`] is not "no button was involved" so much as "no button is
/// held", and it is what a motion event carries when the mouse moves with everything
/// up. It is a real value rather than an `Option` because the button code it maps to
/// on the wire is a real code — 3, the one the legacy protocols also use for
/// "released" — and an `Option` would mean converting it back to that code in every
/// arm.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MouseButton {
    /// Left.
    Left,
    /// Middle.
    Middle,
    /// Right.
    Right,
    /// A tick of the wheel away from the user.
    WheelUp,
    /// A tick of the wheel toward the user.
    WheelDown,
    /// A tick of a horizontal wheel to the left.
    WheelLeft,
    /// A tick of a horizontal wheel to the right.
    WheelRight,
    /// The back button, reported as button 8.
    Back,
    /// The forward button, reported as button 9.
    Forward,
    /// No button. Carried by motion, and by a release whose button the host did not
    /// identify.
    None,
}

/// What happened to the mouse.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MouseAction {
    /// A button went down.
    Press,
    /// A button came up.
    Release,
    /// The pointer moved.
    Motion,
}

/// A mouse event, in cell coordinates.
///
/// `col` and `row` are zero-based cells counted from the top-left of the grid, which
/// is the host's coordinate system and not the one on the wire: every encoding here
/// is one-based, and each of them adds its own offset, so keeping the crate's own
/// type zero-based means the conversion happens once, visibly, inside the encoder.
/// The scrollback offset does not belong here — a program is told where the pointer
/// is on its screen, and where that screen sits in the user's scrollback is zet's
/// business.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MouseEvent {
    /// The button involved, or [`MouseButton::None`] for a motion with nothing held.
    pub button: MouseButton,
    /// What happened.
    pub action: MouseAction,
    /// The column, zero-based.
    pub col: u16,
    /// The row, zero-based.
    pub row: u16,
    /// The modifiers held.
    pub mods: Modifiers,
}
