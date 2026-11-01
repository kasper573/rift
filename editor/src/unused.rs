use std::collections::{BTreeMap, BTreeSet, VecDeque};

use engine::systems::registry;

use crate::project::files::content_holding;
use crate::project::{ContentEdit, Project, ProjectFiles};
use crate::usages::ContentUsages;

/// What nothing the game uses reaches. The game uses its fixtures, which code names, the content
/// its settings name, every file of a type picked from as a whole, and whatever those name, at any
/// depth. Every other content file and media file is unused.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnusedContent {
    rows: BTreeMap<String, UnusedRow>,
    assets: BTreeSet<String>,
}

/// One thing found unused, as picked in the dialog that deletes them.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum UnusedItem {
    Row(String),
    Asset(String),
}

/// Why a content file is unused: nothing names it, or only files that are unused themselves do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnusedRow {
    Unnamed,
    NamedByUnused(Vec<String>),
}

impl UnusedContent {
    pub fn find(files: &ProjectFiles, usages: &ContentUsages) -> UnusedContent {
        let registry = registry();
        let mut names: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        let mut named_by: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        let mut roots: Vec<&str> = Vec::new();
        let mut used_assets: BTreeSet<&str> = BTreeSet::new();
        for (row, uses) in usages.content() {
            for usage in uses {
                match content_holding(&usage.file) {
                    Some(reader) => {
                        names.entry(reader).or_default().insert(row);
                        named_by.entry(row).or_default().insert(reader);
                    }
                    None => roots.push(row),
                }
            }
        }
        let mut asset_readers: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (path, uses) in usages.media() {
            for usage in uses {
                match content_holding(&usage.file) {
                    Some(reader) => asset_readers.entry(reader).or_default().push(path),
                    None => {
                        used_assets.insert(path);
                    }
                }
            }
        }
        let index = files.index();
        let every_row: Vec<(&str, &str)> = index
            .content
            .iter()
            .flat_map(|(table, paths)| {
                paths
                    .iter()
                    .map(move |path| (table.as_str(), path.as_str()))
            })
            .collect();
        roots.extend(
            every_row
                .iter()
                .filter(|(table, path)| {
                    registry.is_fixture(path) || registry.picked_from_table(table)
                })
                .map(|(_, path)| *path),
        );
        let mut used: BTreeSet<&str> = BTreeSet::new();
        let mut pending: VecDeque<&str> = roots.into();
        while let Some(row) = pending.pop_front() {
            if !used.insert(row) {
                continue;
            }
            used_assets.extend(asset_readers.get(row).into_iter().flatten());
            pending.extend(names.get(row).into_iter().flatten());
        }
        let rows = every_row
            .into_iter()
            .filter(|(_, row)| !used.contains(row))
            .map(|(_, row)| {
                let why = match named_by.get(row) {
                    Some(readers) if !readers.is_empty() => {
                        UnusedRow::NamedByUnused(readers.iter().map(|r| (*r).to_owned()).collect())
                    }
                    _ => UnusedRow::Unnamed,
                };
                (row.to_owned(), why)
            })
            .collect();
        let assets = index
            .media
            .iter()
            .filter(|path| !named_within(path, &used_assets))
            .cloned()
            .collect();
        UnusedContent { rows, assets }
    }

    pub fn row(&self, path: &str) -> Option<&UnusedRow> {
        self.rows.get(path)
    }

    pub fn rows(&self) -> impl Iterator<Item = (&String, &UnusedRow)> {
        self.rows.iter()
    }

    /// Whether the media file at `path` is unused.
    pub(crate) fn has_asset(&self, path: &str) -> bool {
        self.assets.contains(path)
    }

    pub fn assets(&self) -> impl Iterator<Item = &str> {
        self.assets.iter().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.assets.is_empty()
    }
}

/// What deleting unused content takes: the content files picked, and the media.
pub struct UnusedDeletion {
    pub edit: ContentEdit,
    pub rows: Vec<String>,
    pub assets: usize,
}

/// Deletes the content files `rows` and the media `assets` in one edit. Refused when something
/// left behind still names one of them.
pub fn delete(
    project: &Project,
    rows: &[String],
    assets: &[String],
) -> Result<UnusedDeletion, String> {
    let gone: BTreeSet<&str> = rows.iter().map(String::as_str).collect();
    let left_naming = |uses: &[crate::usages::ContentUsage]| {
        uses.iter()
            .find_map(|usage| match content_holding(&usage.file) {
                Some(reader) if gone.contains(reader) => None,
                Some(reader) => Some(reader.to_owned()),
                None => Some("the game's settings".to_owned()),
            })
    };
    for row in &gone {
        if let Some(reader) = left_naming(project.usages.of_content(row)) {
            return Err(format!(
                "{reader} still names {row}: tick it too, or keep {row}"
            ));
        }
    }
    for asset in assets {
        if let Some(reader) = left_naming(project.usages.of_media(asset)) {
            return Err(format!(
                "{reader} still names {asset}: tick it too, or keep the file"
            ));
        }
    }
    let mut label = format!(
        "Delete {} unused file{}",
        rows.len(),
        if rows.len() == 1 { "" } else { "s" }
    );
    if !assets.is_empty() {
        label.push_str(&format!(
            " and {} media file{}",
            assets.len(),
            if assets.len() == 1 { "" } else { "s" }
        ));
    }
    let mut edit = ContentEdit::new(label);
    for row in &gone {
        for file in project.files.files_of(row) {
            edit.remove(file);
        }
    }
    for asset in assets {
        edit.remove(asset.clone());
    }
    Ok(UnusedDeletion {
        edit,
        rows: rows.to_vec(),
        assets: assets.len(),
    })
}

/// Whether the file at `path` is named, itself or through a folder holding it.
fn named_within(path: &str, named: &BTreeSet<&str>) -> bool {
    let mut at = path;
    loop {
        if named.contains(at) {
            return true;
        }
        match at.rsplit_once('/') {
            Some((folder, _)) => at = folder,
            None => return false,
        }
    }
}
