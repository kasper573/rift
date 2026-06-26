//! Inventory stacking and loot reservation are pure rules over the public item table, exercised here
//! without a running app: stacks fill before new slots open, non-stackable items take a slot each,
//! capacity is honest about what will fit, and a reservation lapses on its deadline.

use world::core::table::Id;
use world::core::time::Seconds;
use world::systems::item::{Inventory, ItemDef, Reservation, ReservedBy, Slot};
use world::systems::player::ClientId;

fn item(name: &str) -> Id<ItemDef> {
    Id::by_name(name).expect("a known item id")
}

#[test]
fn stacks_fill_before_opening_new_slots() {
    let potion = item("health_potion"); // stackable to 10
    let mut inventory = Inventory {
        slots: Vec::new(),
        max: 3,
    };
    inventory.add(potion, 12);
    assert_eq!(
        inventory.slots,
        vec![
            Slot {
                item: potion,
                count: 10
            },
            Slot {
                item: potion,
                count: 2
            },
        ]
    );
}

#[test]
fn capacity_counts_partial_stacks_and_free_slots() {
    let potion = item("health_potion");
    let inventory = Inventory {
        slots: vec![Slot {
            item: potion,
            count: 7,
        }],
        max: 2,
    };
    // 3 left in the open stack, plus one empty slot that holds another full 10.
    assert_eq!(inventory.capacity_for(potion), 13);
}

#[test]
fn non_stackable_items_take_one_slot_each() {
    let sword = item("rusty_sword"); // not stackable
    let mut inventory = Inventory {
        slots: Vec::new(),
        max: 5,
    };
    inventory.add(sword, 3);
    assert_eq!(inventory.slots.len(), 3);
    assert!(inventory.slots.iter().all(|slot| slot.count == 1));
    assert_eq!(inventory.capacity_for(sword), 2);
}

#[test]
fn a_full_inventory_has_no_room() {
    let potion = item("health_potion");
    let inventory = Inventory {
        slots: vec![Slot {
            item: potion,
            count: 10,
        }],
        max: 1,
    };
    assert_eq!(inventory.capacity_for(potion), 0);
    assert_eq!(inventory.capacity_for(item("rusty_sword")), 0);
}

#[test]
fn a_reservation_lapses_on_its_deadline() {
    let held = Reservation {
        by: ReservedBy::Account(ClientId(1)),
        at: Seconds(10.0),
    };
    assert!(!held.expired(Seconds(60.0)));
    assert!(held.expired(Seconds(70.0)));
}
