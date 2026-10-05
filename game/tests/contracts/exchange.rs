use game::data::item::Id;
use game::systems::item::{ExchangeRefusal, Inventory, ItemStack};

fn bag(max: u32, slots: &[ItemStack]) -> Inventory {
    Inventory {
        slots: slots.to_vec(),
        max,
    }
}

#[test]
fn a_hand_in_frees_the_slot_its_reward_needs() {
    let mut inventory = bag(
        2,
        &[ItemStack::new(Id::OrcTusk, 5), ItemStack::new(Id::Bone, 1)],
    );

    let result = inventory.exchange(
        &[ItemStack::new(Id::OrcTusk, 5)],
        &[ItemStack::new(Id::RustySword, 1)],
    );

    assert_eq!(result, Ok(()));
    assert_eq!(inventory.count(Id::OrcTusk), 0);
    assert_eq!(inventory.count(Id::RustySword), 1);
    assert_eq!(inventory.count(Id::Bone), 1);
}

#[test]
fn a_refused_exchange_changes_nothing() {
    let original = bag(
        2,
        &[ItemStack::new(Id::Gold, 10), ItemStack::new(Id::Bone, 1)],
    );

    let mut short = original.clone();
    assert_eq!(
        short.exchange(&[ItemStack::new(Id::Gold, 15)], &[]),
        Err(ExchangeRefusal::Missing(ItemStack::new(Id::Gold, 5)))
    );
    assert_eq!(short, original);

    let mut full = original.clone();
    assert_eq!(
        full.exchange(
            &[ItemStack::new(Id::Gold, 1)],
            &[ItemStack::new(Id::RustySword, 1)]
        ),
        Err(ExchangeRefusal::NoRoom { slots: 1 })
    );
    assert_eq!(full, original);
}

#[test]
fn costs_of_the_same_item_add_up() {
    let mut inventory = bag(5, &[ItemStack::new(Id::Gold, 20)]);
    assert_eq!(
        inventory.exchange(
            &[ItemStack::new(Id::Gold, 15), ItemStack::new(Id::Gold, 10)],
            &[]
        ),
        Err(ExchangeRefusal::Missing(ItemStack::new(Id::Gold, 5)))
    );
}

#[test]
fn gold_stacks_without_limit_in_one_slot() {
    let mut inventory = bag(1, &[]);
    inventory
        .exchange(&[], &[ItemStack::new(Id::Gold, u32::MAX - 10)])
        .expect("gold fits");
    inventory
        .exchange(&[], &[ItemStack::new(Id::Gold, 10)])
        .expect("gold still fits");
    assert_eq!(inventory.slots.len(), 1);
    assert_eq!(inventory.count(Id::Gold), u32::MAX);
}

#[test]
fn refusals_read_as_what_is_needed() {
    assert_eq!(
        ExchangeRefusal::NoRoom { slots: 1 }.describe(),
        "Needs 1 free slot"
    );
    assert_eq!(
        ExchangeRefusal::Missing(ItemStack::new(Id::Gold, 20)).describe(),
        "Needs 20 more Gold"
    );
}
