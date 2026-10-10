use bevy_ecs::prelude::*;
use game::core::time::WallClock;
use game::data::memory::Id as MemoryId;
use game::data::npc::Id as NpcId;
use game::systems::combat::{self, AttackRequest, Attitude};
use game::systems::memory::{self, Memory, MemoryKind};
use game::systems::npc::{ShownFor, SpawnNear, SpawnNpcs, Summoned, TurnHostile};
use game::systems::player::ClientId;
use game::systems::rule::{Encounter, Terms};
use game::systems::stat::{self, StatKind};
use game::systems::visibility::{self, Presence};

use crate::support::{
    Sim, content, count, give, heard_the_news, later, pick, row, settle, spawn_area, talk,
    townsperson,
};

fn summoned(sim: &mut Sim, what: NpcId) -> Vec<Entity> {
    let world = sim.world();
    world
        .query::<(Entity, &game::systems::npc::Npc)>()
        .iter(world)
        .filter(|(_, npc)| npc.def == what)
        .map(|(entity, _)| entity)
        .collect()
}

fn remember(sim: &mut Sim, player: Entity, key: MemoryId) {
    let clock = *sim.world().resource::<WallClock>();
    sim.world()
        .get_mut::<Memory>(player)
        .expect("memory")
        .remember(content(), key, clock);
}

fn lose_at_dice(sim: &mut Sim, client: u32, player: Entity) {
    let mut node = talk(sim, client, player, row("Pell")).node;
    for _ in 0..40 {
        let roll = if node == row("PellHello") {
            "Roll the dice."
        } else if node == row("PellLoses") {
            "Roll again."
        } else if node == row("PellWins") {
            return;
        } else {
            panic!("Pell is at {node:?}");
        };
        node = pick(sim, client, player, roll)
            .expect("still at the table")
            .node;
    }
    panic!("never lost a roll");
}

fn health(sim: &mut Sim, who: Entity) -> f32 {
    stat::effective(sim.world(), who, StatKind::Health)
}

#[test]
fn pells_dice_take_ten_gold_a_roll_and_pay_twenty_on_a_win() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    heard_the_news(&mut sim, player);
    give(&mut sim, player, row("Gold"), 400);

    let mut node = talk(&mut sim, 1, player, row("Pell")).node;
    let (mut won, mut lost) = (false, false);
    for _ in 0..30 {
        let before = count(&mut sim, player, row("Gold"));
        let roll = if node == row("PellHello") {
            "Roll the dice."
        } else {
            "Roll again."
        };
        node = pick(&mut sim, 1, player, roll).expect("at the table").node;
        let after = count(&mut sim, player, row("Gold"));
        if node == row("PellLoses") {
            assert_eq!(after, before + 10);
            won = true;
        } else if node == row("PellWins") {
            assert_eq!(after, before - 10);
            lost = true;
        } else {
            panic!("the dice landed on {node:?}");
        }
    }
    assert!(won && lost, "thirty rolls went only one way");
}

#[test]
fn calling_pell_a_cheat_sends_guards_after_you_alone() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    let bystander = sim.join(2);
    heard_the_news(&mut sim, player);
    give(&mut sim, player, row("Gold"), 400);

    lose_at_dice(&mut sim, 1, player);
    assert!(pick(&mut sim, 1, player, "You're cheating.").is_none());

    let guards = summoned(&mut sim, row("HarbourGuard"));
    assert_eq!(guards.len(), 2);
    for &guard in &guards {
        assert!(matches!(
            sim.world().get::<Presence>(guard),
            Some(Presence::For(ClientId(1)))
        ));
        assert_eq!(
            sim.world().get::<Summoned>(guard),
            Some(&Summoned { owner: player })
        );
        assert!(visibility::present(sim.world(), guard, player));
        assert!(!visibility::present(sim.world(), guard, bystander));
    }
    assert!(memory::recall(sim.world(), player, row("PellGrudge")).is_some());

    let (full, calm) = (health(&mut sim, player), health(&mut sim, bystander));
    assert!(sim.run_until(15.0, |world| stat::effective(
        world,
        player,
        StatKind::Health
    ) < full));
    assert_eq!(health(&mut sim, bystander), calm);

    assert_eq!(
        talk(&mut sim, 1, player, row("Pell")).node,
        row("PellGrudging")
    );
}

#[test]
fn fighting_pell_leaves_him_dead_in_your_world_only() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    let bystander = sim.join(2);
    heard_the_news(&mut sim, player);
    remember(&mut sim, player, row("PellGrudge"));
    let pell = townsperson(&mut sim, row("Pell"));

    talk(&mut sim, 1, player, row("Pell"));
    assert!(pick(&mut sim, 1, player, "Then let's settle this.").is_none());
    let copies = summoned(&mut sim, row("PellHostile"));
    assert_eq!(copies.len(), 1);
    let copy = copies[0];
    assert!(!visibility::present(sim.world(), pell, player));
    assert!(visibility::present(sim.world(), pell, bystander));
    assert!(!visibility::present(sim.world(), copy, bystander));

    sim.send(1, AttackRequest { target: copy });
    assert!(
        sim.run_until(60.0, |world| stat::is_dead(world, copy)),
        "the copy never fell"
    );
    settle(&mut sim);
    assert!(memory::recall(sim.world(), player, row("PellDead")).is_some());
    assert!(memory::recall(sim.world(), player, row("PellFighting")).is_none());
    assert!(!visibility::present(sim.world(), pell, player));
    assert!(visibility::present(sim.world(), pell, bystander));

    let MemoryKind::Timer(lasts) = row::<MemoryId>("PellDead").get(content()).kind else {
        panic!("Pell stays dead for a while, not forever");
    };
    later(&mut sim, lasts.0);
    assert!(visibility::present(sim.world(), pell, player));
}

#[test]
fn summons_leave_with_their_owner() {
    let guards = SpawnNpcs {
        npc: row("HarbourGuard"),
        count: 2,
        near: SpawnNear::Player,
        shown: ShownFor::Everyone,
    };
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    Terms {
        requires: &[],
        costs: &[],
        outcomes: &[&guards],
    }
    .settle(sim.world(), player, Encounter::default())
    .expect("summoned");
    assert_eq!(summoned(&mut sim, row("HarbourGuard")).len(), 2);

    sim.world().despawn(player);
    sim.tick();
    assert!(summoned(&mut sim, row("HarbourGuard")).is_empty());
}

#[test]
fn a_townsperson_turned_hostile_can_be_fought_until_they_return_friendly() {
    static TURN: TurnHostile = TurnHostile;
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    let bram = townsperson(&mut sim, row("Bram"));
    assert!(!combat::attackable(sim.world(), player, bram));

    Terms {
        requires: &[],
        costs: &[],
        outcomes: &[&TURN],
    }
    .settle(
        sim.world(),
        player,
        Encounter {
            with: Some(bram),
            tether: None,
        },
    )
    .expect("turned");
    assert!(combat::attackable(sim.world(), player, bram));

    stat::apply_damage(sim.world(), bram, 10_000.0);
    let respawn = row::<NpcId>("Bram")
        .get(content())
        .respawn()
        .expect("Bram returns")
        .0;
    assert!(sim.run_until(respawn + 5.0, |world| !stat::is_dead(world, bram)));
    assert_eq!(sim.world().get::<Attitude>(bram), Some(&Attitude::Friendly));
}
