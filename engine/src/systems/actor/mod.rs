use std::collections::{HashMap, HashSet};

pub mod model;
pub mod plate;
pub mod playing;
pub mod render;

pub use model::{ActorModel, ModelDef, ModelId, Timing};
pub use playing::{Animate, LastAction, Playing, Wait};

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumString, VariantArray};

use crate::core::content::{ContentRegistry, LoadStep};
use crate::core::geometry::{Capsule, Radians};
use crate::core::math::{Pos, Rgba};
use crate::core::time::PlaybackRate;
use crate::core::units::{MeterPos, Meters};
use crate::systems::area::AreaObject;
use crate::systems::dialogue;
use crate::systems::movement::{self, Path, Position};
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
        .bundle::<ModelDef>()
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
    Talk,
}

impl Action {
    /// Whether the action's clip marks the moment its hit lands.
    pub fn takes_apex(self) -> bool {
        self == Action::Attack
    }
}

/// How an actor holds itself while idle: standing, or sitting when its model has a clip for it.
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
pub enum Pose {
    #[default]
    Idle,
    Sit,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tint {
    pub rgba: Rgba,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Actor {
    pub color: Rgba,
    pub facing: Radians,
    pub action: Action,
    pub pose: Pose,
    pub model: ModelId,
    pub attack_rate: PlaybackRate,
}

/// The capsule an actor is hit in, standing at its position.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Hitbox(pub Capsule);

/// Whom a point on the ground picks: the nearest actor standing over it or object lying under it
/// that `accept` takes, the dead and `except` passed over.
pub fn target_at(
    world: &mut World,
    point: Pos<Meters>,
    except: Option<Entity>,
    accept: impl Fn(&World, Entity) -> bool,
) -> Option<Entity> {
    let mut hits: Vec<(Entity, f32)> = world
        .query::<(Entity, &Position, &Hitbox)>()
        .iter(world)
        .filter(|(entity, at, hitbox)| Some(*entity) != except && hitbox.0.covers(at.pos, point))
        .map(|(entity, at, _)| (entity, (point - at.pos).length()))
        .collect();
    hits.extend(
        world
            .query::<(Entity, &AreaObject)>()
            .iter(world)
            .filter(|(entity, object)| Some(*entity) != except && object.shape.covers(point))
            .map(|(entity, object)| (entity, (point - object.shape.center()).length())),
    );
    hits.sort_by(|(_, a), (_, b)| a.total_cmp(b));
    hits.into_iter()
        .map(|(entity, _)| entity)
        .find(|&entity| !stat::is_dead(world, entity) && accept(world, entity))
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Name {
    pub name: String,
}

pub fn face_toward(world: &mut World, actor: Entity, toward: Pos<Meters>) {
    let Some(at) = world.get::<Position>(actor).map(|position| position.pos) else {
        return;
    };
    let step = toward - at;
    if step.x == 0.0 && step.y == 0.0 {
        return;
    }
    if let Some(mut component) = world.get_mut::<Actor>(actor) {
        let action = component.action;
        set_facing(&mut component, Radians::of(step), action);
    }
}

pub fn set_facing(actor: &mut Mut<Actor>, facing: Radians, action: Action) {
    if actor.facing != facing || actor.action != action {
        actor.facing = facing;
        actor.action = action;
    }
}

pub type Situations = QueryState<(
    Entity,
    &'static mut Actor,
    Option<&'static Stats>,
    Option<&'static Playing>,
    Has<Path>,
)>;

/// Picks every actor's action from its state: dead, playing, walking, in conversation, or idle.
/// Two in conversation face each other.
pub fn settle(world: &mut World, actors: &mut Situations, sessions: &mut dialogue::Sessions) {
    let talking: HashSet<Entity> = dialogue::parties(world, sessions).into_iter().collect();
    let facing_partner: HashMap<Entity, Radians> = sessions
        .iter(world)
        .filter_map(|(player, _)| Some((player, dialogue::counterpart_of(world, player)?)))
        .flat_map(|(player, partner)| [(player, partner), (partner, player)])
        .filter_map(|(me, partner)| {
            let (at, there) = (
                movement::position(world, me)?,
                movement::position(world, partner)?,
            );
            (at.distance(there) > Meters(1e-3)).then(|| (me, Radians::of(there - at)))
        })
        .collect();
    for (entity, mut actor, stats, playing, walking) in actors.iter_mut(world) {
        let action = if stats.is_some_and(stat::dead) {
            Action::Dead
        } else if let Some(playing) = playing {
            playing.action
        } else if walking {
            match actor.action {
                Action::Walk | Action::Run => actor.action,
                _ => Action::Walk,
            }
        } else if talking.contains(&entity) {
            Action::Talk
        } else {
            Action::Idle
        };
        match facing_partner.get(&entity) {
            Some(&facing) if action == Action::Talk => set_facing(&mut actor, facing, action),
            _ => set_action(&mut actor, action),
        }
    }
}

fn set_action(actor: &mut Mut<Actor>, action: Action) {
    if actor.action != action {
        actor.action = action;
    }
}
