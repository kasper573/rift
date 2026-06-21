use std::io::Cursor;
use std::path::{Component, Path};

use include_dir::{Dir, include_dir};

pub const MAPS: &str = "maps";
pub const TILESETS: &str = "tilesets";
pub const ACTORS: &str = "actors";
pub const ICONS: &str = "icons";

/// The game content, baked into the binary at build time. Embedding (rather than reading a runtime
/// directory) is what lets the same `world` library back both the native server and the wasm client,
/// which has no filesystem — and removes every asset-path env var. The client also serves this same
/// embed to Bevy's `AssetServer` (see `client::assets`), so media is loaded from one source.
static ASSETS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../assets");

/// The embedded asset tree, for backends that read it directly (e.g. the client's Bevy asset source).
pub fn dir() -> &'static Dir<'static> {
    &ASSETS
}

pub fn bytes(name: &str) -> Option<Vec<u8>> {
    ASSETS.get_file(name).map(|file| file.contents().to_vec())
}

pub fn text(name: &str) -> Option<String> {
    ASSETS
        .get_file(name)
        .and_then(|file| file.contents_utf8())
        .map(str::to_owned)
}

pub fn exists(relative: &str) -> bool {
    ASSETS.get_file(relative).is_some()
}

pub fn list(dir: &str) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(dir) = ASSETS.get_dir(dir) {
        walk(dir, &mut names);
    }
    names.sort();
    names
}

pub fn find(dir: &str, reference: &str) -> Option<String> {
    let wanted = file_name(reference);
    list(dir).into_iter().find(|name| file_name(name) == wanted)
}

/// A reader for the `tiled` crate. tiled hands back map-relative paths (e.g. `maps/../tilesets/x.tsx`)
/// that the OS would resolve on open; the embed has no such resolution, so we normalize `.`/`..` first.
pub fn tiled_reader(path: &Path) -> std::io::Result<Cursor<&'static [u8]>> {
    let key = normalize(path);
    ASSETS
        .get_file(&key)
        .map(|file| Cursor::new(file.contents()))
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, format!("missing asset {key}"))
        })
}

pub fn stem(name: &str) -> &str {
    let file = file_name(name);
    file.rsplit_once('.').map_or(file, |(stem, _)| stem)
}

fn file_name(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

fn walk(dir: &Dir, out: &mut Vec<String>) {
    for file in dir.files() {
        if let Some(name) = file.path().to_str() {
            out.push(name.to_owned());
        }
    }
    for sub in dir.dirs() {
        walk(sub, out);
    }
}

fn normalize(path: &Path) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str().unwrap_or_default()),
            Component::ParentDir => {
                parts.pop();
            }
            _ => {}
        }
    }
    parts.join("/")
}
