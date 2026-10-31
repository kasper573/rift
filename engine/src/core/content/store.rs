use std::sync::Arc;

use crate::core::platform::LocalFuture;

/// Where synced files stay between visits, each under its blake3 hash. It is a cache: any entry may
/// be gone or wrong, so the reader verifies what it gets back.
pub trait ContentStore {
    fn list(&self) -> LocalFuture<Vec<String>>;
    fn read(&self, hash: &str) -> LocalFuture<Option<Vec<u8>>>;
    fn write(&self, hash: &str, bytes: Arc<[u8]>) -> LocalFuture<Result<(), String>>;
    fn remove(&self, hash: &str) -> LocalFuture<()>;
    /// The store as it is read from any thread, so the synced files are served from it as they
    /// are asked for rather than held in memory; none where only the sync can read it.
    fn reader(&self) -> Option<Arc<dyn StoreReader>>;
}

/// A store read from any thread, by hash.
pub trait StoreReader: Send + Sync {
    fn read(&self, hash: &str) -> Option<Vec<u8>>;
}
