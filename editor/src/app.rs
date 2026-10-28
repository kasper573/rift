use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime};
use ui::egui;
use ui::words;

use bevy::app::MainScheduleOrder;
use bevy::prelude::*;
use bevy_egui::{
    EguiContext, EguiPlugin, EguiPreUpdateSet, EguiPrimaryContextPass, PrimaryEguiContext,
};
use engine::core::content::{Content, ContentFile, FolderReader, Unreadable, fields_file, version};
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems;
use engine::systems::area::AreaModuleKind;
use engine::systems::frame_rate::{self, FrameCountPlugin, FrameRates};
use serde::Deserialize;
use serde_json::Value;
use strum::IntoStaticStr;
use ui::egui_dock::DockState;

use crate::forms::{FormCx, FormState};
use crate::play::{self, PlayTest};
use crate::project::journal::{RecoveryJournal, RecoveryJournalData, RecoveryJournalFile};
use crate::project::lock::ProjectLock;
use crate::project::watch::FolderWatcher;
use crate::project::{
    AppliedStep, ContentEdit, EditRefusal, EditViewpoint, FileConflict, Project, RowSelection,
};
use crate::rows;
use crate::scenes::{self, SceneStep};
use crate::schema::SchemaShapes;
use crate::shell::{self, row_form};
use crate::tabs::{self, DockRequest, EditorTab, TabOpening};
use crate::thumbs::Thumbnail;
use crate::user::{EditorSettings, UserFolder};
use crate::views::area::ResizeDialog;
use crate::views::area::nodes::CopiedNodes;
use crate::views::script::{LinkNodeDialog, RenameNodeDialog, ScriptRow};
use crate::views::world::ArrangeDialog;
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
        editor.frame_log = self
            .launch
            .frame_log
            .as_ref()
            .and_then(|path| File::create(path).ok())
            .map(BufWriter::new);
        if let Some(folder) = &self.launch.folder {
            editor.open_folder(folder);
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
            .add_systems(Background, background);
    }
}

const FRAME_RATE_MARGIN: f32 = 8.0;

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
    pub panels: DockTabs,
    pub content_tree: crate::shell::content_tree::ContentTree,
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
    pub quit: bool,
    pub frame: u64,
    pub frame_log: Option<BufWriter<File>>,
}

#[derive(Default)]
pub struct OutputLog {
    pub lines: Vec<OutputLine>,
}

#[derive(Clone)]
pub struct OutputLine {
    pub tag: &'static str,
    pub text: String,
}

pub const OUTPUT_KEPT: usize = 20_000;

impl OutputLog {
    pub fn say(&mut self, tag: &'static str, text: impl Into<String>) {
        let text = text.into();
        eprintln!("[{tag}] {text}");
        self.lines.push(OutputLine { tag, text });
        if self.lines.len() > OUTPUT_KEPT {
            let drop = self.lines.len() - OUTPUT_KEPT;
            self.lines.drain(..drop);
        }
    }

    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|line| format!("[{}] {}", line.tag, line.text))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Default)]
pub struct DockTabs {
    pub bottom: BottomTab,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, IntoStaticStr)]
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

pub enum ModalDialog {
    NewGame(NewGameDialog),
    Confirm(ConfirmDialog),
    Conflicts(Vec<FileConflict>),
    Delete(DeleteDialog),
    FindUnused(FindUnusedDialog),
    Close(CloseDialog),
    Binaries,
    Keys,
    Resize(ResizeDialog),
    Arrange(ArrangeDialog),
    PickRow(PickRowDialog),
    RenameNode(RenameNodeDialog),
    LinkNode(LinkNodeDialog),
    Import(crate::import::ImportDialog),
}

pub struct PickRowDialog {
    pub title: String,
    pub table: String,
    pub search: String,
    pub picked: Option<String>,
    pub options: Vec<String>,
    pub then: PickRowThen,
}

pub enum PickRowThen {
    /// Adds a module of `kind` at `at` to `area`, naming the picked file.
    AddModule {
        area: String,
        kind: AreaModuleKind,
        at: Option<Pos<Meters>>,
    },
    ReplaceBlueprint {
        area: String,
        node: String,
    },
    NewFile {
        folder: String,
    },
    MoveInto {
        paths: Vec<String>,
    },
    /// Moves rows of a scene graph into the folder picked, by its labels.
    MoveNodes {
        bundle: String,
        ids: Vec<String>,
        folders: Vec<(String, String)>,
    },
}

impl PickRowThen {
    /// The file the picked row is written into, when it is one.
    pub fn editing(&self) -> Option<&str> {
        match self {
            PickRowThen::AddModule { area, .. } | PickRowThen::ReplaceBlueprint { area, .. } => {
                Some(area)
            }
            _ => None,
        }
    }
}

/// What was copied last: nodes of a scene, or modules of a row's list.
#[derive(Clone)]
pub enum Clipboard {
    Scene(SceneClip),
    Modules(ModuleClip),
}

/// Nodes copied out of the scene of the area or blueprint `from`, moved so what they cover starts
/// at the map's top-left; `origin` is where it started.
#[derive(Clone)]
pub struct SceneClip {
    pub from: String,
    pub nodes: CopiedNodes,
    pub count: usize,
    pub origin: Pos<Meters>,
}

/// Modules copied out of the list of the row `from`.
#[derive(Clone)]
pub struct ModuleClip {
    pub from: String,
    pub modules: Vec<Value>,
}

impl Clipboard {
    pub fn scene(&self) -> Option<&SceneClip> {
        match self {
            Clipboard::Scene(clip) => Some(clip),
            Clipboard::Modules(_) => None,
        }
    }
}

pub struct NewGameDialog {
    pub source: NewGameSource,
    pub name: String,
    pub folder: String,
    pub folder_touched: bool,
    pub error: Option<String>,
}

pub enum NewGameSource {
    Template(String),
    Copy(PathBuf),
}

pub struct ConfirmDialog {
    pub title: String,
    pub body: String,
    pub label: String,
    pub compare: Option<TextComparison>,
    pub action: ConfirmAction,
}

pub struct TextComparison {
    pub left_title: String,
    pub left: String,
    pub right_title: String,
    pub right: String,
}

pub enum ConfirmAction {
    ReplaceModule {
        table: String,
        key: String,
        index: usize,
        module: Value,
    },
    DeleteScriptNodes {
        script: ScriptRow,
        names: Vec<String>,
    },
}

impl ConfirmAction {
    pub fn destroys(&self) -> bool {
        match self {
            ConfirmAction::ReplaceModule { .. } => false,
            ConfirmAction::DeleteScriptNodes { .. } => true,
        }
    }
}

pub struct DeleteDialog {
    /// The files and folders picked to delete.
    pub paths: Vec<String>,
    /// Every content file and media file that goes with them.
    pub gone: Vec<String>,
    pub groups: Vec<rows::UsageGroup>,
    pub error: Option<String>,
}

pub struct FindUnusedDialog {
    /// What is left out of the deletion; everything else found is in it.
    pub unticked: BTreeSet<crate::unused::UnusedItem>,
    pub open: BTreeSet<String>,
    pub error: Option<String>,
}

pub struct CloseDialog {
    pub then: CloseThen,
}

#[derive(Clone)]
pub enum CloseThen {
    Quit,
    Close,
    Open(PathBuf),
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
            panels: DockTabs::default(),
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
            quit: false,
            frame: 0,
            frame_log: None,
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

    pub fn copy_text(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.toast(format!("Copied {text}"));
        self.copied = Some(text);
    }

    pub fn save_settings(&self) {
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

    pub fn open_now(&mut self, folder: &Path, recovered: Vec<RecoveryJournalFile>) {
        self.close_project();
        match Project::open(folder, self.shapes.clone()) {
            Ok(project) if project.report.is_some() && recovered.is_empty() => {
                let report = project
                    .report
                    .as_ref()
                    .map(|report| report.to_string())
                    .unwrap_or_default();
                self.say(format!(
                    "{} did not load: {} errors",
                    folder.display(),
                    project.report.as_ref().map_or(0, |r| r.errors.len())
                ));
                self.settings.remember(folder);
                self.save_settings();
                self.start.failed = None;
                self.start.report = Some((folder.to_path_buf(), report));
                self.start.watch = Some(FolderWatcher::on(folder));
            }
            Ok(mut project) => {
                let lock = match ProjectLock::take(&self.user, folder) {
                    Ok(lock) => Some(lock),
                    Err(reason) => {
                        project.read_only = Some(reason.clone());
                        self.say(format!("Read-only: {reason}"));
                        None
                    }
                };
                if !recovered.is_empty() {
                    let mut edit = ContentEdit::new("Recover unsaved rows from the journal");
                    for file in &recovered {
                        let change = file.change();
                        match change.bytes {
                            Some(bytes) => edit.write(change.file, bytes),
                            None => edit.remove(change.file),
                        };
                    }
                    let changed: Vec<&str> = recovered
                        .iter()
                        .filter(|file| file.changed_on_disk(project.disk_hash(&file.path)))
                        .map(|file| file.path.as_str())
                        .collect();
                    let applied = project
                        .apply(edit, EditViewpoint::default())
                        .and_then(|()| {
                            project.settle();
                            match project.take_refused() {
                                Some(refused) => Err(Box::new(refused.refusal)),
                                None => Ok(()),
                            }
                        });
                    match applied {
                        Err(refusal) => {
                            let kept = RecoveryJournal::set_aside(&self.user, folder)
                                .map_or(String::new(), |aside| {
                                    format!("; it is kept at {}", aside.display())
                                });
                            self.toast(format!(
                                "The journal did not apply: {}{kept}",
                                refusal.sentence()
                            ));
                        }
                        Ok(()) if !changed.is_empty() => self.toast(format!(
                            "Recovered {} unsaved rows; {} changed on disk since, and saving \
                             replaces what is there: {}",
                            recovered.len(),
                            changed.len(),
                            changed.join(", ")
                        )),
                        Ok(()) => self.say(format!("Recovered {} unsaved rows", recovered.len())),
                    }
                }
                match (&project.report, project.last_load) {
                    (Some(report), _) => {
                        self.say(format!(
                            "{} did not load: {} errors",
                            folder.display(),
                            report.errors.len()
                        ));
                    }
                    (None, Some(took)) => {
                        let usages = project.last_usages.map_or(0, |d| d.as_millis());
                        self.say(format!(
                            "Opened {} in {} ms (load) + {usages} ms (usages)",
                            folder.display(),
                            took.as_millis()
                        ));
                    }
                    _ => {}
                }
                self.settings.remember(folder);
                self.save_settings();
                self.lock = lock;
                self.watcher = Some(FolderWatcher::on(folder));
                self.journal = Some(RecoveryJournal::for_folder(&self.user, folder));
                self.project = Some(project);
                self.dock = DockState::new(Vec::new());
                self.panels = DockTabs::default();
                self.selection = RowSelection::default();
                self.form = FormState::default();
                self.views = EditorViews::default();
                self.start.failed = None;
                self.start.report = None;
                self.start.watch = None;
                play::check_binaries(self);
                if let Some(scene) = self.scene.take() {
                    scenes::open(self, &scene);
                }
            }
            Err(reason) => {
                self.start.failed = Some((folder.to_path_buf(), reason.clone()));
                self.say(reason);
            }
        }
    }

    pub fn close_project(&mut self) {
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
        self.project = None;
        self.dock = DockState::new(Vec::new());
        self.panels = DockTabs::default();
    }

    pub fn apply(&mut self, edit: ContentEdit) -> bool {
        self.try_apply(edit).is_ok()
    }

    pub fn try_apply(&mut self, edit: ContentEdit) -> Result<(), String> {
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
        let outcome = match &mut self.project {
            Some(project) => project.apply(edit, viewpoint),
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
                self.refused(&label, &refusal);
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

    pub fn redo(&mut self) {
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

    pub fn save(&mut self) -> bool {
        let Some(project) = &mut self.project else {
            return false;
        };
        if project.read_only.is_some() {
            self.toast("Read-only: nothing saved");
            return false;
        }
        project.settle();
        let Some(project) = &mut self.project else {
            return false;
        };
        if let Some(report) = &project.report {
            let first = EditRefusal::of(report);
            let at = if first.pointer.is_empty() {
                first.file.clone()
            } else {
                format!("{} at {}", first.file, first.pointer)
            };
            self.toast(format!("Not saved · {at}: {}", first.sentence()));
            return false;
        }
        match project.save() {
            Ok(outcome) => {
                let count = outcome.written.len() + outcome.removed.len();
                if let Some(journal) = &mut self.journal {
                    journal.clear();
                }
                self.toast(format!("Saved {count} file{}", ui::words::plural(count)));
                true
            }
            Err(error) => {
                let text = format!(
                    "Save stopped at {}: {} ({} written before it)",
                    error.file,
                    error.reason,
                    error.done.written.len()
                );
                self.toast(text);
                false
            }
        }
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
    pub fn open_media(&mut self, path: &str) {
        self.select(RowSelection::row("asset", path));
        self.preview_tab(EditorTab::Asset {
            path: path.to_owned(),
        });
        crate::shell::content_tree::reveal(self, path);
    }

    /// Opens the content file at `path` in its tab.
    pub fn open_path(&mut self, path: &str) {
        if let Some(table) = engine::core::content::table_of(path) {
            let table = table.to_owned();
            self.open_row(&table, path);
        }
    }

    pub fn open_row(&mut self, table: &str, key: &str) {
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
    pub fn write_content_as<Row: ContentFile>(
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

    pub fn thumb_icon(&mut self, ctx: &egui::Context, path: &str) -> Option<Thumbnail> {
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
    pub fn with_form<R>(
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

    fn poll_project(&mut self) {
        let Some(project) = &mut self.project else {
            return;
        };
        project.poll();
        self.take_refused();
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
        let Some(project) = &mut self.project else {
            return;
        };
        let outcome = project.outside_changes(changed);
        if !outcome.taken.is_empty() {
            let moved = project.moved_outside.clone();
            let text = if project.loads() {
                format!(
                    "{} files changed on disk and were reloaded",
                    outcome.taken.len()
                )
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
                self.panels.bottom = BottomTab::Output;
                crate::moves::moved_outside_summary(&moved, &project.files.index())
            } else {
                format!(
                    "{} files changed on disk and the folder no longer loads",
                    outcome.taken.len()
                )
            };
            let text = crate::views::area::drop_gone(self).unwrap_or(text);
            self.toast(text);
        }
        if !outcome.conflicts.is_empty() {
            self.dialogs.push(ModalDialog::Conflicts(outcome.conflicts));
        }
    }

    fn tick_journal(&mut self) {
        let Some(project) = &self.project else {
            return;
        };
        let Some(journal) = &mut self.journal else {
            return;
        };
        if !journal.due() {
            return;
        }
        let unsaved = project.unsaved();
        if unsaved.is_empty() {
            journal.clear();
            return;
        }
        let data = RecoveryJournalData {
            build: version::build().to_owned(),
            content_version: content_version().to_owned(),
            folder: project.root.clone(),
            written: SystemTime::now().into(),
            files: unsaved
                .iter()
                .map(|path| {
                    RecoveryJournalFile::of(path, project.disk_hash(path), project.files.get(path))
                })
                .collect(),
        };
        journal.write(&data);
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
    let frame_ms = world.resource::<Time>().delta_secs() * 1000.0;
    world.resource_scope(|world, mut editor: Mut<Editor>| {
        editor.frame += 1;
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
        let tab = shell::active_tab(&editor).map(|tab| tab.title());
        if let Some(log) = &mut editor.frame_log {
            let entities = world.query::<Entity>().iter(world).count();
            let images = world.resource::<Assets<Image>>().len();
            let meshes = world.resource::<Assets<Mesh>>().len();
            let _ = writeln!(
                log,
                "{frame_ms:.3}\t{ui_ms:.3}\t{}\t{entities}\t{images}\t{meshes}",
                tab.unwrap_or_default()
            )
            .and_then(|()| log.flush());
        }
    });
    if world.resource::<Editor>().quit {
        world.write_message(bevy::app::AppExit::Success);
    }
}

fn background(world: &mut World) {
    world.resource_scope(|world, mut editor: Mut<Editor>| {
        editor.poll_project();
        editor.poll_outside();
        editor.tick_journal();
        play::poll(&mut editor);
        views::sync(&mut editor, world);
    });
}

pub fn content_version() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| systems::registry().version().0)
}

// A `Receiver` is `!Sync`, and everything the editor holds lives in one bevy resource, so each
// channel end is wrapped once here instead of at every field.
pub struct ChannelInbox<T>(Mutex<Receiver<T>>);

impl<T> ChannelInbox<T> {
    pub fn new(receiver: Receiver<T>) -> ChannelInbox<T> {
        ChannelInbox(Mutex::new(receiver))
    }

    pub fn drain(&self) -> Vec<T> {
        self.0
            .lock()
            .map(|receiver| receiver.try_iter().collect())
            .unwrap_or_default()
    }

    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        match self.0.lock() {
            Ok(receiver) => receiver.try_recv(),
            Err(_) => Err(TryRecvError::Disconnected),
        }
    }

    pub fn recv(&self) -> Option<T> {
        self.0.lock().ok().and_then(|receiver| receiver.recv().ok())
    }
}
