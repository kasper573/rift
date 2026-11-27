use std::collections::BTreeMap;

use bevy_ecs::entity::{EntityHashMap, MapEntities};
use bevy_ecs::prelude::*;
use engine::core::geometry::{PlacedShape, Shape};
use engine::core::math::{Offset, Pos, Rgba};
use engine::core::time::UnixMillis;
use engine::core::units::{Degrees, Meters, Radians};
use engine::core::vfx::{ParamValue, PathStop, VfxDef, VfxInstance, VfxParams, VfxPart, VfxSite};
use engine::systems::actor;
use engine::systems::area::AreaDef;
use engine::systems::area::vfx;
use engine::systems::class::{self, NodeKey};
use engine::systems::combat::{self, Attitude};
use engine::systems::movement::{self, position};
use engine::systems::npc::{self, NpcDef};
use engine::systems::skill::{self, CastAim, Rank, SkillDef, UseSkillRequest};
use engine::systems::stat::{self, StatKind, Vital};
use engine::systems::visibility::Presence;
use serde_json::{Value, json};
use testkit::{Declared, Game, Map, Sim, path};

#[test]
fn healing_shows_but_regeneration_does_not() {
    let (mut sim, player) = arena();
    stat::apply_damage(&mut sim.world, player, 6.0);
    sim.tick();

    stat::restore(&mut sim.world, player, Vital::Health, 1.0);
    sim.tick();
    assert!(instances(&mut sim, "healed").is_empty());

    stat::heal(&mut sim.world, player, 3.0);
    sim.tick();
    let shown = instances(&mut sim, "healed");
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].site, VfxSite::Actor(player));
}

#[test]
fn a_level_reached_plays_on_the_actor_for_everyone() {
    let (mut sim, player) = arena();

    class::grant_xp(&mut sim.world, player, 30);
    sim.tick();

    let level_up = sim.id::<VfxDef>("level_up");
    let shown: Vec<(VfxInstance, Option<Presence>)> = sim
        .world
        .query::<(&VfxInstance, Option<&Presence>)>()
        .iter(&sim.world)
        .filter(|(instance, _)| instance.vfx == level_up)
        .map(|(instance, presence)| (instance.clone(), presence.cloned()))
        .collect();
    let [(on_actor, presence)] = shown.as_slice() else {
        panic!("one level-up vfx, found {}", shown.len());
    };
    assert_eq!(on_actor.site, VfxSite::Actor(player));
    assert!(presence.is_none());
}

#[test]
fn a_skill_plays_its_vfx_on_every_target_and_once_on_its_area() {
    let (mut sim, player) = arena();
    learn(&mut sim, player, "cleave");
    let near = dummy_toward(&mut sim, player, 1.0);
    let far = dummy_toward(&mut sim, player, 2.0);
    let full = stat::effective(&sim.world, near, StatKind::Health);
    let at = position(&sim.world, near).expect("position");

    cast(&mut sim, "cleave", at);
    assert!(sim.run_until(10.0, |world| any_shown(world, "slash_hit")));

    let mut hit: Vec<VfxSite> = instances(&mut sim, "slash_hit")
        .iter()
        .map(|instance| instance.site.clone())
        .collect();
    hit.sort_by_key(|site| format!("{site:?}"));
    let mut struck: Vec<VfxSite> = [near, far]
        .into_iter()
        .filter(|dummy| stat::effective(&sim.world, *dummy, StatKind::Health) < full)
        .map(VfxSite::Actor)
        .collect();
    struck.sort_by_key(|site| format!("{site:?}"));
    assert!(!struck.is_empty());
    assert_eq!(hit, struck);
    let sweeps = instances(&mut sim, "swoosh");
    assert_eq!(sweeps.len(), 1);
    assert!(
        matches!(sweeps[0].site, VfxSite::Area(_)),
        "{:?}",
        sweeps[0].site
    );
}

#[test]
fn an_area_vfx_reaches_the_targets_it_struck_and_not_the_caster() {
    let (mut sim, player) = arena();
    learn(&mut sim, player, "fireball");
    let dummy = dummy_toward(&mut sim, player, 1.0);
    let feet = position(&sim.world, player).expect("position");

    cast(&mut sim, "fireball", feet);
    assert!(sim.run_until(10.0, |world| any_shown(world, "blast")));

    let blasts = instances(&mut sim, "blast");
    let circles: Vec<PlacedShape> = blasts
        .iter()
        .filter_map(|blast| match blast.site {
            VfxSite::Area(footprint) => Some(footprint),
            _ => None,
        })
        .collect();
    assert_eq!(circles.len(), 1);
    assert!(circles[0].contains(feet));
    let struck: Vec<(Entity, Option<Radians>)> = blasts
        .iter()
        .filter_map(|blast| match blast.site {
            VfxSite::Actor(actor) => Some((actor, blast.heading)),
            _ => None,
        })
        .collect();
    assert_eq!(struck, vec![(dummy, Some(circles[0].facing))]);
}

#[test]
fn a_path_vfx_runs_from_the_caster_through_its_targets_nearest_first_as_its_script_fills_it() {
    let (mut sim, player) = arena();
    learn(&mut sim, player, "chain");
    let from = position(&sim.world, player).expect("position");
    let far = dummy_toward(&mut sim, player, 5.0);
    let near = dummy_toward(&mut sim, player, 2.0);
    let middle = dummy_toward(&mut sim, player, 4.0);
    sim.run(0.5);

    cast(&mut sim, "chain", from + Offset::new(3.0, 0.0));
    assert!(sim.run_until(10.0, |world| any_shown(world, "arc")));

    let shown = instances(&mut sim, "arc");
    assert_eq!(shown.len(), 1);
    let stops = [player, near, middle, far].map(PathStop::Actor).to_vec();
    assert_eq!(shown[0].site, VfxSite::Path(stops));
    let filled = VfxParams(vec![
        ("tint".to_owned(), ParamValue::Color(Rgba(0xff00_80ff))),
        ("width".to_owned(), ParamValue::Number(1.5)),
    ]);
    assert_eq!(shown[0].params, filled);
}

#[test]
fn a_bolt_flies_to_where_its_skill_is_aimed_and_its_hit_waits_for_it_to_land() {
    let (mut sim, player) = arena();
    learn(&mut sim, player, "bolt");
    let dummy = dummy_toward(&mut sim, player, 3.0);
    let full = stat::effective(&sim.world, dummy, StatKind::Health);
    let at = position(&sim.world, dummy).expect("position");

    cast(&mut sim, "bolt", at);
    assert!(sim.run_until(10.0, |world| any_shown(world, "flight")));

    let flight = instances(&mut sim, "flight");
    let VfxSite::Path(stops) = &flight[0].site else {
        panic!("a path: {:?}", flight[0].site);
    };
    assert_eq!(stops[0], PathStop::Actor(player));
    assert!(
        matches!(stops[1..], [PathStop::Point(end)] if end.distance_to(at) < 0.01),
        "{stops:?}"
    );
    sim.run(0.8);
    assert_eq!(stat::effective(&sim.world, dummy, StatKind::Health), full);
    sim.run(0.4);
    assert!(stat::effective(&sim.world, dummy, StatKind::Health) < full);
}

#[test]
fn an_instance_arrives_as_the_server_sent_it_whatever_its_parameters_and_site() {
    let (sim, player) = arena();
    let along_a_path = VfxInstance {
        vfx: sim.id("arc"),
        site: VfxSite::Path(vec![
            PathStop::Actor(player),
            PathStop::Point(Pos::new(2.0, 1.5)),
        ]),
        part: VfxPart::Node {
            node: "hand_r".to_owned(),
            radius: Meters(0.3),
        },
        dealer: Some(player),
        params: VfxParams(vec![
            ("amount".to_owned(), ParamValue::Number(4.5)),
            ("tint".to_owned(), ParamValue::Color(Rgba(0xff00_80ff))),
        ]),
        heading: Some(Radians(0.25)),
        started: UnixMillis(1_000),
        until: Some(UnixMillis(2_500)),
    };
    let on_an_area = VfxInstance {
        vfx: sim.id("swoosh"),
        site: VfxSite::Area(PlacedShape {
            at: Pos::new(3.0, 4.0),
            facing: Radians(0.5),
            shape: Shape::Cone {
                angle: Degrees(90.0),
                length: Meters(2.5),
            },
        }),
        params: VfxParams::default(),
        ..along_a_path.clone()
    };
    // The client knows the player by an entity of its own, and every one the instance names
    // must become that.
    let mut client = World::new();
    let mut theirs = client.spawn_empty().id();
    while theirs == player {
        theirs = client.spawn_empty().id();
    }
    let mut names = EntityHashMap::default();
    names.insert(player, theirs);
    for sent in [along_a_path, on_an_area] {
        let bytes = postcard::to_allocvec(&sent).expect("serializes");
        let mut received: VfxInstance = postcard::from_bytes(&bytes).expect("deserializes");
        assert_eq!(received, sent);
        received.map_entities(&mut names);
        let mut seen = sent.clone();
        seen.dealer = Some(theirs);
        if let VfxSite::Path(stops) = &mut seen.site {
            stops[0] = PathStop::Actor(theirs);
        }
        assert_eq!(received, seen);
    }
}

fn arena() -> (Sim, Entity) {
    let content = Game::new()
        .area("arena", &Map::open(15, 9), json!([]))
        .hero("arena", 4.0, 4.0)
        .edit("class", "hero", |hero| {
            hero["levels"] = json!([
                { "xp": 0, "skill_points": 1 },
                { "xp": 30, "skill_points": 1 },
            ]);
            hero["tree"] = json!({
                "cleave": { "skill": "cleave.skill.json", "at": { "x": 0.0, "y": 0.0 } },
                "fireball": { "skill": "fireball.skill.json", "at": { "x": 80.0, "y": 0.0 } },
                "chain": { "skill": "chain.skill.json", "at": { "x": 160.0, "y": 0.0 } },
                "bolt": { "skill": "bolt.skill.json", "at": { "x": 240.0, "y": 0.0 } },
            });
        })
        .row(
            "skill",
            "cleave",
            area_skill(json!({ "type": "cone", "angle": 90.0, "length": 2.0 }), "cleave_sweep"),
        )
        .script(
            "cleave_sweep",
            "skill",
            area_nodes(json!({
                "sweep": { "type": "vfx", "vfx": "swoosh.vfx.json", "on": { "type": "skill_area" }, "next": "hit" },
                "hit": damage("show"),
                "show": { "type": "vfx", "vfx": "slash_hit.vfx.json", "on": { "type": "skill_targets" } },
            }), "sweep"),
        )
        .row(
            "skill",
            "fireball",
            area_skill(json!({ "type": "circle", "radius": 2.0 }), "fireball_blast"),
        )
        .script(
            "fireball_blast",
            "skill",
            area_nodes(json!({
                "blast": { "type": "vfx", "vfx": "blast.vfx.json", "on": { "type": "skill_area" }, "next": "hit" },
                "hit": damage_last(),
            }), "blast"),
        )
        .row(
            "skill",
            "chain",
            area_skill(json!({ "type": "circle", "radius": 3.0 }), "chain_arc"),
        )
        .script(
            "chain_arc",
            "skill",
            area_nodes(json!({
                "arc": {
                    "type": "vfx",
                    "vfx": "arc.vfx.json",
                    "on": { "type": "skill_targets" },
                    "site": { "type": "path" },
                    "params": { "tint": [255, 0, 128, 255], "width": 1.5 },
                    "next": "hit",
                },
                "hit": damage_last(),
            }), "arc"),
        )
        .row(
            "skill",
            "bolt",
            area_skill(json!({ "type": "circle", "radius": 1.0 }), "bolt_flight"),
        )
        .script(
            "bolt_flight",
            "skill",
            area_nodes(json!({
                "fly": {
                    "type": "vfx",
                    "vfx": "flight.vfx.json",
                    "on": { "type": "skill_area" },
                    "site": { "type": "path" },
                    "wait": true,
                    "next": "hit",
                },
                "hit": damage_last(),
            }), "fly"),
        )
        .asset("icons/skill.png", testkit::png(1, 1))
        .npc("dummy", 100.0, json!([]))
        .vfx("slash_hit")
        .vfx("swoosh")
        .vfx("blast")
        .edit("vfx", "blast", |blast| blast["on_actors_inside"] = json!(path("vfx", "blast")))
        .vfx("arc")
        .edit("vfx", "arc", |arc| {
            arc["params"] = json!([
                { "name": "tint", "default": [255, 255, 255, 255] },
                { "name": "width", "default": 1.0 },
            ]);
        })
        .vfx("flight")
        .edit("vfx", "flight", |flight| flight["layers"][0]["place"] = json!({ "travel": {} }))
        .shader("damage_pop")
        .vfx("healed")
        .shader("heal_pop")
        .vfx("level_up")
        .shader("level_pop")
        .vfx("defeated")
        .sfx("level_up")
        .load();
    let mut sim = Sim::new(content);
    sim.serve("arena")
        .accept::<UseSkillRequest>()
        .add((
            vfx::expire,
            skill::casting::request,
            actor::playing::progress,
        ))
        .plan(skill::casting::StepCasts::default())
        .plan((
            engine::systems::roster::ListRoster::default(),
            engine::core::script::run::FlowRunner::default(),
            movement::AdvanceWalks::default(),
        ))
        .plan(stat::SettleVitals::default())
        .add(combat::defeats)
        .plan(actor::SettleActions::default());
    engine::systems::spawn_scripts(&mut sim.world);
    let player = sim.join(1);
    sim.run(0.5);
    (sim, player)
}

fn area_skill(area: Value, script: &str) -> Value {
    json!({
        "name": "Area skill",
        "icon": "icons/skill.png",
        "target": {
            "type": "ranged_area",
            "affects": "enemy",
            "range": 6.0,
            "shape": area,
        },
        "cooldown": 6.0,
        "levels": [{ "cost": 1, "stats": { "power": 1.0 } }],
        "modules": [{ "type": "script", "script": path("script", script) }],
    })
}

fn area_nodes(mut nodes: Value, first: &str) -> Value {
    nodes["used"] = json!({ "type": "on", "event": "used", "next": "swing" });
    nodes["swing"] =
        json!({ "type": "animate", "action": "attack", "wait": "apex", "next": first });
    nodes
}

fn damage(next: &str) -> Value {
    let mut hit = damage_last();
    hit["next"] = json!(next);
    hit
}

fn damage_last() -> Value {
    json!({
        "type": "damage",
        "who": { "type": "skill_targets" },
        "by": { "type": "stat", "stat": "damage" },
    })
}

fn learn(sim: &mut Sim, player: Entity, node: &str) {
    sim.world
        .entity_mut(player)
        .insert(skill::Learned(BTreeMap::from([(
            NodeKey::from(node),
            Rank(1),
        )])));
}

fn cast(sim: &mut Sim, skill: &str, at: Pos<Meters>) {
    let conn = sim.conn(1);
    let skill = sim.id::<SkillDef>(skill);
    sim.send(
        conn,
        UseSkillRequest {
            skill,
            aim: CastAim::Point(at),
        },
    );
}

fn any_shown(world: &mut World, key: &str) -> bool {
    let vfx = world
        .resource::<engine::core::content::Content>()
        .declared::<VfxDef>(key)
        .expect("a declared vfx");
    world
        .query::<&VfxInstance>()
        .iter(world)
        .any(|instance| instance.vfx == vfx)
}

fn instances(sim: &mut Sim, key: &str) -> Vec<VfxInstance> {
    let vfx = sim.id::<VfxDef>(key);
    sim.world
        .query::<&VfxInstance>()
        .iter(&sim.world)
        .filter(|instance| instance.vfx == vfx)
        .cloned()
        .collect()
}

/// A hostile dummy that stands where it's put and never fights back.
fn dummy_toward(sim: &mut Sim, player: Entity, east: f32) -> Entity {
    let at = position(&sim.world, player).expect("position") + Offset::new(east, 0.0);
    let area = sim.id::<AreaDef>("arena");
    let dummy = sim.id::<NpcDef>("dummy");
    let dummy = npc::spawn(&mut sim.world, dummy, at, area, None);
    sim.world.entity_mut(dummy).insert(Attitude::Hostile);
    sim.tick();
    dummy
}
