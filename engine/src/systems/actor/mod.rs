pub mod model;
pub mod plate;
pub mod playing;
pub mod render;

pub use model::{ActorModel, ModelDef, Timing};
pub use playing::{Animate, LastAction, Playing, Wait};

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumString, VariantArray};

use crate::core::content::{ContentRegistry, LoadStep};
use crate::core::math::{Direction, Pos, Rgba, Size};
use crate::core::tiling::{TilePos, Tiles};
use crate::core::time::PlaybackRate;
use crate::systems::actor::model::ModelId;
use crate::systems::movement::{Path, Position};
use crate::systems::stat::{self, Stats};

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.replicate::<Actor>()
        .replicate::<Playing>()
        .replicate_once::<Hitbox>()
        .replicate_once::<Name>();
}

pub fn content(registry: &mut ContentRegistry) {
    registry
        .row::<ModelDef>()
        .modules::<ModelDef>()
        .hook(LoadStep::Files, model::build_all);
}

#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    Default,
    VariantArray,
    EnumString,
    AsRefStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Action {
    #[default]
    Idle,
    Walk,
    Run,
    Attack,
    #[serde(rename = "death")]
    #[strum(serialize = "death")]
    Dead,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tint {
    pub rgba: Rgba,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Actor {
    pub color: Rgba,
    pub dir: Direction,
    pub action: Action,
    pub model: ModelId,
    pub attack_rate: PlaybackRate,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Hitbox {
    pub size: Size<Tiles>,
}

pub fn frontmost_at(
    world: &mut World,
    point: Pos<Tiles>,
    except: Option<Entity>,
    accept: impl Fn(&World, Entity) -> bool,
) -> Option<Entity> {
    let hits: Vec<(Entity, f32)> = world
        .query::<(Entity, &Position, &Hitbox)>()
        .iter(world)
        .filter(|(entity, at, hitbox)| {
            Some(*entity) != except && at.pos.hitbox(hitbox.size).contains(point)
        })
        .map(|(entity, at, _)| (entity, at.pos.y))
        .collect();
    hits.into_iter()
        .filter(|&(entity, _)| !stat::is_dead(world, entity) && accept(world, entity))
        .max_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(entity, _)| entity)
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Name {
    pub name: String,
}

pub fn face_toward(world: &mut World, actor: Entity, toward: Pos<Tiles>) {
    let Some(at) = world.get::<Position>(actor).map(|position| position.pos) else {
        return;
    };
    let step = toward - at;
    if step.x == 0.0 && step.y == 0.0 {
        return;
    }
    if let Some(mut component) = world.get_mut::<Actor>(actor) {
        let action = component.action;
        set_facing(&mut component, Direction::from(step), action);
    }
}

pub fn set_facing(actor: &mut Mut<Actor>, dir: Direction, action: Action) {
    if actor.dir != dir || actor.action != action {
        actor.dir = dir;
        actor.action = action;
    }
}

type Situation = (
    &'static mut Actor,
    Option<&'static Stats>,
    Option<&'static Playing>,
    Has<Path>,
);

pub fn settle(mut actors: Query<Situation>) {
    for (mut actor, stats, playing, walking) in &mut actors {
        let action = if stats.is_some_and(stat::dead) {
            Action::Dead
        } else if let Some(playing) = playing {
            playing.action
        } else if walking {
            match actor.action {
                Action::Walk | Action::Run => actor.action,
                _ => Action::Walk,
            }
        } else {
            Action::Idle
        };
        set_action(&mut actor, action);
    }
}

fn set_action(actor: &mut Mut<Actor>, action: Action) {
    if actor.action != action {
        actor.action = action;
    }
}
