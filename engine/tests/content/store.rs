use std::ops::Deref;

use engine::core::content::store::ContentStore;

/// A store in a folder of its own, gone with it.
pub struct TestStore {
    _folder: tempfile::TempDir,
    store: ContentStore,
}

impl Default for TestStore {
    fn default() -> TestStore {
        let folder = tempfile::tempdir().expect("a scratch folder");
        let store = ContentStore::new(folder.path().join("cache"));
        TestStore {
            _folder: folder,
            store,
        }
    }
}

impl TestStore {
    /// A store whose folder is a file, so nothing can be written to it.
    pub fn refusing_writes() -> TestStore {
        let folder = tempfile::tempdir().expect("a scratch folder");
        let blocked = folder.path().join("cache");
        std::fs::write(&blocked, b"").expect("a file in the store's place");
        TestStore {
            _folder: folder,
            store: ContentStore::new(blocked),
        }
    }

    pub fn hashes(&self) -> Vec<String> {
        let mut hashes = self.store.list();
        hashes.sort();
        hashes
    }

    pub fn drop_entry(&self, hash: &str) {
        self.store.remove(hash);
    }

    pub fn put(&self, hash: &str, bytes: &[u8]) {
        self.store.write(hash, bytes).expect("the store keeps it");
    }
}

impl Deref for TestStore {
    type Target = ContentStore;

    fn deref(&self) -> &ContentStore {
        &self.store
    }
}
