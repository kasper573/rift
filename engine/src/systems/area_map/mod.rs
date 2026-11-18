mod ground;
pub mod minimap;
pub mod tab;

use std::f32::consts::TAU;

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::emath::TSTransform;
use ui::egui::epaint::{Mesh, Vertex};
use ui::egui::{self, Align2, Color32, Pos2, Sense, Shape, Stroke, Ui, Vec2};
use ui::face::{Face, TextRole};
use ui::glyph::{self, Mark};
use ui::{dusk, float, ink};

use crate::core::content::{Content, ImageRef, RowCheck};
use crate::core::gui::{self, GuiImage};
use crate::core::math::{Pos, Rect};
use crate::core::render::CameraRig;
use crate::core::render::world::opacity::Opacity;
use crate::core::script::Scripts;
use crate::core::text::LineText;
use crate::core::text::{self, TextLook};
use crate::core::units::{MeterSize, Meters, Radians};
use crate::systems::actor::{Action, Actor, Name};
use crate::systems::area::{self, AreaId, AreaTag};
use crate::systems::attention::{self, AttentionId};
use crate::systems::combat::Attitude;
use crate::systems::hud::menu::{self, MenuTabName};
use crate::systems::input::map::{self, InputAction};
use crate::systems::item::{self, DroppedItem, Reservation};
use crate::systems::mention::Mention;
use crate::systems::messages::{self, messages};
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::npc::Npc;
use crate::systems::party;
use crate::systems::player::{PlayedBy, session};
use crate::systems::screen::ClientScreen;
use crate::systems::{GameSettings, visibility};

pub struct AreaMapPlugin;

impl Plugin for AreaMapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ground::MapGround>()
            .init_resource::<tab::MapTabLook>()
            .add_systems(
                Update,
                (tab::rest, picture_the_ground)
                    .chain()
                    .run_if(in_state(ClientScreen::Area)),
            );
    }
}

/// What the map is opened to show in place of where its viewer stands.
#[derive(Clone, Debug, PartialEq)]
pub enum MapSight {
    /// An area, from its middle.
    Area(AreaId),
    Pin(MapPin),
}

/// A spot someone pointed out, marked on the map under their name.
#[derive(Clone, Debug, PartialEq)]
pub struct MapPin {
    pub area: AreaId,
    pub at: Pos<Meters>,
    pub name: String,
}

impl MapSight {
    fn spot(&self, content: &Content) -> Option<(AreaId, Pos<Meters>)> {
        match self {
            MapSight::Area(area) => Some((*area, area::loaded(content, *area)?.center())),
            MapSight::Pin(pin) => Some((pin.area, pin.at)),
        }
    }

    fn pin(&self) -> Option<&MapPin> {
        match self {
            MapSight::Area(_) => None,
            MapSight::Pin(pin) => Some(pin),
        }
    }
}

/// Opens the map on `sight`, which it shows for as long as it stays open: closing the map or
/// leaving it for another tab lets go of the sight.
pub fn open_on(world: &mut World, sight: MapSight) {
    world.resource_mut::<tab::MapTabLook>().sight = Some(sight);
    menu::open_on(world, MenuTabName::Map);
}

/// How far a map reaches at each step, closest first.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MapReaches(pub Vec<Meters>);

impl MapReaches {
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

    pub fn step_reaching(&self, sight: Meters) -> MapReachStep {
        let step = self
            .0
            .iter()
            .position(|reach| reach.0 >= sight.0)
            .unwrap_or(self.0.len().saturating_sub(1));
        MapReachStep(step as u32)
    }

    fn last_step(&self) -> MapReachStep {
        MapReachStep(self.0.len().saturating_sub(1) as u32)
    }

    fn at(&self, step: MapReachStep) -> Meters {
        let step = step.min(self.last_step());
        self.0.get(step.0 as usize).copied().unwrap_or(Meters(1.0))
    }

    fn widest(&self) -> Meters {
        self.at(self.last_step())
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct MapReachStep(pub u32);

/// Each kind is drawn as a shape of its own, so none is told from another by colour alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapMarkKind {
    Townsperson,
    Character,
    /// A character of the viewer's party.
    PartyMember,
    WayOut,
    /// An item on the ground whose rarity marks it, drawn as the rarity's icon.
    Loot,
}

#[derive(Clone, Debug)]
pub struct MapMark {
    pub kind: MapMarkKind,
    pub name: String,
    /// The thing the mark's name is, for a mark that is named after one the interface mentions.
    pub mention: Option<Mention>,
    pub at: Pos<Meters>,
    pub attention: Vec<AttentionId>,
    pub opacity: f32,
    /// The picture the mark is drawn as, for a kind that has no shape of its own.
    pub icon: Option<ImageRef>,
}

pub fn marks(world: &mut World, viewer: Entity) -> Vec<MapMark> {
    let Some(area) = world.get::<AreaTag>(viewer).map(|tag| tag.area) else {
        return Vec::new();
    };
    let Some(eye) = stands_at(world, viewer) else {
        return Vec::new();
    };
    let sight = visibility::sight(world);
    let content = world.resource::<Content>().clone();
    let party = party::companions(world, viewer);
    let seen: Vec<(Entity, MapMark)> = world
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
                (Some(npc), Some(Attitude::Friendly), _) => {
                    (MapMarkKind::Townsperson, npc.def.get(&content).name.clone())
                }
                (None, _, Some(by)) => (
                    if party.contains(&by.client) {
                        MapMarkKind::PartyMember
                    } else {
                        MapMarkKind::Character
                    },
                    name.map(|name| name.name.clone()).unwrap_or_default(),
                ),
                _ => return None,
            };
            let mark = MapMark {
                kind,
                name,
                mention: None,
                at,
                attention: Vec::new(),
                opacity: opacity.map_or(1.0, |opacity| opacity.0),
                icon: None,
            };
            Some((entity, mark))
        })
        .collect();
    let people: Vec<MapMark> = seen
        .into_iter()
        .map(|(entity, mark)| MapMark {
            attention: attention::plate_marks(world, Some(viewer), entity),
            ..mark
        })
        .collect();
    let scripts = world.resource::<Scripts>();
    let ways: Vec<MapMark> = area::warp::warp_objects(&content, scripts, area)
        .into_iter()
        .filter(|object| object.to.area != area)
        .map(|object| MapMark {
            kind: MapMarkKind::WayOut,
            name: object.to.area.get(&content).name.clone(),
            mention: Some(Mention::Area(object.to.area)),
            at: object.shape.center(),
            attention: Vec::new(),
            opacity: 1.0,
            icon: None,
        })
        .collect();
    let taker = item::DropTaker::of(world, viewer);
    let loot: Vec<MapMark> = world
        .query::<(
            &AreaTag,
            &Position,
            &DroppedItem,
            Option<&Reservation>,
            Option<&Opacity>,
        )>()
        .iter(world)
        .filter(|(tag, at, _, reservation, _)| {
            tag.area == area
                && eye.distance_to(at.pos) <= sight.0
                && item::takeable(*reservation, &taker)
        })
        .filter_map(|(_, at, dropped, _, opacity)| {
            let def = dropped.item.get(&content);
            let rarity = def.rarity.get(&content);
            rarity.dropped.on_minimap.then(|| MapMark {
                kind: MapMarkKind::Loot,
                name: def.name.clone(),
                mention: Some(Mention::Item(dropped.item)),
                at: at.pos,
                attention: Vec::new(),
                opacity: opacity.map_or(1.0, |opacity| opacity.0),
                icon: rarity.icon.clone(),
            })
        })
        .collect();
    people.into_iter().chain(ways).chain(loot).collect()
}

struct MapView {
    me: Entity,
    area: AreaId,
    at: Pos<Meters>,
    facing: Radians,
    reach: Meters,
    step: MapReachStep,
    last_step: MapReachStep,
    sight: Meters,
    yaw: f32,
    window: MapWindow,
    content: Content,
    /// Whether the map shows an area its viewer is not in.
    away: bool,
}

#[derive(Clone, Copy)]
enum MapWindow {
    Disc {
        center: Pos2,
        turning: bool,
    },
    Sheet {
        rect: egui::Rect,
        middle: Pos<Meters>,
    },
}

impl MapView {
    fn of(
        world: &World,
        step: impl FnOnce(&MapReaches, Meters) -> MapReachStep,
        window: impl FnOnce(Pos<Meters>) -> MapWindow,
    ) -> Option<MapView> {
        let me = session::me(world)?;
        let settings = world.resource::<GameSettings>();
        let reaches = &settings.map_reaches;
        let step = step(reaches, settings.sight).min(reaches.last_step());
        let at = stands_at(world, me)?;
        Some(MapView {
            me,
            area: world.get::<AreaTag>(me)?.area,
            at,
            facing: world.get::<Actor>(me).map_or(Radians(0.0), |me| me.facing),
            reach: reaches.at(step),
            step,
            last_step: reaches.last_step(),
            sight: settings.sight,
            yaw: world.get_resource::<CameraRig>()?.yaw().0,
            window: window(at),
            content: world.resource::<Content>().clone(),
            away: false,
        })
    }

    fn messages(&self) -> &MapMessages {
        &messages::of(&self.content).map
    }

    fn key(&self) -> &'static str {
        match self.window {
            MapWindow::Disc { .. } => "minimap",
            MapWindow::Sheet { .. } => "map",
        }
    }

    fn turns(&self) -> bool {
        matches!(self.window, MapWindow::Disc { turning: true, .. })
    }

    fn ground_turn(&self) -> f32 {
        if self.turns() { self.yaw } else { 0.0 }
    }

    fn center(&self) -> Pos2 {
        match self.window {
            MapWindow::Disc { center, .. } => center,
            MapWindow::Sheet { rect, .. } => rect.center(),
        }
    }

    fn middle(&self) -> Pos<Meters> {
        match self.window {
            MapWindow::Disc { .. } => self.at,
            MapWindow::Sheet { middle, .. } => middle,
        }
    }

    // Both maps draw the ground at one scale, so a reach is as far across either.
    fn per_meter(&self) -> f32 {
        minimap::GROUND / self.reach.0
    }

    fn point(&self, at: Pos<Meters>) -> Pos2 {
        let middle = self.middle();
        let from_middle = Vec2::new(at.x - middle.x, at.y - middle.y) * self.per_meter();
        self.center() + turned(from_middle, self.ground_turn())
    }

    fn ground(&self, point: Pos2) -> Pos<Meters> {
        let middle = self.middle();
        let from_middle = turned(point - self.center(), -self.ground_turn()) / self.per_meter();
        Pos::new(middle.x + from_middle.x, middle.y + from_middle.y)
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

    fn outline(&self) -> Vec<Pos2> {
        match self.window {
            MapWindow::Disc { center, .. } => round(center, minimap::GROUND),
            MapWindow::Sheet { rect, .. } => vec![
                rect.left_top(),
                rect.right_top(),
                rect.right_bottom(),
                rect.left_bottom(),
            ],
        }
    }

    fn holds(&self, point: Pos2) -> bool {
        match self.window {
            MapWindow::Disc { center, .. } => (point - center).length() <= minimap::GROUND - 2.0,
            MapWindow::Sheet { rect, .. } => rect.shrink(PERSON).contains(point),
        }
    }

    fn holds_ring(&self, radius: f32) -> bool {
        match self.window {
            MapWindow::Disc { .. } => radius <= minimap::GROUND - 2.0,
            MapWindow::Sheet { .. } => true,
        }
    }
}

fn chart(ui: &mut Ui, world: &mut World, view: &MapView, pin: Option<&MapPin>) {
    ground_under(ui, world, view);
    if !view.away {
        if !view.turns() {
            wedge(ui, view);
        }
        sight_ring(ui, view);
        let marks = marks(world, view.me);
        marked(ui, world, view, &marks);
        myself(ui, view);
    }
    if let Some(pin) = pin {
        pinned(ui, view, pin);
    }
}

fn pinned(ui: &mut Ui, view: &MapView, pin: &MapPin) {
    let tip = view.point(pin.at);
    if !view.holds(tip) {
        return;
    }
    let head = tip - Vec2::new(0.0, PIN_TALL);
    let edge = Stroke::new(1.5, EDGE);
    let shank = vec![
        tip,
        head + Vec2::new(-PIN_HEAD * 0.8, PIN_HEAD * 0.5),
        head + Vec2::new(PIN_HEAD * 0.8, PIN_HEAD * 0.5),
    ];
    ui.painter().add(Shape::convex_polygon(shank, PIN, edge));
    ui.painter().circle(head, PIN_HEAD, PIN, edge);
    ui.painter().circle_filled(head, PIN_HEAD * 0.35, EDGE);
    let galley =
        ui.painter()
            .layout_no_wrap(pin.name.clone(), TextRole::Name.font_id(), Color32::WHITE);
    let name =
        Align2::LEFT_CENTER.anchor_size(head + Vec2::new(PIN_HEAD + 5.0, 0.0), galley.size());
    dusk::lettering(
        ui.painter(),
        name.min,
        galley,
        dusk::TEXT_SHADOW,
        TSTransform::IDENTITY,
    );
    let spot = egui::Rect::from_center_size(head, Vec2::splat(2.0 * HOVER_REACH)).union(name);
    let said = view.messages().pinned_by(view.key(), &pin.name);
    ui.interact(spot, ui.id().with("pin"), Sense::hover())
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, said.clone()));
}

const ROUND: usize = 72;
const BEAD: f32 = 9.0;
const BUTTON: f32 = 8.5;
/// How large a round button's sign is drawn.
const SIGN: f32 = 11.0;
const WEDGE_HALF: f32 = 0.5;
const WEDGE_REACH: f32 = 0.62;
const PERSON: f32 = 4.5;
const LOOT: f32 = 12.0;
const DIAMOND: f32 = 5.5;
const DOOR: Vec2 = Vec2::new(9.0, 10.0);
const ATTENTION: f32 = 14.0;
const HOVER_REACH: f32 = 9.0;
const PIN_HEAD: f32 = 6.0;
const PIN_TALL: f32 = 15.0;

const BLANK: Color32 = Color32::from_rgb(0xD9, 0xCB, 0xA3);
const NO_GROUND: Color32 = Color32::from_rgb(0x4B, 0x43, 0x3A);
const TOWNSPERSON: Color32 = Color32::from_rgb(0x6F, 0xB8, 0x74);
const CHARACTER: Color32 = Color32::from_rgb(0x5B, 0xA6, 0xE6);
const PARTY_MEMBER: Color32 = Color32::from_rgb(0xE6, 0xC2, 0x5B);
const PIN: Color32 = Color32::from_rgb(0xD9, 0x4F, 0x4F);
const EDGE: Color32 = ink::FRAME;

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
    let widest = world.resource::<GameSettings>().map_reaches.widest().0;
    let about_me = Rect::new(at, default()).inflate(widest, widest);
    let on_the_tab = world.resource::<tab::MapTabLook>().shows();
    let shown: Vec<(AreaId, Rect<Meters>)> = std::iter::once((area, about_me))
        .chain(on_the_tab)
        .collect();
    ground::keep(world, &shown);
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
    let disc = view.outline();
    painter.add(Shape::convex_polygon(disc.clone(), NO_GROUND, Stroke::NONE));
    let content = world.resource::<Content>();
    if let Some(area) = area::loaded(content, view.area) {
        let held = within(&disc, corners(view, area.size.bounds()));
        if held.len() >= 3 {
            painter.add(Shape::convex_polygon(held, BLANK, Stroke::NONE));
        }
    }
    for picture in world.resource::<ground::MapGround>().pictures(view.area) {
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
    let reach = minimap::GROUND * WEDGE_REACH;
    let steps = 12;
    let from = view.point(view.at);
    let arc = (0..=steps).map(|step| {
        let angle = looks - WEDGE_HALF + 2.0 * WEDGE_HALF * step as f32 / steps as f32;
        from + Vec2::angled(angle) * reach
    });
    let shape: Vec<Pos2> = std::iter::once(from).chain(arc).collect();
    ui.painter().add(Shape::convex_polygon(
        shape,
        Color32::from_white_alpha(56),
        Stroke::new(1.0, Color32::from_white_alpha(150)),
    ));
}

fn sight_ring(ui: &Ui, view: &MapView) {
    let radius = view.sight.0 * view.per_meter();
    if !view.holds_ring(radius) {
        return;
    }
    let center = view.point(view.at);
    let dashes = ((TAU * radius / 9.0).round() as usize).max(8);
    let stroke = Stroke::new(1.0, Color32::from_white_alpha(190));
    for dash in 0..dashes {
        let from = TAU * dash as f32 / dashes as f32;
        let to = from + TAU / dashes as f32 * 0.55;
        ui.painter().line_segment(
            [
                center + Vec2::angled(from) * radius,
                center + Vec2::angled(to) * radius,
            ],
            stroke,
        );
    }
}

fn marked(ui: &mut Ui, world: &mut World, view: &MapView, marks: &[MapMark]) {
    let content = world.resource::<Content>().clone();
    let pointer = ui.input(|input| input.pointer.hover_pos());
    let on_disc: Vec<(usize, &MapMark, Pos2)> = marks
        .iter()
        .enumerate()
        .map(|(index, mark)| (index, mark, view.point(mark.at)))
        .filter(|(_, _, at)| view.holds(*at))
        .collect();
    let mut under = Vec::new();
    let mut topmost = None;
    // The more telling a kind, the later it is drawn, so it lies on top.
    for kind in [
        MapMarkKind::WayOut,
        MapMarkKind::Loot,
        MapMarkKind::Townsperson,
        MapMarkKind::Character,
        MapMarkKind::PartyMember,
    ] {
        for &(index, mark, at) in on_disc.iter().filter(|(_, mark, _)| mark.kind == kind) {
            let fill = |color: Color32| color.gamma_multiply(mark.opacity);
            let edge = Stroke::new(1.5, fill(EDGE));
            let said = match kind {
                MapMarkKind::WayOut => {
                    ui.painter()
                        .add(Shape::convex_polygon(door(at), fill(ink::LIGHT), edge));
                    view.messages().way_out(view.key(), &mark.name)
                }
                MapMarkKind::Townsperson => {
                    ui.painter().circle(at, PERSON, fill(TOWNSPERSON), edge);
                    view.messages().named_on(view.key(), &mark.name)
                }
                MapMarkKind::Loot => {
                    if let Some(icon) = mark.icon.as_ref().and_then(|icon| gui::image(world, icon))
                    {
                        ui.painter().image(
                            icon.texture,
                            egui::Rect::from_center_size(at, Vec2::splat(LOOT)),
                            egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            fill(Color32::WHITE),
                        );
                    }
                    view.messages().named_on(view.key(), &mark.name)
                }
                MapMarkKind::Character | MapMarkKind::PartyMember => {
                    let diamond = [Vec2::UP, Vec2::RIGHT, Vec2::DOWN, Vec2::LEFT]
                        .map(|corner| at + corner * DIAMOND)
                        .to_vec();
                    let color = if kind == MapMarkKind::PartyMember {
                        PARTY_MEMBER
                    } else {
                        CHARACTER
                    };
                    ui.painter()
                        .add(Shape::convex_polygon(diamond, fill(color), edge));
                    if kind == MapMarkKind::PartyMember {
                        ui.painter().circle_stroke(at, DIAMOND * 1.5, edge);
                    }
                    if kind == MapMarkKind::PartyMember {
                        view.messages().party_member_on(view.key(), &mark.name)
                    } else {
                        view.messages().player_on(view.key(), &mark.name)
                    }
                }
            };
            let told = match (kind, mark.mention) {
                (MapMarkKind::WayOut, Some(place)) => view.messages().way_to(place.span(&content)),
                (MapMarkKind::Loot, Some(item)) => item.line(&content),
                (MapMarkKind::Character, _) => {
                    LineText::plain(view.messages().player_mark(&mark.name))
                }
                (MapMarkKind::PartyMember, _) => {
                    LineText::plain(view.messages().party_member_mark(&mark.name))
                }
                _ => LineText::plain(mark.name.clone()),
            };
            let spot = egui::Rect::from_center_size(at, Vec2::splat(2.0 * HOVER_REACH));
            let node = ui.interact(spot, ui.id().with(("mark", index)), Sense::hover());
            let label = said.clone();
            node.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label.clone())
            });
            if pointer.is_some_and(|pointer| spot.contains(pointer)) {
                under.push(told);
                topmost = Some(node);
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
    if let Some(topmost) = topmost {
        float::tooltip(&topmost, |ui| {
            let look = TextLook::role(TextRole::Tooltip, ui.visuals().text_color());
            for line in &under {
                text::show(ui, world, &line.styled(&content), look, None);
            }
        });
    }
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
    let at = view.point(view.at);
    arrow(ui, at, view.faces(), 8.0, Color32::WHITE);
    let spot = egui::Rect::from_center_size(at, Vec2::splat(16.0));
    let said = view.messages().you_here(view.key());
    ui.interact(spot, ui.id().with("me"), Sense::hover())
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, said.clone()));
}

fn north_bead(ui: &mut Ui, world: &mut World, at: Pos2) {
    let content = world.resource::<Content>().clone();
    let words = &messages::of(&content).map;
    let spot = egui::Rect::from_center_size(at, Vec2::splat(2.0 * BEAD));
    let bead = ui
        .interact(spot, ui.id().with("north"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    bead.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, words.face_north())
    });
    let rim = if bead.hovered() {
        ink::GOLD2
    } else {
        ink::GOLD
    };
    ui.painter()
        .circle(at, BEAD, ink::FRAME, Stroke::new(1.5, rim));
    glyph::centered(ui.painter(), at, "N", Face::BookBold.id(12.0), ink::GOLD2);
    float::tooltip(&bead, |ui| {
        ui.label(TextRole::Tooltip.rich(words.north_tip()));
    });
    if bead.clicked()
        && let Some(mut rig) = world.get_resource_mut::<CameraRig>()
    {
        rig.face(Radians(0.0));
    }
}

fn reach_buttons(ui: &mut Ui, view: &MapView, places: [Pos2; 2]) -> Option<MapReachStep> {
    let closer = (view.step.0 > 0).then(|| MapReachStep(view.step.0 - 1));
    let wider = (view.step < view.last_step).then(|| MapReachStep(view.step.0 + 1));
    let pressed = [
        (
            "closer",
            Mark::Plus,
            view.messages().closer(view.key()),
            closer,
        ),
        (
            "wider",
            Mark::Minus,
            view.messages().wider(view.key()),
            wider,
        ),
    ]
    .into_iter()
    .zip(places)
    .map(|((key, sign, said, step), at)| {
        let button = round_button(ui, at, key, &said, step.is_some());
        let spot = egui::Rect::from_center_size(at, Vec2::splat(SIGN));
        sign.paint(ui.painter(), spot, button.ink);
        step.filter(|_| button.response.clicked())
    })
    .collect::<Vec<_>>();
    pressed.into_iter().flatten().next()
}

struct RoundButton {
    response: egui::Response,
    ink: Color32,
}

fn round_button(
    ui: &mut Ui,
    at: Pos2,
    key: &'static str,
    said: &str,
    enabled: bool,
) -> RoundButton {
    let spot = egui::Rect::from_center_size(at, Vec2::splat(2.0 * BUTTON));
    let response = ui.interact(spot, ui.id().with(key), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, said));
    let (fill, ink) = match (enabled, response.hovered()) {
        (false, _) => (ink::HONEY2, ink::INK3),
        (true, true) => (ink::LIGHT, ink::INK),
        (true, false) => (ink::CREAM, ink::INK),
    };
    ui.painter()
        .circle(at, BUTTON, fill, Stroke::new(1.5, ink::FRAME));
    RoundButton { response, ink }
}

fn wheeled(ui: &Ui, world: &World, chart: &egui::Response, view: &MapView) -> Option<MapReachStep> {
    if !chart.contains_pointer() {
        return None;
    }
    let zooms = [InputAction::ZoomIn, InputAction::ZoomOut];
    if zooms.iter().any(|zoom| map::takes_wheel(world, *zoom)) {
        // Whatever scrolls around the map stays put while the wheel steps its reach.
        ui.input_mut(|input| input.smooth_scroll_delta = Vec2::ZERO);
    }
    let [closer, wider] = zooms.map(|zoom| i64::from(map::wheeled(world, zoom)));
    let step = (i64::from(view.step.0) - closer + wider).clamp(0, i64::from(view.last_step.0));
    (step != i64::from(view.step.0)).then_some(MapReachStep(step as u32))
}

fn lettered(ui: &mut Ui, name: &str, top: Pos2) -> egui::Rect {
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
    rect
}

messages! {
    /// What the game says on its map.
    pub struct MapMessages {
        /// What a pin on the map is announced as on the minimap or the map, `{view}` being `minimap` or `map` and `{name}` who pinned it.
        pinned_by(view: Text, name: Text),
        /// What the player's own arrow is announced as, `{view}` being `minimap` or `map`.
        you_here(view: Text),
        /// What a way out is announced as, `{view}` being `minimap` or `map` and `{place}` where it leads.
        way_out(view: Text, place: Text),
        /// What a way out says in its tooltip, `{place}` being where it leads.
        way_to[place: Line],
        /// What another player is announced as, `{view}` being `minimap` or `map` and `{name}` their name.
        player_on(view: Text, name: Text),
        /// What another player says in their tooltip, `{name}` being their name.
        player_mark(name: Text),
        /// What a party member is announced as, `{view}` being `minimap` or `map` and `{name}` their name.
        party_member_on(view: Text, name: Text),
        /// What a party member says in their tooltip, `{name}` being their name.
        party_member_mark(name: Text),
        /// What a townsperson or dropped item is announced as, `{view}` being `minimap` or `map` and `{name}` its name.
        named_on(view: Text, name: Text),
        /// What the north bead on the map is announced as.
        face_north,
        /// The tooltip of the north bead on the map.
        north_tip,
        /// What the button that brings the map in closer is announced as, `{view}` being `minimap` or `map`.
        closer(view: Text),
        /// What the button that widens the map is announced as, `{view}` being `minimap` or `map`.
        wider(view: Text),
        /// What the screen reader reads of the minimap, `{area}` being the area and `{reach}` how many meters it reaches.
        minimap_of(area: Text, reach: Number),
        /// What the screen reader reads of the map, `{area}` being the area and `{reach}` how many meters it reaches.
        map_of(area: Text, reach: Number),
        /// The button and tooltip that bring the map back to where the player stands.
        centre_on_you,
        /// What a warp says when it finds nobody to move.
        nobody_to_warp,
        /// What a warp back to the spawn point says when the player has none.
        no_spawn_point,
        /// What a warp says when it would take a non-player out of their area.
        npcs_stay,
        /// What a warp says when it would take someone into an instanced area.
        no_following_into_instance,
        /// What a warp to another actor says when that actor can't be found.
        nobody_to_warp_to,
        /// What spawning an object says when the spot can't hold it.
        nothing_can_stand,
    }
}
