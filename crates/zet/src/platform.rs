//! What Windows says about the user, asked rather than guessed.
//!
//! PRODUCT.md's eighth success criterion is that high contrast, 200% text scaling, and
//! a machine with no usable GPU all produce a readable terminal. Only the first two are
//! this file's business, and they share a shape: the operating system has already asked
//! the user a question, and the answer is sitting in a setting that a terminal which
//! ignores it will look broken to exactly the people who needed to be asked.
//!
//! # Why these are read and not listened to
//!
//! Windows sends `WM_SETTINGCHANGE` when the accessibility settings move, and the host
//! could catch it and re-read. It does not, and the reason is that the event is
//! delivered to the top-level window while the settings that matter are per-user and
//! can change from a session that is not this one. Re-reading on every frame would be
//! four syscalls sixty times a second; re-reading on a timer is a latency nobody can
//! see. So the host re-reads when it has a reason to — a focus change, a resize, a
//! theme reload — and this module is a plain function so that the decision stays there.

// Win32 is an unsafe API and this module is the reason the crate lints `unsafe_code` as
// `deny` rather than `forbid`. Every block below carries a SAFETY comment naming the
// contract it satisfies.
#![allow(unsafe_code)]
// The text scale is a percentage out of the registry, so a `u32` becomes a small `f32`
// and nothing else here casts at all.
#![allow(clippy::cast_precision_loss)]

use windows_sys::Win32::Graphics::Gdi::{COLOR_HIGHLIGHT, GetSysColor};
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MessageBoxW, SPI_GETCLIENTAREAANIMATION,
    SPI_GETHIGHCONTRAST, SystemParametersInfoW,
};
use zet_config::Rgb;

/// Tell the user something they cannot be told on stderr.
///
/// A GUI program launched from the Start menu has no console attached and its stderr
/// goes nowhere, so a terminal that fails to start would simply not appear — which is
/// the least useful thing a program can do, because it leaves the user with nothing to
/// search for and nothing to report. A message box is the only channel left.
///
/// Blocking, deliberately. It is called once, on the way out, when there is no frame
/// left to draw and nothing else the program was going to do.
pub fn alert(title: &str, message: &str) {
    let title = wide(title);
    let message = wide(message);
    // SAFETY: both strings are null-terminated UTF-16 buffers that outlive the call, and
    // a null parent handle is documented as meaning "no owner window", which is the only
    // thing available when the window is the thing that failed to be created. The return
    // value is which button was pressed and there is only one.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
        );
    }
}

/// The accessibility settings, as of the moment they were read.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SystemSettings {
    /// Whether a high-contrast theme is on.
    ///
    /// The flag Windows sets when the user turns the mode on, which is not the same as
    /// "the current theme has good contrast" — a user running a dark theme zet likes is
    /// not asking for anything, and a user running High Contrast Black is.
    pub high_contrast: bool,
    /// The system's highlight colour, which is the accent a forced-colours desktop has
    /// already been told to use.
    ///
    /// [`None`] when high contrast is off. It is the one colour zet does not get to
    /// choose in that mode, and passing it through rather than substituting zet's own
    /// amber is the whole reason it is read.
    pub highlight: Option<Rgb>,
    /// How much the user has asked for text to be made bigger, as a multiplier.
    ///
    /// 1.0 for the default. Windows offers 100% to 225% in the Settings app, and this is
    /// separate from DPI: a 4K laptop at 200% scaling with text at 100% is a different
    /// request from the same laptop at 100% scaling with text at 200%, and a terminal
    /// that conflated them would be wrong about one of them.
    pub text_scale: f32,
    /// Whether the user has asked for as little animation as possible.
    ///
    /// The setting behind "Show animations in Windows", which is what a user with a
    /// vestibular disorder turns off. It stops the blinking cursor and the tab
    /// indicator's travel — the two things zet moves on its own — and nothing else,
    /// because nothing else in zet moves.
    pub reduce_motion: bool,
}

impl Default for SystemSettings {
    /// Everything off, which is what a machine that answers nothing should look like.
    ///
    /// Not the same as [`SystemSettings::read`] failing: a failure to read returns this
    /// because a terminal drawn for a user who asked for nothing is a working terminal,
    /// and a terminal that refused to start because a registry key was missing would be
    /// a terminal nobody could use to find out why.
    fn default() -> Self {
        Self {
            high_contrast: false,
            highlight: None,
            text_scale: 1.0,
            reduce_motion: false,
        }
    }
}

impl SystemSettings {
    /// Read all four settings from the system.
    ///
    /// Cheap enough to call whenever the host has a reason to, which is a handful of
    /// times a session: four calls into user32 and advapi32, none of which allocate.
    #[must_use]
    pub fn read() -> Self {
        let high_contrast = high_contrast();
        Self {
            high_contrast,
            highlight: high_contrast.then(highlight),
            text_scale: text_scale(),
            reduce_motion: reduce_motion(),
        }
    }
}

/// Whether a high-contrast theme is on.
fn high_contrast() -> bool {
    // SAFETY: `HIGHCONTRASTW` is a plain data struct and the only field that needs
    // initialising before the call is `cbSize`, which the API requires to be the
    // struct's own size so that it knows which version it was handed. `pvParam` points
    // at a live local for the duration of the call and nothing else holds a pointer to
    // it. `lpszDefaultScheme` is left null, which the API documents as "do not report
    // the scheme name" rather than as an error.
    unsafe {
        let mut info = HIGHCONTRASTW {
            cbSize: u32::try_from(std::mem::size_of::<HIGHCONTRASTW>()).unwrap_or(0),
            ..HIGHCONTRASTW::default()
        };
        let ok = SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            info.cbSize,
            std::ptr::from_mut(&mut info).cast(),
            0,
        );
        ok != 0 && info.dwFlags & HCF_HIGHCONTRASTON != 0
    }
}

/// The system's highlight colour.
///
/// `GetSysColor` returns a `COLORREF`, which is `0x00BBGGRR` — blue in the high byte and
/// red in the low one, the opposite of every other byte order in the API. The swap is
/// the whole reason this is a function rather than a call site.
fn highlight() -> Rgb {
    // SAFETY: `GetSysColor` takes an index and returns a value. It reads no memory this
    // process owns and cannot fail; an index outside the table returns zero.
    let color = unsafe { GetSysColor(COLOR_HIGHLIGHT) };
    Rgb::new(
        (color & 0xff) as u8,
        ((color >> 8) & 0xff) as u8,
        ((color >> 16) & 0xff) as u8,
    )
}

/// How much bigger the user has asked for text, as a multiplier.
///
/// Read from the registry rather than from an API because there is no API: the text
/// scaling introduced in Windows 10 is a plain per-user value and Microsoft never
/// documented a way to ask for it. A missing or malformed value means the default,
/// which is what an unset key means.
fn text_scale() -> f32 {
    /// The key the Settings app writes, and the only place the value exists.
    const KEY: &str = r"Software\Microsoft\Accessibility";
    /// The value's name.
    const VALUE: &str = "TextScaleFactor";

    let key = wide(KEY);
    let value = wide(VALUE);
    let mut factor: u32 = 0;
    let mut size = u32::try_from(std::mem::size_of::<u32>()).unwrap_or(0);

    // SAFETY: both strings are null-terminated and alive for the call. `pvData` points
    // at a live `u32` and `pcbData` at its size, which is what `RRF_RT_REG_DWORD` says
    // to expect; the API writes at most that many bytes. No output parameter is read
    // unless the return value says the call succeeded.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            std::ptr::from_mut(&mut factor).cast(),
            &raw mut size,
        )
    };

    if status != 0 {
        return 1.0;
    }
    // Windows stores a percentage: 100 is the default, 225 the maximum. Anything below
    // 100 is a value nothing writes, and treating it as a scale would shrink the chrome
    // for a user who asked for nothing.
    if factor < 100 {
        return 1.0;
    }
    factor as f32 / 100.0
}

/// Whether the user has asked for as little animation as possible.
fn reduce_motion() -> bool {
    let mut animations: i32 = 1;
    // SAFETY: `SPI_GETCLIENTAREAANIMATION` reads a single `BOOL` into `pvParam`, which
    // points at a live local. The `uiParam` argument is ignored by this action and zero
    // is what the API documents for it; `fWinIni` is ignored on a get.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            std::ptr::from_mut(&mut animations).cast(),
            0,
        )
    };
    ok != 0 && animations == 0
}

/// A Rust string as a null-terminated UTF-16 buffer, for the `W` entry points.
///
/// Windows paths and registry keys are `PCWSTR` and there is no way around owning the
/// buffer while the call is in flight, so the caller keeps it alive.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    // The comparisons here are exact on purpose: every one of them is an assertion that
    // an arithmetic result equals a literal, not a tolerance test dressed up as one.
    #![allow(clippy::float_cmp)]

    use super::*;

    #[test]
    fn the_default_is_a_machine_that_asked_for_nothing() {
        let settings = SystemSettings::default();
        assert!(!settings.high_contrast);
        assert_eq!(settings.highlight, None);
        assert_eq!(settings.text_scale, 1.0);
        assert!(!settings.reduce_motion);
    }

    #[test]
    fn reading_the_system_answers_without_panicking() {
        // Not an assertion about this machine's settings — a test that asserted those
        // would fail on every machine whose owner has an opinion. What it checks is that
        // the four calls return at all and that what comes back is inside the range the
        // settings can hold, which is the part a wrong `cbSize` or a mis-sized registry
        // buffer would break.
        let settings = SystemSettings::read();
        assert!((1.0..=2.25).contains(&settings.text_scale));
        assert_eq!(settings.high_contrast, settings.highlight.is_some());
    }

    #[test]
    fn a_wide_string_ends_in_a_null_and_carries_no_others() {
        let buffer = wide("Hi");
        assert_eq!(buffer, vec![0x48, 0x69, 0]);
        assert_eq!(wide(""), vec![0]);
    }
}
