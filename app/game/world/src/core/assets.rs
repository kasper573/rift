//! Every file under `assets/` is embedded at build time; the directory layout is the schema.

pub const MAPS: &str = "maps";
pub const TILESETS: &str = "tilesets";
pub const ACTORS: &str = "actors";
pub const ICONS: &str = "icons";

include!(concat!(env!("OUT_DIR"), "/assets.rs"));

pub fn dir(dir: &str) -> impl Iterator<Item = (&'static str, &'static [u8])> {
    FILES
        .iter()
        .filter(move |(name, _)| name.strip_prefix(dir).is_some_and(|r| r.starts_with('/')))
        .map(|&(name, bytes)| (name, bytes))
}

pub fn find(dir: &str, reference: &str) -> Option<(&'static str, &'static [u8])> {
    let file = file_name(reference);
    self::dir(dir).find(|(name, _)| file_name(name) == file)
}

pub fn find_text(dir: &str, reference: &str) -> Option<&'static str> {
    find(dir, reference).and_then(|(_, bytes)| std::str::from_utf8(bytes).ok())
}

pub fn bytes(name: &str) -> Option<&'static [u8]> {
    FILES
        .iter()
        .find(|(file, _)| *file == name)
        .map(|&(_, bytes)| bytes)
}

pub fn text(name: &str) -> Option<&'static str> {
    bytes(name).and_then(|bytes| std::str::from_utf8(bytes).ok())
}

pub fn stem(name: &str) -> &str {
    let file = file_name(name);
    file.rsplit_once('.').map_or(file, |(stem, _)| stem)
}

fn file_name(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}
