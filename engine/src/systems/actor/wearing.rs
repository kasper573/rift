//! What a body shows: the item in each of its hands. Replicated with the actor, so the stance and the
//! props drawn derive from the same state on the server and on every client. A player's is mirrored
//! from its equipped hands; an npc's is written in its file and is looks alone.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::skeleton::Hand;
use super::stance::{self, HandIndex, HandRole, StanceId};
use super::{Actor, ActorModel};
use crate::core::content::Content;
use crate::systems::item::ItemId;

/// The item each hand of a body shows; a two-hander sits under both hands it takes.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Wearing {
    pub hands: BTreeMap<HandIndex, ItemId>,
}

/// A stance a script put a body in with nothing in its hands: it holds until cleared, and gives way
/// to the stance of anything the body's hands show.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ScriptedStance(pub StanceId);

/// The stance the actor stands in: the one its hands show, else the one a script put it in.
pub fn stance_of(world: &World, actor: Entity) -> Option<StanceId> {
    let content = world.resource::<Content>();
    world
        .get::<Wearing>(actor)
        .and_then(|wearing| stance::of_hands(content, &wearing.hands))
        .or_else(|| {
            world
                .get::<ScriptedStance>(actor)
                .map(|scripted| scripted.0)
        })
}

/// The hands the actor's body has: its model's skeleton's, in order.
pub fn hands_of(world: &World, actor: Entity) -> &[Hand] {
    let content = world.resource::<Content>();
    world
        .get::<Actor>(actor)
        .map_or(&[], |actor| ActorModel::get(content, actor.model).hands())
}

/// Where an item lands among `slots` hands already showing `held`, and what it puts back.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placement {
    pub hands: BTreeMap<HandIndex, ItemId>,
    /// What was there and did not slide to a free hand, once each, in hand order.
    pub returned: Vec<ItemId>,
}

/// Puts `item` where its stance's hand says among `slots` hands showing `held`: its main hand first,
/// its off hand second, the first free hand for either, as many hands on from there as it takes.
/// What was in those hands slides to the next free hand if its own stance allows it there, else
/// comes back.
pub fn place(
    content: &Content,
    slots: usize,
    held: &BTreeMap<HandIndex, ItemId>,
    item: ItemId,
) -> Placement {
    let slots = slots.max(1);
    let Some(stance) = stance::of_item(content, item) else {
        return Placement {
            hands: held.clone(),
            returned: Vec::new(),
        };
    };
    let stance = stance.get(content);
    let takes = (stance.hands.0 as usize).clamp(1, slots);
    let free = |start: usize| (start..start + takes).all(|hand| !held.contains_key(&hand_at(hand)));
    let start = match stance.hand {
        HandRole::Main => 0,
        HandRole::Off => 1.min(slots - takes),
        HandRole::Either => (0..=slots - takes).find(|&start| free(start)).unwrap_or(0),
    };
    let taken: Vec<HandIndex> = (start..start + takes).map(hand_at).collect();
    let mut displaced: Vec<ItemId> = Vec::new();
    for hand in &taken {
        if let Some(&there) = held.get(hand)
            && !displaced.contains(&there)
        {
            displaced.push(there);
        }
    }
    let mut hands: BTreeMap<HandIndex, ItemId> = held
        .iter()
        .filter(|(hand, there)| !taken.contains(hand) && !displaced.contains(there))
        .map(|(&hand, &there)| (hand, there))
        .collect();
    for hand in taken {
        hands.insert(hand, item);
    }
    let mut returned = Vec::new();
    for there in displaced {
        let next = (0..slots)
            .map(hand_at)
            .find(|hand| !hands.contains_key(hand));
        let slides = next
            .zip(stance::of_item(content, there))
            .filter(|(hand, stance)| {
                let stance = stance.get(content);
                stance.hands.0 == 1
                    && match stance.hand {
                        HandRole::Either => true,
                        HandRole::Main => *hand == HandIndex::MAIN,
                        HandRole::Off => *hand == HandIndex::OFF,
                    }
            });
        match slides {
            Some((hand, _)) => {
                hands.insert(hand, there);
            }
            None => returned.push(there),
        }
    }
    Placement { hands, returned }
}

/// The hands a wear list shows, listed items filling the body's hands in order, each taking as many
/// as its stance says; what does not fit is left out.
pub fn listed(content: &Content, slots: usize, items: &[ItemId]) -> BTreeMap<HandIndex, ItemId> {
    let mut hands = BTreeMap::new();
    let mut next = 0;
    for &item in items {
        let takes =
            stance::of_item(content, item).map_or(1, |stance| stance.get(content).hands.0 as usize);
        if next + takes > slots {
            break;
        }
        for hand in next..next + takes {
            hands.insert(hand_at(hand), item);
        }
        next += takes;
    }
    hands
}

/// How many hands a wear list asks of a body.
pub fn asked(content: &Content, items: &[ItemId]) -> usize {
    items
        .iter()
        .map(|&item| {
            stance::of_item(content, item).map_or(1, |stance| stance.get(content).hands.0 as usize)
        })
        .sum()
}

fn hand_at(index: usize) -> HandIndex {
    HandIndex(index as u8)
}
