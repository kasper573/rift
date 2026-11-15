//! Blocking: a player keeps another away altogether, until they undo it.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::core::store::{Record, RecordId};
use crate::core::time::UnixMillis;
use crate::systems::account::AccountId;
use crate::systems::society::{AccountNumber, Settling};

/// That `by` keeps `of` away, since when.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Block {
    pub by: AccountId,
    pub of: AccountId,
    pub since: UnixMillis,
}

impl Record for Block {
    const KIND: &'static str = "block";

    fn id(&self) -> RecordId {
        Block::id_of(&self.by, &self.of)
    }
}

impl Block {
    /// The blocker's id is led by its length, so no two pairs of ids make one record id.
    fn id_of(by: &AccountId, of: &AccountId) -> RecordId {
        RecordId(format!("{}:{}{}", by.0.len(), by.0, of.0))
    }
}

/// Who every account keeps away.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockLists(HashMap<AccountId, BTreeMap<AccountId, UnixMillis>>);

impl BlockLists {
    pub fn of(blocks: impl IntoIterator<Item = Block>) -> BlockLists {
        let mut lists = BlockLists::default();
        for block in blocks {
            lists
                .0
                .entry(block.by)
                .or_default()
                .insert(block.of, block.since);
        }
        lists
    }

    /// Whether `by` keeps `of` away.
    pub fn blocks(&self, by: &AccountId, of: &AccountId) -> bool {
        self.0
            .get(by)
            .is_some_and(|blocked| blocked.contains_key(of))
    }

    /// Whether either of the two keeps the other away.
    pub fn part(&self, one: &AccountId, other: &AccountId) -> bool {
        self.blocks(one, other) || self.blocks(other, one)
    }

    /// Everyone `by` keeps away.
    pub fn blocked_by(&self, by: &AccountId) -> impl Iterator<Item = &AccountId> {
        self.0.get(by).into_iter().flat_map(BTreeMap::keys)
    }

    fn block(&mut self, by: &AccountId, of: &AccountId, now: UnixMillis) -> Option<Block> {
        let blocked = self.0.entry(by.clone()).or_default();
        if blocked.contains_key(of) {
            return None;
        }
        blocked.insert(of.clone(), now);
        Some(Block {
            by: by.clone(),
            of: of.clone(),
            since: now,
        })
    }

    fn unblock(&mut self, by: &AccountId, of: &AccountId) -> bool {
        self.0
            .get_mut(by)
            .is_some_and(|blocked| blocked.remove(of).is_some())
    }
}

/// What a player asks be done to their block list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockAsk {
    Block(AccountNumber),
    Unblock(AccountNumber),
}

/// Blocks or unblocks as `by` asked: who they now keep away that they did not. A block ends every
/// request waiting between the two, and neither is told.
pub(crate) fn settle(settling: &mut Settling, by: &AccountId, ask: BlockAsk) -> Option<AccountId> {
    let (BlockAsk::Block(number) | BlockAsk::Unblock(number)) = ask;
    let of = settling.society.accounts().numbered(number).cloned()?;
    if of == *by {
        return None;
    }
    match ask {
        BlockAsk::Block(_) => {
            let block = settling.society.blocks.block(by, &of, settling.now)?;
            settling.unwritten.put(block);
            super::end_between(settling, by, &of);
            settling.society.changed(&of);
            settling.society.changed(by);
            Some(of)
        }
        BlockAsk::Unblock(_) => {
            if settling.society.blocks.unblock(by, &of) {
                settling.unwritten.remove::<Block>(Block::id_of(by, &of));
                settling.society.changed(by);
            }
            None
        }
    }
}
