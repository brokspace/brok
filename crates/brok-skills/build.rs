//! Locates the built-in skills for `include_dir!`.
//!
//! In the repository they live at the top level (`brok/skills/`), next to
//! the docs. A crates.io package cannot reach outside its own directory, so
//! `scripts/vendor-skills.sh` copies them into `crates/brok-skills/skills/`
//! before publishing; that copy wins when present.

use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let vendored = manifest_dir.join("skills");
    let repository = manifest_dir.join("..").join("..").join("skills");
    let dir = if vendored.is_dir() { vendored } else { repository };
    assert!(dir.is_dir(), "built-in skills not found at {}", dir.display());

    let dir = dir.canonicalize().expect("canonical skills path");
    println!("cargo:rustc-env=BROK_BUILTIN_SKILLS={}", dir.display());
    // `include_dir!` cannot track files on stable Rust: Cargo rescans the
    // whole directory and reruns this script when anything changes.
    println!("cargo:rerun-if-changed={}", dir.display());
}
