use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::io::{self, Read};
use std::path::Path;
use std::sync::{Arc, Mutex};

use bevy_ecs::prelude::Resource;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AssetRef(pub &'static str);

pub trait AssetSource: Send + Sync {
    fn open(&self, path: &Path) -> io::Result<Box<dyn Read>>;
}

#[cfg(not(target_arch = "wasm32"))]
pub struct FilesystemSource(pub std::path::PathBuf);

#[cfg(not(target_arch = "wasm32"))]
impl AssetSource for FilesystemSource {
    fn open(&self, path: &Path) -> io::Result<Box<dyn Read>> {
        Ok(Box::new(std::fs::File::open(self.0.join(path))?))
    }
}

type Cache = Mutex<HashMap<(AssetRef, TypeId), &'static (dyn Any + Send + Sync)>>;

#[derive(Resource, Clone)]
pub struct AssetService {
    source: Arc<dyn AssetSource>,
    cache: Arc<Cache>,
}

impl AssetService {
    pub fn new(source: impl AssetSource + 'static) -> AssetService {
        AssetService {
            source: Arc::new(source),
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn open(&self, path: &Path) -> io::Result<Box<dyn Read>> {
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
        let value: &'static T = Box::leak(Box::new(build(self, asset_ref)));
        self.cache
            .lock()
            .expect("asset cache")
            .entry(slot)
            .or_insert(value);
        value
    }
}
