use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::ops::Bound;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, OnceLock};

use engine::core::content::files::{self, GameFile};
use engine::core::content::{FileHash, MemoryFolder, SETTINGS_FILE, table_of};
use engine::systems::registry;
use walkdir::WalkDir;

pub type FileBytes = Arc<[u8]>;

#[derive(Clone, Default)]
pub struct ProjectFiles {
    map: BTreeMap<String, Stored>,
    index: OnceLock<Arc<FolderIndex>>,
    revision: u64,
    paths_revision: u64,
}

/// The hash of every file and kept folder as it stands on disk, as the editor last read or wrote
/// it.
#[derive(Clone, Default)]
pub struct DiskHashes {
    map: BTreeMap<String, String>,
    revision: u64,
}

impl DiskHashes {
    pub fn get(&self, path: &str) -> Option<&str> {
        self.map.get(path).map(String::as_str)
    }

    pub fn paths(&self) -> impl Iterator<Item = &String> {
        self.map.keys()
    }

    /// Every path that starts with `prefix`.
    pub fn under<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = &'a String> {
        self.map
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(path, _)| path)
            .take_while(move |path| path.starts_with(prefix))
    }

    pub fn insert(&mut self, path: String, hash: String) {
        self.revision = next_revision();
        self.map.insert(path, hash);
    }

    pub fn remove(&mut self, path: &str) {
        self.revision = next_revision();
        self.map.remove(path);
    }

    /// Which change the hashes are at, as [`ProjectFiles::revision`] is for the files.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

/// A revision no files or hashes have had before, so no two different states share one.
fn next_revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// What the files of the game folder are, as the loader sorts them by name and place.
#[derive(Default)]
pub struct FolderIndex {
    /// Every content file's path, by its type.
    pub content: BTreeMap<String, BTreeSet<String>>,
    /// Every folder that isn't a bundle, at any depth, empty or not.
    pub folders: BTreeSet<String>,
    pub media: BTreeSet<String>,
    pub notes: BTreeSet<String>,
    /// Files the game never reads, such as a seller's readme or the archive a pack came in.
    pub other: BTreeSet<String>,
    /// Files and folders whose names say something the game can't hold.
    pub unknown: BTreeSet<String>,
    /// What each folder holds directly, its folders, files and bundles, by path; the game folder
    /// is "".
    pub children: BTreeMap<String, BTreeSet<String>>,
    /// How many files each folder holds at any depth, a bundle counting as one.
    pub counts: BTreeMap<String, usize>,
}

impl FolderIndex {
    pub fn paths_of(&self, table: &str) -> impl Iterator<Item = &str> {
        self.content
            .get(table)
            .into_iter()
            .flatten()
            .map(String::as_str)
    }

    pub fn children_of(&self, folder: &str) -> impl Iterator<Item = &str> {
        self.children
            .get(folder)
            .into_iter()
            .flatten()
            .map(String::as_str)
    }

    pub fn count_of(&self, folder: &str) -> usize {
        self.counts.get(folder).copied().unwrap_or(0)
    }

    pub fn is_content(&self, path: &str) -> bool {
        table_of(path).is_some_and(|table| {
            self.content
                .get(table)
                .is_some_and(|paths| paths.contains(path))
        })
    }
}

#[derive(Clone)]
struct Stored {
    bytes: FileBytes,
    hash: String,
    json: OnceLock<Option<Arc<serde_json::Value>>>,
}

impl ProjectFiles {
    pub fn read_dir(root: &Path) -> io::Result<ProjectFiles> {
        let mut files = ProjectFiles::default();
        for file in content_files(root)? {
            let bytes = if is_folder(&file) {
                FileBytes::from([])
            } else {
                FileBytes::from(std::fs::read(root.join(&file))?)
            };
            files.insert(file, bytes);
        }
        Ok(files)
    }

    pub fn get(&self, path: &str) -> Option<&FileBytes> {
        self.map.get(path).map(|stored| &stored.bytes)
    }

    pub fn hash_of(&self, path: &str) -> Option<&str> {
        self.map.get(path).map(|stored| stored.hash.as_str())
    }

    pub fn contains(&self, path: &str) -> bool {
        self.map.contains_key(path)
    }

    pub fn insert(&mut self, path: String, bytes: FileBytes) {
        self.revision = next_revision();
        let hash = hash(&bytes);
        let stored = Stored {
            bytes,
            hash,
            json: OnceLock::new(),
        };
        if self.map.insert(path, stored).is_none() {
            self.paths_changed();
        }
    }

    pub fn remove(&mut self, path: &str) {
        self.revision = next_revision();
        if self.map.remove(path).is_some() {
            self.paths_changed();
        }
    }

    /// Which change the files are at: files at the same revision hold the same, whichever copy
    /// they are, so what is worked out from them keeps while it stays.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Which paths the files are at: files at the same paths revision hold the same paths,
    /// whatever bytes they hold, so what is worked out from the paths alone keeps while it stays.
    pub fn paths_revision(&self) -> u64 {
        self.paths_revision
    }

    fn paths_changed(&mut self) {
        self.index = OnceLock::new();
        self.paths_revision = next_revision();
    }

    pub fn index(&self) -> Arc<FolderIndex> {
        self.index
            .get_or_init(|| Arc::new(FolderIndex::of(self.map.keys())))
            .clone()
    }

    pub fn paths_of(&self, table: &str) -> Vec<String> {
        self.index().paths_of(table).map(str::to_owned).collect()
    }

    /// Every file's path, leaving out the folders kept as entries.
    pub fn paths(&self) -> impl Iterator<Item = &String> {
        self.map.keys().filter(|path| !is_folder(path))
    }

    /// Every file and every kept folder, as saving and history see them.
    pub fn entries(&self) -> impl Iterator<Item = &String> {
        self.map.keys()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &FileBytes)> {
        self.map
            .iter()
            .filter(|(path, _)| !is_folder(path))
            .map(|(path, stored)| (path, &stored.bytes))
    }

    /// The hash of every file, as the files stand now.
    pub fn hashes(&self) -> DiskHashes {
        DiskHashes {
            map: self
                .map
                .iter()
                .map(|(path, stored)| (path.clone(), stored.hash.clone()))
                .collect(),
            revision: next_revision(),
        }
    }

    pub fn text(&self, path: &str) -> Option<String> {
        String::from_utf8(self.get(path)?.to_vec()).ok()
    }

    pub fn json(&self, path: &str) -> Option<Arc<serde_json::Value>> {
        let stored = self.map.get(path)?;
        stored
            .json
            .get_or_init(|| serde_json::from_slice(&stored.bytes).ok().map(Arc::new))
            .clone()
    }

    pub fn json_owned(&self, path: &str) -> Option<serde_json::Value> {
        self.json(path).map(|value| (*value).clone())
    }

    pub fn folder(&self, name: &str) -> MemoryFolder {
        MemoryFolder {
            name: name.to_owned(),
            files: self
                .iter()
                .map(|(path, bytes)| (path.clone(), bytes.clone()))
                .collect(),
        }
    }

    pub fn with(&self, changes: &[FileChange]) -> ProjectFiles {
        let mut files = self.clone();
        for change in changes {
            match &change.bytes {
                Some(bytes) => files.insert(change.file.clone(), bytes.clone()),
                None => files.remove(&change.file),
            }
        }
        files
    }

    /// Every file that makes up what is at `path`: a file itself, or a bundle's every part.
    pub fn files_of(&self, path: &str) -> Vec<String> {
        if self.map.contains_key(path) {
            vec![path.to_owned()]
        } else {
            self.paths_under(&folder_entry(path)).cloned().collect()
        }
    }

    pub fn paths_under(&self, prefix: &str) -> impl Iterator<Item = &String> {
        self.map
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(path, _)| path)
            .take_while(move |path| path.starts_with(prefix))
    }
}

/// The bundle `file` is a part of, and its path inside the bundle, when it is one.
pub fn bundle_of(file: &str) -> Option<(&str, &str)> {
    static BUNDLES: LazyLock<Vec<&'static str>> = LazyLock::new(|| registry().bundle_tables());
    files::bundle_of(file, &BUNDLES)
}

/// The path of the content file that `file` is, or is a part of; `None` for the settings.
pub fn content_holding(file: &str) -> Option<&str> {
    match bundle_of(file) {
        Some((bundle, _)) => Some(bundle),
        None => (file != SETTINGS_FILE && json_table(file)).then_some(file),
    }
}

/// Whether `file` is named as a file of a table the game keeps in a JSON file.
fn json_table(file: &str) -> bool {
    static TABLES: LazyLock<Vec<&'static str>> = LazyLock::new(|| registry().table_names());
    file.ends_with(".json") && table_of(file).is_some_and(|table| TABLES.contains(&table))
}

/// The type and path of the content file that `file` is, or is a part of.
pub fn content_of(file: &str) -> Option<(&str, &str)> {
    let path = content_holding(file)?;
    Some((table_of(path)?, path))
}

/// Whether `path`, an entry of the files, is a folder kept as one: a path ending in a slash.
pub fn is_folder(path: &str) -> bool {
    path.ends_with('/')
}

/// The entry that keeps `folder` while nothing is in it.
pub fn folder_entry(folder: &str) -> String {
    format!("{folder}/")
}

impl FolderIndex {
    fn of<'a>(entries: impl Iterator<Item = &'a String>) -> FolderIndex {
        let registry = registry();
        let types = registry.file_types();
        let mut index = FolderIndex::default();
        let add_folders = |index: &mut FolderIndex, path: &str| {
            let mut child = path;
            loop {
                let parent = files::parent_of(child);
                index
                    .children
                    .entry(parent.to_owned())
                    .or_default()
                    .insert(child.to_owned());
                if parent.is_empty() || !index.folders.insert(parent.to_owned()) {
                    break;
                }
                child = parent;
            }
        };
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for entry in entries {
            if let Some(folder) = entry.strip_suffix('/') {
                if bundle_of(entry).is_none() {
                    index.folders.insert(folder.to_owned());
                    add_folders(&mut index, folder);
                }
                continue;
            }
            let (path, dir) = match bundle_of(entry) {
                Some((bundle, _)) => (bundle, true),
                None => (entry.as_str(), false),
            };
            add_folders(&mut index, path);
            if seen.insert(path) {
                let mut parent = path;
                while !parent.is_empty() {
                    parent = files::parent_of(parent);
                    *index.counts.entry(parent.to_owned()).or_default() += 1;
                }
            }
            match files::classify(path, dir, &types) {
                Ok(Some(GameFile::Content { table, .. })) => {
                    index
                        .content
                        .entry(table.to_owned())
                        .or_default()
                        .insert(path.to_owned());
                }
                Ok(Some(GameFile::Media(_))) => {
                    index.media.insert(path.to_owned());
                }
                Ok(Some(GameFile::Note)) => {
                    index.notes.insert(path.to_owned());
                }
                Ok(Some(GameFile::Other)) => {
                    index.other.insert(path.to_owned());
                }
                Ok(Some(GameFile::Settings)) => {}
                Ok(None) => {}
                Err(_) => {
                    index.unknown.insert(path.to_owned());
                }
            }
        }
        index
    }
}

pub fn hash(bytes: &[u8]) -> String {
    FileHash::of(bytes).hash
}

pub fn json_bytes(value: &serde_json::Value) -> FileBytes {
    let mut bytes = serde_json::to_vec_pretty(value).expect("json");
    bytes.push(b'\n');
    FileBytes::from(bytes)
}

pub fn text_bytes(text: String) -> FileBytes {
    FileBytes::from(text.into_bytes())
}

#[derive(Clone, Debug)]
pub struct FileChange {
    pub file: String,
    pub bytes: Option<FileBytes>,
}

impl PartialEq for FileChange {
    fn eq(&self, other: &FileChange) -> bool {
        self.file == other.file
            && match (&self.bytes, &other.bytes) {
                (Some(a), Some(b)) => a[..] == b[..],
                (None, None) => true,
                _ => false,
            }
    }
}

/// The folder-relative names of the files under `root` that make up its content, and of every
/// folder there that holds nothing, as its entry.
pub fn content_files(root: &Path) -> io::Result<Vec<String>> {
    let mut files = Vec::new();
    let entries = WalkDir::new(root)
        .min_depth(1)
        .follow_links(true)
        .into_iter()
        .filter_entry(|entry| entry.file_name().to_str().is_none_or(|name| !hidden(name)));
    for entry in entries {
        let entry = entry?;
        let file = folder_file(root, entry.path()).expect("a walked file lies under its root");
        if entry.file_type().is_file() {
            files.push(file);
        } else if empty_folder(entry.path())? {
            files.push(folder_entry(&file));
        }
    }
    Ok(files)
}

pub fn empty_folder(path: &Path) -> io::Result<bool> {
    Ok(std::fs::read_dir(path)?
        .filter_map(Result::ok)
        .all(|entry| entry.file_name().to_str().is_some_and(hidden)))
}

/// Whether a file or folder of that name is kept out of the game folder's content: dotfiles, and
/// the temporary files saving writes.
pub fn hidden(name: &str) -> bool {
    name.starts_with('.') || name.ends_with(".tmp")
}

/// The folder-relative, slash-separated name of a file under `root`.
pub fn folder_file(root: &Path, path: &Path) -> Option<String> {
    let parts: Vec<_> = path
        .strip_prefix(root)
        .ok()?
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect();
    Some(parts.join("/"))
}
