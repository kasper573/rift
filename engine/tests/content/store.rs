use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use engine::core::content::store::{ContentStore, StoreReader};
use engine::core::platform::LocalFuture;

#[derive(Clone, Default)]
pub struct MemoryStore {
    entries: Arc<Mutex<HashMap<String, Arc<[u8]>>>>,
    refuses_writes: bool,
}

impl MemoryStore {
    pub fn refusing_writes() -> MemoryStore {
        MemoryStore {
            refuses_writes: true,
            ..MemoryStore::default()
        }
    }

    pub fn hashes(&self) -> Vec<String> {
        let mut hashes: Vec<String> = self.entries.lock().unwrap().keys().cloned().collect();
        hashes.sort();
        hashes
    }

    pub fn drop_entry(&self, hash: &str) {
        self.entries.lock().unwrap().remove(hash);
    }

    pub fn put(&self, hash: &str, bytes: &[u8]) {
        self.entries
            .lock()
            .unwrap()
            .insert(hash.to_owned(), Arc::from(bytes));
    }
}

impl ContentStore for MemoryStore {
    fn list(&self) -> LocalFuture<Vec<String>> {
        Box::pin(std::future::ready(self.hashes()))
    }

    fn read(&self, hash: &str) -> LocalFuture<Option<Vec<u8>>> {
        Box::pin(std::future::ready(StoreReader::read(self, hash)))
    }

    fn write(&self, hash: &str, bytes: Arc<[u8]>) -> LocalFuture<Result<(), String>> {
        let result = if self.refuses_writes {
            Err("the store refuses writes".to_owned())
        } else {
            self.entries.lock().unwrap().insert(hash.to_owned(), bytes);
            Ok(())
        };
        Box::pin(std::future::ready(result))
    }

    fn remove(&self, hash: &str) -> LocalFuture<()> {
        self.entries.lock().unwrap().remove(hash);
        Box::pin(std::future::ready(()))
    }

    fn reader(&self) -> Option<Arc<dyn StoreReader>> {
        Some(Arc::new(self.clone()))
    }
}

impl StoreReader for MemoryStore {
    fn read(&self, hash: &str) -> Option<Vec<u8>> {
        self.entries
            .lock()
            .unwrap()
            .get(hash)
            .map(|bytes| bytes.to_vec())
    }
}
