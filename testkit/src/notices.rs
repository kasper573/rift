use bevy_ecs::message::{MessageCursor, Messages};
use bevy_ecs::prelude::*;
use bevy_replication::ToClient;
use engine::core::content::Content;
use engine::systems::notification::{
    Notification, NotificationDef, NotificationKind, NotificationSent,
};
use engine::systems::script::condition;

use crate::Sim;

impl Sim {
    /// Every notification sent to the client so far, oldest first.
    pub fn notified(&mut self, client: u32) -> Vec<NotificationSent> {
        collect(&mut self.world);
        let conn = self.conn(client);
        self.world
            .resource::<Notified>()
            .seen
            .iter()
            .filter(|(to, _)| *to == conn)
            .map(|(_, sent)| sent.clone())
            .collect()
    }

    pub fn told(&mut self, client: u32) -> Vec<Notification> {
        self.notified(client)
            .into_iter()
            .map(|sent| sent.notification)
            .collect()
    }

    /// The words of every error the client was shown.
    pub fn errors(&mut self, client: u32) -> Vec<String> {
        self.told(client)
            .into_iter()
            .filter(|told| matches!(told.kind, NotificationKind::Error { .. }))
            .map(|told| told.text.plain_words())
            .collect()
    }

    /// The notification row under `key`, as it reaches a player.
    pub fn spoken(&self, key: &str) -> Notification {
        let content = self.world.resource::<Content>();
        let def = self.id::<NotificationDef>(key).get(content);
        def.notification(condition::spoken(
            &World::new(),
            Entity::PLACEHOLDER,
            &def.text,
        ))
    }

    pub fn forget_notified(&mut self) {
        collect(&mut self.world);
        self.world.resource_mut::<Notified>().seen.clear();
    }
}

/// Keeps every notification sent, as messages last only two ticks.
#[derive(Resource, Default)]
pub(crate) struct Notified {
    cursor: MessageCursor<ToClient<NotificationSent>>,
    seen: Vec<(Entity, NotificationSent)>,
}

pub(crate) fn collect(world: &mut World) {
    world.resource_scope(|world, mut notified: Mut<Notified>| {
        let messages = world.resource::<Messages<ToClient<NotificationSent>>>();
        let Notified { cursor, seen } = &mut *notified;
        for message in cursor.read(messages) {
            seen.push((message.client, message.message.clone()));
        }
    });
}
