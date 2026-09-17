//! Stamps the build with the commit it came from.
//!
//! `ZET_TARGET` is what the updater uses to choose a release asset, so it has to come from
//! the compiler rather than from `std::env::consts`, which does not distinguish an MSVC
//! build from a GNU one. `ZET_GIT_SHA` is what `zet --version` prints, so that a bug report
//! names an exact commit instead of a range of them.

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".into());
    println!("cargo:rustc-env=ZET_TARGET={target}");

    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let root = Path::new(&manifest).join("../..");

    // Re-run only when the commit changes. Without this the script runs on every build, and
    // because it sets `rustc-env` that recompiles this crate — and everything downstream of
    // it — every single time.
    println!(
        "cargo:rerun-if-changed={}",
        root.join(".git/HEAD").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        root.join(".git/refs").display()
    );

    let sha = Command::new("git")
        .args(["-C"])
        .arg(&root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|sha| sha.trim().to_owned())
        // A source tarball has no repository behind it, and a build from one is still a
        // build. It simply cannot name a commit.
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=ZET_GIT_SHA={sha}");
}
