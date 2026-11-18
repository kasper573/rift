use std::sync::Arc;

use bevy_app::App;
use bevy_ecs::entity::MapEntities;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::IntoStaticStr;

use crate::core::content::{Content, Fixture};
use crate::core::content::{ContentProblem, Loading, serial};
use crate::core::render::Pointed;
use crate::core::script::{
    EventDef, EventKind, Instances, Procedure, RunnerKind, ScriptEvent, Vocabulary,
};
use crate::core::script::{Scripts, owner_site};
use crate::core::units::Meters;
use crate::systems::area::OBJECT_RUNNER;
use crate::systems::area::script::owner_of;
use crate::systems::area::spawnable::SPAWNABLE_RUNNER;
use crate::systems::area::{self, AreaId};
use crate::systems::attention::{AttentionId, AttentionMark};
use crate::systems::combat::Attitude;
use crate::systems::interface::CursorShape;
use crate::systems::mount::RiderAct;
use crate::systems::npc::NPC_RUNNER;
use crate::systems::player::RequestNeeds;
use crate::systems::player::{player_requests, session};
use crate::systems::reach::{self, Pursuit, ReachAct, ReachTarget};
use crate::systems::{actor, dialogue, stat, visibility};

pub fn register(app: &mut App) {
    use bevy_replication::AppMessageExt;

    use bevy_replication::AppReplicationExt;
    app.replicate::<Interactive>()
        .add_mapped_client_message::<InteractRequest>();
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
    pub reach: Meters,
}

impl Interactive {
    /// How the first of `procedures` that answers an interaction is reached, if any does.
    pub fn of(procedures: &[Arc<Procedure>]) -> Option<Interactive> {
        procedures
            .iter()
            .flat_map(|procedure| procedure.entry(EventKind::of::<Interacted>()))
            .find_map(|(_, event)| {
                event.downcast_ref::<Interacted>().map(|event| Interactive {
                    verb: event.verb,
                    reach: event.reach,
                })
            })
    }
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

pub fn interactable_at(world: &mut World, pointed: &Pointed) -> Option<Entity> {
    let me = session::my_character(world).map(|entity| entity.id());
    actor::target_at(world, pointed, me, |world, entity| {
        interaction_of(world, entity).is_some()
    })
}

pub fn interact_request(world: &mut World) {
    player_requests::<InteractRequest>(
        world,
        RequestNeeds::Seat(RiderAct::Interact),
        |world, player, request| {
            let target = request.target;
            if stat::stands(world, target)
                && interaction_of(world, target).is_some()
                && visibility::in_sight(world, player, target)
            {
                reach::intend(
                    world,
                    player,
                    ReachTarget::Actor(target),
                    ReachAct::Interact,
                );
            }
            Ok(())
        },
    );
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
    let Some(owner) = owner_of(world, target) else {
        return;
    };
    if dialogue::counterpart_of(world, player).is_some_and(|with| with != target) {
        dialogue::close_for(world, player);
    }
    world.resource_mut::<Instances>().raise(
        owner,
        EventKind::of::<Interacted>(),
        Some(player),
        Vec::new(),
        |_: &ScriptEvent| true,
    );
}

pub(crate) fn marks(
    world: &World,
    target: Entity,
    viewers: &[Entity],
    out: &mut dyn FnMut(Entity, AttentionId),
) {
    let Some(owner) = owner_of(world, target) else {
        return;
    };
    let instances = world.resource::<Instances>();
    if !instances.answers(owner, EventKind::of::<Interacted>())
        || interaction_of(world, target).is_none()
    {
        return;
    }
    let mut shown = false;
    let mut news = false;
    for event in instances
        .entries(owner, EventKind::of::<Interacted>())
        .filter_map(|event| event.downcast_ref::<Interacted>())
    {
        shown |= !event.marks.is_empty();
        news |= event.news;
    }
    if !shown && !news {
        return;
    }
    let news_mark = AttentionMark::News.id(world.resource::<Content>());
    for &player in viewers {
        for event in instances
            .entries(owner, EventKind::of::<Interacted>())
            .filter_map(|event| event.downcast_ref::<Interacted>())
        {
            for mark in &event.marks {
                out(player, *mark);
            }
        }
        let unheard = news
            && instances
                .next_dialogue(world, owner, EventKind::of::<Interacted>(), player)
                .iter()
                .any(|(script, node)| !dialogue::heard(world, player, script, node));
        if unheard {
            out(player, news_mark);
        }
    }
}

/// A player used the owner: talked to it, opened it, read it.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug)]
pub struct Interacted {
    pub verb: Verb,
    pub reach: Meters,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<AttentionId>,
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub news: bool,
}

impl EventDef for Interacted {
    fn owners(&self) -> &'static [RunnerKind] {
        &[OBJECT_RUNNER, SPAWNABLE_RUNNER, NPC_RUNNER]
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary.event::<Interacted>("interacted");
}

/// Whoever may be interacted with must have a title to be known by.
pub fn check_titled(loading: &mut Loading) {
    let content = loading.content();
    let scripts = content.built::<Scripts>();
    for heard in scripts.hearing::<Interacted>() {
        if heard.row.kind != OBJECT_RUNNER {
            continue;
        }
        let area: AreaId = heard.row.id();
        let titled = area::objects(area::placed(&content, area))
            .any(|(index, object)| Some(index) == heard.row.object && object.title.is_some());
        if !titled {
            let site = owner_site(&content, heard.row);
            loading.json_error(
                &site.file,
                &site.pointer,
                ContentProblem::new(
                    "interactive without title",
                    heard.label.to_owned(),
                    "a title on every object a script of which answers interacted",
                ),
            );
        }
    }
}
