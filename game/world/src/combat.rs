//! Combat: an entity's replicated [`Vitals`] and the attack request, plus the server systems that
//! engage targets, swing on the model's timing, deal damage, regenerate health, and emit deaths.

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::{Entity, MapEntities};
use bevy_ecs::message::Message;
use bevy_ecs::query::With;
use bevy_ecs::world::World;
use serde::{Deserialize, Serialize};

use crate::actor::{Actor, Hitbox};
use crate::core::math::Pos;
use crate::core::tiling::{TilePos, Tiles};
use crate::movement::Position;
use crate::player::session;

use crate::actor::{ACTION_ATTACK, ACTION_DEAD, action_name, set_action, set_facing};
use crate::area::AreaTag;
use crate::core::math::Direction;
use crate::core::table::Id;
use crate::core::time::{Millis, PlaybackRate, Seconds};
use crate::movement::{MoveTarget, Path, forget, halt, on_tile, position};
use crate::player::{Owner, sender_player};
use bevy_ecs::message::Messages;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::FromClient;
use bevy_time::Time;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.replicate::<Vitals>()
        .add_mapped_client_message::<AttackRequest>(Channel::Ordered);
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Vitals {
    pub health: f32,
    pub max: f32,
}

impl Vitals {
    pub fn heal(&mut self, amount: f32) {
        self.health = (self.health + amount).min(self.max);
    }

    pub fn damage(&mut self, amount: f32) {
        self.health = (self.health - amount).max(0.0);
    }

    pub fn refill(&mut self) {
        self.health = self.max;
    }

    pub fn fraction(&self) -> f32 {
        (self.health / self.max).clamp(0.0, 1.0)
    }

    pub fn is_dead(&self) -> bool {
        self.health <= 0.0
    }
}

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct AttackRequest {
    #[entities]
    pub target: Entity,
}

pub fn is_dead(world: &World, entity: Entity) -> bool {
    world.get::<Vitals>(entity).is_some_and(Vitals::is_dead)
}

/// The living enemy (not the local player) whose hitbox covers `point` — the client's attack target.
pub fn enemy_at(world: &mut World, point: Pos<Tiles>) -> Option<Entity> {
    let me = session::me(world).map(|entity| entity.id());
    let mut actors =
        world.query_filtered::<(Entity, &Position, &Hitbox, Option<&Vitals>), With<Actor>>();
    actors.iter(world).find_map(|(entity, at, hitbox, vitals)| {
        if Some(entity) == me || vitals.is_some_and(Vitals::is_dead) {
            return None;
        }
        at.pos.hitbox(hitbox.size).contains(point).then_some(entity)
    })
}

const TILE_DIAGONAL_MARGIN: Tiles = Tiles(std::f32::consts::SQRT_2 - 1.0);
const CHASE_RETARGET_THRESHOLD: Tiles = Tiles(1.5);
const HP_REGEN_INTERVAL: Seconds = Seconds(10.0);
const HP_REGEN_AMOUNT: f32 = 5.0;

#[derive(Component, Clone, Debug, PartialEq)]
pub struct Stats {
    pub damage: f32,
    pub attack_speed: PlaybackRate,
    pub attack_delay: Millis,
    pub range: Tiles,
}

#[derive(Component, Clone, Debug, PartialEq)]
pub struct AttackTarget {
    pub target: Entity,
}

#[derive(Component, Clone, Debug, PartialEq)]
pub struct LastAttack {
    pub at: Seconds,
}

#[derive(Component, Clone, Debug, PartialEq)]
pub struct Attackers {
    pub ids: Vec<Entity>,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct Died {
    pub entity: Entity,
    pub killer: Entity,
}

#[derive(Component, Clone, Debug, PartialEq)]
pub struct Swing {
    pub target: Entity,
    pub hit_at: Seconds,
    pub ends_at: Seconds,
    pub struck: bool,
}

#[derive(Resource, Default)]
pub struct RegenAt(Seconds);

pub fn regen(
    time: Res<Time>,
    mut last: ResMut<RegenAt>,
    mut players: Query<&mut Vitals, With<Owner>>,
) {
    let now = Seconds(time.elapsed_secs());
    if now - last.0 < HP_REGEN_INTERVAL {
        return;
    }
    last.0 = now;
    for mut vitals in &mut players {
        if !vitals.is_dead() {
            vitals.heal(HP_REGEN_AMOUNT);
        }
    }
}

pub fn request(world: &mut World) {
    let requests: Vec<FromClient<AttackRequest>> = world
        .resource_mut::<Messages<FromClient<AttackRequest>>>()
        .drain()
        .collect();
    for request in requests {
        let Some(entity) = sender_player(world, request.client_id) else {
            continue;
        };
        let target = request.message.target;
        if is_dead(world, entity) || world.get_entity(target).is_err() || is_dead(world, target) {
            continue;
        }
        world.entity_mut(entity).insert(AttackTarget { target });
    }
}

pub fn combat(world: &mut World) {
    let time = Seconds(world.resource::<Time>().elapsed_secs());
    let mut deaths = Vec::new();
    engage(world, time);
    progress_swings(world, time, &mut deaths);
    for (entity, killer) in deaths {
        world.write_message(Died { entity, killer });
    }
}

fn engage(world: &mut World, time: Seconds) {
    let ids: Vec<Entity> = world
        .query_filtered::<Entity, With<AttackTarget>>()
        .iter(world)
        .collect();
    for id in ids {
        if is_dead(world, id) {
            forget(world, id);
            continue;
        }
        if world.get::<Swing>(id).is_some() {
            continue;
        }
        let Some(target) = world.get::<AttackTarget>(id).map(|t| t.target) else {
            continue;
        };
        let same_area = world.get::<AreaTag>(id).map(|t| t.area)
            == world.get::<AreaTag>(target).map(|t| t.area);
        if world.get_entity(target).is_err() || is_dead(world, target) || !same_area {
            forget(world, id);
            continue;
        }
        let (Some(at), Some(target_at)) = (position(world, id), position(world, target)) else {
            continue;
        };
        let stats = stats(world, id);

        if at.distance(target_at) > stats.range + TILE_DIAGONAL_MARGIN {
            let heading = world
                .get::<MoveTarget>(id)
                .is_some_and(|goal| goal.pos.distance(target_at) <= CHASE_RETARGET_THRESHOLD);
            if !heading {
                world
                    .entity_mut(id)
                    .remove::<Path>()
                    .insert(MoveTarget { pos: target_at });
            }
            continue;
        }

        halt(world, id);
        if !on_tile(world, id) {
            continue;
        }
        if world
            .get::<LastAttack>(id)
            .is_some_and(|l| time - l.at < stats.attack_delay.seconds())
        {
            continue;
        }
        let dir = Direction::from(target_at - at) as u8;
        if let Some(mut actor) = world.get_mut::<Actor>(id) {
            set_facing(&mut actor, dir, ACTION_ATTACK);
        }
        let timing = attack_timing(world, id, dir);
        let speed = stats.attack_speed.at_least(0.01);
        world.entity_mut(id).insert(Swing {
            target,
            hit_at: time + timing.apex / speed,
            ends_at: time + timing.duration / speed,
            struck: false,
        });
    }
}

fn progress_swings(world: &mut World, time: Seconds, deaths: &mut Vec<(Entity, Entity)>) {
    let ids: Vec<Entity> = world
        .query_filtered::<Entity, With<Swing>>()
        .iter(world)
        .collect();
    for id in ids {
        if is_dead(world, id)
            || world.get::<MoveTarget>(id).is_some()
            || world.get::<Path>(id).is_some()
        {
            world
                .entity_mut(id)
                .remove::<Swing>()
                .insert(LastAttack { at: time });
            continue;
        }
        let Some(swing) = world.get::<Swing>(id).cloned() else {
            continue;
        };
        if !swing.struck && time >= swing.hit_at {
            strike(world, id, swing.target, deaths);
            if let Some(mut swing) = world.get_mut::<Swing>(id) {
                swing.struck = true;
            }
        }
        if time >= swing.ends_at {
            world
                .entity_mut(id)
                .remove::<Swing>()
                .insert(LastAttack { at: swing.ends_at });
        } else if let Some(mut actor) = world.get_mut::<Actor>(id) {
            set_action(&mut actor, ACTION_ATTACK);
        }
    }
}

fn strike(world: &mut World, attacker: Entity, target: Entity, deaths: &mut Vec<(Entity, Entity)>) {
    let same_area = world.get::<AreaTag>(attacker).map(|t| t.area)
        == world.get::<AreaTag>(target).map(|t| t.area);
    if world.get_entity(target).is_err() || is_dead(world, target) || !same_area {
        return;
    }
    let damage = stats(world, attacker).damage;
    add_attacker(world, target, attacker);
    if let Some(mut vitals) = world.get_mut::<Vitals>(target) {
        vitals.damage(damage);
    }
    if is_dead(world, target) {
        if let Some(mut actor) = world.get_mut::<Actor>(target) {
            set_action(&mut actor, ACTION_DEAD);
        }
        forget(world, target);
        world.entity_mut(target).remove::<Attackers>();
        deaths.push((target, attacker));
    }
}

fn stats(world: &World, entity: Entity) -> Stats {
    world.get::<Stats>(entity).cloned().unwrap_or(Stats {
        damage: 0.0,
        attack_speed: PlaybackRate(1.0),
        attack_delay: Millis(0.0),
        range: Tiles(1.0),
    })
}

// Manifest the client animates from, so the felt hit and applied hit coincide.
fn attack_timing(world: &World, entity: Entity, dir: u8) -> crate::actor::Timing {
    let model = world.get::<Actor>(entity).map_or(Id::new(0), |a| a.model);
    model.get().timing(action_name(ACTION_ATTACK), dir)
}

fn add_attacker(world: &mut World, target: Entity, by: Entity) {
    match world.get_mut::<Attackers>(target) {
        Some(mut attackers) => {
            if !attackers.ids.contains(&by) {
                attackers.ids.push(by);
            }
        }
        None => {
            world.entity_mut(target).insert(Attackers { ids: vec![by] });
        }
    }
}
