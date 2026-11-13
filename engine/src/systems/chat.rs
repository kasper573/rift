use std::sync::Arc;

use bevy_app::App;
use bevy_ecs::message::{MessageCursor, Messages};
use bevy_ecs::prelude::*;
use bevy_replication::{AppMessageExt, ToClient};
use bevy_terminal::{
    LineSpeaker, TerminalCompletionRequest, TerminalCompletions, TerminalInput, TerminalLine,
    TerminalSaid,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::SchemaMark;
use crate::core::script::{Arrow, Compiler, EffectDef, EffectStep, NEXT, ScriptFault, ValueType};
use crate::core::units::{MeterPos, Meters};
use crate::systems::actor::Name as ActorName;
use crate::systems::area::AreaTag;
use crate::systems::input::map::InputMap;
use crate::systems::movement::position;
use crate::systems::notification::{self, Notification, NotificationKind};
use crate::systems::npc::Npc;
use crate::systems::player::{self, ClientId, Players};
use crate::systems::script::event::OBJECT_OR_NPC;
use crate::systems::script::{
    Condition, EffectCall, EffectReach, EventKind, Owner, RiftEvent, RiftInstances, RiftScripting,
    RunnerKind,
};
use crate::systems::stat;
use crate::systems::text::{self, LineText, Span};
use crate::systems::visibility;

pub fn register(app: &mut App) {
    bevy_terminal::register(app, to_client);
    app.add_client_message::<TerminalInput>()
        .add_client_message::<TerminalCompletionRequest>()
        .add_server_message::<TerminalLine>()
        .add_server_message::<TerminalCompletions>();
}

/// Sends the terminal's lines to the clients they are for.
pub fn route_lines(world: &mut World) {
    bevy_terminal::route(world, to_client);
}

fn to_client(world: &mut World, conn: Entity, line: TerminalLine) {
    world.write_message(ToClient {
        client: conn,
        message: line,
    });
}

pub fn hear(world: &mut World) {
    for typed in super::drain_requests::<TerminalInput>(world) {
        bevy_terminal::hear(world, typed.client, &typed.message);
    }
}

/// Raises `heard` on every scripted object and npc in earshot of a player who said what it listens
/// for: a listed word or phrase among the player's words, whatever their case.
pub fn overhear(world: &mut World, mut cursor: Local<MessageCursor<TerminalSaid>>) {
    let saids: Vec<TerminalSaid> = cursor
        .read(world.resource::<Messages<TerminalSaid>>())
        .cloned()
        .collect();
    for said in saids {
        if let Some(speaker) = player::conn_player(world, said.conn) {
            raise_heard(world, speaker, &said.text);
        }
    }
}

pub fn rebroadcast(world: &mut World) {
    let saids: Vec<TerminalSaid> = world
        .resource_mut::<Messages<TerminalSaid>>()
        .drain()
        .collect();
    for said in saids {
        let client = world.get::<ClientId>(said.conn).copied();
        let name = player::conn_player(world, said.conn)
            .and_then(|player| world.get::<ActorName>(player))
            .map(|name| name.name.clone())
            .or_else(|| client.map(|client| format!("player {}", client.0)))
            .unwrap_or_else(|| "player".to_owned());
        let speaker = LineSpeaker {
            name,
            typist: client.map(|client| u64::from(client.0)),
        };
        broadcast(world, speaker, said.text);
    }
}

/// A chat line and a speech bubble from the owner.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("chat"))]
pub struct Say {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text: Vec<Span>,
    /// Without one, as far as the speaker is seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(transform = SchemaMark::Value.with("number"))]
    pub within: Option<Condition>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for Say {
    fn owners(&self) -> &'static [RunnerKind] {
        OBJECT_OR_NPC
    }
    fn needs_subject(&self) -> bool {
        false
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let values = text::values(&self.text)
            .into_iter()
            .map(|value| cx.expr(value, ValueType::Text))
            .collect::<Result<Vec<_>, _>>()?;
        let within = cx.opt_expr(self.within.as_ref(), ValueType::Number)?;
        let spans = self.text.clone();
        Ok(Box::new(move |world, _, cx| {
            let Some(speaker) = cx.owner else {
                return EffectStep::Done;
            };
            let spoken: Vec<_> = values.iter().map(|value| value(world, cx)).collect();
            let within = within.as_ref().map_or(visibility::sight(world), |within| {
                Meters(within(world, cx).as_num() as f32)
            });
            say(world, speaker, LineText::spoken(&spans, &spoken), within);
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        vec![EffectReach::Says]
    }
}

pub fn say(world: &mut World, speaker: Entity, text: LineText, within: Meters) {
    let name = world
        .get::<ActorName>(speaker)
        .map(|name| name.name.clone())
        .unwrap_or_default();
    let words = text.words(world.resource::<InputMap>());
    let voice = LineSpeaker {
        name: name.clone(),
        typist: None,
    };
    broadcast(world, voice, words);
    let kind = match world.get::<Npc>(speaker) {
        Some(npc) => NotificationKind::Speech { speaker: npc.def },
        None => NotificationKind::Narration {
            label: name.into(),
            sfx: None,
        },
    };
    let (Some(at), Some(area)) = (
        position(world, speaker),
        world.get::<AreaTag>(speaker).map(|tag| tag.area),
    ) else {
        return;
    };
    let listeners: Vec<Entity> = world
        .resource::<Players>()
        .0
        .values()
        .copied()
        .filter(|player| {
            world
                .get::<AreaTag>(*player)
                .is_some_and(|tag| tag.area == area)
        })
        .filter(|player| position(world, *player).is_some_and(|there| there.distance(at) <= within))
        .filter(|player| visibility::present(world, speaker, *player))
        .collect();
    for player in listeners {
        notification::notify(
            world,
            player,
            Some(speaker),
            Notification::new(kind.clone(), text.clone()),
        );
    }
}

fn raise_heard(world: &mut World, speaker: Entity, said: &str) {
    let (Some(at), Some(area)) = (
        position(world, speaker),
        world.get::<AreaTag>(speaker).map(|tag| tag.area),
    ) else {
        return;
    };
    let listeners: Vec<(Owner, Meters)> = world
        .resource::<RiftInstances>()
        .owners_answering(EventKind::Heard)
        .iter()
        .filter_map(|&owner| {
            let Owner::Entity(_, listener) = owner else {
                return None;
            };
            let in_earshot = listener != speaker
                && !stat::is_dead(world, listener)
                && world.get::<AreaTag>(listener).map(|tag| tag.area) == Some(area)
                && visibility::coexist(world, listener, speaker);
            let distance = position(world, listener)?.distance(at);
            in_earshot.then_some((owner, distance))
        })
        .collect();
    let spoken = Arc::new(spoken_words(said));
    for (owner, distance) in listeners {
        let spoken = spoken.clone();
        world.resource_mut::<RiftInstances>().raise(
            owner,
            EventKind::Heard,
            Some(speaker),
            Vec::new(),
            move |event| match event {
                RiftEvent::Heard { words, within } => {
                    distance <= *within && words.iter().any(|phrase| says(&spoken, phrase))
                }
                _ => false,
            },
        );
    }
}

fn says(spoken: &[String], phrase: &str) -> bool {
    let phrase = spoken_words(phrase);
    !phrase.is_empty() && spoken.windows(phrase.len()).any(|run| run == phrase)
}

fn spoken_words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn broadcast(world: &mut World, speaker: LineSpeaker, text: String) {
    let conns: Vec<Entity> = world
        .query_filtered::<Entity, With<ClientId>>()
        .iter(world)
        .collect();
    bevy_terminal::say(world, &conns, speaker, text);
}
