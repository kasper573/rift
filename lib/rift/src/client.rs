use std::collections::HashMap;

use crate::codec::{EVENT, Opcode, SNAPSHOT};
use crate::codec::{Tag, Wire, read_varint, tag};
use crate::world::{ClientId, Entity, World};

#[derive(Default)]
pub struct Client {
    pub world: World,
    pub id: Option<ClientId>,
    pub tick: u32,
    inbox: HashMap<Tag, Vec<Vec<u8>>>,
    schema: Vec<Tag>, // wire id -> tag, learned from the server's first snapshot
}

impl Client {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn receive(&mut self, mut packet: &[u8]) {
        let reader = &mut packet;
        match Opcode::decode(reader) {
            Some(SNAPSHOT) => {
                let _ = self.apply(reader);
            }
            Some(EVENT) => {
                if let Some(tag) = Tag::decode(reader) {
                    self.inbox.entry(tag).or_default().push(reader.to_vec());
                }
            }
            _ => {}
        }
    }

    pub fn send<E: Wire + 'static>(&self, event: &E) -> Vec<u8> {
        let mut packet = Vec::new();
        tag::<E>().encode(&mut packet);
        event.encode(&mut packet);
        packet
    }
    pub fn drain_events<E: Wire + 'static>(&mut self) -> Vec<E> {
        self.inbox
            .remove(&tag::<E>())
            .into_iter()
            .flatten()
            .filter_map(|bytes| E::decode(&mut bytes.as_slice()))
            .collect()
    }

    fn apply(&mut self, reader: &mut &[u8]) -> Option<()> {
        self.tick = u32::decode(reader)?;
        self.id = Some(ClientId(u32::decode(reader)?));
        if u8::decode(reader)? != 0 {
            self.world.clear();
            let count = u8::decode(reader)? as usize;
            self.schema = (0..count)
                .map(|_| Tag::decode(reader))
                .collect::<Option<_>>()?;
        }
        for _ in 0..read_varint(reader)? {
            let entity = Entity(u32::decode(reader)?);
            let id = u8::decode(reader)?;
            if id == u8::MAX {
                self.world.drop_entity(entity);
            } else {
                self.world
                    .drop_component(entity, *self.schema.get(id as usize)?);
            }
        }
        for _ in 0..read_varint(reader)? {
            let entity = Entity(u32::decode(reader)?);
            let tag = *self.schema.get(u8::decode(reader)? as usize)?;
            let length = read_varint(reader)?;
            let (head, rest) = reader.split_at_checked(length)?;
            *reader = rest;
            self.world.apply_raw(entity, tag, head.to_vec());
        }
        Some(())
    }
}
