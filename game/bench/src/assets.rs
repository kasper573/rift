use std::io::{self, Read};
use std::path::{Path, PathBuf};

use world::core::assets::{AssetRef, AssetService, AssetSource};

pub fn service() -> AssetService {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"));
    AssetService::new(FilesystemSource { root })
}

struct FilesystemSource {
    root: PathBuf,
}

impl AssetSource for FilesystemSource {
    fn abs(&self, asset_ref: AssetRef) -> io::Result<PathBuf> {
        Ok(self.root.join(asset_ref.0))
    }

    fn open(&self, path: &Path) -> io::Result<Box<dyn Read>> {
        Ok(Box::new(std::fs::File::open(path)?))
    }
}
