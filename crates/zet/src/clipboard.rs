//! The Windows clipboard, in the two directions a terminal uses it.
//!
//! Copy and paste are the only two things zet asks of the system's clipboard, and both
//! of them are `CF_UNICODETEXT` and nothing else. A terminal that copied a rich-text
//! flavour would paste styled text into a shell that cannot hold it, and a terminal
//! that pasted HTML would hand a program markup it did not ask for. The plainest
//! format is the correct one here, and it is also the only one every other program on
//! the machine writes.
//!
//! # Why this is hand-written rather than a crate
//!
//! The obvious answer is `arboard`, which is a good crate that does this properly. It
//! also brings a dependency tree through `objc2` and `x11rb` for platforms zet will not
//! build on, and the amount of Win32 it replaces is about sixty lines. Sixty lines that
//! can be read in one sitting beat a tree that cannot, and zet already links
//! `windows-sys` for the pty layer, so this costs nothing new.
//!
//! # The one thing that is easy to get wrong
//!
//! `OpenClipboard` fails whenever another process holds the clipboard open, which is
//! routine — a clipboard manager, an office suite mid-copy, anything that just called
//! `EmptyClipboard`. It is a lock, not a resource, and the correct response is to wait
//! and try again rather than to report an error. A paste that silently did nothing
//! because something else had the lock for a millisecond is the failure this file
//! exists to avoid.

// Win32 is an unsafe API; see the note in `platform.rs` for why the crate denies rather
// than forbids `unsafe_code` and why this module is allowed to relax it.
#![allow(unsafe_code)]

use std::ffi::c_void;

use windows_sys::Win32::Foundation::{GlobalFree, HWND};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};

/// The plain-text clipboard format. One `u16` per character, null-terminated.
const CF_UNICODETEXT: u32 = 13;

/// How many times to try for the clipboard lock before giving up.
///
/// The lock is held for microseconds by whatever has it, so a handful of attempts
/// spread over a few milliseconds covers every case that is not a hung process. A hung
/// process is a case where waiting longer does not help.
const ATTEMPTS: u32 = 8;

/// The text on the clipboard, or `None` when there is none.
///
/// Returns `None` for an empty clipboard, for a clipboard holding something that is not
/// text, and for a lock that could not be taken. The caller cannot act differently on
/// those three — a paste of nothing and a paste of something unreadable are both a
/// keystroke that does nothing — so they are one answer.
#[must_use]
pub fn get() -> Option<String> {
    let _lock = Lock::take()?;

    // SAFETY: the clipboard is open on this thread, which is the precondition for every
    // call here. `handle` is owned by the clipboard, not by this process, and is not
    // freed or written through: it is locked for reading, the bytes are copied out, and
    // it is unlocked before the lock is dropped. `GlobalSize` reports the block's own
    // size, and the slice is built from that same pointer and no more than that many
    // units, so the walk inside `text_of` cannot leave the allocation.
    unsafe {
        let handle = GetClipboardData(CF_UNICODETEXT);
        if handle.is_null() {
            return None;
        }
        let pointer = GlobalLock(handle);
        if pointer.is_null() {
            return None;
        }

        let units = GlobalSize(handle) / size_of::<u16>();
        let block = std::slice::from_raw_parts(pointer.cast::<u16>(), units);
        let text = text_of(block);

        // The unlock's return value is a lock count for a handle this process does not
        // own the last reference to; failing to decrement it is not something a caller
        // can act on, and the clipboard is unopened on the way out either way.
        let _ = GlobalUnlock(handle);
        text
    }
}

/// The value in a `CF_UNICODETEXT` block, or [`None`] when the block holds none.
///
/// The format is a null-terminated string and the block is an allocation, so the two
/// ends are different places and only the first one is the value's. Walking to the null
/// is only safe because the walk is bounded by the block: a value that never reaches one
/// is malformed, and following it past the end of the allocation reads whatever the block
/// was allocated next to — memory belonging to this process and to no program that asked
/// for it to be pasted, into a terminal, where a paste is fed to a shell.
///
/// A block with no terminator is refused rather than taken whole, for the same reason:
/// what follows the value in a well-formed block is the allocator's slack, and a paste of
/// slack is a paste of somebody else's bytes.
fn text_of(block: &[u16]) -> Option<String> {
    let end = block.iter().position(|&unit| unit == 0)?;
    Some(String::from_utf16_lossy(&block[..end]))
}

/// Put `text` on the clipboard, replacing whatever was there.
///
/// Does nothing when the lock cannot be taken, which is the same outcome as a clipboard
/// manager overwriting the value a moment later. A copy that loses a race is not worth
/// a dialog.
pub fn set(text: &str) {
    let Some(_lock) = Lock::take() else {
        return;
    };

    // A `CF_UNICODETEXT` block is the string as UTF-16 followed by a terminator, and
    // the count is in bytes because that is what the allocator takes.
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = wide.len() * size_of::<u16>();

    // SAFETY: `GMEM_MOVEABLE` is required for a handle handed to `SetClipboardData` — a
    // fixed block would be moved by the heap and leave the clipboard pointing at freed
    // memory. The block is checked for null before anything is written through it. The
    // copy writes exactly `bytes`, which is the allocation's own size, and the source
    // slice is the vector that size was computed from. Ownership of the handle passes to
    // the clipboard on a successful `SetClipboardData`, which is why the failure path is
    // the only one that frees it — freeing it after a success would be a double free.
    unsafe {
        let handle = GlobalAlloc(GMEM_MOVEABLE, bytes);
        if handle.is_null() {
            return;
        }
        let pointer = GlobalLock(handle);
        if pointer.is_null() {
            GlobalFree(handle);
            return;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr().cast::<u8>(), pointer.cast::<u8>(), bytes);
        let _ = GlobalUnlock(handle);

        // `EmptyClipboard` both clears the old contents and takes ownership of the
        // clipboard, which is why it has to come after the lock and before the set.
        EmptyClipboard();
        if SetClipboardData(CF_UNICODETEXT, handle).is_null() {
            GlobalFree(handle);
        }
    }
}

/// The clipboard lock, released on drop.
///
/// A guard rather than a pair of calls because `get` and `set` both have early returns
/// and an early return that skipped `CloseClipboard` would leave the clipboard open on
/// this thread for the rest of the process's life — which on Windows means every other
/// program's copy silently fails until zet exits.
struct Lock;

impl Lock {
    /// Take the clipboard lock, or `None` if it stays busy.
    fn take() -> Option<Self> {
        for _ in 0..ATTEMPTS {
            // SAFETY: a null owner is documented as associating the clipboard with the
            // current task, which is what a terminal with no window of its own wants;
            // the handle would only be used for delayed rendering, which zet does not
            // do. A non-zero result means the clipboard is open on this thread, which
            // is the precondition every other call in this module depends on.
            if unsafe { OpenClipboard(std::ptr::null_mut::<c_void>() as HWND) } != 0 {
                return Some(Lock);
            }
            // A millisecond is far longer than the lock is ever held and far shorter
            // than a user can perceive as lag, and eight of them bound the worst case at
            // something no paste should ever notice.
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        None
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        // SAFETY: the only way to hold a `Lock` is for `OpenClipboard` to have
        // succeeded, and `Lock` is neither `Clone` nor `Copy`, so this closes exactly
        // once per open and on the thread that opened it.
        unsafe {
            CloseClipboard();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_stops_where_the_terminator_is_and_not_where_the_block_ends() {
        // The block Windows hands over is an allocation rather than a string: it is at
        // least as large as the value and is not required to be any smaller, so what
        // follows the terminator belongs to the allocator and not to the program that
        // put the value there.
        assert_eq!(text_of(&[0x48, 0x69, 0, 0x21, 0x21]), Some("Hi".to_owned()));
        assert_eq!(text_of(&[0x48, 0x69, 0]), Some("Hi".to_owned()));
        // A terminator in the first unit is the empty string, which is a value.
        assert_eq!(text_of(&[0]), Some(String::new()));
    }

    #[test]
    fn a_block_that_never_reaches_a_terminator_has_no_text_in_it() {
        // There is no way to know where such a value stops, and the bytes past its end
        // are the allocator's slack rather than anyone's characters. Reading to a null
        // that is not there reads whatever the block was allocated next to, and a paste
        // is fed to a shell: the wrong answer here is a page of somebody's memory typed
        // at a prompt.
        assert_eq!(text_of(&[0x48, 0x69]), None);
        assert_eq!(text_of(&[]), None);
    }

    #[test]
    fn a_lone_surrogate_in_the_block_does_not_take_the_rest_of_it_with_it() {
        // Half a pair is not a reason to refuse the paste: the value is still a value,
        // and one replacement character is a smaller lie than a clipboard that will not
        // paste.
        assert_eq!(
            text_of(&[0xd800, 0x48, 0x69, 0]).as_deref(),
            Some("\u{fffd}Hi")
        );
    }

    /// The one test of the real clipboard.
    ///
    /// There is exactly one test here, and the reason is the resource rather than the
    /// code. The clipboard is machine-wide and thread-owned, so two of these running at
    /// once — which is what `cargo test` does by default — interleave an `EmptyClipboard`
    /// from one thread with a `get` from another, and the failure that produces looks
    /// exactly like a broken implementation. It was written as three tests first and
    /// failed as three tests, for precisely that reason.
    ///
    /// What is covered is everything a mistake would make invisible: the byte count, the
    /// terminator, the swap of ownership that makes the handle the clipboard's rather
    /// than this process's, and the lock guard releasing on the way out of a function
    /// with three early returns.
    #[test]
    fn the_clipboard_round_trips_text() {
        let original = get();

        set("zet round trip \u{1f600}");
        assert_eq!(get().as_deref(), Some("zet round trip \u{1f600}"));

        // An empty string is a value, not an absence, and a paste of it should insert
        // nothing rather than paste whatever was there before.
        set("");
        assert_eq!(get().as_deref(), Some(""));

        // Two takes in a row, which would fail if the first had not been released by its
        // guard — the failure mode of forgetting `CloseClipboard` on an early return.
        {
            let first = Lock::take().expect("the clipboard is not permanently held");
            drop(first);
        }
        let second = Lock::take().expect("the first guard released the lock");
        drop(second);

        match original {
            Some(text) => set(&text),
            None => set(""),
        }
    }
}
