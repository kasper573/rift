use std::path::PathBuf;

use bevy::asset::io::memory::{Dir, MemoryAssetReader};
use bevy::asset::io::{AssetSourceBuilder, ErasedAssetReader};

/// A Bevy asset source backed by the same `include_dir` embed `world::assets` uses, so the browser
/// client loads sprites, fonts, and audio straight from the binary — no HTTP asset fetches, no
/// asset-path configuration. Registered as the default source before `AssetPlugin`.
pub fn embedded_source() -> AssetSourceBuilder {
    let root = Dir::new(PathBuf::new());
    fill(&root, world::assets::dir());
    AssetSourceBuilder::new(move || {
        Box::new(MemoryAssetReader { root: root.clone() }) as Box<dyn ErasedAssetReader>
    })
}

fn fill(dir: &Dir, embedded: &'static include_dir::Dir<'static>) {
    for file in embedded.files() {
        dir.insert_asset(file.path(), file.contents());
    }
    for sub in embedded.dirs() {
        fill(dir, sub);
    }
}
