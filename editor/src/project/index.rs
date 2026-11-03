use std::sync::LazyLock;

use engine::core::content::files::{self, FileType, GameFile};
use engine::core::content::table_of;
use engine::systems::registry;
use imbl::{OrdMap, OrdSet};

use super::files::bundle_of;

/// What the files of the game folder are, as the loader sorts them by name and place, kept up to
/// date path by path as files come and go, so no change costs more than the paths it touches.
#[derive(Clone, Default)]
pub struct FolderIndex {
    /// Every content file's path, by its type.
    pub content: OrdMap<String, OrdSet<String>>,
    /// Every folder that isn't a bundle, at any depth, empty or not.
    pub folders: OrdSet<String>,
    pub media: OrdSet<String>,
    pub notes: OrdSet<String>,
    /// Files the game never reads, such as a seller's readme or the archive a pack came in.
    pub other: OrdSet<String>,
    /// Files and folders whose names say something the game can't hold.
    pub unknown: OrdSet<String>,
    /// What each folder holds directly, its folders, files and bundles, by path; the game folder
    /// is "".
    pub children: OrdMap<String, OrdSet<String>>,
    /// How many files each folder holds at any depth, a bundle counting as one.
    pub counts: OrdMap<String, usize>,
    /// How many entries each file, bundle and folder stands for: itself, every part of a bundle,
    /// everything a folder holds. A node is there while it stands for any.
    held: OrdMap<String, usize>,
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

    pub(crate) fn count_of(&self, folder: &str) -> usize {
        self.counts.get(folder).copied().unwrap_or(0)
    }

    pub fn is_content(&self, path: &str) -> bool {
        table_of(path).is_some_and(|table| {
            self.content
                .get(table)
                .is_some_and(|paths| paths.contains(path))
        })
    }

    /// Takes in `entry`, a file or a kept folder's entry, as one more of the files.
    pub(super) fn add(&mut self, entry: &str) {
        let Some(node) = node_of(entry) else {
            return;
        };
        for at in node.lineage() {
            let held = self.held.entry(at.path.to_owned()).or_insert(0);
            *held += 1;
            if *held == 1 {
                self.appear(&at);
            }
        }
    }

    /// Lets go of `entry`, a file or a kept folder's entry, as one of the files no more.
    pub(super) fn remove(&mut self, entry: &str) {
        let Some(node) = node_of(entry) else {
            return;
        };
        for at in node.lineage() {
            let Some(held) = self.held.get_mut(at.path) else {
                return;
            };
            *held -= 1;
            if *held == 0 {
                self.held.remove(at.path);
                self.disappear(&at);
            }
        }
    }

    fn appear(&mut self, node: &Node) {
        if !node.path.is_empty() {
            self.children
                .entry(files::parent_of(node.path).to_owned())
                .or_default()
                .insert(node.path.to_owned());
        }
        match node.kind {
            NodeKind::Folder => {
                if !node.path.is_empty() {
                    self.folders.insert(node.path.to_owned());
                }
            }
            NodeKind::File { dir } => {
                for folder in ancestors(node.path) {
                    *self.counts.entry(folder.to_owned()).or_insert(0) += 1;
                }
                if let Some(class) = class_of(node.path, dir) {
                    self.set_of(class).insert(node.path.to_owned());
                }
            }
        }
    }

    fn disappear(&mut self, node: &Node) {
        if !node.path.is_empty() {
            let parent = files::parent_of(node.path);
            if let Some(children) = self.children.get_mut(parent) {
                children.remove(node.path);
                if children.is_empty() {
                    self.children.remove(parent);
                }
            }
        }
        match node.kind {
            NodeKind::Folder => {
                self.folders.remove(node.path);
            }
            NodeKind::File { dir } => {
                for folder in ancestors(node.path) {
                    if let Some(count) = self.counts.get_mut(folder) {
                        *count -= 1;
                        if *count == 0 {
                            self.counts.remove(folder);
                        }
                    }
                }
                if let Some(class) = class_of(node.path, dir) {
                    self.set_of(class).remove(node.path);
                    if let Class::Content(table) = class
                        && self.content.get(table).is_some_and(OrdSet::is_empty)
                    {
                        self.content.remove(table);
                    }
                }
            }
        }
    }

    fn set_of(&mut self, class: Class) -> &mut OrdSet<String> {
        match class {
            Class::Content(table) => self.content.entry(table.to_owned()).or_default(),
            Class::Media => &mut self.media,
            Class::Note => &mut self.notes,
            Class::Other => &mut self.other,
            Class::Unknown => &mut self.unknown,
        }
    }
}

/// Which set of the index a file or bundle is sorted into.
#[derive(Clone, Copy)]
enum Class {
    Content(&'static str),
    Media,
    Note,
    Other,
    Unknown,
}

/// The set a file or bundle at `path` is sorted into, none for the settings.
fn class_of(path: &str, dir: bool) -> Option<Class> {
    static TYPES: LazyLock<Vec<FileType>> = LazyLock::new(|| registry().file_types());
    match files::classify(path, dir, &TYPES) {
        Ok(Some(GameFile::Content { table, .. })) => Some(Class::Content(table)),
        Ok(Some(GameFile::Media(_))) => Some(Class::Media),
        Ok(Some(GameFile::Note)) => Some(Class::Note),
        Ok(Some(GameFile::Other)) => Some(Class::Other),
        Ok(Some(GameFile::Settings) | None) => None,
        Err(_) => Some(Class::Unknown),
    }
}

/// A file, bundle or folder of the index.
struct Node<'a> {
    path: &'a str,
    kind: NodeKind,
}

#[derive(Clone, Copy)]
enum NodeKind {
    /// A file, or a bundle when `dir`.
    File {
        dir: bool,
    },
    Folder,
}

impl<'a> Node<'a> {
    /// The node itself, then every folder above it up to the game folder's own.
    fn lineage(self) -> impl Iterator<Item = Node<'a>> {
        let above = ancestors(self.path).map(|path| Node {
            path,
            kind: NodeKind::Folder,
        });
        std::iter::once(self).chain(above)
    }
}

/// The node `entry` stands for: a file, the bundle it is a part of, or the folder a kept folder's
/// entry keeps; none for a kept folder inside a bundle.
fn node_of(entry: &str) -> Option<Node<'_>> {
    if let Some(folder) = entry.strip_suffix('/') {
        return bundle_of(entry).is_none().then_some(Node {
            path: folder,
            kind: NodeKind::Folder,
        });
    }
    Some(match bundle_of(entry) {
        Some((bundle, _)) => Node {
            path: bundle,
            kind: NodeKind::File { dir: true },
        },
        None => Node {
            path: entry,
            kind: NodeKind::File { dir: false },
        },
    })
}

/// Every folder above `path`, nearest first, ending with the game folder's own, "".
fn ancestors(path: &str) -> impl Iterator<Item = &str> {
    let mut at = Some(path);
    std::iter::from_fn(move || {
        let parent = files::parent_of(at?);
        at = (!parent.is_empty()).then_some(parent);
        Some(parent)
    })
}
