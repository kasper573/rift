use std::io::{self, Read};
use std::path::{Path, PathBuf};

use world::core::assets::{AssetRef, AssetService, AssetSource};

/// The asset service the server installs: files are read from a directory on
/// disk, configured via `RIFT_GAME_SERVER_ASSETS_DIR`.
pub fn service(root: PathBuf) -> AssetService {
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
