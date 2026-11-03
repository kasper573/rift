use std::collections::HashSet;
use std::fmt;
use std::io;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock, PoisonError};
use std::time::SystemTime;

use engine::core::content::files;
use engine::core::content::{
    ContentFolder, FileHash, FolderEntry, SETTINGS_FILE, entries_in, table_of,
};
use engine::systems::registry;
use imbl::ordmap::DiffItem;
use imbl::{OrdMap, OrdSet};
use quick_cache::sync::Cache;
use quick_cache::{Options, OptionsBuilder, Weighter};
use rayon::prelude::*;

pub use super::index::FolderIndex;

pub type FileBytes = Arc<[u8]>;

/// The game folder's files as the editor works on them: the folder on disk, read only as each file
/// is asked for, under the edits not yet saved. A copy costs nothing and a change costs what it
/// touches, however many files the game holds.
#[derive(Clone)]
pub struct ProjectFiles {
    root: Arc<PathBuf>,
    entries: OrdMap<String, Entry>,
    disk: OrdMap<String, OnDisk>,
    /// Every path whose entry may differ from what is on disk.
    changed: OrdSet<String>,
    index: Arc<FolderIndex>,
    cache: Arc<DiskCache>,
    revision: u64,
    paths_revision: u64,
    disk_revision: u64,
}

/// Which version of a file an entry holds: two entries with the same stamp hold the same bytes, so
/// what is worked out from a file keeps while its stamp stays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileStamp(u64);

impl fmt::Display for FileStamp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:x}", self.0)
    }
}

/// What stands on disk at a path, as the editor last listed, read or wrote it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OnDisk {
    File(DiskFile),
    /// A folder kept while nothing is in it.
    Folder,
}

/// A file on disk, by what tells a change to it apart without reading it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DiskFile {
    len: u64,
    modified: Option<SystemTime>,
}

impl DiskFile {
    fn of(metadata: &std::fs::Metadata) -> DiskFile {
        DiskFile {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        }
    }
}

impl OnDisk {
    /// What stands at `path` on disk now, none when nothing does or it can't be told.
    pub fn at(path: &Path) -> Option<OnDisk> {
        let metadata = std::fs::metadata(path).ok()?;
        if metadata.is_dir() {
            empty_folder(path)
                .unwrap_or(false)
                .then_some(OnDisk::Folder)
        } else {
            Some(OnDisk::File(DiskFile::of(&metadata)))
        }
    }
}

#[derive(Clone)]
enum Entry {
    /// The file as it stands on disk, read only when asked for.
    OnDisk {
        stamp: FileStamp,
        file: DiskFile,
    },
    Edited(Arc<Edited>),
    /// A folder kept while nothing is in it.
    Folder,
}

/// Bytes an edit put at a path.
struct Edited {
    stamp: FileStamp,
    bytes: FileBytes,
    hash: OnceLock<String>,
    json: OnceLock<Option<Arc<serde_json::Value>>>,
}

impl Edited {
    fn hash(&self) -> &str {
        self.hash.get_or_init(|| hash(&self.bytes))
    }
}

/// No files, of no folder: what a value made for no game reads.
impl Default for ProjectFiles {
    fn default() -> ProjectFiles {
        ProjectFiles::empty(Path::new(""))
    }
}

impl ProjectFiles {
    /// The files of the game folder at `root` as they stand on disk, listed but not read; each
    /// file listed so far is counted to `progress`.
    pub fn open(root: &Path, progress: &(dyn Fn(usize) + Sync)) -> io::Result<ProjectFiles> {
        let listed = list(root, progress)?;
        let mut files = ProjectFiles::empty(root);
        let mut index = FolderIndex::default();
        for (path, _) in &listed {
            index.add(path);
        }
        files.index = Arc::new(index);
        files.disk = listed.iter().cloned().collect();
        files.entries = listed
            .into_iter()
            .map(|(path, on_disk)| (path, Entry::from_disk(on_disk)))
            .collect();
        Ok(files)
    }

    fn empty(root: &Path) -> ProjectFiles {
        ProjectFiles {
            root: Arc::new(root.to_path_buf()),
            entries: OrdMap::new(),
            disk: OrdMap::new(),
            changed: OrdSet::new(),
            index: Arc::default(),
            cache: Arc::new(DiskCache::new()),
            revision: next_revision(),
            paths_revision: next_revision(),
            disk_revision: next_revision(),
        }
    }

    /// The bytes of the file or kept folder at `path`, as the files stand now.
    pub fn get(&self, path: &str) -> Option<FileBytes> {
        match self.entries.get(path)? {
            Entry::Edited(edited) => Some(edited.bytes.clone()),
            Entry::Folder => Some(FileBytes::from([])),
            Entry::OnDisk { stamp, .. } => self.read_disk(path, *stamp),
        }
    }

    /// Which version of the file at `path` the files hold.
    pub fn stamp_of(&self, path: &str) -> Option<FileStamp> {
        self.entries.get(path).map(Entry::stamp)
    }

    /// The hash of the bytes of the file at `path`, as the loader hashes them: read and hashed the
    /// first time it is asked for, then kept.
    pub fn hash_of(&self, path: &str) -> Option<String> {
        match self.entries.get(path)? {
            Entry::Edited(edited) => Some(edited.hash().to_owned()),
            Entry::Folder => Some(hash(&[])),
            Entry::OnDisk { file, .. } => self.disk_file_hash(path, *file),
        }
    }

    pub fn contains(&self, path: &str) -> bool {
        self.entries.contains_key(path)
    }

    pub fn insert(&mut self, path: String, bytes: FileBytes) {
        let edited = Entry::Edited(Arc::new(Edited {
            stamp: FileStamp(next_revision()),
            bytes,
            hash: OnceLock::new(),
            json: OnceLock::new(),
        }));
        let entry = if is_folder(&path) {
            Entry::Folder
        } else {
            edited
        };
        self.put(path, Some(entry));
    }

    pub fn remove(&mut self, path: &str) {
        self.put(path.to_owned(), None);
    }

    fn put(&mut self, path: String, entry: Option<Entry>) {
        self.revision = next_revision();
        self.changed.insert(path.clone());
        let held = match entry {
            Some(entry) => self.entries.insert(path.clone(), entry).is_some(),
            None => self.entries.remove(&path).is_none(),
        };
        if !held {
            let index = Arc::make_mut(&mut self.index);
            if self.entries.contains_key(&path) {
                index.add(&path);
            } else {
                index.remove(&path);
            }
            self.paths_revision = next_revision();
        }
    }

    /// Which change the files are at: files at the same revision hold the same, whichever copy
    /// they are, so what is worked out from them keeps while it stays.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Which paths the files are at: files at the same paths revision hold the same paths,
    /// whatever bytes they hold, so what is worked out from the paths alone keeps while it stays.
    pub(crate) fn paths_revision(&self) -> u64 {
        self.paths_revision
    }

    /// Which change what the editor knows of the disk is at, as [`ProjectFiles::revision`] is for
    /// the files.
    pub(crate) fn disk_revision(&self) -> u64 {
        self.disk_revision
    }

    pub fn index(&self) -> Arc<FolderIndex> {
        self.index.clone()
    }

    pub fn paths_of(&self, table: &str) -> Vec<String> {
        self.index.paths_of(table).map(str::to_owned).collect()
    }

    /// Every file's path, leaving out the folders kept as entries.
    pub fn paths(&self) -> impl Iterator<Item = &String> {
        self.entries.keys().filter(|path| !is_folder(path))
    }

    /// Every file and every kept folder, as saving and history see them.
    pub fn entries(&self) -> impl Iterator<Item = &String> {
        self.entries.keys()
    }

    pub fn text(&self, path: &str) -> Option<String> {
        String::from_utf8(self.get(path)?.to_vec()).ok()
    }

    pub fn json(&self, path: &str) -> Option<Arc<serde_json::Value>> {
        match self.entries.get(path)? {
            Entry::Edited(edited) => edited
                .json
                .get_or_init(|| parse_json(&edited.bytes))
                .clone(),
            Entry::Folder => None,
            Entry::OnDisk { stamp, .. } => {
                let bytes = self.read_disk(path, *stamp)?;
                self.cache
                    .json
                    .get_or_insert_with(stamp, || {
                        Ok::<_, ()>(ParsedJson(bytes.len(), parse_json(&bytes)))
                    })
                    .ok()?
                    .1
            }
        }
    }

    /// Whether the JSON of the file at `path` is at hand, so asking for it parses nothing: the
    /// first ask for a file not yet parsed parses it in the background, so a large file never
    /// holds up a frame.
    pub fn json_at_hand(&self, path: &str) -> bool {
        let stamp = match self.entries.get(path) {
            Some(Entry::Edited(edited)) if edited.json.get().is_none() => edited.stamp,
            Some(Entry::OnDisk { stamp, .. }) if !self.cache.json.contains_key(stamp) => *stamp,
            _ => return true,
        };
        let parsing = &self.cache.parsing;
        if parsing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(stamp)
        {
            let (files, path) = (self.clone(), path.to_owned());
            super::background().spawn(move || {
                files.json(&path);
                files
                    .cache
                    .parsing
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&stamp);
            });
        }
        false
    }

    pub fn json_owned(&self, path: &str) -> Option<serde_json::Value> {
        self.json(path).map(|value| (*value).clone())
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
        if self.entries.contains_key(path) {
            vec![path.to_owned()]
        } else {
            self.paths_under(&folder_entry(path)).cloned().collect()
        }
    }

    pub(crate) fn paths_under(&self, prefix: &str) -> impl Iterator<Item = &String> {
        self.entries
            .range::<_, str>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(path, _)| path)
            .take_while(move |path| path.starts_with(prefix))
    }

    /// Every file and kept folder whose bytes may differ from those `before` holds: an edit, or a
    /// file taken from disk. It costs what changed, however many files are alike.
    pub fn changed_since(&self, before: &ProjectFiles) -> Vec<String> {
        before
            .entries
            .diff(&self.entries)
            .map(|change| match change {
                DiffItem::Add(path, _) | DiffItem::Remove(path, _) => path.clone(),
                DiffItem::Update { new: (path, _), .. } => path.clone(),
            })
            .collect()
    }

    /// Every file and kept folder that differs from disk, by path.
    pub fn unsaved(&self) -> Vec<String> {
        self.changed
            .iter()
            .filter(|path| self.differs_from_disk(path))
            .cloned()
            .collect()
    }

    /// Whether the file or kept folder at `path` differs from what is on disk.
    pub fn differs_from_disk(&self, path: &str) -> bool {
        match (self.entries.get(path), self.disk.get(path)) {
            (None, None) | (Some(Entry::Folder), Some(OnDisk::Folder)) => false,
            (Some(Entry::OnDisk { file, .. }), Some(OnDisk::File(disk))) => file != disk,
            (Some(Entry::Edited(edited)), Some(OnDisk::File(disk))) => {
                self.disk_file_hash(path, *disk).as_deref() != Some(edited.hash())
            }
            _ => true,
        }
    }

    /// The hash of the file at `path` as it stands on disk, as the editor last saw it.
    pub(crate) fn disk_hash(&self, path: &str) -> Option<String> {
        match self.disk.get(path)? {
            OnDisk::File(file) => self.disk_file_hash(path, *file),
            OnDisk::Folder => Some(hash(&[])),
        }
    }

    /// What the editor last saw on disk at `path`.
    pub(crate) fn on_disk(&self, path: &str) -> Option<OnDisk> {
        self.disk.get(path).copied()
    }

    /// Every path the disk held under `prefix`, as the editor last saw it.
    pub(crate) fn on_disk_under<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = &'a String> {
        self.disk
            .range::<_, str>((Bound::Included(prefix), Bound::Unbounded))
            .map(|(path, _)| path)
            .take_while(move |path| path.starts_with(prefix))
    }

    /// Every path whose entry may differ from disk: what saving goes through.
    pub(crate) fn changed(&self) -> impl Iterator<Item = &String> {
        self.changed.iter()
    }

    /// Takes in that the entry at `path` now stands on disk as `now`, written there by saving: the
    /// bytes it holds are read from disk again once they are let go of.
    pub(crate) fn saved(&mut self, path: &str, now: Option<OnDisk>) {
        self.disk_revision = next_revision();
        self.changed.remove(path);
        match now {
            Some(now) => self.disk.insert(path.to_owned(), now),
            None => self.disk.remove(path),
        };
        if let (Some(Entry::Edited(edited)), Some(OnDisk::File(file))) =
            (self.entries.get(path), now)
        {
            let edited = edited.clone();
            self.cache.bytes.insert(edited.stamp, edited.bytes.clone());
            self.cache
                .hashes
                .insert((path.to_owned(), file), edited.hash().to_owned());
            self.entries.insert(
                path.to_owned(),
                Entry::OnDisk {
                    stamp: edited.stamp,
                    file,
                },
            );
        }
    }

    /// Takes in what stands on disk at `path` now in place of the entry there.
    pub(crate) fn take_from_disk(&mut self, path: &str, now: Option<OnDisk>) {
        let entry = now.map(Entry::from_disk);
        self.put(path.to_owned(), entry);
        self.saved(path, now);
    }

    /// Takes in what stands on disk at `path` now, keeping the entry there as the editor has it.
    pub(crate) fn disk_moved_on(&mut self, path: &str, now: Option<OnDisk>) {
        self.disk_revision = next_revision();
        self.changed.insert(path.to_owned());
        match now {
            Some(now) => self.disk.insert(path.to_owned(), now),
            None => self.disk.remove(path),
        };
    }

    fn read_disk(&self, path: &str, stamp: FileStamp) -> Option<FileBytes> {
        self.cache
            .bytes
            .get_or_insert_with(&stamp, || {
                std::fs::read(self.root.join(path)).map(FileBytes::from)
            })
            .ok()
    }

    fn disk_file_hash(&self, path: &str, file: DiskFile) -> Option<String> {
        self.cache
            .hashes
            .get_or_insert_with(&(path.to_owned(), file), || {
                std::fs::read(self.root.join(path)).map(|bytes| hash(&bytes))
            })
            .ok()
    }
}

impl ContentFolder for ProjectFiles {
    fn name(&self) -> String {
        self.root.display().to_string()
    }

    fn entries(&self, dir: &str) -> Vec<FolderEntry> {
        entries_in(dir, |from| {
            self.entries
                .range::<_, str>((Bound::Included(from), Bound::Unbounded))
                .find(|(path, _)| !is_folder(path))
                .map(|(path, _)| path.clone())
        })
    }

    fn read(&self, file: &str) -> io::Result<Vec<u8>> {
        self.get(file)
            .filter(|_| !is_folder(file))
            .map(|bytes| bytes.to_vec())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("missing {file}")))
    }
}

/// Entries alike hold the same bytes; entries unalike may still hold the same.
impl PartialEq for Entry {
    fn eq(&self, other: &Entry) -> bool {
        self.stamp() == other.stamp()
    }
}

impl Entry {
    fn stamp(&self) -> FileStamp {
        match self {
            Entry::OnDisk { stamp, .. } => *stamp,
            Entry::Edited(edited) => edited.stamp,
            Entry::Folder => FileStamp(0),
        }
    }

    fn from_disk(on_disk: OnDisk) -> Entry {
        match on_disk {
            OnDisk::File(file) => Entry::OnDisk {
                stamp: FileStamp(next_revision()),
                file,
            },
            OnDisk::Folder => Entry::Folder,
        }
    }
}

/// What the files read from disk and work out from it, kept within a budget and shared by every
/// copy of the files: a stamp names the same bytes in all of them.
struct DiskCache {
    bytes: Cache<FileStamp, FileBytes, ByteWeight>,
    json: Cache<FileStamp, ParsedJson, ByteWeight>,
    /// The hash of each file read from disk, by its path and how it stood there.
    hashes: Cache<(String, DiskFile), String>,
    /// The files whose JSON is being parsed in the background.
    parsing: Mutex<HashSet<FileStamp>>,
}

impl DiskCache {
    fn new() -> DiskCache {
        DiskCache {
            bytes: Cache::with_options(
                weighed(),
                ByteWeight,
                Default::default(),
                Default::default(),
            ),
            json: Cache::with_options(
                weighed(),
                ByteWeight,
                Default::default(),
                Default::default(),
            ),
            hashes: Cache::new(ENTRIES_CACHED),
            parsing: Mutex::default(),
        }
    }
}

/// How many bytes of files read from disk, and of the JSON parsed from them by the size of its
/// text, the files keep at most.
const BYTES_CACHED: u64 = 512 << 20;
const ENTRIES_CACHED: usize = 1 << 16;

/// How a cache weighed by bytes holds up to [`BYTES_CACHED`] of them. A cache admits nothing
/// heavier than one shard's share of its weight, so it keeps to few shards, each big enough for a
/// whole scene of a big area.
fn weighed() -> Options {
    OptionsBuilder::new()
        .estimated_items_capacity(ENTRIES_CACHED)
        .weight_capacity(BYTES_CACHED)
        .shards(WEIGHED_SHARDS)
        .build()
        .expect("the options set every capacity")
}

const WEIGHED_SHARDS: usize = 4;

/// A file's JSON, with the size of the text it was parsed from.
#[derive(Clone)]
struct ParsedJson(usize, Option<Arc<serde_json::Value>>);

#[derive(Clone)]
struct ByteWeight;

impl Weighter<FileStamp, FileBytes> for ByteWeight {
    fn weight(&self, _: &FileStamp, bytes: &FileBytes) -> u64 {
        bytes.len() as u64 + 1
    }
}

impl Weighter<FileStamp, ParsedJson> for ByteWeight {
    fn weight(&self, _: &FileStamp, parsed: &ParsedJson) -> u64 {
        parsed.0 as u64 + 1
    }
}

fn parse_json(bytes: &[u8]) -> Option<Arc<serde_json::Value>> {
    serde_json::from_slice(bytes).ok().map(Arc::new)
}

/// A revision no files or hashes have had before, so no two different states share one.
fn next_revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The bundle `file` is a part of, and its path inside the bundle, when it is one.
pub fn bundle_of(file: &str) -> Option<(&str, &str)> {
    static BUNDLES: LazyLock<Vec<&'static str>> = LazyLock::new(|| registry().bundle_tables());
    files::bundle_of(file, &BUNDLES)
}

/// The path of the content file that `file` is, or is a part of; `None` for the settings.
pub(crate) fn content_holding(file: &str) -> Option<&str> {
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
pub(crate) fn folder_entry(folder: &str) -> String {
    format!("{folder}/")
}

pub fn hash(bytes: &[u8]) -> String {
    FileHash::of(bytes).hash
}

pub fn json_bytes(value: &serde_json::Value) -> FileBytes {
    let mut bytes = serde_json::to_vec_pretty(value).expect("json");
    bytes.push(b'\n');
    FileBytes::from(bytes)
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

/// Every file under `root` that makes up its content, and every folder there that holds nothing,
/// as its entry, each with how it stands on disk, sorted: listed across threads without reading
/// a file, each file counted to `progress` as it is.
pub(crate) fn list(
    root: &Path,
    progress: &(dyn Fn(usize) + Sync),
) -> io::Result<Vec<(String, OnDisk)>> {
    let walk = jwalk::WalkDir::new(root)
        .follow_links(true)
        .skip_hidden(false)
        .process_read_dir(|_, _, _, children| {
            children.retain(|child| {
                child.as_ref().map_or(true, |child| {
                    child.file_name().to_str().is_none_or(|name| !hidden(name))
                })
            });
        });
    let mut found: Vec<(String, bool)> = Vec::new();
    for entry in walk {
        let entry = entry.map_err(io::Error::other)?;
        if entry.depth() == 0 {
            continue;
        }
        let path = entry.path();
        let file = folder_file(root, &path).expect("a walked file lies under its root");
        found.push((file, entry.file_type().is_dir()));
    }
    found.sort_unstable();
    let listed = AtomicUsize::new(0);
    let mut out: Vec<(String, OnDisk)> = found
        .par_iter()
        .enumerate()
        .filter_map(|(at, (file, dir))| {
            if *dir {
                let holds = found
                    .get(at + 1)
                    .is_some_and(|(next, _)| next.starts_with(&folder_entry(file)));
                return (!holds).then(|| Ok((folder_entry(file), OnDisk::Folder)));
            }
            let metadata = std::fs::metadata(root.join(file));
            progress(listed.fetch_add(1, Ordering::Relaxed) + 1);
            Some(metadata.map(|metadata| (file.clone(), OnDisk::File(DiskFile::of(&metadata)))))
        })
        .collect::<io::Result<_>>()?;
    out.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

pub(crate) fn empty_folder(path: &Path) -> io::Result<bool> {
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
pub(crate) fn folder_file(root: &Path, path: &Path) -> Option<String> {
    let parts: Vec<_> = path
        .strip_prefix(root)
        .ok()?
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect();
    Some(parts.join("/"))
}
