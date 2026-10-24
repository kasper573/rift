pub mod blueprint;
pub mod brush;
pub mod flags;
pub mod grid;
pub mod inspect;
pub mod layers;
pub mod modules;
pub mod nodes;
pub mod overlays;
pub mod palette;
pub mod panel;
pub mod pick;
pub mod pins;
pub mod placement;
pub mod population;
pub mod preview;
pub mod resize;
pub mod scene;
pub mod scene_tree;
pub mod scenery;
pub mod select;
pub mod shapes;
pub mod soundscape;
pub mod stamp;
pub mod status;
pub mod terrain;
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

use bevy::prelude::World;
use engine::core::content::{Content, ContentFile, FolderReader, table_of};
use engine::core::math::{Offset, Pos, Rect, Size};
use engine::core::render::TILE;
use engine::core::units::{CellPos, GridDims, Meters, TilePos, TileSize};
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::scene::{ModuleNode, SceneNode};
use engine::systems::area::{self, AreaDef, AreaId, AreaModule, AreaModuleKind, Picture, Pictured};
use strum::{AsRefStr, VariantArray};
use ui::egui::{Color32, Pos2, Rect as ERect, Sense, Ui, Vec2};
use ui::glyph::{self, Mark};

use self::flags::{SceneFlags, SceneLevel};
use self::overlays::{
    AreaLabels, overlay_grid, overlay_modules, overlay_populations, overlay_selected_scenery,
    overlay_sort_lines, overlay_soundscape_reach, overlay_surfaces, overlay_walk,
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
    pub scenes: scene::AreaScenes,
    pub requests: Vec<scene::AreaSceneRequest>,
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
    Split(String),
    ReturnDoor(String),
    AdoptBlocks(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, VariantArray, AsRefStr)]
pub enum AreaTool {
    #[default]
    Select,
    Paint,
    Terrain,
    Fill,
    Erase,
    Eyedropper,
    Stamp,
    Module,
}

impl AreaTool {
    /// Whether the tool paints the cells of the armed layer or walk.
    pub fn paints(self) -> bool {
        matches!(
            self,
            AreaTool::Paint
                | AreaTool::Terrain
                | AreaTool::Fill
                | AreaTool::Erase
                | AreaTool::Eyedropper
        )
    }

    pub fn key(self) -> EditorKey {
        match self {
            AreaTool::Select => EditorKey::SelectTool,
            AreaTool::Paint => EditorKey::PaintTool,
            AreaTool::Terrain => EditorKey::TerrainTool,
            AreaTool::Fill => EditorKey::FillTool,
            AreaTool::Erase => EditorKey::EraseTool,
            AreaTool::Eyedropper => EditorKey::EyedropperTool,
            AreaTool::Stamp => EditorKey::StampTool,
            AreaTool::Module => EditorKey::ModuleTool,
        }
    }

    pub fn bottom_tab(self) -> Option<BottomTab> {
        match self {
            AreaTool::Paint | AreaTool::Terrain | AreaTool::Fill | AreaTool::Eyedropper => {
                Some(BottomTab::Palette)
            }
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
    /// Keeps the walk brush off, as a layer's padlock keeps the brushes off it.
    pub walk_locked: bool,
    pub grid_lines: bool,
    pub overlay_modules: bool,
    pub overlay_surfaces: bool,
    pub overlay_sort_lines: bool,
    pub gesture: Option<AreaGesture>,
    pub pick: brush::TilePick,
    pub terrain: brush::TerrainPick,
    pub stamp: Option<String>,
    pub listener: Option<Pos<Meters>>,
    pub paste: Option<PasteGhost>,
    pub cursor_cell: Option<CellPos>,
    pub tileset_search: String,
    pub blueprint_search: String,
    pub palette_lens: palette::PaletteLens,
    pub menu: Option<modules::AreaMenuAnchor>,
    pub population_previews: HashMap<String, PopulationPreview>,
    pub walk_value: Option<bool>,
    pub skipped: Option<String>,
    /// The area a blueprint is drawn over in its own tab, a view setting only.
    pub backdrop: Option<String>,
    /// The row shown alone for now, every other row's own flags kept as they are.
    pub show_only: Option<String>,
    /// What the selection arms the brushes with, as last seen.
    pub armed: Armed,
    /// The layer the brushes last painted, armed again when a brush is taken with nothing armed.
    pub last_layer: Option<String>,
    /// The kind of module the Module tool places at the next click, into the insertion folder.
    pub placing: Option<AreaModuleKind>,
    pub scene: panel::ScenePanel,
    cache: Option<SnapshotCache>,
    fitted: bool,
    backdrop_chosen: bool,
}

impl AreaViewState {
    fn new(bundle: &str) -> AreaViewState {
        AreaViewState {
            bundle: bundle.to_owned(),
            zoom: 2.0,
            overlay_modules: true,
            walk_value: Some(false),
            ..AreaViewState::default()
        }
    }

    pub fn drag(&self) -> Option<&AreaDrag> {
        match &self.gesture {
            Some(AreaGesture::Drag(drag)) => Some(drag),
            _ => None,
        }
    }

    pub fn drag_mut(&mut self) -> Option<&mut AreaDrag> {
        match &mut self.gesture {
            Some(AreaGesture::Drag(drag)) => Some(drag),
            _ => None,
        }
    }

    pub fn set_drag(&mut self, drag: AreaDrag) {
        self.gesture = Some(AreaGesture::Drag(drag));
    }

    pub fn set_tool(&mut self, tool: AreaTool) {
        self.tool = tool;
        self.cancel_gesture();
    }

    pub fn cancel_gesture(&mut self) -> bool {
        self.gesture.take().is_some() | self.placing.take().is_some()
    }

    pub fn take_drag(&mut self) -> Option<AreaDrag> {
        match self.gesture.take() {
            Some(AreaGesture::Drag(drag)) => Some(drag),
            other => {
                self.gesture = other;
                None
            }
        }
    }

    pub fn stroke(&self) -> Option<&brush::BrushStroke> {
        match &self.gesture {
            Some(AreaGesture::Stroke(stroke)) => Some(stroke),
            _ => None,
        }
    }

    pub fn stroke_mut(&mut self) -> Option<&mut brush::BrushStroke> {
        match &mut self.gesture {
            Some(AreaGesture::Stroke(stroke)) => Some(stroke),
            _ => None,
        }
    }

    pub fn set_stroke(&mut self, stroke: brush::BrushStroke) {
        self.gesture = Some(AreaGesture::Stroke(stroke));
    }

    pub fn take_stroke(&mut self) -> Option<brush::BrushStroke> {
        match self.gesture.take() {
            Some(AreaGesture::Stroke(stroke)) => Some(stroke),
            other => {
                self.gesture = other;
                None
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PopulationPreview {
    pub hidden: bool,
    pub seed: u64,
}

pub enum AreaGesture {
    Drag(AreaDrag),
    Stroke(brush::BrushStroke),
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
        self.requests.clear();
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

    pub fn content_changed(&mut self) {
        for view in self.views.values_mut() {
            if matches!(view.gesture, Some(AreaGesture::Stroke(_))) {
                view.gesture = None;
            }
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

    /// Points what the views have picked, stamp and draw behind at where `moved` took it.
    pub fn moved(&mut self, moved: &dyn Fn(&str) -> Option<String>) {
        for view in self.views.values_mut() {
            for picked in [
                &mut view.pick.tileset,
                &mut view.terrain.tileset,
                &mut view.stamp,
                &mut view.backdrop,
            ] {
                if let Some(to) = picked.as_deref().and_then(moved) {
                    *picked = Some(to);
                }
            }
        }
    }
}

pub fn sync(editor: &mut Editor, world: &mut World) {
    let requests = std::mem::take(&mut editor.views.area.requests);
    let (content, files) = match &editor.project {
        Some(project) => (project.content.clone(), &project.files),
        None => {
            scene::clear(&mut editor.views.area.scenes, world);
            return;
        }
    };
    let Editor { views, .. } = editor;
    scene::sync(
        &mut views.area.scenes,
        requests,
        content.as_ref(),
        files,
        world,
    );
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
    match tab {
        BottomTab::Palette => {
            if let Some(snapshot) = AreaSnapshot::for_view(editor, view_key) {
                palette::show(editor, ui, view_key, &snapshot);
            }
        }
        BottomTab::Blueprints => {
            if let Some(snapshot) = AreaSnapshot::for_view(editor, view_key) {
                stamp::stamp_panel(editor, ui, view_key, &snapshot);
            }
        }
        _ => {}
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
    choose_backdrop_once(editor, view_key, &snapshot);
    fit_once(editor, view_key, &snapshot, canvas_rect);
    rearm(editor, view_key, &snapshot);
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
    let backdrop = editor.views.area.view(view_key).backdrop.clone();
    let framed = backdrop.and_then(|area| {
        let shift = backdrop_shift(snapshot, &area)?;
        let id = snapshot.content.by_path::<AreaDef>(&area)?;
        Some((area::load(&snapshot.content, id).size, shift))
    });
    let view = editor.views.area.view_mut(view_key);
    if view.fitted {
        return;
    }
    let (size, shift) = framed.unwrap_or((snapshot.size(), Offset::zero()));
    view.center = Pos::new(size.width / 2.0 - 0.5, size.height / 2.0 - 0.5) - shift;
    let fit = navigation::fit_zoom(canvas.size(), Vec2::new(size.width, size.height), TILE.0)
        * FIT_MARGIN;
    view.zoom = fit.clamp(MIN_ZOOM, MAX_FIT_ZOOM);
    view.fitted = true;
    if let Some(camera) = remembered {
        view.center = Pos::new(camera.x, camera.y);
        view.zoom = camera.zoom;
    }
}

/// A blueprint's tab starts over the first area that places it, so it is judged on the ground it
/// stands on.
fn choose_backdrop_once(editor: &mut Editor, view_key: &str, snapshot: &AreaSnapshot) {
    if !matches!(snapshot.pictured, Pictured::Blueprint(_))
        || editor.views.area.view(view_key).backdrop_chosen
    {
        return;
    }
    let first = placement::first_area_using(editor, &snapshot.bundle);
    let view = editor.views.area.view_mut(view_key);
    view.backdrop = first;
    view.backdrop_chosen = true;
}

/// What the selection arms the brushes with: a ground layer of the bundle's own, or its walk.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Armed {
    #[default]
    Nothing,
    Layer {
        pointer: String,
        name: String,
    },
    Walk,
}

/// The selection part that stands for the Walk row, which is no node of the scene.
pub const WALK_ROW: &str = "walk";

pub fn armed(editor: &Editor, snapshot: &AreaSnapshot) -> Armed {
    let parts: Vec<&String> = selected_parts_of(editor, &snapshot.bundle);
    match parts.as_slice() {
        [part] if *part == WALK_ROW => Armed::Walk,
        [part] => match nodes::node(snapshot.nodes(), part).and_then(nodes::layer_name) {
            Some(name) => Armed::Layer {
                pointer: (*part).clone(),
                name: name.to_owned(),
            },
            None => Armed::Nothing,
        },
        _ => Armed::Nothing,
    }
}

/// Follows the selection with the tool in hand: selecting a layer takes a brush to it, selecting the
/// Walk row takes the walk brush, and selecting anything else puts the brushes down.
fn rearm(editor: &mut Editor, view_key: &str, snapshot: &AreaSnapshot) {
    let now = armed(editor, snapshot);
    let view = editor.views.area.view_mut(view_key);
    if view.armed == now {
        return;
    }
    let brushing = view.tool.paints();
    match &now {
        Armed::Layer { name, .. } => {
            view.last_layer = Some(name.clone());
            if !brushing {
                view.set_tool(AreaTool::Paint);
            }
        }
        Armed::Walk => {
            if !matches!(
                view.tool,
                AreaTool::Paint | AreaTool::Fill | AreaTool::Erase
            ) {
                view.set_tool(AreaTool::Paint);
            }
        }
        Armed::Nothing if brushing => view.set_tool(AreaTool::Select),
        Armed::Nothing => {}
    }
    view.armed = now;
    if view.tool.paints() {
        editor.panels.bottom = BottomTab::Palette;
    }
}

/// Takes a brush with nothing armed: arms the layer painted last, or the bundle's topmost layer.
pub fn arm_a_layer(editor: &mut Editor, view_key: &str) {
    let Some(snapshot) = AreaSnapshot::for_view(editor, view_key) else {
        return;
    };
    if armed(editor, &snapshot) != Armed::Nothing {
        return;
    }
    let last = editor.views.area.view(view_key).last_layer.clone();
    let mut found: Vec<(String, String)> = Vec::new();
    nodes::walk(snapshot.nodes(), &mut |pointer, node| {
        if let Some(name) = nodes::layer_name(node) {
            found.push((pointer.to_owned(), name.to_owned()));
        }
    });
    let pick = found
        .iter()
        .find(|(_, name)| Some(name) == last.as_ref())
        .or(found.last());
    if let Some((pointer, name)) = pick.cloned() {
        nodes::select(editor, &snapshot.bundle, vec![pointer.clone()]);
        let view = editor.views.area.view_mut(view_key);
        view.armed = Armed::Layer { pointer, name };
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
    let view = editor.views.area.view(view_key);
    let brushing = view.tool.paints() && view.in_place.is_empty();
    if brushing {
        let (said, ink) = match armed(editor, &snapshot) {
            Armed::Layer { pointer, name } if snapshot.flags.locked(&pointer) => {
                (format!("{name} is locked: no brush paints it"), theme::WARN)
            }
            Armed::Layer { pointer, name } if snapshot.flags.hidden(&pointer) => (
                format!("{name} is hidden: show it to paint it"),
                theme::WARN,
            ),
            Armed::Layer { name, .. } => {
                (format!("{} into {name}", view.tool.as_ref()), theme::TEXT)
            }
            Armed::Walk if view.walk_locked => (
                "Walk is locked: the walk brush is off".to_owned(),
                theme::WARN,
            ),
            Armed::Walk => ("Paint forced walk cells".to_owned(), theme::TEXT),
            Armed::Nothing => (
                "Select a ground layer to paint into".to_owned(),
                theme::WARN,
            ),
        };
        chip(&mut child, &mut |ui| {
            glyph::show(ui, Mark::Grid, 13.0, ink);
            ui.label(theme::mono(&said).size(13.0).color(ink));
        });
    }
    if let Pictured::Blueprint(_) = snapshot.pictured {
        let areas: Vec<String> = snapshot.content.table::<AreaDef>().paths().to_vec();
        let backdrop = editor.views.area.view(view_key).backdrop.clone();
        let mut picked = backdrop.clone();
        chip(&mut child, &mut |ui| {
            glyph::show(ui, Mark::Eye, 13.0, theme::MUTED);
            ui.label(theme::mono("backdrop").size(13.0).color(theme::TEXT));
            controls::combo((view_key, "backdrop"))
                .selected_text(theme::mono(
                    picked.as_deref().map_or("none", file_title).to_owned(),
                ))
                .show_ui(ui, |ui| {
                    if ui.selectable_label(picked.is_none(), "none").clicked() {
                        picked = None;
                    }
                    for area in &areas {
                        if ui
                            .selectable_label(picked.as_ref() == Some(area), theme::mono(area))
                            .clicked()
                        {
                            picked = Some(area.clone());
                        }
                    }
                });
        });
        if picked != backdrop {
            editor.views.area.view_mut(view_key).backdrop = picked;
        }
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
    let ppp = ui.ctx().pixels_per_point();
    let generation = editor.project.as_ref().map_or(0, |p| p.generation);
    let width = (rect.width() * ppp).round().max(1.0) as u32;
    let height = (rect.height() * ppp).round().max(1.0) as u32;
    let (center, zoom, backdrop) = {
        let view = editor.views.area.view(view_key);
        (view.center, view.zoom, view.backdrop.clone())
    };
    let backdrop = backdrop
        .filter(|_| matches!(snapshot.pictured, Pictured::Blueprint(_)))
        .and_then(|area| {
            let shift = backdrop_shift(&snapshot, &area)?;
            Some((area, shift))
        });
    if let Some((area, shift)) = &backdrop {
        let key = format!("{view_key}#backdrop");
        editor.views.area.requests.push(scene::AreaSceneRequest {
            key: key.clone(),
            spec: scene::AreaSceneSpec::ground(area),
            width,
            height,
            center: center + *shift,
            zoom: zoom * ppp,
            generation,
            live: true,
        });
        if let Some(scene) = editor.views.area.scenes.by_path.get(&key) {
            painter.image(scene.texture, rect, scene.uv(), Color32::from_gray(150));
        }
    }
    editor.views.area.requests.push(scene::AreaSceneRequest {
        key: view_key.to_owned(),
        spec: scene::AreaSceneSpec {
            drawn: scene::Drawn::Picture(snapshot.picture.clone()),
            hidden_ground: snapshot.hidden_ground(),
            see_through: backdrop.is_some(),
        },
        width,
        height,
        center,
        zoom: zoom * ppp,
        generation,
        live: true,
    });
    if let Some(scene) = editor.views.area.scenes.by_path.get(view_key) {
        painter.image(scene.texture, rect, scene.uv(), Color32::WHITE);
    }
    let canvas = Viewport {
        screen: rect,
        center,
        zoom,
    };
    let hovered = response.hover_pos().filter(|_| response.hovered());
    let pointer_tile = hovered.map(|p| canvas.to_tile(p));
    if let Some(tile) = pointer_tile {
        editor.views.area.hover = Some(AreaHover {
            area: bundle.clone(),
            tile,
        });
    }
    let is_area = matches!(snapshot.pictured, Pictured::Area(_));
    let area = snapshot.picture.area.clone();
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
    overlay_populations(editor, ui, &painter, view_key, &frame, &area);
    overlay_selected_scenery(editor, ui, &painter, view_key, &frame, &area);
    overlay_soundscape_reach(editor, &painter, &frame);
    let view = editor.views.area.view(view_key);
    if view.overlay_surfaces {
        overlay_surfaces(&painter, &frame, &area);
    }
    if view.walk_overlay {
        overlay_walk(&painter, &frame, &area);
    }
    if view.overlay_sort_lines {
        overlay_sort_lines(&painter, &frame, &area);
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

/// How far the backdrop's tiles sit from the blueprint's: where the area first places it, or else
/// the blueprint's middle over the area's.
fn backdrop_shift(snapshot: &AreaSnapshot, area: &str) -> Option<Offset<Meters>> {
    let Pictured::Blueprint(blueprint) = snapshot.pictured else {
        return None;
    };
    let content = &snapshot.content;
    let area_id = content.by_path::<AreaDef>(area)?;
    let placed = area::placements(content, area_id)
        .iter()
        .find(|placed| placed.blueprint == blueprint)
        .map(|placed| placed.at);
    let at = placed.unwrap_or_else(|| {
        let (size, own) = (area::load(content, area_id).size, snapshot.size());
        Pos::new(
            ((size.width - own.width) / 2.0).round(),
            ((size.height - own.height) / 2.0).round(),
        )
    });
    Some(Offset::new(at.x, at.y))
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
        area::walk::in_map(self.size().grid(), tile)
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
            node = nodes::node(&place.def.scene.nodes, &format!("/{pointer}"))?;
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

    /// The placement whose painted cells cover `cell` in the ground layer `layer`, the last in tree
    /// order since it wins the cell: the pointer of its node in the bundle's scene.
    pub fn covering(&self, cell: CellPos, layer: &str) -> Option<String> {
        self.picture
            .placements
            .iter()
            .rev()
            .find(|placement| {
                let def = placement.blueprint.get(&self.content);
                let mut cells = def.map.size.grid().cells().enumerate().map(|(index, at)| {
                    let square = Rect::new(Pos::new(at.x as f32, at.y as f32), Size::splat(1.0));
                    (
                        index,
                        placement.flip.place(square, placement.at).origin.cell(),
                    )
                });
                let Some((index, _)) = cells.find(|(_, placed)| *placed == cell) else {
                    return false;
                };
                let draft = grid::GridDraft::of(placement.blueprint.path(&self.content), def);
                draft
                    .layer_index(layer)
                    .and_then(|painted| draft.ground[painted].cells.get(index).copied().flatten())
                    .is_some()
            })
            .map(|placement| placement.origin.clone())
    }

    /// The locked module under `tile` a click passes through, for the tip that says what locks it.
    pub fn locked_hit(&self, tile: Pos<Meters>) -> Option<String> {
        self.targets
            .iter()
            .filter(|target| target.shown && target.locked && target.bounds.contains(tile))
            .min_by(|a, b| a.bounds.area().total_cmp(&b.bounds.area()))
            .map(|target| target.pointer.clone())
    }

    /// The names of the bundle's own ground layers left out of the editor's picture.
    pub fn hidden_ground(&self) -> Vec<String> {
        let mut hidden = Vec::new();
        nodes::walk(self.nodes(), &mut |pointer, node| {
            if let Some(name) = nodes::layer_name(node)
                && self.flags.hidden(pointer)
            {
                hidden.push(name.to_owned());
            }
        });
        hidden.sort();
        hidden
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
            scene: &place.def.scene.nodes,
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
    pub fn origin(&self) -> (Pos<Meters>, area::Flip) {
        match self.in_place.last() {
            Some(place) => (place.at, place.flip),
            None => (Pos::new(0.0, 0.0), area::Flip::None),
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
        .retain(|part| part == WALK_ROW || nodes::node(&scene, part).is_some());
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
        AreaAction::Split(node) => scenery::split_scenery(editor, frame, &node),
        AreaAction::ReturnDoor(node) => scenery::return_door(editor, frame, &node),
        AreaAction::AdoptBlocks(node) => scenery::adopt_blocks(editor, frame, &node),
    }
}
