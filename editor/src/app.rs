use std::fs::File;
use std::io::{BufWriter, Write};
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Instant, SystemTime};
use ui::egui;
use ui::words;

use bevy::app::MainScheduleOrder;
use bevy::prelude::*;
use bevy_egui::{
    EguiContext, EguiPlugin, EguiPreUpdateSet, EguiPrimaryContextPass, PrimaryEguiContext,
};
use engine::core::content::{Content, ContentFile, FolderReader, Unreadable, fields_file, version};
use engine::core::render::video::VideoPreferences;
use engine::core::units::{Meters, Millis};
use engine::systems;
use engine::systems::area::scenery;
use engine::systems::frame_rate::{self, FrameCountPlugin, FrameRates};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum::IntoStaticStr;
use ui::egui_dock::DockState;

use crate::clipboard::Clipboard;
use crate::forms::{FormCx, FormState};
use crate::modal::ModalDialog;
use crate::output_log::OutputLog;
use crate::play::{self, PlayTest};
use crate::project::journal::{
    JournalRevision, RecoveryJournal, RecoveryJournalData, RecoveryJournalFile,
};
use crate::project::lock::ProjectLock;
use crate::project::watch::FolderWatcher;
use crate::project::{
    self, AppliedStep, CheckedEdit, ContentEdit, EditRefusal, EditViewpoint, Project, RowSelection,
};
use crate::row_form;
use crate::scenes::{self, SceneStep};
use crate::schema::SchemaShapes;
use crate::shell;
use crate::tabs::{self, DockRequest, EditorTab, TabOpening};
use crate::thumbs::Thumbnail;
use crate::user::{EditorSettings, UserFolder};
use crate::views::{self, EditorCamera, EditorViews};

#[derive(Clone, Debug)]
pub struct EditorLaunch {
    pub folder: Option<PathBuf>,
    pub scene: Option<Vec<SceneStep>>,
    pub user: Option<PathBuf>,
    pub frame_log: Option<PathBuf>,
}

pub struct EditorPlugin {
    pub launch: EditorLaunch,
}

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        let user = match &self.launch.user {
            Some(root) => UserFolder::at(root.clone()),
            None => UserFolder::locate(),
        };
        let settings = user.load_settings();
        let mut editor = Editor::new(user, settings);
        editor.scene = self.launch.scene.clone();
        if let Some(folder) = &self.launch.folder {
            editor.open_folder(folder);
        }
        if let Some(log) = self
            .launch
            .frame_log
            .as_ref()
            .and_then(|path| File::create(path).ok())
        {
            app.insert_resource(FrameLog(BufWriter::new(log)));
        }
        app.init_schedule(Background);
        app.world_mut()
            .resource_mut::<MainScheduleOrder>()
            .insert_after(Update, Background);
        app.add_plugins((EguiPlugin::default(), FrameCountPlugin))
            .insert_resource(editor)
            .init_resource::<crate::thumbs::ReadThumbnails>()
            .add_observer(crate::thumbs::took)
            .add_systems(Startup, setup_camera)
            .add_systems(
                PreUpdate,
                theme_context.after(EguiPreUpdateSet::InitContexts),
            )
            .add_systems(EguiPrimaryContextPass, editor_ui)
            .add_systems(Update, show_video)
            .add_systems(Background, background);
    }
}

const FRAME_RATE_MARGIN: f32 = 8.0;
/// The most bytes of content an edit's check reads on the frame: one reading more checks off it.
const CHECKED_ON_THE_FRAME: usize = 256 << 10;

/// A line per frame, each a [`FrameRecord`] in JSON.
#[derive(Resource)]
struct FrameLog(BufWriter<File>);

/// What the frame log tells of a frame: how long it took and its interface of that, the active
/// tab and its camera, how many entities, images and meshes the world holds, and how its scenery stands.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FrameRecord {
    pub frame: Millis,
    pub ui: Millis,
    pub tab: Option<String>,
    /// How far the open area or blueprint view's camera stands from what it looks at.
    pub camera: Option<Meters>,
    pub entities: usize,
    pub images: usize,
    pub meshes: usize,
    pub scenery: SceneryState,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SceneryState {
    /// No view draws an area.
    None,
    /// What a view takes in is still being stood or readied.
    Standing,
    /// Every view's scenery stands whole.
    Stood,
}

impl SceneryState {
    fn of(world: &mut World) -> SceneryState {
        if scenery::sceneries(world) == 0 {
            SceneryState::None
        } else if scenery::standing(world) || engine::core::render::world::readying(world) {
            SceneryState::Standing
        } else {
            SceneryState::Stood
        }
    }
}

/// Where the editor edits the world its views draw: a schedule of its own after `Update`, since
/// an exclusive system sharing a schedule may despawn an entity between another system queueing a
/// command on it and that command landing.
#[derive(bevy::ecs::schedule::ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
struct Background;

fn setup_camera(mut commands: Commands) {
    commands.spawn((Camera2d, PrimaryEguiContext, EditorCamera));
}

// egui takes up new fonts as a pass begins, so they must be set before the first pass draws text.
fn theme_context(mut contexts: Query<&mut EguiContext, Added<PrimaryEguiContext>>) {
    for mut context in &mut contexts {
        ui::apply(context.get_mut());
    }
}

#[derive(Resource)]
pub struct Editor {
    pub user: UserFolder,
    pub settings: EditorSettings,
    pub shapes: Arc<SchemaShapes>,
    pub project: Option<Project>,
    pub lock: Option<ProjectLock>,
    pub watcher: Option<FolderWatcher>,
    pub journal: Option<RecoveryJournal>,
    pub dock: DockState<EditorTab>,
    pub bottom_tab: BottomTab,
    pub content_tree: crate::content_tree::ContentTree,
    pub status: StatusBar,
    pub last_refusal: Option<EditRefusal>,
    pub history_focus: Option<usize>,
    pub selection: RowSelection,
    pub form: FormState,
    pub output: OutputLog,
    pub dialogs: Vec<ModalDialog>,
    pub toast: Option<(String, Instant)>,
    /// Text to put on the system clipboard on the next frame.
    pub copied: Option<String>,
    pub start: StartScreen,
    pub open_tabs: Vec<(EditorTab, TabOpening)>,
    pub preview: Option<EditorTab>,
    pub dock_requests: Vec<DockRequest>,
    pub scene: Option<Vec<SceneStep>>,
    pub clipboard: Option<Clipboard>,
    pub views: EditorViews,
    pub play: play::PlayTest,
    pub pick: Option<PlacePick>,
    pub busy: crate::busy::Busy,
    pub quit: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
pub enum BottomTab {
    #[default]
    History,
    Output,
    Palette,
    Blueprints,
    #[strum(serialize = "May do")]
    MayDo,
    #[strum(serialize = "Used by")]
    UsedBy,
    Run,
    Timeline,
    Sheet,
    Sounds,
    Listen,
}

impl BottomTab {
    pub fn title(self) -> &'static str {
        self.into()
    }
}

#[derive(Default)]
pub struct StatusBar {
    pub left: Vec<String>,
    pub right: Vec<String>,
}

#[derive(Default)]
pub struct StartScreen {
    pub failed: Option<(PathBuf, String)>,
    pub report: Option<(PathBuf, String)>,
    pub watch: Option<FolderWatcher>,
    pub recover: Option<(PathBuf, RecoveryJournalData)>,
    pub names: Vec<(PathBuf, String)>,
}

#[derive(Clone, Debug)]
pub struct PlacePick {
    pub file: String,
    pub pointer: String,
    pub area: Option<String>,
}

impl Editor {
    pub fn new(user: UserFolder, settings: EditorSettings) -> Editor {
        Editor {
            user,
            settings,
            shapes: Arc::new(SchemaShapes::load()),
            project: None,
            lock: None,
            watcher: None,
            journal: None,
            dock: DockState::new(Vec::new()),
            bottom_tab: BottomTab::default(),
            content_tree: Default::default(),
            status: StatusBar::default(),
            last_refusal: None,
            history_focus: None,
            selection: RowSelection::default(),
            form: FormState::default(),
            output: OutputLog::default(),
            dialogs: Vec::new(),
            toast: None,
            copied: None,
            start: StartScreen::default(),
            open_tabs: Vec::new(),
            preview: None,
            dock_requests: Vec::new(),
            scene: None,
            clipboard: None,
            views: EditorViews::default(),
            play: PlayTest::default(),
            pick: None,
            busy: Default::default(),
            quit: false,
        }
    }

    pub fn say(&mut self, text: impl Into<String>) {
        self.output.say("editor", text);
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.say(text.clone());
        self.toast = Some((text, Instant::now()));
    }

    pub(crate) fn copy_text(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.toast(format!("Copied {text}"));
        self.copied = Some(text);
    }

    pub(crate) fn save_settings(&self) {
        self.user.save_settings(&self.settings);
    }

    pub fn open_folder(&mut self, folder: &Path) {
        // Whatever else reads the folder, such as a play-test's binaries or a later session, may
        // not start from where this one did.
        let folder = match std::path::absolute(folder) {
            Ok(folder) => folder,
            Err(error) => {
                let reason = format!("{}: {error}", folder.display());
                self.start.failed = Some((folder.to_path_buf(), reason.clone()));
                self.say(reason);
                return;
            }
        };
        if let Some(data) = RecoveryJournal::recover(&self.user, &folder)
            && !data.files.is_empty()
            && self.start.recover.is_none()
        {
            self.start.recover = Some((folder, data));
            return;
        }
        self.open_now(&folder, Vec::new());
    }

    /// Opens the game folder at `folder`, taking in the unsaved files `recovered` from its
    /// journal, the editor waiting for it.
    pub fn open_now(&mut self, folder: &Path, recovered: Vec<RecoveryJournalFile>) {
        self.close_project();
        let (root, shapes) = (folder.to_path_buf(), self.shapes.clone());
        let folder = folder.to_path_buf();
        self.run_busy(
            format!("Opening {}", shell::start::game_name(&folder)),
            move |progress| {
                let watcher = FolderWatcher::on(&root);
                (Project::open(&root, shapes, progress), watcher)
            },
            move |editor, (opened, watcher)| editor.opened(&folder, opened, watcher, recovered),
        );
    }

    fn opened(
        &mut self,
        folder: &Path,
        opened: Result<Project, String>,
        watcher: FolderWatcher,
        recovered: Vec<RecoveryJournalFile>,
    ) {
        match opened {
            Ok(project) if project.report.is_some() && recovered.is_empty() => {
                self.say_opened(folder, &project);
                self.settings.remember(folder);
                self.save_settings();
                self.start.failed = None;
                self.start.report = project
                    .report
                    .as_ref()
                    .map(|report| (folder.to_path_buf(), report.to_string()));
                self.start.watch = Some(watcher);
            }
            Ok(project) => self.enter(project, folder, watcher, recovered),
            Err(reason) => {
                self.start.failed = Some((folder.to_path_buf(), reason.clone()));
                self.say(reason);
            }
        }
    }

    fn enter(
        &mut self,
        mut project: Project,
        folder: &Path,
        watcher: FolderWatcher,
        recovered: Vec<RecoveryJournalFile>,
    ) {
        let lock = match ProjectLock::take(&self.user, folder) {
            Ok(lock) => Some(lock),
            Err(reason) => {
                project.read_only = Some(reason.clone());
                self.say(format!("Read-only: {reason}"));
                None
            }
        };
        self.say_opened(folder, &project);
        self.settings.remember(folder);
        self.save_settings();
        self.lock = lock;
        self.watcher = Some(watcher);
        self.journal = Some(RecoveryJournal::for_folder(&self.user, folder));
        self.project = Some(project);
        self.dock = DockState::new(Vec::new());
        self.bottom_tab = BottomTab::default();
        self.selection = RowSelection::default();
        self.form = FormState::default();
        self.views = EditorViews::default();
        self.start.failed = None;
        self.start.report = None;
        self.start.watch = None;
        play::check_binaries(self);
        if !recovered.is_empty() {
            let folder = folder.to_path_buf();
            self.when_settled("Recovering unsaved rows", move |editor| {
                editor.recover(&folder, &recovered);
            });
        }
        if let Some(scene) = self.scene.take() {
            scenes::open(self, &scene);
        }
    }

    /// Applies the unsaved files `recovered` from the journal of `folder` as an edit, once the
    /// folder's loads are in, then once the folder loads after it says how that went: a journal
    /// the folder doesn't load with is set aside.
    fn recover(&mut self, folder: &Path, recovered: &[RecoveryJournalFile]) {
        let Some(project) = &mut self.project else {
            return;
        };
        let mut edit = ContentEdit::new("Recover unsaved rows from the journal");
        for file in recovered {
            let change = file.change();
            match change.bytes {
                Some(bytes) => edit.write(change.file, bytes),
                None => edit.remove(change.file),
            };
        }
        let changed: Vec<String> = recovered
            .iter()
            .filter(|file| file.changed_on_disk(project.disk_hash(&file.path).as_deref()))
            .map(|file| file.path.clone())
            .collect();
        let count = recovered.len();
        let folder = folder.to_path_buf();
        if let Err(refusal) = project.apply(edit, EditViewpoint::default()) {
            self.journal_refused(&folder, &refusal);
            return;
        }
        self.when_settled("Recovering unsaved rows", move |editor| {
            let refused = editor.project.as_mut().and_then(Project::take_refused);
            match refused {
                Some(refused) => editor.journal_refused(&folder, &refused.refusal),
                None if !changed.is_empty() => editor.toast(format!(
                    "Recovered {count} unsaved rows; {} changed on disk since, and saving \
                     replaces what is there: {}",
                    changed.len(),
                    changed.join(", ")
                )),
                None => editor.say(format!("Recovered {count} unsaved rows")),
            }
        });
    }

    /// Sets aside the journal of `folder`, which the folder doesn't load with, saying why.
    fn journal_refused(&mut self, folder: &Path, refusal: &EditRefusal) {
        let kept = RecoveryJournal::set_aside(&self.user, folder).map_or(String::new(), |aside| {
            format!("; it is kept at {}", aside.display())
        });
        self.toast(format!(
            "The journal did not apply: {}{kept}",
            refusal.sentence()
        ));
    }

    fn say_opened(&mut self, folder: &Path, project: &Project) {
        match (&project.report, project.last_load) {
            (Some(report), _) => self.say(format!(
                "{} did not load: {} errors",
                folder.display(),
                report.errors.len()
            )),
            (None, Some(took)) => {
                let usages = project.last_usages.map_or(0, |d| d.as_millis());
                self.say(format!(
                    "Opened {} in {} ms (load), {usages} ms (usages) alongside",
                    folder.display(),
                    took.as_millis()
                ));
            }
            _ => {}
        }
    }

    pub(crate) fn close_project(&mut self) {
        self.save_settings();
        if let Some(mut play) = self.play.running.take() {
            play.stop();
        }
        self.views.close();
        if let Some(journal) = &mut self.journal {
            journal.clear();
        }
        self.journal = None;
        self.watcher = None;
        self.lock = None;
        if let Some(project) = self.project.take() {
            project::drop_in_background(project);
        }
        self.dock = DockState::new(Vec::new());
        self.bottom_tab = BottomTab::default();
    }

    pub fn apply(&mut self, edit: ContentEdit) -> bool {
        self.try_apply(edit).is_ok()
    }

    /// Applies `edit`, or says why it was refused. An edit that must wait for the folder's loads
    /// to be in is applied once they are, and one whose check reads more than a frame can afford
    /// is checked off the frame, the editor waiting for either; such an edit is refused by a toast.
    pub fn try_apply(&mut self, edit: ContentEdit) -> Result<(), String> {
        let waits = self.busy.is_busy()
            || self
                .project
                .as_ref()
                .is_some_and(|project| project.waits_on_load(&edit));
        if waits {
            self.when_settled(edit.label.clone(), move |editor| {
                let _ = editor.try_apply(edit);
            });
            return Ok(());
        }
        let viewpoint = EditViewpoint {
            selection: self.selection.clone(),
            camera: self.views.camera_state(),
        };
        let label = edit.label.clone();
        let files: Vec<String> = edit
            .changes
            .iter()
            .map(|change| change.file.clone())
            .collect();
        let prepared = match &mut self.project {
            Some(project) => {
                if edit.changes.iter().any(|change| change.bytes.is_none()) {
                    project.settle();
                }
                project.prepare(edit)
            }
            None => return Err("no folder is open".to_owned()),
        };
        let prepared = match prepared {
            Ok(Some(prepared)) => prepared,
            Ok(None) => return Ok(()),
            Err(refusal) => {
                self.refused(&label, &refusal);
                return Err(refusal.sentence());
            }
        };
        if prepared.weight() > CHECKED_ON_THE_FRAME {
            self.run_busy(
                label.clone(),
                move |_| prepared.check(),
                move |editor, checked| {
                    let _ = editor.commit(&label, &files, checked, viewpoint);
                },
            );
            return Ok(());
        }
        self.commit(&label, &files, prepared.check(), viewpoint)
    }

    /// Takes in the edit labelled `label` of `files` that `checked` checked, or says why it was
    /// refused.
    fn commit(
        &mut self,
        label: &str,
        files: &[String],
        checked: CheckedEdit,
        viewpoint: EditViewpoint,
    ) -> Result<(), String> {
        let outcome = match &mut self.project {
            Some(project) => project.commit(checked, viewpoint),
            None => return Err("no folder is open".to_owned()),
        };
        match outcome {
            Ok(()) => {
                self.form.errors.clear();
                if self
                    .preview
                    .as_ref()
                    .is_some_and(|tab| files.iter().any(|file| tab.shows(file)))
                {
                    self.preview = None;
                }
                Ok(())
            }
            Err(refusal) => {
                self.refused(label, &refusal);
                Err(refusal.sentence())
            }
        }
    }

    pub fn refused(&mut self, label: &str, refusal: &EditRefusal) {
        let text = format!("{label}: refused · {}", refusal.sentence());
        self.toast(text);
        self.last_refusal = Some(refusal.clone());
        if !refusal.pointer.is_empty() {
            self.form
                .errors
                .insert(refusal.pointer.clone(), refusal.sentence());
        }
    }

    pub fn undo(&mut self) {
        self.revisit(Project::undo, "Undid");
    }

    pub(crate) fn redo(&mut self) {
        self.revisit(Project::redo, "Redid");
    }

    pub fn return_to(&mut self, applied: AppliedStep) {
        self.selection = applied.viewpoint.selection;
        self.views.restore_camera(applied.viewpoint.camera);
    }

    fn revisit(&mut self, travel: fn(&mut Project) -> Option<AppliedStep>, verb: &str) {
        if let Some(applied) = self.project.as_mut().and_then(travel) {
            let label = applied.label.clone();
            self.return_to(applied);
            self.toast(format!("{verb} {label}"));
        }
    }

    pub fn save(&mut self) {
        self.save_then(|_, _| {});
    }

    /// Saves every unsaved file once the folder's loads are in, the editor waiting for it, then
    /// runs `then`, saying whether everything was saved.
    pub fn save_then(&mut self, then: impl FnOnce(&mut Editor, bool) + Send + 'static) {
        let Some(project) = &self.project else {
            then(self, false);
            return;
        };
        if project.read_only.is_some() {
            self.toast("Read-only: nothing saved");
            then(self, false);
            return;
        }
        self.when_settled("Saving", move |editor| editor.save_settled(then));
    }

    fn save_settled(&mut self, then: impl FnOnce(&mut Editor, bool) + Send + 'static) {
        let Some(project) = &self.project else {
            then(self, false);
            return;
        };
        if let Some(report) = &project.report {
            let first = EditRefusal::of(report);
            let at = if first.pointer.is_empty() {
                first.file.clone()
            } else {
                format!("{} at {}", first.file, first.pointer)
            };
            self.toast(format!("Not saved · {at}: {}", first.sentence()));
            then(self, false);
            return;
        }
        let saving = project.saving();
        self.run_busy(
            "Saving",
            move |progress| saving.write(progress),
            move |editor, saved| {
                let Some(project) = &mut editor.project else {
                    return then(editor, false);
                };
                let saved = match project.take_saved(saved) {
                    Ok(outcome) => {
                        let count = outcome.written.len() + outcome.removed.len();
                        if let Some(journal) = &mut editor.journal {
                            journal.clear();
                        }
                        editor.toast(format!("Saved {count} file{}", ui::words::plural(count)));
                        true
                    }
                    Err(error) => {
                        editor.toast(format!(
                            "Save stopped at {}: {} ({} written before it)",
                            error.file,
                            error.reason,
                            error.done.written.len()
                        ));
                        false
                    }
                };
                then(editor, saved);
            },
        );
    }

    pub fn open_tab(&mut self, tab: EditorTab) {
        self.open_tabs.push((tab, TabOpening::Kept));
    }

    pub fn preview_tab(&mut self, tab: EditorTab) {
        self.open_tabs.push((tab, TabOpening::Preview));
    }

    /// Puts the tabs asked for since the dock was last drawn in it, in the order they were asked.
    pub fn place_tabs(&mut self) {
        for (tab, opening) in mem::take(&mut self.open_tabs) {
            tabs::place(&mut self.dock, &mut self.preview, tab, opening);
        }
    }

    pub fn serve_dock_requests(&mut self) {
        for request in mem::take(&mut self.dock_requests) {
            tabs::serve(&mut self.dock, request);
        }
    }

    /// Gives every open, queued or previewed tab the new identity `renamed` answers for it, if any.
    pub fn rename_tabs(&mut self, renamed: impl Fn(&EditorTab) -> Option<EditorTab>) {
        let tabs = self
            .dock
            .iter_all_tabs_mut()
            .map(|(_, tab)| tab)
            .chain(self.open_tabs.iter_mut().map(|(tab, _)| tab))
            .chain(self.preview.as_mut());
        for tab in tabs {
            if let Some(fresh) = renamed(tab) {
                *tab = fresh;
            }
        }
    }

    pub fn select(&mut self, selection: RowSelection) {
        self.selection = selection;
    }

    /// Shows the media file at `path` in a preview tab, and in the content tree.
    pub(crate) fn open_media(&mut self, path: &str) {
        self.select(RowSelection::row("asset", path));
        self.preview_tab(EditorTab::Asset {
            path: path.to_owned(),
        });
        crate::content_tree::reveal(self, path);
    }

    /// Opens the content file at `path` in its tab.
    pub(crate) fn open_path(&mut self, path: &str) {
        if let Some(table) = engine::core::content::table_of(path) {
            let table = table.to_owned();
            self.open_row(&table, path);
        }
    }

    pub(crate) fn open_row(&mut self, table: &str, key: &str) {
        self.select(RowSelection::row(table, key));
        self.open_tab(EditorTab::for_row(table, key));
    }

    pub fn preview_row(&mut self, table: &str, key: &str) {
        self.select(RowSelection::row(table, key));
        self.preview_tab(EditorTab::for_row(table, key));
    }

    pub fn has_unsaved(&self) -> bool {
        self.project
            .as_ref()
            .is_some_and(|project| !project.unsaved().is_empty())
    }

    /// The content file at `path` as the game reads it, as the editor holds it now; `None` without
    /// a project.
    pub fn read<Row: ContentFile>(&self, path: &str) -> Option<Result<Arc<Row>, Arc<Unreadable>>> {
        Some(self.project.as_ref()?.read::<Row>(path))
    }

    /// What reads the project's files as they stand now; `None` without a project.
    pub fn reader(&self) -> Option<Arc<FolderReader>> {
        Some(self.project.as_ref()?.reader())
    }

    /// Writes `row` as the content file at `path`, as one History step labelled `label`.
    pub fn write_content<Row: ContentFile>(&mut self, label: &str, path: &str, row: &Row) -> bool {
        self.write_content_as(ContentEdit::new(label), path, row)
    }

    /// Writes `row` as the content file at `path` as part of `edit`, merging or amending as it
    /// says.
    pub(crate) fn write_content_as<Row: ContentFile>(
        &mut self,
        mut edit: ContentEdit,
        path: &str,
        row: &Row,
    ) -> bool {
        let Some(project) = &self.project else {
            return false;
        };
        if let Err(reason) = edit.write_content(project, path, row) {
            self.toast(format!("{}: {reason}", edit.label));
            return false;
        }
        self.apply(edit)
    }

    /// Writes the content file at `path` as `change` leaves it, as part of `edit`. Writes nothing
    /// when `change` refuses, which is told to the designer, or leaves the file as it was.
    pub fn change<Row: ContentFile + Clone>(
        &mut self,
        edit: ContentEdit,
        path: &str,
        change: impl FnOnce(&mut Row) -> Result<(), String>,
    ) -> bool {
        let Some(Ok(row)) = self.read::<Row>(path) else {
            return false;
        };
        let mut changed = (*row).clone();
        if let Err(refusal) = change(&mut changed) {
            self.toast(format!("{}: {refusal}", tabs::shown_path(path)));
            return false;
        }
        let reader = self.reader();
        let unchanged = reader.is_some_and(|reader| {
            matches!((reader.to_json(&changed), reader.to_json(&*row)), (Ok(now), Ok(before)) if now == before)
        });
        if unchanged {
            return false;
        }
        self.write_content_as(edit, path, &changed)
    }

    /// The fields of the content file at `path` as raw JSON, as the editor holds them now: for
    /// the forms that walk a file by its type's schema, which know no field by name. Code that
    /// names a field reads the file as its type with `read`.
    pub fn fields_json(&self, path: &str) -> Option<Value> {
        self.project.as_ref()?.files.json_owned(&fields_file(path))
    }

    pub fn thumb(&mut self, ctx: &egui::Context, path: &str) -> Option<Thumbnail> {
        let project = self.project.as_ref()?;
        self.form.thumbnails.image(ctx, &project.files, path)
    }

    pub(crate) fn thumb_icon(&mut self, ctx: &egui::Context, path: &str) -> Option<Thumbnail> {
        let project = self.project.as_ref()?;
        let mut budget = u32::MAX;
        self.form
            .thumbnails
            .icon(ctx, &project.files, path, &mut budget)
    }

    pub fn loaded(&self) -> Option<Content> {
        self.project
            .as_ref()
            .and_then(|project| project.content.clone())
    }

    /// Shows a form owned by `key` of `table` against the schema `root`, then carries out the
    /// actions its fields raised; nothing is shown without a project.
    pub(crate) fn with_form<R>(
        &mut self,
        table: &str,
        key: Option<&str>,
        root: &Value,
        show: impl FnOnce(&mut FormCx) -> R,
    ) -> Option<R> {
        let mut actions = Vec::new();
        let shown = {
            let Editor {
                project,
                form,
                shapes,
                ..
            } = self;
            let project = project.as_ref()?;
            let mut cx = FormCx::new(project, root, shapes, form, &mut actions).editing(table, key);
            show(&mut cx)
        };
        for action in actions {
            row_form::act(self, action, table, key.unwrap_or_default());
        }
        Some(shown)
    }

    /// Takes in a load that finished, and says why it took back an edit, unless a step the editor
    /// waits on is to say that.
    fn poll_project(&mut self) {
        let Some(project) = &mut self.project else {
            return;
        };
        project.poll();
        if !self.busy.is_busy() {
            self.take_refused();
        }
    }

    /// Says why the last load took back an edit, when it did.
    fn take_refused(&mut self) {
        let Some(refused) = self.project.as_mut().and_then(Project::take_refused) else {
            return;
        };
        self.refused(&refused.label, &refused.refusal);
    }

    fn poll_outside(&mut self) {
        if self.project.is_none()
            && let Some(watch) = &mut self.start.watch
            && watch.poll().is_some()
            && let Some((folder, _)) = self.start.report.clone()
        {
            self.open_now(&folder, Vec::new());
            return;
        }
        let Some(watcher) = &mut self.watcher else {
            return;
        };
        let Some(changed) = watcher.poll() else {
            return;
        };
        self.when_settled("Reading changes on disk", move |editor| {
            let Some(project) = &editor.project else {
                return;
            };
            let reading = project.reading_disk(changed);
            editor.run_busy(
                "Reading changes on disk",
                move |progress| reading.read(progress),
                |editor, read| editor.take_outside(read),
            );
        });
    }

    /// Takes in what changed on disk, then once the folder loads again says how that went.
    fn take_outside(&mut self, read: project::DiskRead) {
        let Some(project) = &mut self.project else {
            return;
        };
        let outcome = project.take_outside(read);
        if !outcome.conflicts.is_empty() {
            self.dialogs.push(ModalDialog::Conflicts(outcome.conflicts));
        }
        let taken = outcome.taken.len();
        if taken == 0 {
            return;
        }
        self.when_settled("Loading the changed files", move |editor| {
            editor.reloaded_outside(taken);
        });
    }

    /// Says how the folder loaded after `taken` files were taken in from disk.
    fn reloaded_outside(&mut self, taken: usize) {
        let Some(project) = &mut self.project else {
            return;
        };
        project.reloaded_outside(taken);
        let moved = project.moved_outside.clone();
        let text = if project.loads() {
            format!("{taken} files changed on disk and were reloaded")
        } else if !moved.is_empty() && !project.broken_on_disk() {
            for step in &moved {
                self.output.say(
                    "editor",
                    format!(
                        "{} reference{} name{} {}, which isn't there; a file of that name is at {}",
                        step.references,
                        words::plural(step.references),
                        if step.references == 1 { "s" } else { "" },
                        step.from,
                        step.to
                    ),
                );
            }
            self.bottom_tab = BottomTab::Output;
            crate::moves::moved_outside_summary(&moved, &project.files.index())
        } else {
            format!("{taken} files changed on disk and the folder no longer loads")
        };
        let text = crate::views::area::drop_gone(self).unwrap_or(text);
        self.toast(text);
    }

    /// Takes the disk's side of `conflict`, once the folder's loads are in, the editor waiting for
    /// it and for the folder to load again.
    pub(crate) fn take_disk(&mut self, conflict: project::FileConflict) {
        self.when_settled("Taking the file from disk", move |editor| {
            if let Some(project) = &mut editor.project {
                project.take_disk(&conflict);
            }
            editor.when_settled("Loading the changed file", |editor| {
                if let Some(project) = &mut editor.project {
                    project.reloaded_outside(1);
                }
            });
        });
    }

    /// Has the journal keep the unsaved files when they changed since it last did, made and
    /// written on its own thread.
    fn tick_journal(&mut self) {
        let Some(project) = &self.project else {
            return;
        };
        let Some(journal) = &mut self.journal else {
            return;
        };
        let files = project.files.clone();
        let revision = JournalRevision {
            files: files.revision(),
            disk: files.disk_revision(),
        };
        if !journal.due(revision) {
            return;
        }
        let unsaved = project.unsaved();
        if unsaved.is_empty() {
            journal.clear();
            return;
        }
        let folder = project.root.clone();
        journal.write(revision, move || RecoveryJournalData {
            build: version::build().to_owned(),
            content_version: content_version().to_owned(),
            folder,
            written: SystemTime::now().into(),
            files: unsaved
                .iter()
                .map(|path| {
                    RecoveryJournalFile::of(
                        path,
                        files.disk_hash(path).as_deref(),
                        files.get(path).as_ref(),
                    )
                })
                .collect(),
        });
    }
}

fn editor_ui(world: &mut World) {
    let ctx = {
        let mut contexts = world.query_filtered::<&mut EguiContext, With<PrimaryEguiContext>>();
        let Ok(mut context) = contexts.single_mut(world) else {
            return;
        };
        context.get_mut().clone()
    };
    let frame_ms = world.resource::<Time<Real>>().delta_secs() * 1000.0;
    let (ui_ms, tab, camera) = world.resource_scope(|world, mut editor: Mut<Editor>| {
        let started = Instant::now();
        shell::show(&mut editor, &ctx);
        if editor.settings.frame_rate {
            frame_rate::show_meter(
                &ctx,
                world.resource::<FrameRates>(),
                egui::Order::Foreground,
                egui::Align2::RIGHT_BOTTOM,
                egui::vec2(
                    -FRAME_RATE_MARGIN,
                    -shell::STATUS_HEIGHT - FRAME_RATE_MARGIN,
                ),
            );
        }
        let ui_ms = started.elapsed().as_secs_f32() * 1000.0;
        (
            ui_ms,
            crate::tabs::active_tab(&editor).map(|tab| tab.title()),
            editor
                .views
                .area
                .camera_state()
                .map(|state| state.camera.distance),
        )
    });
    if world.contains_resource::<FrameLog>() {
        let record = FrameRecord {
            frame: Millis(frame_ms),
            ui: Millis(ui_ms),
            tab,
            camera,
            entities: world.query::<Entity>().iter(world).count(),
            images: world.resource::<Assets<Image>>().len(),
            meshes: world.resource::<Assets<Mesh>>().len(),
            scenery: SceneryState::of(world),
        };
        let mut log = world.resource_mut::<FrameLog>();
        let _ = serde_json::to_writer(&mut log.0, &record)
            .map_err(std::io::Error::from)
            .and_then(|()| writeln!(log.0))
            .and_then(|()| log.0.flush());
    }
    if world.resource::<Editor>().quit {
        world.write_message(bevy::app::AppExit::Success);
    }
}

/// The editor's window and views are shown as its video settings ask.
fn show_video(editor: Res<Editor>, mut video: ResMut<VideoPreferences>) {
    video.set_if_neq(editor.settings.video);
}

fn background(world: &mut World) {
    world.resource_scope(|world, mut editor: Mut<Editor>| {
        editor.poll_project();
        editor.poll_busy();
        // What the editor waits on may still change the files, such as a journal being recovered
        // or a save being written, so neither the disk nor the journal is looked at meanwhile.
        if !editor.busy.is_busy() {
            editor.poll_outside();
            editor.tick_journal();
        }
        play::poll(&mut editor);
        views::sync(&mut editor, world);
    });
}

pub fn content_version() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| systems::registry().version().0)
}
