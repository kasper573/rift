use bevy_ecs::message::{MessageCursor, Messages};
use bevy_ecs::prelude::*;
use bevy_replication::ToClient;
use engine::core::audio::playback::{SfxDef, SfxId};
use engine::core::content::Content;
use engine::core::ground::{GroundSize, WaterSurface};
use engine::core::math::{Offset, Pos};
use engine::core::units::{Meters, Seconds};
use engine::systems::actor::held::Held;
use engine::systems::actor::skeleton::MoveName;
use engine::systems::actor::{Motion, Playing};
use engine::systems::area::{self, AreaDef};
use engine::systems::item::{ItemDef, ItemId};
use engine::systems::movement::{self, MoveRequest, Position, position};
use engine::systems::notification::SoundPlayed;
use engine::systems::resource::{
    self, GatherRequest, Left, ResourceAfloat, ResourceDef, StopGathering, Working,
};
use engine::systems::spawner::Spawned;
use engine::systems::stat::{self, StatKind, Stats};
use engine::systems::{actor, region};
use serde_json::{Value, json};
use testkit::{APEX, BODY, GEAR, Game, HEIGHTS, Map, Sim, path, rig_clips};

const TAKES: f32 = 2.0;
const SWING: f32 = 0.5;
const AWAY: f32 = 4.0;
const WALK: f32 = 3.0;
/// Where the lake of a meadow that has one begins.
const SHORE: f32 = 10.0;

#[test]
fn an_extraction_takes_one_rolls_once_and_pays_out_and_is_refused_without_the_tool_room_or_bag_room()
 {
    let (mut sim, vein) = quarry(json!({ "quantity": 3 }), |game| game);
    let miner = sim.join(1);
    sim.equip(miner, "pickaxe");
    ask(&mut sim, 1, vein);
    assert!(sim.run_until(WALK, |world| world.get::<Working>(miner).is_some()));
    assert_eq!(sim.count(miner, "ore"), 0);
    let mut sounds = MessageCursor::default();
    let heard = hear(&mut sim, &mut sounds, TAKES - 0.2);
    assert_eq!(left(&sim, vein), Some(3), "it gave before its time was up");
    let paid = hear(&mut sim, &mut sounds, 0.4);
    assert_eq!(left(&sim, vein), Some(2));
    assert_eq!(
        [heard, paid].concat(),
        [],
        "an extraction has no sound of its own: what it pays out sounds as any item gained does"
    );
    assert_eq!(sim.count(miner, "ore"), 2, "one roll of two ore");
    assert!(
        sim.world.get::<Working>(miner).is_some(),
        "work goes on by itself"
    );

    set(&mut sim, miner, StatKind::GatheringSpeed, 2.0);
    set(&mut sim, miner, StatKind::GatheringEfficiency, 100.0);
    sim.run(TAKES + 0.2);
    let swing = swung(&sim, miner).expect("it swings on");
    assert!(
        (swing.ends.since(swing.started).0 - SWING / 2.0).abs() < 0.01,
        "a swing takes half the time at twice the speed: {swing:?}"
    );
    assert_eq!(
        left(&sim, vein),
        Some(1),
        "the extraction under way keeps the time it began with"
    );
    assert_eq!(
        sim.count(miner, "ore"),
        6,
        "a full efficiency rolls the drops twice for the one taken"
    );
    sim.run(TAKES / 2.0 + 0.2);
    assert_eq!(
        left(&sim, vein),
        None,
        "the last one is taken and it is gone"
    );
    assert_eq!(
        sim.count(miner, "ore"),
        10,
        "in half the time at twice the speed"
    );
    assert!(sim.world.get::<Working>(miner).is_none());
    assert_eq!(sim.errors(1), Vec::<String>::new());

    let (mut sim, vein) = quarry(json!({ "quantity": 3 }), |game| game);
    let miner = sim.join(1);
    for worn in ["pickaxe", "blade", "buckler"] {
        sim.equip(miner, worn);
    }
    let armed = shown(&sim, miner);
    assert_eq!(armed.len(), 2, "a blade and a buckler in hand: {armed:?}");
    ask(&mut sim, 1, vein);
    assert!(sim.run_until(WALK, |world| world.get::<Working>(miner).is_some()));
    assert_eq!(
        shown(&sim, miner),
        [sim.id::<ItemDef>("pickaxe")],
        "the tool alone is in hand at work, whatever was held"
    );
    let first = swung(&sim, miner).expect("work begins with a swing");
    assert_eq!(
        (first.motion, first.sound),
        (
            Motion::Move(MoveName::new("mine")),
            Some(sim.id::<SfxDef>("crack"))
        ),
        "the move of the kind of tool worked with, landing with its sound"
    );
    sim.run(TAKES - 0.5);
    let later = swung(&sim, miner).expect("it swings for as long as it works");
    assert!(later.started > first.started, "one swing after another");
    let conn = sim.conn(1);
    sim.send(conn, StopGathering);
    sim.tick();
    assert_eq!(swung(&sim, miner), None, "the swing ends with the work");
    assert_eq!(shown(&sim, miner), armed, "and what was held comes back");
    sim.run(1.0);
    assert_eq!(left(&sim, vein), Some(3), "work stopped under way took one");
    assert_eq!(sim.count(miner, "ore"), 0);

    let (mut sim, vein) = quarry(json!({ "quantity": 3 }), |game| game);
    let miner = sim.join(1);
    let start = position(&sim.world, miner).expect("it stands");
    ask(&mut sim, 1, vein);
    sim.run(1.0);
    assert_eq!(sim.errors(1), ["Needs a Pick"]);
    sim.forget_notified();
    sim.equip(miner, "hatchet");
    ask(&mut sim, 1, vein);
    sim.run(1.0);
    assert_eq!(sim.errors(1), ["Needs a Pick"], "a tool of another kind");
    assert_eq!(position(&sim.world, miner), Some(start));
    assert!(sim.world.get::<Working>(miner).is_none());

    sim.forget_notified();
    sim.equip(miner, "pickaxe");
    ask(&mut sim, 1, vein);
    assert!(sim.run_until(WALK, |world| world.get::<Working>(miner).is_some()));
    let other = sim.join(2);
    sim.equip(other, "pickaxe");
    ask(&mut sim, 2, vein);
    sim.run(1.0);
    assert_eq!(sim.errors(2), ["No room"]);
    assert!(sim.world.get::<Working>(other).is_none());

    let (mut sim, vein) = quarry(json!({ "quantity": 3 }), |game| {
        game.edit_settings(|settings| settings["slots"]["bag"] = json!(1))
    });
    let miner = sim.join(1);
    sim.equip(miner, "pickaxe");
    sim.give(miner, "hatchet", 1);
    ask(&mut sim, 1, vein);
    sim.run(1.0);
    assert_eq!(sim.errors(1), ["No room in your bag"]);
    assert!(sim.world.get::<Working>(miner).is_none());

    let (mut sim, vein) = quarry(json!({ "quantity": 3 }), |game| game);
    let miner = sim.join(1);
    sim.equip(miner, "pickaxe");
    set(&mut sim, miner, StatKind::GatheringEfficiency, 150.0);
    ask(&mut sim, 1, vein);
    assert!(sim.run_until(WALK, |world| world.get::<Working>(miner).is_some()));
    sim.run(TAKES + 0.2);
    assert_eq!(left(&sim, vein), Some(2));
    assert!(
        [4, 6].contains(&sim.count(miner, "ore")),
        "half as much again rolls twice every time, and a third time half the times"
    );

    let by_hand = json!({ "tool": path("tool_type", "hands") });
    let (mut sim, bush) = quarry(by_hand, |game| game);
    let picker = sim.join(1);
    ask(&mut sim, 1, bush);
    assert!(sim.run_until(WALK, |world| world.get::<Working>(picker).is_some()));
    assert_eq!(
        swung(&sim, picker).map(|swing| swing.motion),
        Some(Motion::Move(MoveName::new("pluck"))),
        "work by hand makes the move it names"
    );

    let (mut sim, vein) = quarry(json!({}), |game| {
        game.prop("rock", json!({ "type": "circle", "radius": 1.5 }))
    });
    let miner = sim.join(1);
    sim.equip(miner, "pickaxe");
    ask(&mut sim, 1, vein);
    assert!(sim.run_until(WALK, |world| world.get::<Working>(miner).is_some()));
    let stands = position(&sim.world, miner).expect("it stands");
    let stood = position(&sim.world, vein).expect("it stands");
    assert!(
        stands.distance_to(stood) >= 1.5,
        "work is done from the edge of the ground its look takes up"
    );
}

#[test]
fn two_workers_never_both_get_the_last_one() {
    let (mut sim, vein) = quarry(json!({ "quantity": 1, "max_occupants": 2 }), |game| game);
    let miners = [sim.join(1), sim.join(2)];
    for miner in miners {
        sim.equip(miner, "pickaxe");
    }
    ask(&mut sim, 1, vein);
    ask(&mut sim, 2, vein);
    sim.run(WALK + TAKES + 1.0);

    assert_eq!(left(&sim, vein), None);
    let got = miners.map(|miner| sim.count(miner, "ore"));
    assert!(
        got == [2, 0] || got == [0, 2],
        "one extraction was paid out to {got:?}"
    );
    let told = [sim.errors(1), sim.errors(2)];
    for (got, told) in got.iter().zip(&told) {
        match got {
            0 => assert_eq!(told, &["It ran out"]),
            _ => assert_eq!(told, &Vec::<String>::new()),
        }
    }
    for miner in miners {
        assert!(sim.world.get::<Working>(miner).is_none());
    }
}

#[test]
fn a_resource_takes_what_it_spends_with_every_extraction_and_is_worked_from_as_far_as_it_reaches() {
    let blasted = json!({
        "quantity": 5,
        "spends": [{ "item": path("item", "powder") }],
        "reach": 3.0,
    });
    let (mut sim, vein) = quarry(blasted, |game| game);
    let miner = sim.join(1);
    sim.equip(miner, "pickaxe");
    let start = position(&sim.world, miner).expect("it stands");
    ask(&mut sim, 1, vein);
    sim.run(1.0);
    assert_eq!(
        (sim.errors(1), position(&sim.world, miner)),
        (vec!["Needs 1 powder more".to_owned()], Some(start)),
        "what it spends is asked for before a step is taken, as its tool is"
    );
    sim.forget_notified();

    sim.give(miner, "powder", 2);
    ask(&mut sim, 1, vein);
    assert!(sim.run_until(WALK, |world| world.get::<Working>(miner).is_some()));
    let stands = position(&sim.world, miner).expect("it stands");
    let stood = position(&sim.world, vein).expect("it stands");
    assert!(
        (2.0..=3.0).contains(&stands.distance_to(stood)),
        "it is worked from as far off as it reaches: {stands:?}"
    );
    sim.run(TAKES / 2.0);
    let conn = sim.conn(1);
    sim.send(conn, StopGathering);
    sim.tick();
    sim.run(TAKES);
    assert_eq!(
        (sim.count(miner, "powder"), left(&sim, vein)),
        (2, Some(5)),
        "work stopped under way spends nothing"
    );

    ask(&mut sim, 1, vein);
    sim.run(TAKES + 0.2);
    assert_eq!(
        (
            sim.count(miner, "powder"),
            sim.count(miner, "ore"),
            left(&sim, vein)
        ),
        (1, 2, Some(4)),
        "an extraction takes what it spends as it gives what it drops"
    );
    sim.run(TAKES);
    assert_eq!(
        (
            sim.count(miner, "powder"),
            sim.count(miner, "ore"),
            left(&sim, vein),
            sim.world.get::<Working>(miner).is_some(),
            sim.errors(1),
        ),
        (0, 4, Some(3), false, vec!["Needs 1 powder more".to_owned()]),
        "the extraction that spent the last of it is the last, and the worker is told what is missing"
    );

    let barren = json!({
        "quantity": 5,
        "spends": [{ "item": path("item", "powder"), "count": 2 }],
        "drops": [{ "item": path("item", "ore"), "chance": 0.0 }],
    });
    let (mut sim, vein) = quarry(barren, |game| game);
    let miner = sim.join(1);
    sim.equip(miner, "pickaxe");
    sim.give(miner, "powder", 3);
    ask(&mut sim, 1, vein);
    assert!(sim.run_until(WALK, |world| world.get::<Working>(miner).is_some()));
    sim.run(TAKES + 0.2);
    assert_eq!(
        (
            sim.count(miner, "powder"),
            sim.count(miner, "ore"),
            left(&sim, vein),
            sim.errors(1),
        ),
        (1, 0, Some(4), vec!["Needs 1 powder more".to_owned()]),
        "an extraction that drops nothing spends all the same"
    );
}

#[test]
fn a_spawner_on_water_keeps_its_resources_afloat_where_water_stands() {
    let shoals = |more: Value| {
        let mut spawner = json!({
            "type": "spawner",
            "populations": [{
                "spawns": [{ "type": "resource", "resource": path("resource", "vein") }],
                "max_spawned_at_once": 8,
            }],
            "on": "water",
        });
        spawner
            .as_object_mut()
            .expect("the spawner")
            .extend(more.as_object().expect("fields").clone());
        spawner
    };
    let far = json!({ "reach": 8.0, "max_occupants": 8 });
    let (mut sim, on_land) = quarry(far.clone(), |game| lake(game, shoals(json!({}))));
    region::hold_whole(&mut sim.world);
    let mut spawned = sim
        .world
        .query_filtered::<(Entity, &Position, Has<ResourceAfloat>), With<Spawned>>();
    let afloat: Vec<(Entity, Pos<Meters>, bool)> = spawned
        .iter(&sim.world)
        .map(|(shoal, at, afloat)| (shoal, at.pos, afloat))
        .collect();
    assert_eq!(afloat.len(), 8);
    let meadow = area::load(sim.world.resource::<Content>(), sim.id::<AreaDef>("meadow"));
    let wet = |at: Pos<Meters>| meadow.water.depth_at(&meadow.heights, at) > Meters(0.0);
    assert!(
        afloat.iter().all(|(_, at, afloat)| wet(*at) && *afloat),
        "each lies where water stands, and is told apart from what stands on the ground: {afloat:?}"
    );
    assert!(!wet(Pos::new(2.0, 5.0)), "the meadow's west is dry");
    assert!(
        sim.world.get::<ResourceAfloat>(on_land).is_none(),
        "what stands on the ground is not afloat"
    );
    let miner = sim.join(1);
    sim.equip(miner, "pickaxe");
    let nearest = afloat
        .iter()
        .min_by(|a, b| a.1.x.total_cmp(&b.1.x))
        .expect("a shoal");
    ask(&mut sim, 1, nearest.0);
    assert!(
        sim.run_until(4.0 * WALK, |world| world.get::<Working>(miner).is_some()),
        "one on the water is worked as any other, from within its reach"
    );

    let dry = shoals(json!({ "within": { "x": 1.0, "y": 1.0, "w": 6.0, "h": 6.0 } }));
    assert_eq!(
        refused(lake(game(far.clone()), dry)),
        ["no water to spawn on"]
    );
    let mut walkers = shoals(json!({}));
    walkers["populations"][0]["spawns"] = json!([{ "type": "npc", "npc": path("npc", "wolf") }]);
    let wolves = lake(game(far), walkers)
        .npc("wolf", 20.0, json!([]))
        .shader("damage_pop")
        .vfx("defeated");
    assert_eq!(refused(wolves), ["walks on water"]);
}

/// A meadow holding one vein of `fields` a walk away from where players start, worked with a
/// pick for two ore an extraction, in the game `more` adds to.
fn quarry(fields: Value, more: impl FnOnce(Game) -> Game) -> (Sim, Entity) {
    let mut sim = Sim::new(more(game(fields)).load());
    sim.serve("meadow")
        .accept::<MoveRequest>()
        .accept::<GatherRequest>()
        .accept::<StopGathering>()
        .message::<ToClient<SoundPlayed>>()
        .add((
            movement::move_request,
            movement::file_by_chunk,
            resource::gather_request,
            actor::playing::progress,
            resource::swings,
        ))
        .plan(movement::AdvanceWalks::default())
        .add((resource::gathering, resource::work))
        .plan(stat::SettleVitals::default())
        .plan(actor::SettleActions::default());
    engine::systems::spawn_scripts(&mut sim.world);
    let area = sim.id::<AreaDef>("meadow");
    let def = sim.id::<ResourceDef>("vein");
    let at = Pos::<Meters>::new(2.0, 5.0) + Offset::new(AWAY, 0.0);
    let vein = resource::spawn(&mut sim.world, def, at, area);
    (sim, vein)
}

/// A game of one meadow and a vein of `fields`, with the tools, the moves and the sounds its
/// work takes.
fn game(fields: Value) -> Game {
    let mut vein = json!({
        "name": "Vein",
        "look": path("prop", "rock"),
        "quantity": 1,
        "max_occupants": 1,
        "tool": path("tool_type", "pick"),
        "takes": TAKES,
        "drops": [{ "item": path("item", "ore"), "amount": 2 }],
    });
    vein.as_object_mut()
        .expect("the vein")
        .extend(fields.as_object().expect("fields").clone());
    let kind = |name: &str, work: &str, by_hand: bool| {
        json!({
            "name": name,
            "by_hand": by_hand,
            "work": work,
            "cursor": path("cursor", "dig"),
            "sfx": {
                "swing": path("sfx", "crack"),
                "ran_out": path("sfx", "crumble"),
            },
        })
    };
    let held = |stance: &str| {
        json!({ "kind": {
            "type": "equipment", "category": path("equipment_category", GEAR),
            "wear": "held",
            "stance": path("stance", stance),
        } })
    };
    let tool = |kind: &str| {
        json!({ "kind": {
            "type": "equipment", "category": path("equipment_category", GEAR),
            "wear": "tool",
            "tool": path("tool_type", kind),
            "stance": path("stance", "swing"),
        } })
    };
    let mut clips = rig_clips();
    clips.extend(["mine", "hew", "pluck"].map(|swing| (swing, Seconds(SWING))));
    let lands = |clip: &str| json!({ "clip": testkit::clip(clip), "cues": [{ "at": APEX.0, "apex": true }] });
    Game::new()
        .skeleton_of(
            BODY,
            &clips,
            json!({
                "stances": [{
                    "stance": path("stance", "swing"),
                    "set": { "moves": { "mine": testkit::clip("mine"), "hew": testkit::clip("hew") } },
                }],
                "cues": [lands("attack"), lands("mine"), lands("hew"), lands("pluck")],
            }),
        )
        .edit("skeleton", BODY, |body| {
            body["base"]["moves"] = json!({ "pluck": testkit::clip("pluck") });
        })
        .area("meadow", &Map::open(20, 10), json!([]))
        .hero("meadow", 2.0, 5.0)
        .stance("swing", 1, "main")
        .row("tool_type", "pick", kind("Pick", "mine", false))
        .row("tool_type", "axe", kind("Axe", "hew", false))
        .row("tool_type", "hands", kind("Hands", "pluck", true))
        .cursor("dig")
        .equipment("pickaxe", tool("pick"))
        .equipment("hatchet", tool("axe"))
        .stance("guard", 1, "off")
        .equipment("blade", held("swing"))
        .equipment("buckler", held("guard"))
        .resource("ore", 20)
        .prop("rock", json!({ "type": "none" }))
        .sfx("crack")
        .sfx("crumble")
        .sfx("ui_refuse")
        .resource("powder", 20)
        .row("resource", "vein", vein)
}

/// The meadow of `game` with a lake over its eastern half, `spawner` placed in it.
fn lake(game: Game, spawner: Value) -> Game {
    let size = GroundSize {
        width: 20,
        height: 10,
    };
    let water = WaterSurface::from_fn(size, HEIGHTS, |column, _| {
        Meters(if column as f32 >= SHORE * 2.0 {
            1.0
        } else {
            0.0
        })
    });
    game.area("meadow", &Map::open(20, 10), json!([spawner]))
        .water("meadow", &water)
}

fn refused(game: Game) -> Vec<&'static str> {
    let report = game.try_load().err().expect("refused");
    report
        .errors
        .iter()
        .map(|error| error.problem.rule)
        .collect()
}

fn ask(sim: &mut Sim, client: u32, target: Entity) {
    let conn = sim.conn(client);
    sim.send(conn, GatherRequest { target });
}

/// Runs on for `seconds` and tells every sound sent to anyone meanwhile.
fn hear(
    sim: &mut Sim,
    sounds: &mut MessageCursor<ToClient<SoundPlayed>>,
    seconds: f32,
) -> Vec<SfxId> {
    let mut heard = Vec::new();
    sim.run_until(seconds, |world| {
        let sent = world.resource::<Messages<ToClient<SoundPlayed>>>();
        heard.extend(sounds.read(sent).map(|sent| sent.message.sfx));
        false
    });
    heard
}

/// The items in the worker's hands, in hand order.
fn shown(sim: &Sim, worker: Entity) -> Vec<ItemId> {
    sim.world
        .get::<Held>(worker)
        .map(|held| held.hands.values().copied().collect())
        .unwrap_or_default()
}

/// The swing the worker is in the middle of, if any.
fn swung(sim: &Sim, worker: Entity) -> Option<Playing> {
    sim.world.get::<Playing>(worker).cloned()
}

/// How much the resource still holds, none once it is gone.
fn left(sim: &Sim, resource: Entity) -> Option<u32> {
    sim.world.get::<Left>(resource).map(|left| left.0)
}

fn set(sim: &mut Sim, who: Entity, kind: StatKind, value: f32) {
    sim.world
        .get_mut::<Stats>(who)
        .expect("stats")
        .0
        .insert(kind, value);
}
