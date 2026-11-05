pub mod blueprint;
pub mod brush;
pub mod camera;
pub mod climate;
pub mod clipboard;
pub mod contours;
pub mod flags;
pub mod inspect;
pub mod lighting;
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
pub mod problems;
pub mod render_sync;
pub mod resize;
pub mod scatter;
pub mod scene_tree;
pub mod select;
pub mod shapes;
pub mod soundscape;
pub mod stamp;
pub mod status;
pub mod terrain;
pub mod tools;
pub mod view_ground;
pub mod viewport;
pub mod warps;

pub use resize::ResizeDialog;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use ui::controls;
use ui::egui;
use ui::face;

use ui::theme;
use ui::words;

use bevy::math::Vec3Swizzles;
use bevy::prelude::{Ray3d, Transform, Vec3};
use engine::core::content::media::GltfInfos;
use engine::core::content::{Content, ContentFile, FolderReader, table_of};
use engine::core::math::{Pos, Rect, Size};
use engine::core::render::camera::View;
use engine::core::render::{facing_rotation, world_point};
use engine::core::units::{Degrees, MeterSize, Meters};
use engine::systems::actor::appearance::look;
use engine::systems::actor::held;
use engine::systems::actor::stance;
use engine::systems::actor::{Action, ActorModel, Motion};
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::prop::PropDef;
use engine::systems::area::scene::{ModuleNode, SceneNode};
use engine::systems::area::{self, AreaDef, AreaId, AreaModule, AreaModuleKind, Picture, Pictured};
use engine::systems::npc::NpcDef;
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, VariantArray};
use ui::egui::{Color32, Pos2, Rect as ERect, Sense, Ui, Vec2};
use ui::glyph::{self, Mark};

use self::brush::{Brush, SculptMode, Stroke};
use self::camera::{AreaCanvas, OrbitCamera, StandingBox};
use self::flags::{SceneFlags, SceneLevel};
use self::overlays::{
    AreaLabels, Footing, overlay_bounds, overlay_contours, overlay_footprints, overlay_modules,
    overlay_navmesh, overlay_soundscape_reach,
};
use self::status::status_bar;
use self::viewport::{Request, StageClip, StandingActor, StrokeTerrain, Viewports};
use crate::app::{BottomTab, Editor};
use crate::doc::{Derived, DocModule, NodeGrid, NodeId, SceneDoc};
use crate::keymap::EditorKey;
use crate::play;
use crate::project::{AreaCameraState, Project, RowSelection};
use crate::shown;
use crate::tabs::{EditorTab, file_title};
use crate::views::ViewMap;
use crate::widgets;

/// The key of the view of an area or blueprint bundle.
pub(crate) fn view_key(bundle: &str) -> String {
    format!("bundle:{bundle}")
}

/// The bundle a view key's view shows: an area, or a blueprint.
pub(crate) fn viewed_bundle(view_key: &str) -> Option<&str> {
    view_key.strip_prefix("bundle:")
}

fn camera_key(project: &Project, view_key: &str) -> String {
    format!("{}:{view_key}", project.root.display())
}

#[derive(Default)]
pub struct AreaViews {
    pub views: ViewMap<AreaViewState>,
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

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, VariantArray, AsRefStr, Deserialize, Serialize,
)]
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

/// The pages of an area's view, one for each part of the area it sets: its scene, the ground its
/// terrains lie on, the light it stands in, and the weather it turns through. A blueprint has
/// only a scene.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, VariantArray, AsRefStr, Deserialize, Serialize,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum AreaPage {
    #[default]
    Scene,
    Terrain,
    Lighting,
    Climate,
}

/// What the view can draw over its picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
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
    pub(crate) fn bottom_tab(self) -> Option<BottomTab> {
        match self {
            AreaTool::Place | AreaTool::Scatter => Some(BottomTab::Palette),
            _ => None,
        }
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
    pub page: AreaPage,
    pub climate: climate::ClimatePage,
    pub in_place: Vec<String>,
    pub camera: OrbitCamera,
    pub tool: AreaTool,
    pub overlays: HashSet<AreaOverlay>,
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
    /// The row of the scene tree the pointer is over.
    pub tree_hover: Option<String>,
    /// A row held hovered as if the pointer were over it, "" the whole scene.
    pub held_hover: Option<String>,
    /// A point of the ground the stage is held hovered over while the pointer is off it, as if
    /// the pointer rested there.
    pub held_ground: Option<Pos<Meters>>,
    outlines: overlays::Outlines,
    ground: view_ground::ViewGround,
    cache: Option<SnapshotCache>,
    preview_places: HashMap<String, PreviewPlaces>,
    cast: Option<Cast>,
    fitted: bool,
    /// How the camera looks at the middle once it is fitted, when asked for.
    looks_from: Option<CameraLook>,
}

/// How a camera looks at the middle of what it shows: from `distance` off, turned to `yaw` and
/// tilted to `pitch` where given, else as it frames the whole.
#[derive(Clone, Copy, Debug)]
pub struct CameraLook {
    pub distance: Meters,
    pub yaw: Option<Degrees>,
    pub pitch: Option<Degrees>,
}

impl AreaViewState {
    fn new(bundle: &str) -> AreaViewState {
        AreaViewState {
            bundle: bundle.to_owned(),
            overlays: HashSet::from([AreaOverlay::Modules]),
            ..AreaViewState::default()
        }
    }

    pub(crate) fn set_tool(&mut self, tool: AreaTool) {
        self.tool = tool;
        self.cancel_gesture();
    }

    /// Fits the camera anew, looking at the middle as `look` says.
    pub fn look_from(&mut self, look: CameraLook) {
        self.looks_from = Some(look);
        self.fitted = false;
    }

    /// The row of the scene tree hovered: the one under the pointer, else one held hovered.
    pub fn hovered_row(&self) -> Option<&str> {
        self.tree_hover.as_deref().or(self.held_hover.as_deref())
    }

    pub fn toggle(&mut self, overlay: AreaOverlay) {
        if !self.overlays.remove(&overlay) {
            self.overlays.insert(overlay);
        }
    }

    pub(crate) fn cancel_gesture(&mut self) -> bool {
        self.drag.take().is_some() | self.placing.take().is_some() | self.stroke.take().is_some()
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PopulationPreview {
    pub hidden: bool,
    pub seed: u64,
}

/// Where a population's preview stands its bodies, kept until the content, the population's count
/// or bounds, or the preview's seed change: placing them draws from the wild mesh, too slow to
/// repeat every frame.
struct PreviewPlaces {
    generation: u64,
    count: u32,
    roams: Option<Rect<Meters>>,
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
    pub clip: crate::clipboard::SceneClip,
    pub nodes: Vec<SceneNode>,
}

impl AreaViews {
    pub fn close(&mut self) {
        self.views.clear();
        self.active = None;
    }

    pub(crate) fn camera_state(&self) -> Option<AreaCameraState> {
        let bundle = self.active.as_ref()?;
        let view = self.views.get(&view_key(bundle))?;
        Some(AreaCameraState {
            bundle: bundle.clone(),
            camera: view.camera,
        })
    }

    pub(crate) fn restore_camera(&mut self, camera: &AreaCameraState) {
        if let Some(view) = self.views.get_mut(&view_key(&camera.bundle)) {
            view.camera = camera.camera;
        }
    }

    pub fn ensure_view(&mut self, bundle: &str) -> &mut AreaViewState {
        self.views
            .view_or_insert_with(&view_key(bundle), || AreaViewState::new(bundle))
    }

    pub fn view(&self, key: &str) -> &AreaViewState {
        self.views.view(key)
    }

    pub fn view_mut(&mut self, key: &str) -> &mut AreaViewState {
        self.views.view_mut(key)
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

/// The tab of an area or a blueprint: a bar of the area's pages over the page shown. The scene
/// page holds the scene graph on the left, the map with its tools, and the inspector; each other
/// page its part of the area on the left, and the area seen as that part leaves it.
pub fn show(editor: &mut Editor, ui: &mut Ui, bundle: &str) {
    let view_key = view_key(bundle);
    editor.views.area.ensure_view(bundle);
    let page = shown_page(editor, &view_key);
    match page {
        AreaPage::Scene => {
            ui::panel::left_framed(ui, "area", |ui| panel::show(editor, ui, &view_key));
            ui::panel::inspector(ui, editor, "area", |editor, ui| {
                inspect::sidebar(editor, ui, &view_key)
            });
        }
        AreaPage::Terrain => {
            ui::panel::left_scrolled(ui, "area terrain", |ui| {
                terrain::show(editor, ui, &view_key)
            });
        }
        AreaPage::Lighting => {
            ui::panel::left_scrolled(ui, "area lighting", |ui| {
                lighting::show(editor, ui, &view_key)
            });
        }
        AreaPage::Climate => {
            ui::panel::left_scrolled(ui, "area climate", |ui| {
                climate::show(editor, ui, &view_key)
            });
            ui::panel::inspector(ui, editor, "area climate", |editor, ui| {
                climate::inspector(editor, ui, &view_key)
            });
        }
    }
    show_canvas(editor, ui, &view_key, page);
    play::banner(editor, ui);
}

/// The page the view shows: the one picked, on an area; a blueprint has only its scene.
fn shown_page(editor: &Editor, view_key: &str) -> AreaPage {
    let view = editor.views.area.view(view_key);
    if table_of(&view.bundle) == Some(AreaDef::TABLE) {
        view.page
    } else {
        AreaPage::Scene
    }
}

/// The bar of an area's pages, one chip each.
fn page_bar(editor: &mut Editor, ui: &mut Ui, view_key: &str, page: AreaPage) {
    let picked = ui
        .horizontal(|ui| {
            AreaPage::VARIANTS
                .iter()
                .copied()
                .filter(|other| ui::chip::mode(ui, other.as_ref(), page == *other).clicked())
                .last()
        })
        .inner;
    if let Some(picked) = picked {
        let view = editor.views.area.view_mut(view_key);
        view.cancel_gesture();
        view.page = picked;
    }
}

pub fn bottom(editor: &mut Editor, ui: &mut Ui, view_key: &str, tab: BottomTab) {
    if tab == BottomTab::Listen {
        super::sound::panel(editor, ui);
        return;
    }
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

fn show_canvas(editor: &mut Editor, ui: &mut Ui, view_key: &str, page: AreaPage) {
    if table_of(&editor.views.area.view(view_key).bundle) == Some(AreaDef::TABLE) {
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| page_bar(editor, ui, view_key, page));
    }
    let Some(snapshot) = AreaSnapshot::for_view(editor, view_key) else {
        let bundle = editor.views.area.view(view_key).bundle.clone();
        if editor
            .project
            .as_ref()
            .is_some_and(|project| !nodes::read_yet(project, &bundle))
        {
            ui.centered_and_justified(|ui| ui.spinner());
            return;
        }
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
    fit_once(editor, view_key, &snapshot);
    let full = ui.available_rect_before_wrap();
    if page != AreaPage::Scene {
        let mut canvas_ui = ui.new_child(egui::UiBuilder::new().max_rect(full));
        canvas(editor, &mut canvas_ui, view_key, snapshot, page);
        return;
    }
    let rail_width = 44.0;
    let rail = ERect::from_min_size(full.min, Vec2::new(rail_width, full.height()));
    let canvas_rect = ERect::from_min_max(Pos2::new(rail.max.x, full.min.y), full.max);
    let mut canvas_ui = ui.new_child(egui::UiBuilder::new().max_rect(canvas_rect));
    canvas(editor, &mut canvas_ui, view_key, snapshot, page);
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
    let framing = OrbitCamera::framing(snapshot.size().bounds());
    view.camera = match view.looks_from.take() {
        Some(look) => OrbitCamera {
            distance: look.distance,
            yaw: look.yaw.map_or(framing.yaw, Degrees::radians),
            pitch: look.pitch.map_or(framing.pitch, Degrees::radians),
            ..framing
        },
        None => remembered.unwrap_or(framing),
    };
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
            .scene_node(&place_id(&snapshot.in_place))
            .and_then(|node| node.label().map(str::to_owned))
            .unwrap_or_else(|| file_title(&place.blueprint).to_owned());
        chip(&mut child, &mut |ui| {
            glyph::show(ui, Mark::Grid, 14.0, theme::WARN);
            ui.label(
                face::mono(format!(
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
                face::mono(format!("Showing only {name}"))
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
                face::mono(format!(
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

/// The map of the bundle under the view's camera: on the scene page worked on with the view's
/// tools; on another page only looked around.
fn canvas(
    editor: &mut Editor,
    ui: &mut Ui,
    view_key: &str,
    snapshot: AreaSnapshot,
    page: AreaPage,
) {
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::CANVAS);
    let bundle = snapshot.bundle.clone();
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
    let held = editor.views.area.view(view_key).held_ground;
    let hovered = response
        .hover_pos()
        .filter(|_| response.hovered())
        .or_else(|| held.map(|ground| canvas.to_screen(ground)));
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
    if shown::probed(ui.ctx()) {
        shown::stage(ui.ctx(), shown_stage(&frame, rect));
    }
    let overlays_began = Instant::now();
    overlay_bounds(&painter, &frame);
    if page != AreaPage::Scene {
        shown::overlays(ui.ctx(), overlays_began.elapsed());
        tools::navigate(editor, ui, view_key, &frame);
        status_bar(editor, &painter, view_key, &frame);
        request_viewport(editor, ui, view_key, &frame, rect);
        remember_camera(editor, view_key);
        return;
    }
    shown::overlays(ui.ctx(), overlays_began.elapsed());
    tools::input(editor, ui, &painter, view_key, &frame);
    locked_tip(editor, view_key, &frame);
    let overlays_began = Instant::now();
    if is_area {
        editor.views.area.active = Some(bundle.clone());
    }
    overlay_soundscape_reach(editor, &painter, &frame);
    let view = editor.views.area.view(view_key);
    let shows = |overlay| view.overlays.contains(&overlay);
    let (navmesh, footprints, contours, modules) = (
        shows(AreaOverlay::Navmesh),
        shows(AreaOverlay::Footprints),
        shows(AreaOverlay::Contours),
        shows(AreaOverlay::Modules),
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
    } else {
        editor.views.area.view_mut(view_key).outlines = overlays::Outlines::default();
    }
    if let Some(place) = frame.in_place.last() {
        placement::overlay_in_place(editor, &painter, &frame, place);
    }
    if is_area {
        pins::draw_arrivals(editor, &painter, &frame);
    }
    tools::overlay(editor, &painter, view_key, &frame);
    frame.labels.draw(&painter);
    shown::overlays(ui.ctx(), overlays_began.elapsed());
    status_bar(editor, &painter, view_key, &frame);
    if response.hovered() && !ui.ctx().text_edit_focused() {
        tools::keys(editor, ui, view_key, &frame);
    }
    request_viewport(editor, ui, view_key, &frame, rect);
    remember_camera(editor, view_key);
}

/// The stage as `frame` shows it in `rect`, with what it holds nearest the middle where each shows.
fn shown_stage(frame: &AreaFrame, rect: ERect) -> shown::ShownStage {
    let middle = rect.center();
    let mut marks: Vec<(String, egui::Pos2)> = frame
        .placed()
        .grid
        .within(frame.seen_ground())
        .filter(|(_, _, target)| target.selectable && target.shown)
        .filter_map(|(id, bounds, _)| {
            let (at, _) = frame.canvas.glance(*bounds)?;
            rect.contains(at)
                .then(|| {
                    frame
                        .doc
                        .pointer(*id)
                        .map(|pointer| (pointer.to_owned(), at))
                })
                .flatten()
        })
        .collect();
    marks.sort_by(|a, b| a.1.distance_sq(middle).total_cmp(&b.1.distance_sq(middle)));
    marks.truncate(STAGE_MARKS);
    shown::ShownStage {
        rect,
        view_proj: frame.canvas.view().view_proj(),
        camera: frame.canvas.camera,
        farthest: OrbitCamera::framing_distance(frame.canvas.area().size.bounds()).0,
        marks,
        picked: frame
            .response
            .hovered()
            .then(|| frame.hit_pointer())
            .flatten(),
    }
}

/// How many of what the stage holds the probe is told of, nearest the middle first.
const STAGE_MARKS: usize = 64;

/// Asks the viewport for the picture of the next frame: the scene as it stands, the terrain as a
/// stroke shapes it, the ghost under the pointer and the cast standing where they live.
fn request_viewport(editor: &mut Editor, ui: &Ui, view_key: &str, frame: &AreaFrame, rect: ERect) {
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
    let weather = match view.page {
        AreaPage::Climate => view.climate.weather(&frame.area().climate),
        _ => frame.area().climate.first(),
    };
    let lines = view.outlines.lines();
    let repaint = view.stroke.is_some() || ghost_moves(view);
    editor.views.area.viewports.request(Request {
        key: view_key.to_owned(),
        size,
        camera,
        picture: frame.picture.clone(),
        weather,
        content: frame.content.clone(),
        stroke,
        ghost,
        cast,
        lines,
    });
    if repaint {
        ui.ctx().request_repaint();
    }
}

fn ghost_moves(view: &AreaViewState) -> bool {
    view.tool == AreaTool::Place && view.prop().is_some()
}

/// The rigs standing in the view: each resident at its spot in its pose, and the previewed places
/// of each population, every one in its model's idle, those in sight. The rigs are made once for
/// what the view holds and the content, and the list handed on stays the same while the same rigs
/// are in sight.
fn cast(editor: &mut Editor, view_key: &str, frame: &AreaFrame) -> Arc<[StandingActor]> {
    let generation = editor
        .project
        .as_ref()
        .map_or(0, |project| project.generation);
    let seen = *frame.canvas.view();
    let view = editor.views.area.view_mut(view_key);
    let made_for = (frame.made, generation, view.population_previews.clone());
    if view
        .cast
        .as_ref()
        .is_none_or(|cast| cast.made_for != made_for)
    {
        let everyone = everyone_standing(view, frame, generation);
        view.cast = Some(Cast {
            made_for,
            everyone,
            in_sight: Vec::new(),
            shown: Arc::new([]),
        });
    }
    let cast = view.cast.as_mut().expect("the cast was just made");
    let in_sight: Vec<usize> = cast
        .everyone
        .iter()
        .enumerate()
        .filter(|(_, standing)| in_sight(&seen, standing.middle, standing.extent))
        .map(|(index, _)| index)
        .collect();
    if in_sight != cast.in_sight {
        cast.shown = in_sight
            .iter()
            .map(|&index| cast.everyone[index].actor.clone())
            .collect();
        cast.in_sight = in_sight;
    }
    cast.shown.clone()
}

/// The rigs of a view's cast, made for what the view holds, the content and its previews, and
/// those in sight last.
struct Cast {
    made_for: (u64, u64, HashMap<String, PopulationPreview>),
    everyone: Vec<Standing>,
    in_sight: Vec<usize>,
    shown: Arc<[StandingActor]>,
}

/// A rig of the cast, with the middle of its body and how far across it is.
struct Standing {
    actor: StandingActor,
    middle: Vec3,
    extent: Meters,
}

/// Every rig of the view's cast: each resident at its spot in its pose, and the previewed places of
/// each population.
fn everyone_standing(
    view: &mut AreaViewState,
    frame: &AreaFrame,
    generation: u64,
) -> Vec<Standing> {
    let content = &frame.content;
    let area = frame.area();
    let resolve = |npc: engine::systems::npc::NpcId| {
        frame
            .reader
            .path_of(npc)
            .and_then(|npc| content.by_path::<NpcDef>(npc))
    };
    let stand = |npc: engine::systems::npc::NpcId, at: Pos<Meters>, facing: Degrees, pose| {
        let def = npc.get(content);
        let model = ActorModel::get(content, def.appearance.model);
        let ground = area.heights.height_at(at);
        let body = model.hitbox();
        let hands = held::listed(content, model.hands().len(), &def.held).hands;
        let stance = stance::of_hands(content, &hands);
        let clip = StageClip::played(
            model.skeleton(),
            model.standing_clip(stance, &Motion::Action(Action::Idle), pose),
        );
        Standing {
            middle: world_point(at, ground + Meters(body.height.0 / 2.0)),
            extent: Meters(body.height.0.max(body.radius.0 * 2.0)),
            actor: StandingActor {
                body: model.dressed(content, &look(content, &def.appearance, &BTreeMap::new())),
                clip: Some(clip),
                transform: Transform {
                    translation: world_point(at, ground),
                    rotation: facing_rotation(facing.radians()),
                    ..Transform::default()
                },
                held: None,
                crossed: Vec::new(),
                props: super::stage::props(content, model, model.hands(), &hands),
                layers: None,
            },
        }
    };
    let mut everyone = Vec::new();
    for placed in &frame.picture.placed {
        if let AreaModule::Resident(resident) = &placed.module
            && let Some(npc) = resolve(resident.npc)
        {
            everyone.push(stand(npc, resident.at, resident.facing, resident.pose));
        }
    }
    for node in frame.doc.of_kind(AreaModuleKind::Population) {
        let AreaModule::Population(population) = node.module() else {
            continue;
        };
        if frame.flags.hidden(node.pointer) {
            continue;
        }
        let preview = view
            .population_previews
            .get(node.pointer)
            .copied()
            .unwrap_or_default();
        let Some(npc) = resolve(population.npc).filter(|_| !preview.hidden) else {
            continue;
        };
        let kept = view.preview_places.get(node.pointer).filter(|kept| {
            (kept.generation, kept.count, kept.roams, kept.seed)
                == (generation, population.count, population.roams, preview.seed)
        });
        let roams = population.roams.unwrap_or_else(|| area.size.bounds());
        let places = match kept {
            Some(kept) => kept.places.clone(),
            None if !area.holds(roams) => continue,
            None => {
                let places = population::population_places(area, population, preview.seed);
                view.preview_places.insert(
                    node.pointer.to_owned(),
                    PreviewPlaces {
                        generation,
                        count: population.count,
                        roams: population.roams,
                        seed: preview.seed,
                        places: places.clone(),
                    },
                );
                places
            }
        };
        for at in places {
            everyone.push(stand(
                npc,
                at,
                Degrees(((at.x * 37.0 + at.y * 91.0) % 360.0).abs()),
                engine::systems::actor::Pose::Idle,
            ));
        }
    }
    everyone
}

/// Whether a body `extent` across with its middle at `middle` shows in `view` tall enough to make
/// out as a figure: one smaller is left unstood, as the rigs a cast animates cost alike however
/// few pixels they cover.
fn in_sight(view: &View, middle: bevy::math::Vec3, extent: Meters) -> bool {
    let across = extent.0 * view.pixels_per_meter(Meters(middle.distance(view.eye())));
    let viewport = view.viewport();
    let reach = across / 2.0;
    across >= CAST_PIXELS
        && view.project(middle).is_some_and(|window| {
            (-reach..=viewport.x + reach).contains(&window.x)
                && (-reach..=viewport.y + reach).contains(&window.y)
        })
}

/// How many pixels tall the cast's figures must show to stand in the view.
const CAST_PIXELS: f32 = 16.0;

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
    pub content: Content,
    pub bundle: String,
    parts: SnapshotParts,
}

impl Deref for AreaSnapshot {
    type Target = SnapshotParts;

    fn deref(&self) -> &SnapshotParts {
        &self.parts
    }
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
pub struct SnapshotParts {
    /// Made anew only when what the snapshot holds is, so what is found from it may be kept
    /// until this changes.
    pub made: u64,
    pub pictured: Pictured,
    pub picture: Arc<Picture>,
    /// The bundle's scene as its files read now, its ids those of `reader`.
    pub doc: Arc<SceneDoc>,
    pub reader: Arc<FolderReader>,
    pub flags: Arc<SceneFlags>,
    pub in_place: Arc<[placement::AreaPlacement]>,
    pub placement_bounds: Arc<HashMap<String, Rect<Meters>>>,
    mapped: Arc<Mapped>,
}

/// What the map works out of the bundle's modules as it first asks: where each lies on the
/// ground, and every module that takes room filed by where it does, with how the pointer finds it.
struct Mapped {
    content: Content,
    footings: Derived<Option<Footing>>,
    placed: OnceLock<Placed>,
}

/// The modules that take room on the map, filed by the room they take, and the heights between
/// which their bodies stand.
struct Placed {
    grid: NodeGrid<MapTarget>,
    floor: f32,
    ceiling: f32,
}

/// How the pointer finds a module on the map.
struct MapTarget {
    /// Whether the pointer may pick it at all.
    selectable: bool,
    /// What it stands over its bounds, for a pick to catch it above the ground.
    body: Option<StandingBox>,
    shown: bool,
    locked: bool,
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
    /// change. Its picture is drawn again only when the content or what's hidden changes, and its
    /// scene read again only when its own file changes, so a change to the scene shows without
    /// waiting for the folder to load again.
    pub(crate) fn for_view(editor: &mut Editor, view_key: &str) -> Option<Self> {
        let project = editor.project.as_ref()?;
        let content = project.content.clone()?;
        let generation = project.generation;
        let revision = project.files.revision();
        let view = editor.views.area.views.get(view_key)?;
        let bundle = view.bundle.clone();
        if view.cache.is_none() && !nodes::read_yet(project, &bundle) {
            return None;
        }
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
                let made = SnapshotParts::read(
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
        let view = editor.views.area.view_mut(view_key);
        let camera = view.camera;
        view.ground.follow(&parts.picture.area, &camera);
        Some(Self::from_parts(content, bundle, parts))
    }

    /// The snapshot of `bundle`, through its view when one is open.
    pub fn for_bundle(editor: &mut Editor, bundle: &str) -> Option<Self> {
        let key = view_key(bundle);
        if editor.views.area.views.contains(&key) {
            return Self::for_view(editor, &key);
        }
        let content = editor.project.as_ref()?.content.clone()?;
        let parts = SnapshotParts::read(editor, &content, bundle, &[], None, None)?;
        Some(Self::from_parts(content, bundle.to_owned(), parts))
    }

    fn from_parts(content: Content, bundle: String, parts: SnapshotParts) -> Self {
        Self {
            content,
            bundle,
            parts,
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

    pub(crate) fn in_map(&self, ground: Pos<Meters>) -> bool {
        self.size().bounds().contains(ground)
    }

    /// The bundle's own scene, as its files read now.
    pub fn nodes(&self) -> &[SceneNode] {
        self.doc.nodes()
    }

    /// The node of the scene a tree row stands for, through the placements edited in place.
    pub(crate) fn scene_node(&self, id: &str) -> Option<&SceneNode> {
        let mut levels = id.split("/in/");
        let top = levels.next()?;
        let mut node = self.doc.at(top)?;
        for (pointer, place) in levels.zip(self.in_place.iter()) {
            node = nodes::node(&place.def.scene, &format!("/{pointer}"))?;
        }
        Some(node)
    }

    /// How many modules the folder of the tree row `id` holds, however deep.
    pub(crate) fn count_inside(&self, id: &str) -> usize {
        if !id.contains("/in/") {
            return self.doc.id(id).map_or(0, |id| self.doc.inside(id));
        }
        let mut count = 0;
        if let Some(node) = self.scene_node(id) {
            nodes::walk(nodes::children_of(node), &mut |_, child| {
                count += usize::from(!nodes::is_folder(child));
            });
        }
        count
    }

    pub(crate) fn module_node(&self, pointer: &str) -> Option<&ModuleNode> {
        self.doc.module_at(pointer).map(|module| module.node)
    }

    pub fn module(&self, pointer: &str) -> Option<&AreaModule> {
        self.module_node(pointer).map(|node| &node.module)
    }

    pub fn module_bounds(&self, pointer: &str) -> Option<Rect<Meters>> {
        self.bounds_of(self.doc.module_at(pointer)?)
    }

    /// Where `module`, one of the bundle's own, takes room on the map.
    pub(crate) fn bounds_of(&self, module: DocModule) -> Option<Rect<Meters>> {
        let placed = match module.module() {
            AreaModule::Blueprint(_) => self.placement_bounds.get(module.pointer).copied(),
            _ => None,
        };
        shapes::bounds(module.module(), placed)
    }

    /// Where `module`, one of the bundle's own, lies on the ground, when it takes room.
    pub(crate) fn footing(&self, module: DocModule) -> Option<&Footing> {
        let mapped = &self.mapped;
        mapped
            .footings
            .get(module.id, || {
                let bounds = self.bounds_of(module)?;
                Some(Footing::of(
                    &mapped.content,
                    &self.reader,
                    &self.picture.area,
                    module.module(),
                    bounds,
                ))
            })
            .and_then(Option::as_ref)
    }

    /// Whether the map draws `module`.
    pub(crate) fn shown(&self, module: DocModule) -> bool {
        !self.flags.hidden(module.pointer)
    }

    /// Every module of the bundle that takes room on the map within `reach`, in no order.
    pub(crate) fn placed_within(&self, reach: Rect<Meters>) -> impl Iterator<Item = DocModule<'_>> {
        self.placed()
            .grid
            .within(reach)
            .filter_map(|(id, _, _)| self.doc.doc_module(*id))
    }

    /// Whether the map lets the pointer at the module: not hidden, not locked, and not outside the
    /// placement edited in place.
    pub(crate) fn pickable(&self, pointer: &str) -> bool {
        !self.flags.hidden(pointer) && !self.flags.locked(pointer) && self.in_place.is_empty()
    }

    /// The module under `ground`, the smallest when several cover it.
    pub fn hit(&self, ground: Pos<Meters>) -> Option<String> {
        if !self.in_place.is_empty() {
            return None;
        }
        self.smallest_at(ground, false)
    }

    /// The module the pointer at `screen` picks: the nearest one whose standing body the pointer's
    /// ray passes through, else the smallest one covering the ground under it.
    fn hit_at(
        &self,
        canvas: &AreaCanvas,
        screen: Pos2,
        ground: Option<Pos<Meters>>,
    ) -> Option<String> {
        if !self.in_place.is_empty() {
            return None;
        }
        let standing = self.entered(canvas.ray(screen));
        standing.or_else(|| self.hit(ground?))
    }

    /// The nearest module the map shows and lets the pointer at whose standing body `ray` enters:
    /// looked for along where the ray passes over the ground between the highest any body stands
    /// and the lowest ground, a step at a time, nearest first.
    fn entered(&self, ray: Ray3d) -> Option<String> {
        let placed = self.placed();
        let bounds = self.size().bounds();
        let (from, to) = slab_span(
            ray,
            Vec3::new(bounds.min().x, placed.floor, bounds.min().y),
            Vec3::new(bounds.max().x, placed.ceiling, bounds.max().y),
        )?;
        let along = ray.direction.xz().length();
        let step = if along < 1e-3 {
            to - from
        } else {
            PICK_STEP.0 / along
        };
        let ground_at = |t: f32| {
            let at = ray.get_point(t);
            Pos::new(at.x, at.z)
        };
        let mut best: Option<(f32, NodeId)> = None;
        let mut start = from;
        while start <= to {
            let end = (start + step).min(to);
            let (a, b) = (ground_at(start), ground_at(end));
            let reach = Rect::from_points([a, b]);
            for (id, _, target) in placed.grid.within(reach) {
                if !target.selectable || !target.shown || target.locked {
                    continue;
                }
                let Some(entered) = target.body.and_then(|body| body.entered_by(ray)) else {
                    continue;
                };
                if best.is_none_or(|(nearest, _)| entered < nearest) {
                    best = Some((entered, *id));
                }
            }
            if best.is_some_and(|(nearest, _)| nearest <= end) || end >= to {
                break;
            }
            start = end;
        }
        best.and_then(|(_, id)| self.doc.pointer(id))
            .map(str::to_owned)
    }

    /// The smallest module the map shows under `ground`, among the locked ones or the others.
    fn smallest_at(&self, ground: Pos<Meters>, locked: bool) -> Option<String> {
        self.placed()
            .grid
            .at(ground)
            .filter(|(_, _, target)| target.selectable && target.shown && target.locked == locked)
            .min_by(|a, b| a.1.area().total_cmp(&b.1.area()))
            .and_then(|(id, _, _)| self.doc.pointer(*id))
            .map(str::to_owned)
    }

    /// The modules the selection `parts` takes on the map: those selected, and every visible,
    /// unlocked module inside a selected folder, in tree order.
    pub fn selected_modules(&self, parts: &HashSet<String>) -> Vec<String> {
        let mut taken: Vec<DocModule> = Vec::new();
        for part in parts {
            let Some(id) = self.doc.id(part) else {
                continue;
            };
            match self.doc.doc_module(id) {
                Some(module) => taken.push(module),
                None => taken.extend(
                    self.doc
                        .modules_under(id)
                        .filter(|module| self.pickable(module.pointer)),
                ),
            }
        }
        taken.sort_by_key(|module| module.id);
        taken.dedup_by_key(|module| module.id);
        taken
            .into_iter()
            .map(|module| module.pointer.to_owned())
            .collect()
    }

    /// The box around what the selected folders of `parts` hold on the map, with the label of the
    /// first, when a folder is selected.
    pub fn folder_box(&self, parts: &HashSet<String>) -> Option<(Rect<Meters>, String)> {
        let folders: Vec<NodeId> = parts
            .iter()
            .filter_map(|part| self.doc.id(part))
            .filter(|&id| self.doc.node(id).is_some_and(nodes::is_folder))
            .collect();
        let first = folders
            .iter()
            .min_by_key(|&&id| self.doc.pointer(id))
            .and_then(|&id| self.doc.node(id))?;
        let label = shapes::node_label(first, self.reader.as_ref());
        let bounds = folders
            .iter()
            .flat_map(|&folder| self.doc.modules_under(folder))
            .filter(|module| self.pickable(module.pointer) && is_selectable(module.module()))
            .filter_map(|module| self.bounds_of(module))
            .reduce(|a, b| a.union(&b))?;
        Some((bounds, label))
    }

    /// The names objects take in the bundle: its own, as the editor holds them, and those its
    /// placements bring.
    pub(crate) fn object_names(&self) -> HashSet<String> {
        let own = self
            .doc
            .of_kind(AreaModuleKind::Object)
            .filter_map(|module| match module.module() {
                AreaModule::Object(object) => object.name.clone(),
                _ => None,
            });
        own.chain(stamp::object_names(&self.picture.placed))
            .collect()
    }

    /// The locked module under `ground` a click passes through, for the tip that says what locks it.
    pub(crate) fn locked_hit_at(&self, ground: Pos<Meters>) -> Option<String> {
        self.smallest_at(ground, true)
    }

    fn placed(&self) -> &Placed {
        self.mapped.placed.get_or_init(|| {
            let area = &self.picture.area;
            let grid = NodeGrid::new(self.doc.modules().filter_map(|module| {
                let bounds = self.bounds_of(module)?;
                let height = standing_height(&self.mapped.content, &self.reader, module.module());
                Some((
                    module.id,
                    bounds,
                    MapTarget {
                        selectable: is_selectable(module.module()),
                        body: (height > 0.0).then(|| StandingBox::on(area, bounds, height)),
                        shown: !self.flags.hidden(module.pointer),
                        locked: self.flags.locked(module.pointer),
                    },
                ))
            }));
            let range = area.heights.range();
            let ceiling = grid
                .iter()
                .filter_map(|(_, _, target)| target.body.map(|body| body.top()))
                .fold(range.highest.0, f32::max);
            Placed {
                grid,
                floor: range.lowest.0,
                ceiling,
            }
        })
    }
}

/// How far along the ground a pick looks at a time.
const PICK_STEP: Meters = Meters(32.0);

/// Where along `ray` it is within the box from `min` to `max`: from where it enters, or its start
/// when it starts inside, to where it leaves.
fn slab_span(ray: Ray3d, min: Vec3, max: Vec3) -> Option<(f32, f32)> {
    let (mut enter, mut exit) = (0.0f32, f32::INFINITY);
    for axis in 0..3 {
        let (origin, direction) = (ray.origin[axis], ray.direction[axis]);
        if direction.abs() < 1e-6 {
            if origin < min[axis] || origin > max[axis] {
                return None;
            }
            continue;
        }
        let t1 = (min[axis] - origin) / direction;
        let t2 = (max[axis] - origin) / direction;
        enter = enter.max(t1.min(t2));
        exit = exit.min(t1.max(t2));
    }
    (enter <= exit).then_some((enter, exit))
}

impl SnapshotParts {
    /// The snapshot of `bundle` as its files read now. Of `drawn`, made from the same content with
    /// the same rows asked to show alone, it keeps the scene when its file reads alike, the
    /// picture when the same rows are hidden, and what the map worked out when it keeps both.
    fn read(
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
        let doc = match drawn.filter(|drawn| drawn.doc.bundle().same(&def)) {
            Some(drawn) => drawn.doc.clone(),
            None => Arc::new(SceneDoc::of(def)),
        };
        let in_place: Arc<[placement::AreaPlacement]> =
            placement::resolve_in_place(editor, doc.nodes(), in_place)
                .unwrap_or_default()
                .into();
        let mut levels = vec![SceneLevel {
            under: String::new(),
            scene: doc.nodes(),
        }];
        for (depth, place) in in_place.iter().enumerate() {
            levels.push(SceneLevel {
                under: place_id(&in_place[..=depth]),
                scene: &place.def.scene,
            });
        }
        let flags = SceneFlags::of(&levels, show_only);
        let drawn_alike = drawn.filter(|drawn| drawn.flags.hides_alike(&flags));
        let (picture, placement_bounds) = match drawn_alike {
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
        let mapped = match drawn_alike
            .filter(|drawn| Arc::ptr_eq(&drawn.doc, &doc) && Arc::ptr_eq(&drawn.picture, &picture))
        {
            Some(drawn) => drawn.mapped.clone(),
            None => Arc::new(Mapped {
                content: content.clone(),
                footings: Derived::of(&doc),
                placed: OnceLock::new(),
            }),
        };
        static MADE: AtomicU64 = AtomicU64::new(0);
        Some(SnapshotParts {
            made: MADE.fetch_add(1, Ordering::Relaxed),
            pictured,
            picture,
            doc,
            reader,
            flags: Arc::new(flags),
            in_place,
            placement_bounds,
            mapped,
        })
    }
}

/// The tree row of the placement edited in place at the end of `chain`.
pub(super) fn place_id(chain: &[placement::AreaPlacement]) -> String {
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
    /// The ground the canvas may show, as far as the map goes: what lies between the eye and where
    /// the rays through the canvas's corners come down to the lowest ground, or the whole map when
    /// one of them never does.
    pub(crate) fn seen_ground(&self) -> Rect<Meters> {
        let map = self.size().bounds();
        let floor = self.area().heights.range().lowest.0;
        let screen = self.canvas.screen;
        let eye = self.canvas.view().eye();
        let mut seen = vec![Pos::new(eye.x, eye.z)];
        for corner in [
            screen.left_top(),
            screen.right_top(),
            screen.left_bottom(),
            screen.right_bottom(),
        ] {
            let ray = self.canvas.ray(corner);
            let along = (floor - ray.origin.y) / ray.direction.y;
            if ray.direction.y > -1e-4 || along < 0.0 {
                return map;
            }
            let down = ray.get_point(along);
            seen.push(Pos::new(down.x, down.z));
        }
        Rect::from_points(seen)
            .intersection(&map)
            .unwrap_or_else(|| Rect::new(map.min(), Size::new(0.0, 0.0)))
    }

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
    pub(crate) fn drag_ground(&self) -> Option<Pos<Meters>> {
        self.pointer_ground.or_else(|| {
            self.response
                .interact_pointer_pos()
                .and_then(|p| self.canvas.to_ground(p))
        })
    }

    /// The module the pointer picks: by its standing body, else by the ground under it.
    pub(crate) fn hit_pointer(&self) -> Option<String> {
        let screen = self.pointer?;
        self.snapshot
            .hit_at(&self.canvas, screen, self.pointer_ground)
    }

    /// The locked module a click under the pointer passes through.
    fn locked_hit(&self) -> Option<String> {
        self.snapshot.locked_hit_at(self.pointer_ground?)
    }
}

fn is_selectable(module: &AreaModule) -> bool {
    !matches!(module, AreaModule::Population(population) if population.roams.is_none())
}

pub(crate) fn selected_parts(editor: &Editor, bundle: &str) -> HashSet<String> {
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
    let key = camera_key(project, view_key);
    let camera = editor.views.area.view(view_key).camera;
    editor.settings.cameras.insert(key, camera);
}

pub(crate) fn open_area_at(
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
    let read = nodes::bundle_of(editor, &bundle);
    let before = editor.selection.parts.len();
    editor.selection.parts.retain(|part| {
        read.as_ref()
            .is_some_and(|read| nodes::node(read.nodes(), part).is_some())
    });
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
        AreaAction::ReturnDoor(node) => warps::return_door(editor, frame, &node),
    }
}
