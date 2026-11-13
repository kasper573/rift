mod ground;

use std::f32::consts::TAU;

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::emath::TSTransform;
use ui::egui::epaint::{Mesh, Vertex};
use ui::egui::{self, Align2, Color32, CornerRadius, Pos2, Sense, Shape, Stroke, Ui, Vec2};
use ui::face::{Face, TextRole};
use ui::{dusk, float, ink, slip};

use crate::core::content::{Content, RowCheck};
use crate::core::gui::{self, GuiImage};
use crate::core::math::{Pos, Rect};
use crate::core::render::CameraRig;
use crate::core::render::world::opacity::Opacity;
use crate::core::units::{MeterSize, Meters, Radians};
use crate::systems::actor::{Action, Actor, Name};
use crate::systems::area::{self, AreaId, AreaTag};
use crate::systems::attention::{self, AttentionId};
use crate::systems::combat::Attitude;
use crate::systems::hud::{self, HudWidget};
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::npc::Npc;
use crate::systems::player::{PlayedBy, session};
use crate::systems::screen::ClientScreen;
use crate::systems::script::RiftScripts;
use crate::systems::settings::UserSettings;
use crate::systems::{GameSettings, visibility};

pub struct MinimapPlugin;

impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ground::MinimapGround>().add_systems(
            Update,
            picture_the_ground.run_if(in_state(ClientScreen::Area)),
        );
    }
}

/// How far a map reaches at each step, closest first.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MinimapReaches(pub Vec<Meters>);

impl MinimapReaches {
    pub fn check(&self, pointer: &str, check: &mut RowCheck) {
        if self.0.is_empty() {
            check.error(pointer, "no reach", "an empty list", "at least one reach");
        }
        let mut closer: Option<Meters> = None;
        for (index, reach) in self.0.iter().enumerate() {
            let at = format!("{pointer}/{index}");
            if reach.0 <= 0.0 {
                check.error(at, "out of range", reach.0, "meters above 0");
            } else if let Some(closer) = closer.filter(|closer| reach.0 <= closer.0) {
                let further = format!("a reach further than the {} m before it", closer.0);
                check.error(at, "out of order", reach.0, further);
            }
            closer = Some(*reach);
        }
    }

    pub fn step_reaching(&self, sight: Meters) -> MinimapReachStep {
        let step = self
            .0
            .iter()
            .position(|reach| reach.0 >= sight.0)
            .unwrap_or(self.0.len().saturating_sub(1));
        MinimapReachStep(step as u32)
    }

    fn last_step(&self) -> MinimapReachStep {
        MinimapReachStep(self.0.len().saturating_sub(1) as u32)
    }

    fn at(&self, step: MinimapReachStep) -> Meters {
        let step = step.min(self.last_step());
        self.0.get(step.0 as usize).copied().unwrap_or(Meters(1.0))
    }

    fn widest(&self) -> Meters {
        self.at(self.last_step())
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct MinimapReachStep(pub u32);

/// Each kind is drawn as a shape of its own, so none is told from another by colour alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MinimapMarkKind {
    Townsperson,
    Player,
    WayOut,
}

#[derive(Clone, Debug)]
pub struct MinimapMark {
    pub kind: MinimapMarkKind,
    pub name: String,
    pub at: Pos<Meters>,
    pub attention: Vec<AttentionId>,
    pub opacity: f32,
}

pub fn marks(world: &mut World, viewer: Entity) -> Vec<MinimapMark> {
    let Some(area) = world.get::<AreaTag>(viewer).map(|tag| tag.area) else {
        return Vec::new();
    };
    let Some(eye) = stands_at(world, viewer) else {
        return Vec::new();
    };
    let sight = visibility::sight(world);
    let content = world.resource::<Content>().clone();
    let seen: Vec<(Entity, MinimapMark)> = world
        .query::<(
            Entity,
            &AreaTag,
            &Position,
            Option<&RenderPosition>,
            (Option<&Npc>, Option<&Attitude>, Option<&PlayedBy>),
            (Option<&Name>, Option<&Actor>, Option<&Opacity>),
        )>()
        .iter(world)
        .filter(|(entity, tag, ..)| *entity != viewer && tag.area == area)
        .filter_map(|(entity, _, at, rendered, side, (name, body, opacity))| {
            let at = rendered.map_or(at.pos, |rendered| rendered.0);
            if eye.distance_to(at) > sight.0 || body.is_some_and(|body| body.action == Action::Dead)
            {
                return None;
            }
            let (kind, name) = match side {
                (Some(npc), Some(Attitude::Friendly), _) => (
                    MinimapMarkKind::Townsperson,
                    npc.def.get(&content).name.clone(),
                ),
                (None, _, Some(_)) => (
                    MinimapMarkKind::Player,
                    name.map(|name| name.name.clone()).unwrap_or_default(),
                ),
                _ => return None,
            };
            let mark = MinimapMark {
                kind,
                name,
                at,
                attention: Vec::new(),
                opacity: opacity.map_or(1.0, |opacity| opacity.0),
            };
            Some((entity, mark))
        })
        .collect();
    let people = seen.into_iter().map(|(entity, mark)| MinimapMark {
        attention: attention::plate_marks(world, Some(viewer), entity),
        ..mark
    });
    let scripts = world.resource::<RiftScripts>();
    let ways = area::warp::warp_objects(&content, scripts, area)
        .into_iter()
        .filter(|object| object.to.area != area)
        .map(|object| MinimapMark {
            kind: MinimapMarkKind::WayOut,
            name: object.to.area.get(&content).name.clone(),
            at: object.shape.center(),
            attention: Vec::new(),
            opacity: 1.0,
        });
    people.chain(ways).collect()
}

pub const MINIMAP: &str = "minimap";
pub const RESTING_SIZE: Vec2 = Vec2::new(
    DISC + 2.0 * RIM_OVERHANG,
    DISC + 2.0 * RIM_OVERHANG + NAME_ROOM,
);

pub struct MinimapWidget;

impl HudWidget for MinimapWidget {
    fn fallback(&self, screen: egui::Rect) -> Pos2 {
        Pos2::new(
            screen.right() - RESTING_SIZE.x - hud::MARGIN,
            screen.top() + hud::MARGIN,
        )
    }

    fn show(&self, ui: &mut Ui, world: &mut World) {
        let (piece, _) = ui.allocate_exact_size(RESTING_SIZE, Sense::hover());
        let center = piece.center_top() + Vec2::new(0.0, RIM_OVERHANG + DISC / 2.0);
        let whole = egui::Rect::from_center_size(center, Vec2::splat(DISC));
        ui.painter()
            .add(slip::SHADOW.as_shape(whole, CornerRadius::same((DISC / 2.0) as u8)));
        let Some(view) = MapView::of(world, center) else {
            ui.painter().circle_filled(center, GROUND, BLANK);
            rim(ui, center);
            return;
        };
        let content = world.resource::<Content>().clone();
        let area_name = view.area.get(&content).name.clone();
        let disc = ui.interact(whole, ui.id().with("disc"), Sense::hover());
        let reach = view.reach.0;
        disc.widget_info(|| {
            let said = format!("Map of {area_name}, reaching {reach} m");
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, said)
        });
        ground_under(ui, world, &view);
        if !view.turning {
            wedge(ui, &view);
        }
        sight_ring(ui, &view);
        let marks = marks(world, view.me);
        let under = marked(ui, world, &view, &marks);
        myself(ui, &view);
        rim(ui, center);
        ways_beyond(ui, &view, &marks);
        if north_bead(ui, &view).clicked()
            && let Some(mut rig) = world.get_resource_mut::<CameraRig>()
        {
            rig.face(Radians(0.0));
        }
        let stepped = reach_buttons(ui, &view).or_else(|| wheeled(ui, &disc, &view));
        if let Some(step) = stepped {
            world.resource_mut::<UserSettings>().set_minimap_reach(step);
        }
        lettered(
            ui,
            &area_name,
            center + Vec2::new(0.0, DISC / 2.0 + RIM_OVERHANG),
        );
        if !under.is_empty() {
            float::tooltip(&disc, |ui| {
                for name in &under {
                    ui.label(TextRole::Tooltip.rich(name.as_str()));
                }
            });
        }
    }
}

const DISC: f32 = 164.0;
const RIM: f32 = 6.0;
const GROUND: f32 = DISC / 2.0 - RIM;
const RIM_OVERHANG: f32 = 7.0;
const NAME_ROOM: f32 = 22.0;
const ROUND: usize = 72;
const BEAD: f32 = 9.0;
const BUTTON: f32 = 8.5;
// Clockwise from the rim's right end.
const CLOSER_AT: Radians = Radians(0.56);
const WIDER_AT: Radians = Radians(0.84);
const WEDGE_HALF: f32 = 0.5;
const WEDGE_REACH: f32 = 0.62;
const PERSON: f32 = 4.5;
const DIAMOND: f32 = 5.5;
const DOOR: Vec2 = Vec2::new(9.0, 10.0);
const ATTENTION: f32 = 14.0;
const HOVER_REACH: f32 = 9.0;
const POINTS_PER_NOTCH: f32 = 50.0;

const BLANK: Color32 = Color32::from_rgb(0xD9, 0xCB, 0xA3);
const NO_GROUND: Color32 = Color32::from_rgb(0x4B, 0x43, 0x3A);
const TOWNSPERSON: Color32 = Color32::from_rgb(0x6F, 0xB8, 0x74);
const PLAYER: Color32 = Color32::from_rgb(0x5B, 0xA6, 0xE6);
const EDGE: Color32 = ink::FRAME;

struct MapView {
    me: Entity,
    area: AreaId,
    at: Pos<Meters>,
    facing: Radians,
    reach: Meters,
    step: MinimapReachStep,
    last_step: MinimapReachStep,
    sight: Meters,
    yaw: f32,
    turning: bool,
    center: Pos2,
}

impl MapView {
    fn of(world: &World, center: Pos2) -> Option<MapView> {
        let me = session::me(world)?;
        let settings = world.resource::<GameSettings>();
        let reaches = &settings.minimap_reaches;
        let user = world.resource::<UserSettings>();
        let step = user
            .minimap_reach()
            .unwrap_or(reaches.step_reaching(settings.sight))
            .min(reaches.last_step());
        Some(MapView {
            me,
            area: world.get::<AreaTag>(me)?.area,
            at: stands_at(world, me)?,
            facing: world.get::<Actor>(me).map_or(Radians(0.0), |me| me.facing),
            reach: reaches.at(step),
            step,
            last_step: reaches.last_step(),
            sight: settings.sight,
            yaw: world.get_resource::<CameraRig>()?.yaw().0,
            turning: user.minimap_turns(),
            center,
        })
    }

    fn ground_turn(&self) -> f32 {
        if self.turning { self.yaw } else { 0.0 }
    }

    fn per_meter(&self) -> f32 {
        GROUND / self.reach.0
    }

    fn point(&self, at: Pos<Meters>) -> Pos2 {
        let from_me = Vec2::new(at.x - self.at.x, at.y - self.at.y) * self.per_meter();
        self.center + turned(from_me, self.ground_turn())
    }

    fn ground(&self, point: Pos2) -> Pos<Meters> {
        let from_me = turned(point - self.center, -self.ground_turn()) / self.per_meter();
        Pos::new(self.at.x + from_me.x, self.at.y + from_me.y)
    }

    fn north(&self) -> Vec2 {
        turned(Vec2::UP, self.ground_turn())
    }

    // At yaw nothing the camera looks north.
    fn looks(&self) -> Vec2 {
        turned(Vec2::UP, self.ground_turn() - self.yaw)
    }

    fn faces(&self) -> Vec2 {
        let heading = self.facing.heading();
        turned(Vec2::new(heading.x, heading.y), self.ground_turn())
    }
}

fn picture_the_ground(world: &mut World) {
    let Some(me) = session::me(world) else {
        return;
    };
    let (Some(area), Some(at)) = (
        world.get::<AreaTag>(me).map(|tag| tag.area),
        stands_at(world, me),
    ) else {
        return;
    };
    let widest = world.resource::<GameSettings>().minimap_reaches.widest();
    ground::keep(world, area, at, widest);
}

fn stands_at(world: &World, entity: Entity) -> Option<Pos<Meters>> {
    world
        .get::<RenderPosition>(entity)
        .map(|rendered| rendered.0)
        .or_else(|| world.get::<Position>(entity).map(|at| at.pos))
}

fn turned(offset: Vec2, by: f32) -> Vec2 {
    let (sin, cos) = by.sin_cos();
    Vec2::new(
        offset.x * cos - offset.y * sin,
        offset.x * sin + offset.y * cos,
    )
}

fn round(center: Pos2, radius: f32) -> Vec<Pos2> {
    (0..ROUND)
        .map(|step| center + Vec2::angled(TAU * step as f32 / ROUND as f32) * radius)
        .collect()
}

fn corners(view: &MapView, rect: Rect<Meters>) -> [Pos2; 4] {
    [
        Pos::new(rect.min_x(), rect.min_y()),
        Pos::new(rect.max_x(), rect.min_y()),
        Pos::new(rect.max_x(), rect.max_y()),
        Pos::new(rect.min_x(), rect.max_y()),
    ]
    .map(|corner| view.point(corner))
}

// egui clips to upright rects alone, so what the disc shows of a tile is cut out as a shape of
// its own. Both shapes must bulge outward everywhere.
fn within(shape: &[Pos2], quad: [Pos2; 4]) -> Vec<Pos2> {
    let middle = quad
        .iter()
        .fold(Vec2::ZERO, |sum, corner| sum + corner.to_vec2())
        / 4.0;
    let mut kept = shape.to_vec();
    for edge in 0..4 {
        let (from, to) = (quad[edge], quad[(edge + 1) % 4]);
        let along = to - from;
        let side = |point: Pos2| along.x * (point.y - from.y) - along.y * (point.x - from.x);
        let inward = side(middle.to_pos2()).signum();
        let cut = std::mem::take(&mut kept);
        for (index, &point) in cut.iter().enumerate() {
            let next = cut[(index + 1) % cut.len()];
            let (here, there) = (side(point) * inward, side(next) * inward);
            if here >= 0.0 {
                kept.push(point);
            }
            if (here >= 0.0) != (there >= 0.0) {
                kept.push(point + (next - point) * (here / (here - there)));
            }
        }
        if kept.is_empty() {
            break;
        }
    }
    kept
}

fn ground_under(ui: &Ui, world: &World, view: &MapView) {
    let painter = ui.painter();
    let disc = round(view.center, GROUND);
    painter.add(Shape::convex_polygon(disc.clone(), NO_GROUND, Stroke::NONE));
    let content = world.resource::<Content>();
    if let Some(area) = area::loaded(content, view.area) {
        let held = within(&disc, corners(view, area.size.bounds()));
        if held.len() >= 3 {
            painter.add(Shape::convex_polygon(held, BLANK, Stroke::NONE));
        }
    }
    for picture in world
        .resource::<ground::MinimapGround>()
        .pictures(view.area)
    {
        let shown = within(&disc, corners(view, picture.rect));
        if shown.len() < 3 {
            continue;
        }
        let mut mesh = Mesh::with_texture(picture.texture.id());
        for point in &shown {
            let at = view.ground(*point);
            mesh.vertices.push(Vertex {
                pos: *point,
                uv: Pos2::new(
                    (at.x - picture.rect.min_x()) / picture.rect.width(),
                    (at.y - picture.rect.min_y()) / picture.rect.height(),
                ),
                color: Color32::WHITE,
            });
        }
        for corner in 1..shown.len() as u32 - 1 {
            mesh.add_triangle(0, corner, corner + 1);
        }
        painter.add(Shape::mesh(mesh));
    }
}

fn wedge(ui: &Ui, view: &MapView) {
    let looks = view.looks().angle();
    let reach = GROUND * WEDGE_REACH;
    let steps = 12;
    let arc = (0..=steps).map(|step| {
        let angle = looks - WEDGE_HALF + 2.0 * WEDGE_HALF * step as f32 / steps as f32;
        view.center + Vec2::angled(angle) * reach
    });
    let shape: Vec<Pos2> = std::iter::once(view.center).chain(arc).collect();
    ui.painter().add(Shape::convex_polygon(
        shape,
        Color32::from_white_alpha(56),
        Stroke::new(1.0, Color32::from_white_alpha(150)),
    ));
}

fn sight_ring(ui: &Ui, view: &MapView) {
    let radius = view.sight.0 * view.per_meter();
    if radius > GROUND - 2.0 {
        return;
    }
    let dashes = ((TAU * radius / 9.0).round() as usize).max(8);
    let stroke = Stroke::new(1.0, Color32::from_white_alpha(190));
    for dash in 0..dashes {
        let from = TAU * dash as f32 / dashes as f32;
        let to = from + TAU / dashes as f32 * 0.55;
        ui.painter().line_segment(
            [
                view.center + Vec2::angled(from) * radius,
                view.center + Vec2::angled(to) * radius,
            ],
            stroke,
        );
    }
}

fn marked(ui: &mut Ui, world: &mut World, view: &MapView, marks: &[MinimapMark]) -> Vec<String> {
    let content = world.resource::<Content>().clone();
    let pointer = ui.input(|input| input.pointer.hover_pos());
    let on_disc: Vec<(usize, &MinimapMark, Pos2)> = marks
        .iter()
        .enumerate()
        .map(|(index, mark)| (index, mark, view.point(mark.at)))
        .filter(|(_, _, at)| (*at - view.center).length() <= GROUND - 2.0)
        .collect();
    let mut under = Vec::new();
    // The more telling a kind, the later it is drawn, so it lies on top.
    for kind in [
        MinimapMarkKind::WayOut,
        MinimapMarkKind::Townsperson,
        MinimapMarkKind::Player,
    ] {
        for &(index, mark, at) in on_disc.iter().filter(|(_, mark, _)| mark.kind == kind) {
            let fill = |color: Color32| color.gamma_multiply(mark.opacity);
            let edge = Stroke::new(1.5, fill(EDGE));
            let said = match kind {
                MinimapMarkKind::WayOut => {
                    ui.painter()
                        .add(Shape::convex_polygon(door(at), fill(ink::LIGHT), edge));
                    format!("Way to {}", mark.name)
                }
                MinimapMarkKind::Townsperson => {
                    ui.painter().circle(at, PERSON, fill(TOWNSPERSON), edge);
                    mark.name.clone()
                }
                MinimapMarkKind::Player => {
                    let diamond = [Vec2::UP, Vec2::RIGHT, Vec2::DOWN, Vec2::LEFT]
                        .map(|corner| at + corner * DIAMOND)
                        .to_vec();
                    ui.painter()
                        .add(Shape::convex_polygon(diamond, fill(PLAYER), edge));
                    format!("{} · player", mark.name)
                }
            };
            let spot = egui::Rect::from_center_size(at, Vec2::splat(2.0 * HOVER_REACH));
            let node = ui.interact(spot, ui.id().with(("mark", index)), Sense::hover());
            let label = format!("On the map: {said}");
            node.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label.clone())
            });
            if pointer.is_some_and(|pointer| spot.contains(pointer)) {
                under.push(said);
            }
        }
    }
    for &(_, mark, at) in &on_disc {
        let icons: Vec<GuiImage> = mark
            .attention
            .iter()
            .filter_map(|attention| gui::image(world, &attention.get(&content).icon))
            .collect();
        for (place, icon) in icons.iter().enumerate() {
            let over = at + Vec2::new(place as f32 * ATTENTION, -PERSON - ATTENTION / 2.0);
            let tint = Color32::WHITE.gamma_multiply(mark.opacity);
            ui.put(
                egui::Rect::from_center_size(over, Vec2::splat(ATTENTION)),
                icon.widget(Vec2::splat(ATTENTION)).tint(tint),
            );
        }
    }
    under.reverse();
    under
}

fn door(at: Pos2) -> Vec<Pos2> {
    let half = DOOR.x / 2.0;
    let arch = (0..=8).map(|step| {
        let angle = TAU / 2.0 + TAU / 2.0 * step as f32 / 8.0;
        at + Vec2::new(0.0, half - DOOR.y / 2.0) + Vec2::angled(angle) * half
    });
    arch.chain([
        at + Vec2::new(half, DOOR.y / 2.0),
        at + Vec2::new(-half, DOOR.y / 2.0),
    ])
    .collect()
}

// Two halves, since egui fills only shapes that bulge outward everywhere.
fn arrow(ui: &Ui, at: Pos2, toward: Vec2, size: f32, fill: Color32) {
    let across = toward.rot90();
    let tip = at + toward * size;
    let notch = at - toward * size * 0.35;
    let wings = [-1.0, 1.0].map(|side| at - toward * size * 0.8 + across * side * size * 0.7);
    for wing in wings {
        ui.painter().add(Shape::convex_polygon(
            vec![tip, wing, notch],
            fill,
            Stroke::NONE,
        ));
    }
    ui.painter().add(Shape::closed_line(
        vec![tip, wings[0], notch, wings[1]],
        Stroke::new(1.5, EDGE),
    ));
}

fn myself(ui: &mut Ui, view: &MapView) {
    arrow(ui, view.center, view.faces(), 8.0, Color32::WHITE);
    let spot = egui::Rect::from_center_size(view.center, Vec2::splat(16.0));
    ui.interact(spot, ui.id().with("me"), Sense::hover())
        .widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "On the map: you")
        });
}

fn rim(ui: &Ui, center: Pos2) {
    let painter = ui.painter();
    painter.circle_stroke(center, GROUND + RIM / 2.0, Stroke::new(RIM, ink::FRAME));
    painter.circle_stroke(center, GROUND, Stroke::new(1.0, ink::LIGHT));
    painter.circle_stroke(center, DISC / 2.0, Stroke::new(1.5, ink::GOLD));
}

fn ways_beyond(ui: &mut Ui, view: &MapView, marks: &[MinimapMark]) {
    let ways = marks
        .iter()
        .enumerate()
        .filter(|(_, mark)| mark.kind == MinimapMarkKind::WayOut);
    for (index, mark) in ways {
        let toward = view.point(mark.at) - view.center;
        if toward.length() <= GROUND - 2.0 {
            continue;
        }
        let toward = toward.normalized();
        let at = view.center + toward * (GROUND - 8.0);
        arrow(ui, at, toward, 6.5, ink::LIGHT);
        let spot = egui::Rect::from_center_size(at, Vec2::splat(14.0));
        let said = format!("On the map's rim: way to {}", mark.name);
        let node = ui.interact(spot, ui.id().with(("way", index)), Sense::hover());
        node.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, said.clone()));
        float::tooltip(&node, |ui| {
            ui.label(TextRole::Tooltip.rich(format!("Way to {}", mark.name)));
        });
    }
}

fn north_bead(ui: &mut Ui, view: &MapView) -> egui::Response {
    let at = view.center + view.north() * (GROUND + RIM / 2.0);
    let spot = egui::Rect::from_center_size(at, Vec2::splat(2.0 * BEAD));
    let bead = ui
        .interact(spot, ui.id().with("north"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    bead.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Face north"));
    let rim = if bead.hovered() {
        ink::GOLD2
    } else {
        ink::GOLD
    };
    ui.painter()
        .circle(at, BEAD, ink::FRAME, Stroke::new(1.5, rim));
    ui.painter().text(
        at,
        Align2::CENTER_CENTER,
        "N",
        Face::BookBold.id(12.0),
        ink::GOLD2,
    );
    float::tooltip(&bead, |ui| {
        ui.label(TextRole::Tooltip.rich("North · click to face it"));
    });
    bead
}

fn reach_buttons(ui: &mut Ui, view: &MapView) -> Option<MinimapReachStep> {
    let closer = (view.step.0 > 0).then(|| MinimapReachStep(view.step.0 - 1));
    let wider = (view.step < view.last_step).then(|| MinimapReachStep(view.step.0 + 1));
    let pressed = [
        (CLOSER_AT, "+", "Map closer", closer),
        (WIDER_AT, "−", "Map wider", wider),
    ]
    .map(|(angle, sign, said, step)| {
        let at = view.center + Vec2::angled(angle.0) * (DISC / 2.0 - 1.0);
        let spot = egui::Rect::from_center_size(at, Vec2::splat(2.0 * BUTTON));
        let button = ui.interact(spot, ui.id().with(said), Sense::click());
        button.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, step.is_some(), said)
        });
        let (fill, sign_ink) = match (step, button.hovered()) {
            (None, _) => (ink::HONEY2, ink::INK3),
            (Some(_), true) => (ink::LIGHT, ink::INK),
            (Some(_), false) => (ink::CREAM, ink::INK),
        };
        ui.painter()
            .circle(at, BUTTON, fill, Stroke::new(1.5, ink::FRAME));
        ui.painter().text(
            at,
            Align2::CENTER_CENTER,
            sign,
            Face::BookBold.id(13.0),
            sign_ink,
        );
        step.filter(|_| button.clicked())
    });
    pressed.into_iter().flatten().next()
}

fn wheeled(ui: &Ui, disc: &egui::Response, view: &MapView) -> Option<MinimapReachStep> {
    if !disc.contains_pointer() {
        return None;
    }
    let turned: f32 = ui.input(|input| {
        input
            .events
            .iter()
            .map(|event| match event {
                egui::Event::MouseWheel { unit, delta, .. } => match unit {
                    egui::MouseWheelUnit::Point => delta.y / POINTS_PER_NOTCH,
                    egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => delta.y,
                },
                _ => 0.0,
            })
            .sum()
    });
    let notches = ui.data_mut(|data| {
        let gathered = data.get_temp_mut_or_default::<f32>(disc.id);
        *gathered += turned;
        let notches = gathered.trunc();
        *gathered -= notches;
        notches as i64
    });
    let step = (i64::from(view.step.0) - notches).clamp(0, i64::from(view.last_step.0));
    (step != i64::from(view.step.0)).then_some(MinimapReachStep(step as u32))
}

fn lettered(ui: &mut Ui, name: &str, top: Pos2) {
    let galley =
        ui.painter()
            .layout_no_wrap(name.to_owned(), TextRole::Name.font_id(), Color32::WHITE);
    let rect = Align2::CENTER_TOP.anchor_size(top + Vec2::new(0.0, 3.0), galley.size());
    ui.interact(rect, ui.id().with("area"), Sense::hover())
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, name));
    dusk::lettering(
        ui.painter(),
        rect.min,
        galley,
        dusk::TEXT_SHADOW,
        TSTransform::IDENTITY,
    );
}
