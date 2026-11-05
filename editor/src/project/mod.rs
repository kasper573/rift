pub mod files;
mod gltfs;
pub mod history;
mod index;
pub mod journal;
pub mod lock;
pub mod save;
mod typed;
pub mod watch;

pub use files::{FileBytes, FileChange, FileStamp, OnDisk, ProjectFiles, hash, json_bytes};
pub use gltfs::GltfCache;
pub use history::{EditHistory, EditStep, RowSelection};
pub use typed::IdPaths;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use engine::core::content::ErrorPlace::WholeFile;
use engine::core::content::{
    CheckedFiles, Content, ContentFile, ContentId, ContentNames, ContentProblem, FileHashes,
    FolderReader, LoadError, LoadPurpose, LoadReport, MISSING_FILE, NamedContent, ReadRow,
    SETTINGS_FILE, Unreadable, parent_of, table_of,
};
use engine::systems::{self, GameSettings};

use self::files::{Vouched, content_holding};
use self::watch::FolderChange;
use crate::busy::Progress;
use crate::inbox::ChannelInbox;
use crate::schema::SchemaShapes;
use crate::unused::UnusedContent;
use crate::usages::ContentUsages;
use crate::views::script::nodes::NodeCatalogue;

#[derive(Clone, Debug, Default)]
pub struct ContentEdit {
    pub label: String,
    pub changes: Vec<FileChange>,
    pub merge: bool,
    /// The label of the last step this edit joins, taking its own label, when that step is still
    /// the last one made.
    pub amends: Option<String>,
}

impl ContentEdit {
    pub fn new(label: impl Into<String>) -> ContentEdit {
        ContentEdit {
            label: label.into(),
            changes: Vec::new(),
            merge: false,
            amends: None,
        }
    }

    pub fn merging(mut self) -> ContentEdit {
        self.merge = true;
        self
    }

    pub(crate) fn amending(mut self, step: &str) -> ContentEdit {
        self.amends = Some(step.to_owned());
        self
    }

    pub fn write(&mut self, file: impl Into<String>, bytes: FileBytes) -> &mut ContentEdit {
        let file = file.into();
        self.changes.retain(|change| change.file != file);
        self.changes.push(FileChange {
            file,
            bytes: Some(bytes),
        });
        self
    }

    pub(crate) fn write_json(
        &mut self,
        file: impl Into<String>,
        value: &serde_json::Value,
    ) -> &mut ContentEdit {
        self.write(file, json_bytes(value))
    }

    pub fn update_json(
        &mut self,
        files: &ProjectFiles,
        file: &str,
        update: impl FnOnce(&mut serde_json::Value) -> Option<()>,
    ) -> Option<()> {
        let mut json = self.pending_json(files, file)?;
        update(&mut json)?;
        self.write_json(file, &json);
        Some(())
    }

    /// Changes the value at `pointer` in the content file `file` as a `T`, its ids among
    /// `project`'s paths: a value the usages of another file found there, in a row of any type. The
    /// rest of the row is kept as it stands, and the edit is checked whole when it is applied.
    pub(crate) fn update_at<T: serde::Serialize + serde::de::DeserializeOwned>(
        &mut self,
        project: &Project,
        file: &str,
        pointer: &str,
        change: impl FnOnce(&mut T),
    ) -> Result<(), String> {
        let reader = project.reader();
        let mut json = self
            .pending_json(&project.files, file)
            .ok_or_else(|| format!("{} doesn't read", crate::tabs::shown_path(file)))?;
        let at = json
            .pointer_mut(pointer)
            .ok_or_else(|| format!("{pointer} is gone from {}", crate::tabs::shown_path(file)))?;
        let mut value: T = reader.from_json(at)?;
        change(&mut value);
        *at = reader.to_json(&value)?;
        self.write_json(file.to_owned(), &json);
        Ok(())
    }

    /// The file as it stands once this edit is applied, or `None` when the edit removes it.
    fn pending_json(&self, files: &ProjectFiles, file: &str) -> Option<serde_json::Value> {
        match self.changes.iter().find(|change| change.file == file) {
            Some(FileChange {
                bytes: Some(bytes), ..
            }) => serde_json::from_slice(bytes).ok(),
            Some(FileChange { bytes: None, .. }) => None,
            None => files.json_owned(file),
        }
    }

    /// The file's bytes once this edit is applied, or `None` when it has none by then.
    pub(crate) fn pending_bytes(&self, files: &ProjectFiles, file: &str) -> Option<FileBytes> {
        match self.changes.iter().find(|change| change.file == file) {
            Some(change) => change.bytes.clone(),
            None => files.get(file),
        }
    }

    /// Writes `row` as the content file at `path`: its fields, and the parts its type writes.
    pub fn write_content<Row: ContentFile>(
        &mut self,
        project: &Project,
        path: &str,
        row: &Row,
    ) -> Result<&mut ContentEdit, String> {
        self.write_content_in(&project.reader(), path, row)
    }

    /// Writes `row` as the content file at `path`, its ids those of `reader`'s paths.
    pub(crate) fn write_content_in<Row: ContentFile>(
        &mut self,
        reader: &FolderReader,
        path: &str,
        row: &Row,
    ) -> Result<&mut ContentEdit, String> {
        let files = reader.write(path, row).map_err(|error| {
            format!("{} does not write: {error}", crate::tabs::shown_path(path))
        })?;
        for (file, bytes) in files.files {
            self.write(file, FileBytes::from(bytes));
        }
        for file in files.removed {
            self.remove(file);
        }
        Ok(self)
    }

    pub fn write_settings(
        &mut self,
        project: &Project,
        settings: &GameSettings,
    ) -> Result<&mut ContentEdit, String> {
        let files = project
            .reader()
            .write_settings(settings)
            .map_err(|error| format!("the settings do not write: {error}"))?;
        for (file, bytes) in files {
            self.write(file, FileBytes::from(bytes));
        }
        Ok(self)
    }

    pub fn remove(&mut self, file: impl Into<String>) -> &mut ContentEdit {
        let file = file.into();
        self.changes.retain(|change| change.file != file);
        self.changes.push(FileChange { file, bytes: None });
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditRefusal {
    pub file: String,
    pub pointer: String,
    pub problem: ContentProblem,
    pub errors: usize,
}

impl EditRefusal {
    pub fn of(report: &LoadReport) -> EditRefusal {
        EditRefusal::of_errors(&report.errors)
    }

    fn of_errors(errors: &[LoadError]) -> EditRefusal {
        let first = errors.first().cloned().unwrap_or(LoadError {
            file: String::new(),
            at: WholeFile,
            problem: ContentProblem::new("not loaded", "", "a folder that loads"),
        });
        EditRefusal {
            pointer: first.at.pointer().to_owned(),
            file: first.file,
            problem: first.problem,
            errors: errors.len(),
        }
    }

    pub fn sentence(&self) -> String {
        let mut text = self.problem.to_string();
        if self.errors > 1 {
            text.push_str(&format!(" · {} errors in all", self.errors));
        }
        text
    }

    pub fn place(&self) -> String {
        problem_place(&self.file, &self.pointer)
    }
}

/// Where in the folder a problem is: its file, and the JSON pointer within it when it has one.
pub(crate) fn problem_place(file: &str, pointer: &str) -> String {
    if pointer.is_empty() {
        file.to_owned()
    } else {
        format!("{file} at {pointer}")
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditViewpoint {
    pub selection: RowSelection,
    pub camera: Option<AreaCameraState>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AreaCameraState {
    /// The area or blueprint bundle the camera looks at.
    pub bundle: String,
    pub camera: crate::views::area::camera::OrbitCamera,
}

pub struct AppliedStep {
    pub viewpoint: EditViewpoint,
    pub label: String,
}

impl AppliedStep {
    fn of(step: &EditStep) -> AppliedStep {
        AppliedStep {
            viewpoint: EditViewpoint {
                selection: step.selection.clone(),
                camera: step.camera.clone(),
            },
            label: step.label.clone(),
        }
    }
}

pub struct Project {
    pub root: PathBuf,
    pub files: ProjectFiles,
    unsaved: Mutex<Option<UnsavedFiles>>,
    pub content: Option<Content>,
    pub report: Option<Arc<LoadReport>>,
    pub history: EditHistory,
    pub usages: ContentUsages,
    pub unused: UnusedContent,
    pub shapes: Arc<SchemaShapes>,
    pub read_only: Option<String>,
    pub last_load: Option<Duration>,
    pub last_usages: Option<Duration>,
    pub generation: u64,
    pub outside_rewrote: usize,
    /// The paths references name that were moved away on disk, while the folder doesn't load.
    pub moved_outside: Vec<MovedOutside>,
    pub catalogue: NodeCatalogue,
    typed: typed::TypedFiles,
    loading: Option<ChannelInbox<Result<Loaded, LoadReport>>>,
    stale: bool,
    unverified: Option<Unverified>,
    checked: Option<Checked>,
    refused: Option<RefusedEdit>,
    /// The revision of the files whose load to be worked on is being held to every rule, while
    /// it is.
    verifying: Option<u64>,
}

// An edit is adopted at once and its content loaded on a thread, so the interface never waits on
// a load. An edit to a folder that loaded is checked up front only in the files it touches; the
// load then vouches for the rules across files, and an edit it finds problems in is taken back.
// Files that don't load for anything else, such as a change on disk, are kept as they are, with
// the load's report beside the last content that did load.

/// The edits made since the folder last loaded, vouched for together by the next load so the
/// editor never waits on one: the first edit's label, how many came after it, and the files and
/// history from before them.
struct Unverified {
    label: String,
    later: usize,
    files: ProjectFiles,
    history: EditHistory,
}

/// The problems the files an edit touched have as it left them, which the next edit to the same
/// files starts from.
struct Checked {
    revision: u64,
    touched: Vec<String>,
    errors: Vec<LoadError>,
}

/// An edit taken back because the folder didn't load after it.
pub struct RefusedEdit {
    pub label: String,
    pub refusal: EditRefusal,
}

struct Loaded {
    content: Content,
    /// The revision of the files the content was loaded from.
    revision: u64,
    usages: ContentUsages,
    unused: UnusedContent,
    took: Duration,
    usages_took: Duration,
    /// The files it loaded from, when it held them to every rule.
    vouched: Option<Arc<Vouched>>,
}

/// The problems a load found in one file, by JSON pointer (an empty pointer for the file as a
/// whole).
pub(crate) fn errors_in(report: &LoadReport, file: &str) -> Vec<(String, String)> {
    report
        .errors
        .iter()
        .filter(|error| error.file == file)
        .map(|error| (error.at.pointer().to_owned(), error.problem.to_string()))
        .collect()
}

impl Project {
    /// The game folder at `root` open to work on, its files listed and loaded, each step told to
    /// `progress`.
    pub fn open(
        root: &Path,
        shapes: Arc<SchemaShapes>,
        progress: &Progress,
    ) -> Result<Project, String> {
        if !root.join(SETTINGS_FILE).is_file() {
            return Err(format!("No {SETTINGS_FILE} in this folder"));
        }
        save::clean_leftovers(root);
        progress.doing("Listing the folder's files", None);
        let files = ProjectFiles::open(root, &|listed| progress.done(listed))
            .map_err(|error| error.to_string())?;
        progress.doing("Loading the game", None);
        let catalogue = NodeCatalogue::build();
        let mut project = Project {
            root: root.to_path_buf(),
            loading: None,
            stale: false,
            files,
            unsaved: Mutex::new(None),
            content: None,
            report: None,
            history: EditHistory::default(),
            usages: ContentUsages::default(),
            unused: UnusedContent::default(),
            shapes,
            read_only: None,
            last_load: None,
            last_usages: None,
            generation: 0,
            outside_rewrote: 0,
            moved_outside: Vec::new(),
            catalogue,
            typed: typed::TypedFiles::default(),
            unverified: None,
            checked: None,
            refused: None,
            verifying: None,
        };
        project.open_load();
        Ok(project)
    }

    /// The content file at `path` as the game reads it, as the files stand now, or the errors that
    /// keep it from reading.
    pub fn read<Row: ContentFile>(&self, path: &str) -> Result<Arc<Row>, Arc<Unreadable>> {
        self.typed.read(&self.files, path)
    }

    /// The content file at `path` as [`Project::read`] reads it, once a read in the background has
    /// made it: none until then.
    pub fn read_soon<Row: ContentFile>(
        &self,
        path: &str,
    ) -> Option<Result<Arc<Row>, Arc<Unreadable>>> {
        self.typed.read_soon(&self.files, path)
    }

    /// The content file `id` names among the files as they stand now, when it reads.
    pub(crate) fn read_id<Row: ContentFile>(&self, id: ContentId<Row>) -> Option<Arc<Row>> {
        self.read(self.reader().path_of(id)?).ok()
    }

    /// The game's settings as the game reads them, as the files stand now.
    pub fn settings(&self) -> Result<Arc<GameSettings>, Arc<Unreadable>> {
        self.typed.settings(&self.files)
    }

    /// The value at `pointer` in the file `file` as a `T`, its ids among the files' paths as they
    /// stand now: a value the usages of another file found there, in a row of any type.
    pub(crate) fn value_at<T: serde::de::DeserializeOwned>(
        &self,
        file: &str,
        pointer: &str,
    ) -> Option<T> {
        let json = self.files.json(file)?;
        self.reader().from_json(json.pointer(pointer)?).ok()
    }

    /// What reads the files as they stand now: the paths their references are ids among.
    pub fn reader(&self) -> Arc<FolderReader> {
        self.typed.reader(&self.files)
    }

    /// What reads the files as they would stand after `edit`: how an edit names a file it makes.
    pub(crate) fn reader_after(&self, edit: &ContentEdit) -> FolderReader {
        self.reader_of(&self.files.with(&edit.changes))
    }

    /// What reads `files`, the project's files as an edit would leave them.
    fn reader_of(&self, files: &ProjectFiles) -> FolderReader {
        let folder = Arc::new(files.clone());
        if files.paths_revision() == self.files.paths_revision() {
            self.reader().rereading(folder)
        } else {
            FolderReader::new(systems::making_registry(), folder)
        }
    }

    /// What finds, on any thread, what keeps the game from being played as it is now, though it
    /// loads to be worked on: quests and shops out of a player's reach.
    pub(crate) fn playable(&self) -> PlayableCheck {
        PlayableCheck(self.files.clone())
    }

    pub fn loads(&self) -> bool {
        self.report.is_none() && self.content.is_some()
    }

    /// Whether the files as read from disk stopped loading, as opposed to an edit made here, for
    /// more than files moved away from what names them, which the editor can point there.
    pub fn broken_on_disk(&self) -> bool {
        self.report.is_some() && self.outside_rewrote > 0 && !self.moves_explain_report()
    }

    /// Whether every problem of the load is a reference to a file moved away on disk.
    fn moves_explain_report(&self) -> bool {
        let Some(report) = &self.report else {
            return false;
        };
        let naming: BTreeSet<&str> = self
            .moved_outside
            .iter()
            .flat_map(|step| {
                self.usages
                    .of_content(&step.from)
                    .iter()
                    .chain(self.usages.of_media(&step.from))
            })
            .map(|usage| usage.file.as_str())
            .collect();
        !naming.is_empty()
            && report.errors.iter().all(|error| {
                error.problem.rule == MISSING_FILE && naming.contains(error.file.as_str())
            })
    }

    /// Loads the files as they stand to be worked on at once, trusting them as play does, and holds
    /// them to every rule meanwhile: the problems found come in as they would from any load, while
    /// the content worked on stays the one already loaded.
    fn open_load(&mut self) {
        self.loading = None;
        self.stale = false;
        self.unverified = None;
        match Loaded::of(&self.files, LoadPurpose::Play) {
            Ok(loaded) => {
                self.report = None;
                self.install(loaded);
                self.verifying = Some(self.files.revision());
                self.begin_load();
            }
            Err(report) => self.report = Some(Arc::new(report)),
        }
    }

    /// Loads the files as they stand afresh, held to every rule, in place of any load under way:
    /// for taking in what changed on disk. What it finds comes in as from any load.
    pub fn reload(&mut self) {
        self.verifying = None;
        self.loading = None;
        self.stale = false;
        self.unverified = None;
        self.begin_load();
    }

    /// Whether an adopted edit's content is still loading.
    pub fn loading(&self) -> bool {
        self.loading.is_some()
    }

    /// Takes in a load that finished, saying whether one did. Content that loaded is swapped in;
    /// a report on files that did not is kept, unless newer files are already waiting to be
    /// loaded, which then get their own say.
    pub fn poll(&mut self) -> bool {
        let Some(loading) = self.loading.as_ref() else {
            return false;
        };
        let outcome = match loading.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {
                self.loading = None;
                return false;
            }
        };
        self.loading = None;
        self.take(outcome);
        true
    }

    /// Waits out every pending load, so what is saved or asked of the content is what loaded.
    pub fn settle(&mut self) {
        while self.loading.is_some() {
            let outcome = self.loading.take().and_then(|loading| loading.recv());
            if let Some(outcome) = outcome {
                self.take(outcome);
            }
        }
    }

    fn take(&mut self, outcome: Result<Loaded, LoadReport>) {
        if let Ok(Loaded {
            vouched: Some(vouched),
            ..
        }) = &outcome
        {
            self.files.vouch(vouched.clone());
        }
        let verified = self.verifying.take() == Some(self.files.revision()) && !self.stale;
        match outcome {
            Ok(loaded) if verified => {
                self.report = None;
                self.keep_rows(&loaded);
                drop_in_background(loaded);
            }
            Ok(loaded) => {
                if !self.stale {
                    self.unverified = None;
                }
                self.report = None;
                self.install(loaded);
            }
            Err(_) if self.stale => {}
            Err(report) => match self.unverified.take() {
                Some(before) => {
                    self.files = before.files;
                    self.history = before.history;
                    self.refused = Some(RefusedEdit {
                        label: match before.later {
                            0 => before.label,
                            1 => format!("{} and the edit after it", before.label),
                            later => format!("{} and the {later} edits after it", before.label),
                        },
                        refusal: EditRefusal::of_errors(&report.errors),
                    });
                    self.begin_load();
                }
                None => self.report = Some(Arc::new(report)),
            },
        }
        if std::mem::take(&mut self.stale) {
            self.begin_load();
        }
    }

    /// Whether applying `edit` must wait for the load under way: an edit that removes a file must
    /// know what names it, which only a settled load's usages tell.
    pub fn waits_on_load(&self, edit: &ContentEdit) -> bool {
        self.loading()
            && (edit.changes.iter().any(|change| change.bytes.is_none())
                || !self.filled_folders(&edit.changes).is_empty())
    }

    /// The edit the last load took back, once: what the folder didn't load for after it.
    pub fn take_refused(&mut self) -> Option<RefusedEdit> {
        self.refused.take()
    }

    /// Waits for the load to vouch for the edits made since the last, so what is written or taken
    /// from disk is files that load.
    fn settle_unverified(&mut self) {
        if self.unverified.is_some() {
            self.settle();
        }
    }

    /// Applies `edit` as [`Project::prepare`], [`PreparedEdit::check`] and [`Project::commit`] do,
    /// here and now.
    pub fn apply(
        &mut self,
        edit: ContentEdit,
        viewpoint: EditViewpoint,
    ) -> Result<(), Box<EditRefusal>> {
        if edit.changes.iter().any(|change| change.bytes.is_none()) {
            self.settle();
        }
        match self.prepare(edit)? {
            Some(prepared) => self.commit(prepared.check(), viewpoint),
            None => Ok(()),
        }
    }

    /// `edit` worked out against the files as they stand, ready to be checked on any thread: none
    /// when it changes nothing. What names a removed file is known only from the usages of a
    /// settled load.
    pub fn prepare(&self, edit: ContentEdit) -> Result<Option<PreparedEdit>, Box<EditRefusal>> {
        if let Some(reason) = &self.read_only {
            return Err(Box::new(EditRefusal {
                file: String::new(),
                pointer: String::new(),
                problem: ContentProblem::new(
                    "read-only",
                    reason.clone(),
                    "a folder this editor may write",
                ),
                errors: 1,
            }));
        }
        let ContentEdit {
            label,
            changes,
            merge,
            amends,
        } = edit;
        let mut changes: Vec<FileChange> = changes
            .into_iter()
            .filter(
                |change| match (&change.bytes, self.files.get(&change.file)) {
                    (Some(new), Some(old)) => new[..] != old[..],
                    (None, None) => false,
                    _ => true,
                },
            )
            .collect();
        if changes.is_empty() {
            return Ok(None);
        }
        let filled = self.filled_folders(&changes);
        changes.extend(filled.into_iter().map(|entry| FileChange {
            file: entry,
            bytes: None,
        }));
        let mut touched: BTreeSet<String> = changes.iter().map(|c| c.file.clone()).collect();
        // The game decodes only the media its content names, so a media file is checked through
        // whatever names it, and whatever named a file removed must do without it.
        for change in &changes {
            let naming = match content_holding(&change.file) {
                Some(content) if change.bytes.is_none() => self.usages.of_content(content),
                Some(_) => continue,
                None => self.usages.of_media(&change.file),
            };
            touched.extend(naming.iter().map(|usage| usage.file.clone()));
        }
        let touched: Vec<String> = touched.into_iter().collect();
        // Files that loaded whole and haven't changed since have no errors an edit may keep.
        let clean = self.loads() && !self.loading();
        let known = match &self.checked {
            _ if clean => Known::Found(HashSet::new()),
            Some(checked)
                if checked.revision == self.files.revision() && checked.touched == touched =>
            {
                Known::Found(checked.errors.iter().map(error_key).collect())
            }
            _ => Known::Unchecked(self.reader()),
        };
        let next = self.files.with(&changes);
        Ok(Some(PreparedEdit {
            label,
            merge,
            amends,
            from: self.files.revision(),
            next_reader: self.reader_of(&next),
            next,
            changes,
            touched,
            known,
        }))
    }

    /// Takes in `checked`, unless it brings problems into the files it touches that they did not
    /// have before: what refuses an edit, so the editor never writes content the game would
    /// refuse. An edit prepared against files that have changed since is checked again here.
    pub fn commit(
        &mut self,
        checked: CheckedEdit,
        viewpoint: EditViewpoint,
    ) -> Result<(), Box<EditRefusal>> {
        if checked.from != self.files.revision() {
            let edit = ContentEdit {
                label: checked.label,
                changes: checked.changes,
                merge: checked.merge,
                amends: checked.amends,
            };
            return match self.prepare(edit)? {
                Some(prepared) => self.commit(prepared.check(), viewpoint),
                None => Ok(()),
            };
        }
        if !checked.introduced.is_empty() {
            return Err(Box::new(EditRefusal::of_errors(&checked.introduced)));
        }
        let CheckedEdit {
            label,
            merge,
            amends,
            next,
            changes,
            checked,
            read,
            ..
        } = checked;
        let continues = merge || amends.is_some();
        self.checked = Some(checked);
        let before: Vec<FileChange> = changes
            .iter()
            .map(|change| FileChange {
                file: change.file.clone(),
                bytes: self.files.get(&change.file),
            })
            .collect();
        let loads = self.loads();
        match &mut self.unverified {
            Some(unverified) if !continues => unverified.later += 1,
            Some(_) => {}
            None if loads => {
                self.unverified = Some(Unverified {
                    label: label.clone(),
                    later: 0,
                    files: self.files.clone(),
                    history: self.history.clone(),
                });
            }
            None => {}
        }
        self.adopt(next);
        self.typed.keep(&self.files, read);
        let amended = amends.as_deref().is_some_and(|step| {
            self.history
                .amend_last(step, &label, changes.clone(), &before)
        });
        let merged = amended
            || (merge
                && self
                    .history
                    .merge_into_last(&label, changes.clone(), &before));
        if !merged {
            self.history.push(EditStep {
                label,
                before,
                after: changes,
                selection: viewpoint.selection,
                camera: viewpoint.camera,
                at: SystemTime::now(),
            });
        }
        Ok(())
    }

    pub fn undo(&mut self) -> Option<AppliedStep> {
        let step = self.history.undo_step()?;
        let (applied, changes) = (AppliedStep::of(step), step.before.clone());
        self.restore(&changes);
        self.history.undone();
        Some(applied)
    }

    pub(crate) fn redo(&mut self) -> Option<AppliedStep> {
        let step = self.history.redo_step()?;
        let (applied, changes) = (AppliedStep::of(step), step.after.clone());
        self.restore(&changes);
        self.history.redone();
        Some(applied)
    }

    pub fn jump(&mut self, index: usize) -> Option<AppliedStep> {
        let mut last = None;
        while self.history.applied() > index {
            let Some(applied) = self.undo() else {
                break;
            };
            last = Some(applied);
        }
        while self.history.applied() < index {
            let Some(applied) = self.redo() else {
                break;
            };
            last = Some(applied);
        }
        last
    }

    fn restore(&mut self, changes: &[FileChange]) {
        let next = self.files.with(changes);
        self.adopt(next);
    }

    /// What refuses an edit, on any thread, when loading the whole folder after it finds
    /// problems the folder doesn't have now: the rules across files that `apply` leaves to the
    /// next load, checked up front for an edit that reaches many files at once.
    pub fn checker(&self) -> EditChecker {
        EditChecker {
            clean: self.loads() && !self.loading(),
            files: self.files.clone(),
            known: OnceLock::new(),
        }
    }

    fn adopt(&mut self, next: ProjectFiles) {
        self.files = next;
        self.begin_load();
    }

    fn begin_load(&mut self) {
        if self.loading.is_some() {
            self.stale = true;
            return;
        }
        let files = self.files.clone();
        let (sender, receiver) = mpsc::channel();
        self.loading = Some(ChannelInbox::new(receiver));
        std::thread::spawn(move || {
            let outcome = background().install(|| Loaded::of(&files, LoadPurpose::Check));
            let _ = sender.send(outcome);
        });
    }

    fn install(&mut self, loaded: Loaded) {
        self.keep_rows(&loaded);
        self.moved_outside.clear();
        self.last_load = Some(loaded.took);
        self.last_usages = Some(loaded.usages_took);
        self.usages = loaded.usages;
        self.unused = loaded.unused;
        drop_in_background(self.content.replace(loaded.content));
        self.generation += 1;
    }

    /// Keeps the rows `loaded` holds as the reads of their files, when it was loaded from the files
    /// as they stand, so no view reads again what a load just read.
    fn keep_rows(&self, loaded: &Loaded) {
        if loaded.revision == self.files.revision() {
            let rows = loaded.content.read_rows();
            self.typed.keep(&self.files, rows);
        }
    }

    /// Every file and kept folder that differs from disk, by path.
    pub fn unsaved(&self) -> Arc<[String]> {
        let mut known = self.unsaved.lock().unwrap_or_else(PoisonError::into_inner);
        let revision = (self.files.revision(), self.files.disk_revision());
        if let Some(known) = &*known
            && known.revision == revision
        {
            return known.paths.clone();
        }
        let unsaved: Arc<[String]> = self.files.unsaved().into();
        *known = Some(UnsavedFiles {
            revision,
            paths: unsaved.clone(),
        });
        unsaved
    }

    /// Whether the file at `path`, or any part of the bundle there, differs from disk.
    pub(crate) fn holds_unsaved(&self, path: &str) -> bool {
        let unsaved = self.unsaved();
        let parts = files::folder_entry(path);
        let first_part = unsaved.partition_point(|file| *file < parts);
        unsaved
            .binary_search_by(|file| file.as_str().cmp(path))
            .is_ok()
            || unsaved
                .get(first_part)
                .is_some_and(|file| file.starts_with(&parts))
    }

    /// The hash of the file at `path` as it stands on disk.
    pub(crate) fn disk_hash(&self, path: &str) -> Option<String> {
        self.files.disk_hash(path)
    }

    /// Writes every unsaved file to disk, waiting for the edits made since the last load to be
    /// vouched for first.
    pub fn save(&mut self) -> Result<save::SaveOutcome, save::SaveError> {
        self.settle_unverified();
        let saved = self.saving().write(&Progress::default());
        self.take_saved(saved)
    }

    /// What writes the unsaved files to disk, on any thread, once the edits made since the last
    /// load are vouched for; [`Project::take_saved`] takes in what it wrote.
    pub fn saving(&self) -> save::Saving {
        save::Saving {
            root: self.root.clone(),
            files: self.files.clone(),
        }
    }

    pub fn take_saved(&mut self, saved: save::Saved) -> Result<save::SaveOutcome, save::SaveError> {
        self.files = saved.files;
        if saved.outcome.is_ok() {
            self.history.mark_saved();
        }
        saved.outcome
    }

    /// Takes in what changed on disk as [`Project::take_outside`] does, waiting for every step:
    /// the edits made since the last load to be vouched for, the disk to be read, and the folder
    /// to load again.
    pub fn outside_changes(&mut self, change: FolderChange) -> OutsideChanges {
        self.settle_unverified();
        let read = self.reading_disk(change).read(&Progress::default());
        let changes = self.take_outside(read);
        if !changes.taken.is_empty() {
            self.settle();
            self.reloaded_outside(changes.taken.len());
        }
        changes
    }

    /// What reads how the files `change` may touch stand on disk now, on any thread, once the
    /// edits made since the last load are vouched for; [`Project::take_outside`] takes it in.
    pub fn reading_disk(&self, change: FolderChange) -> DiskReading {
        DiskReading {
            root: self.root.clone(),
            files: self.files.clone(),
            change,
        }
    }

    /// Takes in what changed on disk: a file unedited here is taken as the disk has it, an edited
    /// one is a conflict. Files taken start the history over and the folder loading again; once
    /// that load is in, [`Project::reloaded_outside`] says how it went.
    pub fn take_outside(&mut self, read: DiskRead) -> OutsideChanges {
        let mut taken = Vec::new();
        let mut conflicts = Vec::new();
        for change in read.changes {
            match change.conflict {
                Some(disk) => {
                    self.files.disk_moved_on(&change.path, change.now);
                    conflicts.push(FileConflict {
                        file: change.path,
                        disk,
                    });
                }
                None => {
                    self.files.take_from_disk(&change.path, change.now);
                    taken.push(change.path);
                }
            }
        }
        if !taken.is_empty() {
            self.reload_after_outside();
        }
        OutsideChanges { taken, conflicts }
    }

    /// The entries keeping folders that were empty and that `changes` put something in: a folder
    /// is kept by its entry only while nothing else is in it, as the disk shows it.
    fn filled_folders(&self, changes: &[FileChange]) -> BTreeSet<String> {
        let mut filled = BTreeSet::new();
        for change in changes.iter().filter(|change| change.bytes.is_some()) {
            let mut folder = parent_of(change.file.trim_end_matches('/'));
            while !folder.is_empty() {
                let entry = files::folder_entry(folder);
                if self.files.contains(&entry) && !changes.iter().any(|change| change.file == entry)
                {
                    filled.insert(entry);
                }
                folder = parent_of(folder);
            }
        }
        filled
    }

    /// Takes the disk's side of `conflict`, once the edits made since the last load are vouched
    /// for, starting the folder loading again as [`Project::take_outside`] does.
    pub(crate) fn take_disk(&mut self, conflict: &FileConflict) {
        let now = self.files.on_disk(&conflict.file);
        self.files.take_from_disk(&conflict.file, now);
        self.reload_after_outside();
    }

    /// Files rewritten from outside start the history over and are loaded again.
    fn reload_after_outside(&mut self) {
        self.history.reset();
        self.reload();
    }

    /// Notes how the load after files were rewritten from outside went, once it is in: if it
    /// failed, `rewrote` says how many files the failure came in with.
    pub fn reloaded_outside(&mut self, rewrote: usize) {
        self.outside_rewrote = if self.report.is_some() { rewrote } else { 0 };
        self.moved_outside = match self.report {
            Some(_) => moved_outside(&self.usages, &self.files),
            None => Vec::new(),
        };
    }
}

/// Names files as they stand now; a file that doesn't read is named by its path.
impl ContentNames for Project {
    fn name_of<Row: NamedContent>(&self, id: ContentId<Row>) -> String {
        let reader = self.reader();
        let Some(path) = reader.path_of(id) else {
            return String::new();
        };
        match self.read::<Row>(path) {
            Ok(row) => row.shown_name().to_owned(),
            Err(_) => crate::tabs::shown_path(path).to_owned(),
        }
    }
}

impl Loaded {
    /// The files loaded for `purpose`, and the usages they hold, worked out side by side.
    fn of(files: &ProjectFiles, purpose: LoadPurpose) -> Result<Loaded, LoadReport> {
        let reader = FolderReader::new(systems::making_registry(), Arc::new(files.clone()));
        let (loaded, (usages, usages_took)) = rayon::join(
            || load_for(files, purpose),
            || {
                let started = Instant::now();
                (ContentUsages::build(&reader, files), started.elapsed())
            },
        );
        let (content, took) = loaded?;
        let unused = UnusedContent::find(files, &usages);
        let vouched = (purpose == LoadPurpose::Check)
            .then(|| Arc::new(Vouched::of(files, content.built::<FileHashes>())));
        Ok(Loaded {
            vouched,
            content,
            revision: files.revision(),
            usages,
            unused,
            took,
            usages_took,
        })
    }
}

static NO_CATALOGUE: NodeCatalogue = NodeCatalogue {
    entries: Vec::new(),
    events: Vec::new(),
};

pub(crate) fn catalogue_of(project: Option<&Project>) -> &NodeCatalogue {
    project.map_or(&NO_CATALOGUE, |project| &project.catalogue)
}

/// A path the folder's references name that is gone from disk, and the one file of the same
/// name and type now found elsewhere in the folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovedOutside {
    pub from: String,
    pub to: String,
    pub references: usize,
}

/// Every path named at the last load that is gone now, with where a file of its name and type is.
fn moved_outside(usages: &ContentUsages, files: &ProjectFiles) -> Vec<MovedOutside> {
    let index = files.index();
    let name = |path: &str| crate::moves::file_name(path).to_owned();
    let found = |from: &str, candidates: Vec<&str>| {
        let mut same = candidates
            .into_iter()
            .filter(|path| name(path) == name(from));
        match (same.next(), same.next()) {
            (Some(only), None) => Some(only.to_owned()),
            _ => None,
        }
    };
    let content = usages.content().filter_map(|(from, uses)| {
        let table = table_of(from)?;
        if index.is_content(from) {
            return None;
        }
        let to = found(from, index.paths_of(table).collect())?;
        Some(MovedOutside {
            from: from.to_owned(),
            to,
            references: uses.len(),
        })
    });
    let media = usages.media().filter_map(|(from, uses)| {
        if index.media.contains(from) {
            return None;
        }
        let to = found(from, index.media.iter().map(String::as_str).collect())?;
        Some(MovedOutside {
            from: from.to_owned(),
            to,
            references: uses.len(),
        })
    });
    let mut moved: Vec<MovedOutside> = content.chain(media).collect();
    moved.sort_by(|a, b| a.from.cmp(&b.from));
    moved
}

/// An edit worked out against the files as they stand, to be checked on any thread.
pub struct PreparedEdit {
    label: String,
    merge: bool,
    amends: Option<String>,
    /// The revision of the files it was worked out against.
    from: u64,
    next: ProjectFiles,
    next_reader: FolderReader,
    changes: Vec<FileChange>,
    /// The files whose problems the edit may change.
    touched: Vec<String>,
    known: Known,
}

/// The problems the touched files have as they stand: found already, or to be found by reading
/// them.
enum Known {
    Found(HashSet<(String, &'static str, String)>),
    Unchecked(Arc<FolderReader>),
}

/// An edit and what checking the files after it found.
pub struct CheckedEdit {
    label: String,
    merge: bool,
    amends: Option<String>,
    from: u64,
    next: ProjectFiles,
    changes: Vec<FileChange>,
    /// The problems it brings into the files it touches that they did not have before.
    introduced: Vec<LoadError>,
    checked: Checked,
    /// The content among the touched files that reads whole after it.
    read: Vec<ReadRow>,
}

impl PreparedEdit {
    /// The bytes of the content the check reads: what it costs.
    pub fn weight(&self) -> usize {
        self.touched
            .iter()
            .filter_map(|file| self.next.get(file))
            .map(|bytes| bytes.len())
            .sum()
    }

    /// Checks the files the edit touches as they stand and after it.
    pub fn check(self) -> CheckedEdit {
        let known = match &self.known {
            Known::Found(known) => known.clone(),
            Known::Unchecked(reader) => reader
                .check(&self.touched)
                .errors
                .iter()
                .map(error_key)
                .collect(),
        };
        let CheckedFiles { errors, read } = self.next_reader.check(&self.touched);
        let introduced = errors
            .iter()
            .filter(|error| !known.contains(&error_key(error)))
            .cloned()
            .collect();
        CheckedEdit {
            label: self.label,
            merge: self.merge,
            amends: self.amends,
            from: self.from,
            checked: Checked {
                revision: self.next.revision(),
                touched: self.touched,
                errors,
            },
            next: self.next,
            changes: self.changes,
            introduced,
            read,
        }
    }
}

/// The files as they stand, to load whole as an edit would leave them.
pub struct EditChecker {
    files: ProjectFiles,
    /// Whether the files load whole as they stand, so an edit may keep no problem of theirs.
    clean: bool,
    /// The problems the files have as they stand, once worked out.
    known: OnceLock<HashSet<(String, &'static str, String)>>,
}

impl EditChecker {
    /// Refuses `edit` when the folder loaded after it has problems it doesn't have now.
    pub fn check(&self, edit: &ContentEdit) -> Result<(), Box<EditRefusal>> {
        let Err(report) = load(&self.files.with(&edit.changes)) else {
            return Ok(());
        };
        let known = self.known.get_or_init(|| {
            if self.clean {
                return HashSet::new();
            }
            load(&self.files)
                .err()
                .map(|report| report.errors.iter().map(error_key).collect())
                .unwrap_or_default()
        });
        let introduced: Vec<LoadError> = report
            .errors
            .into_iter()
            .filter(|error| !known.contains(&error_key(error)))
            .collect();
        if introduced.is_empty() {
            Ok(())
        } else {
            Err(Box::new(EditRefusal::of_errors(&introduced)))
        }
    }
}

/// The files as they stand, to be loaded as a play-test would.
pub(crate) struct PlayableCheck(ProjectFiles);

impl PlayableCheck {
    /// What keeps the files from being played, when anything does.
    pub(crate) fn run(self) -> Option<LoadReport> {
        let folder = Arc::new(self.0);
        systems::load_content(folder, "it can't be played yet", LoadPurpose::Check).err()
    }
}

pub struct OutsideChanges {
    pub taken: Vec<String>,
    pub conflicts: Vec<FileConflict>,
}

/// The files a change on disk may touch, to be read as they stand there now.
pub struct DiskReading {
    root: PathBuf,
    files: ProjectFiles,
    change: FolderChange,
}

/// How the files a change on disk touched stand there, against the files as the editor holds them.
pub struct DiskRead {
    changes: Vec<DiskChange>,
}

/// A file that stands on disk otherwise than the editor knew it to: `now` as it stands, and the
/// disk's side of a conflict when the file is edited here too.
struct DiskChange {
    path: String,
    now: Option<OnDisk>,
    conflict: Option<Option<FileBytes>>,
}

impl DiskReading {
    pub fn read(self, progress: &Progress) -> DiskRead {
        let DiskReading {
            root,
            files,
            change,
        } = self;
        let candidates: Vec<(String, Option<OnDisk>)> = match change {
            FolderChange::Files(paths) => held_by(&root, &files, paths)
                .into_iter()
                .map(|path| {
                    let now = OnDisk::at(&root.join(&path));
                    (path, now)
                })
                .collect(),
            FolderChange::Unknown => {
                progress.doing("Listing the folder's files", None);
                let listed: BTreeMap<String, OnDisk> =
                    files::list(&root, &|listed| progress.done(listed))
                        .map(|listed| listed.into_iter().collect())
                        .unwrap_or_default();
                let mut all: BTreeSet<String> = listed.keys().cloned().collect();
                all.extend(files.on_disk_under("").cloned());
                all.into_iter()
                    .map(|path| {
                        let now = listed.get(&path).copied();
                        (path, now)
                    })
                    .collect()
            }
        };
        let changes = candidates
            .into_iter()
            .filter(|(path, now)| *now != files.on_disk(path))
            .map(|(path, now)| DiskChange {
                conflict: files
                    .differs_from_disk(&path)
                    .then(|| read_entry(&root, &path)),
                path,
                now,
            })
            .collect();
        DiskRead { changes }
    }
}

/// Every file and kept folder a change to `paths` may touch: each path itself and, for a folder or
/// a bundle, everything it holds on disk and held before.
fn held_by(root: &Path, files: &ProjectFiles, paths: Vec<String>) -> Vec<String> {
    let mut held: BTreeSet<String> = BTreeSet::new();
    for path in paths {
        let inside = files::folder_entry(&path);
        held.extend(files.paths_under(&inside).cloned());
        held.extend(files.on_disk_under(&inside).cloned());
        let on_disk = root.join(&path);
        if on_disk.is_dir() {
            let walked = files::list(&on_disk, &|_| {}).unwrap_or_default();
            held.extend(
                walked
                    .into_iter()
                    .map(|(file, _)| format!("{inside}{file}")),
            );
            if files::empty_folder(&on_disk).unwrap_or(false) {
                held.insert(inside);
            }
        } else {
            held.insert(path);
        }
    }
    held.into_iter().collect()
}

#[derive(Clone)]
pub struct FileConflict {
    pub file: String,
    pub disk: Option<FileBytes>,
}

/// What is at `entry` on disk: a file's bytes, or nothing for a kept folder that stays empty.
fn read_entry(root: &Path, entry: &str) -> Option<FileBytes> {
    if files::is_folder(entry) {
        let folder = root.join(entry);
        return files::empty_folder(&folder)
            .unwrap_or(false)
            .then(|| FileBytes::from([]));
    }
    std::fs::read(root.join(entry)).ok().map(FileBytes::from)
}

/// The pool the editor works off the frame with, such as loading the folder, holding an area's
/// ground or what the editor waits on, and what that work runs in parallel: a thread for every
/// core at the lowest priority, so the frames always come first and the work takes what cores
/// they leave.
pub(crate) fn background() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(
                std::thread::available_parallelism()
                    .map_or(1, |threads| threads.get() / 2)
                    .max(1),
            )
            .thread_name(|index| format!("background-{index}"))
            .start_handler(|_| {
                // A thread left at the priority it had still works, only competing with frames.
                let _ = thread_priority::set_current_thread_priority(
                    thread_priority::ThreadPriority::Min,
                );
            })
            .build()
            .expect("a background pool")
    })
}

/// Lets go of `value` on the background pool: content let go of holds so much that freeing it
/// would hold up a frame.
pub(crate) fn drop_in_background<T: Send + 'static>(value: T) {
    background().spawn(move || drop(value));
}

pub fn load(files: &ProjectFiles) -> Result<(Content, Duration), LoadReport> {
    load_for(files, LoadPurpose::Check)
}

/// The files loaded for `purpose`, and how long it took.
fn load_for(files: &ProjectFiles, purpose: LoadPurpose) -> Result<(Content, Duration), LoadReport> {
    let started = Instant::now();
    let folder = Arc::new(files.clone());
    systems::load_making(folder, "the folder did not load", purpose)
        .map(|content| (content, started.elapsed()))
}

/// What tells two reports of a problem apart, wherever in its file the problem moved to.
fn error_key(error: &LoadError) -> (String, &'static str, String) {
    (
        error.file.clone(),
        error.problem.rule,
        error.at.pointer().to_owned(),
    )
}

/// The unsaved files, as last worked out, and the revisions of the files and disk hashes they
/// were worked out from.
struct UnsavedFiles {
    revision: (u64, u64),
    paths: Arc<[String]>,
}
