//! What the view draws over the viewport's picture: the area's edge, its modules as outlines on
//! the ground, the navmesh, the height contours and every prop's footprint, each bent over the
//! terrain the way the picture shows it: the modules shaped as rectangles as lines the viewport
//! draws in its picture, the rest over it. Scatters and blueprint placements, which can tile a
//! whole area, are outlined only when selected or hovered, and a module is named only when
//! hovered, on the map or in the scene tree.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use ui::egui;
use ui::ink;
use ui::theme;

use bevy::math::Vec3;
use engine::core::content::{Content, FolderReader};
use engine::core::math::{Pos, Rect};
use engine::core::render::world_point;
use engine::core::units::{MeterPos, MeterSize, Meters};
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::prop::{Footprint, PropDef, PropId};
use engine::systems::area::{Area, AreaModule, AreaModuleKind, Pictured};
use rayon::prelude::*;
use ui::egui::{Color32, Pos2, Rect as ERect, Stroke as EStroke, Vec2};

use super::camera::AreaCanvas;
use super::{AreaFrame, AreaSnapshot, AreaTool, selected_parts, shapes};
use crate::app::Editor;
use crate::doc::DocModule;
use crate::forms;

const LABEL_GAP: f32 = 2.0;
const LABEL_SHIFTS: u32 = 3;
/// The side of the cells of the screen placed labels are filed by.
const LABEL_CELL: f32 = 64.0;
/// The size of a label's text, monospaced, so a label's size is known before it is laid out.
const LABEL_TEXT: f32 = 11.5;
/// The room the shortest label takes on the screen.
const LABEL_SPOT: Vec2 = Vec2::new(48.0, 20.0);

/// The edge of the area's ground, or a blueprint's canvas with its size.
pub(super) fn overlay_bounds(painter: &egui::Painter, frame: &AreaFrame) {
    let canvas = &frame.canvas;
    let area = frame.area();
    let bounds = area.size.bounds();
    if matches!(frame.pictured, Pictured::Blueprint(_)) {
        painter.add(egui::Shape::dashed_line(
            &canvas.outline(bounds),
            EStroke::new(1.5, theme::DROP),
            6.0,
            4.0,
        ));
        frame.labels.chip(
            canvas.rect(bounds).right_top() + Vec2::new(6.0, 18.0),
            format!("{} × {} canvas", area.size.width, area.size.height),
            theme::DROP,
        );
    } else {
        painter.add(egui::Shape::closed_line(
            canvas.outline(bounds),
            EStroke::new(1.0, Color32::from_gray(120)),
        ));
    }
}

/// The navmesh's polygons and the safe zones.
pub(super) fn overlay_navmesh(painter: &egui::Painter, frame: &AreaFrame) {
    let canvas = &frame.canvas;
    let area = frame.area();
    let outline = EStroke::new(1.0, ink::wash(theme::BLOCKED, 0.43));
    for polygon in area.mesh().polygons() {
        if !canvas.sees(Rect::from_points(polygon.iter().copied())) {
            continue;
        }
        painter.add(egui::Shape::closed_line(canvas.closed(polygon), outline));
    }
    for zone in &area.safe_zones {
        let corners = engine::core::geometry::corners(*zone).map(|corner| canvas.to_screen(corner));
        painter.add(egui::Shape::convex_polygon(
            corners.to_vec(),
            Color32::from_rgba_unmultiplied(80, 120, 220, 40),
            EStroke::NONE,
        ));
    }
}

/// Lines where the ground crosses each whole metre of height, every fifth one stronger.
pub(super) fn overlay_contours(
    editor: &mut Editor,
    painter: &egui::Painter,
    view_key: &str,
    frame: &AreaFrame,
) {
    let canvas = &frame.canvas;
    let contours = editor
        .views
        .area
        .view_mut(view_key)
        .contours
        .of(&frame.picture.area);
    for contour in contours.iter() {
        let major = (contour.height.0 / 5.0).fract().abs() < 0.01;
        let color = if major {
            Color32::from_rgba_unmultiplied(255, 255, 255, 150)
        } else {
            Color32::from_rgba_unmultiplied(255, 255, 255, 70)
        };
        for [from, to] in &contour.segments {
            if !canvas.sees(Rect::from_points([*from, *to])) {
                continue;
            }
            painter.line_segment(
                [canvas.to_screen(*from), canvas.to_screen(*to)],
                EStroke::new(1.0, color),
            );
        }
        if major && let Some([from, _]) = contour.segments.first() {
            frame.labels.chip(
                canvas.to_screen(*from),
                format!("{:+} m", contour.height.0),
                Color32::from_gray(200),
            );
        }
    }
}

/// Every prop the area stands, scattered ones too, as its footprint and the outline the navmesh
/// keeps clear of it.
pub(super) fn overlay_footprints(painter: &egui::Painter, frame: &AreaFrame) {
    let color = ink::wash(module_color(AreaModuleKind::Prop), 0.6);
    let mut dots = shapes::Dots::default();
    for prop in &frame.area().props {
        let def = prop.prop.get(&frame.content);
        let ground = def
            .footprint
            .bounds(prop.at, prop.yaw, prop.scale)
            .unwrap_or_else(|| shapes::cell_rect(prop.at));
        let Some((at, across)) = frame.canvas.glance(ground) else {
            continue;
        };
        if !frame.canvas.shows(at, across) {
            continue;
        }
        if !shapes::shows_shape(across) {
            dots.add(painter, at, color);
            continue;
        }
        shapes::draw_footprint(
            painter,
            &frame.canvas,
            &def.footprint,
            prop.at,
            prop.yaw,
            prop.scale,
            EStroke::new(1.0, color),
        );
    }
}

pub(crate) fn module_color(kind: AreaModuleKind) -> Color32 {
    match kind {
        AreaModuleKind::Object => Color32::from_rgb(255, 200, 80),
        AreaModuleKind::SafeZone => Color32::from_rgb(90, 160, 255),
        AreaModuleKind::Fog => Color32::from_rgb(200, 210, 225),
        AreaModuleKind::Soundscape => Color32::from_rgb(170, 110, 255),
        AreaModuleKind::Prop => Color32::from_rgb(140, 220, 140),
        AreaModuleKind::Scatter => Color32::from_rgb(110, 190, 110),
        AreaModuleKind::Light => Color32::from_rgb(255, 240, 160),
        AreaModuleKind::Blueprint => Color32::from_rgb(90, 220, 220),
        AreaModuleKind::Population => Color32::from_rgb(255, 120, 120),
        AreaModuleKind::Resident => Color32::from_rgb(255, 160, 200),
        AreaModuleKind::Script => Color32::from_gray(200),
    }
}

/// Where a module lies on the ground, worked out once for a snapshot so that a frame only projects
/// it: its rectangle, the rectangle a glance at it measures (a prop's footprint), the middle of
/// that on the ground and the corners of its rectangle on the ground.
pub struct Footing {
    bounds: Rect<Meters>,
    glanced: Rect<Meters>,
    middle: Vec3,
    corners: [Vec3; 4],
    /// The way round the rectangle on the ground, bent over the terrain, once it is outlined.
    bent: OnceLock<Arc<[Vec3]>>,
}

impl Footing {
    pub(super) fn of(
        content: &Content,
        reader: &FolderReader,
        area: &Area,
        module: &AreaModule,
        bounds: Rect<Meters>,
    ) -> Footing {
        let glanced = match module {
            AreaModule::Prop(prop) => reader
                .path_of(prop.prop)
                .and_then(|path| content.by_path::<PropDef>(path))
                .and_then(|id| {
                    id.get(content)
                        .footprint
                        .bounds(prop.at, prop.yaw, prop.scale)
                })
                .unwrap_or(bounds),
            _ => bounds,
        };
        let on_ground = |at: Pos<Meters>| world_point(at, area.heights.height_at(at));
        Footing {
            bounds,
            glanced,
            middle: on_ground(glanced.center()),
            corners: engine::core::geometry::corners(bounds).map(on_ground),
            bent: OnceLock::new(),
        }
    }

    /// The way round the rectangle on the ground, closed back to its start, each side bent over
    /// the terrain in pieces no longer than `BENT_STEP`.
    fn bent(&self, area: &Area) -> Arc<[Vec3]> {
        self.bent
            .get_or_init(|| {
                let corners = engine::core::geometry::corners(self.bounds);
                let mut round: Vec<Vec3> = (0..4)
                    .flat_map(|side| {
                        let (from, to) = (corners[side], corners[(side + 1) % 4]);
                        let pieces = (from.distance(to).0 / BENT_STEP.0)
                            .ceil()
                            .clamp(1.0, MOST_PIECES) as usize;
                        (0..pieces).map(move |piece| from.lerp(to, piece as f32 / pieces as f32))
                    })
                    .map(|at| world_point(at, area.heights.height_at(at)))
                    .collect();
                round.push(round[0]);
                round.into()
            })
            .clone()
    }

    /// The box on the screen around the rectangle.
    fn screen_box(&self, canvas: &AreaCanvas) -> ERect {
        ERect::from_points(&self.corners.map(|corner| canvas.point(corner)))
    }
}

/// How long the pieces of a module's outline are at most, and how many a side takes at most: the
/// outline bends with the terrain as far as a view shows the bends.
const BENT_STEP: Meters = Meters(4.0);
const MOST_PIECES: f32 = 32.0;

/// A set of outlines the viewport draws in its picture, numbered so it lays them again only when
/// they change: around what stands of the modules at `standing`, wherever the picture shows it, in
/// `color`, and along the ground as the lines `runs`.
#[derive(Clone, Default)]
pub struct OutlineSet {
    pub revision: u64,
    pub standing: Arc<HashSet<String>>,
    pub color: Color32,
    pub runs: Arc<[GroundLine]>,
}

/// The outlines the view draws, each set kept with what it was laid for: the scene's as
/// the view holds it, and the modules hovered on the map or in the scene tree where the scene's
/// leave them out, kept apart so hovering lays only its own.
#[derive(Default)]
pub struct Outlines {
    pub scene: Option<(u64, OutlineSet)>,
    pub hovered: Option<(u64, OutlineSet)>,
}

impl Outlines {
    /// Both sets, the scene's first.
    pub fn sets(&self) -> [OutlineSet; 2] {
        [&self.scene, &self.hovered].map(|kept| {
            kept.as_ref()
                .map(|(_, lines)| lines.clone())
                .unwrap_or_default()
        })
    }
}

/// One closed line over the ground, its colour, and whether it is drawn bold.
pub struct GroundLine {
    pub points: Arc<[Vec3]>,
    pub color: Color32,
    pub bold: bool,
}

/// How the view holds a module: selected, hovered on the map or in the scene tree, or one of the
/// parts the selection names.
struct Held {
    selected: bool,
    hovered: bool,
    part: bool,
}

impl Held {
    fn asked(&self) -> bool {
        self.selected || self.hovered || self.part
    }
}

/// Every module of the view's scene shown on the map: those outlined on the ground as lines the
/// viewport draws, laid again only when what is outlined changes, the rest drawn over the picture,
/// and each hovered module, on the map or in the scene tree, named.
pub(super) fn overlay_modules(
    editor: &mut Editor,
    painter: &egui::Painter,
    view_key: &str,
    frame: &AreaFrame,
) {
    let canvas = &frame.canvas;
    let parts: HashSet<String> = selected_parts(editor, &frame.bundle);
    let selected: HashSet<String> = frame.selected_modules(&parts).into_iter().collect();
    let view = editor.views.area.view(view_key);
    let row = view.hovered_row().map(str::to_owned);
    if let Some((bounds, label)) = frame.folder_box(&parts) {
        painter.add(egui::Shape::dashed_line(
            &canvas.outline(bounds.inflate(0.2, 0.2)),
            EStroke::new(1.5, theme::ACCENT),
            6.0,
            4.0,
        ));
        if row.as_ref().is_some_and(|row| parts.contains(row)) {
            frame.labels.chip(
                canvas.rect(bounds).min + Vec2::new(0.0, -4.0),
                label,
                theme::ACCENT,
            );
        }
    }
    let editing = view.in_place.first().cloned();
    let pointed = match view.tool {
        AreaTool::Select | AreaTool::Module => frame.hit_pointer(),
        _ => None,
    };
    let held = |pointer: &str| Held {
        selected: !selected.is_empty() && selected.contains(pointer),
        hovered: pointed.as_deref() == Some(pointer)
            || row.as_deref().is_some_and(|row| holds(row, pointer)),
        part: !parts.is_empty() && parts.contains(pointer),
    };
    let held_still = |pointer: &str| Held {
        hovered: false,
        ..held(pointer)
    };
    let wash = |kind: AreaModuleKind, pointer: &str, selected: bool| {
        let alpha = if editing.as_deref().is_some_and(|e| e != pointer) {
            0.24
        } else if selected {
            1.0
        } else {
            0.67
        };
        ink::wash(module_color(kind), alpha)
    };
    let key = |hovered: Option<(&Option<String>, &Option<String>)>| {
        let mut hasher = std::hash::DefaultHasher::new();
        let mut asked: Vec<&String> = selected.iter().chain(&parts).collect();
        asked.sort_unstable();
        (frame.made, hovered, &editing, asked).hash(&mut hasher);
        hasher.finish()
    };
    let (scene_key, hovered_key) = (key(None), key(Some((&pointed, &row))));
    let view = editor.views.area.view_mut(view_key);
    if view
        .outlines
        .scene
        .as_ref()
        .is_none_or(|(kept, _)| *kept != scene_key)
    {
        let set = outline_set(
            frame,
            frame.doc.modules(),
            &|pointer| Some(held_still(pointer)),
            &wash,
            module_color(AreaModuleKind::Prop),
        );
        view.outlines.scene = Some((scene_key, set));
    }
    if view
        .outlines
        .hovered
        .as_ref()
        .is_none_or(|(kept, _)| *kept != hovered_key)
    {
        let only_hovered = |pointer: &str| {
            let held = held(pointer);
            (held.hovered && !held_still(pointer).asked()).then_some(held)
        };
        let hovered = hovered_modules(frame, pointed.as_deref(), row.as_deref());
        let set = outline_set(
            frame,
            hovered.into_iter(),
            &only_hovered,
            &wash,
            HOVERED_OUTLINE,
        );
        view.outlines.hovered = Some((hovered_key, set));
    }
    let mut dots = shapes::Dots::default();
    let mut footprints: HashMap<PropId, Option<&Footprint>> = HashMap::new();
    let asked = hovered_modules(frame, pointed.as_deref(), row.as_deref())
        .into_iter()
        .chain(
            selected
                .iter()
                .chain(&parts)
                .filter_map(|pointer| frame.doc.module_at(pointer)),
        );
    let mut seen: Vec<DocModule> = frame
        .drawn_unasked_within(frame.seen_ground())
        .chain(asked)
        .collect();
    seen.sort_by_key(|module| module.id);
    seen.dedup_by_key(|module| module.id);
    for node in seen {
        let Some(footing) = frame.footing(node).filter(|_| frame.shown(node)) else {
            continue;
        };
        let module = node.module();
        let held = held(node.pointer);
        let on_ground = outlined_on_ground(frame, module);
        if on_ground && !held.hovered && !held.part {
            continue;
        }
        if !on_ground && !held.asked() && !shown_unasked(module) {
            continue;
        }
        let Some((at, across)) = canvas.glance_from(footing.middle, footing.glanced) else {
            continue;
        };
        if !canvas.shows(at, across) {
            continue;
        }
        let bounds = footing.bounds;
        let color = wash(module.kind(), node.pointer, held.selected);
        let width = if held.selected { 2.0 } else { 1.0 };
        if !loads(frame, module) {
            painter.add(egui::Shape::dashed_line(
                &canvas.outline(bounds),
                EStroke::new(1.5, theme::DANGER),
                5.0,
                3.0,
            ));
            frame.labels.chip(
                footing.screen_box(canvas).min + Vec2::new(0.0, -4.0),
                format!(
                    "{} doesn't load",
                    shapes::label(module, frame.reader.as_ref())
                ),
                theme::DANGER,
            );
            continue;
        }
        if !on_ground {
            if !held.asked() && !shapes::shows_shape(across) {
                dots.add(painter, at, color);
                continue;
            }
            let footprint = match module {
                AreaModule::Prop(prop) => *footprints.entry(prop.prop).or_insert_with(|| {
                    frame
                        .reader
                        .path_of(prop.prop)
                        .and_then(|path| frame.content.by_path::<PropDef>(path))
                        .map(|id| &id.get(&frame.content).footprint)
                }),
                _ => None,
            };
            draw_shape(
                painter,
                canvas,
                module,
                bounds,
                footprint,
                EStroke::new(width, color),
            );
        }
        if held.hovered {
            let anchor = footing.screen_box(canvas).min + Vec2::new(0.0, -4.0);
            let label = || shapes::label(module, frame.reader.as_ref());
            if pointed.as_deref() == Some(node.pointer) {
                frame.labels.chip(anchor, label(), color);
            } else {
                frame.labels.aside(anchor, label, color);
            }
        }
        if held.part && shapes::has_size(module) {
            for corner in footing.corners {
                painter.rect_filled(
                    ERect::from_center_size(canvas.point(corner), Vec2::splat(6.0)),
                    0.0,
                    color,
                );
            }
        }
    }
}

/// Whether the tree row `row` holds the module at `pointer`: the module's own row, or a folder
/// above it, "" the whole scene.
fn holds(row: &str, pointer: &str) -> bool {
    pointer
        .strip_prefix(row)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Whether the map draws `module` though nothing asks for it: one it doesn't outline on the
/// ground, of a kind that shows unasked.
pub(super) fn drawn_unasked(snapshot: &AreaSnapshot, module: &AreaModule) -> bool {
    !outlined_on_ground(snapshot, module) && shown_unasked(module)
}

/// Whether a module shows on the map when nothing asks for it: a scatter or a blueprint placement,
/// which can tile a whole area, only when selected or hovered.
fn shown_unasked(module: &AreaModule) -> bool {
    !matches!(module, AreaModule::Scatter(_) | AreaModule::Blueprint(_))
}

/// Whether a module is outlined on the ground by the rectangle it takes, as a line the viewport
/// draws: every module shaped as its rectangle, rather than as a point, a circle or a footprint.
fn outlined_on_ground(snapshot: &AreaSnapshot, module: &AreaModule) -> bool {
    match module {
        AreaModule::Soundscape(_) => !shapes::is_ellipse(module),
        AreaModule::Object(_) => !shapes::is_point(module),
        AreaModule::Blueprint(_) => loads(snapshot, module),
        AreaModule::SafeZone(_)
        | AreaModule::Fog(_)
        | AreaModule::Scatter(_)
        | AreaModule::Population(_) => true,
        AreaModule::Prop(_)
        | AreaModule::Light(_)
        | AreaModule::Resident(_)
        | AreaModule::Script(_) => false,
    }
}

/// Whether a module loads: a blueprint placement whose blueprint does; every other module.
fn loads(snapshot: &AreaSnapshot, module: &AreaModule) -> bool {
    match module {
        AreaModule::Blueprint(placement) => snapshot
            .reader
            .path_of(placement.blueprint)
            .and_then(|path| snapshot.content.by_path::<BlueprintDef>(path))
            .is_some(),
        _ => true,
    }
}

/// The modules hovered on the map, at `pointed`, or under the scene tree's row `row`, "" the
/// whole scene.
fn hovered_modules<'a>(
    frame: &'a AreaFrame,
    pointed: Option<&str>,
    row: Option<&str>,
) -> Vec<DocModule<'a>> {
    let doc = &frame.doc;
    let mut hovered: Vec<DocModule> = pointed
        .and_then(|pointer| doc.module_at(pointer))
        .into_iter()
        .collect();
    match row {
        Some("") => hovered.extend(doc.modules()),
        Some(row) => hovered.extend(doc.id(row).into_iter().flat_map(|id| doc.modules_under(id))),
        None => {}
    }
    hovered.sort_by_key(|module| module.id);
    hovered.dedup_by_key(|module| module.id);
    hovered
}

/// The outlines, in `color`, of every module among `modules` that `held` says how the view holds:
/// what stands of those that stand props, and on the ground those that show as a line there.
fn outline_set<'a>(
    frame: &'a AreaFrame,
    modules: impl Iterator<Item = DocModule<'a>>,
    held: &dyn Fn(&str) -> Option<Held>,
    wash: &dyn Fn(AreaModuleKind, &str, bool) -> Color32,
    color: Color32,
) -> OutlineSet {
    static REVISION: AtomicU64 = AtomicU64::new(0);
    let area = frame.area();
    let mut standing = HashSet::new();
    let outlined: Vec<(&Footing, Color32, bool)> = modules
        .filter_map(|node| {
            let footing = frame.footing(node).filter(|_| frame.shown(node))?;
            let module = node.module();
            let held = held(node.pointer)?;
            if held.asked() && stands_props(module) {
                standing.insert(node.pointer.to_owned());
            }
            if !outlined_on_ground(frame, module) || !(held.asked() || shown_unasked(module)) {
                return None;
            }
            Some((
                footing,
                wash(module.kind(), node.pointer, held.selected),
                held.selected,
            ))
        })
        .collect();
    let runs: Vec<GroundLine> = outlined
        .into_par_iter()
        .map(|(footing, color, bold)| GroundLine {
            points: footing.bent(area),
            color,
            bold,
        })
        .collect();
    OutlineSet {
        revision: REVISION.fetch_add(1, Ordering::Relaxed),
        standing: Arc::new(standing),
        color,
        runs: runs.into(),
    }
}

/// Whether a module stands props the picture shows: a prop, a scatter, or a blueprint placement.
fn stands_props(module: &AreaModule) -> bool {
    matches!(
        module,
        AreaModule::Prop(_) | AreaModule::Scatter(_) | AreaModule::Blueprint(_)
    )
}

/// A module drawn over the picture by its own shape.
fn draw_shape(
    painter: &egui::Painter,
    canvas: &AreaCanvas,
    module: &AreaModule,
    bounds: Rect<Meters>,
    footprint: Option<&Footprint>,
    stroke: EStroke,
) {
    match module {
        AreaModule::Soundscape(_) => {
            painter.add(egui::Shape::closed_line(canvas.ellipse(bounds), stroke));
        }
        AreaModule::Prop(prop) => match footprint {
            Some(footprint) => shapes::draw_footprint(
                painter, canvas, footprint, prop.at, prop.yaw, prop.scale, stroke,
            ),
            None => {
                painter.add(egui::Shape::closed_line(
                    canvas.circle(prop.at, 0.25),
                    stroke,
                ));
            }
        },
        AreaModule::Light(light) => {
            let foot = canvas.to_screen(light.at);
            let bulb = canvas.lifted(light.at, light.height);
            painter.line_segment(
                [foot, bulb],
                EStroke::new(stroke.width, ink::wash(stroke.color, 0.6)),
            );
            painter.circle_stroke(bulb, 5.0, stroke);
            painter.add(egui::Shape::dashed_line(
                &canvas.circle(light.at, light.radius.0),
                EStroke::new(1.0, ink::wash(stroke.color, 0.5)),
                6.0,
                6.0,
            ));
        }
        AreaModule::Resident(resident) => {
            painter.add(egui::Shape::closed_line(
                canvas.circle(resident.at, 0.35),
                stroke,
            ));
            let heading = resident.at
                + resident
                    .facing
                    .rotate(engine::core::math::Offset::new(0.0, -0.7));
            painter.add(egui::Shape::line(
                canvas.polyline(&[resident.at, heading]),
                stroke,
            ));
        }
        _ => {
            let center = bounds.center();
            painter.add(egui::Shape::closed_line(
                canvas.circle(Pos::new(center.x, center.y), 0.35),
                stroke,
            ));
        }
    }
}

pub(super) fn overlay_soundscape_reach(
    editor: &Editor,
    painter: &egui::Painter,
    frame: &AreaFrame,
) {
    let canvas = &frame.canvas;
    for part in selected_parts(editor, &frame.bundle) {
        let Some(module @ AreaModule::Soundscape(soundscape)) = frame.module(&part) else {
            continue;
        };
        let bounds = soundscape.shape.bounds();
        let reach = soundscape.reach.0;
        let outer = bounds.inflate(reach, reach);
        let color = Color32::from_rgba_unmultiplied(170, 110, 255, 120);
        let line = if shapes::is_ellipse(module) {
            canvas.ellipse(outer)
        } else {
            canvas.outline(outer)
        };
        painter.add(egui::Shape::dashed_line(
            &line,
            EStroke::new(1.0, color),
            6.0,
            6.0,
        ));
        frame.labels.chip(
            canvas.rect(outer).left_bottom() + Vec2::new(0.0, 22.0),
            format!("heard within {} m", forms::metres(reach)),
            Color32::from_rgb(190, 150, 255),
        );
    }
}

/// The labels drawn over the map this frame, each kept clear of those before it: those it must
/// show, and those it shows only where they find room, so however many ask, the labels never pile
/// over one another. Of those shown where they find room, one at most begins on each spot a label
/// takes, so however many ask, no more are named than the screen has room for.
#[derive(Default)]
pub struct AreaLabels {
    labels: RefCell<Vec<AreaLabel>>,
    spots: RefCell<HashSet<(i32, i32)>>,
}

struct AreaLabel {
    bottom_left: Pos2,
    text: String,
    color: Color32,
    /// Whether the label must show, else only where it finds room.
    needed: bool,
}

impl AreaLabels {
    pub fn chip(&self, bottom_left: Pos2, text: impl Into<String>, color: Color32) {
        self.push(bottom_left, text.into(), color, true);
    }

    /// A label shown only where it finds room among the others, its text asked for only when no
    /// other has begun on its spot.
    pub fn aside(&self, bottom_left: Pos2, text: impl FnOnce() -> String, color: Color32) {
        let spot = (
            (bottom_left.x / LABEL_SPOT.x).floor() as i32,
            (bottom_left.y / LABEL_SPOT.y).floor() as i32,
        );
        if self.spots.borrow_mut().insert(spot) {
            self.push(bottom_left, text(), color, false);
        }
    }

    pub fn draw(&self, painter: &egui::Painter) {
        let mut placed = Placed::default();
        let clip = painter.clip_rect();
        let (needed, aside): (Vec<AreaLabel>, Vec<AreaLabel>) = self
            .labels
            .take()
            .into_iter()
            .partition(|label| label.needed);
        let font = egui::FontId::monospace(LABEL_TEXT);
        let glyph = painter
            .layout_no_wrap("0".to_owned(), font.clone(), theme::TEXT)
            .size();
        for label in needed.into_iter().chain(aside) {
            if !clip.contains(label.bottom_left) {
                continue;
            }
            let size = Vec2::new(glyph.x * label.text.chars().count() as f32, glyph.y)
                + Vec2::new(14.0, 6.0);
            let preferred = ERect::from_min_size(label.bottom_left - Vec2::new(0.0, size.y), size);
            let back = match placed.free_slot(preferred) {
                Some(slot) => slot,
                None if label.needed => preferred,
                None => continue,
            };
            placed.add(back);
            let galley = painter.layout_no_wrap(label.text, font.clone(), theme::TEXT);
            ui::slip::paint(painter, back);
            painter.rect_filled(
                ERect::from_min_size(back.min, Vec2::new(3.0, back.height())),
                0.0,
                label.color,
            );
            painter.galley(back.min + Vec2::new(9.0, 3.0), galley, theme::TEXT);
        }
    }

    fn push(&self, bottom_left: Pos2, text: String, color: Color32, needed: bool) {
        self.labels.borrow_mut().push(AreaLabel {
            bottom_left,
            text,
            color,
            needed,
        });
    }
}

/// The labels placed so far, filed by the cells of the screen they cover.
#[derive(Default)]
struct Placed(HashMap<(i32, i32), Vec<ERect>>);

impl Placed {
    /// Where a label wanting `preferred` goes clear of those placed: there, or a few rows above or
    /// below; none where all of those are taken.
    fn free_slot(&self, preferred: ERect) -> Option<ERect> {
        let row = preferred.height() + LABEL_GAP;
        let shifted = (1..=LABEL_SHIFTS)
            .flat_map(|shift| [-(shift as f32), shift as f32])
            .map(|rows| preferred.translate(Vec2::new(0.0, rows * row)));
        std::iter::once(preferred).chain(shifted).find(|slot| {
            !cells(*slot).any(|cell| {
                self.0
                    .get(&cell)
                    .is_some_and(|rects| rects.iter().any(|other| other.intersects(*slot)))
            })
        })
    }

    fn add(&mut self, rect: ERect) {
        for cell in cells(rect) {
            self.0.entry(cell).or_default().push(rect);
        }
    }
}

/// The cells of the screen `rect` covers.
fn cells(rect: ERect) -> impl Iterator<Item = (i32, i32)> {
    let cell = |at: f32| (at / LABEL_CELL).floor() as i32;
    (cell(rect.min.y)..=cell(rect.max.y))
        .flat_map(move |row| (cell(rect.min.x)..=cell(rect.max.x)).map(move |column| (column, row)))
}

/// The colour of the outline around what stands of a hovered module.
const HOVERED_OUTLINE: Color32 = Color32::from_gray(235);
