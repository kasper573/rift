//! Embeds the `assets/` tree into the shipped binary. Behind the `dist` feature, the whole
//! directory is compiled in and served as the default asset source, so a distributed client is
//! self-contained — no `assets/` folder or `RIFT_ASSETS` needed.

use std::path::{Path, PathBuf};

use bevy::asset::AssetApp;
use bevy::asset::io::{
    AssetReader, AssetReaderError, AssetSourceBuilder, AssetSourceId, PathStream, Reader, VecReader,
};
use bevy::prelude::*;
use include_dir::{Dir, include_dir};

static ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/../../assets");

/// Registers the embedded tree as the default asset source. Must run before `DefaultPlugins`.
pub fn register(app: &mut App) {
    app.register_asset_source(
        AssetSourceId::Default,
        AssetSourceBuilder::new(|| Box::new(EmbeddedReader)),
    );
}

struct EmbeddedReader;

impl AssetReader for EmbeddedReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        match ASSETS.get_file(key(path)) {
            Some(file) => Ok(VecReader::new(file.contents().to_vec())),
            None => Err(AssetReaderError::NotFound(path.to_path_buf())),
        }
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        Err::<VecReader, _>(AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        let entries: Vec<PathBuf> = ASSETS
            .get_dir(key(path))
            .map(|dir| {
                dir.entries()
                    .iter()
                    .map(|entry| entry.path().to_path_buf())
                    .collect()
            })
            .unwrap_or_default();
        Ok(Box::new(futures_lite::stream::iter(entries)))
    }

    async fn is_directory<'a>(&'a self, path: &'a Path) -> Result<bool, AssetReaderError> {
        Ok(ASSETS.get_dir(key(path)).is_some())
    }
}

fn key(path: &Path) -> &str {
    path.to_str().unwrap_or_default()
}
