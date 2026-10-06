use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::ui::{ComputedNode, UiGlobalTransform};
use bevy::window::PrimaryWindow;
use game::core::assets::AssetService;
use game::core::math::{Offset, Pos, Rect, Size};
use game::core::render::tile_to_window;
use game::core::tiling::{TilePos, Tiles};
use game::data;
use game::systems::actor::{self, Actor, Hitbox};
use game::systems::announcement::{Announcement, Announcements};
use game::systems::area::{self, AreaTag};
use game::systems::attention::{Attention, StatusBadges};
use game::systems::combat::{self, Attitude};
use game::systems::dialogue::{history, stage};
use game::systems::input::map::InputMap;
use game::systems::interact;
use game::systems::item::card::ItemCardWindow;
use game::systems::item::{self, DroppedItem, Inventory};
use game::systems::movement::Position;
use game::systems::npc::Npc;
use game::systems::player::{Owner, commands_locked, session};
use game::systems::prop::Prop;
use game::systems::quest::{QuestLog, QuestResult};
use game::systems::shop;
use game::systems::stat::{StatKind, Stats};
use serde::Serialize;

use crate::platform::expose_global_fn;

pub struct ProbePlugin;

impl Plugin for ProbePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, install).add_systems(Last, publish);
    }
}

#[derive(Serialize)]
struct Snapshot {
    view: Option<View>,
    area: Option<area::Id>,
    me: Option<Body>,
    viewpoint: Option<Body>,
    actors: Vec<Body>,
    props: Vec<Fixture>,
    items: Vec<GroundItem>,
    portals: Vec<Exit>,
    markers: Vec<Marker>,
    walkable: Vec<Pos<Tiles>>,
    area_size: Option<Size<Tiles>>,
    safe_zones: Vec<Rect<Tiles>>,
    ui: Vec<UiElement>,
    covered: Vec<Cover>,
    stage: Option<Stage>,
    announcement: Lane,
    history: bool,
    shop: Option<Shop>,
    item_card: Option<ItemCard>,
    bag: Vec<Stack>,
    quests: Quests,
}

#[derive(Serialize, Default)]
struct Quests {
    active: Vec<QuestEntry>,
    finished: Vec<FinishedEntry>,
    tracked: Vec<data::quest::Id>,
}

#[derive(Serialize)]
struct QuestEntry {
    quest: data::quest::Id,
    ready: bool,
    left: Option<f32>,
    progress: Vec<[u32; 2]>,
}

#[derive(Serialize)]
struct FinishedEntry {
    quest: data::quest::Id,
    result: QuestResult,
}

#[derive(Serialize, Clone, Copy)]
struct Cover {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Serialize)]
struct ItemCard {
    item: data::item::Id,
    rect: Cover,
}

#[derive(Serialize)]
struct Shop {
    shop: data::shop::Id,
    offers: Vec<Offer>,
    buyback: Vec<Stack>,
}

#[derive(Serialize)]
struct Offer {
    item: data::item::Id,
    count: u32,
    left: Option<u32>,
    refusal: Option<String>,
}

#[derive(Serialize)]
struct Stack {
    item: data::item::Id,
    count: u32,
}

#[derive(Serialize)]
struct Stage {
    node: data::dialogue::Id,
    with: Option<String>,
    line: usize,
    lines: usize,
    speaker: Option<String>,
    typing: bool,
    choices: Vec<StageChoice>,
    waiting: Option<data::dialogue::Id>,
}

#[derive(Serialize, Default)]
struct Lane {
    showing: Option<LaneLine>,
    next: Option<LaneLine>,
    missed: Vec<LaneLine>,
}

#[derive(Serialize)]
struct LaneLine {
    by: String,
    text: String,
}

impl LaneLine {
    fn of(announcement: &Announcement, inputs: &InputMap) -> LaneLine {
        LaneLine {
            by: announcement.by.name().to_owned(),
            text: announcement.text.words(inputs),
        }
    }
}

#[derive(Serialize)]
struct StageChoice {
    label: String,
    locked: bool,
    rect: Option<Cover>,
}

#[derive(Serialize, Clone, Copy)]
struct View {
    origin: [f32; 2],
    tile_size: [f32; 2],
}

#[derive(Serialize)]
struct Body {
    id: String,
    name: String,
    model: data::model::Id,
    flies: bool,
    player: bool,
    at: Pos<Tiles>,
    aim: Pos<Tiles>,
    hitbox: Rect<Tiles>,
    health: f32,
    max_health: f32,
    npc: Option<data::npc::Id>,
    role: Option<&'static str>,
    friendly: bool,
    locked: bool,
    marks: Vec<data::attention::Id>,
    badges: Vec<data::attention::Id>,
}

#[derive(Serialize)]
struct Fixture {
    id: String,
    prop: data::prop::Id,
    at: Pos<Tiles>,
    aim: Pos<Tiles>,
    hitbox: Rect<Tiles>,
    marks: Vec<data::attention::Id>,
}

#[derive(Serialize)]
struct GroundItem {
    item: data::item::Id,
    count: u32,
    at: Pos<Tiles>,
}

#[derive(Serialize)]
struct Marker {
    name: String,
    at: Pos<Tiles>,
}

#[derive(Serialize)]
struct Exit {
    name: String,
    to: area::Id,
    at: Pos<Tiles>,
}

#[derive(Serialize)]
struct UiElement {
    text: Option<String>,
    image: Option<String>,
    editable: bool,
    focused: bool,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

thread_local! {
    static REQUESTED: Cell<bool> = const { Cell::new(false) };
    static LATEST: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn install() {
    expose_global_fn("rift_probe", || {
        REQUESTED.set(true);
        LATEST.with_borrow(Clone::clone)
    });
}

// A snapshot scans the whole world every frame, so only a page that has asked for one pays for it.
fn publish(world: &mut World) {
    if REQUESTED.get() {
        let snapshot = serde_json::to_string(&snapshot(world)).expect("a serializable snapshot");
        LATEST.set(Some(snapshot));
    }
}

fn snapshot(world: &mut World) -> Snapshot {
    let me = session::my_character(world).map(|me| me.id());
    let viewpoint = world.resource::<session::Viewpoint>().0;
    let area = viewpoint
        .and_then(|seen| world.get::<AreaTag>(seen))
        .map(|tag| tag.area);
    let view = view(world);
    let rows = choice_rows(world);
    let input_map = world.resource::<InputMap>().clone();
    Snapshot {
        stage: stage::view(world).map(|view| Stage {
            node: view.node,
            with: view.with.map(|with| with.to_string()),
            line: view.line,
            lines: view.lines,
            speaker: view.speaker,
            typing: view.typing,
            choices: view
                .choices
                .into_iter()
                .enumerate()
                .map(|(index, choice)| StageChoice {
                    label: choice.label,
                    locked: choice.locked,
                    rect: rows.get(&index).copied(),
                })
                .collect(),
            waiting: view.waiting,
        }),
        announcement: viewpoint
            .and_then(|seen| world.get::<Announcements>(seen))
            .map(|lane| Lane {
                showing: lane
                    .showing
                    .as_ref()
                    .map(|shown| LaneLine::of(&shown.announcement, &input_map)),
                next: lane
                    .next
                    .as_ref()
                    .map(|next| LaneLine::of(&next.announcement, &input_map)),
                missed: lane
                    .missed
                    .iter()
                    .map(|missed| LaneLine::of(&missed.announcement, &input_map))
                    .collect(),
            })
            .unwrap_or_default(),
        history: history::is_open(world),
        item_card: world
            .query::<(&ItemCardWindow, &ComputedNode, &UiGlobalTransform)>()
            .iter(world)
            .next()
            .map(|(card, node, transform)| ItemCard {
                item: card.item,
                rect: rect_of(node, transform),
            }),
        shop: shop::window::view(world).map(|window| Shop {
            shop: window.shop,
            offers: window
                .shop
                .get()
                .sells
                .iter()
                .zip(&window.offers)
                .map(|(offer, view)| Offer {
                    item: offer.item,
                    count: offer.count,
                    left: view.left,
                    refusal: view.refusal.clone(),
                })
                .collect(),
            buyback: window
                .buyback
                .iter()
                .map(|sale| Stack {
                    item: sale.item,
                    count: sale.count,
                })
                .collect(),
        }),
        bag: viewpoint
            .and_then(|seen| world.get::<Inventory>(seen))
            .map(|inventory| {
                inventory
                    .slots
                    .iter()
                    .map(|stack| Stack {
                        item: stack.item,
                        count: stack.count,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        quests: viewpoint
            .and_then(|seen| quests(world, seen))
            .unwrap_or_default(),
        me: me.and_then(|me| body(world, me)),
        viewpoint: viewpoint.and_then(|seen| body(world, seen)),
        actors: actors(world, me, area),
        props: props(world, area),
        items: items(world, area),
        portals: portals(world, me),
        markers: markers(world, me),
        walkable: walkable(world, me, view),
        area_size: me.and_then(|me| area::of(world, me)).map(|area| area.size),
        safe_zones: me
            .and_then(|me| area::of(world, me))
            .map(|area| area.safe_zones.clone())
            .unwrap_or_default(),
        ui: ui(world),
        covered: covered(world),
        view,
        area,
    }
}

fn quests(world: &World, seen: Entity) -> Option<Quests> {
    let log = world.get::<QuestLog>(seen)?;
    let inventory = world.get::<Inventory>(seen);
    Some(Quests {
        active: log
            .active
            .iter()
            .map(|active| QuestEntry {
                quest: active.quest,
                ready: active.ready,
                left: active.left.map(|left| left.0),
                progress: active
                    .quest
                    .get()
                    .progress(active, inventory)
                    .iter()
                    .map(|progress| [progress.have, progress.need])
                    .collect(),
            })
            .collect(),
        finished: log
            .finished
            .iter()
            .map(|finished| FinishedEntry {
                quest: finished.quest,
                result: finished.result,
            })
            .collect(),
        tracked: log.tracked.clone(),
    })
}

fn view(world: &mut World) -> Option<View> {
    let origin = tile_to_window(world, Pos::new(0.0, 0.0))?;
    let unit = tile_to_window(world, Pos::new(1.0, 1.0))?;
    Some(View {
        origin: origin.to_array(),
        tile_size: (unit - origin).to_array(),
    })
}

fn body(world: &World, entity: Entity) -> Option<Body> {
    let viewer = world.resource::<session::Viewpoint>().0;
    let marks = viewer
        .and_then(|viewer| world.get::<Attention>(viewer))
        .map(|attention| attention.of(entity).to_vec())
        .unwrap_or_default();
    let locked = commands_locked(world, entity);
    let entity = world.get_entity(entity).ok()?;
    let npc = entity.get::<Npc>().map(|npc| npc.def);
    let at = entity.get::<Position>()?.pos;
    let hitbox = at.hitbox(entity.get::<Hitbox>()?.size);
    let model = entity.get::<Actor>()?.model;
    let flies = world
        .resource::<AssetService>()
        .resolve(model.get().sheet, actor::build_model)
        .airborne;
    let stat = |kind| entity.get::<Stats>().map_or(0.0, |stats| stats.get(kind));
    Some(Body {
        id: entity.id().to_string(),
        name: entity
            .get::<actor::Name>()
            .map_or_else(String::new, |name| name.name.clone()),
        model,
        flies,
        player: entity.contains::<Owner>(),
        at,
        aim: hitbox.center(),
        hitbox,
        health: stat(StatKind::Health),
        max_health: stat(StatKind::MaxHealth),
        npc,
        role: npc.and_then(|npc| npc.get().role),
        friendly: entity.get::<Attitude>() == Some(&Attitude::Friendly),
        locked,
        marks,
        badges: entity
            .get::<StatusBadges>()
            .map(|badges| badges.0.clone())
            .unwrap_or_default(),
    })
}

fn actors(world: &mut World, me: Option<Entity>, area: Option<area::Id>) -> Vec<Body> {
    let others: Vec<Entity> = world
        .query_filtered::<(Entity, &AreaTag), With<Actor>>()
        .iter(world)
        .filter(|(entity, tag)| Some(*entity) != me && area.is_none_or(|area| tag.area == area))
        .map(|(entity, _)| entity)
        .collect();
    let bodies: Vec<(Entity, Body)> = others
        .into_iter()
        .filter_map(|entity| body(world, entity).map(|body| (entity, body)))
        .collect();
    bodies
        .into_iter()
        .map(|(entity, body)| Body {
            aim: aim(world, entity, body.hitbox),
            ..body
        })
        .collect()
}

fn props(world: &mut World, area: Option<area::Id>) -> Vec<Fixture> {
    let viewer = world.resource::<session::Viewpoint>().0;
    let attention = viewer
        .and_then(|viewer| world.get::<Attention>(viewer))
        .cloned();
    let fixtures: Vec<(Entity, Fixture)> = world
        .query::<(Entity, &Prop, &Position, &Hitbox, &AreaTag)>()
        .iter(world)
        .filter(|(.., tag)| area.is_none_or(|area| tag.area == area))
        .map(|(entity, prop, at, hitbox, _)| {
            let hitbox = at.pos.hitbox(hitbox.size);
            let fixture = Fixture {
                id: entity.to_string(),
                prop: prop.def,
                at: at.pos,
                aim: hitbox.center(),
                hitbox,
                marks: attention
                    .as_ref()
                    .map(|attention| attention.of(entity).to_vec())
                    .unwrap_or_default(),
            };
            (entity, fixture)
        })
        .collect();
    fixtures
        .into_iter()
        .map(|(entity, fixture)| Fixture {
            aim: aim(world, entity, fixture.hitbox),
            ..fixture
        })
        .collect()
}

fn aim(world: &mut World, entity: Entity, hitbox: Rect<Tiles>) -> Pos<Tiles> {
    const STEPS: u8 = 5;
    let center = hitbox.center();
    let mut spots: Vec<Pos<Tiles>> = (0..STEPS)
        .flat_map(|x| (0..STEPS).map(move |y| (x, y)))
        .map(|(x, y)| {
            hitbox.origin
                + Offset::new(
                    hitbox.size.width * (f32::from(x) + 0.5) / f32::from(STEPS),
                    hitbox.size.height * (f32::from(y) + 0.5) / f32::from(STEPS),
                )
        })
        .collect();
    spots.sort_by(|a, b| a.distance_to(center).total_cmp(&b.distance_to(center)));
    std::iter::once(center)
        .chain(spots)
        .find(|&spot| clicked(world, spot) == Some(entity))
        .unwrap_or(center)
}

fn clicked(world: &mut World, spot: Pos<Tiles>) -> Option<Entity> {
    if let Some(enemy) = combat::enemy_at(world, spot) {
        return Some(enemy);
    }
    if item::pickable_at(world, spot).is_some() {
        return None;
    }
    interact::interactable_at(world, spot)
}

fn items(world: &mut World, area: Option<area::Id>) -> Vec<GroundItem> {
    world
        .query::<(&DroppedItem, &Position, &AreaTag)>()
        .iter(world)
        .filter(|(.., tag)| area.is_none_or(|area| tag.area == area))
        .map(|(dropped, at, _)| GroundItem {
            item: dropped.item,
            count: dropped.count,
            at: at.pos,
        })
        .collect()
}

fn portals(world: &World, me: Option<Entity>) -> Vec<Exit> {
    me.and_then(|me| area::of(world, me))
        .map(|area| {
            area.portals
                .iter()
                .map(|portal| Exit {
                    name: portal.name.clone(),
                    to: portal.dest_area,
                    at: portal.rect.center(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn markers(world: &World, me: Option<Entity>) -> Vec<Marker> {
    let Some(area) = me.and_then(|me| area::of(world, me)) else {
        return Vec::new();
    };
    let mut markers: Vec<Marker> = area
        .markers
        .iter()
        .map(|(name, marker)| Marker {
            name: name.clone(),
            at: marker.center(),
        })
        .collect();
    markers.sort_by(|a, b| a.name.cmp(&b.name));
    markers
}

fn walkable(world: &mut World, me: Option<Entity>, view: Option<View>) -> Vec<Pos<Tiles>> {
    let (Some(area), Some(view)) = (me.and_then(|me| area::of(world, me)), view) else {
        return Vec::new();
    };
    let Ok(window) = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
    else {
        return Vec::new();
    };
    let span = |axis: usize, extent: f32| {
        let first = (-view.origin[axis] / view.tile_size[axis]).ceil() as i32;
        let last = ((extent - view.origin[axis]) / view.tile_size[axis]).floor() as i32;
        first..=last
    };
    let (columns, rows) = (
        span(0, window.resolution.width()),
        span(1, window.resolution.height()),
    );
    let reachable = me
        .and_then(|me| world.get::<Position>(me))
        .and_then(|at| area.grid.component(at.pos));
    rows.flat_map(|y| columns.clone().map(move |x| Pos::new(x as f32, y as f32)))
        .filter(|&tile| reachable.is_some() && area.grid.component(tile) == reachable)
        .collect()
}

fn ui(world: &mut World) -> Vec<UiElement> {
    let spans: HashMap<Entity, String> = world
        .query::<(Entity, &TextSpan)>()
        .iter(world)
        .map(|(entity, span)| (entity, span.0.clone()))
        .collect();
    let focused = world.get_resource::<InputFocus>().and_then(InputFocus::get);
    let mut nodes = world.query::<(
        Entity,
        &ComputedNode,
        &UiGlobalTransform,
        &InheritedVisibility,
        Option<&Text>,
        Option<&ui::RichText>,
        Option<&Children>,
        Option<&ImageNode>,
        Option<&EditableText>,
    )>();
    let assets = world.resource::<AssetServer>();
    let catalog = world.resource::<ui::InputCatalog>();
    nodes
        .iter(world)
        .filter(|(_, node, _, visibility, ..)| visibility.get() && node.size().min_element() > 0.0)
        .filter_map(
            |(entity, node, transform, _, text, rich, children, image, field)| {
                let text = text
                    .map(|text| {
                        let spanned = children
                            .into_iter()
                            .flatten()
                            .filter_map(|child| spans.get(child).map(String::as_str));
                        std::iter::once(text.0.as_str()).chain(spanned).collect()
                    })
                    .or_else(|| rich.map(|rich| rich.plain(catalog)))
                    .or_else(|| field.map(|field| field.value().to_string()))
                    .filter(|text: &String| !text.is_empty());
                let image = image
                    .and_then(|image| assets.get_path(image.image.id()))
                    .map(|path| path.to_string());
                if text.is_none() && image.is_none() && field.is_none() {
                    return None;
                }
                let scale = node.inverse_scale_factor();
                let size = node.size() * scale;
                let min = transform.translation * scale - size / 2.0;
                Some(UiElement {
                    text,
                    image,
                    editable: field.is_some(),
                    focused: focused == Some(entity),
                    x: min.x,
                    y: min.y,
                    width: size.x,
                    height: size.y,
                })
            },
        )
        .collect()
}

fn choice_rows(world: &mut World) -> HashMap<usize, Cover> {
    world
        .query::<(&ui::ChoiceRow, &ComputedNode, &UiGlobalTransform)>()
        .iter(world)
        .map(|(row, node, transform)| (row.index, rect_of(node, transform)))
        .collect()
}

fn rect_of(node: &ComputedNode, transform: &UiGlobalTransform) -> Cover {
    let scale = node.inverse_scale_factor();
    let size = node.size() * scale;
    let min = transform.translation * scale - size / 2.0;
    Cover {
        x: min.x,
        y: min.y,
        width: size.x,
        height: size.y,
    }
}

fn covered(world: &mut World) -> Vec<Cover> {
    let blocking: HashMap<Entity, Cover> = world
        .query::<(
            Entity,
            &ComputedNode,
            &UiGlobalTransform,
            &InheritedVisibility,
            Option<&Pickable>,
        )>()
        .iter(world)
        .filter(|(_, node, _, visibility, pickable)| {
            visibility.get()
                && node.size().min_element() > 0.0
                && pickable.is_none_or(|pickable| pickable.is_hoverable)
        })
        .map(|(entity, node, transform, ..)| (entity, rect_of(node, transform)))
        .collect();
    let ancestors = |entity: Entity| {
        std::iter::successors(world.get::<ChildOf>(entity).map(ChildOf::parent), |&up| {
            world.get::<ChildOf>(up).map(ChildOf::parent)
        })
    };
    blocking
        .iter()
        .filter(|&(&entity, _)| ancestors(entity).all(|up| !blocking.contains_key(&up)))
        .map(|(_, rect)| *rect)
        .collect()
}
