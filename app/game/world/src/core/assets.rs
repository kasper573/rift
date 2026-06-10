//! Every file under `assets/` is embedded at compile time; the directory layout is the schema.

use std::io::Cursor;
use std::path::Path;

use include_dir::{Dir, include_dir};

pub const MAPS: &str = "maps";
pub const TILESETS: &str = "tilesets";
pub const ACTORS: &str = "actors";
pub const ICONS: &str = "icons";

static ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/assets");

pub fn dir(path: &str) -> impl Iterator<Item = (&'static str, &'static [u8])> {
    let mut files = Vec::new();
    if let Some(dir) = ASSETS.get_dir(path) {
        walk(dir, &mut files);
    }
    files.sort_by_key(|&(name, _)| name);
    files.into_iter()
}

pub fn find(dir: &str, reference: &str) -> Option<(&'static str, &'static [u8])> {
    let file = file_name(reference);
    self::dir(dir).find(|(name, _)| file_name(name) == file)
}

pub fn find_text(dir: &str, reference: &str) -> Option<&'static str> {
    find(dir, reference).and_then(|(_, bytes)| std::str::from_utf8(bytes).ok())
}

pub fn bytes(name: &str) -> Option<&'static [u8]> {
    Some(ASSETS.get_file(name)?.contents())
}

/// Reads an embedded asset for the tiled loader, normalizing the loader's relative paths
/// (`maps/../tilesets/x.tsx`) to embedded keys.
pub fn tiled_reader(path: &Path) -> std::io::Result<Cursor<&'static [u8]>> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::Normal(name) => {
                parts.push(name.to_str().unwrap_or_default());
            }
            _ => {}
        }
    }
    let key = parts.join("/");
    bytes(&key)
        .map(Cursor::new)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, key))
}

pub fn text(name: &str) -> Option<&'static str> {
    ASSETS.get_file(name)?.contents_utf8()
}

pub fn stem(name: &str) -> &str {
    let file = file_name(name);
    file.rsplit_once('.').map_or(file, |(stem, _)| stem)
}

fn walk(dir: &'static Dir, out: &mut Vec<(&'static str, &'static [u8])>) {
    for file in dir.files() {
        if let Some(name) = file.path().to_str() {
            out.push((name, file.contents()));
        }
    }
    for sub in dir.dirs() {
        walk(sub, out);
    }
}

fn file_name(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}
