use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::hash::Hash;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bevy_ecs::prelude::Resource;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AssetRef(pub &'static str);

pub trait AssetSource: Send + Sync {
    fn abs(&self, asset_ref: AssetRef) -> io::Result<PathBuf>;
    fn open(&self, path: &Path) -> io::Result<Box<dyn Read>>;
}

#[derive(Resource, Clone)]
pub struct AssetService {
    source: Arc<dyn AssetSource>,
    cache: Arc<Mutex<HashMap<TypeId, Box<dyn Any + Send + Sync>>>>,
}

impl AssetService {
    pub fn new(source: impl AssetSource + 'static) -> AssetService {
        AssetService {
            source: Arc::new(source),
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn abs(&self, asset_ref: AssetRef) -> io::Result<PathBuf> {
        self.source.abs(asset_ref)
    }

    pub fn open(&self, path: &Path) -> io::Result<Box<dyn Read>> {
        self.source.open(path)
    }

    pub fn resolve<K, T>(&self, key: K, build: impl FnOnce(&AssetService) -> T) -> &'static T
    where
        K: Copy + Eq + Hash + Send + Sync + 'static,
        T: Send + Sync + 'static,
    {
        let slot = TypeId::of::<(K, T)>();
        {
            let cache = self.cache.lock().expect("asset cache");
            if let Some(value) = cache
                .get(&slot)
                .and_then(|map| map.downcast_ref::<HashMap<K, &'static T>>())
                .and_then(|map| map.get(&key).copied())
            {
                return value;
            }
        }
        let value: &'static T = Box::leak(Box::new(build(self)));
        self.cache
            .lock()
            .expect("asset cache")
            .entry(slot)
            .or_insert_with(|| Box::new(HashMap::<K, &'static T>::new()))
            .downcast_mut::<HashMap<K, &'static T>>()
            .expect("asset cache type")
            .entry(key)
            .or_insert(value);
        value
    }
}
