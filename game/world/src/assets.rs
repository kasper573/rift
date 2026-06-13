//! The directory layout is the schema. Render-facing lookups return paths relative to the root —
//! the client resolves them through its asset server, the server only validates them.

use std::path::{Path, PathBuf};

pub const MAPS: &str = "maps";
pub const TILESETS: &str = "tilesets";
pub const ACTORS: &str = "actors";
pub const ICONS: &str = "icons";

/// The assets root, from the required `RIFT_ASSETS_DIR`. The client sets this from the `.env` shipped
/// beside its executable; the server gets it from its container. A missing value is a hard error,
/// never a guessed path.
pub fn root() -> PathBuf {
    std::env::var_os("RIFT_ASSETS_DIR")
        .map(PathBuf::from)
        .expect("RIFT_ASSETS_DIR must be set")
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
