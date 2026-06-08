use rift::{Builder, Ctx, Entity, Wire, World};

use crate::core::actors::ActorModelId;
use crate::core::math::{Direction, Millis, PlaybackRate, Seconds, Tiles};
use crate::core::protocol::{
    ACTION_ATTACK, ACTION_DEAD, AreaTag, Vitals, action_name, is_dead, position, set_action,
    set_facing,
};
use crate::core::{actors, protocol};
use crate::features::movement::{MoveTarget, Path, forget, halt, on_tile};
use crate::features::player::Players;

const TILE_DIAGONAL_MARGIN: f32 = std::f32::consts::SQRT_2 - 1.0;
const CHASE_RETARGET_THRESHOLD: Tiles = Tiles(1.5);

/// `attack_speed` scales the whole swing along with the attack animation;
/// `attack_delay` is the recovery between swings.
#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Stats {
    pub damage: f32,
    pub attack_speed: PlaybackRate,
    pub attack_delay: Millis,
    pub range: Tiles,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct AttackTarget {
    pub target: Entity,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct LastAttack {
    pub at: Seconds,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Attackers {
    pub ids: Vec<Entity>,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct AttackRequest {
    pub target: Entity,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Died {
    pub entity: Entity,
    pub killer: Entity,
}

/// A committed swing: the hit lands at `hit_at` — the attack animation's apex — and
/// the attacker is occupied until `ends_at`.
#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Swing {
    pub target: Entity,
    pub hit_at: Seconds,
    pub ends_at: Seconds,
    pub struck: bool,
}

pub fn feature(b: &mut Builder) {
    b.intent(request);
    b.system(combat);
}

fn request(ctx: &mut Ctx) {
    for (client, req) in ctx.server.drain_events::<AttackRequest>() {
        let Some(&entity) = ctx.res.get::<Players>().and_then(|p| p.0.get(&client)) else {
            continue;
        };
        let world = &mut ctx.server.world;
        if is_dead(world, entity) || !world.alive(req.target) || is_dead(world, req.target) {
            continue;
        }
        world.insert(entity, AttackTarget { target: req.target });
    }
}

fn combat(ctx: &mut Ctx) {
    let time = Seconds(ctx.time);
    let mut deaths = Vec::new();
    {
        let world = &mut ctx.server.world;
        engage(world, time);
        progress_swings(world, time, &mut deaths);
    }
    // `Died` is the extension seam (loot/xp/quests subscribe here). Combat keeps its own
    // inline cleanup in `strike` so behavior is unchanged this tick.
    for (entity, killer) in deaths {
        ctx.events.emit(Died { entity, killer });
    }
}

/// Approaches the target and, in range with recovery elapsed, commits to a swing whose
/// timing comes from the attacker's own attack animation.
fn engage(world: &mut World, time: Seconds) {
    for id in world.ids::<AttackTarget>() {
        if is_dead(world, id) {
            forget(world, id);
            continue;
        }
        if world.has::<Swing>(id) {
            continue;
        }
        let Some(target) = world.get::<AttackTarget>(id).map(|t| t.target) else {
            continue;
        };
        let same_area = world.get::<AreaTag>(id).map(|t| t.area)
            == world.get::<AreaTag>(target).map(|t| t.area);
        if !world.alive(target) || is_dead(world, target) || !same_area {
            forget(world, id);
            continue;
        }
        let (Some(at), Some(target_at)) = (position(world, id), position(world, target)) else {
            continue;
        };
        let stats = stats(world, id);

        if at.distance(target_at) > stats.range.0 + TILE_DIAGONAL_MARGIN {
            let heading = world
                .get::<MoveTarget>(id)
                .is_some_and(|goal| goal.pos.distance(target_at) <= CHASE_RETARGET_THRESHOLD.0);
            if !heading {
                world.remove::<Path>(id);
                world.insert(id, MoveTarget { pos: target_at });
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
        let dir = Direction::from_vec(target_at.x.0 - at.x.0, target_at.y.0 - at.y.0) as u8;
        set_facing(world, id, dir, ACTION_ATTACK);
        let timing = attack_timing(world, id, dir);
        let speed = stats.attack_speed.0.max(0.01);
        world.insert(
            id,
            Swing {
                target,
                hit_at: time + Seconds(timing.apex / speed),
                ends_at: time + Seconds(timing.duration / speed),
                struck: false,
            },
        );
    }
}

/// Carries committed swings forward: the hit lands at the apex, the attacker animates
/// until the swing ends, and recovery starts from there. Death or movement cancels.
fn progress_swings(world: &mut World, time: Seconds, deaths: &mut Vec<(Entity, Entity)>) {
    for id in world.ids::<Swing>() {
        if is_dead(world, id) || world.has::<MoveTarget>(id) || world.has::<Path>(id) {
            world.remove::<Swing>(id);
            world.insert(id, LastAttack { at: time });
            continue;
        }
        let Some(swing) = world.get::<Swing>(id) else {
            continue;
        };
        if !swing.struck && time >= swing.hit_at {
            strike(world, id, swing.target, deaths);
            world.modify::<Swing>(id, |s| s.struck = true);
        }
        if time >= swing.ends_at {
            let ended = swing.ends_at;
            world.remove::<Swing>(id);
            world.insert(id, LastAttack { at: ended });
        } else {
            set_action(world, id, ACTION_ATTACK);
        }
    }
}

fn strike(world: &mut World, attacker: Entity, target: Entity, deaths: &mut Vec<(Entity, Entity)>) {
    let same_area = world.get::<AreaTag>(attacker).map(|t| t.area)
        == world.get::<AreaTag>(target).map(|t| t.area);
    if !world.alive(target) || is_dead(world, target) || !same_area {
        return;
    }
    let damage = stats(world, attacker).damage;
    add_attacker(world, target, attacker);
    world.modify::<Vitals>(target, |v| v.health = (v.health - damage).max(0.0));
    if is_dead(world, target) {
        set_action(world, target, ACTION_DEAD);
        forget(world, target);
        world.remove::<Attackers>(target);
        deaths.push((target, attacker));
    }
}

fn stats(world: &World, entity: Entity) -> Stats {
    world.get::<Stats>(entity).unwrap_or(Stats {
        damage: 0.0,
        attack_speed: PlaybackRate(1.0),
        attack_delay: Millis(0.0),
        range: Tiles(1.0),
    })
}

/// The attack animation's native timing for this attacker's model and facing — the same
/// manifest the client animates from, so the felt hit and the applied hit coincide.
fn attack_timing(world: &World, entity: Entity, dir: u8) -> actor::Timing {
    let model = world
        .get::<protocol::Actor>(entity)
        .map_or(ActorModelId(0), |a| a.model);
    actors::models()[model.0 as usize].timing(action_name(ACTION_ATTACK), dir)
}

fn add_attacker(world: &mut World, target: Entity, by: Entity) {
    match world.get::<Attackers>(target) {
        Some(mut attackers) => {
            if !attackers.ids.contains(&by) {
                attackers.ids.push(by);
                world.insert(target, attackers);
            }
        }
        None => world.insert(target, Attackers { ids: vec![by] }),
    }
}
