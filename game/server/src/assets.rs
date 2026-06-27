use std::io::{self, Read};
use std::path::{Path, PathBuf};

use world::core::assets::{AssetService, AssetSource};

/// The asset service the server installs: files are read from a directory on
/// disk, configured via `RIFT_GAME_SERVER_ASSETS_DIR`.
pub fn service(root: PathBuf) -> AssetService {
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
