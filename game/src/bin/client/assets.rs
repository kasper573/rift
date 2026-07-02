use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};

use bevy::asset::AssetPath;
use bevy::asset::io::memory::{Dir, MemoryAssetReader};
use bevy::asset::io::{AssetSourceBuilder, ErasedAssetReader};
use game::core::assets::{AssetService, AssetSource};
use include_dir::{Dir as Embedded, include_dir};

static ASSETS: Embedded<'static> = include_dir!("$CARGO_MANIFEST_DIR/../assets");

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

struct EmbeddedSource;

impl AssetSource for EmbeddedSource {
    fn open(&self, path: &Path) -> io::Result<Box<dyn Read>> {
        let name = path.to_string_lossy();
        let resolved = AssetPath::from("").resolve(&AssetPath::parse(&name));
        ASSETS
            .get_file(resolved.path())
            .map(|file| Box::new(Cursor::new(file.contents())) as Box<dyn Read>)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("missing asset {}", resolved.path().display()),
                )
            })
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
