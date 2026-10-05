use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{SendTargets, ToClients};
use serde::{Deserialize, Serialize};

use crate::systems::player::Owner;
use crate::systems::visibility::PrivateSight;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.add_server_message::<Notice>(Channel::Ordered);
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Notice {
    pub text: String,
    pub tone: NoticeTone,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeTone {
    Info,
    Good,
    Bad,
}

pub fn tell(world: &mut World, player: Entity, text: impl Into<String>, tone: NoticeTone) {
    let Some(owner) = world.get::<Owner>(player).map(|owner| owner.client) else {
        return;
    };
    let viewers: Vec<Entity> = world
        .query::<(Entity, &PrivateSight)>()
        .iter(world)
        .filter(|(_, sight)| sight.own == owner || sight.watching == Some(owner))
        .map(|(conn, _)| conn)
        .collect();
    let notice = Notice {
        text: text.into(),
        tone,
    };
    for conn in viewers {
        world.write_message(ToClients {
            targets: SendTargets::Single(bevy_replicon::prelude::ClientId::Client(conn)),
            message: notice.clone(),
        });
    }
}
