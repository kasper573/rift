use bevy_ecs::prelude::*;
use bevy_replication::Interest;
use engine::core::math::{Offset, Pos};
use engine::core::units::Meters;
use engine::systems::area::AreaDef;
use engine::systems::minimap::{self, MinimapMarkKind};
use engine::systems::movement::{self, Position, position};
use engine::systems::npc::{self, NpcDef};
use engine::systems::player::ClientId;
use engine::systems::visibility::{self, ClientSight};
use serde_json::json;
use testkit::{Game, Map, Sim};

#[test]
fn a_player_is_shown_what_stands_in_sight_and_is_there_for_them() {
    let mut sim = field();
    let hero = sim.join(1);
    let other = sim.join(2);
    let sight = visibility::sight(&sim.world);
    let near = spawn(&mut sim, hero, Offset::new(sight.0 - 4.0, 0.0));
    let far = spawn(&mut sim, hero, Offset::new(sight.0 + 40.0, 0.0));
    let crow = sim.id::<NpcDef>("crow");
    let theirs = npc::summon(
        &mut sim.world,
        crow,
        1,
        Meters(1.5),
        hero,
        Some(ClientId(2)),
        None,
    )[0];
    sim.tick();

    let (shown, owned) = shown_to(&mut sim, 1);
    assert!(shown.contains(&hero) && shown.contains(&near));
    assert!(!shown.contains(&far), "beyond sight");
    assert!(!shown.contains(&theirs), "there for another player alone");
    assert!(owned.contains(&hero) && !owned.contains(&other));
    assert!(shown_to(&mut sim, 2).0.contains(&theirs));

    let beside = position(&sim.world, hero).expect("a position") + Offset::new(3.0, 0.0);
    sim.world.entity_mut(far).insert(Position::at(beside));
    sim.tick();
    let (shown, _) = shown_to(&mut sim, 1);
    assert!(shown.contains(&far), "come into sight");
}

#[test]
fn a_players_map_marks_the_townsfolk_and_players_in_sight_and_never_a_creature() {
    let mut sim = field();
    let hero = sim.join(1);
    sim.join(2);
    let sight = visibility::sight(&sim.world);
    spawn(&mut sim, hero, Offset::new(sight.0 - 4.0, 0.0));
    spawn(&mut sim, hero, Offset::new(sight.0 + 40.0, 0.0));
    let beside = position(&sim.world, hero).expect("a position") + Offset::new(3.0, 0.0);
    let (wolf, area) = (sim.id::<NpcDef>("wolf"), sim.id::<AreaDef>("field"));
    npc::spawn(&mut sim.world, wolf, beside, area, None);
    sim.tick();

    let mut marked: Vec<(MinimapMarkKind, String)> = minimap::marks(&mut sim.world, hero)
        .into_iter()
        .map(|mark| (mark.kind, mark.name))
        .collect();
    marked.sort_by_key(|(kind, _)| *kind == MinimapMarkKind::Player);
    let kinds: Vec<MinimapMarkKind> = marked.iter().map(|(kind, _)| *kind).collect();
    assert_eq!(
        kinds,
        [MinimapMarkKind::Townsperson, MinimapMarkKind::Player],
        "{marked:?}"
    );
    assert_eq!(marked[0].1, "crow", "the one in sight, by its name");
}

/// What the client playing `client` is shown, and whose private state it is told.
fn shown_to(sim: &mut Sim, client: u32) -> (Vec<Entity>, Vec<Entity>) {
    let conn = sim.conn(client);
    let mut sight = ClientSight::default();
    sight.prepare(&mut sim.world);
    let mut shown = Vec::new();
    sight.seen_by(&sim.world, conn, &mut shown);
    let owned = shown
        .iter()
        .copied()
        .filter(|&entity| sight.owns(&sim.world, conn, entity))
        .collect();
    (shown, owned)
}

fn spawn(sim: &mut Sim, near: Entity, offset: Offset<Meters>) -> Entity {
    let at: Pos<Meters> = position(&sim.world, near).expect("a position") + offset;
    let (crow, area) = (sim.id::<NpcDef>("crow"), sim.id::<AreaDef>("field"));
    npc::spawn(&mut sim.world, crow, at, area, None)
}

fn field() -> Sim {
    let content = Game::new()
        .area("field", &Map::open(120, 12), json!([]))
        .hero("field", 4.0, 6.0)
        .npc("crow", 5.0, json!([]))
        .npc("wolf", 5.0, json!([]))
        .edit("npc", "wolf", |wolf| wolf["attitude"] = json!("hostile"))
        .load();
    let mut sim = Sim::new(content);
    sim.serve("field").add(movement::file_by_chunk);
    sim
}
