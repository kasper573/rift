use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::sync::{Arc, Mutex};

use bevy_ecs::prelude::Resource;

use crate::core::content::Content;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AssetRef(pub &'static str);

pub trait AssetSource: Send + Sync {
    fn open(&self, path: &Path) -> io::Result<Box<dyn BufRead>>;
}

pub struct FilesystemSource(pub std::path::PathBuf);

impl AssetSource for FilesystemSource {
    fn open(&self, path: &Path) -> io::Result<Box<dyn BufRead>> {
        Ok(Box::new(BufReader::new(std::fs::File::open(
            self.0.join(path),
        )?)))
    }
}

type Cache = Mutex<HashMap<(AssetRef, TypeId), &'static (dyn Any + Send + Sync)>>;

#[derive(Resource, Clone)]
pub struct AssetService {
    source: Arc<dyn AssetSource>,
    cache: Arc<Cache>,
    content: Content,
}

impl AssetService {
    pub fn new(source: impl AssetSource + 'static, content: Content) -> AssetService {
        AssetService {
            source: Arc::new(source),
            cache: Arc::new(Mutex::new(HashMap::new())),
            content,
        }
    }

    pub fn content(&self) -> &Content {
        &self.content
    }

    pub fn open(&self, path: &Path) -> io::Result<Box<dyn BufRead>> {
        self.source.open(path)
    }

    /// Builds the `T` that `asset_ref` produces (the builder reads it, and anything
    /// it references, back through the service), then caches and returns it. Keyed
    /// by reference and output type, so refs to the same file share one result.
    pub fn resolve<T: Send + Sync + 'static>(
        &self,
        asset_ref: AssetRef,
        build: impl FnOnce(&AssetService, AssetRef) -> T,
    ) -> &'static T {
        let slot = (asset_ref, TypeId::of::<T>());
        if let Some(&cached) = self.cache.lock().expect("asset cache").get(&slot) {
            return cached
                .downcast_ref::<T>()
                .expect("asset resolved under one type");
        }
        let built: &'static (dyn Any + Send + Sync) = Box::leak(Box::new(build(self, asset_ref)));
        let cached = *self
            .cache
            .lock()
            .expect("asset cache")
            .entry(slot)
            .or_insert(built);
        cached
            .downcast_ref::<T>()
            .expect("asset resolved under one type")
    }

    pub fn resolve_all<T: Send + Sync + 'static>(
        &self,
        asset_refs: impl IntoIterator<Item = AssetRef>,
        build: impl Fn(&AssetService, AssetRef) -> T + Sync,
    ) -> Vec<&'static T> {
        use rayon::prelude::*;

        let asset_refs: Vec<AssetRef> = asset_refs.into_iter().collect();
        asset_refs
            .into_par_iter()
            .map(|asset_ref| self.resolve(asset_ref, &build))
            .collect()
    }
}
