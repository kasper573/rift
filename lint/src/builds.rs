use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use serde::Deserialize;
use serde::de::IgnoredAny;

/// What keeps builds fast without anyone watching the clock: every recipe builds on one profile
/// and one feature resolution, so none rebuilds what another built, and only the production
/// recipes build `release`.
pub fn check(root: &Path, out: &mut Vec<String>) {
    check_recipes(&justfile(root), out);
    check_profiles(&manifest(root), out);
}

/// The recipes that build what ships, or sample it, and so alone choose `release`.
const PRODUCTION: &[&str] = &["archives", "profile"];

/// The cargo commands that compile, each of which must build on the recipe's profile.
const COMPILING: &[&str] = &[
    "cargo build",
    "cargo test",
    "cargo clippy",
    "cargo check",
    "cargo doc",
    "cargo bench",
    "cargo run",
    "cargo nextest",
];

/// Settings that make one build differ from another, so that each rebuilds what the other made.
const DIVERGING: &[&str] = &[
    "--release",
    "--target-dir",
    "RUSTFLAGS",
    "CARGO_PROFILE_",
    "CARGO_TARGET_DIR",
    "CARGO_INCREMENTAL",
    "RUSTDOCFLAGS",
];

fn check_recipes(justfile: &Justfile, out: &mut Vec<String>) {
    for (name, recipe) in &justfile.recipes {
        for line in recipe.body.iter().map(|line| rendered(line)) {
            let mut refuse = |why: &str| out.push(format!("[Justfile] {name}: `{line}` — {why}"));
            if line.contains("cargo run") {
                refuse(
                    "`cargo run` resolves one package's features: `cargo build --workspace --bin <name>`, then run it from {{out_dir}}",
                );
            } else if COMPILING.iter().any(|command| line.contains(command)) {
                if !line.contains("--workspace") {
                    refuse(
                        "builds without `--workspace`, so it resolves other features than the other recipes and rebuilds what they built",
                    );
                }
                if !line.contains("--profile {{profile}}") {
                    refuse("builds on a profile of its own: pass `--profile {{profile}}`");
                }
            }
            for setting in DIVERGING.iter().filter(|setting| line.contains(*setting)) {
                refuse(&format!(
                    "`{setting}` makes this build differ from the others, so each rebuilds what the other made"
                ));
            }
            if line.contains("profile=release") && !PRODUCTION.contains(&name.as_str()) {
                refuse(&format!(
                    "only {} build release: everything else builds on {{{{profile}}}}",
                    PRODUCTION.join(" and ")
                ));
            }
        }
    }
}

/// The release profile's settings, which make the shipped binaries as fast as they can be; and no
/// profile beyond dev and release, since each is a build tree of its own.
fn check_profiles(manifest: &Manifest, out: &mut Vec<String>) {
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
        |release| matches!(release.lto, Some(Lto::Named(ref lto)) if lto == "thin" || lto == "fat"),
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

fn justfile(root: &Path) -> Justfile {
    let output = Command::new("just")
        .args(["--dump", "--dump-format", "json", "--justfile"])
        .arg(root.join("Justfile"))
        .output()
        .expect("run `just --dump`");
    assert!(output.status.success(), "`just --dump` failed");
    serde_json::from_slice(&output.stdout).expect("just's dump of the Justfile")
}

fn manifest(root: &Path) -> Manifest {
    let text = fs::read_to_string(root.join("Cargo.toml")).expect("the workspace's Cargo.toml");
    toml::from_str(&text).expect("the workspace's Cargo.toml")
}

/// A recipe line as written, its interpolations of a variable as `{{name}}` and any other as
/// `{{…}}`.
fn rendered(line: &[Fragment]) -> String {
    line.iter()
        .map(|fragment| match fragment {
            Fragment::Text(text) => text.clone(),
            Fragment::Interpolation((Expression::Variable(_, name),)) => format!("{{{{{name}}}}}"),
            Fragment::Interpolation((Expression::Other(_),)) => "{{…}}".to_owned(),
        })
        .collect()
}

#[derive(Deserialize)]
struct Justfile {
    recipes: BTreeMap<String, Recipe>,
}

#[derive(Deserialize)]
struct Recipe {
    body: Vec<Vec<Fragment>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Fragment {
    Text(String),
    Interpolation((Expression,)),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Expression {
    Variable(VariableTag, String),
    Other(IgnoredAny),
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum VariableTag {
    Variable,
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
