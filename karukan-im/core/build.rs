//! Embed the git commit into the build so a running IME can report exactly
//! which build it is (`karukan_im::version()`).

use std::process::Command;

fn main() {
    let desc = git_describe().unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=KARUKAN_GIT_DESC={desc}");

    // Re-run when the checked-out commit changes. HEAD covers branch
    // switches; the ref file covers new commits on the current branch.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    if let Some(head_ref) = read_head_ref() {
        println!("cargo:rerun-if-changed=../../.git/{head_ref}");
    }
}

/// Short commit hash, with `-dirty` appended when the working tree has
/// uncommitted changes. `None` when git or the repository is unavailable
/// (e.g. building from a source tarball).
fn git_describe() -> Option<String> {
    let hash = run_git(&["rev-parse", "--short", "HEAD"])?;
    let dirty = run_git(&["status", "--porcelain", "--untracked-files=no"])
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    Some(if dirty { format!("{hash}-dirty") } else { hash })
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
