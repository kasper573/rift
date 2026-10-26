use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use engine::core::math::Offset;
use engine::core::time::{Millis, Seconds, UnixMillis};
use engine::core::units::Meters;
use engine::systems::actor::skeleton::MoveName;
use engine::systems::actor::stance::{HandIndex, StanceDef};
use engine::systems::actor::wearing::{self, Wearing};
use engine::systems::actor::{self, Action, Motion, Playing};
use engine::systems::area::AreaDef;
use engine::systems::combat::{self, AttackRequest, Attitude};
use engine::systems::equipment::{self, Equipment, EquipmentSlot};
use engine::systems::item::{DroppedItem, Inventory, ItemDef, ItemId};
use engine::systems::movement::{self, position};
use engine::systems::npc::{self, NpcDef, Pack};
use engine::systems::skill::{self, SkillDef};
use engine::systems::stat::{self, StatKind};
use engine::systems::{rewards, script, status};
use serde_json::{Value, json};
use testkit::{APEX, BODY, CLIP, Game, Map, Sim, path, rig_clips};

/// How long each clip of a stance plays, and when it lands.
const SLASH: (Millis, Millis) = (Millis(600.0), Millis(400.0));
const THRUST: (Millis, Millis) = (Millis(500.0), Millis(250.0));
const STAB: (Millis, Millis) = (Millis(400.0), Millis(150.0));

#[test]
fn a_held_item_plays_its_stances_clip_where_the_body_has_its_set() {
    let plain = armoury().skeleton("plain").edit("model", "hero", |hero| {
        hero["skeleton"] = json!(path("skeleton", "plain"))
    });
    let cases = [
        ("a body with a sword set", armoury(), SLASH),
        ("a body without one", plain, (CLIP, APEX)),
    ];
    for (case, game, timing) in cases {
        let (mut sim, player) = armed(game);
        wear(&mut sim, player, "sword");

        let played =
            actor::playing::play(&mut sim.world, player, Motion::Action(Action::Attack), None)
                .expect("an attack plays");

        assert_timed(&sim, player, &played, timing, case);
        let strike = sim.id::<SkillDef>("strike");
        assert!(
            skill::known(&sim.world, player).contains_key(&strike),
            "{case}: the sword grants its skill"
        );
    }
}

#[test]
fn the_hands_put_their_wearer_in_one_stance() {
    let cases: [(&str, &[&str], [&str; 2], &str); 3] = [
        (
            "two daggers",
            &["dagger", "dagger"],
            ["dagger", "dagger"],
            "dual_daggers",
        ),
        (
            "a sword, then a dagger",
            &["sword", "dagger"],
            ["sword", "dagger"],
            "sword",
        ),
        (
            "a dagger, then a sword",
            &["dagger", "sword"],
            ["sword", "dagger"],
            "sword",
        ),
    ];
    for (case, equipped, [main, off], stance) in cases {
        let (mut sim, player) = armed(armoury());
        for item in equipped {
            wear(&mut sim, player, item);
        }

        assert_eq!(
            hands(&sim, player),
            BTreeMap::from([
                (HandIndex::MAIN, sim.id::<ItemDef>(main)),
                (HandIndex::OFF, sim.id::<ItemDef>(off)),
            ]),
            "{case}"
        );
        assert_eq!(
            wearing::stance_of(&sim.world, player),
            Some(sim.id::<StanceDef>(stance)),
            "{case}"
        );
    }
}

#[test]
fn a_two_hander_takes_both_hands_returning_what_they_held_or_is_refused_whole() {
    for full in [false, true] {
        let (mut sim, player) = armed(armoury());
        wear(&mut sim, player, "sword");
        wear(&mut sim, player, "shield");
        sim.give(player, "greatsword", 1);
        if full {
            let mut bag = sim.world.get_mut::<Inventory>(player).expect("a bag");
            bag.max = bag.slots.len() as u32;
        }
        let before = (hands(&sim, player), bag(&sim, player));
        let greatsword = sim.id::<ItemDef>("greatsword");

        let equipped = equipment::equip_held(&mut sim.world, player, greatsword);

        if full {
            assert!(equipped.is_err(), "a full bag has no room for both");
            assert_eq!((hands(&sim, player), bag(&sim, player)), before);
            continue;
        }
        equipped.expect("the greatsword is worn");
        assert_eq!(
            hands(&sim, player),
            BTreeMap::from([(HandIndex::MAIN, greatsword), (HandIndex::OFF, greatsword)])
        );
        assert_eq!(
            [
                sim.count(player, "sword"),
                sim.count(player, "shield"),
                sim.count(player, "greatsword")
            ],
            [1, 1, 0]
        );
    }
}

#[test]
fn a_move_the_stance_lacks_plays_its_attack_and_still_lands_at_the_apex() {
    let cases = [
        ("a rapier, whose stance thrusts", "rapier", THRUST),
        ("a dagger, whose stance only stabs", "dagger", STAB),
    ];
    for (case, weapon, timing) in cases {
        let (mut sim, player) = armed(armoury());
        wear(&mut sim, player, weapon);
        let foe = foe(&mut sim, player, "brute");
        sim.tick();
        let full = stat::effective(&sim.world, foe, StatKind::Health);

        let conn = sim.conn(1);
        sim.send(conn, AttackRequest { target: foe });

        assert!(
            sim.run_until(5.0, |world| world.get::<Playing>(player).is_some()),
            "{case}"
        );
        let played = sim.world.get::<Playing>(player).expect("playing").clone();
        assert_eq!(
            played.motion,
            Motion::Move(MoveName::new("thrust")),
            "{case}"
        );
        assert_timed(&sim, player, &played, timing, case);
        assert!(
            sim.run_until(5.0, |world| stat::effective(world, foe, StatKind::Health)
                < full),
            "{case}: the lunge lands"
        );
    }
}

#[test]
fn an_npcs_wear_list_puts_it_in_a_stance_and_grants_nothing() {
    let game = armoury()
        .resource("bone", 10)
        .npc(
            "guard",
            25.0,
            json!([{ "type": "loot", "xp": 0, "items": [{ "item": path("item", "bone") }] }]),
        )
        .edit("npc", "guard", |guard| {
            guard["wearing"] = json!([path("item", "sword")])
        });
    let (mut sim, player) = armed(game);
    let guard = foe(&mut sim, player, "guard");
    sim.tick();

    assert_eq!(
        sim.world
            .get::<Wearing>(guard)
            .map(|wearing| wearing.hands.clone()),
        Some(BTreeMap::from([(
            HandIndex::MAIN,
            sim.id::<ItemDef>("sword")
        )]))
    );
    assert_eq!(
        wearing::stance_of(&sim.world, guard),
        Some(sim.id::<StanceDef>("sword"))
    );
    assert!(skill::known(&sim.world, guard).is_empty());
    assert_eq!(stat::effective(&sim.world, guard, StatKind::Damage), 6.0);

    combat::inflict(&mut sim.world, guard, Some(player), 1_000_000.0);
    sim.tick();
    sim.tick();
    let dropped: Vec<ItemId> = sim
        .world
        .query::<&DroppedItem>()
        .iter(&sim.world)
        .map(|dropped| dropped.item)
        .collect();
    assert_eq!(dropped, vec![sim.id::<ItemDef>("bone")]);
}

#[test]
fn a_players_body_shows_its_equipped_hands_and_never_its_head() {
    let (mut sim, player) = armed(armoury());
    wear(&mut sim, player, "sword");
    wear(&mut sim, player, "helm");

    let sword = sim.id::<ItemDef>("sword");
    assert_eq!(
        shown(&sim, player),
        BTreeMap::from([(HandIndex::MAIN, sword)])
    );

    equipment::unequip_slot(&mut sim.world, player, EquipmentSlot::Hand(HandIndex::MAIN))
        .expect("the sword comes off");
    assert_eq!(shown(&sim, player), BTreeMap::new());
    assert_eq!(
        equipment::in_slot(&sim.world, player, EquipmentSlot::Head),
        Some(sim.id::<ItemDef>("helm"))
    );

    let (sim, player) = armed(armoury().edit("player", "hero", |hero| {
        hero["equipped"] = json!([path("item", "sword"), path("item", "helm")]);
    }));
    assert_eq!(
        shown(&sim, player),
        BTreeMap::from([(HandIndex::MAIN, sim.id::<ItemDef>("sword"))]),
        "a player who joins equipped shows it from the start"
    );
}

/// A field with a hero on a body of the test rig whose library carries a slash, a thrust and a
/// stab beside every action: the sword stance slashes and thrusts, the dagger stance stabs, and two
/// daggers pair. A sword and a dagger strike, a rapier lunges in the sword stance, a greatsword
/// takes both hands, a shield the off hand and a helm the head. Every hand item sharpens its wearer.
fn armoury() -> Game {
    let clip = |(length, _): (Millis, Millis)| length.seconds();
    let apex = |(_, apex): (Millis, Millis)| json!([{ "at": apex.0, "apex": true }]);
    let mut clips: Vec<(&str, Seconds)> = rig_clips();
    clips.extend([
        ("slash", clip(SLASH)),
        ("thrust", clip(THRUST)),
        ("stab", clip(STAB)),
    ]);
    let game = Game::new()
        .skeleton_of(
            BODY,
            &clips,
            json!({
                "stances": [
                    {
                        "stance": path("stance", "sword"),
                        "set": { "clips": { "attack": "slash" }, "moves": { "thrust": "thrust" } },
                    },
                    { "stance": path("stance", "dagger"), "set": { "clips": { "attack": "stab" } } },
                ],
                "cues": {
                    "attack": apex((CLIP, APEX)),
                    "slash": apex(SLASH),
                    "thrust": apex(THRUST),
                    "stab": apex(STAB),
                },
            }),
        )
        .area("field", &Map::open(11, 11), json!([]))
        .hero("field", 5.0, 5.0)
        .npc("brute", 25.0, json!([]))
        .vfx("damage_taken")
        .shader("damage_pop")
        .vfx("defeated")
        .script(
            "keen_edge",
            "item",
            json!({
                "activated": { "type": "on", "event": "activated", "next": "keen" },
                "keen": { "type": "boost", "stat": "damage", "by": 4.0 },
            }),
        )
        .stance("sword", 1, "main")
        .stance("dagger", 1, "either")
        .edit("stance", "dagger", |dagger| {
            dagger["paired"] = json!([{
                "with": path("stance", "dagger"),
                "becomes": path("stance", "dual_daggers"),
            }])
        })
        .stance("dual_daggers", 1, "either")
        .stance("greatsword", 2, "main")
        .stance("shield", 1, "off")
        .item("helm", json!({ "kind": { "type": "equipment", "slot": "head" } }));
    let game = skill(game, "strike", json!({ "action": "attack" }));
    let game = skill(game, "lunge", json!({ "move": "thrust" }));
    [
        ("sword", "sword", Some("strike")),
        ("dagger", "dagger", Some("lunge")),
        ("rapier", "sword", Some("lunge")),
        ("greatsword", "greatsword", None),
        ("shield", "shield", None),
    ]
    .into_iter()
    .fold(game, |game, (name, stance, grants)| {
        let grants: Vec<String> = grants
            .map(|skill| path("skill", skill))
            .into_iter()
            .collect();
        game.item(
            name,
            json!({
                "kind": {
                    "type": "equipment",
                    "slot": "hand",
                    "stance": path("stance", stance),
                    "grants": grants,
                },
                "modules": [{ "type": "script", "script": path("script", "keen_edge") }],
            }),
        )
    })
}

/// A basic attack on one enemy that plays `motion`, landing the wearer's damage at its apex.
fn skill(game: Game, name: &str, motion: Value) -> Game {
    let mut swing = json!({ "type": "animate", "wait": "apex", "next": "hit" });
    for (field, value) in motion.as_object().expect("a motion") {
        swing[field] = value.clone();
    }
    game.row(
        "skill",
        name,
        json!({
            "name": name,
            "icon": format!("icons/{name}.png"),
            "target": "single_enemy",
            "basic_attack": true,
            "levels": [{ "cost": 1, "stats": { "power": 1.0 } }],
            "modules": [{ "type": "script", "script": path("script", name) }],
        }),
    )
    .script(
        name,
        "skill",
        json!({
            "used": { "type": "on", "event": "used", "next": "swing" },
            "swing": swing,
            "hit": {
                "type": "damage",
                "who": { "type": "skill_targets" },
                "by": { "type": "stat", "stat": "damage" },
            },
        }),
    )
    .asset(&format!("icons/{name}.png"), testkit::png(1, 1))
}

fn armed(game: Game) -> (Sim, Entity) {
    let mut sim = Sim::new(game.load());
    sim.serve("field")
        .accept::<AttackRequest>()
        .add((
            combat::request,
            actor::playing::progress,
            skill::casting::casts,
        ))
        .add((
            script::query::take_tick_roster,
            script::run_tick,
            movement::advance,
        ))
        .add((
            (status::expire, status::aspect::settle).chain(),
            stat::settle_health,
            combat::defeats,
            skill::hotbar::settle_basic_attack,
            actor::settle,
        ))
        .add(rewards::grant);
    script::spawn_stateless(&mut sim.world);
    let player = sim.join(1);
    sim.tick();
    (sim, player)
}

/// The npc `name`, hostile, a step from `near`, never striking back.
fn foe(sim: &mut Sim, near: Entity, name: &str) -> Entity {
    let at = position(&sim.world, near).expect("a position") + Offset::<Meters>::new(1.0, 0.0);
    let area = sim.id::<AreaDef>("field");
    let def = sim.id::<NpcDef>(name);
    let foe = npc::spawn(&mut sim.world, def, at, area, Pack(u32::MAX));
    sim.world.entity_mut(foe).insert(Attitude::Hostile);
    foe
}

fn wear(sim: &mut Sim, player: Entity, item: &str) {
    sim.give(player, item, 1);
    let item = sim.id::<ItemDef>(item);
    equipment::equip_held(&mut sim.world, player, item).expect("worn");
}

fn hands(sim: &Sim, player: Entity) -> BTreeMap<HandIndex, ItemId> {
    equipment::hands(sim.world.get::<Equipment>(player).expect("equipment"))
}

fn shown(sim: &Sim, player: Entity) -> BTreeMap<HandIndex, ItemId> {
    sim.world
        .get::<Wearing>(player)
        .map(|wearing| wearing.hands.clone())
        .unwrap_or_default()
}

fn bag(sim: &Sim, player: Entity) -> Vec<(ItemId, u32)> {
    sim.world
        .get::<Inventory>(player)
        .expect("a bag")
        .slots
        .iter()
        .map(|stack| (stack.item, stack.count))
        .collect()
}

/// Whether `played` lasts `length` and lands at `apex` of its clip, at the player's attack speed.
fn assert_timed(
    sim: &Sim,
    player: Entity,
    played: &Playing,
    (length, apex): (Millis, Millis),
    case: &str,
) {
    let speed = stat::effective(&sim.world, player, StatKind::AttackSpeed);
    let since = |at: UnixMillis| (at.0 - played.started.0) as f32;
    assert!(
        (since(played.ends) - length.0 / speed).abs() <= 1.0,
        "{case}: {played:?}"
    );
    assert!(
        (since(played.apex) - apex.0 / speed).abs() <= 1.0,
        "{case}: {played:?}"
    );
}
