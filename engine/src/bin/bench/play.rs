//! What the bench's players do, so a tick carries the load of a server whose players play: each
//! joins as one of the content's test characters that start in the area, and every couple of
//! seconds casts the next skill on its bar at whatever fits, walking into range, cooling down,
//! being refused, spreading statuses and raising vfx exactly as a live player does.

use bevy_ecs::prelude::*;
use engine::core::content::Content;
use engine::core::math::Pos;
use engine::core::tiling::{TilePos, Tiles};
use engine::systems::area::AreaId;
use engine::systems::from_client;
use engine::systems::movement::Position;
use engine::systems::npc::Npc;
use engine::systems::player::snapshot::{CharacterSnapshot, JoinAs, start_of};
use engine::systems::player::{ClientId, Players};
use engine::systems::skill::{Aim, Hotbar, HotbarEntry, SkillDef, Targeting, UseSkillRequest};

/// Ticks between one player's casts: two seconds at the server's rate.
pub const CAST_EVERY: usize = 60;

/// The test characters that start in `zone`, cycled through as the roster joins; a content with
/// none leaves every player the fresh character a join makes.
pub fn join_as(world: &mut World, content: &Content, zone: AreaId, conn: Entity, index: usize) {
    let characters: Vec<&CharacterSnapshot> = content
        .table::<CharacterSnapshot>()
        .ids()
        .map(|id| id.get(content))
        .filter(|snapshot| start_of(content, snapshot).area == zone)
        .collect();
    if characters.is_empty() {
        return;
    }
    let snapshot = characters[index % characters.len()].clone();
    let start = start_of(content, &snapshot);
    world.entity_mut(conn).insert(JoinAs { snapshot, start });
}

/// Every player whose turn it is this tick casts the next skill on its bar.
pub fn act(world: &mut World, content: &Content, roster: &[Entity], tick: usize) {
    for (index, &conn) in roster.iter().enumerate() {
        if !(tick + index).is_multiple_of(CAST_EVERY) {
            continue;
        }
        let Some(player) = player_of(world, conn) else {
            continue;
        };
        let skills: Vec<_> = world
            .get::<Hotbar>(player)
            .map(|bar| {
                bar.0
                    .iter()
                    .filter_map(|entry| match entry {
                        HotbarEntry::Skill(skill) => Some(*skill),
                        HotbarEntry::Empty | HotbarEntry::Item(_) => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        if skills.is_empty() {
            continue;
        }
        let skill = skills[(tick / CAST_EVERY + index) % skills.len()];
        let def: &SkillDef = skill.get(content);
        let Some(at) = world.get::<Position>(player).map(|position| position.pos) else {
            continue;
        };
        let aim = match def.target {
            Targeting::Caster => Aim::None,
            Targeting::SingleEnemy => match nearest_npc(world, at) {
                Some((npc, _)) => Aim::Actor(npc),
                None => continue,
            },
            Targeting::SingleAlly => {
                let other = roster[(index + 1) % roster.len()];
                match player_of(world, other) {
                    Some(ally) => Aim::Actor(ally),
                    None => continue,
                }
            }
            Targeting::AreaEnemy => Aim::Point(nearest_npc(world, at).map_or(at, |(_, pos)| pos)),
            Targeting::AreaAlly => Aim::Point(at),
        };
        from_client(world, conn, UseSkillRequest { skill, aim });
    }
}

fn player_of(world: &World, conn: Entity) -> Option<Entity> {
    let client = world.get::<ClientId>(conn)?;
    world.resource::<Players>().0.get(client).copied()
}

fn nearest_npc(world: &mut World, at: Pos<Tiles>) -> Option<(Entity, Pos<Tiles>)> {
    world
        .query_filtered::<(Entity, &Position), With<Npc>>()
        .iter(world)
        .map(|(entity, position)| (entity, position.pos))
        .min_by(|a, b| a.1.distance(at).0.total_cmp(&b.1.distance(at).0))
}
