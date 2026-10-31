use bevy_ecs::message::MessageCursor;
use bevy_ecs::prelude::*;
use engine::core::time::Seconds;
use engine::systems::area::AreaDef;
use engine::systems::combat;
use engine::systems::movement::position;
use engine::systems::npc::{self, NpcDef};
use engine::systems::rewards::{self, KillCredited};
use engine::systems::{item, stat};
use serde_json::json;
use testkit::{Game, Map, Sim};

#[test]
fn a_kill_credits_the_player_who_reserved_it_once_and_an_unreserved_one_nobody() {
    let (mut sim, player) = field();
    let victim = brute(&mut sim, player);
    item::reserve(&mut sim.world, victim, player, Seconds(0.0));
    kill(&mut sim, victim, player);

    let credited = credits(&mut sim, 3);

    assert_eq!(
        credited,
        vec![KillCredited {
            npc: sim.id::<NpcDef>("brute"),
            victim,
            credited: player,
        }]
    );

    let unreserved = brute(&mut sim, player);
    combat::inflict(&mut sim.world, unreserved, None, 1_000_000.0);
    assert!(credits(&mut sim, 3).is_empty());
}

fn field() -> (Sim, Entity) {
    let content = Game::new()
        .area("field", &Map::open(5, 5), json!([]))
        .hero("field", 2.0, 2.0)
        .npc("brute", 25.0, json!([]))
        .vfx("damage_taken")
        .shader("damage_pop")
        .vfx("defeated")
        .load();
    let mut sim = Sim::new(content);
    sim.serve("field")
        .add(rewards::grant)
        .add((stat::settle_health, combat::defeats));
    let player = sim.join(1);
    (sim, player)
}

fn brute(sim: &mut Sim, near: Entity) -> Entity {
    let at = position(&sim.world, near).expect("player position");
    let area = sim.id::<AreaDef>("field");
    let brute = sim.id::<NpcDef>("brute");
    npc::spawn(&mut sim.world, brute, at, area, None)
}

fn kill(sim: &mut Sim, victim: Entity, killer: Entity) {
    combat::inflict(&mut sim.world, victim, Some(killer), 1_000_000.0);
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
