use bevy::math::Vec3;
use bevy_ecs::prelude::*;
use engine::core::math::{Offset, Pos};
use engine::core::time::{TimeCourse, WeatherClock};
use engine::core::units::Meters;
use engine::core::vfx::Stage;
use engine::core::vfx::rain::RAIN_CELL;
use engine::systems::area::rain::falling_on;
use engine::systems::area::spawnable::SpawnedObject;
use engine::systems::area::{self, AreaObject};
use engine::systems::dialogue::{self, ConversationRequest};
use engine::systems::interact::{self, InteractRequest};
use engine::systems::item::{self, UseItemRequest};
use engine::systems::movement::{self, position};
use engine::systems::skill::{self, CastAim, SkillDef, UseSkillRequest};
use engine::systems::status::{self, StatusEffectDef};
use engine::systems::{actor, region, script};
use serde_json::json;
use testkit::{Game, Map, Sim, box_glb, dialogue, path};

/// How many metres the moor is across: a few stretches of its field of rain.
const MOOR: usize = 384;
/// Where a roof stands on the moor.
const ROOF: Pos<Meters> = Pos::new(40.0, 40.0);

#[test]
fn a_kit_used_at_a_point_stands_a_fire_there_that_burns_until_a_hand_or_a_cloudburst_puts_it_out_and_is_kicked_apart()
 {
    let mut sim = moor();
    let player = sim.join(1);
    let conn = sim.conn(1);
    sim.give(player, "kit", 2);
    sim.run(0.5);
    let pitch = sim.id::<SkillDef>("pitch_fire");
    let burning = sim.id::<StatusEffectDef>("burning");
    assert!(
        skill::known(&sim.world, player).contains_key(&pitch),
        "whoever carries an item that casts a skill knows the skill"
    );

    let slot = sim.slot_of(player, "kit");
    sim.send(conn, UseItemRequest { slot });
    sim.run(0.5);
    assert_eq!(
        fires(&mut sim),
        [],
        "a kit used with nowhere aimed at stands nothing"
    );
    assert_eq!(sim.count(player, "kit"), 2, "and is kept");

    storm(&mut sim);
    sim.tick();
    let wet = (0..(MOOR as f32 / RAIN_CELL.0) as usize)
        .flat_map(|row| (0..(MOOR as f32 / RAIN_CELL.0) as usize).map(move |column| (column, row)))
        .map(|(column, row)| {
            Pos::new(
                (column as f32 + 0.5) * RAIN_CELL.0,
                (row as f32 + 0.5) * RAIN_CELL.0,
            )
        })
        .find(|at| falling_on(&sim.world, *at) > 0.9)
        .expect("under a storm it pours somewhere on the moor");
    assert_eq!(
        falling_on(&sim.world, ROOF),
        0.0,
        "no rain falls on the ground under a roof"
    );
    drizzle(&mut sim);

    let open = pitched(&mut sim, player, pitch, wet);
    assert_eq!(
        sim.count(player, "kit"),
        1,
        "the skill's cost takes the kit"
    );
    assert!(
        position(&sim.world, open).is_some_and(|at| at.distance_to(wet) < 0.5),
        "the fire stands where the kit was aimed"
    );
    sim.open_with(1, open);
    sim.pick(1, "Light it.");
    assert!(
        status::has(&sim.world, open, burning),
        "a thing carries a status as an actor does"
    );
    sim.leave(1);
    sim.run(4.0);
    assert!(
        status::has(&sim.world, open, burning),
        "a drizzle leaves a fire burning"
    );
    sim.open_with(1, open);
    sim.pick(1, "Put it out.");
    assert!(!status::has(&sim.world, open, burning));
    sim.pick(1, "Light it.");
    sim.leave(1);

    let sheltered = pitched(&mut sim, player, pitch, ROOF);
    sim.open_with(1, sheltered);
    sim.pick(1, "Light it.");
    sim.leave(1);
    storm(&mut sim);
    assert!(
        sim.run_until(4.0, |world| !status::has(world, open, burning)),
        "a cloudburst puts a fire in the open out"
    );
    sim.run(4.0);
    assert!(
        status::has(&sim.world, sheltered, burning),
        "and leaves one under a roof burning"
    );

    sim.open_with(1, sheltered);
    sim.pick(1, "Kick it apart.");
    sim.tick();
    assert_eq!(
        fires(&mut sim),
        [open],
        "a spawned thing a script despawns is gone"
    );
}

/// The fires standing on the moor.
fn fires(sim: &mut Sim) -> Vec<Entity> {
    let mut spawned = sim.world.query_filtered::<Entity, With<SpawnedObject>>();
    let mut fires: Vec<Entity> = spawned.iter(&sim.world).collect();
    fires.sort_unstable();
    fires
}

/// The player goes beside `at`, pitches a fire there, and the fire that then stands.
fn pitched(sim: &mut Sim, player: Entity, pitch: skill::SkillId, at: Pos<Meters>) -> Entity {
    let before = fires(sim);
    movement::relocate(&mut sim.world, player, at + Offset::new(2.0, 0.0));
    let conn = sim.conn(1);
    sim.send(
        conn,
        UseSkillRequest {
            skill: pitch,
            aim: CastAim::Point(at),
        },
    );
    assert!(
        sim.run_until(5.0, |world| {
            world
                .query_filtered::<(), With<SpawnedObject>>()
                .iter(world)
                .count()
                > before.len()
        }),
        "no fire was pitched"
    );
    let stood = fires(sim)
        .into_iter()
        .find(|fire| !before.contains(fire))
        .expect("the new fire");
    assert_eq!(
        sim.world
            .get::<AreaObject>(stood)
            .and_then(|fire| fire.title.clone())
            .as_deref(),
        Some("Fire")
    );
    stood
}

fn drizzle(sim: &mut Sim) {
    sim.world
        .insert_resource(WeatherClock(Some(TimeCourse::Held(0))));
}

fn storm(sim: &mut Sim) {
    sim.world
        .insert_resource(WeatherClock(Some(TimeCourse::Held(1))));
}

/// A moor under a drizzle or a storm, with one roof on it, where a kit pitches a fire that is
/// lit, put out and kicked apart by hand, and that a cloudburst puts out.
fn moor() -> Sim {
    let content = Game::new()
        .area(
            "moor",
            &Map::open(MOOR, MOOR),
            json!([{ "type": "prop", "prop": path("prop", "roof"), "at": { "x": ROOF.x, "y": ROOF.y } }]),
        )
        .edit("area", "moor", |area| {
            area["climate"] = json!({
                "turns": 10.0,
                "spells": [
                    {
                        "name": "drizzle",
                        "weather": { "clouds": 1.0, "water": 1.0, "rain": 0.2 },
                        "lasts": { "shortest": 60.0, "longest": 60.0 },
                    },
                    {
                        "name": "storm",
                        "weather": { "clouds": 1.0, "water": 1.0, "rain": 1.0 },
                        "lasts": { "shortest": 60.0, "longest": 60.0 },
                    },
                ],
                "rain": {
                    "shader": path("vfx_shader", "falling"),
                    "shells": [4.0],
                    "slant": 0.0,
                    "wets": 10.0,
                    "dries": 10.0,
                },
            });
        })
        .shader_on("falling", Stage::Rain)
        .prop_of(
            "roof",
            box_glb(Vec3::new(8.0, 4.0, 8.0), "thatch", [1.0; 4]),
            json!({ "footprint": { "type": "none" }, "shelter": true }),
        )
        .prop("logs", json!({ "type": "none" }))
        .hero("moor", 20.0, 20.0)
        .asset("icons/fire.png", testkit::png(1, 1))
        .item(
            "kit",
            json!({
                "kind": { "type": "usable", "casts": path("skill", "pitch_fire") },
                "modules": [{ "type": "stack", "max": 10 }],
            }),
        )
        .row(
            "skill",
            "pitch_fire",
            json!({
                "name": "Pitch a fire",
                "icon": "icons/fire.png",
                "target": "area_ally",
                "range": 4.0,
                "area": { "type": "circle", "radius": 0.5 },
                "cost": [{ "item": path("item", "kit"), "count": 1 }],
                "levels": [{ "cost": 1 }],
                "modules": [{ "type": "script", "script": path("script", "pitch_fire_stand") }],
            }),
        )
        .script(
            "pitch_fire_stand",
            "skill",
            json!({
                "used": { "type": "on", "event": "used", "next": "stand" },
                "stand": {
                    "type": "spawn_object",
                    "spawnable": path("spawnable", "fire"),
                    "at": { "type": "aimed_point" },
                },
            }),
        )
        .row(
            "spawnable",
            "fire",
            json!({
                "title": "Fire",
                "look": path("prop", "logs"),
                "modules": [{ "type": "script", "script": path("script", "fire_tend") }],
            }),
        )
        .script(
            "fire_tend",
            "spawnable",
            json!({
                "interacted": {
                    "type": "on",
                    "event": "interacted",
                    "verb": "use",
                    "reach": 3.0,
                    "next": "tend",
                },
                "tend": dialogue("A ring of logs.", json!([
                    { "label": ["Light it."], "to": "light" },
                    { "label": ["Put it out."], "to": "douse" },
                    { "label": ["Kick it apart."], "to": "apart" },
                ])),
                "light": {
                    "type": "apply_status",
                    "who": { "type": "owner" },
                    "effect": path("status_effect", "burning"),
                    "next": "tend",
                },
                "douse": {
                    "type": "remove_status",
                    "who": { "type": "owner" },
                    "effect": path("status_effect", "burning"),
                    "next": "tend",
                },
                "apart": { "type": "despawn", "who": { "type": "owner" } },
            }),
        )
        .row(
            "status_effect",
            "burning",
            json!({
                "name": "Burning",
                "icon": "icons/fire.png",
                "category": "buff",
                "modules": [{ "type": "script", "script": path("script", "burning_doused") }],
            }),
        )
        .script(
            "burning_doused",
            "status_effect",
            json!({
                "every": { "type": "on", "event": "interval", "seconds": 1.0, "next": "pouring" },
                "pouring": {
                    "type": "if",
                    "test": { "type": "at_least", "a": { "type": "rain" }, "b": 0.5 },
                    "yes": "out",
                },
                "out": { "type": "remove_status", "effect": path("status_effect", "burning") },
            }),
        )
        .shader("status_pop")
        .sfx("ui_refuse")
        .load();
    let mut sim = Sim::new(content);
    sim.serve("moor")
        .accept::<UseItemRequest>()
        .accept::<UseSkillRequest>()
        .accept::<InteractRequest>()
        .accept::<ConversationRequest>()
        .add((
            area::weather::turn_sky,
            status::expire,
            area::spawnable::expire,
            item::use_item,
            skill::casting::request,
            interact::interact_request,
            dialogue::requests,
            actor::playing::progress,
        ))
        .plan((
            status::aspect::SettleAspects::default(),
            status::aspect::TickAspects::default(),
        ))
        .plan(skill::casting::StepCasts::default())
        .plan((
            script::query::ListRoster::default(),
            script::RunFlows::default(),
            movement::AdvanceWalks::default(),
        ))
        .add((
            interact::interactions,
            (dialogue::hold, dialogue::refresh).chain(),
        ))
        .plan(actor::SettleActions::default());
    script::spawn_stateless(&mut sim.world);
    region::hold_whole(&mut sim.world);
    sim
}
