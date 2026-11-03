//! A game folder as the tools read and write it: through the loader's reader, so every file is
//! read and written as the editor does.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use engine::core::content::{
    Content, ContentFile, FilesystemFolder, FolderReader, LoadPurpose, LoadReport, WrittenFiles,
};
use engine::systems::{load_content, registry};

pub fn reader_of(folder: &Path) -> FolderReader {
    FolderReader::new(registry(), Arc::new(FilesystemFolder(folder.to_path_buf())))
}

/// The file at `path` as it reads, or the count of problems that keep it from reading.
pub fn read<Row: ContentFile>(reader: &FolderReader, path: &str) -> Result<Row, String> {
    reader
        .read(path)
        .map_err(|errors| format!("{path} doesn't read: {} problem(s)", errors.len()))
}

/// The game in `folder` loaded for `purpose`, `outcome` naming what the load is for.
pub fn load(folder: &Path, outcome: &str, purpose: LoadPurpose) -> Result<Content, LoadReport> {
    load_content(
        Arc::new(FilesystemFolder(folder.to_path_buf())),
        outcome,
        purpose,
    )
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
