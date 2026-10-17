use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use bevy::prelude::*;
use bevy_egui::{
    EguiContext, EguiPlugin, EguiPreUpdateSet, EguiPrimaryContextPass, PrimaryEguiContext,
};
use egui_dock::DockState;
use engine::core::content::{Content, ContentRegistry, version};
use engine::core::math::Pos;
use engine::core::tiling::Tiles;
use engine::systems;
use engine::systems::script::Place;
use serde::Deserialize;
use serde_json::Value;

use crate::forms::FormState;
use crate::project::journal::{RecoveryJournal, RecoveryJournalData, RecoveryJournalFile};
use crate::project::lock::ProjectLock;
use crate::project::watch::FolderWatcher;
use crate::project::{
    ContentEdit, EditRefusal, EditViewpoint, FileConflict, Project, RowSelection, row_file,
};
use crate::scenes::{self, Step};
use crate::schema::SchemaShapes;
use crate::tabs::{CloseTabs, EditorTab};
use crate::thumbs::Thumbnail;
use crate::user::{BrushSettings, EditorSettings, UserFolder, unix_seconds};
use crate::views::area::placement::ReplaceScope;
use crate::views::area::{DeleteBlueprintDialog, MakeBlueprintDialog, ResizeDialog};
use crate::views::script::inspect::{MakeInlineDialog, MakeSharedDialog};
use crate::views::script::{LinkNodeDialog, RenameNodeDialog, ScriptOwner};
use crate::views::world::ArrangeDialog;
use crate::views::{EditorCamera, EditorViews};
use crate::{import, play, rows, ui, views};
use engine::core::vfx::render::VfxMaterial;

#[derive(Clone, Debug, Default)]
pub struct EditorLaunch {
    pub folder: Option<PathBuf>,
    pub scene: Option<Vec<Step>>,
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
            .and_then(|path| std::fs::File::create(path).ok())
            .map(std::io::BufWriter::new);
        if let Some(folder) = &self.launch.folder {
            editor.open_folder(folder);
        }
        app.add_plugins((
            EguiPlugin::default(),
            bevy_tilemap::TilemapPlugin,
            bevy::sprite_render::Material2dPlugin::<VfxMaterial>::default(),
        ))
        .insert_resource(editor)
        .add_systems(Startup, setup_camera)
        .add_systems(
            PreUpdate,
            theme_context.after(EguiPreUpdateSet::InitContexts),
        )
        .add_systems(EguiPrimaryContextPass, editor_ui)
        .add_systems(Update, background);
    }
}

fn setup_camera(mut commands: Commands) {
    commands.spawn((Camera2d, PrimaryEguiContext, EditorCamera));
}

// egui takes up new fonts as a pass begins, so they must be set before the first pass draws text.
fn theme_context(mut contexts: Query<&mut EguiContext, Added<PrimaryEguiContext>>) {
    for mut context in &mut contexts {
        ui::theme::apply(context.get_mut());
    }
}

pub const REFUSED: &str = "refused; see the output";

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
    pub status: StatusBar,
    pub last_refusal: Option<EditRefusal>,
    pub history_focus: Option<usize>,
    pub selection: RowSelection,
    pub form: FormState,
    pub output: OutputLog,
    pub dialogs: Vec<ModalDialog>,
    pub toast: Option<(String, Instant)>,
    pub start: StartScreen,
    pub open_tabs: Vec<EditorTab>,
    pub close_tabs: Option<CloseTabs>,
    pub scene: Option<Vec<Step>>,
    pub clipboard: Option<Clipboard>,
    pub views: EditorViews,
    pub play: play::PlayTest,
    pub imports: import::ImportRecords,
    pub jobs: import::ImportJobs,
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
    pub left: LeftTab,
    pub right: RightTab,
    pub bottom: BottomTab,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LeftTab {
    #[default]
    Content,
    Assets,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RightTab {
    #[default]
    Inspector,
    UsedBy,
    Preview,
    Conflicts,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BottomTab {
    #[default]
    History,
    Output,
    Palette,
    Blueprints,
    MayDo,
    UsedBy,
    Run,
    Timeline,
    Sheet,
    Sounds,
    Surfaces,
    Mixes,
    Listen,
}

impl BottomTab {
    pub fn title(self) -> &'static str {
        match self {
            BottomTab::History => "History",
            BottomTab::Output => "Output",
            BottomTab::Palette => "Palette",
            BottomTab::Blueprints => "Blueprints",
            BottomTab::MayDo => "May do",
            BottomTab::UsedBy => "Used by",
            BottomTab::Run => "Run",
            BottomTab::Timeline => "Timeline",
            BottomTab::Sheet => "Sheet",
            BottomTab::Sounds => "Sounds",
            BottomTab::Surfaces => "Surfaces",
            BottomTab::Mixes => "Mixes",
            BottomTab::Listen => "Listen",
        }
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
    NewRow(NewRowDialog),
    Confirm(ConfirmDialog),
    Conflicts(Vec<FileConflict>),
    Error(String, String),
    Rename(RenameDialog),
    Delete(DeleteDialog),
    Close(CloseDialog),
    Binaries,
    Keys,
    MakeBlueprint(MakeBlueprintDialog),
    Resize(ResizeDialog),
    DeleteBlueprint(DeleteBlueprintDialog),
    Arrange(ArrangeDialog),
    Import(Box<import::ImportDialog>),
    PickRow(PickRowDialog),
    MakeShared(MakeSharedDialog),
    MakeInline(MakeInlineDialog),
    RenameNode(RenameNodeDialog),
    LinkNode(LinkNodeDialog),
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
    AddModule {
        area: String,
        module: Value,
        field: String,
    },
    ReplaceBlueprint {
        area: String,
        index: usize,
        scope: ReplaceScope,
    },
}

#[derive(Clone)]
pub struct Clipboard {
    pub table: String,
    pub from: String,
    pub modules: Vec<Value>,
    pub origin: Option<Pos<Tiles>>,
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

pub struct NewRowDialog {
    pub table: String,
    pub key: String,
    pub value: Value,
    pub error: Option<String>,
    pub into: Option<(String, String)>,
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
    RemoveLayer {
        area: String,
        layer: String,
    },
    DeleteScriptNodes {
        owner: ScriptOwner,
        names: Vec<String>,
    },
    RestartPlay {
        start: Option<Place>,
    },
}

pub struct RenameDialog {
    pub target: RenameTarget,
    pub draft: String,
    pub error: Option<String>,
}

pub enum RenameTarget {
    Row { table: String, key: String },
    Layer { area: String, layer: String },
}

impl RenameTarget {
    pub fn name(&self) -> &str {
        match self {
            RenameTarget::Row { key, .. } => key,
            RenameTarget::Layer { layer, .. } => layer,
        }
    }
}

pub struct DeleteDialog {
    pub table: String,
    pub keys: Vec<String>,
    pub groups: Vec<rows::UsageGroup>,
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
            status: StatusBar::default(),
            last_refusal: None,
            history_focus: None,
            selection: RowSelection::default(),
            form: FormState::default(),
            output: OutputLog::default(),
            dialogs: Vec::new(),
            toast: None,
            start: StartScreen::default(),
            open_tabs: Vec::new(),
            close_tabs: None,
            scene: None,
            clipboard: None,
            views: EditorViews::default(),
            play: crate::play::PlayTest::default(),
            imports: crate::import::ImportRecords::default(),
            jobs: crate::import::ImportJobs::default(),
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
        self.output.say("editor", text.clone());
        self.toast = Some((text, Instant::now()));
    }

    pub fn save_settings(&self) {
        self.user.save_settings(&self.settings);
    }

    pub fn open_folder(&mut self, folder: &Path) {
        let folder = folder.to_path_buf();
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
                self.output.say(
                    "editor",
                    format!(
                        "{} did not load: {} errors",
                        folder.display(),
                        project.report.as_ref().map_or(0, |r| r.errors.len())
                    ),
                );
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
                        self.output.say("editor", format!("Read-only: {reason}"));
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
                    if let Err(refusal) = project.apply(edit, EditViewpoint::default()) {
                        self.output.say(
                            "editor",
                            format!("The journal did not apply: {}", refusal.sentence()),
                        );
                    } else {
                        self.output.say(
                            "editor",
                            format!("Recovered {} unsaved rows", recovered.len()),
                        );
                    }
                }
                match (&project.report, project.last_load) {
                    (Some(report), _) => {
                        self.output.say(
                            "editor",
                            format!(
                                "{} did not load: {} errors",
                                folder.display(),
                                report.errors.len()
                            ),
                        );
                    }
                    (None, Some(took)) => {
                        let usages = project.last_usages.map_or(0, |d| d.as_millis());
                        self.output.say(
                            "editor",
                            format!(
                                "Opened {} in {} ms (load) + {usages} ms (usages)",
                                folder.display(),
                                took.as_millis()
                            ),
                        );
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
                self.imports = crate::import::ImportRecords::load(&self.user, folder);
                if let Some(problem) = self.imports.unreadable.clone() {
                    self.toast(format!(
                        "Import records are unreadable and will not be saved: {problem}"
                    ));
                }
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
                self.output.say("editor", reason);
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
        if let Some(project) = self.project.take() {
            drop(project);
        }
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
        let outcome = match &mut self.project {
            Some(project) => project.apply(edit, viewpoint),
            None => return Err("no folder is open".to_owned()),
        };
        match outcome {
            Ok(()) => {
                self.form.errors.clear();
                self.views.content_changed();
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
        let Some(project) = &mut self.project else {
            return;
        };
        match project.undo() {
            Ok(Some(applied)) => {
                self.selection = applied.viewpoint.selection;
                self.views.restore_camera(applied.viewpoint.camera);
                self.views.content_changed();
                self.toast(format!("Undid {}", applied.label));
            }
            Ok(None) => {}
            Err(refusal) => self.refused("Undo", &refusal),
        }
    }

    pub fn redo(&mut self) {
        let Some(project) = &mut self.project else {
            return;
        };
        match project.redo() {
            Ok(Some(applied)) => {
                self.selection = applied.viewpoint.selection;
                self.views.restore_camera(applied.viewpoint.camera);
                self.views.content_changed();
                self.toast(format!("Redid {}", applied.label));
            }
            Ok(None) => {}
            Err(refusal) => self.refused("Redo", &refusal),
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
        let was_loading = project.loading();
        project.settle();
        if was_loading {
            self.views.content_changed();
        }
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
                self.toast(format!("Saved {count} files"));
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
        self.open_tabs.push(tab);
    }

    pub fn select(&mut self, selection: RowSelection) {
        self.selection = selection;
    }

    pub fn has_unsaved(&self) -> bool {
        self.project
            .as_ref()
            .is_some_and(|project| !project.unsaved().is_empty())
    }

    pub fn row(&self, table: &str, key: &str) -> Option<Arc<Value>> {
        self.project.as_ref()?.files.json(&row_file(table, key))
    }

    pub fn row_value(&self, table: &str, key: &str) -> Option<Value> {
        self.row(table, key).map(|row| (*row).clone())
    }

    pub fn thumb(&mut self, ctx: &egui::Context, path: &str) -> Option<Thumbnail> {
        let project = self.project.as_ref()?;
        self.form.thumbnails.image(ctx, &project.files, path)
    }

    pub fn update_brush(&mut self, change: impl FnOnce(&mut BrushSettings)) {
        change(&mut self.settings.brush);
        self.save_settings();
    }

    pub fn set_brush(&mut self, brush: BrushSettings) {
        self.settings.brush = brush;
        self.save_settings();
    }

    pub fn loaded(&self) -> Option<Content> {
        self.project
            .as_ref()
            .and_then(|project| project.content.clone())
    }

    fn poll_project(&mut self) {
        let Some(project) = &mut self.project else {
            return;
        };
        if project.poll() {
            self.views.content_changed();
        }
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
            let loads = project.loads();
            let text = if loads {
                format!(
                    "↻ {} rows changed on disk and were reloaded",
                    outcome.taken.len()
                )
            } else {
                format!(
                    "{} files changed on disk and the folder no longer loads: read-only until it does",
                    outcome.taken.len()
                )
            };
            self.toast(text);
            self.views.content_changed();
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
            written: unix_seconds(),
            files: unsaved
                .iter()
                .map(|path| {
                    RecoveryJournalFile::of(
                        path,
                        project.disk.get(path).map(String::as_str),
                        project.files.get(path),
                    )
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
        ui::show(&mut editor, &ctx);
        let ui_ms = started.elapsed().as_secs_f32() * 1000.0;
        let tab = ui::active_tab(&editor).map(|tab| tab.title());
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
        import::poll(&mut editor);
        views::sync(&mut editor, world);
    });
}

pub fn registry() -> &'static ContentRegistry {
    static REGISTRY: OnceLock<ContentRegistry> = OnceLock::new();
    REGISTRY.get_or_init(systems::registry)
}

pub fn content_version() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| registry().version().0)
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
