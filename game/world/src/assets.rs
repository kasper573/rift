use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const MAPS: &str = "maps";
pub const TILESETS: &str = "tilesets";
pub const ACTORS: &str = "actors";
pub const ICONS: &str = "icons";

static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Idempotent: the first root wins.
pub fn init(root: impl Into<PathBuf>) {
    let _ = ROOT.set(root.into());
}

pub fn root() -> PathBuf {
    ROOT.get()
        .cloned()
        .expect("assets::init must be called before loading assets")
}

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

pub fn list(dir: &str) -> Vec<String> {
    let mut names = Vec::new();
    walk(dir, &mut names);
    names.sort();
    names
}

pub fn find(dir: &str, reference: &str) -> Option<String> {
    let wanted = file_name(reference);
    list(dir).into_iter().find(|name| file_name(name) == wanted)
}

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
