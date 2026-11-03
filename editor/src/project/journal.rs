use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use base64::Engine;
use engine::core::time::UnixMillis;
use serde::{Deserialize, Serialize};

use super::files::{FileBytes, FileChange};
use crate::user::UserFolder;

const EVERY: Duration = Duration::from_secs(3);

/// Where the unsaved files of a folder are kept as they are edited, so a session that ends without
/// saving them can bring them back. It is written on a thread of its own, in the order asked, so
/// keeping it never holds up a frame.
pub struct RecoveryJournal {
    last: Option<Instant>,
    /// Which change of the files the journal was last asked to keep.
    kept: Option<JournalRevision>,
    writer: Option<(Sender<JournalWork>, JoinHandle<()>)>,
}

/// A change of the files and of what the editor knows of the disk, as [`RecoveryJournal::due`]
/// tells them apart.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct JournalRevision {
    pub files: u64,
    pub disk: u64,
}

enum JournalWork {
    Write(Box<dyn FnOnce() -> RecoveryJournalData + Send>),
    Clear,
}

#[derive(Serialize, Deserialize)]
pub struct RecoveryJournalData {
    pub build: String,
    pub content_version: String,
    pub folder: PathBuf,
    pub written: UnixMillis,
    pub files: Vec<RecoveryJournalFile>,
}

#[derive(Serialize, Deserialize)]
pub struct RecoveryJournalFile {
    pub path: String,
    pub base: Option<String>,
    pub bytes: Option<String>,
}

impl RecoveryJournal {
    pub fn for_folder(user: &UserFolder, folder: &Path) -> RecoveryJournal {
        let _ = std::fs::create_dir_all(user.journal_dir());
        let (sender, work) = mpsc::channel();
        let written = file_for(user, folder);
        let writer = std::thread::spawn(move || {
            for work in work {
                match work {
                    JournalWork::Write(data) => write_journal(&written, &data()),
                    JournalWork::Clear => {
                        let _ = std::fs::remove_file(&written);
                    }
                }
            }
        });
        RecoveryJournal {
            last: None,
            kept: None,
            writer: Some((sender, writer)),
        }
    }

    /// Whether the journal is due to keep the files as they stand at `revision`: they changed
    /// since it last kept them, and it hasn't kept them for a while.
    pub fn due(&self, revision: JournalRevision) -> bool {
        self.kept != Some(revision) && self.last.is_none_or(|last| last.elapsed() >= EVERY)
    }

    /// Keeps the files `data` makes, as they stand at `revision`.
    pub fn write(
        &mut self,
        revision: JournalRevision,
        data: impl FnOnce() -> RecoveryJournalData + Send + 'static,
    ) {
        self.kept = Some(revision);
        self.send(JournalWork::Write(Box::new(data)));
    }

    pub fn clear(&mut self) {
        self.kept = None;
        self.send(JournalWork::Clear);
    }

    fn send(&mut self, work: JournalWork) {
        self.last = Some(Instant::now());
        if let Some((sender, _)) = &self.writer {
            let _ = sender.send(work);
        }
    }

    pub fn recover(user: &UserFolder, folder: &Path) -> Option<RecoveryJournalData> {
        let text = std::fs::read_to_string(file_for(user, folder)).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Moves the folder's journal out of the way of the next one, keeping it beside it. Gives
    /// where it went.
    pub(crate) fn set_aside(user: &UserFolder, folder: &Path) -> Option<PathBuf> {
        let file = file_for(user, folder);
        let aside = file.with_extension("json.old");
        std::fs::rename(&file, &aside).ok().map(|()| aside)
    }
}

impl RecoveryJournalFile {
    pub fn of(path: &str, base: Option<&str>, bytes: Option<&FileBytes>) -> RecoveryJournalFile {
        RecoveryJournalFile {
            path: path.to_owned(),
            base: base.map(str::to_owned),
            bytes: bytes.map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes)),
        }
    }

    /// Whether the file on disk is no longer the one the journaled bytes were edited from, given
    /// the hash of the one there now.
    pub(crate) fn changed_on_disk(&self, now: Option<&str>) -> bool {
        self.base.as_deref() != now
    }

    pub fn change(&self) -> FileChange {
        FileChange {
            file: self.path.clone(),
            bytes: self.bytes.as_ref().and_then(|text| {
                base64::engine::general_purpose::STANDARD
                    .decode(text)
                    .ok()
                    .map(FileBytes::from)
            }),
        }
    }
}

// What was asked of the journal is on disk once it is let go of.
impl Drop for RecoveryJournal {
    fn drop(&mut self) {
        if let Some((sender, writer)) = self.writer.take() {
            drop(sender);
            let _ = writer.join();
        }
    }
}

fn write_journal(file: &Path, data: &RecoveryJournalData) {
    if let Ok(text) = serde_json::to_string(data) {
        let temp = file.with_extension("json.tmp");
        if std::fs::write(&temp, text).is_ok() {
            let _ = std::fs::rename(&temp, file);
        }
    }
}

fn file_for(user: &UserFolder, folder: &Path) -> PathBuf {
    user.journal_dir()
        .join(format!("{}.json", UserFolder::folder_key(folder)))
}
