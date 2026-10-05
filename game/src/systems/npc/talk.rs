use bevy_ecs::entity::MapEntities;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::tiling::Tiles;
use crate::data::dialogue::Id as DialogueId;
use crate::systems::combat::Attitude;
use crate::systems::dialogue::{self, BusyPolicy, Start};
use crate::systems::player::{commands_locked, sender_player};
use crate::systems::reach::{self, Pursuit, ReachAct, Tether};
use crate::systems::rule::{self, Requirement};
use crate::systems::stat;
use crate::systems::visibility;

use super::Npc;

pub struct Talk {
    pub reach: Tiles,
    pub greetings: &'static [Greeting],
}

pub struct Greeting {
    pub requires: &'static [&'static dyn Requirement],
    pub node: DialogueId,
    pub news: bool,
}

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Copy, Debug, PartialEq)]
pub struct TalkRequest {
    #[entities]
    pub target: Entity,
}

pub fn talk_of(world: &World, npc: Entity) -> Option<&'static Talk> {
    let def = world.get::<Npc>(npc)?.def.get();
    let friendly = world.get::<Attitude>(npc) == Some(&Attitude::Friendly);
    def.talk.as_ref().filter(|_| friendly)
}

pub fn greeting(world: &World, player: Entity, talk: &'static Talk) -> Option<&'static Greeting> {
    talk.greetings
        .iter()
        .find(|greeting| rule::met(world, player, greeting.requires))
}

pub fn talk_request(world: &mut World) {
    for request in crate::systems::requests::<TalkRequest>(world) {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        let target = request.message.target;
        if stat::is_dead(world, player)
            || commands_locked(world, player)
            || world.get_entity(target).is_err()
            || stat::is_dead(world, target)
            || talk_of(world, target).is_none()
            || !visibility::present(world, target, player)
        {
            continue;
        }
        reach::intend(world, player, target, ReachAct::Talk);
    }
}

pub fn talks(world: &mut World, intents: &mut reach::Intents) {
    for (player, npc) in reach::intending(world, intents, ReachAct::Talk) {
        let Some(talk) = talk_of(world, npc) else {
            reach::forget(world, player);
            continue;
        };
        match reach::pursue(world, player, talk.reach) {
            Pursuit::Approaching => {}
            Pursuit::Lost => reach::forget(world, player),
            Pursuit::Arrived => {
                reach::forget(world, player);
                greet(world, player, npc, talk);
            }
        }
    }
}

pub(super) fn greetings() -> Vec<DialogueId> {
    crate::data::npc::TABLE
        .iter()
        .flat_map(|def| def.talk.iter().flat_map(|talk| talk.greetings))
        .map(|greeting| greeting.node)
        .collect()
}

fn greet(world: &mut World, player: Entity, npc: Entity, talk: &'static Talk) {
    let Some(greeting) = greeting(world, player, talk) else {
        return;
    };
    let tether = Tether::around(world, player, talk.reach, Some(npc));
    dialogue::start(
        world,
        player,
        Start {
            node: greeting.node,
            with: Some(npc),
            tether,
            requires: &[],
            busy: BusyPolicy::Replace,
        },
    );
}
