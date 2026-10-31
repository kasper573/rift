use std::path::PathBuf;
use std::sync::Arc;

use engine::core::content::store::{ContentStore, StoreReader};
use engine::core::platform::LocalFuture;

#[derive(Clone)]
pub struct FileStore {
    dir: PathBuf,
}

impl FileStore {
    pub fn new(dir: PathBuf) -> FileStore {
        FileStore { dir }
    }
}

impl ContentStore for FileStore {
    fn list(&self) -> LocalFuture<Vec<String>> {
        let hashes = std::fs::read_dir(&self.dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter_map(|entry| entry.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default();
        Box::pin(std::future::ready(hashes))
    }

    fn read(&self, hash: &str) -> LocalFuture<Option<Vec<u8>>> {
        Box::pin(std::future::ready(StoreReader::read(self, hash)))
    }

    fn write(&self, hash: &str, bytes: Arc<[u8]>) -> LocalFuture<Result<(), String>> {
        let result = std::fs::create_dir_all(&self.dir)
            .and_then(|()| {
                let part = self.dir.join(format!("{hash}.part"));
                std::fs::write(&part, &bytes)?;
                std::fs::rename(part, self.dir.join(hash))
            })
            .map_err(|error| error.to_string());
        Box::pin(std::future::ready(result))
    }

    fn remove(&self, hash: &str) -> LocalFuture<()> {
        let _ = std::fs::remove_file(self.dir.join(hash));
        Box::pin(std::future::ready(()))
    }

    fn reader(&self) -> Option<Arc<dyn StoreReader>> {
        Some(Arc::new(self.clone()))
    }
}

impl StoreReader for FileStore {
    fn read(&self, hash: &str) -> Option<Vec<u8>> {
        std::fs::read(self.dir.join(hash)).ok()
    }
}
