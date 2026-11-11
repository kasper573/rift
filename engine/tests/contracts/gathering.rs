use bevy_ecs::message::{MessageCursor, Messages};
use bevy_ecs::prelude::*;
use bevy_replication::ToClient;
use engine::core::audio::playback::{SfxDef, SfxId};
use engine::core::math::{Offset, Pos};
use engine::core::units::{Meters, Seconds};
use engine::systems::actor::skeleton::MoveName;
use engine::systems::actor::{Motion, Playing};
use engine::systems::area::AreaDef;
use engine::systems::movement::{self, MoveRequest, position};
use engine::systems::notification::SoundPlayed;
use engine::systems::resource::{self, GatherRequest, Left, ResourceDef, StopGathering, Working};
use engine::systems::stat::{self, StatKind, Stats};
use engine::systems::{actor, script};
use serde_json::{Value, json};
use testkit::{APEX, BODY, Game, Map, Sim, path, rig_clips};

const TAKES: f32 = 2.0;
const SWING: f32 = 0.5;
const AWAY: f32 = 4.0;
const WALK: f32 = 3.0;

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
    let [crack, chip] = ["crack", "chip"].map(|sound| sim.id::<SfxDef>(sound));
    let heard = hear(&mut sim, &mut sounds, TAKES - 0.2);
    assert!(
        heard.contains(&crack) && !heard.contains(&chip),
        "each swing sounds as its kind of tool does while nothing is taken yet: {heard:?}"
    );
    assert_eq!(left(&sim, vein), Some(3), "it gave before its time was up");
    let heard = hear(&mut sim, &mut sounds, 0.4);
    assert!(heard.contains(&chip), "an extraction sounds as it pays out");
    assert_eq!(left(&sim, vein), Some(2));
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
    sim.equip(miner, "pickaxe");
    ask(&mut sim, 1, vein);
    assert!(sim.run_until(WALK, |world| world.get::<Working>(miner).is_some()));
    let first = swung(&sim, miner).expect("work begins with a swing");
    assert_eq!(
        first.motion,
        Motion::Move(MoveName::new("mine")),
        "the move of the kind of tool worked with"
    );
    sim.run(TAKES - 0.5);
    let later = swung(&sim, miner).expect("it swings for as long as it works");
    assert!(later.started > first.started, "one swing after another");
    let conn = sim.conn(1);
    sim.send(conn, StopGathering);
    sim.tick();
    assert_eq!(swung(&sim, miner), None, "the swing ends with the work");
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

/// A meadow holding one vein of `fields` a walk away from where players start, worked with a
/// pick for two ore an extraction, in the game `more` adds to.
fn quarry(fields: Value, more: impl FnOnce(Game) -> Game) -> (Sim, Entity) {
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
                "extract": path("sfx", "chip"),
                "ran_out": path("sfx", "crumble"),
            },
        })
    };
    let tool = |kind: &str| {
        json!({ "kind": {
            "type": "equipment",
            "wear": "tool",
            "tool": path("tool_type", kind),
            "stance": path("stance", "swing"),
        } })
    };
    let mut clips = rig_clips();
    clips.extend(["mine", "hew", "pluck"].map(|swing| (swing, Seconds(SWING))));
    let lands = |clip: &str| json!({ "clip": testkit::clip(clip), "cues": [{ "at": APEX.0, "apex": true }] });
    let game = Game::new()
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
        .item("pickaxe", tool("pick"))
        .item("hatchet", tool("axe"))
        .resource("ore", 20)
        .prop("rock", json!({ "type": "none" }))
        .sfx("chip")
        .sfx("crack")
        .sfx("crumble")
        .sfx("ui_refuse")
        .row("resource", "vein", vein);
    let mut sim = Sim::new(more(game).load());
    sim.serve("meadow")
        .accept::<MoveRequest>()
        .accept::<GatherRequest>()
        .accept::<StopGathering>()
        .message::<ToClient<SoundPlayed>>()
        .add((
            movement::move_request,
            movement::file_by_chunk,
            resource::gather_request,
        ))
        .plan(movement::AdvanceWalks::default())
        .add(
            (
                actor::playing::progress,
                resource::gathering,
                resource::work,
            )
                .chain(),
        )
        .plan(stat::SettleHealth::default())
        .plan(actor::SettleActions::default());
    script::spawn_stateless(&mut sim.world);
    let area = sim.id::<AreaDef>("meadow");
    let def = sim.id::<ResourceDef>("vein");
    let at = Pos::<Meters>::new(2.0, 5.0) + Offset::new(AWAY, 0.0);
    let vein = resource::spawn(&mut sim.world, def, at, area);
    (sim, vein)
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
