use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use engine::core::content::{
    Content, ContentFile, ContentId, FolderReader, ReadRow, SETTINGS_FILE, Unreadable,
};
use engine::systems::{self, GameSettings};

use super::files::ProjectFiles;

/// The project's files read as the types the game reads them as, each read only when asked for and
/// again only once its files change or the folder's paths do: everything the editor knows of a
/// file's fields comes from its type, so a field the game no longer has is one the editor can no
/// longer name.
#[derive(Default)]
pub(super) struct TypedFiles {
    at: Mutex<Option<Typed>>,
}

struct Typed {
    revision: u64,
    paths_revision: u64,
    reader: Arc<FolderReader>,
    read: HashMap<(TypeId, String), Kept>,
    /// Reads under way in the background, each until it is taken in as read.
    reading: HashMap<(TypeId, String), Arc<OnceLock<Read>>>,
}

type Read = Result<Arc<dyn Any + Send + Sync>, Arc<Unreadable>>;

/// A read, and the hash of every file it was read from.
struct Kept {
    read: Read,
    from: Vec<(String, String)>,
}

impl Kept {
    fn holds(&self, files: &ProjectFiles) -> bool {
        self.from
            .iter()
            .all(|(file, hash)| files.hash_of(file) == Some(hash.as_str()))
    }
}

impl TypedFiles {
    pub(super) fn reader(&self, files: &ProjectFiles, name: &str) -> Arc<FolderReader> {
        self.with(files, name, |typed| typed.reader.clone())
    }

    pub(super) fn read<Row: ContentFile>(
        &self,
        files: &ProjectFiles,
        name: &str,
        path: &str,
    ) -> Result<Arc<Row>, Arc<Unreadable>> {
        self.read_as(files, name, path, |reader| reader.read::<Row>(path))
    }

    /// The file at `path` as the game reads it once it has been read: the first ask reads it in
    /// the background, so a large file never holds up a frame, and the asks after it find it read.
    pub(super) fn read_soon<Row: ContentFile>(
        &self,
        files: &ProjectFiles,
        name: &str,
        path: &str,
    ) -> Option<Result<Arc<Row>, Arc<Unreadable>>> {
        let read = self.with(files, name, |typed| {
            let key = (TypeId::of::<Row>(), path.to_owned());
            if let Some(kept) = typed.read.get(&key) {
                return Some(kept.read.clone());
            }
            let Some(reading) = typed.reading.get(&key) else {
                let slot = Arc::new(OnceLock::new());
                let (reader, path, filled) = (typed.reader.clone(), path.to_owned(), slot.clone());
                super::background().spawn(move || {
                    let _ = filled.set(kept_read(reader.read::<Row>(&path)));
                });
                typed.reading.insert(key, slot);
                return None;
            };
            let read = reading.get()?.clone();
            typed.reading.remove(&key);
            typed.read.insert(
                key,
                Kept {
                    read: read.clone(),
                    from: hashes_of(files, path),
                },
            );
            Some(read)
        })?;
        Some(read.map(|row| row.downcast::<Row>().expect("read as its own type")))
    }

    pub(super) fn settings(
        &self,
        files: &ProjectFiles,
        name: &str,
    ) -> Result<Arc<GameSettings>, Arc<Unreadable>> {
        self.read_as(
            files,
            name,
            SETTINGS_FILE,
            FolderReader::settings::<GameSettings>,
        )
    }

    fn read_as<T: Any + Send + Sync>(
        &self,
        files: &ProjectFiles,
        name: &str,
        path: &str,
        read: impl FnOnce(&FolderReader) -> Result<T, Unreadable>,
    ) -> Result<Arc<T>, Arc<Unreadable>> {
        self.with(files, name, |typed| {
            let key = (TypeId::of::<T>(), path.to_owned());
            let reader = typed.reader.clone();
            typed
                .read
                .entry(key)
                .or_insert_with(|| Kept {
                    read: kept_read(read(&reader)),
                    from: hashes_of(files, path),
                })
                .read
                .clone()
                .map(|row| row.downcast::<T>().expect("read as its own type"))
        })
    }

    /// Keeps `rows`, read whole from `files` by a check, as the reads of their files.
    pub(super) fn keep(&self, files: &ProjectFiles, name: &str, rows: Vec<ReadRow>) {
        self.with(files, name, |typed| {
            for row in rows {
                let from = hashes_of(files, &row.path);
                typed.read.entry((row.row_type, row.path)).or_insert(Kept {
                    read: Ok(row.row),
                    from,
                });
            }
        })
    }

    fn with<R>(&self, files: &ProjectFiles, name: &str, f: impl FnOnce(&mut Typed) -> R) -> R {
        let mut at = self
            .at
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let typed = match at.take() {
            Some(typed) if typed.revision == files.revision() => at.insert(typed),
            last => {
                let folder = Arc::new(files.folder(name));
                let (reader, mut read) = match last {
                    Some(last) if last.paths_revision == files.paths_revision() => {
                        (last.reader.rereading(folder), last.read)
                    }
                    _ => (
                        FolderReader::new(systems::making_registry(), folder),
                        HashMap::new(),
                    ),
                };
                read.retain(|_, kept| kept.holds(files));
                at.insert(Typed {
                    revision: files.revision(),
                    paths_revision: files.paths_revision(),
                    reader: Arc::new(reader),
                    read,
                    reading: HashMap::new(),
                })
            }
        };
        f(typed)
    }
}

/// Where the ids a value holds lead, by the paths of the scope they were read in: the files as they
/// stand now for what the editor reads of them, the content loaded last for what it built. An id
/// means a file only among the paths it was read against.
pub trait IdPaths {
    fn path_of<Row: ContentFile>(&self, id: ContentId<Row>) -> Option<&str>;
}

impl IdPaths for FolderReader {
    fn path_of<Row: ContentFile>(&self, id: ContentId<Row>) -> Option<&str> {
        FolderReader::path_of(self, id)
    }
}

impl IdPaths for Content {
    fn path_of<Row: ContentFile>(&self, id: ContentId<Row>) -> Option<&str> {
        self.table::<Row>()
            .paths()
            .get(id.index())
            .map(String::as_str)
    }
}

fn kept_read<T: Any + Send + Sync>(read: Result<T, Unreadable>) -> Read {
    match read {
        Ok(row) => Ok(Arc::new(row)),
        Err(errors) => Err(Arc::new(errors)),
    }
}

/// The hash of every file the content at `path` is read from.
fn hashes_of(files: &ProjectFiles, path: &str) -> Vec<(String, String)> {
    files
        .files_of(path)
        .into_iter()
        .filter_map(|file| Some((file.clone(), files.hash_of(&file)?.to_owned())))
        .collect()
}
