use bevy_ecs::prelude::*;
use engine::core::math::Pos;
use engine::systems::area::{self, AreaDef};
use engine::systems::movement::Position;
use engine::systems::npc::Npc;
use engine::systems::{region, sleep};
use serde_json::json;
use testkit::{Game, Map, Sim, path};

#[test]
fn an_areas_ground_and_what_stands_on_it_come_to_life_as_a_player_comes_near_and_the_ground_is_let_go_as_they_leave()
 {
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
        .load_to_play();
    let mut sim = Sim::new(content.clone());
    sim.serve("field").add((sleep::settle, region::settle));
    let field = area::load(&content, sim.id::<AreaDef>("field"));
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
    assert!(field.mesh().walkable(Pos::new(540.0, 6.0)));

    sim.world
        .entity_mut(hero)
        .insert(Position::at(Pos::new(4.0, 6.0)));
    assert!(
        sim.run_until(5.0, |_| {
            std::thread::sleep(std::time::Duration::from_millis(5));
            !field.mesh().walkable(Pos::new(540.0, 6.0))
        }),
        "the ground left behind is let go"
    );
    assert!(field.mesh().walkable(Pos::new(20.0, 6.0)));
    assert_eq!(
        crows(&mut sim.world),
        [10.0, 550.0],
        "what came to life stays, once"
    );
}

/// Worlds of one area share its ground. Each has the ground it holds underfoot as soon as it
/// holds it, whichever of them laid it.
#[test]
fn every_world_of_an_area_has_the_ground_it_holds_underfoot_at_once() {
    let content = Game::new()
        .area("field", &Map::open(600, 12), json!([]))
        .hero("field", 4.0, 6.0)
        .load_to_play();
    let field = area::load(&content, Sim::new(content.clone()).id::<AreaDef>("field"));
    let far = Pos::new(540.0, 6.0);
    let tile = field.ground_size().tile_at(far);
    for round in 0..20 {
        let underfoot: Vec<bool> = std::thread::scope(|scope| {
            let worlds: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        let _held = field.hold([tile]);
                        field.mesh().walkable(far)
                    })
                })
                .collect();
            worlds
                .into_iter()
                .map(|world| world.join().expect("a world holds its ground"))
                .collect()
        });
        assert!(
            underfoot.iter().all(|&walks| walks),
            "round {round}: {underfoot:?}"
        );
    }
}
