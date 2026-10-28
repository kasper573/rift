//! What a body shows: the item in each of its hands. Replicated with the actor, so the stance and the
//! props drawn derive from the same state on the server and on every client. A player's is mirrored
//! from its equipped hands; an npc's is written in its file and is looks alone.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ActorModel;
use super::skeleton::Hand;
use super::stance::{self, HandIndex, HandRole, StanceId};
use crate::core::content::Content;
use crate::core::script::{Arrow, Compiler, EffectDef, EffectStep, NEXT, ScriptFault};
use crate::systems::item::ItemId;
use crate::systems::script::event::NPC;
use crate::systems::script::{EffectCall, Reach, RiftScripting, RunnerKind};

/// The item each hand of a body shows; a two-hander sits under both hands it takes.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Held {
    pub hands: BTreeMap<HandIndex, ItemId>,
}

/// A stance a script put a body in with nothing in its hands: it holds until cleared, and gives way
/// to the stance of anything the body's hands show.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ScriptedStance(pub StanceId);

/// Fills the hands of the npc whose script this is with items to show, each placed in turn as
/// equipping places it; what that puts back is left out.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(extend("x-system" = "npc body"))]
pub struct SetHeld {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ItemId>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for SetHeld {
    fn owners(&self) -> &'static [RunnerKind] {
        NPC
    }
    fn needs_subject(&self) -> bool {
        false
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(
        &'c self,
        _cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let items = self.items.clone();
        Ok(Box::new(move |world, _, cx| {
            if let Some(npc) = cx.owner {
                let content = world.resource::<Content>().clone();
                let shown = Held {
                    hands: listed(&content, hands_of(world, npc).len(), &items).hands,
                };
                world.entity_mut(npc).insert(shown);
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        Vec::new()
    }
}

/// The stance the actor stands in: the one its hands show, else the one a script put it in.
pub fn stance_of(world: &World, actor: Entity) -> Option<StanceId> {
    let content = world.resource::<Content>();
    world
        .get::<Held>(actor)
        .and_then(|wearing| stance::of_hands(content, &wearing.hands))
        .or_else(|| {
            world
                .get::<ScriptedStance>(actor)
                .map(|scripted| scripted.0)
        })
}

/// The hands the actor's body has: its model's skeleton's, in order.
pub fn hands_of(world: &World, actor: Entity) -> &[Hand] {
    super::appearance::model_of(world, actor).map_or(&[], ActorModel::hands)
}

/// Where an item lands among `slots` hands already showing `held`, and what it puts back.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placement {
    pub hands: BTreeMap<HandIndex, ItemId>,
    /// What was there and did not slide to a free hand, once each, in hand order.
    pub returned: Vec<ItemId>,
}

/// Puts `item` among `slots` hands showing `held`, in `hand` when one is picked, any hand being
/// fine for any item, else where its stance's hand says: its main hand first, its off hand second,
/// the first free hand for either; it takes as many hands on from there as it needs, or the last
/// ones when too few are left. What was in those hands slides to the next free hand if its own
/// stance allows it there, else comes back.
pub fn place(
    content: &Content,
    slots: usize,
    held: &BTreeMap<HandIndex, ItemId>,
    item: ItemId,
    hand: Option<HandIndex>,
) -> Placement {
    let Some(stance) = stance::of_item(content, item).filter(|_| slots > 0) else {
        return Placement {
            hands: held.clone(),
            returned: Vec::new(),
        };
    };
    let stance = stance.get(content);
    let takes = (stance.hands.0 as usize).clamp(1, slots);
    let free = |start: usize| (start..start + takes).all(|hand| !held.contains_key(&hand_at(hand)));
    let start = match (hand, stance.hand) {
        (Some(hand), _) => hand.index().min(slots - takes),
        (None, HandRole::Main) => 0,
        (None, HandRole::Off) => 1.min(slots - takes),
        (None, HandRole::Either) => (0..=slots - takes).find(|&start| free(start)).unwrap_or(0),
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

/// The hands a wear list shows: each listed item placed in turn as equipping places it, so a list
/// shows what equipping its items in that order would, and what that puts back is left out.
pub fn listed(content: &Content, slots: usize, items: &[ItemId]) -> Placement {
    items.iter().fold(Placement::default(), |shown, &item| {
        let placed = place(content, slots, &shown.hands, item, None);
        Placement {
            hands: placed.hands,
            returned: [shown.returned, placed.returned].concat(),
        }
    })
}

fn hand_at(index: usize) -> HandIndex {
    HandIndex(index as u8)
}
