use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde::de::IgnoredAny;

/// The workspace's build profiles: dev and release alone, since each profile is a build tree of
/// its own, and release with the settings that make the shipped binaries as fast as they can be.
pub fn check(root: &Path, out: &mut Vec<String>) {
    let text = fs::read_to_string(root.join("Cargo.toml")).expect("the workspace's Cargo.toml");
    let manifest: Manifest = toml::from_str(&text).expect("the workspace's Cargo.toml");
    for name in manifest.profile.keys() {
        if name != "dev" && name != "release" {
            out.push(format!(
                "[Cargo.toml] profile.{name} — a profile of its own builds every crate again: build on dev or release"
            ));
        }
    }
    let release = manifest.profile.get("release");
    let mut refuse = |why: &str| out.push(format!("[Cargo.toml] profile.release — {why}"));
    if !release.is_some_and(
        |release| matches!(&release.lto, Some(Lto::Named(lto)) if lto == "thin" || lto == "fat"),
    ) {
        refuse("builds without LTO: set `lto` to thin or fat");
    }
    if release.and_then(|release| release.codegen_units) != Some(1) {
        refuse("splits its crates into several codegen units: set `codegen-units = 1`");
    }
    if release.and_then(|release| release.incremental) != Some(false) {
        refuse("builds incrementally: set `incremental = false`");
    }
    if release.and_then(|release| release.panic.as_deref()) == Some("abort") {
        refuse(
            "aborts on panic: the servers' request tasks and the asset loaders rely on a panic unwinding",
        );
    }
}

#[derive(Deserialize)]
struct Manifest {
    #[serde(default)]
    profile: BTreeMap<String, Profile>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Profile {
    lto: Option<Lto>,
    codegen_units: Option<u32>,
    incremental: Option<bool>,
    panic: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Lto {
    Named(String),
    Switch(IgnoredAny),
}
