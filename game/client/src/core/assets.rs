use std::path::PathBuf;

use bevy::asset::io::memory::{Dir, MemoryAssetReader};
use bevy::asset::io::{AssetSourceBuilder, ErasedAssetReader};

pub fn embedded_source() -> AssetSourceBuilder {
    let root = Dir::new(PathBuf::new());
    fill(&root, world::core::assets::dir());
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
