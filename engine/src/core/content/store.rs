use std::path::PathBuf;

/// Where synced files stay between visits, each under its blake3 hash. It is a cache: any entry may
/// be gone or wrong, so the reader verifies what it gets back.
#[derive(Clone)]
pub struct ContentStore {
    dir: PathBuf,
}

impl ContentStore {
    pub fn new(dir: PathBuf) -> ContentStore {
        ContentStore { dir }
    }

    pub fn list(&self) -> Vec<String> {
        std::fs::read_dir(&self.dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter_map(|entry| entry.file_name().into_string().ok())
                    .filter(|name| !name.ends_with(PART))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn read(&self, hash: &str) -> Option<Vec<u8>> {
        std::fs::read(self.dir.join(hash)).ok()
    }

    pub fn write(&self, hash: &str, bytes: &[u8]) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir)
            .and_then(|()| {
                let part = self.dir.join(format!("{hash}{PART}"));
                std::fs::write(&part, bytes)?;
                std::fs::rename(part, self.dir.join(hash))
            })
            .map_err(|error| error.to_string())
    }

    pub fn remove(&self, hash: &str) {
        let _ = std::fs::remove_file(self.dir.join(hash));
    }
}

/// A file written in part, renamed to its hash once whole, so a crash mid-write leaves no entry
/// under the hash.
const PART: &str = ".part";
