pub mod loader;

use std::path::Path;
use std::sync::{Arc, RwLock};

use bevy::asset::io::{
    AssetReader, AssetReaderError, AssetSourceBuilder, AssetSourceId, ErasedAssetReader,
    PathStream, Reader, VecReader,
};
use bevy::asset::{AssetApp, AssetPath, AssetServer, RenderAssetUsages};
use bevy::gltf::{Gltf, GltfAssetLabel, GltfLoaderSettings};
use bevy::prelude::{App, Handle, Resource};
use bevy::world_serialization::WorldAsset;

use crate::core::content::gltf;
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

/// A model as a whole, for the props the arenas draw: they read its meshes once, into their own
/// buffers, so its meshes stay off the GPU, and its textures stand there alone as
/// [`model_scene`]'s do. It loads through a source of its own, so a scene of the same model, which
/// bevy draws from those meshes, loads apart from it.
pub fn model(assets: &AssetServer, model: impl Into<AssetPath<'static>>) -> Handle<Gltf> {
    assets
        .load_builder()
        .with_settings(read_into_arenas)
        .load(model.into().with_source(ARENA_SOURCE))
}

/// Has `app` load every asset from the game folder at `folder`, from each source the engine
/// loads from.
pub fn serve_folder(app: &mut App, folder: &Path) {
    let folder = folder.display().to_string();
    for source in sources() {
        app.register_asset_source(source, AssetSourceBuilder::platform_default(&folder, None));
    }
}

/// Has `assets` read the file at `path` again, wherever it was loaded from.
pub fn reload(assets: &AssetServer, path: &str) {
    for source in sources() {
        let path = AssetPath::from(path.to_owned()).with_source(source);
        if assets.get_path_id(path.clone()).is_some() {
            assets.reload(path);
        }
    }
}

const ARENA_SOURCE: &str = "arena";

/// Every source the engine loads assets from.
fn sources() -> [AssetSourceId<'static>; 2] {
    [AssetSourceId::Default, AssetSourceId::from(ARENA_SOURCE)]
}

fn drawn_only(settings: &mut GltfLoaderSettings) {
    settings.load_materials = RenderAssetUsages::RENDER_WORLD;
}

fn read_into_arenas(settings: &mut GltfLoaderSettings) {
    drawn_only(settings);
    settings.load_meshes = RenderAssetUsages::MAIN_WORLD;
}

/// The reader bevy loads every asset through: the assets of the latest sync, swapped whole when
/// the content syncs again. A model naming an image whose bytes another file holds too names
/// that file instead, the first of those bytes, so an image many models share, each bundle with
/// its copy, loads once.
#[derive(Resource, Clone, Default)]
pub struct SyncedAssetReader(Arc<RwLock<SyncedAssets>>);

impl SyncedAssetReader {
    /// Has `app` load every asset through these files, from each source the engine loads from.
    pub fn register(&self, app: &mut App) {
        for source in sources() {
            app.register_asset_source(source, self.source());
        }
    }

    pub fn replace(&self, assets: SyncedAssets) {
        *self.0.write().expect("synced assets lock") = assets;
    }

    fn source(&self) -> AssetSourceBuilder {
        let files = self.clone();
        AssetSourceBuilder::new(move || Box::new(files.clone()) as Box<dyn ErasedAssetReader>)
    }

    fn get(&self, path: &Path) -> Option<Arc<[u8]>> {
        let assets = self.0.read().expect("synced assets lock");
        let bytes = assets.read(path.to_str()?)?;
        if path.extension().is_none_or(|extension| extension != "glb") {
            return Some(bytes);
        }
        let model = AssetPath::from_path(path);
        let shared = |uri: &str| {
            let image = model.resolve_embed_str(uri).ok()?;
            let first = assets.copy_of(image.path().to_str()?)?;
            Some(format!("/{first}"))
        };
        Some(gltf::with_images_renamed(&bytes, shared).map_or(bytes, Arc::from))
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
