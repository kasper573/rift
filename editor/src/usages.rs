use std::collections::HashMap;

use engine::core::content::id::LeafKind;
use engine::core::content::locate::unescape;
use engine::core::content::{FolderReader, Reference};
use engine::systems::registry;
use rayon::prelude::*;
use serde_json::Value;

use crate::project::ProjectFiles;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentUsage {
    pub file: String,
    pub pointer: String,
    pub key: bool,
    pub known: bool,
}

impl ContentUsage {
    /// The path its file names here as `file` holds it now, which differs from the path it was
    /// found naming once the file changed since.
    pub(crate) fn named_in(&self, file: &Value) -> Option<String> {
        if !self.key {
            return file.pointer(&self.pointer)?.as_str().map(str::to_owned);
        }
        let (map, key) = self.pointer.rsplit_once('/')?;
        let key = unescape(key);
        file.pointer(map)?
            .as_object()?
            .contains_key(&key)
            .then_some(key)
    }
}

#[derive(Default)]
pub struct ContentUsages {
    content: HashMap<String, Vec<ContentUsage>>,
    media: HashMap<String, Vec<ContentUsage>>,
    by_file: HashMap<String, Vec<Reference>>,
}

impl ContentUsages {
    pub fn build(reader: &FolderReader, files: &ProjectFiles) -> ContentUsages {
        let registry = registry();
        let naming: Vec<&String> = files
            .paths()
            .filter(|path| registry.names_content(path))
            .collect();
        let parsed: Vec<(String, Vec<Reference>)> = naming
            .par_iter()
            .filter_map(|path| {
                let text = files.text(path)?;
                Some(((*path).clone(), registry.references(reader, path, &text)))
            })
            .collect();
        let mut usages = ContentUsages::default();
        for (file, references) in parsed {
            for reference in &references {
                let usage = ContentUsage {
                    file: file.clone(),
                    pointer: reference.pointer.clone(),
                    key: reference.key,
                    known: reference.known,
                };
                let named = match reference.kind {
                    LeafKind::Row(_) => &mut usages.content,
                    LeafKind::Asset => &mut usages.media,
                };
                named
                    .entry(reference.value.clone())
                    .or_default()
                    .push(usage);
            }
            usages.by_file.insert(file, references);
        }
        usages
    }

    /// Every content file named anywhere, by its path, with where it is named.
    pub fn content(&self) -> impl Iterator<Item = (&str, &[ContentUsage])> {
        self.content
            .iter()
            .map(|(path, usages)| (path.as_str(), usages.as_slice()))
    }

    /// Every media path named anywhere, a file's or a folder's, with where it is named.
    pub fn media(&self) -> impl Iterator<Item = (&str, &[ContentUsage])> {
        self.media
            .iter()
            .map(|(path, usages)| (path.as_str(), usages.as_slice()))
    }

    pub(crate) fn of_content(&self, path: &str) -> &[ContentUsage] {
        self.content.get(path).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn of_media(&self, path: &str) -> &[ContentUsage] {
        self.media.get(path).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn in_file(&self, file: &str) -> &[Reference] {
        self.by_file.get(file).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn files_naming(&self, path: &str) -> Vec<(String, Vec<String>)> {
        let mut by_file: Vec<(String, Vec<String>)> = Vec::new();
        for usage in self.of_content(path) {
            match by_file.iter_mut().find(|(file, _)| *file == usage.file) {
                Some((_, pointers)) => pointers.push(usage.pointer.clone()),
                None => by_file.push((usage.file.clone(), vec![usage.pointer.clone()])),
            }
        }
        by_file
    }
}
