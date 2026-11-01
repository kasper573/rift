use std::collections::HashMap;
use std::sync::Arc;

use engine::core::content::AssetKind;
use engine::core::content::gltf::{self, GltfInfo};

use super::ProjectFiles;

/// Every glTF binary of the game folder the editor has read, by its path, with the hash of the
/// bytes it was read from.
#[derive(Default)]
pub struct GltfCache(HashMap<String, (String, Option<Arc<GltfInfo>>)>);

impl GltfCache {
    /// The file at `path` as the loader reads it, when it reads.
    pub fn info(&mut self, files: &ProjectFiles, path: &str) -> Option<Arc<GltfInfo>> {
        let hash = files.hash_of(path)?;
        if let Some((held, read)) = self.0.get(path)
            && held == hash
        {
            return read.clone();
        }
        let read = files
            .get(path)
            .and_then(|bytes| gltf::inspect(bytes).ok())
            .map(Arc::new);
        self.0
            .insert(path.to_owned(), (hash.to_owned(), read.clone()));
        read
    }

    /// Every glTF binary of the game folder that reads, by its path.
    pub fn every(&mut self, files: &ProjectFiles) -> Vec<(String, Arc<GltfInfo>)> {
        let index = files.index();
        index
            .media
            .iter()
            .filter(|path| AssetKind::of(path) == Some(AssetKind::Model))
            .filter_map(|path| Some((path.clone(), self.info(files, path)?)))
            .collect()
    }
}
