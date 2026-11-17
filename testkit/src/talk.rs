use bevy_ecs::prelude::*;
use engine::systems::area::AreaObject;
use engine::systems::dialogue::{Conversation, ConversationRequest};
use engine::systems::interact::InteractRequest;
use engine::systems::npc::{Npc, NpcDef};

use crate::Sim;
use engine::core::content::Content;

impl Sim {
    /// The client's player asks to interact with `target`, and the world runs until a
    /// conversation opens.
    pub fn open_with(&mut self, client: u32, target: Entity) -> Conversation {
        let conn = self.conn(client);
        let player = self.player(client);
        self.send(conn, InteractRequest { target });
        assert!(
            self.run_until(20.0, |world| world.get::<Conversation>(player).is_some()),
            "the conversation never opened"
        );
        self.conversation(client).expect("open")
    }

    /// The client's player talks to the character of the npc row under `key`.
    pub fn talk(&mut self, client: u32, key: &str) -> Conversation {
        let npc = self.npc(key);
        let opened = self.open_with(client, npc);
        assert_eq!(opened.with, Some(npc), "someone else spoke first");
        opened
    }

    pub fn conversation(&self, client: u32) -> Option<Conversation> {
        self.world.get::<Conversation>(self.player(client)).cloned()
    }

    /// Picks the choice labelled `label` in the client's conversation, and lets a tick answer it.
    pub fn pick(&mut self, client: u32, label: &str) -> Option<Conversation> {
        let now = self.conversation(client).expect("talking");
        let conn = self.conn(client);
        self.send(
            conn,
            ConversationRequest::Pick {
                step: now.step,
                choice: self.choice(&now, label),
            },
        );
        self.tick();
        self.conversation(client)
    }

    pub fn leave(&mut self, client: u32) {
        if let Some(now) = self.conversation(client) {
            let conn = self.conn(client);
            self.send(conn, ConversationRequest::Leave { step: now.step });
            self.tick();
        }
    }

    /// The map object named `name` in the area's map.
    pub fn object(&mut self, name: &str) -> Entity {
        let mut objects = self.world.query::<(Entity, &AreaObject)>();
        objects
            .iter(&self.world)
            .find(|(_, object)| object.name.as_deref() == Some(name))
            .map(|(entity, _)| entity)
            .unwrap_or_else(|| panic!("no object `{name}` on the map"))
    }

    /// The character of the npc row under `key`.
    pub fn npc(&mut self, key: &str) -> Entity {
        self.npcs(key)
            .first()
            .copied()
            .unwrap_or_else(|| panic!("no `{key}` in the area"))
    }

    /// Every character of the npc row under `key`.
    pub fn npcs(&mut self, key: &str) -> Vec<Entity> {
        let def = self.id::<NpcDef>(key);
        let mut npcs = self.world.query::<(Entity, &Npc)>();
        npcs.iter(&self.world)
            .filter(|(_, npc)| npc.def == def)
            .map(|(entity, _)| entity)
            .collect()
    }
}

impl Sim {
    /// The words of every line `conversation` shows.
    pub fn lines(&self, conversation: &Conversation) -> Vec<String> {
        let content = self.world.resource::<Content>();
        conversation
            .lines
            .iter()
            .map(|line| line.text.plain_words(content))
            .collect()
    }

    /// The words of every choice `conversation` offers.
    pub fn labels(&self, conversation: &Conversation) -> Vec<String> {
        let content = self.world.resource::<Content>();
        conversation
            .choices
            .iter()
            .map(|choice| choice.label.plain_words(content))
            .collect()
    }

    pub fn locked(&self, conversation: &Conversation, label: &str) -> bool {
        conversation.choices[self.choice(conversation, label) as usize].locked
    }

    fn choice(&self, conversation: &Conversation, label: &str) -> u32 {
        let content = self.world.resource::<Content>();
        conversation
            .choices
            .iter()
            .position(|choice| choice.label.plain_words(content) == label)
            .unwrap_or_else(|| panic!("no choice {label:?} in {:?}", self.labels(conversation)))
            as u32
    }
}
