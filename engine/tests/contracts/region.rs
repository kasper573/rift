use bevy_ecs::prelude::*;
use engine::core::ground::TileAt;
use engine::core::math::Pos;
use engine::systems::area::{self, AreaDef};
use engine::systems::movement::Position;
use engine::systems::npc::Npc;
use engine::systems::{region, sleep};
use serde_json::json;
use testkit::{Game, Map, Sim, path};

#[test]
fn an_areas_ground_and_what_stands_on_it_come_to_life_as_a_player_comes_near() {
    let content = Game::new()
        .npc("crow", 5.0, json!([]))
        .npc("hawk", 5.0, json!([]))
        .area(
            "field",
            &Map::open(600, 12),
            json!([
                { "type": "resident", "npc": path("npc", "crow"), "at": { "x": 10.0, "y": 6.0 } },
                { "type": "resident", "npc": path("npc", "hawk"), "at": { "x": 550.0, "y": 6.0 } },
            ]),
        )
        .hero("field", 4.0, 6.0)
        .edit_settings(|settings| settings["spatial_partition_cell_size"] = json!(1000.0))
        .load_to_play();
    let mut sim = Sim::new(content.clone());
    sim.serve("field").add((sleep::settle, region::settle));
    let field = area::load(&content, sim.id::<AreaDef>("field"));
    let (near, far) = (TileAt { column: 0, row: 0 }, TileAt { column: 2, row: 0 });
    let crows = |world: &mut World| -> Vec<f32> {
        let mut crows: Vec<f32> = world
            .query_filtered::<&Position, With<Npc>>()
            .iter(world)
            .map(|position| position.pos.x.round())
            .collect();
        crows.sort_by(f32::total_cmp);
        crows
    };

    let hero = sim.join(1);
    sim.tick();
    assert_eq!(
        crows(&mut sim.world),
        [10.0],
        "only what stands near the player"
    );
    assert!(field.held().contains(&near) && !field.held().contains(&far));
    assert!(field.mesh().walkable(Pos::new(20.0, 6.0)));
    assert!(!field.mesh().walkable(Pos::new(540.0, 6.0)));

    sim.world
        .entity_mut(hero)
        .insert(Position::at(Pos::new(540.0, 6.0)));
    // The ground is held on a thread of its own, which takes real time, not the world's.
    assert!(
        sim.run_until(5.0, |world| {
            std::thread::sleep(std::time::Duration::from_millis(5));
            crows(world).len() == 2
        }),
        "the far ground reached: {:?}",
        crows(&mut sim.world)
    );
    assert_eq!(crows(&mut sim.world), [10.0, 550.0]);
    assert!(field.held().contains(&far));
    assert!(field.mesh().walkable(Pos::new(540.0, 6.0)));

    sim.world
        .entity_mut(hero)
        .insert(Position::at(Pos::new(4.0, 6.0)));
    sim.run(1.0);
    assert_eq!(
        crows(&mut sim.world),
        [10.0, 550.0],
        "what came to life stays, once"
    );
}
