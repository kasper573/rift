pub mod render;

use bevy_app::App;
use bevy_ecs::entity::MapEntities;
use bevy_ecs::prelude::*;
use bevy_time::Time;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::Content;
use crate::core::math::Pos;
use crate::core::tiling::Tiles;
use crate::core::time::Seconds;
use crate::core::vfx::VfxSite;
use crate::systems::actor::{self, Actor};
use crate::systems::notification;
use crate::systems::npc::Pack;
use crate::systems::player::{self, Owner, sender_player, session};
use crate::systems::reach;
use crate::systems::skill::{self, BasicAttack};
use crate::systems::stat::{self, Stats};
use crate::systems::vfx::{self, VfxFixture};
use crate::systems::{drain_requests, visibility};

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.add_mapped_client_message::<AttackRequest>(Channel::Ordered)
        .replicate::<Attitude>();
}

#[derive(JsonSchema, Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Attitude {
    Friendly,
    Hostile,
}

/// How two actors stand to each other. Every character is every character's ally, as there are no
/// parties yet; a friendly npc is neutral to them and every other npc their enemy. Npcs side with
/// their pack and with each other when both are friendly, and stand against everyone else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    Own,
    Ally,
    Neutral,
    Enemy,
}

pub fn relation(world: &World, from: Entity, to: Entity) -> Relation {
    if from == to {
        return Relation::Own;
    }
    match (side(world, from), side(world, to)) {
        (Side::Character, Side::Character) => Relation::Ally,
        (Side::Character, Side::Friendly) | (Side::Friendly, Side::Character) => Relation::Neutral,
        (Side::Character, Side::Wild) | (Side::Wild, Side::Character) => Relation::Enemy,
        (Side::Friendly, Side::Friendly) => Relation::Ally,
        _ => {
            let pack = |entity| world.get::<Pack>(entity).copied();
            match (pack(from), pack(to)) {
                (Some(a), Some(b)) if a == b => Relation::Ally,
                _ => Relation::Enemy,
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Character,
    Friendly,
    Wild,
}

fn side(world: &World, entity: Entity) -> Side {
    if world.get::<Owner>(entity).is_some() {
        Side::Character
    } else if world.get::<Attitude>(entity) == Some(&Attitude::Friendly) {
        Side::Friendly
    } else {
        Side::Wild
    }
}

pub fn attackable(world: &World, attacker: Entity, target: Entity) -> bool {
    relation(world, attacker, target) == Relation::Enemy
        && !player::held(world, target)
        && visibility::coexist(world, attacker, target)
}

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct AttackRequest {
    #[entities]
    pub target: Entity,
}

pub fn enemy_at(world: &mut World, point: Pos<Tiles>) -> Option<Entity> {
    let me = session::my_character(world).map(|entity| entity.id())?;
    actor::frontmost_at(world, point, Some(me), |world, entity| {
        world.get::<Actor>(entity).is_some() && relation(world, me, entity) == Relation::Enemy
    })
}

#[derive(Component, Clone, Debug, PartialEq)]
pub struct Attackers {
    pub ids: Vec<Entity>,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct Died {
    pub entity: Entity,
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Dead {
    pub at: Seconds,
}

#[derive(JsonSchema, Clone, Copy, Serialize, Deserialize)]
pub struct HealthRegen {
    pub every: Seconds,
    pub health: f32,
}

#[derive(Resource, Default)]
pub struct RegenAt(Seconds);

pub fn regen(world: &mut World) {
    let Some(regen) = player::def(world.resource::<Content>()).regen() else {
        return;
    };
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    {
        let mut last = world.resource_mut::<RegenAt>();
        if now - last.0 < regen.every {
            return;
        }
        last.0 = now;
    }
    let players: Vec<Entity> = world
        .query_filtered::<Entity, With<Owner>>()
        .iter(world)
        .collect();
    for player in players {
        stat::regenerate(world, player, regen.health);
    }
}

/// A click on an enemy casts whatever sits in the basic attack slot at it, again and again until
/// it falls.
pub fn request(world: &mut World) {
    for request in drain_requests::<AttackRequest>(world) {
        let Some(entity) = sender_player(world, request.client_id) else {
            continue;
        };
        let target = request.message.target;
        if !player::may_command(world, entity)
            || world.get_entity(target).is_err()
            || stat::is_dead(world, target)
            || !attackable(world, entity, target)
        {
            continue;
        }
        let Some(skill) = world.get::<BasicAttack>(entity).and_then(|slot| slot.0) else {
            notification::error(world, entity, "Nothing to attack with");
            continue;
        };
        if let Err(reason) = skill::cast(world, entity, skill, skill::Aim::Actor(target), true) {
            notification::error(world, entity, reason);
        }
    }
}

pub fn inflict(world: &mut World, target: Entity, attacker: Option<Entity>, damage: f32) {
    if world.get_entity(target).is_err()
        || stat::is_dead(world, target)
        || player::held(world, target)
    {
        return;
    }
    let time = Seconds(world.resource::<Time>().elapsed_secs());
    if let Some(attacker) = attacker {
        add_attacker(world, target, attacker);
        crate::systems::item::reserve(world, target, attacker, time);
        crate::systems::npc::attacked(world, target, attacker);
    }
    stat::apply_damage(world, target, damage);
}

/// The one place a fallen actor is declared dead. Every health change lands before this runs, so
/// however health reached zero, the death is recorded and announced exactly once.
pub fn defeats(world: &mut World) {
    let time = Seconds(world.resource::<Time>().elapsed_secs());
    let fallen: Vec<Entity> = world
        .query_filtered::<(Entity, &Stats), Without<Dead>>()
        .iter(world)
        .filter(|(_, stats)| stat::dead(stats))
        .map(|(entity, _)| entity)
        .collect();
    for entity in fallen {
        reach::forget(world, entity);
        world
            .entity_mut(entity)
            .remove::<Attackers>()
            .insert(Dead { at: time });
        vfx::fixture::play(world, VfxFixture::Defeated, VfxSite::Actor(entity), &[]);
        world.write_message(Died { entity });
    }
}

pub fn revive(world: &mut World, entity: Entity, at: Pos<Tiles>) {
    reach::forget(world, entity);
    crate::systems::movement::relocate(world, entity, at);
    stat::revive(world, entity);
    world.entity_mut(entity).remove::<Dead>();
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
