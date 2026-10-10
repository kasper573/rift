use bevy_ecs::prelude::*;

use crate::core::content::Content;
use crate::core::math::{Offset, Pos};
use crate::core::tiling::{TilePos, Tiles};
use crate::data;
use crate::systems::area::{self, AreaTag};
use crate::systems::combat::Attitude;
use crate::systems::movement::position;
use crate::systems::npc::NpcDef;
use crate::systems::player::Owner;
use crate::systems::rule::{Outcome, RuleContext};
use crate::systems::stat;
use crate::systems::visibility::Presence;

use super::{Home, Npc};

const RING: Tiles = Tiles(1.5);

pub struct SpawnNpcs {
    pub npc: data::npc::Id,
    pub count: u32,
    pub near: SpawnNear,
    pub shown: ShownFor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnNear {
    Speaker,
    Player,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShownFor {
    You,
    Everyone,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summoned {
    pub owner: Entity,
}

impl Outcome for SpawnNpcs {
    fn apply(&self, ctx: &mut RuleContext) {
        let near = match self.near {
            SpawnNear::Speaker => ctx.encounter.with.unwrap_or(ctx.player),
            SpawnNear::Player => ctx.player,
        };
        let (Some(at), Some(tag), Some(region)) = (
            position(ctx.world, near),
            ctx.world.get::<AreaTag>(near).cloned(),
            area::of(ctx.world, near),
        ) else {
            return;
        };
        let presence = match self.shown {
            ShownFor::You => match ctx.world.get::<Owner>(ctx.player) {
                Some(owner) => Some(Presence::For(owner.client)),
                None => return,
            },
            ShownFor::Everyone => None,
        };
        let pack = super::next_pack(ctx.world);
        for index in 0..self.count {
            let spot = beside(region, at, index, self.count);
            let summon = super::spawn(ctx.world, self.npc, spot, tag.area, pack);
            ctx.world
                .entity_mut(summon)
                .insert((Summoned { owner: ctx.player }, Home(spot)));
            if let Some(presence) = presence {
                ctx.world.entity_mut(summon).insert(presence);
            }
        }
    }
}

pub struct TurnHostile;

impl Outcome for TurnHostile {
    fn apply(&self, ctx: &mut RuleContext) {
        if let Some(npc) = ctx
            .encounter
            .with
            .filter(|&with| ctx.world.get::<Npc>(with).is_some())
        {
            ctx.world.entity_mut(npc).insert(Attitude::Hostile);
        }
    }
}

pub fn dismiss(world: &mut World, summons: &mut QueryState<(Entity, &'static Summoned)>) {
    let dismissed: Vec<Entity> = summons
        .iter(world)
        .filter(|&(summon, summoned)| {
            let area = |entity| world.get::<AreaTag>(entity).map(|tag| tag.area);
            world.get_entity(summoned.owner).is_err()
                || stat::is_dead(world, summoned.owner)
                || area(summoned.owner) != area(summon)
        })
        .map(|(summon, _)| summon)
        .collect();
    for summon in dismissed {
        world.despawn(summon);
    }
}

fn beside(region: &area::Area, at: Pos<Tiles>, index: u32, count: u32) -> Pos<Tiles> {
    let angle = std::f32::consts::TAU * index as f32 / count as f32;
    let spot = at + Offset::new(angle.cos() * RING.0, angle.sin() * RING.0);
    region.grid.nearest_walkable(spot).unwrap_or(at.snap())
}

/// Spawn an NPC beside you, for everyone to see.
#[bevy_terminal::command(name = "spawn", access = crate::systems::account::role::is_admin)]
fn spawn_command(
    world: &mut World,
    ctx: &bevy_terminal::CommandCtx,
    npc: String,
) -> Result<String, String> {
    let npc = crate::core::content::named::<NpcDef>(world, &npc)?;
    let content = world.resource::<Content>().clone();
    let player = crate::systems::player::conn_player(world, ctx.conn)
        .ok_or_else(|| "you have no player".to_owned())?;
    let spawn = SpawnNpcs {
        npc,
        count: 1,
        near: SpawnNear::Player,
        shown: ShownFor::Everyone,
    };
    let terms = crate::systems::rule::Terms {
        requires: &[],
        costs: &[],
        outcomes: &[&spawn],
    };
    terms
        .settle(world, player, crate::systems::rule::Encounter::default())
        .map_err(|refusal| refusal.0)?;
    Ok(format!("spawned {}", npc.get(&content).name))
}
