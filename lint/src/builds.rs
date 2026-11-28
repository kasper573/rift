use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

/// The workspace's build profiles: dev and release alone, since each profile is a build tree of its
/// own, and none that aborts on a panic.
pub fn check(root: &Path, out: &mut Vec<String>) {
    let text = fs::read_to_string(root.join("Cargo.toml")).expect("the workspace's Cargo.toml");
    let manifest: Manifest = toml::from_str(&text).expect("the workspace's Cargo.toml");
    for (name, profile) in &manifest.profile {
        if name != "dev" && name != "release" {
            out.push(format!(
                "[Cargo.toml] profile.{name} — a profile of its own builds every crate again: build on dev or release"
            ));
        }
        if profile.panic.as_deref() == Some("abort") {
            out.push(format!(
                "[Cargo.toml] profile.{name} — aborts on panic: the servers' request tasks and the asset loaders rely on a panic unwinding"
            ));
        }
    }
}

#[derive(Deserialize)]
struct Manifest {
    #[serde(default)]
    profile: BTreeMap<String, Profile>,
}

#[derive(Deserialize)]
struct Profile {
    panic: Option<String>,
}
