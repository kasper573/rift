use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use notify::event::{AccessKind, AccessMode, ModifyKind};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

use super::files;
use crate::inbox::ChannelInbox;

pub struct FolderWatcher {
    root: PathBuf,
    _watcher: Option<RecommendedWatcher>,
    events: ChannelInbox<notify::Result<notify::Event>>,
    pending: Vec<PathBuf>,
    quiet_since: Option<Instant>,
    lost: bool,
}

pub const QUIET: Duration = Duration::from_millis(500);

/// What changed in a watched folder, once it has kept quiet a moment.
pub enum FolderChange {
    /// These files, relative to the folder.
    Files(Vec<String>),
    /// Anything: the watcher lost track of what.
    Unknown,
}

impl FolderWatcher {
    pub fn on(root: &Path) -> FolderWatcher {
        let (sender, events) = channel();
        let watcher = notify::recommended_watcher(move |event| {
            let _ = sender.send(event);
        })
        .ok()
        .and_then(|mut watcher| {
            watcher.watch(root, RecursiveMode::Recursive).ok()?;
            Some(watcher)
        });
        FolderWatcher {
            root: root.to_path_buf(),
            _watcher: watcher,
            events: ChannelInbox::new(events),
            pending: Vec::new(),
            quiet_since: None,
            lost: false,
        }
    }

    pub fn poll(&mut self) -> Option<FolderChange> {
        for event in self.events.drain() {
            match event {
                Ok(event) if event.need_rescan() => self.lost = true,
                Ok(event) if !changes_content(&event.kind) => continue,
                Ok(event) => self.pending.extend(event.paths),
                Err(_) => self.lost = true,
            }
            self.quiet_since = Some(Instant::now());
        }
        let quiet = self.quiet_since?;
        if quiet.elapsed() < QUIET {
            return None;
        }
        self.quiet_since = None;
        if self.lost {
            self.lost = false;
            self.pending.clear();
            return Some(FolderChange::Unknown);
        }
        let mut paths: Vec<String> = self
            .pending
            .drain(..)
            .filter_map(|path| relative(&self.root, &path))
            .collect();
        paths.sort();
        paths.dedup();
        (!paths.is_empty()).then_some(FolderChange::Files(paths))
    }
}

/// Whether an event can change what a file holds or where it is. Opening and reading a file is
/// no change, and must not count as one: taking a change in reads the files it names.
fn changes_content(kind: &EventKind) -> bool {
    match kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) | EventKind::Modify(ModifyKind::Metadata(_)) => false,
        _ => true,
    }
}

fn relative(root: &Path, path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    (!files::hidden(name)).then(|| files::folder_file(root, path))?
}
