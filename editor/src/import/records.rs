use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::tileset::{TileBlock, TileGridSettings};
use crate::project::watch::{FolderChange, FolderWatcher};
use crate::user::UserFolder;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportRecord {
    pub files: Vec<PathBuf>,
    pub table: String,
    pub key: String,
    pub mapping: ImportMapping,
    pub size: u64,
    pub hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ImportMapping {
    Grid { grid: TileGridSettings },
    Animation { animation: TileBlock },
}

impl ImportRecord {
    pub fn new(files: &[PathBuf], table: &str, key: &str, mapping: ImportMapping) -> ImportRecord {
        let (size, hash) = fingerprint(files);
        ImportRecord {
            files: files.to_vec(),
            table: table.to_owned(),
            key: key.to_owned(),
            mapping,
            size,
            hash,
        }
    }

    pub fn refreshed(&self) -> ImportRecord {
        let (size, hash) = fingerprint(&self.files);
        ImportRecord {
            size,
            hash,
            ..self.clone()
        }
    }

    pub fn block(&self) -> Option<TileBlock> {
        match self.mapping {
            ImportMapping::Animation { animation } => Some(animation),
            _ => None,
        }
    }

    pub fn missing(&self) -> bool {
        self.files.iter().any(|file| !file.exists())
    }

    pub fn changed(&self) -> Option<bool> {
        if self.missing() {
            return None;
        }
        let (size, hash) = fingerprint(&self.files);
        Some(size != self.size || hash != self.hash)
    }

    fn is_for(&self, table: &str, key: &str, block: Option<TileBlock>) -> bool {
        self.table == table && self.key == key && self.block() == block
    }
}

pub const RETRIES: u32 = 3;

#[derive(Clone, Debug)]
pub struct DueReimport {
    pub table: String,
    pub key: String,
    pub block: Option<TileBlock>,
    pub at: Instant,
    pub tries: u32,
    pub while_closed: bool,
}

impl DueReimport {
    fn of(record: &ImportRecord, while_closed: bool) -> DueReimport {
        DueReimport {
            table: record.table.clone(),
            key: record.key.clone(),
            block: record.block(),
            at: Instant::now(),
            tries: 0,
            while_closed,
        }
    }
}

#[derive(Default)]
pub struct ImportRecords {
    pub records: Vec<ImportRecord>,
    file: Option<PathBuf>,
    watchers: HashMap<PathBuf, FolderWatcher>,
    pub due: Vec<DueReimport>,
    pub unreadable: Option<String>,
}

impl ImportRecords {
    pub fn load(user: &UserFolder, folder: &Path) -> ImportRecords {
        let file = user
            .imports_dir()
            .join(format!("{}.json", UserFolder::folder_key(folder)));
        let (records, unreadable): (Vec<ImportRecord>, Option<String>) = match std::fs::read(&file)
        {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(records) => (records, None),
                Err(error) => (Vec::new(), Some(format!("{}: {error}", file.display()))),
            },
            Err(_) => (Vec::new(), None),
        };
        let due = records
            .iter()
            .filter(|record| record.changed() == Some(true))
            .map(|record| DueReimport::of(record, true))
            .collect();
        let mut imports = ImportRecords {
            records,
            file: unreadable.is_none().then_some(file),
            watchers: HashMap::new(),
            due,
            unreadable,
        };
        imports.watch();
        imports
    }

    pub fn record_for(&self, table: &str, key: &str) -> Option<&ImportRecord> {
        self.record_with(table, key, None)
    }

    pub fn record_with(
        &self,
        table: &str,
        key: &str,
        block: Option<TileBlock>,
    ) -> Option<&ImportRecord> {
        self.records
            .iter()
            .rev()
            .find(|record| record.is_for(table, key, block))
    }

    pub fn forget(&mut self, table: &str, key: &str) {
        self.records
            .retain(|seen| !(seen.table == table && seen.key == key));
        self.save();
    }

    /// Keeps the records of what moved, at where it went.
    pub fn moved(&mut self, moved: &dyn Fn(&str) -> Option<String>) {
        let mut changed = false;
        for record in &mut self.records {
            if let Some(to) = moved(&record.key) {
                record.key = to;
                changed = true;
            }
        }
        if changed {
            self.save();
        }
    }

    pub fn remember(&mut self, record: ImportRecord) {
        self.records
            .retain(|seen| !seen.is_for(&record.table, &record.key, record.block()));
        self.records.push(record);
        self.save();
        self.watch();
    }

    fn save(&self) {
        if let Some(file) = &self.file
            && let Some(parent) = file.parent()
            && std::fs::create_dir_all(parent).is_ok()
            && let Ok(bytes) = serde_json::to_vec_pretty(&self.records)
        {
            let _ = std::fs::write(file, bytes);
        }
    }

    fn watch(&mut self) {
        let dirs: Vec<PathBuf> = self
            .records
            .iter()
            .flat_map(|record| &record.files)
            .filter_map(|file| file.parent().map(Path::to_path_buf))
            .collect();
        for dir in dirs {
            if !self.watchers.contains_key(&dir) && dir.is_dir() {
                self.watchers.insert(dir.clone(), FolderWatcher::on(&dir));
            }
        }
    }

    pub fn poll(&mut self) -> Vec<DueReimport> {
        let mut touched: Vec<PathBuf> = Vec::new();
        for (dir, watcher) in &mut self.watchers {
            match watcher.poll() {
                Some(FolderChange::Files(paths)) => {
                    touched.extend(paths.into_iter().map(|path| dir.join(path)));
                }
                Some(FolderChange::Unknown) => touched.push(dir.clone()),
                None => {}
            }
        }
        for record in &self.records {
            let hit = touched.iter().any(|path| {
                record
                    .files
                    .iter()
                    .any(|file| file == path || file.parent() == Some(path.as_path()))
            });
            if hit
                && record.changed() == Some(true)
                && !self
                    .due
                    .iter()
                    .any(|due| record.is_for(&due.table, &due.key, due.block))
            {
                self.due.push(DueReimport::of(record, false));
            }
        }
        let now = Instant::now();
        let (ready, waiting): (Vec<DueReimport>, Vec<DueReimport>) =
            self.due.drain(..).partition(|due| due.at <= now);
        self.due = waiting;
        ready
    }

    pub fn retry(&mut self, due: DueReimport) -> bool {
        if due.tries + 1 >= RETRIES {
            return false;
        }
        self.due.push(DueReimport {
            at: Instant::now() + Duration::from_millis(500),
            tries: due.tries + 1,
            ..due
        });
        true
    }
}

fn fingerprint(files: &[PathBuf]) -> (u64, String) {
    let mut hasher = blake3::Hasher::new();
    let mut size = 0;
    for file in files {
        if let Ok(bytes) = std::fs::read(file) {
            size += bytes.len() as u64;
            hasher.update(&bytes);
        }
    }
    (size, hasher.finalize().to_hex()[..16].to_owned())
}
