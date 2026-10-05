use game::core::tiling::Tiles;
use game::data;
use game::systems::combat::AttackRequest;
use game::systems::movement::position;
use game::systems::npc::{self, Pack};
use game::systems::stat::{self, StatKind};

use crate::support::Sim;

#[test]
fn an_attack_walks_into_range_and_strikes() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let start = position(sim.world(), player).expect("position");
    let spot = sim.walkable_near(start, Tiles(3.0), Tiles(5.0));
    let orc = npc::spawn(
        sim.world(),
        data::npc::Id::Orc,
        spot,
        data::area::SPAWN_ID,
        Pack(u32::MAX),
    );
    let full = stat::effective(sim.world(), orc, StatKind::Health);

    sim.send(1, AttackRequest { target: orc });

    assert!(
        sim.run_until(10.0, |world| stat::effective(world, orc, StatKind::Health)
            < full)
    );
}

#[test]
fn an_attacked_monster_is_fought_until_it_falls() {
    use bevy_ecs::prelude::*;
    use game::core::tiling::TilePos;
    use game::systems::combat::Attitude;
    use game::systems::npc::Npc;

    for seed in 0..6 {
        let mut sim = Sim::area(data::area::SPAWN_ID);
        sim.world()
            .insert_resource(game::core::math::Rng::new(seed));
        let player = sim.join(1);
        sim.run_until(2.0, |_| false);
        let at = position(sim.world(), player).expect("position");
        let world = sim.world();
        let foe = world
            .query_filtered::<Entity, With<Npc>>()
            .iter(world)
            .collect::<Vec<_>>()
            .into_iter()
            .filter(|&npc| {
                !stat::is_dead(world, npc) && world.get::<Attitude>(npc) == Some(&Attitude::Hostile)
            })
            .min_by(|&a, &b| {
                let da = position(world, a).expect("pos").distance(at).0;
                let db = position(world, b).expect("pos").distance(at).0;
                da.total_cmp(&db)
            })
            .expect("a monster");

        sim.send(1, AttackRequest { target: foe });

        assert!(
            sim.run_until(60.0, |world| stat::is_dead(world, foe)),
            "seed {seed}: the monster never fell"
        );
    }
}
