use bevy_ecs::message::Messages;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::FromClient;
use bevy_time::Time;

use super::movement::{MoveTarget, Path, forget, halt, on_tile};
use super::player::sender_player;
use crate::math::{Direction, Millis, PlaybackRate, Seconds, Tiles};
use crate::protocol;
use crate::protocol::{
    ACTION_ATTACK, ACTION_DEAD, Actor, AreaTag, AttackRequest, Vitals, action_name, is_dead,
    position, set_action, set_facing,
};
use crate::table::Id;

const TILE_DIAGONAL_MARGIN: Tiles = Tiles(std::f32::consts::SQRT_2 - 1.0);
const CHASE_RETARGET_THRESHOLD: Tiles = Tiles(1.5);

/// `attack_speed` scales the whole swing along with the attack animation;
/// `attack_delay` is the recovery between swings.
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

/// `Died` is the extension seam loot/xp/quests subscribe to; combat's own death cleanup stays
/// inline in `strike` rather than reacting to `Died`.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct Died {
    pub entity: Entity,
    pub killer: Entity,
}

/// A committed swing: the hit lands at `hit_at` — the attack animation's apex — and
/// the attacker is occupied until `ends_at`.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct Swing {
    pub target: Entity,
    pub hit_at: Seconds,
    pub ends_at: Seconds,
    pub struck: bool,
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

/// Approaches the target and, in range with recovery elapsed, commits to a swing whose
/// timing comes from the attacker's own attack animation.
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

        if Tiles(at.distance_to(target_at)) > stats.range + TILE_DIAGONAL_MARGIN {
            let heading = world.get::<MoveTarget>(id).is_some_and(|goal| {
                Tiles(goal.pos.distance_to(target_at)) <= CHASE_RETARGET_THRESHOLD
            });
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
        let dir = Direction::from_vec(target_at.x - at.x, target_at.y - at.y) as u8;
        if let Some(mut actor) = world.get_mut::<Actor>(id) {
            set_facing(&mut actor, dir, ACTION_ATTACK);
        }
        let timing = attack_timing(world, id, dir);
        let speed = PlaybackRate(stats.attack_speed.0.max(0.01));
        world.entity_mut(id).insert(Swing {
            target,
            hit_at: time + timing.apex / speed,
            ends_at: time + timing.duration / speed,
            struck: false,
        });
    }
}

/// Carries committed swings forward: the hit lands at the apex, the attacker animates
/// until the swing ends, and recovery starts from there. Death or movement cancels.
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
        vitals.health = (vitals.health - damage).max(0.0);
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

/// The attack animation's native timing for this attacker's model and facing — the same
/// manifest the client animates from, so the felt hit and the applied hit coincide.
fn attack_timing(world: &World, entity: Entity, dir: u8) -> crate::actors::Timing {
    let model = world
        .get::<protocol::Actor>(entity)
        .map_or(Id::new(0), |a| a.model);
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
