use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::{BasicAttack, HOTBAR_SLOTS, Hotbar, HotbarEntry, Learned, SkillId, SkillKind};
use crate::core::content::Content;
use crate::systems::drain_requests;
use crate::systems::equipment::Equipment;
use crate::systems::history::HistoryTopic;
use crate::systems::item::Inventory;
use crate::systems::notification::{self, Notification, NotificationKind};
use crate::systems::player::sender_player;
use crate::systems::text::LineText;

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct AssignHotbarRequest {
    pub slot: u32,
    pub entry: HotbarEntry,
}

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct AssignBasicAttackRequest {
    pub skill: Option<SkillId>,
}

pub fn requests(world: &mut World) {
    for request in drain_requests::<AssignHotbarRequest>(world) {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        let AssignHotbarRequest { slot, entry } = request.message;
        if let Err(reason) = assign(world, player, slot, entry) {
            notification::error(world, player, reason);
        }
    }
    for request in drain_requests::<AssignBasicAttackRequest>(world) {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        if let Err(reason) = assign_basic_attack(world, player, request.message.skill) {
            notification::error(world, player, reason);
        }
    }
}

/// Puts something the player has onto a slot: a skill they know or an item they hold.
pub fn assign(
    world: &mut World,
    player: Entity,
    slot: u32,
    entry: HotbarEntry,
) -> Result<(), String> {
    let content = world.resource::<Content>().clone();
    if slot >= HOTBAR_SLOTS {
        return Err("No such slot".to_owned());
    }
    match entry {
        HotbarEntry::Empty => {}
        HotbarEntry::Skill(skill) => {
            if !super::known(world, player).contains_key(&skill) {
                return Err(format!("You don't know {}", skill.get(&content).name));
            }
            if skill.get(&content).kind == SkillKind::Passive {
                return Err(format!(
                    "{} is passive: it holds without a slot",
                    skill.get(&content).name
                ));
            }
        }
        HotbarEntry::Item(item) => {
            let held = world
                .get::<Inventory>(player)
                .is_some_and(|inventory| inventory.count(item) > 0)
                || world
                    .get::<Equipment>(player)
                    .is_some_and(|equipment| equipment.slots.values().any(|worn| *worn == item));
            if !held {
                return Err(format!("You don't have {}", item.get(&content).name));
            }
        }
    }
    let mut hotbar = world.get_mut::<Hotbar>(player).ok_or("No hotbar")?;
    hotbar.set(slot, entry);
    Ok(())
}

/// Puts a known, flagged skill in the basic attack slot, or empties it.
pub fn assign_basic_attack(
    world: &mut World,
    player: Entity,
    skill: Option<SkillId>,
) -> Result<(), String> {
    let content = world.resource::<Content>().clone();
    if let Some(skill) = skill {
        let def = skill.get(&content);
        if !super::known(world, player).contains_key(&skill) {
            return Err(format!("You don't know {}", def.name));
        }
        if !def.basic_attack {
            return Err(format!("{} can't be a basic attack", def.name));
        }
    }
    let mut slot = world
        .get_mut::<BasicAttack>(player)
        .ok_or("No basic attack slot")?;
    slot.0 = skill;
    Ok(())
}

type Changed = Or<(
    bevy_ecs::query::Changed<Equipment>,
    bevy_ecs::query::Changed<Learned>,
    Added<BasicAttack>,
)>;

/// Empties the basic attack slot when its skill stops being known, and fills an empty slot with
/// the first flagged skill the worn equipment grants, so swapping weapons swaps the attack.
pub fn settle_basic_attack(
    world: &mut World,
    changed: &mut QueryState<Entity, (With<BasicAttack>, Changed)>,
) {
    let players: Vec<Entity> = changed.iter(world).collect();
    for player in players {
        let content = world.resource::<Content>().clone();
        let known = super::known(world, player);
        let current = world.get::<BasicAttack>(player).and_then(|slot| slot.0);
        if let Some(current) = current
            && !known.contains_key(&current)
            && let Some(mut slot) = world.get_mut::<BasicAttack>(player)
        {
            slot.0 = None;
        }
        let empty = world
            .get::<BasicAttack>(player)
            .is_some_and(|slot| slot.0.is_none());
        if !empty {
            continue;
        }
        let Some(skill) = first_flagged_grant(world, player) else {
            continue;
        };
        if let Some(mut slot) = world.get_mut::<BasicAttack>(player) {
            slot.0 = Some(skill);
        }
        let def = skill.get(&content);
        notification::notify(
            world,
            player,
            Notification::new(
                NotificationKind::Feed {
                    topic: HistoryTopic::Notification,
                    icon: Some(def.icon.path()),
                    tally: None,
                    failure: false,
                    sfx: None,
                },
                LineText::plain(format!("{} is your basic attack", def.name)),
            ),
        );
    }
}

pub fn first_flagged_grant(world: &World, actor: Entity) -> Option<SkillId> {
    let content = world.resource::<Content>();
    super::granted(world, actor)
        .into_iter()
        .find(|skill| skill.get(content).basic_attack)
}
