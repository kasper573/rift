use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::{
    BasicAttack, HOTBAR_SLOTS, Hotbar, HotbarEntry, HotbarSlot, ItemId, Learned, SkillId, SkillKind,
};
use crate::core::content::Content;
use crate::systems::equipment::{self, Equipment};
use crate::systems::history::HistoryTopic;
use crate::systems::item::Inventory;
use crate::systems::notification::{self, Notification};
use crate::systems::player::player_requests;

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct AssignHotbarRequest {
    pub slot: HotbarSlot,
    pub entry: HotbarEntry,
}

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct AssignBasicAttackRequest {
    pub pick: BasicAttackPick,
}

/// What the player puts in the basic attack slot: a skill they know, a weapon whose attack they
/// want (worn first if it is still in the bag), or nothing.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum BasicAttackPick {
    Empty,
    Skill(SkillId),
    Weapon(ItemId),
}

pub fn requests(world: &mut World) {
    player_requests::<AssignHotbarRequest>(world, false, |world, player, request| {
        assign(world, player, request.slot, request.entry)
    });
    player_requests::<AssignBasicAttackRequest>(world, false, |world, player, request| {
        assign_basic_attack(world, player, request.pick)
    });
}

/// Puts something the player has onto a slot: a skill they know or an item they hold.
pub fn assign(
    world: &mut World,
    player: Entity,
    slot: HotbarSlot,
    entry: HotbarEntry,
) -> Result<(), String> {
    let content = world.resource::<Content>().clone();
    if slot.0 >= HOTBAR_SLOTS {
        return Err("No such slot".to_owned());
    }
    match entry {
        HotbarEntry::Empty => {}
        HotbarEntry::Skill(skill) => {
            let def = skill.get(&content);
            if !super::known(world, player).contains_key(&skill) {
                return Err(format!("You don't know {}", def.name));
            }
            if def.kind == SkillKind::Passive {
                return Err(super::unslotted(def));
            }
        }
        HotbarEntry::Item(item) => {
            if !holds_item(world, player, item) {
                return Err(format!("You don't have {}", item.get(&content).name));
            }
        }
    }
    let mut hotbar = world.get_mut::<Hotbar>(player).ok_or("No hotbar")?;
    hotbar.set(slot, entry);
    Ok(())
}

/// Puts a known, flagged skill in the basic attack slot, or a weapon's attack once it is worn, or
/// empties the slot.
pub fn assign_basic_attack(
    world: &mut World,
    player: Entity,
    pick: BasicAttackPick,
) -> Result<(), String> {
    let content = world.resource::<Content>().clone();
    let skill = match pick {
        BasicAttackPick::Empty => None,
        BasicAttackPick::Skill(skill) => {
            let def = skill.get(&content);
            if !super::known(world, player).contains_key(&skill) {
                return Err(format!("You don't know {}", def.name));
            }
            if !def.basic_attack {
                return Err(format!("{} can't be a basic attack", def.name));
            }
            Some(skill)
        }
        BasicAttackPick::Weapon(item) => {
            let skill = super::first_flagged_grant(&content, std::iter::once(item))
                .ok_or_else(|| format!("{} has no basic attack", item.get(&content).name))?;
            if !equipment::wears(world, player, item) {
                equipment::equip_from_bag(world, player, item)?;
            }
            Some(skill)
        }
    };
    let mut slot = world
        .get_mut::<BasicAttack>(player)
        .ok_or("No basic attack slot")?;
    slot.0 = skill;
    Ok(())
}

/// Whether an item is in the actor's bag or worn.
pub fn holds_item(world: &World, actor: Entity, item: ItemId) -> bool {
    world
        .get::<Inventory>(actor)
        .is_some_and(|inventory| inventory.count(item) > 0)
        || equipment::wears(world, actor, item)
}

type Stirred = Or<(Changed<Equipment>, Changed<Learned>, Added<BasicAttack>)>;

/// Empties the basic attack slot when its skill stops being known, and fills an empty slot with
/// the first flagged skill the worn equipment grants, so swapping weapons swaps the attack.
pub fn settle_basic_attack(
    world: &mut World,
    changed: &mut QueryState<Entity, (With<BasicAttack>, Stirred)>,
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
        let worn = equipment::worn(world, player);
        let Some(skill) = super::first_flagged_grant(&content, worn) else {
            continue;
        };
        if let Some(mut slot) = world.get_mut::<BasicAttack>(player) {
            slot.0 = Some(skill);
        }
        let def = skill.get(&content);
        notification::notify(
            world,
            player,
            Notification::feed(
                HistoryTopic::Notification,
                format!("{} is your basic attack", def.name),
            )
            .icon(def.icon.path()),
        );
    }
}
