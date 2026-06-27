use std::io::{self, Read};
use std::path::{Path, PathBuf};

use world::core::assets::{AssetService, AssetSource};

/// The asset service the benchmark installs: a dev-only tool, it reads the
/// repo's `assets/` directory straight off disk.
pub fn service() -> AssetService {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"));
    AssetService::new(FilesystemSource { root })
}

struct FilesystemSource {
    root: PathBuf,
}

impl AssetSource for FilesystemSource {
    fn open(&self, path: &Path) -> io::Result<Box<dyn Read>> {
        Ok(Box::new(std::fs::File::open(self.root.join(path))?))
    }
}
