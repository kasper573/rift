use std::path::Path;
use std::process::Command;

/// A build is named by `RIFT_BUILD` when set, as CI names a release, and otherwise by its commit.
/// The name moves only when HEAD does: a build script that runs again recompiles this crate and
/// everything built on it even when it prints what it printed before, so it watches nothing that
/// changes while code is edited (the index, which `git status` rewrites), and no path that may not
/// exist (a missing path makes it run on every build).
fn main() {
    println!("cargo:rerun-if-env-changed=RIFT_BUILD");
    println!("cargo:rerun-if-env-changed=RIFT_RELEASE_KEY");
    let build = match std::env::var("RIFT_BUILD") {
        Ok(build) if !build.is_empty() => build,
        _ => commit().unwrap_or_else(|| "unknown".to_owned()),
    };
    println!("cargo:rustc-env=RIFT_BUILD={build}");
    if let Ok(key) = std::env::var("RIFT_RELEASE_KEY") {
        println!("cargo:rustc-env=RIFT_RELEASE_KEY={key}");
    }
}

/// The commit checked out, watching what moves with it: HEAD for a checkout, and HEAD's log for a
/// commit on the branch it names.
fn commit() -> Option<String> {
    let commit = git(&["rev-parse", "--short=8", "HEAD"])?;
    let watched = git(&[
        "rev-parse",
        "--path-format=absolute",
        "--git-path",
        "HEAD",
        "--git-path",
        "logs/HEAD",
    ])?;
    for path in watched.lines().filter(|path| Path::new(path).exists()) {
        println!("cargo:rerun-if-changed={path}");
    }
    Some(commit)
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}
