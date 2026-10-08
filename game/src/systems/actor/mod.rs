pub mod bust;
mod model;
pub mod plate;
pub mod render;

use std::path::Path;

use bevy_app::App;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::assets::AssetService;
use crate::core::math::{Direction, Pos, Size};
use crate::core::tiling::{TilePos, Tiles};
use crate::core::time::PlaybackRate;
use crate::data;
use crate::systems::movement::Position;
use crate::systems::stat::{self, StatKind, Stats};

pub use model::{ActorModel, Timing, build_model};

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.replicate::<Actor>()
        .replicate::<Hitbox>()
        .replicate::<Name>();
}

pub fn check(assets: &AssetService) {
    assets.resolve_all(data::model::TABLE.iter().map(|def| def.sheet), build_model);
    for def in data::model::TABLE {
        for bust in def.busts.iter().flat_map(bust::Busts::all) {
            if let Err(error) = assets.open(Path::new(bust.0)) {
                panic!("bust {}: {error}", bust.0);
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Action {
    #[default]
    Idle,
    Walk,
    Run,
    Attack,
    Dead,
}

impl Action {
    pub const ALL: [Action; 5] = [
        Action::Idle,
        Action::Walk,
        Action::Run,
        Action::Attack,
        Action::Dead,
    ];

    pub fn named(name: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|action| action.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            Action::Idle => "idle",
            Action::Walk => "walk",
            Action::Run => "run",
            Action::Attack => "attack",
            Action::Dead => "death",
        }
    }
}

#[derive(
    Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default,
)]
pub struct Rgba(pub u32);

impl Rgba {
    pub fn color(self) -> bevy::color::Color {
        let [r, g, b, a] = self.0.to_be_bytes();
        bevy::color::Color::srgba_u8(r, g, b, a)
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Actor {
    pub color: Rgba,
    pub dir: Direction,
    pub action: Action,
    pub model: data::model::Id,
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

pub fn set_action(actor: &mut Mut<Actor>, action: Action) {
    if actor.action != action {
        actor.action = action;
    }
}

pub fn set_facing(actor: &mut Mut<Actor>, dir: Direction, action: Action) {
    if actor.dir != dir || actor.action != action {
        actor.dir = dir;
        actor.action = action;
    }
}

pub fn reset(mut actors: Query<(&mut Actor, Option<&Stats>)>) {
    for (mut actor, stats) in &mut actors {
        let dead = stats.is_some_and(|stats| stats.get(StatKind::Health) <= 0.0);
        set_action(&mut actor, if dead { Action::Dead } else { Action::Idle });
    }
}
