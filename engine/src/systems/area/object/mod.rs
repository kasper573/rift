use std::borrow::Cow;

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};

use crate::core::content::Content;
use crate::core::math::{Offset, Pos, Rect, Size};
use crate::core::units::{Cell, CellPos, Meters, TilePos};
use crate::systems::WorldArea;
use crate::systems::actor::Name as ActorName;
use crate::systems::area::PropId;
use crate::systems::area::{self, AreaId, AreaTag};
use crate::systems::interact::Interactive;
use crate::systems::movement::Position;
use crate::systems::npc::Npc;
use crate::systems::player::PlayedBy;
use crate::systems::script::condition::CompiledCondition;
use crate::systems::script::{
    self, Condition, EventKind, Owner, OwnerRow, Place, RiftEvent, RiftInstances, RunnerKind,
    ScriptModule,
};
use crate::systems::visibility::Presence;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate_once::<AreaObject>();
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct ObjectDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("x-name" = true))]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub shape: ObjectShape,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<PropId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown: Option<Condition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scripts: Vec<ScriptModule>,
}

impl ObjectDef {
    pub fn display_name(&self, index: u32) -> String {
        self.name
            .clone()
            .or_else(|| self.title.clone())
            .unwrap_or_else(|| format!("object {index}"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ObjectShape {
    Point(Pos<Meters>),
    Rect(Rect<Meters>),
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ShapeRow {
    #[schemars(extend("x-pos" = true))]
    Point { x: f32, y: f32 },
    #[schemars(extend("x-rect" = true))]
    Rect { x: f32, y: f32, w: f32, h: f32 },
}

/// The wire form: externally tagged, so a binary format needs no self-description to read it.
#[derive(Serialize, Deserialize)]
enum ShapeWire {
    Point(f32, f32),
    Rect(f32, f32, f32, f32),
}

impl Serialize for ObjectShape {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            return ShapeRow::from(*self).serialize(serializer);
        }
        match *self {
            ObjectShape::Point(point) => ShapeWire::Point(point.x, point.y),
            ObjectShape::Rect(rect) => ShapeWire::Rect(
                rect.origin.x,
                rect.origin.y,
                rect.size.width,
                rect.size.height,
            ),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ObjectShape {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<ObjectShape, D::Error> {
        if deserializer.is_human_readable() {
            return ShapeRow::deserialize(deserializer).map(ObjectShape::from);
        }
        Ok(match ShapeWire::deserialize(deserializer)? {
            ShapeWire::Point(x, y) => ObjectShape::Point(Pos::new(x, y)),
            ShapeWire::Rect(x, y, w, h) => {
                ObjectShape::Rect(Rect::new(Pos::new(x, y), Size::new(w, h)))
            }
        })
    }
}

impl JsonSchema for ObjectShape {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("ObjectShape")
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        ShapeRow::json_schema(generator)
    }

    fn inline_schema() -> bool {
        true
    }
}

impl From<ShapeRow> for ObjectShape {
    fn from(row: ShapeRow) -> ObjectShape {
        match row {
            ShapeRow::Point { x, y } => ObjectShape::Point(Pos::new(x, y)),
            ShapeRow::Rect { x, y, w, h } => {
                ObjectShape::Rect(Rect::new(Pos::new(x, y), Size::new(w, h)))
            }
        }
    }
}

impl From<ObjectShape> for ShapeRow {
    fn from(shape: ObjectShape) -> ShapeRow {
        match shape {
            ObjectShape::Point(point) => ShapeRow::Point {
                x: point.x,
                y: point.y,
            },
            ObjectShape::Rect(rect) => ShapeRow::Rect {
                x: rect.origin.x,
                y: rect.origin.y,
                w: rect.size.width,
                h: rect.size.height,
            },
        }
    }
}

impl ObjectShape {
    pub fn covers(&self, at: Pos<Meters>) -> bool {
        self.covers_cell(at.cell())
    }

    pub fn covers_cell(&self, cell: CellPos) -> bool {
        match self {
            ObjectShape::Point(point) => point.cell() == cell,
            ObjectShape::Rect(rect) => rect.contains(cell.center()),
        }
    }

    pub fn origin(&self) -> Pos<Meters> {
        match self {
            ObjectShape::Point(point) => *point,
            ObjectShape::Rect(rect) => rect.origin,
        }
    }

    pub fn center(&self) -> Pos<Meters> {
        match self {
            ObjectShape::Point(point) => *point,
            ObjectShape::Rect(rect) => rect.center(),
        }
    }

    pub fn bounds(&self) -> Rect<Meters> {
        match self {
            ObjectShape::Point(point) => {
                Rect::new(*point - Offset::new(0.5, 0.5), Size::new(1.0, 1.0))
            }
            ObjectShape::Rect(rect) => *rect,
        }
    }

    pub fn overlaps(&self, other: &ObjectShape) -> bool {
        let a = self.bounds();
        let b = other.bounds();
        a.origin.x < b.origin.x + b.size.width
            && b.origin.x < a.origin.x + a.size.width
            && a.origin.y < b.origin.y + b.size.height
            && b.origin.y < a.origin.y + a.size.height
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AreaObject {
    pub index: u32,
    pub name: Option<String>,
    pub title: Option<String>,
    pub shape: ObjectShape,
    pub look: Option<String>,
    pub warp: Option<Place>,
}

#[derive(Component, Default, Clone, PartialEq)]
pub struct Underfoot {
    inside: Vec<Entity>,
    standing: Vec<Entity>,
}

pub fn spawn_all(world: &mut World) {
    let zone = world.resource::<WorldArea>().0;
    let content = world.resource::<Content>().clone();
    let scripts = world.resource::<script::RiftScripts>().clone();
    for (index, object) in area::objects(area::placed(&content, zone)) {
        let index = index as usize;
        let owner_row = OwnerRow::of_object(zone, index as u32);
        let procedures = scripts.of(owner_row).to_vec();
        let may_do = scripts.may_do(owner_row);
        let interactive = Interactive::of(&procedures);
        let warp = may_do.reaches.iter().find_map(|reach| match reach {
            script::Reach::Warp(place) => Some(*place),
            _ => None,
        });
        let mut entity = world.spawn((
            Position::at(match object.shape {
                ObjectShape::Point(point) => point,
                ObjectShape::Rect(rect) => Pos::new(
                    rect.origin.x + rect.size.width / 2.0 - 0.5,
                    rect.origin.y + rect.size.height - 1.0,
                ),
            }),
            AreaTag { area: zone },
            AreaObject {
                index: index as u32,
                name: object.name.clone(),
                title: object.title.clone(),
                shape: object.shape,
                look: object.look.map(|look| look.path(&content).to_owned()),
                warp,
            },
        ));
        if object.look.is_some() || object.title.is_some() || interactive.is_some() {
            entity.insert(bevy_replicon::prelude::Replicated);
        }
        if let Some(title) = &object.title {
            entity.insert(ActorName {
                name: title.clone(),
            });
        }
        if let Some(interactive) = interactive {
            entity.insert(interactive);
        }
        if let Some(shown) = &object.shown {
            entity.insert(Presence::When(CompiledCondition::new(&content, shown)));
        }
        let id = entity.id();
        let owner = Owner::Entity(RunnerKind::Object, id);
        let mut instances = world.resource_mut::<RiftInstances>();
        instances.spawn_owner(owner, procedures);
        instances.raise(owner, EventKind::Started, None, Vec::new(), |_| true);
    }
}

pub fn named(world: &World, area: AreaId, name: &str) -> Option<Entity> {
    world.iter_entities().find_map(|entity| {
        let object = entity.get::<AreaObject>()?;
        let tag = entity.get::<AreaTag>()?;
        (tag.area == area && object.name.as_deref() == Some(name)).then_some(entity.id())
    })
}

pub fn inside(world: &World, actor: Entity, name: &str) -> bool {
    let Some(tag) = world.get::<AreaTag>(actor) else {
        return false;
    };
    let Some(at) = world.get::<Position>(actor) else {
        return false;
    };
    named(world, tag.area, name)
        .and_then(|object| world.get::<AreaObject>(object))
        .is_some_and(|object| object.shape.covers(at.pos))
}

pub fn shape_of(world: &World, object: Entity) -> Option<ObjectShape> {
    world.get::<AreaObject>(object).map(|object| object.shape)
}

type Actors =
    QueryState<(Entity, &'static Position, &'static AreaTag), Or<(With<PlayedBy>, With<Npc>)>>;

pub fn track(
    world: &mut World,
    actors: &mut Actors,
    objects: &mut QueryState<(Entity, &AreaObject, &AreaTag)>,
    mut now: Local<Vec<Entity>>,
) {
    let watchers = watchers(world);
    let instances = world.resource::<RiftInstances>();
    let crossed = |object: Entity| {
        [EventKind::Entered, EventKind::Left].iter().any(|kind| {
            instances
                .owners_answering(*kind)
                .contains(&Owner::Entity(RunnerKind::Object, object))
        })
    };
    let watched = |object: &AreaObject, area: AreaId| {
        watchers
            .iter()
            .any(|watcher| watcher.area == area && Some(&watcher.object) == object.name.as_ref())
    };
    let placed: Vec<Placed> = objects
        .iter(world)
        .filter(|(entity, object, tag)| crossed(*entity) || watched(object, tag.area))
        .map(|(entity, object, tag)| Placed {
            entity,
            name: object.name.clone(),
            shape: object.shape,
            area: tag.area,
        })
        .collect();
    if placed.is_empty() {
        return;
    }
    let positions: Vec<(Entity, Pos<Meters>, AreaId)> = actors
        .iter(world)
        .map(|(entity, at, tag)| (entity, at.pos, tag.area))
        .collect();
    for (actor, at, area_id) in positions {
        let cell = at.cell();
        now.clear();
        now.extend(
            placed
                .iter()
                .filter(|object| object.area == area_id && object.shape.covers_cell(cell))
                .filter(|object| crate::systems::visibility::present(world, object.entity, actor))
                .map(|object| object.entity),
        );
        now.sort_unstable();
        let is_npc = world.get::<Npc>(actor).is_some();
        let before = world.get::<Underfoot>(actor).cloned().unwrap_or_default();
        let standing = standing_on(world, actor, &before.standing, &now);
        for object in standing
            .iter()
            .filter(|object| !before.standing.contains(object))
        {
            raise_standing(world, &placed, &watchers, *object, actor, is_npc);
        }
        for object in now.iter().filter(|object| !before.inside.contains(object)) {
            raise_crossing(world, *object, EventKind::Entered, actor, is_npc);
        }
        for object in before.inside.iter().filter(|object| !now.contains(object)) {
            raise_crossing(world, *object, EventKind::Left, actor, is_npc);
        }
        let after = Underfoot {
            inside: now.clone(),
            standing,
        };
        if after != before {
            world.entity_mut(actor).insert(after);
        }
    }
}

fn standing_on(
    world: &World,
    actor: Entity,
    standing: &[Entity],
    inside: &[Entity],
) -> Vec<Entity> {
    if crate::systems::movement::moving(world, actor) {
        standing
            .iter()
            .copied()
            .filter(|object| inside.contains(object))
            .collect()
    } else {
        inside.to_vec()
    }
}

struct Placed {
    entity: Entity,
    name: Option<String>,
    shape: ObjectShape,
    area: AreaId,
}

struct Watcher {
    npc: Entity,
    area: AreaId,
    object: String,
}

fn watchers(world: &World) -> Vec<Watcher> {
    let instances = world.resource::<RiftInstances>();
    let mut watchers = Vec::new();
    for owner in instances.owners_answering(EventKind::StoodOn) {
        let Owner::Entity(RunnerKind::Npc, npc) = *owner else {
            continue;
        };
        let Some(area) = world.get::<AreaTag>(npc).map(|tag| tag.area) else {
            continue;
        };
        for event in instances.entries(*owner, EventKind::StoodOn) {
            if let RiftEvent::StoodOn { object, .. } = event {
                watchers.push(Watcher {
                    npc,
                    area,
                    object: object.clone(),
                });
            }
        }
    }
    watchers
}

fn raise_standing(
    world: &mut World,
    placed: &[Placed],
    watchers: &[Watcher],
    object: Entity,
    actor: Entity,
    is_npc: bool,
) {
    let Some(stood) = placed.iter().find(|placed| placed.entity == object) else {
        return;
    };
    let Some(name) = stood.name.clone() else {
        return;
    };
    let mut instances = world.resource_mut::<RiftInstances>();
    for watcher in watchers
        .iter()
        .filter(|watcher| watcher.area == stood.area && watcher.object == name)
    {
        let name = name.clone();
        instances.raise(
            Owner::Entity(RunnerKind::Npc, watcher.npc),
            EventKind::StoodOn,
            Some(actor),
            Vec::new(),
            move |event| match event {
                RiftEvent::StoodOn { by, object } => *object == name && by.admits(is_npc),
                _ => false,
            },
        );
    }
}

fn raise_crossing(world: &mut World, object: Entity, kind: EventKind, actor: Entity, is_npc: bool) {
    world.resource_mut::<RiftInstances>().raise(
        Owner::Entity(RunnerKind::Object, object),
        kind,
        Some(actor),
        Vec::new(),
        move |event| match event {
            RiftEvent::Entered { by } | RiftEvent::Left { by } => by.admits(is_npc),
            _ => false,
        },
    );
}
