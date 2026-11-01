pub mod graph;
pub mod inspect;
pub mod nodes;
pub mod outline;
pub mod palette;
pub mod panel;
pub mod run;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use ui::chip;
use ui::controls;
use ui::dialog;
use ui::egui;
use ui::input;
use ui::theme;
use ui::words;

use engine::core::content::{ContentFile, FolderReader, fields_file};
use engine::core::script::{Arrow, Choice, Exit, RunnerKindDef};
use engine::systems::area::AreaModule;
use engine::systems::area::scene::{SCENE_FILE, SceneNode};
use engine::systems::npc::NpcDef;
use engine::systems::script::{
    ListedScriptModule, Node, RunnerKind, ScriptDef, ScriptModule, Speaker,
};
use engine::systems::text::plain;
use graph::{FaceImage, NodePaletteOpen, ScriptGraph, ScriptGraphAction, ScriptGraphLayout};
use nodes::{NodeText, Nodes};
use serde_json::Value;
use ui::egui::emath::TSTransform;
use ui::egui::{Pos2, Rect, RichText, Stroke, Ui, vec2};
use ui::egui_snarl::ui::get_selected_nodes;

use crate::app::{BottomTab, Editor};
use crate::forms::{FormAction, FormCx};
use crate::keymap::EditorKey;
use crate::modal::{ConfirmAction, ConfirmDialog, ModalDialog};
use crate::project::files::content_of;
use crate::project::{ContentEdit, Project, RowSelection};
use crate::scene_graph;
use crate::tabs::{DockRequest, EditorTab};
use crate::thumbs;
use crate::user::UserFolder;
use ui::input::InputField;

/// Where in its file a module that runs a script is: a module by its pointer, the place of a
/// module in a list of modules or of a node in a scene, or one of an object's scripts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ScriptPath {
    Module(String),
    ObjectScript { object: String, script: usize },
}

impl ScriptPath {
    pub fn parse(pointer: &str) -> Option<ScriptPath> {
        let module = scene_graph::module_pointer(pointer)?;
        let rest = &pointer[module.len()..];
        let Some(script) = rest
            .strip_prefix("/scripts/")
            .and_then(|rest| rest.split('/').next())
            .and_then(|index| index.parse().ok())
        else {
            return Some(ScriptPath::Module(module));
        };
        Some(ScriptPath::ObjectScript {
            object: module,
            script,
        })
    }

    pub fn pointer(&self) -> String {
        match self {
            ScriptPath::Module(module) => module.clone(),
            ScriptPath::ObjectScript { object, script } => format!("{object}/scripts/{script}"),
        }
    }

    /// The script module at this place of `file`, as the file holds it now: one of a row's
    /// modules, a script module of a scene, or one of the scripts of a scene's object.
    pub(crate) fn module_in(&self, project: &Project, file: &str) -> Option<ScriptModule> {
        let Some(bundle) = file.strip_suffix(&format!("/{SCENE_FILE}")) else {
            return match self {
                ScriptPath::Module(pointer) => project
                    .value_at::<ListedScriptModule>(file, pointer)
                    .map(|ListedScriptModule::Script(module)| module),
                ScriptPath::ObjectScript { .. } => None,
            };
        };
        let scene = crate::views::area::nodes::bundle_in(project, bundle)?;
        let pointer = match self {
            ScriptPath::Module(pointer) => pointer,
            ScriptPath::ObjectScript { object, .. } => object,
        };
        let Some(SceneNode::Module(node)) = crate::views::area::nodes::node(scene.nodes(), pointer)
        else {
            return None;
        };
        match (&node.module, self) {
            (AreaModule::Script(module), ScriptPath::Module(_)) => Some(module.clone()),
            (AreaModule::Object(object), ScriptPath::ObjectScript { script, .. }) => {
                object.scripts.get(*script).cloned()
            }
            _ => None,
        }
    }

    pub fn object(&self) -> Option<&str> {
        match self {
            ScriptPath::Module(_) => None,
            ScriptPath::ObjectScript { object, .. } => Some(object),
        }
    }

    pub fn module(&self) -> &str {
        match self {
            ScriptPath::Module(module) | ScriptPath::ObjectScript { object: module, .. } => module,
        }
    }
}

pub(crate) struct ScriptRef {
    pub module: Option<ScriptPath>,
    pub node: Option<String>,
}

impl ScriptRef {
    pub fn parse(pointer: &str) -> ScriptRef {
        ScriptRef {
            module: ScriptPath::parse(pointer),
            node: pointer
                .split_once("/nodes/")
                .and_then(|(_, rest)| rest.split('/').next())
                .map(str::to_owned),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ScriptRow {
    pub key: String,
}

impl ScriptRow {
    pub fn new(key: impl Into<String>) -> ScriptRow {
        ScriptRow { key: key.into() }
    }

    pub fn title(&self) -> String {
        crate::tabs::file_title(&self.key).to_owned()
    }

    pub fn row(&self) -> (String, String) {
        (ScriptDef::TABLE.to_owned(), self.key.clone())
    }

    pub fn file(&self) -> String {
        fields_file(&self.key)
    }

    pub fn slug(&self) -> String {
        format!("script-{}", self.key)
    }
}

#[derive(Clone)]
pub struct ScriptDoc {
    pub at: ScriptRow,
    pub file: String,
    pub script: Arc<ScriptDef>,
    /// What names the files the script's references are ids among.
    pub reader: Arc<FolderReader>,
    pub kind: RunnerKind,
    /// The one npc that names the script, who speaks its lines unless a line says otherwise.
    pub speaker: Option<String>,
}

impl ScriptDoc {
    /// The script at `at` as the game reads it, when it does.
    pub fn load(editor: &Editor, at: &ScriptRow) -> Option<ScriptDoc> {
        let project = editor.project.as_ref()?;
        let script = project.read::<ScriptDef>(&at.key).ok()?;
        let mut npcs: Vec<&str> = project
            .usages
            .of_content(&at.key)
            .iter()
            .filter_map(|usage| content_of(&usage.file))
            .filter(|(table, _)| *table == NpcDef::TABLE)
            .map(|(_, npc)| npc)
            .collect();
        npcs.dedup();
        Some(ScriptDoc {
            at: at.clone(),
            file: at.file(),
            kind: script.runs_on,
            speaker: match npcs.as_slice() {
                [npc] => Some((*npc).to_owned()),
                _ => None,
            },
            reader: project.reader(),
            script,
        })
    }

    pub fn nodes(&self) -> &Nodes {
        &self.script.nodes
    }

    /// The script as it stands, to change and write back.
    pub fn edited(&self) -> ScriptDef {
        (*self.script).clone()
    }

    pub(crate) fn default_speaker(&self) -> Speaker {
        let npc = self
            .speaker
            .as_deref()
            .and_then(|npc| self.reader.id_of::<NpcDef>(npc));
        match (self.kind, npc) {
            (RunnerKind::Npc, Some(npc)) => Speaker::Npc(npc),
            (RunnerKind::Object, _) => Speaker::Object,
            _ => Speaker::Narrator,
        }
    }

    /// What reads the names and files a node's summary shows.
    pub fn text<'a>(&'a self, project: &'a Project) -> NodeText<'a> {
        NodeText {
            project,
            reader: &self.reader,
            catalogue: &project.catalogue,
        }
    }

    pub(crate) fn form_cx<'a>(
        &self,
        editor: &'a mut Editor,
        root: &'a Value,
        actions: &'a mut Vec<FormAction>,
    ) -> FormCx<'a> {
        let Editor {
            project,
            form,
            shapes,
            ..
        } = editor;
        let project = project.as_ref().expect("a project");
        let mut cx = FormCx::new(project, root, shapes, form, actions)
            .editing(ScriptDef::TABLE, Some(&self.at.key));
        cx.kept = Some(crate::schema::KeptValues::of(&self.script));
        cx.speaker = match self.default_speaker() {
            Speaker::Npc(npc) => self.reader.path_of(npc).map(str::to_owned),
            _ => None,
        };
        cx
    }
}

#[derive(Default)]
pub struct ScriptViewState {
    pub graph: ScriptGraph,
    pub outline: bool,
    pub layout_dirty: bool,
    pub selected: Vec<String>,
    pub marked: Vec<String>,
    pub palette: Option<palette::NodePalette>,
    pub outline_focus: Option<String>,
    pub folded: HashSet<String>,
    pub to_global: Option<TSTransform>,
    pub rects: HashMap<String, Rect>,
}

impl ScriptViewState {
    pub fn opened(layout: ScriptGraphLayout) -> ScriptViewState {
        ScriptViewState {
            graph: ScriptGraph::new(&layout),
            outline: layout.outline,
            ..Default::default()
        }
    }

    fn toggle_outline(&mut self) {
        self.outline = !self.outline;
        self.layout_dirty = true;
    }

    fn place_node(&mut self, name: &str, at: Pos2) {
        self.graph.place(name, at);
        self.layout_dirty = true;
    }

    pub fn rename_node(&mut self, old: &str, new: &str) {
        self.graph.rename(old, new);
        self.layout_dirty = true;
    }
}

fn snarl_id(at: &ScriptRow) -> egui::Id {
    egui::Id::new(("script-graph", at.slug()))
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ScriptKeys {
    pub fire: bool,
    pub tab: bool,
    pub outline: bool,
    pub delete: bool,
    pub rename: bool,
    pub duplicate: bool,
    pub enter: bool,
    pub ctrl_enter: bool,
    pub link: bool,
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
}

impl ScriptKeys {
    pub fn take(ctx: &egui::Context) -> ScriptKeys {
        ctx.input_mut(|i| {
            let arrow = EditorKey::Move.take_arrow(i);
            ScriptKeys {
                fire: EditorKey::RunScript.consume(i),
                tab: EditorKey::Palette.consume(i),
                outline: EditorKey::Outline.consume(i),
                delete: EditorKey::Delete.consume(i),
                rename: EditorKey::Rename.consume(i),
                duplicate: EditorKey::Duplicate.consume(i),
                enter: EditorKey::Confirm.consume(i),
                ctrl_enter: EditorKey::AddChoice.consume(i),
                link: EditorKey::Link.consume(i),
                up: arrow == Some([0, -1]),
                down: arrow == Some([0, 1]),
                left: arrow == Some([-1, 0]),
                right: arrow == Some([1, 0]),
            }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScriptSessionId(u64);

struct ScriptSession {
    id: ScriptSessionId,
    at: ScriptRow,
}

#[derive(Default)]
pub struct ScriptViews {
    sessions: Vec<ScriptSession>,
    views: HashMap<ScriptSessionId, ScriptViewState>,
    pub keys: ScriptKeys,
    /// Where a script view was drawn the frame before, which the next frame's keys are taken for
    /// while the pointer is over it.
    pub shown_rect: Option<Rect>,
    pub run: Option<run::ScriptRun>,
    setups: HashMap<ScriptSessionId, run::RunSetup>,
    fire: Option<ScriptSessionId>,
    pending: Option<(ScriptSessionId, PendingScriptAction)>,
    pub callout: Option<ScriptCallout>,
}

impl ScriptViews {
    pub fn session(&mut self, at: &ScriptRow) -> ScriptSessionId {
        if let Some(id) = self.find(at) {
            return id;
        }
        let id = ScriptSessionId(self.sessions.len() as u64);
        self.sessions.push(ScriptSession { id, at: at.clone() });
        id
    }

    pub fn find(&self, at: &ScriptRow) -> Option<ScriptSessionId> {
        self.sessions
            .iter()
            .find(|session| session.at == *at)
            .map(|session| session.id)
    }

    pub fn relocate(&mut self, from: &ScriptRow, to: ScriptRow) {
        if let Some(session) = self.sessions.iter_mut().find(|session| session.at == *from) {
            session.at = to.clone();
        }
        if let Some(callout) = self.callout.as_mut().filter(|callout| callout.at == *from) {
            callout.at = to;
        }
    }

    pub fn view(&self, at: &ScriptRow) -> Option<&ScriptViewState> {
        self.views.get(&self.find(at)?)
    }

    pub fn view_mut(
        &mut self,
        at: &ScriptRow,
        user: &UserFolder,
        project: Option<&Project>,
    ) -> &mut ScriptViewState {
        let id = self.session(at);
        self.views.entry(id).or_insert_with(|| {
            let layout = project
                .map(|project| user.load_layout(&project.root, &at.slug()))
                .unwrap_or_default();
            ScriptViewState::opened(layout)
        })
    }

    pub(crate) fn run_of(&self, at: &ScriptRow) -> Option<&run::ScriptRun> {
        let id = self.find(at)?;
        self.run.as_ref().filter(|run| run.doc == id)
    }

    pub(crate) fn run_of_mut(&mut self, at: &ScriptRow) -> Option<&mut run::ScriptRun> {
        let id = self.find(at)?;
        self.run.as_mut().filter(|run| run.doc == id)
    }

    fn existing_view_mut(&mut self, at: &ScriptRow) -> Option<&mut ScriptViewState> {
        let id = self.find(at)?;
        self.views.get_mut(&id)
    }

    pub(crate) fn setup_of(&self, at: &ScriptRow) -> run::RunSetup {
        self.find(at)
            .and_then(|id| self.setups.get(&id))
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn set_setup(&mut self, at: &ScriptRow, setup: run::RunSetup) {
        let id = self.session(at);
        self.setups.insert(id, setup);
    }

    pub fn fire(&mut self, at: &ScriptRow) {
        self.fire = Some(self.session(at));
    }

    pub(crate) fn take_fire(&mut self, at: &ScriptRow) -> bool {
        let fired = self.fire.take();
        fired.is_some() && fired == self.find(at)
    }

    pub(crate) fn set_pending(&mut self, at: &ScriptRow, action: PendingScriptAction) {
        let id = self.session(at);
        self.pending = Some((id, action));
    }

    fn take_pending(&mut self, at: &ScriptRow) -> Option<PendingScriptAction> {
        let id = self.find(at)?;
        self.pending
            .take_if(|(pending, _)| *pending == id)
            .map(|(_, action)| action)
    }

    pub fn close(&mut self) {
        self.sessions.clear();
        self.views.clear();
        self.setups.clear();
        self.run = None;
        self.fire = None;
        self.pending = None;
    }
}

pub(crate) enum PendingScriptAction {
    /// Links the output of `from` a scene names by its label to `to`.
    Connect {
        from: String,
        output: String,
        to: String,
    },
}

pub struct ScriptCallout {
    pub at: ScriptRow,
    pub node: String,
    pub to: Option<String>,
    pub rule: String,
    pub text: String,
    pub since: Instant,
}

impl ScriptCallout {
    pub fn live(&self, at: &ScriptRow) -> bool {
        self.at == *at && self.since.elapsed().as_secs_f32() < 12.0
    }
}

pub struct RenameNodeDialog {
    pub at: ScriptRow,
    pub old: String,
    pub draft: String,
}

pub struct LinkNodeDialog {
    pub at: ScriptRow,
    pub from: String,
    pub search: String,
    pub port: usize,
}

fn content_generation(editor: &Editor) -> u64 {
    editor
        .project
        .as_ref()
        .map_or(0, |project| project.generation)
}

fn sync_graph(editor: &mut Editor, doc: &ScriptDoc) {
    let generation = content_generation(editor);
    let Editor {
        user,
        project,
        views,
        ..
    } = editor;
    let Some(project) = project.as_ref() else {
        return;
    };
    views
        .script
        .view_mut(&doc.at, user, Some(project))
        .graph
        .sync(doc, &doc.text(project), generation);
}

pub(crate) fn sync_with_content(editor: &mut Editor) {
    let generation = content_generation(editor);
    let views = &mut editor.views.script;
    if views
        .run
        .as_ref()
        .is_some_and(|run| run.generation != generation)
    {
        views.run = None;
    }
}

pub fn show(editor: &mut Editor, ui: &mut Ui, at: &ScriptRow) {
    if crate::views::read_shown::<ScriptDef>(editor, ui, &at.key).is_none() {
        return;
    }
    let Some(doc) = ScriptDoc::load(editor, at) else {
        return;
    };
    editor.views.script.shown_rect = Some(ui.max_rect());
    if let Some(pending) = editor.views.script.take_pending(at) {
        match pending {
            PendingScriptAction::Connect { from, output, to } => {
                let exit = doc
                    .nodes()
                    .get(&from)
                    .and_then(|node| nodes::exit_called(node, &output));
                if let Some(exit) = exit {
                    act(editor, &doc, ScriptGraphAction::Connect { from, exit, to })
                }
            }
        }
    }
    if ui.input(|i| i.pointer.any_pressed()) && ui.rect_contains_pointer(ui.max_rect()) {
        editor
            .dock_requests
            .push(DockRequest::Focus(EditorTab::Script(at.clone())));
    }
    sync_with_content(editor);
    sync_graph(editor, &doc);
    let mut actions = Vec::new();
    keys(editor, ui, &doc, &mut actions);
    status(editor, &doc);
    ui::panel::left_scrolled(ui, "script", |ui| inspect::script_row(editor, ui, &doc));
    let (table, key) = at.row();
    let selection = if editor.selection.table == table
        && editor.selection.keys.first().is_some_and(|k| *k == key)
    {
        editor.selection.clone()
    } else {
        RowSelection::row(&table, &key)
    };
    ui::panel::inspector(ui, editor, "script", |editor, ui| {
        inspect::show(editor, ui, &doc, &selection)
    });
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            if editor
                .views
                .script
                .view_mut(at, &editor.user, editor.project.as_ref())
                .outline
            {
                outline::show(editor, ui, &doc, &mut actions);
            } else {
                canvas(editor, ui, &doc, &mut actions);
            }
            view_switch(editor, ui, at);
        });
    palette::show(editor, ui, &doc);
    for action in actions {
        let Some(doc) = ScriptDoc::load(editor, at) else {
            break;
        };
        act(editor, &doc, action);
    }
    editor.views.script.keys = ScriptKeys::default();
    save_layout(editor, ui, at);
}

pub fn bottom(editor: &mut Editor, ui: &mut Ui, at: &ScriptRow, tab: BottomTab) {
    let Some(doc) = ScriptDoc::load(editor, at) else {
        return;
    };
    sync_with_content(editor);
    panel::show(editor, ui, &doc, tab);
}

/// Canvas and Outline, at the top right of the view they switch between, in a layer above the
/// canvas's, which would otherwise take their clicks.
fn view_switch(editor: &mut Editor, ui: &mut Ui, at: &ScriptRow) {
    let view = editor
        .views
        .script
        .view_mut(at, &editor.user, editor.project.as_ref());
    let mut switch = false;
    egui::Area::new(ui.id().with("view-switch"))
        .order(egui::Order::Foreground)
        .fixed_pos(ui.max_rect().right_top() + vec2(-10.0, 10.0))
        .pivot(egui::Align2::RIGHT_TOP)
        .show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (label, outline) in [("Canvas", false), ("Outline", true)] {
                    let tip = format!("{} switches between them", EditorKey::Outline.name());
                    if chip::mode(ui, label, view.outline == outline)
                        .on_hover_text(tip)
                        .clicked()
                        && view.outline != outline
                    {
                        switch = true;
                    }
                }
            });
        });
    if switch {
        view.toggle_outline();
    }
}

fn canvas(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc, actions: &mut Vec<ScriptGraphAction>) {
    let at = doc.at.clone();
    let (lit, current): (HashSet<String>, Option<String>) = editor
        .views
        .script
        .run_of(&at)
        .map(|run| {
            (
                run.visited.iter().cloned().collect(),
                run.visited.last().cloned(),
            )
        })
        .unwrap_or_default();
    let selected_before = {
        let views = &mut editor.views.script;
        let view = views.view_mut(&at, &editor.user, editor.project.as_ref());
        let selection = &editor.selection;
        if selection.table == ScriptDef::TABLE
            && selection.keys == [at.key.as_str()]
            && is_node_selection(selection)
        {
            let mut names: Vec<String> = selection
                .parts
                .iter()
                .filter_map(|part| part.strip_prefix("node:"))
                .map(str::to_owned)
                .collect();
            names.sort();
            view.selected = names;
        }
        view.selected.clone()
    };
    let faces = faces(editor, ui.ctx(), doc);
    let id = snarl_id(&at);
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let (dragged, to_global, mut taken, clicked, rects) = {
        let views = &mut editor.views.script;
        let view = views.view_mut(&at, &editor.user, Some(project));
        let chosen: HashSet<String> = selected_before.iter().cloned().collect();
        let mut viewer = graph::ScriptGraphViewer {
            doc,
            text: doc.text(project),
            ports: graph::ports_of(doc),
            lit: &lit,
            current: current.as_deref(),
            selected: &chosen,
            faces: &faces,
            actions: Vec::new(),
            clicks: Vec::new(),
            to_global: TSTransform::IDENTITY,
            rects: HashMap::new(),
            fit: None,
        };
        if view.to_global.is_none() {
            let placed = view.graph.positions();
            let spots: Vec<Pos2> = if placed.is_empty() {
                vec![graph::FIRST]
            } else {
                placed.into_values().collect()
            };
            let bounds = spots.into_iter().fold(Rect::NOTHING, |bounds, at| {
                bounds.union(Rect::from_min_size(at, vec2(graph::CARD + 20.0, 120.0)))
            });
            viewer.fit = Some((bounds, ui.max_rect()));
        }
        let graph = &mut view.graph;
        let before = graph.node_spots();
        graph::widget(id).show(&mut graph.snarl, &mut viewer, ui);
        let dragged = graph.node_spots() != before;
        (
            dragged,
            viewer.to_global,
            viewer.actions,
            viewer.clicks,
            viewer.rects,
        )
    };
    header_chip(editor, ui, doc);
    editor
        .views
        .script
        .view_mut(&at, &editor.user, editor.project.as_ref())
        .rects = rects;
    run::conversation_box(editor, ui, doc);
    actions.append(&mut taken);
    let selected: Vec<String> = {
        let view = editor
            .views
            .script
            .view_mut(&at, &editor.user, editor.project.as_ref());
        let graph = &view.graph;
        let mut marked: Vec<String> = get_selected_nodes(id, ui.ctx())
            .iter()
            .filter_map(|id| graph.snarl.get_node(*id).map(|node| node.name.clone()))
            .collect();
        marked.sort();
        if dragged {
            view.layout_dirty = true;
        }
        view.to_global = Some(to_global);
        let mut names = if marked != view.marked {
            view.marked = marked.clone();
            marked
        } else {
            selected_before.clone()
        };
        for graph::NodeClick { name, extend } in clicked {
            if !extend {
                names.clear();
            }
            if !names.contains(&name) {
                names.push(name);
            }
            names.sort();
        }
        names
    };
    callout(editor, ui, &at);
    if selected != selected_before {
        editor
            .views
            .script
            .view_mut(&at, &editor.user, editor.project.as_ref())
            .selected = selected.clone();
        if !selected.is_empty() {
            select_nodes(editor, doc, &selected);
        }
    }
}

fn faces(editor: &mut Editor, ctx: &egui::Context, doc: &ScriptDoc) -> graph::SpeakerFaces {
    let mut faces = graph::SpeakerFaces::new();
    let Some(project) = editor.project.as_ref() else {
        return faces;
    };
    let thumbnails = &mut editor.form.thumbnails;
    let lines = doc.nodes().values().flat_map(|node| match node {
        Node::Dialogue(dialogue) => dialogue.lines.as_slice(),
        _ => &[],
    });
    for line in lines {
        let speaker = (line.by, line.face.clone());
        if faces.contains_key(&speaker) {
            continue;
        }
        let Some(file) = nodes::bust_file(project, line.by, line.face.as_ref()) else {
            continue;
        };
        let Some(thumb) = thumbnails.image(ctx, &project.files, &file) else {
            continue;
        };
        faces.insert(
            speaker,
            FaceImage {
                texture: thumb.texture.id(),
                uv: thumbs::square_uv(thumb.size),
            },
        );
    }
    faces
}

fn header_chip(editor: &Editor, ui: &Ui, doc: &ScriptDoc) {
    let text = match editor.views.script.run_of(&doc.at) {
        Some(_) => format!("{} · running with a pretend subject", doc.at.title()),
        None => chip_title(editor, doc),
    };
    let above_the_canvas = ui
        .ctx()
        .layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            ui.id().with("header-chip"),
        ))
        .with_clip_rect(ui.max_rect());
    chip::floating(
        &above_the_canvas,
        ui.max_rect().min + vec2(16.0, 12.0),
        egui::Align2::LEFT_TOP,
        &text,
        theme::PANEL,
        theme::TEXT,
        12.5,
    );
}

fn chip_title(editor: &Editor, doc: &ScriptDoc) -> String {
    let count = doc.nodes().len();
    let mut text = format!(
        "{} · runs on {} · {count} node{}",
        doc.at.title(),
        doc.kind.name(),
        words::plural(count)
    );
    if editor.shapes.is_fixture(ScriptDef::TABLE, &doc.at.key) {
        text.push_str(" · from New Game");
    }
    text
}

fn status(editor: &mut Editor, doc: &ScriptDoc) {
    let at = doc.at.clone();
    let view = editor.views.script.view(&at);
    let outline = view.is_some_and(|view| view.outline);
    let palette = view.is_some_and(|view| view.palette.is_some());
    let selected = view.map(|view| view.selected.clone()).unwrap_or_default();
    let head = match selected.as_slice() {
        [one] if !outline && !palette => format!("{} › {one}", at.title()),
        _ => {
            let mode = if outline {
                " · outline"
            } else if palette {
                " · palette"
            } else {
                ""
            };
            format!("{} › script{mode}", at.title())
        }
    };
    let refused = editor
        .views
        .script
        .callout
        .as_ref()
        .filter(|callout| callout.live(&at))
        .map(|callout| callout.rule.clone());
    let hint = if let Some(rule) = refused {
        format!("refused · {rule}  nothing changed")
    } else if let Some(run) = editor.views.script.run_of(&at) {
        let waiting = run
            .visited
            .last()
            .map(|node| format!("waiting at {node}"))
            .unwrap_or_else(|| "ended".to_owned());
        format!(
            "Run · 1 flow · {waiting}   {} pick a choice · {} stops",
            EditorKey::Pick.name(),
            EditorKey::Cancel.name()
        )
    } else if palette {
        format!(
            "{} choose · {} opens a system · {} places it",
            EditorKey::Move.name(),
            EditorKey::Move.name(),
            EditorKey::Confirm.name()
        )
    } else if outline {
        format!(
            "{} switches to the canvas, keeping the selection",
            EditorKey::Outline.name()
        )
    } else {
        match selected.as_slice() {
            [one] => doc.nodes().get(one).map_or_else(String::new, nodes::hint),
            [] => format!(
                "{} opens the palette · drag from a dot to link · {} removes",
                EditorKey::Palette.name(),
                EditorKey::Delete.name()
            ),
            many => format!(
                "{} nodes · drag moves them together · {} removes them",
                many.len(),
                EditorKey::Delete.name()
            ),
        }
    };
    editor.status.left.push(head);
    editor.status.left.push(hint);
}

fn beside(graph: &ScriptGraph, from: &str) -> Pos2 {
    let placed = graph.positions();
    let Some(origin) = placed.get(from) else {
        return graph::FIRST;
    };
    let mut pos = Pos2::new(origin.x + 280.0, origin.y);
    while placed
        .values()
        .any(|p| (p.x - pos.x).abs() < 200.0 && (p.y - pos.y).abs() < 60.0)
    {
        pos.y += 130.0;
    }
    pos
}

fn callout(editor: &mut Editor, ui: &mut Ui, at: &ScriptRow) {
    let Some(callout) = editor
        .views
        .script
        .callout
        .as_ref()
        .filter(|callout| callout.live(at))
    else {
        return;
    };
    let Some(view) = editor.views.script.view(at) else {
        return;
    };
    let Some(to_global) = view.to_global else {
        return;
    };
    let on_screen = |name: &str| view.rects.get(name).map(|rect| to_global * *rect);
    let Some(anchor) = on_screen(&callout.node) else {
        return;
    };
    let painter = ui
        .ctx()
        .layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            ui.id().with("callout"),
        ))
        .with_clip_rect(ui.max_rect());
    let stroke = Stroke::new(1.5, theme::DANGER);
    if let Some(target) = callout.to.as_deref().and_then(on_screen) {
        callout_arrow(&painter, anchor.right_center(), target.left_center());
        painter.rect_stroke(target, 6.0, stroke, egui::StrokeKind::Outside);
    }
    painter.rect_stroke(anchor, 6.0, stroke, egui::StrokeKind::Outside);
    callout_card(ui, &painter, anchor, callout);
}

fn callout_arrow(painter: &egui::Painter, from: Pos2, to: Pos2) {
    let lift = vec2(0.0, -(24.0 + (from.x - to.x).abs() * 0.06).min(80.0));
    let curve = egui::epaint::CubicBezierShape::from_points_stroke(
        [
            from,
            from + lift + vec2(60.0, 0.0),
            to + lift - vec2(60.0, 0.0),
            to,
        ],
        false,
        egui::Color32::TRANSPARENT,
        Stroke::NONE,
    );
    let points: Vec<Pos2> = curve.flatten(Some(0.5));
    painter.add(egui::Shape::dashed_line(
        &points,
        Stroke::new(1.5, theme::DANGER),
        6.0,
        5.0,
    ));
}

fn callout_card(ui: &Ui, painter: &egui::Painter, anchor: Rect, callout: &ScriptCallout) {
    let width = 300.0;
    let title = ui.fonts_mut(|fonts| {
        fonts.layout(
            callout.rule.clone(),
            egui::FontId::proportional(13.5),
            theme::DANGER,
            width,
        )
    });
    let body = ui.fonts_mut(|fonts| {
        fonts.layout(
            callout.text.clone(),
            egui::FontId::proportional(12.5),
            theme::TEXT,
            width,
        )
    });
    let pad = 10.0;
    let size = vec2(
        width + pad * 2.0,
        title.size().y + body.size().y + pad * 2.0 + 4.0,
    );
    let mut top_left = anchor.left_bottom() + vec2(0.0, 10.0);
    top_left.x = top_left
        .x
        .min(ui.max_rect().right() - size.x - 8.0)
        .max(ui.max_rect().left() + 8.0);
    if top_left.y + size.y > ui.max_rect().bottom() - 8.0 {
        top_left.y = anchor.top() - size.y - 10.0;
    }
    let rect = Rect::from_min_size(top_left, size);
    ui::slip::paint(painter, rect);
    painter.rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, theme::DANGER),
        egui::StrokeKind::Inside,
    );
    let title_height = title.size().y;
    painter.galley(rect.min + vec2(pad, pad), title, theme::DANGER);
    painter.galley(
        rect.min + vec2(pad, pad + title_height + 4.0),
        body,
        theme::TEXT,
    );
}

pub fn select_nodes(editor: &mut Editor, doc: &ScriptDoc, names: &[String]) {
    editor.select(RowSelection::parts(
        ScriptDef::TABLE,
        &doc.at.key,
        names.iter().map(|name| format!("node:{name}")).collect(),
    ));
    editor
        .views
        .script
        .view_mut(&doc.at, &editor.user, editor.project.as_ref())
        .selected = names.to_vec();
}

fn is_node_selection(selection: &RowSelection) -> bool {
    !selection.parts.is_empty()
        && selection
            .parts
            .iter()
            .all(|part| part.starts_with("node:") || part.starts_with("choice:"))
}

fn keys(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc, actions: &mut Vec<ScriptGraphAction>) {
    let at = doc.at.clone();
    let keys = editor.views.script.keys;
    if editor
        .views
        .script
        .view_mut(&at, &editor.user, editor.project.as_ref())
        .palette
        .is_some()
    {
        return;
    }
    if keys.fire {
        editor.bottom_tab = BottomTab::Run;
        editor.views.script.fire(&at);
        return;
    }
    let views = &mut editor.views.script;
    let view = views.view_mut(&at, &editor.user, editor.project.as_ref());
    let in_outline = view.outline;
    if keys.outline {
        view.toggle_outline();
    }
    if in_outline {
        return;
    }
    let selected = view.selected.clone();
    let first = selected.first().cloned();
    if keys.tab {
        let link = first.as_ref().and_then(|name| free_port_link(doc, name));
        let beside_node = first
            .as_ref()
            .and_then(|name| view.rects.get(name))
            .zip(view.to_global)
            .map(|(rect, to_global)| to_global * rect.right_top() + vec2(16.0, 0.0));
        actions.push(ScriptGraphAction::Palette(NodePaletteOpen {
            at: beside_node
                .or_else(|| ui.input(|i| i.pointer.hover_pos()))
                .unwrap_or_else(|| ui.max_rect().center()),
            placed: false,
            link,
        }));
    }
    if keys.delete && !selected.is_empty() {
        actions.push(ScriptGraphAction::Delete(selected.clone()));
    }
    if keys.rename
        && let Some(name) = &first
    {
        actions.push(ScriptGraphAction::Rename(name.clone()));
    }
    if keys.duplicate && !selected.is_empty() {
        actions.push(ScriptGraphAction::Duplicate(selected.clone()));
    }
    if let Some(name) = &first
        && doc.nodes().get(name).is_some_and(nodes::is_dialogue)
    {
        if keys.ctrl_enter {
            actions.push(ScriptGraphAction::AddChoice(name.clone()));
        } else if keys.enter {
            actions.push(ScriptGraphAction::AddLine(name.clone()));
        }
    }
    if keys.link
        && let Some(name) = first
    {
        link_by_name(editor, &at, name);
    }
}

pub(crate) fn free_port_link(doc: &ScriptDoc, name: &str) -> Option<(String, Exit)> {
    let node = doc.nodes().get(name)?;
    nodes::ports(node)
        .into_iter()
        .find(|port| port.target.is_none())
        .map(|port| (name.to_owned(), port.exit))
}

pub(crate) fn link_by_name(editor: &mut Editor, at: &ScriptRow, from: String) {
    editor.dialogs.push(ModalDialog::LinkNode(LinkNodeDialog {
        at: at.clone(),
        from,
        search: String::new(),
        port: 0,
    }));
}

pub fn act(editor: &mut Editor, doc: &ScriptDoc, action: ScriptGraphAction) {
    let at = doc.at.clone();
    let mut script = doc.edited();
    match action {
        ScriptGraphAction::Connect { from, exit, to } => {
            if let Some(node) = script.nodes.get_mut(&from) {
                nodes::set_arrow(node, exit, Some(&to));
            }
            let label = format!("Link {from} to {to} in {}", at.title());
            if !write(editor, doc, script, label)
                && let Some(callout) = editor.views.script.callout.as_mut()
            {
                callout.node = from;
                callout.to = Some(to);
            }
        }
        ScriptGraphAction::Disconnect { from, exit } => {
            if let Some(node) = script.nodes.get_mut(&from) {
                nodes::set_arrow(node, exit, None);
            }
            let label = format!("Unlink {from} in {}", at.title());
            write(editor, doc, script, label);
        }
        ScriptGraphAction::DropInputs(to) => {
            nodes::unlink(&mut script.nodes, &to);
            let label = format!("Unlink arrows into {to} in {}", at.title());
            write(editor, doc, script, label);
        }
        ScriptGraphAction::Delete(names) => delete_nodes(editor, doc, names),
        ScriptGraphAction::Rename(name) => {
            editor
                .dialogs
                .push(ModalDialog::RenameNode(RenameNodeDialog {
                    at,
                    old: name.clone(),
                    draft: name,
                }));
        }
        ScriptGraphAction::Duplicate(names) => duplicate(editor, doc, &names),
        ScriptGraphAction::Palette(open) => {
            editor
                .views
                .script
                .view_mut(&at, &editor.user, editor.project.as_ref())
                .palette = Some(palette::NodePalette::new(open));
        }
        ScriptGraphAction::AddLine(name) => {
            if let Some(Node::Dialogue(dialogue)) = script.nodes.get_mut(&name) {
                dialogue
                    .lines
                    .push(nodes::blank_line(doc.default_speaker()));
            }
            let label = format!("Add a line to {name} in {}", at.title());
            write(editor, doc, script, label);
        }
        ScriptGraphAction::EditLine { name, index, text } => {
            if let Some(Node::Dialogue(dialogue)) = script.nodes.get_mut(&name)
                && let Some(line) = dialogue.lines.get_mut(index)
            {
                line.text = text;
            }
            let label = format!("Edit line {} of {name} · {}", index + 1, at.title());
            write(editor, doc, script, label);
        }
        ScriptGraphAction::AddChoice(name) => {
            if let Some(node) = script.nodes.get_mut(&name) {
                add_choice(node);
            }
            let label = format!("Add a choice to {name} in {}", at.title());
            write(editor, doc, script, label);
        }
    }
}

/// Gives a dialogue a choice more, its plain next given up to the choices.
pub(crate) fn add_choice(node: &mut Node) {
    let Node::Dialogue(dialogue) = node else {
        return;
    };
    dialogue.next.point(None);
    dialogue.choices.push(Choice {
        label: vec![plain("…")],
        requires: None,
        reveal: Vec::new(),
        warn: None,
        to: Arrow::default(),
    });
}

/// Copies each of the nodes `names`, each copy led to from a free output of its original, as one
/// step; a node with no free output is left uncopied.
fn duplicate(editor: &mut Editor, doc: &ScriptDoc, names: &[String]) {
    let at = doc.at.clone();
    let mut script = doc.edited();
    let mut copies: Vec<(String, String)> = Vec::new();
    for name in names {
        let Some(mut copy) = doc.nodes().get(name).cloned() else {
            continue;
        };
        let Some(free) = nodes::ports(&copy)
            .into_iter()
            .find(|port| port.target.is_none())
        else {
            editor.toast(format!("{name} has no free output to lead to its copy"));
            continue;
        };
        for (_, arrow) in copy.exits_mut() {
            arrow.point(None);
        }
        let fresh = nodes::free_name(&script.nodes, name);
        script.nodes.insert(fresh.clone(), copy);
        if let Some(source) = script.nodes.get_mut(name) {
            nodes::set_arrow(source, free.exit, Some(&fresh));
        }
        copies.push((name.clone(), fresh));
    }
    let label = match copies.as_slice() {
        [] => return,
        [(name, _)] => format!("Duplicate {name} in {}", at.title()),
        many => format!("Duplicate {} nodes in {}", many.len(), at.title()),
    };
    if write(editor, doc, script, label) {
        let view = editor
            .views
            .script
            .view_mut(&at, &editor.user, editor.project.as_ref());
        for (name, fresh) in &copies {
            let at = beside(&view.graph, name);
            view.place_node(fresh, at);
        }
        let fresh: Vec<String> = copies.into_iter().map(|(_, fresh)| fresh).collect();
        select_nodes(editor, doc, &fresh);
    }
}

fn remove_nodes(doc: &ScriptDoc, names: &[String]) -> ScriptDef {
    let mut script = doc.edited();
    for name in names {
        script.nodes.remove(name);
        nodes::unlink(&mut script.nodes, name);
    }
    script
}

pub(crate) fn delete_nodes(editor: &mut Editor, doc: &ScriptDoc, names: Vec<String>) {
    let script = remove_nodes(doc, &names);
    let reachable = nodes::reachable(&script.nodes);
    let were_reachable = nodes::reachable(doc.nodes());
    let newly_stranded: Vec<String> = script
        .nodes
        .iter()
        .filter(|(name, node)| {
            !nodes::is_event(node) && !reachable.contains(*name) && were_reachable.contains(*name)
        })
        .map(|(name, _)| name.clone())
        .collect();
    if !newly_stranded.is_empty() {
        editor.dialogs.push(ModalDialog::Confirm(ConfirmDialog {
            title: format!("Delete {}?", names.join(", ")),
            body: format!(
                "No flow would reach {} afterwards. Delete those too, or link them elsewhere first.",
                newly_stranded.join(", ")
            ),
            label: format!("Delete {} nodes", names.len() + newly_stranded.len()),
            compare: None,
            action: ConfirmAction::DeleteScriptNodes {
                script: doc.at.clone(),
                names: names.iter().chain(newly_stranded.iter()).cloned().collect(),
            },
        }));
        return;
    }
    let label = match names.as_slice() {
        [one] => format!("Delete {one} in {}", doc.at.title()),
        many => format!("Delete {} nodes in {}", many.len(), doc.at.title()),
    };
    if write(editor, doc, script, label) {
        select_nodes(editor, doc, &[]);
    }
}

pub(crate) fn delete_nodes_now(editor: &mut Editor, at: &ScriptRow, names: Vec<String>) {
    let Some(doc) = ScriptDoc::load(editor, at) else {
        return;
    };
    let script = remove_nodes(&doc, &names);
    let label = format!("Delete {} nodes in {}", names.len(), at.title());
    if write(editor, &doc, script, label) {
        select_nodes(editor, &doc, &[]);
    }
}

pub fn add_node(
    editor: &mut Editor,
    doc: &ScriptDoc,
    kind: &str,
    at: Option<Pos2>,
    link: Option<(String, Exit)>,
) {
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let nearby = crate::schema::Nearby::of(project, Some(&doc.file));
    let Some(mut node) = nodes::fresh(&doc.reader, kind, doc.kind, nearby, doc.default_speaker())
    else {
        editor.toast(format!("No {kind} node can be made here"));
        return;
    };
    if nodes::is_event(&node) {
        let used: HashSet<&str> = doc.nodes().values().filter_map(nodes::event_name).collect();
        let Some(event) =
            nodes::free_event(&doc.reader, &project.catalogue, doc.kind, &used, nearby)
        else {
            editor.toast(format!(
                "Every event a {} script answers already starts a flow here",
                doc.kind.name()
            ));
            return;
        };
        node = event;
    }
    let base = if nodes::is_dialogue(&node) {
        &doc.at.key
    } else {
        kind
    };
    let name = nodes::free_name(doc.nodes(), base);
    let mut script = doc.edited();
    script.nodes.insert(name.clone(), node);
    if let Some((from, exit)) = &link
        && let Some(source) = script.nodes.get_mut(from)
    {
        nodes::set_arrow(source, *exit, Some(&name));
    }
    let script_row = doc.at.clone();
    let label = match &link {
        Some((from, _)) => format!("Add {kind} after {from} in {}", script_row.title()),
        None => format!("Add {kind} to {}", script_row.title()),
    };
    if write(editor, doc, script, label) {
        let view = editor
            .views
            .script
            .view_mut(&script_row, &editor.user, editor.project.as_ref());
        let graph_pos = match (at, view.to_global) {
            (Some(at), Some(to_global)) => to_global.inverse() * at,
            (Some(at), None) => at,
            (None, _) => link
                .as_ref()
                .map_or(graph::FIRST, |(from, _)| beside(&view.graph, from)),
        };
        view.place_node(&name, graph_pos);
        select_nodes(editor, doc, &[name]);
    }
}

pub fn write(editor: &mut Editor, doc: &ScriptDoc, script: ScriptDef, label: String) -> bool {
    let Some(project) = editor.project.as_ref() else {
        return false;
    };
    let mut edit = ContentEdit::new(label);
    if let Err(reason) = edit.write_content(project, &doc.at.key, &script) {
        editor.toast(format!("{}: {reason}", edit.label));
        return false;
    }
    let written = editor.apply(edit);
    editor.views.script.callout = (!written)
        .then_some(editor.last_refusal.as_ref())
        .flatten()
        .map(|refusal| {
            let named = ScriptRef::parse(&refusal.pointer)
                .node
                .filter(|name| doc.nodes().contains_key(name));
            let selected = editor
                .views
                .script
                .view(&doc.at)
                .and_then(|view| view.selected.first().cloned());
            ScriptCallout {
                at: doc.at.clone(),
                node: named.or(selected).unwrap_or_default(),
                to: None,
                rule: refusal.problem.rule.replace('_', " "),
                text: format!(
                    "{}; expected {}.",
                    refusal.problem.found, refusal.problem.expected
                ),
                since: Instant::now(),
            }
        });
    written
}

pub(crate) fn rename_node_dialog(
    editor: &mut Editor,
    ctx: &egui::Context,
    mut dialog: RenameNodeDialog,
) -> Option<RenameNodeDialog> {
    let doc = ScriptDoc::load(editor, &dialog.at)?;
    let mut outcome: Option<Option<String>> = None;
    dialog::card(ctx, "rename-node", 360.0, |ui| {
        let pressed = dialog::keys(ui);
        ui.label(RichText::new("Rename node").strong().size(18.0));
        ui.add_space(6.0);
        ui.label(theme::muted(format!("{} becomes:", dialog.old)).size(13.5));
        let response = InputField::new(&mut dialog.draft).show(ui);
        if !response.has_focus() {
            response.request_focus();
            let mut state = egui::TextEdit::load_state(ui.ctx(), response.id).unwrap_or_default();
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(dialog.draft.chars().count()),
                )));
            state.store(ui.ctx(), response.id);
        }
        let taken = dialog.draft != dialog.old && doc.nodes().contains_key(&dialog.draft);
        if taken {
            ui.colored_label(theme::DANGER, "that name is taken");
        }
        let ok = !dialog.draft.trim().is_empty()
            && !taken
            && dialog
                .draft
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_');
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(ok, egui::Button::new("Rename")).clicked() || (ok && pressed.confirm)
            {
                outcome = Some(Some(dialog.draft.clone()));
            }
            if ui.button("Cancel").clicked() || pressed.cancel {
                outcome = Some(None);
            }
        });
    });
    match outcome {
        None => Some(dialog),
        Some(None) => None,
        Some(Some(new)) => {
            if new != dialog.old {
                rename_node(editor, &doc, &dialog.old, &new);
            }
            None
        }
    }
}

pub fn rename_node(editor: &mut Editor, doc: &ScriptDoc, old: &str, new: &str) {
    let mut script = doc.edited();
    nodes::rename(&mut script.nodes, old, new);
    let at = doc.at.clone();
    let label = format!("Rename {old} to {new} in {}", at.title());
    if write(editor, doc, script, label) {
        editor
            .views
            .script
            .view_mut(&at, &editor.user, editor.project.as_ref())
            .rename_node(old, new);
        select_nodes(editor, doc, &[new.to_owned()]);
    }
}

pub(crate) fn link_node_dialog(
    editor: &mut Editor,
    ctx: &egui::Context,
    mut dialog: LinkNodeDialog,
) -> Option<LinkNodeDialog> {
    let doc = ScriptDoc::load(editor, &dialog.at)?;
    let ports = doc
        .nodes()
        .get(&dialog.from)
        .map(nodes::ports)
        .unwrap_or_default();
    let mut outcome: Option<Option<String>> = None;
    dialog::card(ctx, "link-node", 360.0, |ui| {
        let pressed = dialog::keys(ui);
        ui.label(
            RichText::new(format!("Link {} to…", dialog.from))
                .strong()
                .size(18.0),
        );
        ui.add_space(6.0);
        let response = input::search(ui, &mut dialog.search, "node name", None);
        input::default_focus(ui, &response);
        let needle = dialog.search.to_lowercase();
        if ports.len() > 1 {
            controls::combo("link-port")
                .selected_text(
                    ports
                        .get(dialog.port)
                        .map(|p| p.label.clone())
                        .unwrap_or_default(),
                )
                .show_ui(ui, |ui| {
                    for (index, port) in ports.iter().enumerate() {
                        ui.selectable_value(&mut dialog.port, index, &port.label);
                    }
                });
        }
        let matches: Vec<&String> = doc
            .nodes()
            .keys()
            .filter(|name| *name != &dialog.from && name.to_lowercase().contains(&needle))
            .collect();
        egui::ScrollArea::vertical()
            .max_height(240.0)
            .show(ui, |ui| {
                for name in &matches {
                    if ui.selectable_label(false, *name).clicked() {
                        outcome = Some(Some((*name).clone()));
                    }
                }
            });
        if pressed.confirm
            && let Some(name) = matches.first()
        {
            outcome = Some(Some((*name).clone()));
        }
        if pressed.cancel {
            outcome = Some(None);
        }
    });
    match outcome {
        None => Some(dialog),
        Some(None) => None,
        Some(Some(to)) => {
            if let Some(port) = ports.get(dialog.port) {
                let action = ScriptGraphAction::Connect {
                    from: dialog.from,
                    exit: port.exit,
                    to,
                };
                act(editor, &doc, action);
            }
            None
        }
    }
}

fn save_layout(editor: &mut Editor, ui: &Ui, at: &ScriptRow) {
    if ui.input(|input| input.pointer.any_down()) {
        return;
    }
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let Some(view) = editor.views.script.existing_view_mut(at) else {
        return;
    };
    if !view.layout_dirty {
        return;
    }
    view.layout_dirty = false;
    let layout = ScriptGraphLayout::of(&view.graph, view.outline);
    if let Err(error) = editor.user.save_layout(&project.root, &at.slug(), &layout) {
        editor.toast(format!("The graph layout was not saved: {error}"));
    }
}
