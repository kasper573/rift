//! Which of many things hold the few slots the GPU keeps for them, as tiles of ground hold the
//! layers of an atlas: what is wanted keeps its slot, what is wanted anew takes a free one or the
//! one wanted longest ago, and nothing wanted this frame gives its slot up.

use std::collections::HashMap;
use std::hash::Hash;

/// The slots, each held by a key as its data stood when it was written there (its `stamp`), and
/// when each was last wanted.
#[derive(Debug)]
pub struct Residency<K, S> {
    slots: Vec<Option<Held<K, S>>>,
    by_key: HashMap<K, u32>,
    frame: u64,
}

/// A slot given to a key to write its data into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Admitted<K> {
    pub slot: u32,
    /// What held the slot before, which holds none now.
    pub evicted: Option<K>,
}

impl<K: Copy + Eq + Hash, S: PartialEq> Residency<K, S> {
    pub fn new(slots: u32) -> Residency<K, S> {
        Residency {
            slots: (0..slots).map(|_| None).collect(),
            by_key: HashMap::new(),
            frame: 0,
        }
    }

    /// Begins a frame: what was wanted before it may now give up its slot.
    pub fn next_frame(&mut self) {
        self.frame += 1;
    }

    /// The slot `key` holds and the stamp of what it wrote there, marking it wanted this frame.
    pub fn want(&mut self, key: K) -> Option<(u32, &S)> {
        let slot = *self.by_key.get(&key)?;
        let held = self.slots[slot as usize].as_mut()?;
        held.wanted = self.frame;
        Some((slot, &held.stamp))
    }

    /// Whether `key` holds a slot whose data is as it is now, at `stamp`.
    pub fn is_current(&self, key: K, stamp: &S) -> bool {
        self.slot(key)
            .and_then(|slot| self.slots[slot as usize].as_ref())
            .is_some_and(|held| held.stamp == *stamp)
    }

    /// The slot `key` holds, if any.
    pub fn slot(&self, key: K) -> Option<u32> {
        self.by_key.get(&key).copied()
    }

    /// A slot for `key`'s data at `stamp`, wanted this frame: its own when it holds one, else a
    /// free one, else the one wanted longest ago, unless every slot was wanted this frame.
    pub fn admit(&mut self, key: K, stamp: S) -> Option<Admitted<K>> {
        let frame = self.frame;
        if let Some(slot) = self.slot(key) {
            self.slots[slot as usize] = Some(Held {
                key,
                stamp,
                wanted: frame,
            });
            return Some(Admitted {
                slot,
                evicted: None,
            });
        }
        let slot = self.vacancy()?;
        let evicted = self.slots[slot as usize].take().map(|held| held.key);
        if let Some(evicted) = evicted {
            self.by_key.remove(&evicted);
        }
        self.slots[slot as usize] = Some(Held {
            key,
            stamp,
            wanted: frame,
        });
        self.by_key.insert(key, slot);
        Some(Admitted { slot, evicted })
    }

    /// Lets go of the slot `key` holds, if any.
    pub fn release(&mut self, key: K) {
        if let Some(slot) = self.by_key.remove(&key) {
            self.slots[slot as usize] = None;
        }
    }

    /// Whether `key` holds a slot or could take one this frame.
    pub fn has_room(&self, key: K) -> bool {
        self.by_key.contains_key(&key) || self.vacancy().is_some()
    }

    /// A free slot, else the one wanted longest ago before this frame.
    fn vacancy(&self) -> Option<u32> {
        let free = self.slots.iter().position(Option::is_none);
        let oldest = || {
            self.slots
                .iter()
                .enumerate()
                .filter_map(|(slot, held)| Some((slot, held.as_ref()?.wanted)))
                .filter(|&(_, wanted)| wanted < self.frame)
                .min_by_key(|&(_, wanted)| wanted)
                .map(|(slot, _)| slot)
        };
        free.or_else(oldest).map(|slot| slot as u32)
    }
}

#[derive(Debug)]
struct Held<K, S> {
    key: K,
    stamp: S,
    wanted: u64,
}
