//! The two things about `zet.exe` that its own Rust code cannot see.
//!
//! The PE subsystem is not a value a running program can read, and it is the whole reason
//! the terminal used to open a second, empty console window behind itself: a
//! console-subsystem binary is handed a console by Windows, and on Windows 11 that console
//! is hosted by Windows Terminal. Nothing inside the process can assert the fix, so this
//! test reads the header the linker wrote. The command line is the other half: the move to a
//! GUI subsystem takes the standard handles away, and this test proves the output still
//! reaches a redirected one.

use std::path::Path;

/// The subsystem Windows reads out of the built binary.
///
/// `2` is `IMAGE_SUBSYSTEM_WINDOWS_GUI` and `3` is `IMAGE_SUBSYSTEM_WINDOWS_CUI`, the
/// console one. The offsets are fixed by the PE format: the DOS header's `e_lfanew` field
/// at `0x3c` points at the `PE\0\0` signature, the optional header follows the 4-byte
/// signature and the 20-byte COFF header, and `Subsystem` is a little-endian `u16` at
/// offset 68 of the optional header in both the PE32 and PE32+ spellings, because every
/// field before it is the same size in each.
fn subsystem_of(path: &Path) -> u16 {
    let bytes = std::fs::read(path).expect("the built binary is there");
    let pe = usize::try_from(u32::from_le_bytes(
        bytes[0x3c..0x40].try_into().expect("four bytes"),
    ))
    .expect("an offset fits in a usize");
    assert_eq!(
        &bytes[pe..pe + 4],
        b"PE\0\0",
        "no PE signature in {}",
        path.display()
    );
    let at = pe + 4 + 20 + 68;
    u16::from_le_bytes(bytes[at..at + 2].try_into().expect("two bytes"))
}

#[test]
fn the_binary_is_a_gui_subsystem_app_so_windows_allocates_no_console() {
    // A test that reads 3 has found the background console window back.
    assert_eq!(
        subsystem_of(Path::new(env!("CARGO_BIN_EXE_zet"))),
        2,
        "a console subsystem is what puts a second window behind the terminal"
    );
}

#[test]
fn a_gui_subsystem_binary_still_prints_when_its_output_is_redirected() {
    // A pipe is the launch path `attach_console` must leave alone: the handles are already
    // open, so redirection is the caller's decision and reopening them would undo it. It is
    // also the one path a test can observe without a console of its own.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_zet"))
        .arg("--version")
        .output()
        .expect("zet runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "the version is not in {stdout:?}"
    );
    assert!(stdout.starts_with("zet "), "{stdout:?}");
}
