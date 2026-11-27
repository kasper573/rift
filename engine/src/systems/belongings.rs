use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;

use crate::core::content::{Content, Fixture};
use crate::core::text::LineText;
use crate::systems::class::{self, Class, SkillPoints};
use crate::systems::equipment::{self, Equipment};
use crate::systems::history::{HistoryTopic, RecordTally};
use crate::systems::interface::InterfaceSound;
use crate::systems::item::inline::InlineItem;
use crate::systems::item::{Inventory, ItemId};
use crate::systems::messages::{self, messages};
use crate::systems::notification::{self, Notification, NotificationKind};
use crate::systems::player::Xp;
use crate::systems::skill::Learned;

#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Belongings {
    items: BTreeMap<ItemId, u32>,
    worn: BTreeSet<ItemId>,
    xp: u32,
    level: u32,
    points: SkillPoints,
}

pub type ChangedBelongings = Or<(
    Changed<Inventory>,
    Changed<Equipment>,
    Changed<Xp>,
    Changed<Class>,
    Changed<Learned>,
)>;

pub fn of(world: &World, player: Entity) -> Belongings {
    let mut items: BTreeMap<ItemId, u32> = BTreeMap::new();
    let worn: BTreeSet<ItemId> = equipment::worn(world, player).collect();
    let carried = world
        .get::<Inventory>(player)
        .into_iter()
        .flat_map(|inventory| {
            inventory
                .stacks()
                .map(|(_, stack)| (stack.item, stack.count))
        });
    for (item, count) in carried.chain(worn.iter().map(|&item| (item, 1))) {
        *items.entry(item).or_default() += count;
    }
    Belongings {
        items,
        worn,
        xp: world.get::<Xp>(player).map_or(0, |xp| xp.amount),
        level: class::level(world, player),
        points: class::points_to_spend(world, player),
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
        let content = world.resource::<Content>().clone();
        for notification in changes(&content, &before, &now) {
            notification::notify(world, player, None, notification);
        }
    }
}

fn changes(content: &Content, before: &Belongings, now: &Belongings) -> Vec<Notification> {
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
    let messages = &messages::of(content).belongings;
    let lost = items.iter().filter(|(_, change)| *change < 0);
    let gained = items.iter().filter(|(_, change)| *change > 0);
    let mut told: Vec<Notification> = lost
        .chain(gained)
        .map(|&(item, change)| item_row(content, item, change))
        .collect();
    if now.xp > before.xp {
        told.push(
            Notification::feed(HistoryTopic::Item, messages.xp())
                .tally(RecordTally::Change(change(before.xp, now.xp)))
                .into(),
        );
    }
    for &on in now.worn.difference(&before.worn) {
        told.push(worn_row(content, messages.equipped(InlineItem::of(on)), on));
    }
    for &off in before.worn.difference(&now.worn) {
        told.push(worn_row(
            content,
            messages.took_off(InlineItem::of(off)),
            off,
        ));
    }
    if now.level > before.level {
        told.push(Notification::new(
            NotificationKind::Milestone {
                label: messages.level_up().to_owned().into(),
                topic: HistoryTopic::Notification,
                sfx: Some(InterfaceSound::LevelUp.id(content)),
            },
            LineText::plain(messages.level_reached(now.level)),
        ));
    }
    if now.points > before.points {
        told.push(
            Notification::feed(HistoryTopic::Notification, messages.points_to_spend())
                .tally(RecordTally::Change(change(before.points.0, now.points.0)))
                .into(),
        );
    }
    told
}

fn item_row(content: &Content, item: ItemId, change: i32) -> Notification {
    let sounds = &item.get(content).sfx;
    let sfx = if change > 0 {
        sounds.pickup
    } else {
        sounds.trade
    };
    Notification::feed(HistoryTopic::Item, InlineItem::of(item))
        .tally(RecordTally::Change(change))
        .sound(sfx)
        .into()
}

/// How far a count moved, as a tally shows it.
fn change(before: u32, now: u32) -> i32 {
    i32::try_from(i64::from(now) - i64::from(before)).unwrap_or(i32::MAX)
}

fn worn_row(content: &Content, text: LineText, item: ItemId) -> Notification {
    Notification::feed(HistoryTopic::Item, text)
        .sound(item.get(content).sfx.trade)
        .into()
}

messages! {
    /// What the game says of the player's belongings.
    pub struct BelongingsMessages {
        /// What the feed names experience gained.
        xp,
        /// What the feed says of the item `{item}` the player put on.
        equipped[item: Line],
        /// What the feed says of the item `{item}` the player took off.
        took_off[item: Line],
        /// The label of the notice that the player reached a level.
        level_up,
        /// What the notice says of the level `{level}` the player reached.
        level_reached(level: Number),
        /// What the feed names skill points gained.
        points_to_spend,
    }
}
