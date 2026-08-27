//! Embed the build's version into the binary so a running IME can report
//! exactly which build it is (`karukan_im::version()`).
//!
//! Preferred form is `git describe --tags`: a build at a release tag reports
//! the tag itself (`v0.1.0-rakukan.2`), a build between releases reports the
//! distance from the last one (`v0.1.0-rakukan.2-3-g1234567`), and a dirty
//! working tree appends `-dirty`. When no tag is reachable (shallow clone,
//! upstream checkout) the previous `<pkg version>+<short hash>` form is the
//! fallback, and `<pkg version>+unknown` when git itself is unavailable
//! (e.g. building from a source tarball).

use std::process::Command;

fn main() {
    let version = std::env::var("KARUKAN_BUILD_VERSION")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(describe_tags)
        .or_else(hash_fallback)
        .unwrap_or_else(|| format!("{}+unknown", pkg_version()));
    println!("cargo:rustc-env=KARUKAN_VERSION={version}");
    println!("cargo:rerun-if-env-changed=KARUKAN_BUILD_VERSION");

    // Re-run when the checked-out commit or the tags change. HEAD covers
    // branch switches; the ref file covers new commits on the current
    // branch; refs/tags and packed-refs cover new or re-fetched tags.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    if let Some(head_ref) = read_head_ref() {
        println!("cargo:rerun-if-changed=../../.git/{head_ref}");
    }
    println!("cargo:rerun-if-changed=../../.git/refs/tags");
    println!("cargo:rerun-if-changed=../../.git/packed-refs");
}

fn pkg_version() -> String {
    std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string())
}

/// `git describe --tags --dirty`, `None` when no tag is reachable or git
/// is unavailable.
fn describe_tags() -> Option<String> {
    run_git(&["describe", "--tags", "--dirty"]).filter(|s| !s.is_empty())
}

/// Short commit hash prefixed with the package version, with `-dirty`
/// appended when the working tree has uncommitted changes.
fn hash_fallback() -> Option<String> {
    let hash = run_git(&["rev-parse", "--short", "HEAD"])?;
    let dirty = run_git(&["status", "--porcelain", "--untracked-files=no"])
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    let suffix = if dirty { "-dirty" } else { "" };
    Some(format!("{}+{hash}{suffix}", pkg_version()))
}

fn read_head_ref() -> Option<String> {
    let head = run_git(&["symbolic-ref", "-q", "HEAD"])?;
    if head.is_empty() { None } else { Some(head) }
}

fn run_git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
