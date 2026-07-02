use bevy_app::App;
use bevy_ecs::entity::MapEntities;
use bevy_ecs::prelude::*;
use bevy_time::Time;
use serde::{Deserialize, Serialize};

use crate::core::assets::AssetService;
use crate::core::math::{Direction, Pos};
use crate::core::tiling::{TilePos, Tiles};
use crate::core::time::{Millis, PlaybackRate, Seconds};
use crate::systems::actor::{Action, Actor, Hitbox, set_action, set_facing};
use crate::systems::area::AreaTag;
use crate::systems::movement::{
    MoveTarget, Path, Position, approach, forget, halt, on_tile, position,
};
use crate::systems::player::{Owner, sender_player, session};
use crate::systems::stat::{self, StatKind};

const TILE_DIAGONAL_MARGIN: Tiles = Tiles(std::f32::consts::SQRT_2 - 1.0);
const HP_REGEN_INTERVAL: Seconds = Seconds(10.0);
const HP_REGEN_AMOUNT: f32 = 5.0;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.add_mapped_client_message::<AttackRequest>(Channel::Ordered);
}

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct AttackRequest {
    #[entities]
    pub target: Entity,
}

pub fn enemy_at(world: &mut World, point: Pos<Tiles>) -> Option<Entity> {
    let me = session::me(world).map(|entity| entity.id());
    let hitboxes: Vec<(Entity, _)> = world
        .query_filtered::<(Entity, &Position, &Hitbox), With<Actor>>()
        .iter(world)
        .map(|(entity, at, hitbox)| (entity, at.pos.hitbox(hitbox.size)))
        .collect();
    hitboxes.into_iter().find_map(|(entity, hitbox)| {
        if Some(entity) == me || stat::is_dead(world, entity) {
            return None;
        }
        hitbox.contains(point).then_some(entity)
    })
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

pub fn regen(world: &mut World) {
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    {
        let mut last = world.resource_mut::<RegenAt>();
        if now - last.0 < HP_REGEN_INTERVAL {
            return;
        }
        last.0 = now;
    }
    let players: Vec<Entity> = world
        .query_filtered::<Entity, With<Owner>>()
        .iter(world)
        .collect();
    for player in players {
        if !stat::is_dead(world, player) {
            stat::heal(world, player, HP_REGEN_AMOUNT);
        }
    }
}

pub fn request(world: &mut World) {
    for request in crate::systems::requests::<AttackRequest>(world) {
        let Some(entity) = sender_player(world, request.client_id) else {
            continue;
        };
        let target = request.message.target;
        if stat::is_dead(world, entity)
            || world.get_entity(target).is_err()
            || stat::is_dead(world, target)
        {
            continue;
        }
        world.entity_mut(entity).insert(AttackTarget { target });
    }
}

pub fn combat(
    world: &mut World,
    targets: &mut QueryState<Entity, With<AttackTarget>>,
    swings: &mut QueryState<Entity, With<Swing>>,
) {
    let time = Seconds(world.resource::<Time>().elapsed_secs());
    let mut deaths = Vec::new();
    engage(world, time, targets);
    progress_swings(world, time, &mut deaths, swings);
    for (entity, killer) in deaths {
        world.write_message(Died { entity, killer });
    }
}

fn engage(world: &mut World, time: Seconds, targets: &mut QueryState<Entity, With<AttackTarget>>) {
    let ids: Vec<Entity> = targets.iter(world).collect();
    for id in ids {
        if stat::is_dead(world, id) {
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
        if world.get_entity(target).is_err() || stat::is_dead(world, target) || !same_area {
            forget(world, id);
            continue;
        }
        let (Some(at), Some(target_at)) = (position(world, id), position(world, target)) else {
            continue;
        };
        let stats = stat::effective_all(world, id);
        let range = Tiles(stats.get(StatKind::Range));
        let attack_delay = Millis(stats.get(StatKind::AttackDelay));
        let attack_speed = PlaybackRate(stats.get(StatKind::AttackSpeed));

        if !approach(world, id, target_at, range + TILE_DIAGONAL_MARGIN) {
            continue;
        }

        halt(world, id);
        if !on_tile(world, id) {
            continue;
        }
        if world
            .get::<LastAttack>(id)
            .is_some_and(|l| time - l.at < attack_delay.seconds())
        {
            continue;
        }
        let dir = Direction::from(target_at - at);
        if let Some(mut actor) = world.get_mut::<Actor>(id) {
            set_facing(&mut actor, dir, Action::Attack);
        }
        let timing = attack_timing(world, id, dir);
        let speed = attack_speed.at_least(0.01);
        world.entity_mut(id).insert(Swing {
            target,
            hit_at: time + timing.apex / speed,
            ends_at: time + timing.duration / speed,
            struck: false,
        });
    }
}

fn progress_swings(
    world: &mut World,
    time: Seconds,
    deaths: &mut Vec<(Entity, Entity)>,
    swings: &mut QueryState<Entity, With<Swing>>,
) {
    let ids: Vec<Entity> = swings.iter(world).collect();
    for id in ids {
        if stat::is_dead(world, id)
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
            strike(world, id, swing.target, deaths, time);
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
            set_action(&mut actor, Action::Attack);
        }
    }
}

fn strike(
    world: &mut World,
    attacker: Entity,
    target: Entity,
    deaths: &mut Vec<(Entity, Entity)>,
    time: Seconds,
) {
    let same_area = world.get::<AreaTag>(attacker).map(|t| t.area)
        == world.get::<AreaTag>(target).map(|t| t.area);
    if world.get_entity(target).is_err() || stat::is_dead(world, target) || !same_area {
        return;
    }
    let damage = stat::effective(world, attacker, StatKind::Damage);
    add_attacker(world, target, attacker);
    crate::systems::item::reserve(world, target, attacker, time);
    stat::apply_damage(world, target, damage);
    if stat::is_dead(world, target) {
        if let Some(mut actor) = world.get_mut::<Actor>(target) {
            set_action(&mut actor, Action::Dead);
        }
        forget(world, target);
        world.entity_mut(target).remove::<Attackers>();
        deaths.push((target, attacker));
    }
}

fn attack_timing(world: &World, entity: Entity, dir: Direction) -> crate::systems::actor::Timing {
    let assets = world.resource::<AssetService>();
    world
        .get::<Actor>(entity)
        .map(|a| {
            assets
                .resolve(*a.model.get(), crate::systems::actor::build_model)
                .timing(Action::Attack.name(), dir)
        })
        .unwrap_or_default()
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
