use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use engine::core::content::Content;
use engine::core::math::{Offset, Pos};
use engine::core::time::Seconds;
use engine::core::units::Meters;
use engine::core::vfx::{VfxDef, VfxInstance, VfxSite};
use engine::systems::area::AreaDef;
use engine::systems::class::NodeKey;
use engine::systems::combat::{self, Attitude};
use engine::systems::equipment::{self, EquipmentSlot, UnequipRequest};
use engine::systems::item::{self, Inventory, ItemDef, ItemStack, UseItemRequest};
use engine::systems::movement::{self, position};
use engine::systems::npc::{self, NpcDef, Pack};
use engine::systems::skill::{
    self, AssignBasicAttackRequest, AssignHotbarRequest, CastAim, Hotbar, HotbarEntry, HotbarSlot,
    Rank, SkillDef, UseSkillRequest,
};
use engine::systems::stat::{self, StatKind};
use engine::systems::status::{self, StatusCategory, StatusEffectDef, Statuses};
use engine::systems::{actor, script, vfx};
use serde_json::{Value, json};
use testkit::{Game, Map, Sim, path};

#[test]
fn a_status_holds_its_boost_until_its_time_runs_out() {
    let (mut sim, player) = clinic();
    let base = stat::max_health(&sim.world, player);
    let guarded = sim.id::<StatusEffectDef>("guarded");

    status::apply(&mut sim.world, player, guarded, 10.0, Some(Seconds(5.0)));
    sim.run(0.5);
    assert_eq!(stat::max_health(&sim.world, player), base + 10.0);

    sim.later(5.0);
    assert_eq!(stat::max_health(&sim.world, player), base);
    assert!(!status::has(&sim.world, player, guarded));
}

#[test]
fn applying_a_held_status_refreshes_it_instead_of_stacking() {
    let (mut sim, player) = clinic();
    let base = stat::max_health(&sim.world, player);
    let guarded = sim.id::<StatusEffectDef>("guarded");

    status::apply(&mut sim.world, player, guarded, 10.0, Some(Seconds(1.0)));
    sim.run(0.5);
    status::apply(&mut sim.world, player, guarded, 18.0, Some(Seconds(5.0)));
    sim.run(0.5);
    assert_eq!(statuses(&sim, player).0.len(), 1);
    assert_eq!(stat::max_health(&sim.world, player), base + 18.0);

    sim.later(1.5);
    assert_eq!(stat::max_health(&sim.world, player), base + 18.0);
}

#[test]
fn a_cleanse_takes_only_the_category_it_names() {
    let (mut sim, player) = clinic();
    let guarded = sim.id::<StatusEffectDef>("guarded");
    let poisoned = sim.id::<StatusEffectDef>("poisoned");
    status::apply(&mut sim.world, player, guarded, 10.0, None);
    status::apply(&mut sim.world, player, poisoned, 1.0, None);
    sim.run(0.5);

    use_item(&mut sim, player, "antidote");

    assert!(!status::has(&sim.world, player, poisoned));
    assert!(status::has(&sim.world, player, guarded));
    assert_eq!(sim.count(player, "antidote"), 0);
    let content = sim.world.resource::<engine::core::content::Content>();
    assert_eq!(
        statuses(&sim, player).count(content, Some(StatusCategory::Debuff)),
        0
    );
}

#[test]
fn a_status_on_an_interval_strikes_or_heals_every_period_while_it_holds() {
    let (mut sim, player) = clinic();
    let full = stat::current_health(&sim.world, player);
    let poisoned = sim.id::<StatusEffectDef>("poisoned");

    use_item(&mut sim, player, "bog_water");
    assert!(status::has(&sim.world, player, poisoned));
    assert_eq!(stat::current_health(&sim.world, player), full);

    sim.later(2.0);
    assert_eq!(stat::current_health(&sim.world, player), full - 2.0);
    sim.later(2.0);
    assert_eq!(stat::current_health(&sim.world, player), full - 4.0);

    sim.later(10.0);
    assert!(!status::has(&sim.world, player, poisoned));
    let bitten = stat::current_health(&sim.world, player);
    sim.later(2.0);
    assert_eq!(stat::current_health(&sim.world, player), bitten);

    let at = position(&sim.world, player).expect("position") + Offset::new(3.0, 0.0);
    let dummy = dummy(&mut sim, at);
    stat::apply_damage(&mut sim.world, dummy, 20.0);
    let hurt = stat::current_health(&sim.world, dummy);
    let sanctified = sim.id::<StatusEffectDef>("sanctified");
    status::apply(&mut sim.world, dummy, sanctified, 3.0, Some(Seconds(9.0)));
    sim.run(0.5);
    assert_eq!(stat::current_health(&sim.world, dummy), hurt);
    sim.later(2.0);
    assert_eq!(
        stat::current_health(&sim.world, dummy),
        hurt + 3.0,
        "a status of positive power heals each period"
    );
    sim.later(2.0);
    assert_eq!(stat::current_health(&sim.world, dummy), hurt + 6.0);
}

#[test]
fn worn_gear_boosts_only_while_it_is_worn() {
    let (mut sim, player) = clinic();
    let bare = stat::effective(&sim.world, player, StatKind::Damage);

    sim.give(player, "blade", 1);
    let blade = sim.id::<ItemDef>("blade");
    equipment::equip_held(&mut sim.world, player, blade).expect("the blade is worn");
    sim.run(0.5);
    assert_eq!(
        stat::effective(&sim.world, player, StatKind::Damage),
        bare + 4.0
    );

    let conn = sim.conn(1);
    sim.send(
        conn,
        UnequipRequest {
            slot: EquipmentSlot::Weapon,
        },
    );
    sim.run(0.5);
    assert_eq!(stat::effective(&sim.world, player, StatKind::Damage), bare);
}

#[test]
fn a_carried_charm_boosts_only_while_it_is_carried() {
    let (mut sim, player) = clinic();
    let base = stat::effective(&sim.world, player, StatKind::MovementSpeed);

    sim.give(player, "wing", 1);
    sim.run(0.5);
    assert_eq!(
        stat::effective(&sim.world, player, StatKind::MovementSpeed),
        base + 0.5
    );

    let wing = sim.id::<ItemDef>("wing");
    let content = sim.world.resource::<Content>().clone();
    sim.world
        .get_mut::<Inventory>(player)
        .expect("bag")
        .exchange(&content, &[ItemStack::new(wing, 1)], &[])
        .expect("the wing is handed over");
    sim.run(0.5);
    assert_eq!(
        stat::effective(&sim.world, player, StatKind::MovementSpeed),
        base
    );
}

#[test]
fn a_passive_skill_holds_its_boost_while_known_and_cannot_be_cast_or_slotted() {
    let (mut sim, player) = clinic();
    let base = stat::max_health(&sim.world, player);
    let toughness = sim.id::<SkillDef>("toughness");

    skill::learn::learn(
        &mut sim.world,
        player,
        &[(NodeKey::from("toughness"), Rank(1))],
    )
    .expect("toughness is learned");
    sim.run(0.5);
    assert_eq!(stat::max_health(&sim.world, player), base + 6.0);

    sim.forget_notified();
    let conn = sim.conn(1);
    sim.send(
        conn,
        UseSkillRequest {
            skill: toughness,
            aim: CastAim::Actor(player),
        },
    );
    sim.run(0.5);
    assert!(sim.errors(1).iter().any(|text| text.contains("passive")));

    sim.send(
        conn,
        AssignHotbarRequest {
            slot: HotbarSlot(8),
            entry: HotbarEntry::Skill(toughness),
        },
    );
    sim.run(0.5);
    let hotbar = sim.world.get::<Hotbar>(player).cloned().unwrap_or_default();
    assert_eq!(hotbar.entry(HotbarSlot(8)), HotbarEntry::Empty);

    skill::reset::reset(&mut sim.world, player);
    sim.run(0.5);
    assert_eq!(stat::max_health(&sim.world, player), base);
}

#[test]
fn a_status_plays_its_vfx_coming_staying_and_going() {
    let (mut sim, player) = clinic();
    let warded = sim.id::<StatusEffectDef>("warded");

    status::apply(&mut sim.world, player, warded, 10.0, Some(Seconds(3.0)));
    sim.run(0.5);
    assert!(playing(&mut sim, "ward_up", player));
    assert!(playing(&mut sim, "ward_bubble", player));

    sim.later(1.6);
    assert!(playing(&mut sim, "ward_bubble", player));
    assert!(!playing(&mut sim, "ward_break", player));

    sim.later(1.5);
    assert!(!status::has(&sim.world, player, warded));
    assert!(playing(&mut sim, "ward_break", player));
    sim.later(2.0);
    assert!(!playing(&mut sim, "ward_bubble", player));
}

#[test]
fn an_area_skill_leaves_its_status_on_everyone_it_hit() {
    let (mut sim, player) = clinic();
    knows_fireball(&mut sim, player);
    let aimed = position(&sim.world, player).expect("position") + Offset::new(3.0, 0.0);
    let near = dummy(&mut sim, aimed);
    let far = dummy(&mut sim, aimed + Offset::new(1.0, 0.0));
    let area = sim.id::<AreaDef>("clinic");
    let roamer = sim.id::<NpcDef>("roamer");
    let scripted = npc::spawn(
        &mut sim.world,
        roamer,
        aimed + Offset::new(0.0, 1.0),
        area,
        Pack(7),
    );
    sim.run(0.5);
    let burning = sim.id::<StatusEffectDef>("burning");

    cast_fireball(&mut sim, aimed);
    let hit = [near, far, scripted];
    assert!(sim.run_until(10.0, |world| {
        hit.iter()
            .all(|target| status::has(world, *target, burning))
    }));
    sim.run(0.5);
    for target in hit {
        assert!(playing(&mut sim, "embers", target));
    }
}

fn clinic() -> (Sim, Entity) {
    let content = Game::new()
        .area("clinic", &Map::open(13, 9), json!([]))
        .hero("clinic", 4.0, 4.0)
        .edit("class", "hero", |hero| {
            hero["tree"] = json!({
                "toughness": { "skill": "toughness.skill.json", "at": { "x": 0.0, "y": 0.0 } },
                "fireball": { "skill": "fireball.skill.json", "at": { "x": 80.0, "y": 0.0 } },
            });
        })
        .row("status_effect", "guarded", effect("buff", json!({ "on_activate": "guard_up.vfx.json" }), "guarded_boost"))
        .script("guarded_boost", "status_effect", boost_by_power())
        .row(
            "status_effect",
            "warded",
            effect(
                "buff",
                json!({ "on_activate": "ward_up.vfx.json", "during": "ward_bubble.vfx.json", "on_expire": "ward_break.vfx.json" }),
                "warded_boost",
            ),
        )
        .script("warded_boost", "status_effect", boost_by_power())
        .row(
            "status_effect",
            "poisoned",
            effect("debuff", json!({ "during": "poison_bubbles.vfx.json" }), "poisoned_damage"),
        )
        .script("poisoned_damage", "status_effect", every(2.0, "damage"))
        .row(
            "status_effect",
            "sanctified",
            effect("buff", json!({ "during": "holy_motes.vfx.json" }), "sanctified_heal"),
        )
        .script("sanctified_heal", "status_effect", every(2.0, "heal"))
        .row(
            "status_effect",
            "burning",
            effect("debuff", json!({ "during": "embers.vfx.json" }), "burning_damage"),
        )
        .script("burning_damage", "status_effect", every(1.0, "damage"))
        .asset("icons/status.png", testkit::png(1, 1))
        .item("antidote", usable("antidote_drink"))
        .script(
            "antidote_drink",
            "item",
            drinking(json!({
                "drink": { "type": "items", "take": [{ "item": "antidote.item.json" }], "done": "cleanse" },
                "cleanse": { "type": "remove_status", "category": "debuff" },
            })),
        )
        .item("bog_water", usable("bog_water_drink"))
        .script(
            "bog_water_drink",
            "item",
            drinking(json!({
                "drink": { "type": "items", "take": [{ "item": "bog_water.item.json" }], "done": "sicken" },
                "sicken": { "type": "apply_status", "effect": "poisoned.status_effect.json", "power": 2.0, "seconds": 12.0 },
            })),
        )
        .item(
            "wing",
            json!({
                "kind": { "type": "resource" },
                "modules": [{ "type": "stack", "max": 50 }, { "type": "script", "script": "wing_boost.script.json" }],
            }),
        )
        .script("wing_boost", "item", boosting("movement_speed", 0.5))
        .item(
            "blade",
            json!({
                "kind": { "type": "equipment", "slot": "weapon" },
                "modules": [{ "type": "script", "script": "blade_boost.script.json" }],
            }),
        )
        .script("blade_boost", "item", boosting("damage", 4.0))
        .row(
            "skill",
            "toughness",
            json!({
                "name": "Toughness",
                "icon": "icons/status.png",
                "kind": "passive",
                "target": "self",
                "levels": [{ "cost": 1, "stats": { "power": 6.0 } }],
                "modules": [{ "type": "script", "script": "toughness_harden.script.json" }],
            }),
        )
        .script(
            "toughness_harden",
            "skill",
            json!({
                "activated": { "type": "on", "event": "activated", "next": "harden" },
                "harden": {
                    "type": "boost",
                    "stat": "max_health",
                    "by": { "type": "skill_stat", "skill": "toughness.skill.json", "stat": "power" },
                },
            }),
        )
        .row(
            "skill",
            "fireball",
            json!({
                "name": "Fireball",
                "icon": "icons/status.png",
                "target": "area_enemy",
                "range": 6.0,
                "area": { "type": "circle", "radius": 2.0 },
                "levels": [{ "cost": 1, "stats": { "power": 1.0 } }],
                "modules": [{ "type": "script", "script": "fireball_burn.script.json" }],
            }),
        )
        .script(
            "fireball_burn",
            "skill",
            json!({
                "used": { "type": "on", "event": "used", "next": "cast" },
                "cast": { "type": "animate", "action": "attack", "wait": "apex", "next": "burn" },
                "burn": {
                    "type": "apply_status",
                    "who": { "type": "skill_targets" },
                    "effect": "burning.status_effect.json",
                    "power": 1.0,
                    "seconds": 6.0,
                },
            }),
        )
        .npc("dummy", 100.0, json!([]))
        .npc("roamer", 100.0, json!([{ "type": "script", "script": "roamer_rest.script.json" }]))
        .script(
            "roamer_rest",
            "npc",
            json!({
                "spawned": { "type": "on", "event": "started", "next": "hostile" },
                "hostile": { "type": "set_attitude", "attitude": "hostile", "next": "rest" },
                "rest": { "type": "wait", "seconds": 0, "at_most": 1.0, "next": "rest" },
            }),
        )
        .vfx("guard_up")
        .vfx("ward_up")
        .vfx("ward_bubble")
        .vfx("ward_break")
        .vfx("poison_bubbles")
        .vfx("holy_motes")
        .vfx("embers")
        .vfx("damage_taken")
        .shader("damage_pop")
        .vfx("healed")
        .shader("heal_pop")
        .shader("status_pop")
        .vfx("defeated")
        .sfx("ui_refuse")
        .load();
    let mut sim = Sim::new(content);
    sim.serve("clinic")
        .accept::<UseItemRequest>()
        .accept::<UnequipRequest>()
        .accept::<UseSkillRequest>()
        .accept::<AssignHotbarRequest>()
        .accept::<AssignBasicAttackRequest>()
        .add((
            (status::expire, status::aspect::settle, status::aspect::tick).chain(),
            vfx::expire,
            skill::casting::request,
            skill::hotbar::requests,
            item::use_item,
            equipment::unequip,
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

fn effect(category: &str, vfx: Value, script: &str) -> Value {
    json!({
        "name": "Effect",
        "icon": "icons/status.png",
        "category": category,
        "description": "An effect.",
        "vfx": vfx,
        "modules": [{ "type": "script", "script": path("script", script) }],
    })
}

fn boost_by_power() -> Value {
    json!({
        "activated": { "type": "on", "event": "activated", "next": "hold" },
        "hold": { "type": "boost", "stat": "max_health", "by": { "type": "status_power" } },
    })
}

fn every(seconds: f32, act: &str) -> Value {
    json!({
        "every": { "type": "on", "event": "interval", "seconds": seconds, "next": "act" },
        "act": { "type": act, "by": { "type": "status_power" } },
    })
}

fn usable(script: &str) -> Value {
    json!({
        "kind": { "type": "usable" },
        "modules": [{ "type": "stack", "max": 5 }, { "type": "script", "script": path("script", script) }],
    })
}

fn drinking(mut nodes: Value) -> Value {
    nodes["used"] = json!({ "type": "on", "event": "used", "next": "drink" });
    nodes
}

fn boosting(stat: &str, by: f32) -> Value {
    json!({
        "activated": { "type": "on", "event": "activated", "next": "boost" },
        "boost": { "type": "boost", "stat": stat, "by": by },
    })
}

fn use_item(sim: &mut Sim, player: Entity, key: &str) {
    sim.give(player, key, 1);
    let slot = sim.slot_of(player, key);
    let conn = sim.conn(1);
    sim.send(conn, UseItemRequest { slot });
    sim.run(0.5);
}

fn knows_fireball(sim: &mut Sim, player: Entity) {
    sim.world
        .entity_mut(player)
        .insert(skill::Learned(BTreeMap::from([(
            NodeKey::from("fireball"),
            Rank(1),
        )])));
}

fn cast_fireball(sim: &mut Sim, at: Pos<Meters>) {
    let conn = sim.conn(1);
    let fireball = sim.id::<SkillDef>("fireball");
    sim.send(
        conn,
        UseSkillRequest {
            skill: fireball,
            aim: CastAim::Point(at),
        },
    );
}

/// A hostile dummy that stands where it's put and never fights back.
fn dummy(sim: &mut Sim, at: Pos<Meters>) -> Entity {
    let area = sim.id::<AreaDef>("clinic");
    let def = sim.id::<NpcDef>("dummy");
    let dummy = npc::spawn(&mut sim.world, def, at, area, Pack(u32::MAX));
    sim.world.entity_mut(dummy).insert(Attitude::Hostile);
    sim.tick();
    dummy
}

fn statuses(sim: &Sim, who: Entity) -> Statuses {
    sim.world.get::<Statuses>(who).cloned().unwrap_or_default()
}

fn playing(sim: &mut Sim, vfx: &str, on: Entity) -> bool {
    let wanted = sim.id::<VfxDef>(vfx);
    sim.world
        .query::<&VfxInstance>()
        .iter(&sim.world)
        .any(|instance| instance.vfx == wanted && instance.site == VfxSite::Actor(on))
}
