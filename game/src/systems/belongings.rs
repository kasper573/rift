use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::core::sfx::SfxId;
use crate::data::item::Id as ItemId;
use crate::systems::equipment::{Equipment, EquipmentSlot};
use crate::systems::history::{HistoryTopic, RecordTally};
use crate::systems::item::Inventory;
use crate::systems::job;
use crate::systems::notification::{self, Notification, NotificationKind};
use crate::systems::player::Xp;
use crate::systems::text::LineText;

#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Belongings {
    items: BTreeMap<ItemId, u32>,
    worn: BTreeMap<EquipmentSlot, ItemId>,
    xp: u32,
    level: u32,
}

pub type ChangedBelongings = Or<(Changed<Inventory>, Changed<Equipment>, Changed<Xp>)>;

pub fn of(world: &World, player: Entity) -> Belongings {
    let mut items: BTreeMap<ItemId, u32> = BTreeMap::new();
    let worn = world
        .get::<Equipment>(player)
        .map(|equipment| equipment.slots.clone())
        .unwrap_or_default();
    let carried = world
        .get::<Inventory>(player)
        .into_iter()
        .flat_map(|inventory| inventory.slots.iter().map(|slot| (slot.item, slot.count)));
    for (item, count) in carried.chain(worn.values().map(|&item| (item, 1))) {
        *items.entry(item).or_default() += count;
    }
    Belongings {
        items,
        worn,
        xp: world.get::<Xp>(player).map_or(0, |xp| xp.amount),
        level: job::level(world, player),
    }
}

pub fn notify_changes(
    world: &mut World,
    changed: &mut QueryState<Entity, (With<Belongings>, ChangedBelongings)>,
) {
    let players: Vec<Entity> = changed.iter(world).collect();
    for player in players {
        let now = of(world, player);
        let Some(before) = world.get::<Belongings>(player).cloned() else {
            continue;
        };
        if before == now {
            continue;
        }
        world.entity_mut(player).insert(now.clone());
        for notification in changes(&before, &now) {
            notification::notify(world, player, notification);
        }
    }
}

fn changes(before: &Belongings, now: &Belongings) -> Vec<Notification> {
    let items: Vec<(ItemId, i32)> = before
        .items
        .keys()
        .chain(now.items.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|&item| {
            let had = before.items.get(&item).copied().unwrap_or(0) as i32;
            let has = now.items.get(&item).copied().unwrap_or(0) as i32;
            (item, has - had)
        })
        .filter(|&(_, change)| change != 0)
        .collect();
    let lost = items.iter().filter(|(_, change)| *change < 0);
    let gained = items.iter().filter(|(_, change)| *change > 0);
    let mut told: Vec<Notification> = lost
        .chain(gained)
        .map(|&(item, change)| item_row(item, change))
        .collect();
    if now.xp > before.xp {
        told.push(Notification::new(
            NotificationKind::Feed {
                topic: HistoryTopic::Item,
                icon: None,
                tally: Some(RecordTally::Change((now.xp - before.xp) as i32)),
                failure: false,
                sfx: None,
            },
            LineText::plain("XP"),
        ));
    }
    for slot in EquipmentSlot::all() {
        match (before.worn.get(slot), now.worn.get(slot)) {
            (_, Some(&on)) if before.worn.get(slot) != Some(&on) => {
                told.push(worn_row(format!("Equipped {}", on.get().display_name), on));
            }
            (Some(&off), None) => {
                told.push(worn_row(
                    format!("Took off {}", off.get().display_name),
                    off,
                ));
            }
            _ => {}
        }
    }
    if now.level > before.level {
        told.push(Notification::new(
            NotificationKind::Milestone {
                label: "Level up".into(),
                topic: HistoryTopic::Notification,
                sfx: Some(SfxId::RisingChime),
            },
            LineText::plain(format!("Level {}", now.level)),
        ));
    }
    told
}

fn item_row(item: ItemId, change: i32) -> Notification {
    let sounds = &item.get().sfx;
    let sfx = match change > 0 {
        true => sounds.pickup,
        false => sounds.trade,
    };
    Notification::new(
        NotificationKind::Feed {
            topic: HistoryTopic::Item,
            icon: Some(item.get().icon.0.to_owned()),
            tally: Some(RecordTally::Change(change)),
            failure: false,
            sfx: Some(sfx),
        },
        LineText::plain(item.get().display_name),
    )
}

fn worn_row(text: String, item: ItemId) -> Notification {
    Notification::new(
        NotificationKind::Feed {
            topic: HistoryTopic::Item,
            icon: Some(item.get().icon.0.to_owned()),
            tally: None,
            failure: false,
            sfx: Some(item.get().sfx.trade),
        },
        LineText::plain(text),
    )
}
