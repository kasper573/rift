use bevy_ecs::prelude::*;
use engine::core::math::{Offset, Pos};
use engine::core::time::UnixMillis;
use engine::core::units::{Meters, Millis, Seconds};
use engine::systems::area::AreaDef;
use engine::systems::combat::{self, AttackRequest, Attitude};
use engine::systems::movement::{self, position};
use engine::systems::npc::{self, NpcDef};
use engine::systems::stat::{self, StatKind};
use engine::systems::{actor, skill};
use serde_json::json;
use strum::VariantArray;
use testkit::{APEX, BODY, CLIP, Game, Map, Sim, clip, hunting, path, rig_clips};

#[test]
fn an_attack_walks_into_range_and_strikes() {
    let (mut sim, player) = fight();
    let brute = brute(&mut sim, player, Offset::new(4.0, 0.0));
    let full = stat::effective(&sim.world, brute, StatKind::Health);

    let conn = sim.conn(1);
    sim.send(conn, AttackRequest { target: brute });

    assert!(sim.run_until(10.0, |world| stat::effective(
        world,
        brute,
        StatKind::Health
    ) < full));
    assert_eq!(
        sim.world.get::<actor::Hurt>(brute),
        Some(&actor::Hurt(1)),
        "the blow counts, for its body to flinch at"
    );
}

// Right beside the player, so the fight stays where it starts.
#[test]
fn an_attacks_timing_comes_from_its_clip_at_the_attackers_speed() {
    let (mut sim, player) = fight();
    let speed = stat::effective(&sim.world, player, StatKind::AttackSpeed);

    let playing = actor::playing::play(
        &mut sim.world,
        player,
        actor::Motion::Action(actor::Action::Attack),
        None,
    )
    .expect("an attack plays");

    let since = |at: UnixMillis| (at.0 - playing.started.0) as f32;
    assert!(
        (since(playing.apex) - APEX.0 / speed).abs() <= 1.0,
        "{playing:?}"
    );
    assert!(
        (since(playing.ends) - CLIP.0 / speed).abs() <= 1.0,
        "{playing:?}"
    );
}

#[test]
fn an_attacks_clips_take_turns_each_timed_by_its_own_clip() {
    let long = Millis(CLIP.0 * 2.0);
    for (order, plays) in [("in_turn", vec![0, 1, 2, 0, 1]), ("random", Vec::new())] {
        let clips: Vec<(&str, Seconds)> = rig_clips()
            .into_iter()
            .chain([("thrust", CLIP.seconds()), ("lunge", long.seconds())])
            .collect();
        let mut base: serde_json::Map<String, serde_json::Value> = actor::Action::VARIANTS
            .iter()
            .map(|action| (action.as_ref().to_owned(), json!(clip(action.as_ref()))))
            .collect();
        base.insert(
            "attack".to_owned(),
            json!({ "clips": [clip("attack"), clip("thrust"), clip("lunge")], "order": order }),
        );
        let apex =
            |name: &str| json!({ "clip": clip(name), "cues": [{ "at": APEX.0, "apex": true }] });
        let content = Game::new()
            .area("field", &Map::open(7, 7), json!([]))
            .skeleton_of(
                BODY,
                &clips,
                json!({
                    "base": { "clips": base },
                    "cues": [apex("attack"), apex("thrust"), apex("lunge")],
                }),
            )
            .hero("field", 3.0, 3.0)
            .load();
        let mut sim = Sim::new(content);
        sim.serve("field").add(actor::playing::progress);
        let player = sim.join(1);
        let played: Vec<(usize, f32)> = (0..30)
            .map(|_| {
                let playing = actor::playing::play(
                    &mut sim.world,
                    player,
                    actor::Motion::Action(actor::Action::Attack),
                    None,
                )
                .expect("an attack plays");
                assert!(
                    sim.run_until(1.0, |world| world
                        .get::<actor::playing::Playing>(player)
                        .is_none()),
                    "the swing plays through"
                );
                (playing.pick, (playing.ends.0 - playing.started.0) as f32)
            })
            .collect();
        let speed = stat::effective(&sim.world, player, StatKind::AttackSpeed);
        for (pick, lasts) in &played {
            let clip = if *pick == 2 { long } else { CLIP };
            assert!(
                (lasts - clip.0 / speed).abs() <= 1.0,
                "{order}: pick {pick} lasts as its clip does: {played:?}"
            );
        }
        let picks: Vec<usize> = played.iter().map(|(pick, _)| *pick).collect();
        if !plays.is_empty() {
            assert_eq!(picks[..plays.len()], plays[..], "{order}");
        }
        assert!(
            picks.windows(2).all(|pair| pair[0] != pair[1]),
            "{order}: no clip plays twice running: {picks:?}"
        );
        assert!(
            (0..3).all(|pick| picks.contains(&pick)),
            "{order}: every clip takes its turn: {picks:?}"
        );
    }
}

#[test]
fn an_attacked_monster_is_fought_until_it_falls() {
    let cases = [
        (
            "a monster that never strikes back",
            Foe::Brute,
            Offset::new(1.0, 0.0),
            1.0,
        ),
        (
            "a monster coming for the player, with a longer reach",
            Foe::Hunter,
            Offset::new(4.0, 0.0),
            1.0,
        ),
        (
            "a monster already striking from its longer reach",
            Foe::Hunter,
            Offset::new(4.0, 0.0),
            4.0,
        ),
    ];
    for (case, foe, offset, before) in cases {
        let (mut sim, player) = fight();
        let foe = match foe {
            Foe::Brute => brute(&mut sim, player, offset),
            Foe::Hunter => hunter(&mut sim, player, offset),
        };

        sim.run_until(before, |_| false);
        let conn = sim.conn(1);
        sim.send(conn, AttackRequest { target: foe });

        assert!(
            sim.run_until(60.0, |world| stat::is_dead(world, foe)),
            "{case}"
        );
    }
}

#[test]
fn an_immortal_player_is_worn_down_but_never_falls() {
    let (mut sim, player) = fight();
    let full = stat::effective(&sim.world, player, StatKind::Health);

    stat::apply_damage(&mut sim.world, player, full * 0.5);
    assert!(stat::effective(&sim.world, player, StatKind::Health) < full);

    stat::apply_damage(&mut sim.world, player, full * 10.0);
    sim.tick();
    assert!(!stat::is_dead(&sim.world, player));
}

#[test]
fn a_player_crossing_into_another_area_stays_immortal() {
    let content = Game::new()
        .area("field", &Map::open(7, 7), json!([]))
        .area("meadow", &Map::open(7, 7), json!([]))
        .hero("field", 3.0, 3.0)
        .shader("damage_pop")
        .load();
    let mut field = Sim::new(content.clone());
    field.serve("field");
    let mut meadow = Sim::new(content);
    meadow.serve("meadow");
    let player = field.join(1);

    let player = field.cross(player, &mut meadow, Pos::new(3.0, 3.0));
    let full = stat::effective(&meadow.world, player, StatKind::Health);
    stat::apply_damage(&mut meadow.world, player, full * 10.0);

    assert!(!stat::is_dead(&meadow.world, player));
}

fn fight() -> (Sim, Entity) {
    let content = Game::new()
        .area("field", &Map::open(11, 11), json!([]))
        .hero("field", 5.0, 5.0)
        .sword()
        .npc("brute", 25.0, json!([]))
        .npc(
            "hunter",
            25.0,
            json!([{ "type": "script", "script": "hunter_hunt.script.json" }]),
        )
        .row("script", "hunter_hunt", hunting(8.0))
        .edit("npc", "hunter", |hunter| {
            hunter["basic_attack"] = json!(path("skill", "strike"));
            hunter["stats"]["range"] = json!(2.5);
        })
        .shader("damage_pop")
        .vfx("defeated")
        .load();
    let mut sim = Sim::new(content);
    sim.serve("field")
        .accept::<AttackRequest>()
        .add((combat::request, actor::playing::progress))
        .add(skill::hotbar::settle_basic_attack)
        .plan(skill::casting::StepCasts::default())
        .plan((
            engine::systems::roster::ListRoster::default(),
            engine::core::script::run::FlowRunner::default(),
            npc::body::DriveBodies::default(),
            movement::AdvanceWalks::default(),
        ))
        .plan(stat::SettleVitals::default())
        .add(combat::defeats)
        .plan(actor::SettleActions::default());
    engine::systems::spawn_scripts(&mut sim.world);
    let player = sim.join(1);
    sim.equip(player, "sword");
    sim.tick();
    (sim, player)
}

/// A monster turned hostile that stands where it's put and never strikes back.
fn brute(sim: &mut Sim, player: Entity, offset: Offset<Meters>) -> Entity {
    let at: Pos<Meters> = position(&sim.world, player).expect("the player") + offset;
    let area = sim.id::<AreaDef>("field");
    let brute = sim.id::<NpcDef>("brute");
    let brute = npc::spawn(&mut sim.world, brute, at, area, None);
    sim.world.entity_mut(brute).insert(Attitude::Hostile);
    brute
}

enum Foe {
    Brute,
    Hunter,
}

/// A monster that comes for the nearest player and strikes from a longer reach than a sword's.
fn hunter(sim: &mut Sim, player: Entity, offset: Offset<Meters>) -> Entity {
    let at: Pos<Meters> = position(&sim.world, player).expect("the player") + offset;
    let area = sim.id::<AreaDef>("field");
    let hunter = sim.id::<NpcDef>("hunter");
    npc::spawn(&mut sim.world, hunter, at, area, None)
}
