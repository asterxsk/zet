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

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{COLOR_HIGHLIGHT, GetSysColor};
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, LWA_ALPHA, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MessageBoxW,
    SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST, SW_SHOWNORMAL, SetLayeredWindowAttributes,
    SetWindowLongPtrW, SystemParametersInfoW, WS_EX_LAYERED,
};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;
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

/// How opaque the window is, as the desktop understands it.
///
/// `window.opacity` is a promise the configuration has been making since it was written
/// and that nothing kept until now: a window that fades into whatever is behind it.
/// Windows spells that as a layered window — `WS_EX_LAYERED` plus one constant alpha —
/// and it is the whole of the feature, because the desktop composites every pixel of the
/// window against the desktop at that alpha. Text fades with the ground it is drawn on,
/// rather than the ground fading behind text that stays opaque, which is the only reading
/// under which a terminal being translucent means anything.
///
/// The whole window and not the grid: the frame is one surface, and a chrome that stayed
/// solid while the grid faded would be a titlebar floating over the desktop.
///
/// Called once when the window is made and again whenever the configuration's window
/// section changes. Both go through this one function, so a window that has been faded
/// cannot differ from one that was born faded — there is no second path to drift from.
#[allow(clippy::cast_possible_wrap)]
pub fn set_opacity(window: &Window, opacity: f32) {
    let Some(hwnd) = window_handle(window) else {
        return;
    };
    // `WS_EX_LAYERED` is a `u32` in the API and the extended style is pointer-sized here.
    // The lint cannot see that the value is `0x0008_0000`, which is a positive `i32` and
    // so is exact at every width; a named constant is worth more than a cast it cannot
    // read, and the alternative is spelling the bit out.
    let layered = WS_EX_LAYERED as isize;
    // SAFETY: `hwnd` is this process's own window, taken from the handle the window
    // handed out and alive for as long as it is. `GWL_EXSTYLE` asks for one pointer-sized
    // value, which is what the API returns; it reads the window's own state and cannot
    // fail in a way that needs reporting, because a window that has gone away is a window
    // there is nothing left to fade.
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    // The window is left alone when it is already wearing what was asked for, which is
    // the default window's whole story: it is made without the layered bit, so a user who
    // never touches the setting never pays a write for it.
    let wanted = match layered_alpha(opacity) {
        Some(_) => style | layered,
        None => style & !layered,
    };
    if wanted == style {
        return;
    }
    // SAFETY: both calls take the same live window. `SetWindowLongPtrW` writes one
    // pointer-sized value — the extended style with the layered bit set or cleared — and
    // returns the previous one, which is not needed here. `SetLayeredWindowAttributes`
    // takes a key colour, an alpha byte, and a flag: the key is unused under `LWA_ALPHA`
    // and zero is what the API documents for it. Neither call is a `SetWindowPos`, and
    // none is needed: making a window layered after creation is the documented two-step
    // of setting the style and then the attributes.
    unsafe {
        // Both calls answer with the previous value, which nothing here wants: what the
        // window is wearing is read back through `GetWindowLongPtrW` when it changes
        // again, and a return value that was stored instead would be a second copy of the
        // window's own state.
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, wanted);
        // A window that is at full opacity loses the style as well as the alpha, and one
        // that is not gets its alpha set: a layered window is composited by the desktop
        // instead of being handed to the display controller, and paying that for a window
        // that is not translucent buys nothing at all.
        if let Some(alpha) = layered_alpha(opacity) {
            let _ = SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);
        }
    }
}

/// Open a URL in whatever the system opens URLs with.
///
/// `ShellExecuteW` rather than `cmd /c start`, and that is not a preference. The URL was
/// chosen by whatever program printed the OSC 8 sequence, and handing that string to a
/// shell means a URL containing `&` or a quote becomes a command line — the exact thing
/// `docs/security.html` puts out of bounds ("a paste that escapes a bracketed-paste
/// guard, a title string that reaches a shell"). This call takes the URL as one argument
/// and parses no metacharacters at all.
///
/// The verb is `open`, so the association the user has set up is the one that runs: a
/// browser for a page, a mail client for `mailto:`. Nothing here decides that, and
/// nothing here is handed a path — `zet-app` has already refused every scheme that is not
/// one of the three that mean a page, so what arrives is a URL rather than a program.
///
/// Failure is silent. The user asked for a page and the system would not produce one, and
/// a message box about a browser is a message box between the user and their terminal.
pub fn open_url(url: &str) {
    let operation = wide("open");
    let target = wide(url);
    // SAFETY: both strings are null-terminated UTF-16 buffers that outlive the call,
    // which is the whole of what this API asks of them. A null window handle is
    // documented as meaning "no owner", which is what a terminal that is not asking the
    // user anything has; the remaining parameters are null, which is "no working
    // directory and no arguments", and the show command. The return value is an
    // `HINSTANCE` that the API overloads as a legacy error code, so it is a `<= 32`
    // failure indicator rather than an allocation, and there is nothing to free.
    let _ = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
}

/// The byte the alpha attribute takes, or `None` for a window that should not be layered.
///
/// 1.0 is not "255, but the fast path" — it is "this is an ordinary opaque window", and
/// that is a different window as far as the desktop is concerned. A terminal spends its
/// life at the default, so the default is the one that has to be free.
///
/// Rounded rather than truncated, because 0.5 is exactly halfway between opaque and clear
/// and a byte that landed a half-step below every value the user typed would be a setting
/// that quietly reads dark.
// The clamp puts the product in `0.0..=255.0` and the round leaves a whole number, so
// the cast is exact and cannot lose a sign it does not have. Both lints are right in
// general and cannot see the two lines above them.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn layered_alpha(opacity: f32) -> Option<u8> {
    if opacity >= 1.0 {
        return None;
    }
    Some((opacity.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// The window's `HWND`, if this platform has one and the window is still alive.
///
/// `None` is not an error to report: a window that cannot hand out a handle is a window
/// that has already gone, and there is nothing a caller could do about it. The raw handle
/// is a `NonZeroIsize` because that is what the API guarantees about it — a null `HWND` is
/// not a window — and Windows itself takes it as a pointer.
fn window_handle(window: &Window) -> Option<HWND> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return None;
    };
    Some(win32.hwnd.get() as HWND)
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
    fn a_window_at_full_opacity_is_not_layered_at_all() {
        // The default, and the one that has to cost nothing: a user who never opens the
        // setting gets the same window they would have got before the setting existed.
        assert_eq!(layered_alpha(1.0), None);
        // And a value the validator would have rejected still does not layer a window
        // that is more than opaque.
        assert_eq!(layered_alpha(1.5), None);
    }

    #[test]
    fn every_stop_from_clear_to_opaque_has_a_byte() {
        assert_eq!(layered_alpha(0.0), Some(0));
        assert_eq!(layered_alpha(0.25), Some(64));
        assert_eq!(layered_alpha(0.5), Some(128));
        assert_eq!(layered_alpha(0.75), Some(191));
        assert_eq!(layered_alpha(0.999), Some(255));
        // 0.9 × 255 is 229.5, the one value here where the rule is visible: truncating
        // would read 229, a step darker than the user asked for.
        assert_eq!(layered_alpha(0.9), Some(230));
    }

    #[test]
    fn a_wide_string_ends_in_a_null_and_carries_no_others() {
        let buffer = wide("Hi");
        assert_eq!(buffer, vec![0x48, 0x69, 0]);
        assert_eq!(wide(""), vec![0]);
    }
}
