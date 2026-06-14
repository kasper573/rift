//! The directory layout is the schema. Render-facing lookups return paths relative to the root —
//! the client resolves them through its asset server, the server only validates them.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const MAPS: &str = "maps";
pub const TILESETS: &str = "tilesets";
pub const ACTORS: &str = "actors";
pub const ICONS: &str = "icons";

static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Sets the assets root for the process. A composition root — a binary's `main`, or a test — calls
/// this once before any content loads; the library never reaches into the environment for it, so
/// where the root comes from (a container env var, the `.env` beside a client, a test fixture) is
/// the caller's concern, not the domain's. Idempotent: the first root wins.
pub fn init(root: impl Into<PathBuf>) {
    let _ = ROOT.set(root.into());
}

/// The assets root set by [`init`]. Panics if content is loaded before a root is injected.
pub fn root() -> PathBuf {
    ROOT.get()
        .cloned()
        .expect("assets::init must be called before loading assets")
}

/// An asset's absolute path from its root-relative name.
pub fn path(relative: &str) -> PathBuf {
    root().join(relative)
}

pub fn bytes(name: &str) -> Option<Vec<u8>> {
    std::fs::read(path(name)).ok()
}

pub fn text(name: &str) -> Option<String> {
    std::fs::read_to_string(path(name)).ok()
}

pub fn exists(relative: &str) -> bool {
    path(relative).is_file()
}

/// The root-relative paths of every file under `dir`, recursively, sorted by name.
pub fn list(dir: &str) -> Vec<String> {
    let mut names = Vec::new();
    walk(dir, &mut names);
    names.sort();
    names
}

/// The root-relative path of the file under `dir` whose file name matches `reference`'s.
pub fn find(dir: &str, reference: &str) -> Option<String> {
    let wanted = file_name(reference);
    list(dir).into_iter().find(|name| file_name(name) == wanted)
}

/// rs-tiled's reader callback: opens a content file (a `.tmx`/`.tsx`, with relative references
/// already joined) under the assets root.
pub fn tiled_reader(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(root().join(path))
}

pub fn stem(name: &str) -> &str {
    let file = file_name(name);
    file.rsplit_once('.').map_or(file, |(stem, _)| stem)
}

fn file_name(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

fn walk(dir: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(path(dir)) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let relative = format!("{dir}/{name}");
        if entry.path().is_dir() {
            walk(&relative, out);
        } else {
            out.push(relative);
        }
    }
}
