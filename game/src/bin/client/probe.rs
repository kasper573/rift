use std::cell::{Cell, RefCell};

use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::ui::{ComputedNode, UiGlobalTransform};
use bevy::window::PrimaryWindow;
use game::core::math::Pos;
use game::core::render::tile_to_window;
use game::core::tiling::{TilePos, Tiles};
use game::data;
use game::systems::actor::{self, Actor, Hitbox};
use game::systems::area::{self, AreaTag};
use game::systems::item::DroppedItem;
use game::systems::movement::Position;
use game::systems::player::{Owner, session};
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
    actors: Vec<Body>,
    items: Vec<GroundItem>,
    portals: Vec<Exit>,
    walkable: Vec<Pos<Tiles>>,
    ui: Vec<UiElement>,
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
    player: bool,
    at: Pos<Tiles>,
    aim: Pos<Tiles>,
    health: f32,
    max_health: f32,
}

#[derive(Serialize)]
struct GroundItem {
    item: data::item::Id,
    count: u32,
    at: Pos<Tiles>,
}

#[derive(Serialize)]
struct Exit {
    to: area::Id,
    at: Pos<Tiles>,
}

#[derive(Serialize)]
struct UiElement {
    text: Option<String>,
    image: Option<String>,
    editable: bool,
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
    let area = session::my_viewpoint(world)
        .and_then(|viewpoint| viewpoint.get::<AreaTag>())
        .map(|tag| tag.area);
    let view = view(world);
    Snapshot {
        me: me.and_then(|me| body(world, me)),
        actors: actors(world, me, area),
        items: items(world, area),
        portals: portals(world, me),
        walkable: walkable(world, me, view),
        ui: ui(world),
        view,
        area,
    }
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
    let entity = world.get_entity(entity).ok()?;
    let at = entity.get::<Position>()?.pos;
    let hitbox = entity.get::<Hitbox>()?;
    let model = entity.get::<Actor>()?.model;
    let stat = |kind| entity.get::<Stats>().map_or(0.0, |stats| stats.get(kind));
    Some(Body {
        id: entity.id().to_string(),
        name: entity
            .get::<actor::Name>()
            .map_or_else(String::new, |name| name.name.clone()),
        model,
        player: entity.contains::<Owner>(),
        at,
        aim: at.hitbox(hitbox.size).center(),
        health: stat(StatKind::Health),
        max_health: stat(StatKind::MaxHealth),
    })
}

fn actors(world: &mut World, me: Option<Entity>, area: Option<area::Id>) -> Vec<Body> {
    let others: Vec<Entity> = world
        .query_filtered::<(Entity, &AreaTag), With<Actor>>()
        .iter(world)
        .filter(|(entity, tag)| Some(*entity) != me && area.is_none_or(|area| tag.area == area))
        .map(|(entity, _)| entity)
        .collect();
    others
        .into_iter()
        .filter_map(|entity| body(world, entity))
        .collect()
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
                    to: portal.dest_area,
                    at: portal.rect.center(),
                })
                .collect()
        })
        .unwrap_or_default()
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
    rows.flat_map(|y| columns.clone().map(move |x| Pos::new(x as f32, y as f32)))
        .filter(|&tile| area.grid.walkable(tile))
        .collect()
}

fn ui(world: &mut World) -> Vec<UiElement> {
    let mut nodes = world.query::<(
        &ComputedNode,
        &UiGlobalTransform,
        &InheritedVisibility,
        Option<&Text>,
        Option<&ImageNode>,
        Option<&EditableText>,
    )>();
    let assets = world.resource::<AssetServer>();
    nodes
        .iter(world)
        .filter(|(node, _, visibility, ..)| visibility.get() && node.size().min_element() > 0.0)
        .filter_map(|(node, transform, _, text, image, field)| {
            let text = text
                .map(|text| text.0.clone())
                .or_else(|| field.map(|field| field.value().to_string()))
                .filter(|text| !text.is_empty());
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
                x: min.x,
                y: min.y,
                width: size.x,
                height: size.y,
            })
        })
        .collect()
}
