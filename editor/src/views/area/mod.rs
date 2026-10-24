pub mod blueprint;
pub mod flags;
pub mod ground;
pub mod inspect;
pub mod modules;
pub mod nodes;
pub mod overlays;
pub mod panel;
pub mod pick;
pub mod pins;
pub mod placement;
pub mod population;
pub mod preview;
pub mod resize;
pub mod scene_tree;
pub mod select;
pub mod shapes;
pub mod soundscape;
pub mod stamp;
pub mod status;
pub mod tools;

pub use resize::ResizeDialog;

use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use ui::controls;
use ui::egui;
use ui::theme;
use ui::words;

use engine::core::content::{Content, ContentFile, FolderReader, table_of};
use engine::core::geometry::Degrees;
use engine::core::math::{Pos, Rect, Size};
use engine::core::render::TILE;
use engine::core::units::{CellPos, Meters, TilePos, TileSize};
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::scene::{ModuleNode, SceneNode};
use engine::systems::area::{self, AreaDef, AreaId, AreaModule, AreaModuleKind, Picture, Pictured};
use strum::{AsRefStr, VariantArray};
use ui::egui::{Color32, Pos2, Rect as ERect, Sense, Ui, Vec2};
use ui::glyph::{self, Mark};

use self::flags::{SceneFlags, SceneLevel};
use self::overlays::{
    AreaLabels, overlay_grid, overlay_modules, overlay_populations, overlay_props,
    overlay_soundscape_reach, overlay_surfaces, overlay_walk,
};
use self::status::status_bar;
use crate::app::{BottomTab, Editor};
use crate::keymap::EditorKey;
use crate::project::{AreaCameraState, Project, RowSelection};
use crate::shell::{sidebar, widgets};
use crate::tabs::{EditorTab, file_title};
use crate::views::navigation::{self, Viewport};
use crate::{play, user};

pub const MIN_ZOOM: f32 = 0.125;
pub const MAX_ZOOM: f32 = 16.0;
const MAX_FIT_ZOOM: f32 = 4.0;
const FIT_MARGIN: f32 = 0.94;

/// The key of the view of an area or blueprint bundle.
pub fn view_key(bundle: &str) -> String {
    format!("bundle:{bundle}")
}

/// The bundle a view key's view shows: an area, or a blueprint.
pub fn viewed_bundle(view_key: &str) -> Option<&str> {
    view_key.strip_prefix("bundle:")
}

fn camera_key(project: &Project, view_key: &str) -> String {
    format!("{}:{view_key}", project.root.display())
}

#[derive(Default)]
pub struct AreaViews {
    pub grounds: ground::GroundArts,
    pub views: HashMap<String, AreaViewState>,
    pub hover: Option<AreaHover>,
    pub active: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AreaHover {
    pub area: String,
    pub tile: Pos<Meters>,
}

#[derive(Clone, Debug)]
pub enum AreaAction {
    Copy,
    Duplicate,
    Delete,
    OpenBlueprint(String),
    MakeBlueprint,
    GoTo(String, Pos<Meters>),
    Unpack(String),
    Replace(String),
    ReturnDoor(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, VariantArray, AsRefStr)]
pub enum AreaTool {
    #[default]
    Select,
    Stamp,
    Module,
}

impl AreaTool {
    pub fn key(self) -> EditorKey {
        match self {
            AreaTool::Select => EditorKey::SelectTool,
            AreaTool::Stamp => EditorKey::StampTool,
            AreaTool::Module => EditorKey::ModuleTool,
        }
    }

    pub fn bottom_tab(self) -> Option<BottomTab> {
        match self {
            AreaTool::Stamp => Some(BottomTab::Blueprints),
            _ => None,
        }
    }
}

#[derive(Default)]
pub struct AreaViewState {
    /// The area or blueprint bundle the view edits.
    pub bundle: String,
    pub in_place: Vec<String>,
    pub center: Pos<Meters>,
    pub zoom: f32,
    pub tool: AreaTool,
    pub walk_overlay: bool,
    pub grid_lines: bool,
    pub overlay_modules: bool,
    pub overlay_surfaces: bool,
    pub drag: Option<AreaDrag>,
    pub stamp: Option<String>,
    pub listener: Option<Pos<Meters>>,
    pub paste: Option<PasteGhost>,
    pub cursor_cell: Option<CellPos>,
    pub blueprint_search: String,
    pub menu: Option<modules::AreaMenuAnchor>,
    pub population_previews: HashMap<String, PopulationPreview>,
    pub skipped: Option<String>,
    /// The row shown alone for now, every other row's own flags kept as they are.
    pub show_only: Option<String>,
    /// The kind of module the Module tool places at the next click, into the insertion folder.
    pub placing: Option<AreaModuleKind>,
    pub scene: panel::ScenePanel,
    cache: Option<SnapshotCache>,
    fitted: bool,
}

impl AreaViewState {
    fn new(bundle: &str) -> AreaViewState {
        AreaViewState {
            bundle: bundle.to_owned(),
            zoom: 2.0,
            overlay_modules: true,
            ..AreaViewState::default()
        }
    }

    pub fn drag(&self) -> Option<&AreaDrag> {
        self.drag.as_ref()
    }

    pub fn drag_mut(&mut self) -> Option<&mut AreaDrag> {
        self.drag.as_mut()
    }

    pub fn set_drag(&mut self, drag: AreaDrag) {
        self.drag = Some(drag);
    }

    pub fn set_tool(&mut self, tool: AreaTool) {
        self.tool = tool;
        self.cancel_gesture();
    }

    pub fn cancel_gesture(&mut self) -> bool {
        self.drag.take().is_some() | self.placing.take().is_some()
    }

    pub fn take_drag(&mut self) -> Option<AreaDrag> {
        self.drag.take()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PopulationPreview {
    pub hidden: bool,
    pub seed: u64,
}

#[derive(Clone, Debug)]
pub enum AreaDrag {
    Box {
        from: Pos<Meters>,
    },
    Move {
        from: Pos<Meters>,
        moved: bool,
    },
    Resize {
        part: String,
        corner: ResizeCorner,
        start: Rect<Meters>,
    },
    Pin {
        file: String,
        pointer: String,
    },
    InitialSpawn,
    Stamp {
        placed: Vec<Pos<Meters>>,
        last: Pos<Meters>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Clone)]
/// What is being pasted, following the pointer until a click places it: the nodes as they read
/// when the paste started, drawn as a ghost.
pub struct PasteGhost {
    pub clip: crate::app::SceneClip,
    pub nodes: Vec<SceneNode>,
}

impl AreaViews {
    pub fn close(&mut self) {
        self.views.clear();
        self.active = None;
    }

    pub fn camera_state(&self) -> Option<AreaCameraState> {
        let bundle = self.active.as_ref()?;
        let view = self.views.get(&view_key(bundle))?;
        Some(AreaCameraState {
            bundle: bundle.clone(),
            center: view.center,
            zoom: view.zoom,
        })
    }

    pub fn restore_camera(&mut self, camera: &AreaCameraState) {
        if let Some(view) = self.views.get_mut(&view_key(&camera.bundle)) {
            view.center = camera.center;
            view.zoom = camera.zoom;
        }
    }

    pub fn ensure_view(&mut self, bundle: &str) -> &mut AreaViewState {
        self.views
            .entry(view_key(bundle))
            .or_insert_with(|| AreaViewState::new(bundle))
    }

    pub fn view(&self, key: &str) -> &AreaViewState {
        &self.views[key]
    }

    pub fn view_mut(&mut self, key: &str) -> &mut AreaViewState {
        self.views.get_mut(key).expect("an open area view")
    }

    /// Points what the views stamp with at where `moved` took it.
    pub fn moved(&mut self, moved: &dyn Fn(&str) -> Option<String>) {
        for view in self.views.values_mut() {
            if let Some(to) = view.stamp.as_deref().and_then(moved) {
                view.stamp = Some(to);
            }
        }
    }
}

/// The tab of an area: its scene graph on the left, its map, and the inspector.
pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    show_bundle(editor, ui, key);
}

/// The tab of a blueprint: the same view as an area's, on the blueprint's own canvas.
pub fn show_blueprint(editor: &mut Editor, ui: &mut Ui, key: &str) {
    show_bundle(editor, ui, key);
}

fn show_bundle(editor: &mut Editor, ui: &mut Ui, bundle: &str) {
    let view_key = view_key(bundle);
    editor.views.area.ensure_view(bundle);
    sidebar::panel(ui, "area", |ui| panel::show(editor, ui, &view_key));
    sidebar::inspector(editor, ui, "area", |editor, ui| {
        inspect::sidebar(editor, ui, &view_key)
    });
    show_canvas(editor, ui, &view_key);
    play::banner(editor, ui);
}

pub fn bottom(editor: &mut Editor, ui: &mut Ui, view_key: &str, tab: BottomTab) {
    if editor
        .project
        .as_ref()
        .is_none_or(|project| project.content.is_none())
    {
        ui.label(theme::muted("The folder does not load."));
        return;
    }
    if tab == BottomTab::Blueprints
        && let Some(snapshot) = AreaSnapshot::for_view(editor, view_key)
    {
        stamp::stamp_panel(editor, ui, view_key, &snapshot);
    }
}

fn show_canvas(editor: &mut Editor, ui: &mut Ui, view_key: &str) {
    let Some(snapshot) = AreaSnapshot::for_view(editor, view_key) else {
        let bundle = editor.views.area.view(view_key).bundle.clone();
        if let Some(Err(errors)) = nodes::read_bundle(editor, &bundle) {
            widgets::unreadable(ui, &bundle, &errors);
            return;
        }
        let loads = editor
            .project
            .as_ref()
            .is_some_and(|project| project.content.is_some());
        let said = if loads {
            "This bundle doesn't load; Output says why."
        } else {
            "The folder does not load; the view waits for it."
        };
        ui.label(egui::RichText::new(said).weak());
        return;
    };
    let rail_width = 44.0;
    let full = ui.available_rect_before_wrap();
    let rail = ERect::from_min_size(full.min, Vec2::new(rail_width, full.height()));
    let canvas_rect = ERect::from_min_max(Pos2::new(rail.max.x, full.min.y), full.max);
    fit_once(editor, view_key, &snapshot, canvas_rect);
    let mut canvas_ui = ui.new_child(egui::UiBuilder::new().max_rect(canvas_rect));
    canvas(editor, &mut canvas_ui, view_key, snapshot);
    banners(editor, ui, view_key, canvas_rect);
    let mut rail_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rail)
            .layout(egui::Layout::top_down(egui::Align::Center)),
    );
    rail_ui.painter().rect_filled(rail, 0.0, theme::PANEL);
    tools::rail(editor, &mut rail_ui, view_key);
}

fn fit_once(editor: &mut Editor, view_key: &str, snapshot: &AreaSnapshot, canvas: ERect) {
    let Some(project) = &editor.project else {
        return;
    };
    let remembered = editor
        .settings
        .cameras
        .get(&camera_key(project, view_key))
        .cloned();
    let view = editor.views.area.view_mut(view_key);
    if view.fitted {
        return;
    }
    let size = snapshot.size();
    view.center = Pos::new(size.width / 2.0 - 0.5, size.height / 2.0 - 0.5);
    let fit = navigation::fit_zoom(canvas.size(), Vec2::new(size.width, size.height), TILE.0)
        * FIT_MARGIN;
    view.zoom = fit.clamp(MIN_ZOOM, MAX_FIT_ZOOM);
    view.fitted = true;
    if let Some(camera) = remembered {
        view.center = Pos::new(camera.x, camera.y);
        view.zoom = camera.zoom;
    }
}

fn banners(editor: &mut Editor, ui: &mut Ui, view_key: &str, canvas_rect: ERect) {
    let Some(snapshot) = AreaSnapshot::for_view(editor, view_key) else {
        return;
    };
    let origin = canvas_rect.min + Vec2::new(8.0, 8.0);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(ERect::from_min_max(origin, canvas_rect.max))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let chip = |ui: &mut Ui, body: &mut dyn FnMut(&mut Ui)| {
        ui::slip::show(ui, |ui| {
            ui.horizontal(|ui| body(ui));
        });
        ui.add_space(4.0);
    };
    let mut done = false;
    if let Some(place) = snapshot.in_place.last() {
        let count = snapshot
            .content
            .by_path::<BlueprintDef>(&place.blueprint)
            .map_or(0, |id| {
                placement::placements_of(&snapshot.content, id).direct.len()
            });
        let label = snapshot
            .scene_node(&place_id(&editor.views.area.view(view_key).in_place))
            .and_then(|node| node.label().map(str::to_owned))
            .unwrap_or_else(|| file_title(&place.blueprint).to_owned());
        chip(&mut child, &mut |ui| {
            glyph::show(ui, Mark::Grid, 14.0, theme::WARN);
            ui.label(
                theme::mono(format!(
                    "Editing {label}'s blueprint · {count} placement{} change",
                    words::plural(count)
                ))
                .size(13.0)
                .color(theme::TEXT),
            );
            if controls::primary(ui, "Done").clicked() {
                done = true;
            }
            controls::key_hint(ui, &EditorKey::Cancel.name());
        });
    }
    let view = editor.views.area.view(view_key);
    if let Some(only) = view.show_only.clone() {
        let name = snapshot
            .scene_node(&only)
            .map(|node| shapes::node_label(node, snapshot.reader.as_ref()))
            .unwrap_or_default();
        let mut back = false;
        chip(&mut child, &mut |ui| {
            glyph::show(ui, Mark::Eye, 13.0, theme::MUTED);
            ui.label(
                theme::mono(format!("Showing only {name}"))
                    .size(13.0)
                    .color(theme::TEXT),
            );
            if ui.button("Show the rest").clicked() {
                back = true;
            }
            controls::key_hint(ui, &EditorKey::Cancel.name());
        });
        if back {
            editor.views.area.view_mut(view_key).show_only = None;
        }
    }
    let hidden = snapshot.flags.hidden;
    if editor.play.running.is_some() && hidden > 0 {
        chip(&mut child, &mut |ui| {
            glyph::show(ui, Mark::Play, 13.0, theme::MUTED);
            ui.label(
                theme::mono(format!(
                    "{hidden} hidden module{} play{} too",
                    words::plural(hidden),
                    if hidden == 1 { "s" } else { "" }
                ))
                .size(13.0)
                .color(theme::TEXT),
            );
        });
    }
    if done {
        let view = editor.views.area.view_mut(view_key);
        view.in_place.pop();
        editor.selection.parts.clear();
    }
}

fn canvas(editor: &mut Editor, ui: &mut Ui, view_key: &str, snapshot: AreaSnapshot) {
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::CANVAS);
    let bundle = snapshot.bundle.clone();
    let generation = editor.project.as_ref().map_or(0, |p| p.generation);
    let (center, zoom) = {
        let view = editor.views.area.view(view_key);
        (view.center, view.zoom)
    };
    let canvas = Viewport {
        screen: rect,
        center,
        zoom,
    };
    let area = snapshot.picture.area.clone();
    editor
        .views
        .area
        .grounds
        .of(
            ui.ctx(),
            &format!("{view_key}#{}", snapshot.made),
            generation,
            &snapshot.content,
            &area,
        )
        .draw(&painter, canvas, Color32::WHITE);
    let hovered = response.hover_pos().filter(|_| response.hovered());
    let pointer_tile = hovered.map(|p| canvas.to_tile(p));
    if let Some(tile) = pointer_tile {
        editor.views.area.hover = Some(AreaHover {
            area: bundle.clone(),
            tile,
        });
    }
    let is_area = matches!(snapshot.pictured, Pictured::Area(_));
    let frame = AreaFrame {
        snapshot,
        canvas,
        pointer: hovered,
        pointer_tile,
        response: &response,
        labels: AreaLabels::default(),
    };
    overlay_grid(
        &painter,
        &frame,
        &area,
        editor.views.area.view(view_key).grid_lines,
    );
    tools::input(editor, ui, &painter, view_key, &frame);
    locked_tip(editor, view_key, &frame);
    if is_area {
        editor.views.area.active = Some(bundle.clone());
    }
    overlay_props(&painter, &frame, &area);
    overlay_populations(editor, &painter, view_key, &frame, &area);
    overlay_soundscape_reach(editor, &painter, &frame);
    let view = editor.views.area.view(view_key);
    if view.overlay_surfaces {
        overlay_surfaces(&painter, &frame, &area);
    }
    if view.walk_overlay {
        overlay_walk(&painter, &frame, &area);
    }
    if view.overlay_modules || !editor.selection.parts.is_empty() {
        overlay_modules(editor, &painter, view_key, &frame);
    }
    if let Some(place) = frame.in_place.last() {
        placement::overlay_in_place(editor, &painter, &frame, place);
    }
    if is_area {
        pins::draw_arrivals(editor, &painter, &frame);
    }
    tools::overlay(editor, &painter, view_key, &frame);
    frame.labels.draw(&painter);
    status_bar(editor, &painter, view_key, &frame);
    if response.hovered() && !ui.ctx().text_edit_focused() {
        tools::keys(editor, ui, view_key, &frame);
    }
    remember_camera(editor, view_key);
}

/// Says what locks the module under the pointer, since a click passes through it.
fn locked_tip(editor: &Editor, view_key: &str, frame: &AreaFrame) {
    if editor.views.area.view(view_key).tool != AreaTool::Select {
        return;
    }
    let Some(locked) = frame.pointer_tile.and_then(|tile| frame.locked_hit(tile)) else {
        return;
    };
    let by = std::iter::successors(Some(locked.clone()), |id| {
        Some(scene_tree::parent_id(id)).filter(|parent| !parent.is_empty())
    })
    .find(|id| frame.flags.row(id).locked == flags::Flag::Own)
    .filter(|id| *id != locked)
    .and_then(|id| {
        frame
            .scene_node(&id)
            .map(|node| shapes::node_label(node, frame.reader.as_ref()))
    });
    let what = frame
        .module(&locked)
        .map(|module| shapes::label(module, frame.reader.as_ref()))
        .unwrap_or_default();
    let text = match by {
        Some(folder) => format!("{what} is locked by {folder}: a click selects what's under it"),
        None => format!("{what} is locked: a click selects what's under it"),
    };
    frame.response.clone().on_hover_text_at_pointer(text);
}

/// What a view shows of its bundle: the scene as the editor holds it, the picture it makes with
/// what is hidden left out, and the placements being edited in place.
pub struct AreaSnapshot {
    /// Made anew only when what the snapshot holds is, so what is found from it may be kept
    /// until this changes.
    pub made: u64,
    pub content: Content,
    pub bundle: String,
    pub pictured: Pictured,
    pub picture: Arc<Picture>,
    /// The bundle as its files read now, its ids those of `reader`.
    pub def: nodes::SceneBundle,
    pub reader: Arc<FolderReader>,
    pub flags: Arc<SceneFlags>,
    pub modules: Arc<nodes::SceneModules>,
    pub in_place: Vec<placement::AreaPlacement>,
    pub placement_bounds: Arc<HashMap<String, Rect<Meters>>>,
    targets: Arc<[MapTarget]>,
}

/// A module of the bundle, where the pointer finds it on the map: hovering asks of every one on
/// every frame.
struct MapTarget {
    pointer: String,
    bounds: Rect<Meters>,
    shown: bool,
    locked: bool,
}

#[derive(Clone)]
struct SnapshotCache {
    generation: u64,
    revision: u64,
    show_only: Option<String>,
    in_place: Vec<String>,
    snapshot: SnapshotParts,
}

#[derive(Clone)]
struct SnapshotParts {
    made: u64,
    pictured: Pictured,
    picture: Arc<Picture>,
    def: nodes::SceneBundle,
    reader: Arc<FolderReader>,
    flags: Arc<SceneFlags>,
    modules: Arc<nodes::SceneModules>,
    in_place: Vec<placement::AreaPlacement>,
    placement_bounds: Arc<HashMap<String, Rect<Meters>>>,
    targets: Arc<[MapTarget]>,
}

pub struct AreaFrame<'a> {
    pub snapshot: AreaSnapshot,
    pub canvas: Viewport<Meters>,
    pub pointer: Option<Pos2>,
    pub pointer_tile: Option<Pos<Meters>>,
    pub response: &'a egui::Response,
    pub labels: AreaLabels,
}

impl Deref for AreaFrame<'_> {
    type Target = AreaSnapshot;

    fn deref(&self) -> &AreaSnapshot {
        &self.snapshot
    }
}

impl AreaSnapshot {
    /// The snapshot of an open view, made again when the files, the content or the view's flags
    /// change. Its picture is drawn again only when the content or what's hidden changes, so a
    /// change to the scene shows without waiting for the folder to load again.
    pub fn for_view(editor: &mut Editor, view_key: &str) -> Option<Self> {
        let project = editor.project.as_ref()?;
        let content = project.content.clone()?;
        let generation = project.generation;
        let revision = project.files.revision();
        let view = editor.views.area.views.get(view_key)?;
        let bundle = view.bundle.clone();
        let current = view.cache.as_ref().filter(|cache| {
            cache.generation == generation
                && cache.show_only == view.show_only
                && cache.in_place == view.in_place
        });
        let parts = match current.filter(|cache| cache.revision == revision) {
            Some(cache) => cache.snapshot.clone(),
            None => {
                let drawn = current.map(|cache| cache.snapshot.clone());
                let show_only = view.show_only.clone();
                let path = view.in_place.clone();
                let made = snapshot_parts(
                    editor,
                    &content,
                    &bundle,
                    &path,
                    show_only.as_deref(),
                    drawn.as_ref(),
                );
                let view = editor.views.area.view_mut(view_key);
                let Some(parts) = made else {
                    view.in_place.clear();
                    view.cache = None;
                    return None;
                };
                if parts.in_place.len() != path.len() {
                    view.in_place.truncate(parts.in_place.len());
                }
                view.cache = Some(SnapshotCache {
                    generation,
                    revision,
                    show_only,
                    in_place: view.in_place.clone(),
                    snapshot: parts.clone(),
                });
                parts
            }
        };
        Some(Self::from_parts(content, bundle, parts))
    }

    /// The snapshot of `bundle`, through its view when one is open.
    pub fn for_bundle(editor: &mut Editor, bundle: &str) -> Option<Self> {
        let key = view_key(bundle);
        if editor.views.area.views.contains_key(&key) {
            return Self::for_view(editor, &key);
        }
        let content = editor.project.as_ref()?.content.clone()?;
        let parts = snapshot_parts(editor, &content, bundle, &[], None, None)?;
        Some(Self::from_parts(content, bundle.to_owned(), parts))
    }

    fn from_parts(content: Content, bundle: String, parts: SnapshotParts) -> Self {
        Self {
            made: parts.made,
            content,
            bundle,
            pictured: parts.pictured,
            picture: parts.picture,
            def: parts.def,
            reader: parts.reader,
            flags: parts.flags,
            modules: parts.modules,
            in_place: parts.in_place,
            placement_bounds: parts.placement_bounds,
            targets: parts.targets,
        }
    }

    pub fn area(&self) -> &area::Area {
        &self.picture.area
    }

    /// The area this is the view of, when it is one.
    pub fn area_id(&self) -> Option<AreaId> {
        match self.pictured {
            Pictured::Area(id) => Some(id),
            Pictured::Blueprint(_) => None,
        }
    }

    pub fn size(&self) -> Size<Meters> {
        self.area().size
    }

    pub fn in_map(&self, tile: Pos<Meters>) -> bool {
        self.size().bounds().contains(tile)
    }

    /// The bundle's own scene, as its files read now.
    pub fn nodes(&self) -> &[SceneNode] {
        self.def.nodes()
    }

    /// The node of the scene a tree row stands for, through the placements edited in place.
    pub fn scene_node(&self, id: &str) -> Option<&SceneNode> {
        let mut levels = id.split("/in/");
        let top = levels.next()?;
        let mut node = nodes::node(self.nodes(), top)?;
        for (pointer, place) in levels.zip(self.in_place.iter()) {
            node = nodes::node(&place.def.scene, &format!("/{pointer}"))?;
        }
        Some(node)
    }

    pub fn module_node(&self, pointer: &str) -> Option<&ModuleNode> {
        self.modules.get(pointer).map(|module| &module.node)
    }

    pub fn module(&self, pointer: &str) -> Option<&AreaModule> {
        self.module_node(pointer).map(|node| &node.module)
    }

    pub fn module_bounds(&self, pointer: &str) -> Option<Rect<Meters>> {
        self.bounds_of(self.modules.get(pointer)?)
    }

    /// Where `module`, one of the bundle's own, takes room on the map.
    pub fn bounds_of(&self, module: &nodes::SceneModule) -> Option<Rect<Meters>> {
        shapes::bounds(
            module.module(),
            self.placement_bounds.get(&module.pointer).copied(),
        )
    }

    /// Whether the map lets the pointer at the module: not hidden, not locked, and not outside the
    /// placement edited in place.
    pub fn pickable(&self, pointer: &str) -> bool {
        !self.flags.hidden(pointer) && !self.flags.locked(pointer) && self.in_place.is_empty()
    }

    pub fn hit(&self, tile: Pos<Meters>) -> Option<String> {
        if !self.in_place.is_empty() {
            return None;
        }
        let mut best: Option<&MapTarget> = None;
        for target in self.targets.iter() {
            if target.shown
                && !target.locked
                && target.bounds.contains(tile)
                && best.is_none_or(|best| target.bounds.area() < best.bounds.area())
            {
                best = Some(target);
            }
        }
        best.map(|target| target.pointer.clone())
    }

    /// The modules the selection `parts` takes on the map: those selected, and every visible,
    /// unlocked module inside a selected folder.
    pub fn selected_modules(&self, parts: &HashSet<String>) -> Vec<String> {
        let folders: Vec<String> = parts
            .iter()
            .filter(|part| self.scene_node(part).is_some_and(nodes::is_folder))
            .map(|folder| format!("{folder}/"))
            .collect();
        self.modules
            .iter()
            .filter(|module| {
                parts.contains(&module.pointer)
                    || (self.pickable(&module.pointer)
                        && folders
                            .iter()
                            .any(|folder| module.pointer.starts_with(folder)))
            })
            .map(|module| module.pointer.clone())
            .collect()
    }

    /// The box around what the selected folders of `parts` hold on the map, with the label of the
    /// first, when a folder is selected.
    pub fn folder_box(&self, parts: &HashSet<String>) -> Option<(Rect<Meters>, String)> {
        let folders: Vec<&String> = parts
            .iter()
            .filter(|part| self.scene_node(part).is_some_and(nodes::is_folder))
            .collect();
        let label = folders
            .iter()
            .min()
            .and_then(|folder| self.scene_node(folder))
            .map(|node| shapes::node_label(node, self.reader.as_ref()))?;
        let bounds = self
            .modules
            .iter()
            .filter(|module| {
                self.pickable(&module.pointer)
                    && is_selectable(module.module())
                    && folders
                        .iter()
                        .any(|folder| module.pointer.starts_with(&format!("{folder}/")))
            })
            .filter_map(|module| self.bounds_of(module))
            .reduce(|a, b| a.union(&b))?;
        Some((bounds, label))
    }

    /// The names objects take in the bundle: its own, as the editor holds them, and those its
    /// placements bring.
    pub fn object_names(&self) -> HashSet<String> {
        let own = self
            .modules
            .iter()
            .filter_map(|module| match module.module() {
                AreaModule::Object(object) => object.name.clone(),
                _ => None,
            });
        own.chain(stamp::object_names(&self.picture.placed))
            .collect()
    }

    /// The locked module under `tile` a click passes through, for the tip that says what locks it.
    pub fn locked_hit(&self, tile: Pos<Meters>) -> Option<String> {
        self.targets
            .iter()
            .filter(|target| target.shown && target.locked && target.bounds.contains(tile))
            .min_by(|a, b| a.bounds.area().total_cmp(&b.bounds.area()))
            .map(|target| target.pointer.clone())
    }
}

/// The snapshot of `bundle` as its files read now, its picture `drawn`'s when that one was drawn
/// from the same content with the same rows hidden.
fn snapshot_parts(
    editor: &Editor,
    content: &Content,
    bundle: &str,
    in_place: &[String],
    show_only: Option<&str>,
    drawn: Option<&SnapshotParts>,
) -> Option<SnapshotParts> {
    let pictured = match table_of(bundle)? {
        AreaDef::TABLE => Pictured::Area(content.by_path::<AreaDef>(bundle)?),
        BlueprintDef::TABLE => Pictured::Blueprint(content.by_path::<BlueprintDef>(bundle)?),
        _ => return None,
    };
    let def = nodes::bundle_of(editor, bundle)?;
    let reader = editor.reader()?;
    let in_place = placement::resolve_in_place(editor, def.nodes(), in_place).unwrap_or_default();
    let mut levels = vec![SceneLevel {
        under: String::new(),
        scene: def.nodes(),
    }];
    for (depth, place) in in_place.iter().enumerate() {
        levels.push(SceneLevel {
            under: place_id_of(&in_place[..=depth]),
            scene: &place.def.scene,
        });
    }
    let flags = SceneFlags::of(&levels, show_only);
    let (picture, placement_bounds) = match drawn.filter(|drawn| drawn.flags.hides_alike(&flags)) {
        Some(drawn) => (drawn.picture.clone(), drawn.placement_bounds.clone()),
        None => {
            let keep = |path: &[&str]| flags.keep(path);
            let kept = if flags.any_hidden() {
                area::Kept::Only(&keep)
            } else {
                area::Kept::All
            };
            let picture = Arc::new(area::picture(content, pictured, kept)?);
            let placement_bounds = Arc::new(placement::placement_bounds(&picture.placed));
            (picture, placement_bounds)
        }
    };
    let modules = nodes::SceneModules::of(def.nodes());
    let targets = modules
        .iter()
        .filter(|module| is_selectable(module.module()))
        .filter_map(|module| {
            Some(MapTarget {
                bounds: shapes::bounds(
                    module.module(),
                    placement_bounds.get(&module.pointer).copied(),
                )?,
                shown: !flags.hidden(&module.pointer),
                locked: flags.locked(&module.pointer),
                pointer: module.pointer.clone(),
            })
        })
        .collect();
    static MADE: AtomicU64 = AtomicU64::new(0);
    Some(SnapshotParts {
        made: MADE.fetch_add(1, Ordering::Relaxed),
        pictured,
        picture,
        modules: Arc::new(modules),
        def,
        reader,
        flags: Arc::new(flags),
        in_place,
        placement_bounds,
        targets,
    })
}

/// The tree row of the placement edited in place at the end of `path`.
pub fn place_id(path: &[String]) -> String {
    path.join("/in")
}

fn place_id_of(chain: &[placement::AreaPlacement]) -> String {
    chain
        .iter()
        .map(|place| place.node.as_str())
        .collect::<Vec<_>>()
        .join("/in")
}

impl AreaFrame<'_> {
    /// Where the scene being edited stands on the map, and how it is turned: the placement edited
    /// in place, or the bundle's own origin.
    pub fn origin(&self) -> (Pos<Meters>, Degrees) {
        match self.in_place.last() {
            Some(place) => (place.at, place.yaw),
            None => (Pos::new(0.0, 0.0), Degrees::default()),
        }
    }

    pub fn pointer_cell(&self) -> Option<CellPos> {
        self.pointer_tile.map(|t| t.cell())
    }

    pub fn drag_tile(&self) -> Option<Pos<Meters>> {
        self.pointer_tile.or_else(|| {
            self.response
                .interact_pointer_pos()
                .map(|p| self.canvas.to_tile(p))
        })
    }
}

fn is_selectable(module: &AreaModule) -> bool {
    !matches!(module, AreaModule::Population(population) if population.roams.is_none())
}

pub fn selected_parts(editor: &Editor, bundle: &str) -> HashSet<String> {
    selected_parts_of(editor, bundle)
        .into_iter()
        .cloned()
        .collect()
}

fn selected_parts_of<'a>(editor: &'a Editor, bundle: &str) -> Vec<&'a String> {
    if editor.selection.keys.iter().any(|k| k == bundle) {
        editor.selection.parts.iter().collect()
    } else {
        Vec::new()
    }
}

fn remember_camera(editor: &mut Editor, view_key: &str) {
    let Some(project) = &editor.project else {
        return;
    };
    let view = editor.views.area.view(view_key);
    let key = camera_key(project, view_key);
    let camera = user::CameraSettings {
        x: view.center.x,
        y: view.center.y,
        zoom: view.zoom,
    };
    let changed = editor.settings.cameras.get(&key).is_none_or(|known| {
        (known.x - camera.x).abs() > 0.01
            || (known.y - camera.y).abs() > 0.01
            || (known.zoom - camera.zoom).abs() > 0.001
    });
    if changed {
        editor.settings.cameras.insert(key, camera);
        if editor.frame.is_multiple_of(120) {
            editor.save_settings();
        }
    }
}

pub fn open_area_at(
    editor: &mut Editor,
    area: &str,
    at: Option<Pos<Meters>>,
    part: Option<String>,
) {
    editor.open_tab(EditorTab::Area {
        key: area.to_owned(),
    });
    let view = editor.views.area.ensure_view(area);
    if let Some(at) = at {
        view.center = at;
        view.fitted = true;
    }
    if let Some(part) = part {
        editor.selection = RowSelection {
            table: AreaDef::TABLE.to_owned(),
            keys: vec![area.to_owned()],
            parts: vec![part],
        };
    }
}

/// Drops the selected nodes a change on disk took away; answers what was dropped, said for a toast.
pub fn drop_gone(editor: &mut Editor) -> Option<String> {
    let bundle = editor.selection.keys.first()?.clone();
    if !matches!(
        table_of(&bundle),
        Some(AreaDef::TABLE | BlueprintDef::TABLE)
    ) {
        return None;
    }
    let scene = nodes::scene_of(editor, &bundle);
    let before = editor.selection.parts.len();
    editor
        .selection
        .parts
        .retain(|part| nodes::node(&scene, part).is_some());
    let gone = before - editor.selection.parts.len();
    let what = match (gone, editor.selection.parts.is_empty()) {
        (0, _) => return None,
        (1, true) => "the selected node is gone, so the selection was cleared".to_owned(),
        (n, true) => format!("the {n} selected nodes are gone, so the selection was cleared"),
        (n, false) => format!(
            "{n} selected node{} gone and left the selection",
            if n == 1 { " is" } else { "s are" }
        ),
    };
    Some(format!("{} changed on disk: {what}", file_title(&bundle)))
}

pub fn run_action(editor: &mut Editor, frame: &AreaSnapshot, action: AreaAction) {
    match action {
        AreaAction::Copy => select::copy_selection(editor, frame),
        AreaAction::Duplicate => select::duplicate_selection(editor, frame),
        AreaAction::Delete => select::delete_selection(editor, frame),
        AreaAction::OpenBlueprint(key) => {
            editor.select(RowSelection::row(BlueprintDef::TABLE, &key));
            editor.open_tab(EditorTab::Blueprint { key });
        }
        AreaAction::MakeBlueprint => blueprint::make_blueprint_from_selection(editor, frame),
        AreaAction::GoTo(area, at) => open_area_at(editor, &area, Some(at), None),
        AreaAction::Unpack(node) => placement::unpack(editor, frame, &node),
        AreaAction::Replace(node) => placement::replace_dialog(editor, frame, &node),
        AreaAction::ReturnDoor(node) => placement::return_door(editor, frame, &node),
    }
}
