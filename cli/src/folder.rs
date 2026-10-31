//! A game folder as the tools read and write it: through the loader's reader, so every file is
//! read and written as the editor does.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use engine::core::content::{FilesystemFolder, FolderReader, WrittenFiles};
use engine::systems::registry;

pub fn reader_of(folder: &Path) -> FolderReader {
    FolderReader::new(registry(), Arc::new(FilesystemFolder(folder.to_path_buf())))
}

/// Lands what a write produced in the folder at `into`.
pub fn write(into: &Path, written: std::io::Result<WrittenFiles>) -> Result<(), String> {
    let written = written.map_err(|error| error.to_string())?;
    for (path, bytes) in written.files {
        let file = into.join(path);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&file, bytes).map_err(|error| format!("{}: {error}", file.display()))?;
    }
    for path in written.removed {
        let _ = fs::remove_file(into.join(path));
    }
    Ok(())
}
