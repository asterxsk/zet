//! Puts the application icon and the version block into the executable.
//!
//! Neither can be done at runtime. An icon that Explorer, the taskbar, Alt-Tab and the
//! Start Menu shortcut draw is a section of the PE file — `RT_GROUP_ICON` and the
//! `RT_ICON` images it points at — and it has to be in the file before the linker is
//! finished. Until this script existed, `zet.exe` had no resource section at all, so
//! every one of those places drew the generic Windows application icon while
//! `packaging/zet.ico` was used for the installer alone.
//!
//! The version block is the other half of the same section: it is what the Properties
//! dialog's Details tab reads, and what a crash reporter or a support thread would read
//! to find out which build someone is running. It is filled from the crate's own version
//! rather than from a number written out twice.
//!
//! The script is Windows-only by construction: another platform's linker would not know
//! what to do with the resource object this produces, so it does nothing there and the
//! build carries on.

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../packaging/zet.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let icon = Path::new("../../packaging/zet.ico");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let description = std::env::var("CARGO_PKG_DESCRIPTION").unwrap_or_default();
    let repository = std::env::var("CARGO_PKG_REPOSITORY").unwrap_or_default();

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(icon.to_str().expect("a UTF-8 path"));
    resource.set("ProductName", "zet");
    // What the taskbar tooltip and the Explorer details column show, and the one field
    // that has to be a sentence rather than a name: "zet" alone says nothing about what
    // the file is.
    resource.set("FileDescription", &description);
    resource.set("ProductVersion", &version);
    resource.set("FileVersion", &version);
    resource.set("CompanyName", "asterxsk");
    // Both licences, because zet is dual-licensed and picking one here would be a claim
    // about which one a given user took it under.
    resource.set("LegalCopyright", "MIT or Apache-2.0");
    // `Comments` is where a URL goes in a version block; there is no repository field.
    resource.set("Comments", &repository);

    // A failure here is worth stopping for. The alternative is a build that succeeds and
    // ships an application with no icon in it, which is exactly the state this script was
    // written to end — and it is not a state anything downstream would notice, because
    // the icon is the one part of a binary nothing reads until it is on screen.
    if let Err(error) = resource.compile() {
        panic!(
            "could not write the resource section, so the icon and the version block are \
             missing from zet.exe: {error}. The Windows SDK's rc.exe is what compiles it, \
             and it ships with the same build tools that link this crate."
        );
    }
}
