pub mod blueprint;
pub mod brush;
pub mod camera;
pub mod contours;
pub mod flags;
pub mod inspect;
pub mod modules;
pub mod nodes;
pub mod overlays;
pub mod palette;
pub mod panel;
pub mod pick;
pub mod pins;
pub mod place;
pub mod placement;
pub mod population;
pub mod preview;
pub mod problems;
pub mod resize;
pub mod scatter;
pub mod scene_tree;
pub mod select;
pub mod shapes;
pub mod soundscape;
pub mod stamp;
pub mod status;
pub mod tools;
pub mod viewport;

pub use resize::ResizeDialog;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Deref;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use ui::controls;
use ui::egui;

use ui::theme;
use ui::words;

use bevy::prelude::Transform;
use engine::core::content::media::GltfInfos;
use engine::core::content::{Content, ContentFile, FolderReader, table_of};
use engine::core::geometry::Degrees;
use engine::core::math::{Pos, Rect, Size};
use engine::core::render::{facing_rotation, world_point};
use engine::core::units::{MeterSize, Meters};
use engine::systems::actor::appearance::look;
use engine::systems::actor::held;
use engine::systems::actor::stance;
use engine::systems::actor::{Action, ActorModel, Motion};
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::prop::PropDef;
use engine::systems::area::scene::{ModuleNode, SceneNode};
use engine::systems::area::{self, AreaDef, AreaId, AreaModule, AreaModuleKind, Picture, Pictured};
use engine::systems::npc::NpcDef;
use serde::Deserialize;
use strum::{AsRefStr, VariantArray};
use ui::egui::{Color32, Pos2, Rect as ERect, Sense, Ui, Vec2};
use ui::glyph::{self, Mark};

use self::brush::{Brush, SculptMode, Stroke};
use self::camera::{AreaCanvas, OrbitCamera};
use self::flags::{SceneFlags, SceneLevel};
use self::overlays::{
    AreaLabels, overlay_bounds, overlay_contours, overlay_footprints, overlay_modules,
    overlay_navmesh, overlay_soundscape_reach,
};
use self::status::status_bar;
use self::viewport::{Request, StageClip, Standing, StrokeTerrain, Viewports};
use crate::app::{BottomTab, Editor};
use crate::keymap::EditorKey;
use crate::play;
use crate::project::{AreaCameraState, Project, RowSelection};
use crate::shell::{sidebar, widgets};
use crate::tabs::{EditorTab, file_title};

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
    pub views: HashMap<String, AreaViewState>,
    pub viewports: Viewports,
    pub hover: Option<AreaHover>,
    pub active: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AreaHover {
    pub area: String,
    pub ground: Pos<Meters>,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, VariantArray, AsRefStr, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AreaTool {
    #[default]
    Select,
    Sculpt,
    Paint,
    Place,
    Scatter,
    Module,
}

/// What the view can draw over its picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AreaOverlay {
    Navmesh,
    Contours,
    Footprints,
    Modules,
}

impl AreaTool {
    pub fn key(self) -> EditorKey {
        match self {
            AreaTool::Select => EditorKey::SelectTool,
            AreaTool::Sculpt => EditorKey::SculptTool,
            AreaTool::Paint => EditorKey::PaintTool,
            AreaTool::Place => EditorKey::PlaceTool,
            AreaTool::Scatter => EditorKey::ScatterTool,
            AreaTool::Module => EditorKey::ModuleTool,
        }
    }

    /// The bottom tab a tool works from, brought up when the tool is taken.
    pub fn bottom_tab(self) -> Option<BottomTab> {
        match self {
            AreaTool::Place | AreaTool::Scatter => Some(BottomTab::Palette),
            _ => None,
        }
    }

    /// Whether the tool shapes the terrain, which only an area has.
    pub fn shapes_terrain(self) -> bool {
        matches!(self, AreaTool::Sculpt | AreaTool::Paint)
    }
}

/// What the Place tool puts down at a click: a prop from the palette, or a blueprint from the
/// Blueprints tab, whichever was picked last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placeable {
    Prop(String),
    Blueprint(String),
}

#[derive(Default)]
pub struct AreaViewState {
    /// The area or blueprint bundle the view edits.
    pub bundle: String,
    pub in_place: Vec<String>,
    pub camera: OrbitCamera,
    pub tool: AreaTool,
    pub overlay_navmesh: bool,
    pub overlay_contours: bool,
    pub overlay_footprints: bool,
    pub overlay_modules: bool,
    pub drag: Option<AreaDrag>,
    pub brush: Brush,
    pub sculpt: SculptMode,
    /// The height a level stroke pulls the ground toward.
    pub level: Meters,
    /// The terrain file the paint brush lays, once one is picked.
    pub paint_terrain: Option<String>,
    pub stroke: Option<Stroke>,
    pub placeable: Option<Placeable>,
    /// How a prop placed next is turned.
    pub place_yaw: Degrees,
    pub listener: Option<Pos<Meters>>,
    pub paste: Option<PasteGhost>,
    pub blueprint_search: String,
    pub palette_search: String,
    pub menu: Option<modules::AreaMenuAnchor>,
    pub population_previews: HashMap<String, PopulationPreview>,
    pub skipped: Option<String>,
    /// The row shown alone for now, every other row's own flags kept as they are.
    pub show_only: Option<String>,
    /// The kind of module the Module tool places at the next click, into the insertion folder.
    pub placing: Option<AreaModuleKind>,
    pub scene: panel::ScenePanel,
    pub contours: contours::ContourCache,
    cache: Option<SnapshotCache>,
    preview_places: HashMap<String, PreviewPlaces>,
    fitted: bool,
}

impl AreaViewState {
    fn new(bundle: &str) -> AreaViewState {
        AreaViewState {
            bundle: bundle.to_owned(),
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

    pub fn toggle(&mut self, overlay: AreaOverlay) {
        let flag = match overlay {
            AreaOverlay::Navmesh => &mut self.overlay_navmesh,
            AreaOverlay::Contours => &mut self.overlay_contours,
            AreaOverlay::Footprints => &mut self.overlay_footprints,
            AreaOverlay::Modules => &mut self.overlay_modules,
        };
        *flag = !*flag;
    }

    pub fn cancel_gesture(&mut self) -> bool {
        self.drag.take().is_some() | self.placing.take().is_some() | self.stroke.take().is_some()
    }

    pub fn take_drag(&mut self) -> Option<AreaDrag> {
        self.drag.take()
    }

    /// The blueprint the Place tool stamps, when one is picked.
    pub fn stamp(&self) -> Option<&str> {
        match &self.placeable {
            Some(Placeable::Blueprint(blueprint)) => Some(blueprint),
            _ => None,
        }
    }

    /// The prop the Place and Scatter tools put down, when one is picked.
    pub fn prop(&self) -> Option<&str> {
        match &self.placeable {
            Some(Placeable::Prop(prop)) => Some(prop),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PopulationPreview {
    pub hidden: bool,
    pub seed: u64,
}

/// Where a population's preview stands its bodies, kept until the files or the preview's seed
/// change: placing them draws from the whole wild mesh, too slow to repeat every frame.
struct PreviewPlaces {
    generation: u64,
    revision: u64,
    seed: u64,
    places: Vec<Pos<Meters>>,
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
    Scatter {
        from: Pos<Meters>,
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
            camera: view.camera,
        })
    }

    pub fn restore_camera(&mut self, camera: &AreaCameraState) {
        if let Some(view) = self.views.get_mut(&view_key(&camera.bundle)) {
            view.camera = camera.camera;
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

    /// Points what the views place at where `moved` took it.
    pub fn moved(&mut self, moved: &dyn Fn(&str) -> Option<String>) {
        for view in self.views.values_mut() {
            view.placeable = match view.placeable.take() {
                Some(Placeable::Prop(prop)) => Some(Placeable::Prop(moved(&prop).unwrap_or(prop))),
                Some(Placeable::Blueprint(blueprint)) => {
                    Some(Placeable::Blueprint(moved(&blueprint).unwrap_or(blueprint)))
                }
                None => None,
            };
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
    let Some(snapshot) = AreaSnapshot::for_view(editor, view_key) else {
        return;
    };
    match tab {
        BottomTab::Blueprints => stamp::stamp_panel(editor, ui, view_key, &snapshot),
        BottomTab::Palette => palette::show(editor, ui, view_key, &snapshot),
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
    fit_once(editor, view_key, &snapshot);
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

fn fit_once(editor: &mut Editor, view_key: &str, snapshot: &AreaSnapshot) {
    let Some(project) = &editor.project else {
        return;
    };
    let remembered = editor
        .settings
        .cameras
        .get(&camera_key(project, view_key))
        .copied();
    let view = editor.views.area.view_mut(view_key);
    if view.fitted {
        return;
    }
    view.camera = remembered.unwrap_or_else(|| OrbitCamera::framing(snapshot.size().bounds()));
    view.fitted = true;
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
    let shown = editor.views.area.viewports.shown(view_key);
    let camera = shown.map_or_else(
        || editor.views.area.view(view_key).camera,
        |shown| shown.camera,
    );
    if let Some(shown) = shown {
        painter.image(shown.texture, rect, crate::thumbs::FULL_UV, Color32::WHITE);
        let grade = snapshot.area().lighting.grade;
        if grade.strength > 0.0 {
            let [r, g, b] = grade.color.0;
            painter.rect_filled(
                rect,
                0.0,
                Color32::from_rgba_unmultiplied(r, g, b, (grade.strength * 255.0) as u8),
            );
        }
    } else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "standing the scenery…",
            egui::FontId::proportional(13.0),
            theme::MUTED,
        );
        ui.ctx().request_repaint();
    }
    let canvas = AreaCanvas::new(rect, camera, snapshot.picture.area.clone());
    let hovered = response.hover_pos().filter(|_| response.hovered());
    let pointer_ground = hovered.and_then(|p| canvas.to_ground(p));
    if let Some(ground) = pointer_ground {
        editor.views.area.hover = Some(AreaHover {
            area: bundle.clone(),
            ground,
        });
    }
    let is_area = matches!(snapshot.pictured, Pictured::Area(_));
    let frame = AreaFrame {
        snapshot,
        canvas,
        pointer: hovered,
        pointer_ground,
        response: &response,
        labels: AreaLabels::default(),
    };
    overlay_bounds(&painter, &frame);
    tools::input(editor, ui, &painter, view_key, &frame);
    locked_tip(editor, view_key, &frame);
    if is_area {
        editor.views.area.active = Some(bundle.clone());
    }
    overlay_soundscape_reach(editor, &painter, &frame);
    let view = editor.views.area.view(view_key);
    let (navmesh, footprints, contours, modules) = (
        view.overlay_navmesh,
        view.overlay_footprints,
        view.overlay_contours,
        view.overlay_modules,
    );
    if navmesh {
        overlay_navmesh(&painter, &frame);
    }
    if footprints {
        overlay_footprints(&painter, &frame);
    }
    if contours {
        overlay_contours(editor, &painter, view_key, &frame);
    }
    if modules || !editor.selection.parts.is_empty() {
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
    request_viewport(editor, ui, view_key, &frame, rect, generation);
    remember_camera(editor, view_key);
}

/// Asks the viewport for the picture of the next frame: the scene as it stands, the terrain as a
/// stroke shapes it, the ghost under the pointer and the cast standing where they live.
fn request_viewport(
    editor: &mut Editor,
    ui: &Ui,
    view_key: &str,
    frame: &AreaFrame,
    rect: ERect,
    generation: u64,
) {
    let pixels_per_point = ui.ctx().pixels_per_point();
    let size = [
        (rect.width() * pixels_per_point).round().max(1.0) as u32,
        (rect.height() * pixels_per_point).round().max(1.0) as u32,
    ];
    let cast = cast(editor, view_key, frame);
    let ghost = place::ghost(editor, view_key, frame);
    let view = editor.views.area.view(view_key);
    let stroke = view.stroke.as_ref().map(|stroke| StrokeTerrain {
        revision: stroke.revision,
        heights: stroke.heights.clone(),
        paint: stroke.paint.clone(),
    });
    let camera = view.camera;
    let repaint = view.stroke.is_some() || ghost_moves(view);
    editor.views.area.viewports.request(Request {
        key: view_key.to_owned(),
        size,
        camera,
        picture: frame.picture.clone(),
        made: frame.made,
        content: frame.content.clone(),
        generation,
        stroke,
        ghost,
        cast,
    });
    if repaint {
        ui.ctx().request_repaint();
    }
}

fn ghost_moves(view: &AreaViewState) -> bool {
    view.tool == AreaTool::Place && view.prop().is_some()
}

/// The rigs standing in the view: each resident at its spot in its pose, and the previewed places
/// of each population, every one in its model's idle.
fn cast(editor: &mut Editor, view_key: &str, frame: &AreaFrame) -> Vec<Standing> {
    let content = &frame.content;
    let area = frame.area();
    let (generation, revision) = editor.project.as_ref().map_or((0, 0), |project| {
        (project.generation, project.files.revision())
    });
    let view = editor.views.area.view_mut(view_key);
    let previews = view.population_previews.clone();
    let stand = |npc: engine::systems::npc::NpcId, at: Pos<Meters>, facing: Degrees, pose| {
        let npc = frame.reader.path_of(npc)?;
        let npc = content.by_path::<NpcDef>(npc)?;
        let def = npc.get(content);
        let model = ActorModel::get(content, def.appearance.model);
        let hands = held::listed(content, model.hands().len(), &def.held);
        let stance = stance::of_hands(content, &hands);
        let clip = StageClip::played(
            model.skeleton(),
            model.standing_clip(stance, &Motion::Action(Action::Idle), pose),
        );
        let ground = area.heights.height_at(at);
        Some(Standing {
            body: model.dressed(content, &look(content, &def.appearance, &BTreeMap::new())),
            clip: Some(clip),
            transform: Transform {
                translation: world_point(at, ground),
                rotation: facing_rotation(facing.radians()),
                ..Transform::default()
            },
            held: None,
            crossed: Vec::new(),
            props: super::stage::props(content, model, &hands),
            layers: None,
        })
    };
    let mut cast = Vec::new();
    for placed in &frame.picture.placed {
        if let AreaModule::Resident(resident) = &placed.module {
            cast.extend(stand(
                resident.npc,
                resident.at,
                resident.facing,
                resident.pose,
            ));
        }
    }
    for node in frame.modules.iter() {
        let AreaModule::Population(population) = node.module() else {
            continue;
        };
        if frame.flags.hidden(&node.pointer) {
            continue;
        }
        let preview = previews.get(&node.pointer).copied().unwrap_or_default();
        if preview.hidden {
            continue;
        }
        let kept = view.preview_places.get(&node.pointer).filter(|kept| {
            (kept.generation, kept.revision, kept.seed) == (generation, revision, preview.seed)
        });
        let places = match kept {
            Some(kept) => kept.places.clone(),
            None => {
                let places = population::population_places(area, population, preview.seed);
                view.preview_places.insert(
                    node.pointer.clone(),
                    PreviewPlaces {
                        generation,
                        revision,
                        seed: preview.seed,
                        places: places.clone(),
                    },
                );
                places
            }
        };
        for at in places {
            cast.extend(stand(
                population.npc,
                at,
                Degrees(((at.x * 37.0 + at.y * 91.0) % 360.0).abs()),
                engine::systems::actor::Pose::Idle,
            ));
        }
    }
    cast
}

/// Says what locks the module under the pointer, since a click passes through it.
fn locked_tip(editor: &Editor, view_key: &str, frame: &AreaFrame) {
    if editor.views.area.view(view_key).tool != AreaTool::Select {
        return;
    }
    let Some(locked) = frame.locked_hit() else {
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
    /// How tall the module stands over its bounds, for a pick to catch it above the ground.
    height: f32,
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
    pub canvas: AreaCanvas,
    pub pointer: Option<Pos2>,
    pub pointer_ground: Option<Pos<Meters>>,
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
            Pictured::Blueprint(_) | Pictured::Prop(_) | Pictured::Stage => None,
        }
    }

    pub fn size(&self) -> Size<Meters> {
        self.area().size
    }

    pub fn in_map(&self, ground: Pos<Meters>) -> bool {
        self.size().bounds().contains(ground)
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

    /// The module under `ground`, the smallest when several cover it.
    pub fn hit(&self, ground: Pos<Meters>) -> Option<String> {
        if !self.in_place.is_empty() {
            return None;
        }
        self.targets
            .iter()
            .filter(|target| target.shown && !target.locked && target.bounds.contains(ground))
            .min_by(|a, b| a.bounds.area().total_cmp(&b.bounds.area()))
            .map(|target| target.pointer.clone())
    }

    /// The module the pointer at `screen` picks: the nearest one whose standing body the pointer's
    /// ray passes through, else the smallest one covering the ground under it.
    pub fn hit_at(
        &self,
        canvas: &AreaCanvas,
        screen: Pos2,
        ground: Option<Pos<Meters>>,
    ) -> Option<String> {
        if !self.in_place.is_empty() {
            return None;
        }
        let standing = self
            .targets
            .iter()
            .filter(|target| target.shown && !target.locked && target.height > 0.0)
            .filter_map(|target| {
                canvas
                    .hits_box(screen, target.bounds, target.height)
                    .map(|along| (along, target))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, target)| target.pointer.clone());
        standing.or_else(|| self.hit(ground?))
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

    /// The locked module under `ground` a click passes through, for the tip that says what locks it.
    pub fn locked_hit_at(&self, ground: Pos<Meters>) -> Option<String> {
        self.targets
            .iter()
            .filter(|target| target.shown && target.locked && target.bounds.contains(ground))
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
                height: standing_height(content, &reader, module.module()),
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

/// How tall a module stands over the ground, for picking it by its body: a prop by its model, a
/// resident by its npc's hitbox, an object by the prop it looks like; nothing for the flat ones.
fn standing_height(content: &Content, reader: &FolderReader, module: &AreaModule) -> f32 {
    let model_height = |prop: &str| {
        let prop = content.by_path::<PropDef>(prop)?;
        let bounds = content
            .built::<GltfInfos>()
            .0
            .get(&prop.get(content).model.path())?
            .bounds;
        Some(bounds.max.y - bounds.min.y)
    };
    match module {
        AreaModule::Prop(prop) => reader
            .path_of(prop.prop)
            .and_then(model_height)
            .map_or(0.0, |height| height * prop.scale),
        AreaModule::Object(object) => object
            .look
            .and_then(|look| reader.path_of(look))
            .and_then(model_height)
            .unwrap_or(0.0),
        AreaModule::Resident(resident) => reader
            .path_of(resident.npc)
            .and_then(|npc| content.by_path::<NpcDef>(npc))
            .map_or(0.0, |npc| {
                ActorModel::get(content, npc.get(content).appearance.model)
                    .hitbox()
                    .height
                    .0
            }),
        _ => 0.0,
    }
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

    /// The ground under the pointer, or under where a drag began when the pointer has left the
    /// ground.
    pub fn drag_ground(&self) -> Option<Pos<Meters>> {
        self.pointer_ground.or_else(|| {
            self.response
                .interact_pointer_pos()
                .and_then(|p| self.canvas.to_ground(p))
        })
    }

    /// The module the pointer picks: by its standing body, else by the ground under it.
    pub fn hit_pointer(&self) -> Option<String> {
        let screen = self.pointer?;
        self.snapshot
            .hit_at(&self.canvas, screen, self.pointer_ground)
    }

    /// The locked module a click under the pointer passes through.
    pub fn locked_hit(&self) -> Option<String> {
        self.snapshot.locked_hit_at(self.pointer_ground?)
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
    let camera = view.camera;
    let changed = editor
        .settings
        .cameras
        .get(&key)
        .is_none_or(|known| !known.near(&camera));
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
        view.camera.target = at;
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
