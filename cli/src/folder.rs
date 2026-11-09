//! A game folder as the tools read and write it: through the loader's reader, so every file is
//! read and written as the editor does.

use std::path::Path;
use std::sync::Arc;

use engine::core::content::{Content, FilesystemFolder, LoadPurpose, LoadReport};
use engine::systems::load_content;

/// The game in `folder` loaded for `purpose`, `outcome` naming what the load is for.
pub fn load(folder: &Path, outcome: &str, purpose: LoadPurpose) -> Result<Content, LoadReport> {
    load_content(
        Arc::new(FilesystemFolder(folder.to_path_buf())),
        outcome,
        purpose,
    )
}
