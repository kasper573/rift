use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use bevy::asset::io::{
    AssetReader, AssetReaderError, AssetSourceBuilder, ErasedAssetReader, PathStream, Reader,
    VecReader,
};
use world::core::assets::AssetRef;

pub fn embedded_source() -> AssetSourceBuilder {
    AssetSourceBuilder::new(|| Box::new(EmbeddedReader) as Box<dyn ErasedAssetReader>)
}

struct EmbeddedReader;

impl AssetReader for EmbeddedReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        intern(path)
            .resolve()
            .map(|file| VecReader::new(file.contents().to_vec()))
            .ok_or_else(|| AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        Err::<VecReader, _>(AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        Err(AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn is_directory<'a>(&'a self, _path: &'a Path) -> Result<bool, AssetReaderError> {
        Ok(false)
    }
}

pub(crate) fn intern(path: &Path) -> AssetRef {
    static POOL: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let pool = POOL.get_or_init(|| Mutex::new(HashMap::new()));
    let key = path.to_string_lossy().into_owned();
    let mut guard = pool.lock().expect("asset path pool");
    if let Some(&interned) = guard.get(&key) {
        return AssetRef(interned);
    }
    let interned: &'static str = Box::leak(key.clone().into_boxed_str());
    guard.insert(key, interned);
    AssetRef(interned)
}
