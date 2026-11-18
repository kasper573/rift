pub mod credit;
pub mod effects;
pub mod query;
pub mod render;

use crate::core::script::Vocabulary;
use bevy_app::App;
use bevy_ecs::entity::{EntityMapper, MapEntities};
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

use crate::core::content::{Content, Fixture, Loading, serial};
use crate::core::math::Pos;
use crate::core::render::Pointed;
use crate::core::time;
use crate::core::units::{Meters, Seconds};
use crate::core::vfx::{SiteKind, VfxDef, VfxSite, check_fixtures_played};
use crate::systems::actor::{self, Actor};
use crate::systems::area;
use crate::systems::mount::RiderAct;
use crate::systems::npc::Pack;
use crate::systems::player::RequestNeeds;
use crate::systems::player::{self, PlayedBy, player_requests, session};
use crate::systems::reach;
use crate::systems::skill::{self, BasicAttack, CastAim, CastWay, Targeting};
use crate::systems::stat::{self, Stats, Vital};
use crate::systems::visibility;
use crate::systems::{GameSettings, messages};

/// The vfx every game provides, played on an actor where it is defeated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, VariantArray, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum DefeatVfx {
    Defeated,
}

impl Fixture for DefeatVfx {
    type Row = VfxDef;
    const FOLDER: Option<&'static str> = Some("vfx");
}

pub fn check_vfx(loading: &mut Loading) {
    check_fixtures_played::<DefeatVfx>(loading, &[SiteKind::Actor]);
}

pub fn register(app: &mut App) {
    use bevy_replication::{AppMessageExt, AppReplicationExt};

    app.add_mapped_client_message::<AttackRequest>()
        .replicate::<Attitude>();
}

#[derive(JsonSchema, Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Attitude {
    Friendly,
    Hostile,
}

/// How two actors stand to each other: as the game's relations regard the sides they are on, npcs
/// of one pack always allies.
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
    let relations = &world.resource::<GameSettings>().relations;
    let regard = match (side(world, from), side(world, to)) {
        (Side::Character, Side::Character) => relations.characters,
        (Side::Character, Side::Friendly) | (Side::Friendly, Side::Character) => {
            relations.characters_and_friendly
        }
        (Side::Character, Side::Hostile) | (Side::Hostile, Side::Character) => {
            relations.characters_and_hostile
        }
        (Side::Friendly, Side::Friendly) => relations.friendly,
        _ => {
            let pack = |entity| world.get::<Pack>(entity).copied();
            match (pack(from), pack(to)) {
                (Some(a), Some(b)) if a == b => Regard::Ally,
                _ => relations.npcs_apart,
            }
        }
    };
    match regard {
        Regard::Ally => Relation::Ally,
        Regard::Neutral => Relation::Neutral,
        Regard::Enemy => Relation::Enemy,
    }
}

/// How one side of the game regards another.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Regard {
    Ally,
    Neutral,
    Enemy,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Character,
    Friendly,
    Hostile,
}

fn side(world: &World, entity: Entity) -> Side {
    if world.get::<PlayedBy>(entity).is_some() {
        Side::Character
    } else if world.get::<Attitude>(entity) == Some(&Attitude::Friendly) {
        Side::Friendly
    } else {
        Side::Hostile
    }
}

/// Whether a basic attack may land on the target: what a single-target enemy skill asks.
pub fn attackable(world: &World, attacker: Entity, target: Entity) -> bool {
    skill::casting::fits(world, attacker, target, Targeting::SingleEnemy)
}

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct AttackRequest {
    #[entities]
    pub target: Entity,
}

pub fn enemy_at(world: &mut World, pointed: &Pointed) -> Option<Entity> {
    let me = session::my_character(world).map(|entity| entity.id())?;
    actor::target_at(world, pointed, Some(me), |world, entity| {
        world.get::<Actor>(entity).is_some() && relation(world, me, entity) == Relation::Enemy
    })
}

#[derive(Component, Clone, Debug, PartialEq)]
#[component(map_entities)]
pub struct Attackers {
    pub ids: Vec<Entity>,
}

impl MapEntities for Attackers {
    fn map_entities<E: EntityMapper>(&mut self, mapper: &mut E) {
        self.ids.retain_mut(|id| {
            *id = mapper.get_mapped(*id);
            *id != Entity::PLACEHOLDER
        });
    }
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct Died {
    pub entity: Entity,
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Dead {
    pub at: Seconds,
}

/// How much health and mana every character regains, and how often.
#[derive(JsonSchema, Clone, Copy, Serialize, Deserialize)]
pub struct VitalRegen {
    pub every: Seconds,
    pub health: f32,
    #[serde(default, skip_serializing_if = "serial::is_zero_f32")]
    pub mana: f32,
}

#[derive(Resource, Default)]
pub struct RegenAt(Seconds);

pub fn regen(world: &mut World) {
    let Some(regen) = player::def(world.resource::<Content>()).regen() else {
        return;
    };
    let now = time::uptime(world);
    {
        let mut last = world.resource_mut::<RegenAt>();
        if now - last.0 < regen.every {
            return;
        }
        last.0 = now;
    }
    let players: Vec<Entity> = world
        .query_filtered::<Entity, With<PlayedBy>>()
        .iter(world)
        .collect();
    for player in players {
        stat::restore(world, player, Vital::Health, regen.health);
        stat::restore(world, player, Vital::Mana, regen.mana);
    }
}

/// A click on an enemy casts whatever sits in the basic attack slot at it, again and again until
/// it falls.
pub fn request(world: &mut World) {
    player_requests::<AttackRequest>(
        world,
        RequestNeeds::Seat(RiderAct::Skills),
        |world, player, request| {
            if !visibility::in_sight(world, player, request.target)
                || !attackable(world, player, request.target)
            {
                return Ok(());
            }
            let skill = world
                .get::<BasicAttack>(player)
                .and_then(|slot| slot.0)
                .ok_or_else(|| {
                    messages::of(world.resource::<Content>())
                        .skill
                        .nothing_to_attack_with()
                        .to_owned()
                })?;
            skill::cast(
                world,
                player,
                skill,
                CastAim::Actor(request.target),
                CastWay::Repeated,
            )
        },
    );
}

pub fn inflict(world: &mut World, target: Entity, attacker: Option<Entity>, damage: f32) {
    if !stat::stands(world, target) || player::arriving(world, target) {
        return;
    }
    let time = time::uptime(world);
    if let Some(attacker) = attacker {
        for fighter in [attacker, target] {
            world.entity_mut(fighter).insert(Fought { at: time });
        }
        add_attacker(world, target, attacker);
        crate::systems::item::reserve(world, target, attacker, time);
        crate::systems::npc::attacked(world, target, attacker);
        let reason = messages::of(world.resource::<Content>())
            .skill
            .under_attack()
            .to_owned();
        crate::systems::dialogue::interrupt(world, target, &reason);
    }
    stat::apply_damage(world, target, damage);
}

/// When an actor last struck someone or was struck by someone.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Fought {
    pub at: Seconds,
}

/// Whether `actor` is in combat: they struck someone or were struck by someone within the time
/// the game's claims last.
pub fn in_combat(world: &World, actor: Entity) -> bool {
    let lasts = world.resource::<GameSettings>().claims_last;
    let now = time::uptime(world);
    world
        .get::<Fought>(actor)
        .is_some_and(|fought| now - fought.at < lasts)
}

/// The actors still standing whose stats just changed: an entity falls only as its stats change.
pub type Standing = QueryState<(Entity, &'static Stats), (Changed<Stats>, Without<Dead>)>;

/// The one place a fallen actor is declared dead. Every health change lands before this runs, so
/// however health reached zero, the death is recorded and announced exactly once.
pub fn defeats(world: &mut World, standing: &mut Standing) {
    let time = time::uptime(world);
    let fallen: Vec<Entity> = standing
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
        area::vfx::play_fixture(world, DefeatVfx::Defeated, VfxSite::Actor(entity));
        world.write_message(Died { entity });
    }
}

pub fn revive(world: &mut World, entity: Entity, at: Pos<Meters>) {
    reach::forget(world, entity);
    stat::revive(world, entity);
    world.entity_mut(entity).remove::<Dead>();
    area::warp::land(world, entity, at);
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

pub fn scripting(vocabulary: &mut Vocabulary) {
    query::scripting(vocabulary);
    effects::scripting(vocabulary);
}
