use std::sync::LazyLock;

use game::core::tiling::Tiles;
use game::systems::combat::{AttackRequest, Attitude};
use game::systems::memory::Remembers;
use game::systems::movement::position;
use game::systems::npc::{self, Pack};
use game::systems::player::ClientId;
use game::systems::reach::{self, ReachAct};
use game::systems::rule::Not;
use game::systems::stat::{self, StatKind};
use game::systems::visibility::{self, Presence};

use crate::support::{CLOCK, Sim, content, row, spawn_area};

fn orc_near(sim: &mut Sim, player: bevy_ecs::entity::Entity) -> bevy_ecs::entity::Entity {
    let start = position(sim.world(), player).expect("position");
    let spot = sim.walkable_near(start, Tiles(2.0), Tiles(4.0));
    npc::spawn(sim.world(), row("Orc"), spot, spawn_area(), Pack(u32::MAX))
}

#[test]
fn an_npc_spawned_for_one_player_cannot_be_fought_by_another() {
    let mut sim = Sim::area(spawn_area());
    let owner = sim.join(1);
    sim.join(2);
    let orc = orc_near(&mut sim, owner);
    sim.world()
        .entity_mut(orc)
        .insert(Presence::For(ClientId(1)));
    let full = stat::effective(sim.world(), orc, StatKind::Health);

    sim.send(2, AttackRequest { target: orc });
    sim.run_until(4.0, |_| false);
    assert_eq!(stat::effective(sim.world(), orc, StatKind::Health), full);

    sim.send(1, AttackRequest { target: orc });
    assert!(
        sim.run_until(10.0, |world| stat::effective(world, orc, StatKind::Health)
            < full)
    );
}

#[test]
fn a_friendly_npc_cannot_be_attacked() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    let orc = orc_near(&mut sim, player);
    sim.world().entity_mut(orc).insert(Attitude::Friendly);
    let full = stat::effective(sim.world(), orc, StatKind::Health);

    sim.send(1, AttackRequest { target: orc });
    sim.run_until(4.0, |_| false);

    assert_eq!(reach::intent(sim.world(), player, ReachAct::Attack), None);
    assert_eq!(stat::effective(sim.world(), orc, StatKind::Health), full);
}

#[test]
fn a_monster_spawned_for_one_player_ignores_everyone_else() {
    let mut sim = Sim::area(spawn_area());
    let owner = sim.join(1);
    let bystander = sim.join(2);
    let start = position(sim.world(), bystander).expect("position");
    let spot = sim.walkable_near(start, Tiles(1.0), Tiles(2.0));
    let skeleton = npc::spawn(
        sim.world(),
        row("Skeleton"),
        spot,
        spawn_area(),
        Pack(u32::MAX),
    );
    sim.world()
        .entity_mut(skeleton)
        .insert(Presence::For(ClientId(1)));

    let mut hunted = Vec::new();
    sim.run_until(3.0, |world| {
        hunted.extend(reach::intent(world, skeleton, ReachAct::Attack));
        false
    });

    assert!(hunted.contains(&owner));
    assert!(!hunted.contains(&bystander));
}

#[test]
fn presence_by_requirement_follows_the_players_memory() {
    static PELL_DEAD: LazyLock<Remembers> = LazyLock::new(|| Remembers(row("PellDead")));
    static PELL_ALIVE: LazyLock<Not> = LazyLock::new(|| Not(&*PELL_DEAD));
    static SHOWN: LazyLock<[&dyn game::systems::rule::Requirement; 1]> =
        LazyLock::new(|| [&*PELL_ALIVE]);
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    let orc = orc_near(&mut sim, player);
    sim.world().entity_mut(orc).insert(Presence::When(&*SHOWN));
    assert!(visibility::present(sim.world(), orc, player));

    sim.world()
        .get_mut::<game::systems::memory::Memory>(player)
        .expect("memory")
        .remember(content(), row("PellDead"), CLOCK);

    assert!(!visibility::present(sim.world(), orc, player));
}
