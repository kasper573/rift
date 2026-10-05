use bevy_ecs::message::MessageCursor;
use bevy_ecs::prelude::*;
use game::core::time::Seconds;
use game::data;
use game::systems::combat::Died;
use game::systems::item;
use game::systems::movement::position;
use game::systems::npc::{self, Pack};
use game::systems::rewards::KillCredited;

use crate::support::Sim;

fn orc(sim: &mut Sim, near: Entity) -> Entity {
    let world = sim.world();
    let at = position(world, near).expect("player position");
    npc::spawn(
        world,
        data::npc::Id::Orc,
        at,
        data::area::SPAWN_ID,
        Pack(u32::MAX),
    )
}

fn kill(sim: &mut Sim, victim: Entity, killer: Entity) {
    sim.world().write_message(Died {
        entity: victim,
        killer,
    });
}

fn credits(sim: &mut Sim, ticks: usize) -> Vec<KillCredited> {
    let mut cursor = MessageCursor::<KillCredited>::default();
    let mut seen = Vec::new();
    for _ in 0..ticks {
        sim.tick();
        seen.extend(sim.read(&mut cursor));
    }
    seen
}

#[test]
fn a_reserved_kill_credits_the_reserving_player_once() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let victim = orc(&mut sim, player);
    item::reserve(sim.world(), victim, player, Seconds(0.0));
    kill(&mut sim, victim, player);

    let credits = credits(&mut sim, 3);

    assert_eq!(
        credits,
        vec![KillCredited {
            npc: data::npc::Id::Orc,
            victim,
            credited: player,
        }]
    );
}

#[test]
fn an_unreserved_kill_credits_nobody() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let victim = orc(&mut sim, player);
    kill(&mut sim, victim, player);

    assert!(credits(&mut sim, 3).is_empty());
}

#[test]
fn deaths_stay_observable_to_every_reader() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let victim = orc(&mut sim, player);
    item::reserve(sim.world(), victim, player, Seconds(0.0));
    kill(&mut sim, victim, player);
    sim.tick();

    assert!(
        sim.read_all::<Died>()
            .iter()
            .any(|died| died.entity == victim)
    );
}
