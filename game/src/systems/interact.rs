use bevy_app::App;
use bevy_ecs::entity::MapEntities;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::math::Pos;
use crate::core::tiling::Tiles;
use crate::data::attention::Id as AttentionId;
use crate::data::dialogue::Id as DialogueId;
use crate::data::npc::Id as NpcId;
use crate::data::prop::Id as PropId;
use crate::systems::actor;
use crate::systems::attention;
use crate::systems::dialogue;
use crate::systems::player::{commands_locked, sender_player, session};
use crate::systems::reach::{self, Pursuit, ReachAct, Tether};
use crate::systems::rule::{self, Encounter, Outcome, Requirement, Terms};
use crate::systems::stat;
use crate::systems::visibility;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Interactive>()
        .add_mapped_client_message::<InteractRequest>(Channel::Ordered)
        .init_resource::<InteractionSources>();
    attention::mark_source(app, marks);
}

pub struct Interaction {
    pub verb: Verb,
    pub reach: Tiles,
    pub responses: &'static [Response],
    pub marks: &'static [AttentionId],
}

pub struct Response {
    pub requires: &'static [&'static dyn Requirement],
    pub then: &'static [&'static dyn Outcome],
    pub news: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Talk,
    Use,
    Open,
    Read,
}

impl Verb {
    pub fn label(self) -> &'static str {
        match self {
            Verb::Talk => "Talk",
            Verb::Use => "Use",
            Verb::Open => "Open",
            Verb::Read => "Read",
        }
    }

    pub fn cursor(self) -> &'static str {
        match self {
            Verb::Talk => "icons/cursors/talk001.png",
            Verb::Use => "icons/cursors/hand002.png",
            Verb::Open => "icons/cursors/chest001.png",
            Verb::Read => "icons/cursors/book001.png",
        }
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Interactive;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Counterpart {
    Npc(NpcId),
    Prop(PropId),
}

impl Counterpart {
    pub fn name(self) -> &'static str {
        match self {
            Counterpart::Npc(npc) => npc.get().display_name,
            Counterpart::Prop(prop) => prop.get().display_name,
        }
    }
}

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Copy, Debug, PartialEq)]
pub struct InteractRequest {
    #[entities]
    pub target: Entity,
}

pub type InteractionSource = fn(&World, Entity) -> Option<&'static Interaction>;

#[derive(Resource, Default)]
pub struct InteractionSources(Vec<InteractionSource>);

pub fn interaction_source(app: &mut App, source: InteractionSource) {
    app.world_mut()
        .resource_mut::<InteractionSources>()
        .0
        .push(source);
}

pub fn interaction_of(world: &World, target: Entity) -> Option<&'static Interaction> {
    world.get::<Interactive>(target)?;
    world
        .resource::<InteractionSources>()
        .0
        .iter()
        .find_map(|source| source(world, target))
}

pub fn response(
    world: &World,
    player: Entity,
    interaction: &'static Interaction,
) -> Option<&'static Response> {
    interaction
        .responses
        .iter()
        .find(|response| rule::met(world, player, response.requires))
}

pub fn interactable_at(world: &mut World, point: Pos<Tiles>) -> Option<Entity> {
    let me = session::my_character(world).map(|entity| entity.id());
    actor::frontmost_at(world, point, me, |world, entity| {
        interaction_of(world, entity).is_some()
    })
}

pub fn interact_request(world: &mut World) {
    for request in crate::systems::requests::<InteractRequest>(world) {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        let target = request.message.target;
        if stat::is_dead(world, player)
            || commands_locked(world, player)
            || world.get_entity(target).is_err()
            || stat::is_dead(world, target)
            || interaction_of(world, target).is_none()
            || !visibility::present(world, target, player)
        {
            continue;
        }
        reach::intend(world, player, target, ReachAct::Interact);
    }
}

pub fn interactions(world: &mut World, intents: &mut reach::Intents) {
    for (player, target) in reach::intending(world, intents, ReachAct::Interact) {
        let Some(interaction) = interaction_of(world, target) else {
            reach::forget(world, player);
            continue;
        };
        match reach::pursue(world, player, interaction.reach) {
            Pursuit::Approaching => {}
            Pursuit::Lost => reach::forget(world, player),
            Pursuit::Arrived => {
                reach::forget(world, player);
                respond(world, player, target, interaction);
            }
        }
    }
}

pub fn conversation_starts(interaction: &Interaction) -> impl Iterator<Item = DialogueId> + '_ {
    interaction
        .responses
        .iter()
        .flat_map(|response| response.then)
        .flat_map(|outcome| outcome.leads_to())
}

fn respond(world: &mut World, player: Entity, target: Entity, interaction: &'static Interaction) {
    let Some(response) = response(world, player, interaction) else {
        return;
    };
    let encounter = Encounter {
        with: Some(target),
        tether: Tether::around(world, player, interaction.reach, Some(target)),
    };
    let terms = Terms {
        requires: response.requires,
        costs: &[],
        outcomes: response.then,
    };
    if let Err(refusal) = terms.settle(world, player, encounter) {
        crate::systems::notice::tell(
            world,
            player,
            refusal.0,
            crate::systems::notice::NoticeTone::Bad,
        );
    }
}

fn marks(world: &World, player: Entity, target: Entity) -> Vec<AttentionId> {
    let Some(interaction) = interaction_of(world, target) else {
        return Vec::new();
    };
    let news = response(world, player, interaction).is_some_and(|response| {
        response.news
            && response
                .then
                .iter()
                .flat_map(|outcome| outcome.leads_to())
                .any(|node| !dialogue::heard(world, player, node))
    });
    interaction
        .marks
        .iter()
        .copied()
        .chain(news.then_some(AttentionId::News))
        .collect()
}
