pub mod loader;

use std::path::Path;
use std::sync::{Arc, RwLock};

use bevy::asset::io::{
    AssetReader, AssetReaderError, AssetSourceBuilder, ErasedAssetReader, PathStream, Reader,
    VecReader,
};
use bevy::asset::{AssetPath, AssetServer, RenderAssetUsages};
use bevy::gltf::{Gltf, GltfAssetLabel, GltfLoaderSettings};
use bevy::prelude::{Handle, Resource};
use bevy::world_serialization::WorldAsset;

use crate::core::content::sync::SyncedAssets;

/// A model's scene as a prop, an item or what a figure holds is drawn: its textures stand on the
/// GPU alone, as nothing reads them back, where a skin's are read for its paint.
pub fn model_scene(
    assets: &AssetServer,
    model: impl Into<AssetPath<'static>>,
) -> Handle<WorldAsset> {
    assets
        .load_builder()
        .with_settings(drawn_only)
        .load(GltfAssetLabel::Scene(0).from_asset(model))
}

/// A model as a whole, loaded as [`model_scene`] loads it.
pub fn model(assets: &AssetServer, model: impl Into<AssetPath<'static>>) -> Handle<Gltf> {
    assets.load_builder().with_settings(drawn_only).load(model)
}

fn drawn_only(settings: &mut GltfLoaderSettings) {
    settings.load_materials = RenderAssetUsages::RENDER_WORLD;
}

/// The reader bevy loads every asset through: the assets of the latest sync, swapped whole when
/// the content syncs again.
#[derive(Resource, Clone, Default)]
pub struct SyncedAssetReader(Arc<RwLock<SyncedAssets>>);

impl SyncedAssetReader {
    pub fn replace(&self, assets: SyncedAssets) {
        *self.0.write().expect("synced assets lock") = assets;
    }

    pub fn source(&self) -> AssetSourceBuilder {
        let files = self.clone();
        AssetSourceBuilder::new(move || Box::new(files.clone()) as Box<dyn ErasedAssetReader>)
    }

    fn get(&self, path: &Path) -> Option<Arc<[u8]>> {
        self.0
            .read()
            .expect("synced assets lock")
            .read(path.to_str()?)
    }
}

impl AssetReader for SyncedAssetReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        self.get(path)
            .map(|bytes| VecReader::new(bytes.to_vec()))
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
