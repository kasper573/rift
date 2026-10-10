use std::sync::LazyLock;

use game::data;
use game::systems::item::{GiveItems, Holding, Inventory, ItemStack};
use game::systems::job::MinLevel;
use game::systems::player::Xp;
use game::systems::rule::{AnyOf, Encounter, Not, Requirement, RuleRefusal, Terms};

use crate::support::{Sim, content, row};

static REWARD_ITEMS: LazyLock<[ItemStack; 1]> =
    LazyLock::new(|| [ItemStack::new(row("RustySword"), 1)]);
static REWARD: LazyLock<GiveItems> = LazyLock::new(|| GiveItems(&*REWARD_ITEMS));
static HOLDING_GOLD: LazyLock<Holding> = LazyLock::new(|| Holding(ItemStack::new(row("Gold"), 20)));
static GOLD_OR_LEVEL_TERMS: LazyLock<[&dyn Requirement; 2]> =
    LazyLock::new(|| [&*HOLDING_GOLD, &MinLevel(2)]);
static GOLD_OR_LEVEL: LazyLock<AnyOf> = LazyLock::new(|| AnyOf(&*GOLD_OR_LEVEL_TERMS));

fn give(sim: &mut Sim, player: bevy_ecs::entity::Entity, stack: ItemStack) {
    sim.world()
        .get_mut::<Inventory>(player)
        .expect("bag")
        .exchange(content(), &[], &[stack])
        .expect("fits");
}

#[test]
fn level_requirements_follow_experience() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let needed = row::<data::job::Id>("Adventurer").get(content()).levels[1].exp;

    assert!(!MinLevel(2).met(sim.world(), player));
    sim.world().get_mut::<Xp>(player).expect("xp").gain(needed);
    assert!(MinLevel(2).met(sim.world(), player));
    assert!(!Not(&MinLevel(2)).met(sim.world(), player));
}

#[test]
fn any_of_holds_when_one_of_its_requirements_does() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    assert!(!GOLD_OR_LEVEL.met(sim.world(), player));
    give(&mut sim, player, ItemStack::new(row("Gold"), 20));
    assert!(GOLD_OR_LEVEL.met(sim.world(), player));
}

#[test]
fn settling_pays_costs_and_grants_outcomes_in_one_step() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    give(&mut sim, player, ItemStack::new(row("Gold"), 25));
    let terms = Terms {
        requires: &[&*HOLDING_GOLD],
        costs: &[ItemStack::new(row("Gold"), 20)],
        outcomes: &[&*REWARD],
    };

    assert_eq!(
        terms.settle(sim.world(), player, Encounter::default()),
        Ok(())
    );

    let inventory = sim.world().get::<Inventory>(player).expect("bag").clone();
    assert_eq!(inventory.count(row("Gold")), 5);
    assert_eq!(inventory.count(row("RustySword")), 1);
}

#[test]
fn unmet_terms_are_refused_and_change_nothing() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    give(&mut sim, player, ItemStack::new(row("Gold"), 5));
    let before = sim.world().get::<Inventory>(player).expect("bag").clone();
    let locked = Terms {
        requires: &[&MinLevel(3)],
        costs: &[],
        outcomes: &[&*REWARD],
    };
    let unaffordable = Terms {
        requires: &[],
        costs: &[ItemStack::new(row("Gold"), 20)],
        outcomes: &[&*REWARD],
    };

    assert_eq!(
        locked.settle(sim.world(), player, Encounter::default()),
        Err(RuleRefusal("Needs Level 3".to_owned()))
    );
    assert_eq!(
        unaffordable.settle(sim.world(), player, Encounter::default()),
        Err(RuleRefusal("Needs 15 more Gold".to_owned()))
    );
    assert_eq!(sim.world().get::<Inventory>(player).expect("bag"), &before);
}
