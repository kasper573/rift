use bevy_ecs::message::Messages;
use bevy_ecs::prelude::*;
use bevy_terminal::TerminalSaid;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
use crate::systems::script::{Condition, EffectCall, Reach, RiftScripting, RunnerKind};
use crate::systems::text::{self, LineText, Span};
use crate::systems::visibility::{self, VIEW_DISTANCE};

pub fn rebroadcast(world: &mut World) {
    let saids: Vec<TerminalSaid> = world
        .resource_mut::<Messages<TerminalSaid>>()
        .drain()
        .collect();
    for said in saids {
        let name = player::conn_player(world, said.conn)
            .and_then(|player| world.get::<ActorName>(player))
            .map(|name| name.name.clone())
            .or_else(|| {
                world
                    .get::<ClientId>(said.conn)
                    .map(|client| format!("player {}", client.0))
            })
            .unwrap_or_else(|| "player".to_owned());
        bevy_terminal::broadcast(world, format!("{name}: {}", said.text));
    }
}

/// A chat line and a speech bubble from the owner.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(extend("x-system" = "chat"))]
pub struct Say {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text: Vec<Span>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = 8.0))]
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
            let within = within.as_ref().map_or(VIEW_DISTANCE, |within| {
                Meters(within(world, cx).as_num() as f32)
            });
            say(world, speaker, LineText::spoken(&spans, &spoken), within);
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![Reach::Says]
    }
}

pub fn say(world: &mut World, speaker: Entity, text: LineText, within: Meters) {
    let name = world
        .get::<ActorName>(speaker)
        .map(|name| name.name.clone())
        .unwrap_or_default();
    let words = text.words(world.resource::<InputMap>());
    bevy_terminal::broadcast(world, format!("{name}: {words}"));
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
        notification::notify_from(
            world,
            player,
            Some(speaker),
            Notification::new(kind.clone(), text.clone()),
        );
    }
}
