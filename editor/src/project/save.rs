use std::fs::File;
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::time::Duration;

use walkdir::WalkDir;

use engine::core::content::parent_of;

use super::files::{FileBytes, OnDisk, ProjectFiles, folder_entry, is_folder};
use crate::busy::Progress;

/// The files of the game folder at `root`, to be written where they differ from disk.
pub struct Saving {
    pub(super) root: std::path::PathBuf,
    pub(super) files: ProjectFiles,
}

/// The files as saving left them, and what it wrote.
pub struct Saved {
    pub(super) files: ProjectFiles,
    pub(super) outcome: Result<SaveOutcome, SaveError>,
}

impl Saving {
    /// Writes every file that differs from disk, each counted to `progress`.
    pub fn write(self, progress: &Progress) -> Saved {
        let Saving { root, mut files } = self;
        let outcome = save(&root, &mut files, progress);
        Saved { files, outcome }
    }
}

pub struct SaveOutcome {
    pub written: Vec<String>,
    pub removed: Vec<String>,
}

pub struct SaveError {
    pub file: String,
    pub reason: String,
    pub done: SaveOutcome,
}

fn save(
    root: &Path,
    files: &mut ProjectFiles,
    progress: &Progress,
) -> Result<SaveOutcome, SaveError> {
    let mut done = SaveOutcome {
        written: Vec::new(),
        removed: Vec::new(),
    };
    let (writes, removals): (Vec<String>, Vec<String>) = files
        .changed()
        .cloned()
        .partition(|path| files.contains(path));
    progress.doing("Writing files", Some(writes.len() + removals.len()));
    for path in writes {
        progress.one_more();
        if !files.differs_from_disk(&path) {
            files.saved(&path, files.on_disk(&path));
            continue;
        }
        let bytes = files.get(&path).expect("an entry of the files");
        let written = if is_folder(&path) {
            std::fs::create_dir_all(root.join(&path)).map_err(|error| error.to_string())
        } else {
            write_atomic(root, &path, &bytes)
        };
        if let Err(reason) = written {
            return Err(SaveError {
                file: path,
                reason,
                done,
            });
        }
        files.saved(&path, OnDisk::at(&root.join(&path)));
        done.written.push(path);
    }
    for path in removals {
        progress.one_more();
        if files.on_disk(&path).is_none() {
            files.saved(&path, None);
            continue;
        }
        let removed = if is_folder(&path) {
            Ok(())
        } else {
            std::fs::remove_file(root.join(&path))
        };
        match removed {
            Ok(()) => remove_emptied_folders(root, files, &path),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                remove_emptied_folders(root, files, &path)
            }
            Err(error) => {
                return Err(SaveError {
                    file: path,
                    reason: error.to_string(),
                    done,
                });
            }
        }
        files.saved(&path, None);
        done.removed.push(path);
    }
    Ok(done)
}

/// Removes the folders above `path` that the files no longer hold anything in, as the editor
/// shows them.
fn remove_emptied_folders(root: &Path, files: &ProjectFiles, path: &str) {
    let mut folder = match path.strip_suffix('/') {
        Some(folder) => folder,
        None => parent_of(path),
    };
    while !folder.is_empty() {
        let kept = files.paths_under(&folder_entry(folder)).next().is_some();
        if kept || std::fs::remove_dir(root.join(folder)).is_err() {
            return;
        }
        folder = parent_of(folder);
    }
}

pub(crate) fn clean_leftovers(root: &Path) {
    let leftovers = WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with('.') && name.ends_with(".tmp"))
        });
    for leftover in leftovers {
        let _ = std::fs::remove_file(leftover.path());
    }
}

fn write_atomic(root: &Path, file: &str, bytes: &FileBytes) -> Result<(), String> {
    let path = root.join(file);
    let parent = path.parent().ok_or("no parent folder")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("no file name")?;
    let temp = parent.join(format!(".{name}.tmp"));
    let written = (|| {
        let mut out = File::create(&temp)?;
        out.write_all(bytes)?;
        out.sync_all()
    })();
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(error.to_string());
    }
    let mut last = None;
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(50));
        }
        match std::fs::rename(&temp, &path) {
            Ok(()) => return Ok(()),
            Err(error) => last = Some(error),
        }
    }
    let _ = std::fs::remove_file(&temp);
    Err(last.map_or_else(|| "rename failed".to_owned(), |error| error.to_string()))
}
