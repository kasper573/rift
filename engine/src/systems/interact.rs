use bevy_app::App;
use bevy_ecs::entity::MapEntities;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::IntoStaticStr;

use crate::core::content::{Content, Fixture};
use crate::core::math::Pos;
use crate::core::tiling::Tiles;
use crate::systems::attention::{self, AttentionId, AttentionMark};
use crate::systems::combat::Attitude;
use crate::systems::dialogue;
use crate::systems::interface::CursorShape;
use crate::systems::player::{self, sender_player, session};
use crate::systems::reach::{self, Pursuit, ReachAct, ReachTarget};
use crate::systems::script::{self, EventKind, RiftEvent, RiftInstances};
use crate::systems::stat;
use crate::systems::visibility;
use crate::systems::{actor, drain_requests};

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Interactive>()
        .add_mapped_client_message::<InteractRequest>(Channel::Ordered);
    attention::mark_source(app, marks);
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
pub enum Verb {
    Talk,
    Use,
    Open,
    Read,
}

impl Verb {
    pub fn label(self) -> &'static str {
        self.into()
    }

    pub fn cursor(self) -> CursorShape {
        match self {
            Verb::Talk => CursorShape::Talk,
            Verb::Use => CursorShape::Use,
            Verb::Open => CursorShape::Open,
            Verb::Read => CursorShape::Read,
        }
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Interactive {
    pub verb: Verb,
    pub reach: Tiles,
}

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Copy, Debug, PartialEq)]
pub struct InteractRequest {
    #[entities]
    pub target: Entity,
}

pub fn interaction_of(world: &World, target: Entity) -> Option<Interactive> {
    let interactive = *world.get::<Interactive>(target)?;
    (world.get::<Attitude>(target) != Some(&Attitude::Hostile)).then_some(interactive)
}

pub fn interactable_at(world: &mut World, point: Pos<Tiles>) -> Option<Entity> {
    let me = session::my_character(world).map(|entity| entity.id());
    actor::frontmost_at(world, point, me, |world, entity| {
        interaction_of(world, entity).is_some()
    })
}

pub fn interact_request(world: &mut World) {
    for request in drain_requests::<InteractRequest>(world) {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        let target = request.message.target;
        if !player::may_command(world, player)
            || world.get_entity(target).is_err()
            || stat::is_dead(world, target)
            || interaction_of(world, target).is_none()
            || !visibility::present(world, target, player)
        {
            continue;
        }
        reach::intend(
            world,
            player,
            ReachTarget::Actor(target),
            ReachAct::Interact,
        );
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
                respond(world, player, target);
            }
        }
    }
}

fn respond(world: &mut World, player: Entity, target: Entity) {
    let Some(owner) = script::owner_of(world, target) else {
        return;
    };
    if dialogue::counterpart_of(world, player).is_some_and(|with| with != target) {
        dialogue::close_for(world, player);
    }
    world.resource_mut::<RiftInstances>().raise(
        owner,
        EventKind::Interacted,
        Some(player),
        Vec::new(),
        |_| true,
    );
}

fn marks(
    world: &World,
    target: Entity,
    viewers: &[Entity],
    out: &mut dyn FnMut(Entity, AttentionId),
) {
    let Some(owner) = script::owner_of(world, target) else {
        return;
    };
    let instances = world.resource::<RiftInstances>();
    if !instances.answers(owner, EventKind::Interacted) || interaction_of(world, target).is_none() {
        return;
    }
    let mut shown = false;
    let mut news = false;
    for event in instances.entries(owner, EventKind::Interacted) {
        if let RiftEvent::Interacted {
            marks, news: fresh, ..
        } = event
        {
            shown |= !marks.is_empty();
            news |= *fresh;
        }
    }
    if !shown && !news {
        return;
    }
    let news_mark = AttentionMark::News.id(world.resource::<Content>());
    for &player in viewers {
        for event in instances.entries(owner, EventKind::Interacted) {
            if let RiftEvent::Interacted { marks, .. } = event {
                for mark in marks.iter() {
                    out(player, *mark);
                }
            }
        }
        let unheard = news
            && instances
                .next_dialogue(world, owner, EventKind::Interacted, player)
                .iter()
                .any(|(script, node)| !dialogue::heard(world, player, script, node));
        if unheard {
            out(player, news_mark);
        }
    }
}
