use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=RIFT_BUILD");
    println!("cargo:rerun-if-env-changed=RIFT_RELEASE_KEY");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/index");
    if let Some(head) = git(&["rev-parse", "--symbolic-full-name", "HEAD"]) {
        println!("cargo:rerun-if-changed=../.git/{head}");
    }
    let build = std::env::var("RIFT_BUILD")
        .ok()
        .filter(|build| !build.is_empty())
        .or_else(git_build)
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=RIFT_BUILD={build}");
    if let Ok(key) = std::env::var("RIFT_RELEASE_KEY") {
        println!("cargo:rustc-env=RIFT_RELEASE_KEY={key}");
    }
}

fn git_build() -> Option<String> {
    let commit = git(&["rev-parse", "--short=8", "HEAD"])?;
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"])
        .is_some_and(|status| !status.is_empty());
    Some(if dirty {
        format!("{commit}-dirty")
    } else {
        commit
    })
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}
