use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};

use bevy::asset::io::memory::{Dir, MemoryAssetReader};
use bevy::asset::io::{AssetSourceBuilder, ErasedAssetReader};
use include_dir::{Dir as Embedded, include_dir};
use world::core::assets::{AssetService, AssetSource};

static ASSETS: Embedded<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../assets");

pub fn service() -> AssetService {
    AssetService::new(EmbeddedSource)
}

pub fn bevy_source() -> AssetSourceBuilder {
    let root = Dir::new(PathBuf::new());
    fill(&root, &ASSETS);
    AssetSourceBuilder::new(move || {
        Box::new(MemoryAssetReader { root: root.clone() }) as Box<dyn ErasedAssetReader>
    })
}

pub fn key(path: &Path) -> Option<String> {
    let normalized = normalize(&path.to_string_lossy());
    ASSETS.get_file(&normalized).map(|_| normalized)
}

struct EmbeddedSource;

impl AssetSource for EmbeddedSource {
    fn open(&self, path: &Path) -> io::Result<Box<dyn Read>> {
        ASSETS
            .get_file(normalize(&path.to_string_lossy()))
            .map(|file| Box::new(Cursor::new(file.contents())) as Box<dyn Read>)
            .ok_or_else(|| missing(&path.to_string_lossy()))
    }
}

fn fill(dir: &Dir, embedded: &'static Embedded<'static>) {
    for file in embedded.files() {
        dir.insert_asset(file.path(), file.contents());
    }
    for sub in embedded.dirs() {
        fill(dir, sub);
    }
}

fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

fn missing(path: &str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("missing asset {path}"))
}
