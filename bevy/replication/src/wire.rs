use bevy_ecs::entity::{Entity, EntityGeneration, EntityIndex};
use serde::{Deserialize, Serialize};

/// What a server tells a client in one send: first, on a link's first send, that whatever the
/// client holds is void; then what left its sight; then what came into it or changed; then the
/// messages of the ticks since the last send, so a message may name any entity the same send
/// brings.
#[derive(Serialize, Deserialize, Default)]
pub(crate) struct ServerFrame<'a> {
    /// The link's epoch, on its first send.
    pub fresh: Option<u32>,
    pub left: Vec<WireEntity>,
    #[serde(borrow)]
    pub entities: Vec<EntityChanges<'a>>,
    #[serde(borrow)]
    pub messages: Vec<Part<'a>>,
}

impl ServerFrame<'_> {
    pub fn is_empty(&self) -> bool {
        self.fresh.is_none()
            && self.left.is_empty()
            && self.entities.is_empty()
            && self.messages.is_empty()
    }
}

/// What changed of one entity: an entity the client doesn't know yet comes whole, one it knows by
/// what changed of it.
#[derive(Serialize, Deserialize)]
pub(crate) struct EntityChanges<'a> {
    pub entity: WireEntity,
    #[serde(borrow)]
    pub changes: Vec<Change<'a>>,
}

/// One component of an entity. To a client that doesn't hold the kind, its whole bytes. To one
/// that does, a patch over the bytes it holds, or whole bytes that replace them, or no bytes at
/// all to remove it: no component a client holds is ever told whole as nothing, as one that
/// serializes to nothing never changes.
#[derive(Serialize, Deserialize, Clone, Copy)]
pub(crate) struct Change<'a> {
    /// The kind shifted up a bit, with the bit below it set for a patch, so that both fit one
    /// varint byte for the first 64 kinds.
    tag: u16,
    pub bytes: &'a [u8],
}

impl<'a> Change<'a> {
    pub fn whole(kind: u16, bytes: &'a [u8]) -> Change<'a> {
        Change {
            tag: kind << 1,
            bytes,
        }
    }

    pub fn patch(kind: u16, bytes: &'a [u8]) -> Change<'a> {
        Change {
            tag: kind << 1 | 1,
            bytes,
        }
    }

    pub fn kind(self) -> u16 {
        self.tag >> 1
    }

    pub fn is_patch(self) -> bool {
        self.tag & 1 == 1
    }
}

/// What a client sends, tagged with the epoch of the link it last heard from, so a link handed
/// over drops what was meant for the one before.
#[derive(Serialize, Deserialize)]
pub(crate) struct ClientFrame<'a> {
    pub epoch: u32,
    #[serde(borrow)]
    pub messages: Vec<Part<'a>>,
}

/// One component or message, by its kind's place in the protocol.
#[derive(Serialize, Deserialize, Clone, Copy)]
pub(crate) struct Part<'a> {
    pub kind: u16,
    pub bytes: &'a [u8],
}

/// A server's entity by its index and generation, each a varint: `Entity::to_bits` packs them
/// into bits no varint shortens.
#[derive(Serialize, Deserialize, Clone, Copy)]
pub(crate) struct WireEntity {
    index: u32,
    generation: u32,
}

impl From<Entity> for WireEntity {
    fn from(entity: Entity) -> WireEntity {
        WireEntity {
            index: entity.index_u32(),
            generation: entity.generation().to_bits(),
        }
    }
}

impl WireEntity {
    pub fn entity(self) -> Option<Entity> {
        Some(Entity::from_index_and_generation(
            EntityIndex::from_raw_u32(self.index)?,
            EntityGeneration::from_bits(self.generation),
        ))
    }
}

/// Writes `new` as a patch over `old`, of the same length, into `out`: a bit per byte for whether
/// it differs, then the bytes that do. Most changes of a component touch a field or two, which
/// leaves most of its bytes as they were.
pub(crate) fn write_patch(old: &[u8], new: &[u8], out: &mut Vec<u8>) {
    debug_assert_eq!(old.len(), new.len());
    let mask = out.len();
    out.resize(mask + new.len().div_ceil(8), 0);
    for (at, (&was, &is)) in old.iter().zip(new).enumerate() {
        if was != is {
            out[mask + at / 8] |= 1 << (at % 8);
            out.push(is);
        }
    }
}

/// Applies a patch `write_patch` wrote over `held`, or `None` when it doesn't fit it.
pub(crate) fn apply_patch(held: &mut [u8], patch: &[u8]) -> Option<()> {
    let (mask, mut bytes) = patch.split_at_checked(held.len().div_ceil(8))?;
    for (at, byte) in held.iter_mut().enumerate() {
        if mask[at / 8] & 1 << (at % 8) != 0 {
            let (&first, rest) = bytes.split_first()?;
            *byte = first;
            bytes = rest;
        }
    }
    bytes.is_empty().then_some(())
}
