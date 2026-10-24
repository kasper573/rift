use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use engine::core::geometry::{Degrees, Footprint, Radians, Shape};
use engine::core::math::{Offset, Pos, Rgba};
use engine::core::time::UnixMillis;
use engine::core::units::Meters;
use engine::core::vfx::{ParamValue, VfxDef, VfxInstance, VfxParams, VfxSite};
use engine::systems::area::AreaDef;
use engine::systems::class::{self, NodeKey};
use engine::systems::combat::{self, Attitude};
use engine::systems::movement::{self, position};
use engine::systems::npc::{self, NpcDef, Pack};
use engine::systems::player::PlayedBy;
use engine::systems::skill::{self, CastAim, Rank, SkillDef, UseSkillRequest};
use engine::systems::stat::{self, StatKind};
use engine::systems::visibility::Presence;
use engine::systems::{actor, script, vfx};
use serde_json::{Value, json};
use testkit::{Declared, Game, Map, Sim, path};

#[test]
fn damage_shows_on_the_one_hit_and_the_effect_passes() {
    let (mut sim, player) = arena();
    let dummy = dummy_toward(&mut sim, player, 1.0);

    combat::inflict(&mut sim.world, dummy, Some(player), 4.0);
    sim.tick();

    let shown = instances(&mut sim, "damage_taken");
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].site, VfxSite::Actor(dummy));

    sim.later(3.0);
    assert!(instances(&mut sim, "damage_taken").is_empty());
}

#[test]
fn healing_shows_but_regeneration_does_not() {
    let (mut sim, player) = arena();
    stat::apply_damage(&mut sim.world, player, 6.0);
    sim.tick();

    stat::regenerate(&mut sim.world, player, 1.0);
    sim.tick();
    assert!(instances(&mut sim, "healed").is_empty());

    stat::heal(&mut sim.world, player, 3.0);
    sim.tick();
    let shown = instances(&mut sim, "healed");
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].site, VfxSite::Actor(player));
}

#[test]
fn a_level_reached_plays_on_the_actor_for_everyone_and_on_the_screen_for_the_player_alone() {
    let (mut sim, player) = arena();
    let client = sim.world.get::<PlayedBy>(player).expect("owner").client;

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
    assert_eq!(shown.len(), 2);
    let on_actor = shown
        .iter()
        .find(|(instance, _)| instance.site == VfxSite::Actor(player))
        .expect("one on the actor");
    assert!(on_actor.1.is_none());
    let on_screen = shown
        .iter()
        .find(|(instance, _)| instance.site == VfxSite::Screen)
        .expect("one on the screen");
    assert!(matches!(on_screen.1, Some(Presence::For(shown)) if shown == client));
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
    let circles: Vec<Footprint> = blasts
        .iter()
        .filter_map(|blast| match blast.site {
            VfxSite::Area(footprint) => Some(footprint),
            _ => None,
        })
        .collect();
    assert_eq!(circles.len(), 1);
    assert!(circles[0].contains(feet));
    let struck: Vec<(Entity, Radians)> = blasts
        .iter()
        .filter_map(|blast| match blast.site {
            VfxSite::Actor(actor) => Some((actor, blast.heading)),
            _ => None,
        })
        .collect();
    assert_eq!(struck, vec![(dummy, circles[0].facing)]);
}

#[test]
fn a_path_vfx_runs_from_the_caster_through_its_targets_nearest_first() {
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
    let stops: Vec<_> = [player, near, middle, far]
        .iter()
        .map(|who| position(&sim.world, *who).expect("position"))
        .collect();
    assert_eq!(shown[0].site, VfxSite::Path(stops));
}

#[test]
fn an_instance_arrives_as_the_server_sent_it_whatever_its_parameters_and_site() {
    let (sim, _) = arena();
    let on_screen = VfxInstance {
        vfx: sim.id("damage_taken"),
        site: VfxSite::Screen,
        params: VfxParams(vec![
            ("amount".to_owned(), ParamValue::Number(4.5)),
            ("tint".to_owned(), ParamValue::Color(Rgba(0xff00_80ff))),
        ]),
        heading: Radians(0.25),
        started: UnixMillis(1_000),
        until: UnixMillis(2_500),
    };
    let on_an_area = VfxInstance {
        vfx: sim.id("swoosh"),
        site: VfxSite::Area(Footprint {
            at: Pos::new(3.0, 4.0),
            facing: Radians(0.5),
            shape: Shape::Cone {
                angle: Degrees(90.0),
                length: Meters(2.5),
            },
        }),
        params: VfxParams::default(),
        ..on_screen.clone()
    };
    for sent in [on_screen, on_an_area] {
        let bytes = postcard::to_allocvec(&sent).expect("serializes");
        let received: VfxInstance = postcard::from_bytes(&bytes).expect("deserializes");
        assert_eq!(received, sent);
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
                    "next": "hit",
                },
                "hit": damage_last(),
            }), "arc"),
        )
        .asset("icons/skill.png", testkit::png(1, 1))
        .npc("dummy", 100.0, json!([]))
        .vfx("slash_hit")
        .vfx("swoosh")
        .vfx("blast")
        .edit("vfx", "blast", |blast| blast["on_actors_inside"] = json!(path("vfx", "blast")))
        .vfx("arc")
        .vfx("damage_taken")
        .shader("damage_pop")
        .vfx("healed")
        .shader("heal_pop")
        .vfx("level_up")
        .shader("level_pop")
        .vfx("defeated")
        .sfx("rising_chime")
        .load();
    let mut sim = Sim::new(content);
    sim.serve("arena")
        .accept::<UseSkillRequest>()
        .add((
            vfx::expire,
            skill::casting::request,
            actor::playing::progress,
            skill::casting::casts,
        ))
        .add((
            script::query::take_tick_roster,
            script::run_tick,
            movement::advance,
        ))
        .add((stat::settle_health, combat::defeats, actor::settle));
    script::spawn_stateless(&mut sim.world);
    let player = sim.join(1);
    sim.run(0.5);
    (sim, player)
}

fn area_skill(area: Value, script: &str) -> Value {
    json!({
        "name": "Area skill",
        "icon": "icons/skill.png",
        "target": "area_enemy",
        "range": 6.0,
        "area": area,
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
    let dummy = npc::spawn(&mut sim.world, dummy, at, area, Pack(u32::MAX));
    sim.world.entity_mut(dummy).insert(Attitude::Hostile);
    sim.tick();
    dummy
}
