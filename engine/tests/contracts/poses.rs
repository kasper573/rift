use bevy_ecs::schedule::IntoScheduleConfigs;
use engine::core::math::Offset;
use engine::core::units::{Millis, Seconds};
use engine::systems::actor::held::Held;
use engine::systems::actor::playing::Playing;
use engine::systems::actor::skeleton::GestureName;
use engine::systems::actor::{self, Actor, Motion, Pose};
use engine::systems::emote::{self, EmoteDef, EmoteRequest};
use engine::systems::movement::{self, MoveRequest, MoveSeq, position};
use engine::systems::region;
use engine::systems::script;
use serde_json::{Value, json};
use strum::VariantArray;
use testkit::{BODY, Game, Map, Sim, clip, path, rig_clips};

#[test]
fn a_script_sits_a_body_down_and_stands_it_up_through_the_clips_its_skeleton_names() {
    struct Case {
        case: &'static str,
        placed: &'static str,
        pose: &'static str,
        shift: Option<Motion>,
        then: Pose,
    }
    let cases = [
        Case {
            case: "sitting down plays the sit-down clip, then the script goes on",
            placed: "idle",
            pose: "sit",
            shift: Some(Motion::Begin(Pose::Sit)),
            then: Pose::Sit,
        },
        Case {
            case: "standing up plays the stand-up clip, then the script goes on",
            placed: "sit",
            pose: "idle",
            shift: Some(Motion::End(Pose::Sit)),
            then: Pose::Idle,
        },
        Case {
            case: "a pose the skeleton names no clip for taking is taken at once",
            placed: "idle",
            pose: "sleep",
            shift: None,
            then: Pose::Sleep,
        },
    ];
    for case in cases {
        let mut sim = Sim::new(game(case.placed, case.pose).load());
        sim.serve("room")
            .add(actor::playing::progress)
            .plan(script::RunFlows::default());
        script::spawn_stateless(&mut sim.world);
        region::hold_whole(&mut sim.world);
        let guard = sim.npc("guard");
        sim.tick();

        let pose = |sim: &Sim| sim.world.get::<Actor>(guard).map(|actor| actor.pose);
        let armed = |sim: &Sim| {
            sim.world
                .get::<Held>(guard)
                .is_some_and(|held| !held.hands.is_empty())
        };
        assert_eq!(pose(&sim), Some(case.then), "{}", case.case);
        let playing = sim.world.get::<Playing>(guard).cloned();
        assert_eq!(
            playing.as_ref().map(|playing| playing.motion.clone()),
            case.shift,
            "{}",
            case.case
        );
        if let Some(playing) = playing {
            assert_eq!(
                Millis((playing.ends.0 - playing.started.0) as f32),
                SHIFT,
                "{}: the shift lasts as its clip does",
                case.case
            );
            assert!(!armed(&sim), "{}: the script waits for the clip", case.case);
            assert!(
                sim.run_until(1.0, |world| world.get::<Playing>(guard).is_none()),
                "{}",
                case.case
            );
            sim.tick();
        }
        assert!(armed(&sim), "{}: the script goes on", case.case);
        assert_eq!(pose(&sim), Some(case.then), "{}", case.case);
    }
}

#[test]
fn a_player_picks_an_emote_and_their_character_makes_the_gesture_or_holds_the_pose_until_it_does_anything_else()
 {
    let content = game("idle", "idle")
        .asset("icons/emote.png", testkit::png(1, 1))
        .row("emote", "wave", emote(json!({ "gesture": "wave" })))
        .row("emote", "sit", emote(json!({ "pose": "sit" })))
        .row("emote", "nap", emote(json!({ "pose": "sleep" })))
        .sfx("ui_refuse")
        .load();
    let mut sim = Sim::new(content);
    sim.serve("room")
        .accept::<EmoteRequest>()
        .accept::<MoveRequest>()
        .add((
            movement::move_request,
            (emote::requests, emote::rise).chain(),
            actor::playing::progress,
        ))
        .plan(movement::AdvanceWalks::default());
    region::hold_whole(&mut sim.world);
    let player = sim.join(1);
    let conn = sim.conn(1);
    let ask = |sim: &mut Sim, emote: &str| {
        let emote = sim.id::<EmoteDef>(emote);
        sim.send(conn, EmoteRequest { emote });
        sim.tick();
    };
    let pose = |sim: &Sim| sim.world.get::<Actor>(player).map(|actor| actor.pose);
    let shown = |sim: &Sim| {
        sim.world
            .get::<Playing>(player)
            .map(|playing| playing.motion.clone())
    };

    ask(&mut sim, "wave");
    assert_eq!(
        shown(&sim),
        Some(Motion::Gesture(GestureName("wave".to_owned()))),
        "an emote that is a gesture is made at once"
    );
    assert!(
        sim.run_until(1.0, |world| world.get::<Playing>(player).is_none()),
        "and made once"
    );

    ask(&mut sim, "sit");
    assert_eq!(
        pose(&sim),
        Some(Pose::Sit),
        "an emote that is a pose is taken"
    );
    assert_eq!(shown(&sim), Some(Motion::Begin(Pose::Sit)));
    sim.run(2.0);
    assert_eq!(
        pose(&sim),
        Some(Pose::Sit),
        "and held while nothing else is done"
    );

    ask(&mut sim, "wave");
    assert_eq!(pose(&sim), Some(Pose::Idle), "a gesture is made standing");
    sim.run(1.0);

    ask(&mut sim, "sit");
    ask(&mut sim, "sit");
    assert_eq!(
        pose(&sim),
        Some(Pose::Idle),
        "a pose picked while held is left"
    );
    assert_eq!(shown(&sim), Some(Motion::End(Pose::Sit)));
    sim.run(1.0);

    ask(&mut sim, "sit");
    sim.run(1.0);
    let ahead = position(&sim.world, player).expect("somewhere") + Offset::new(2.0, 0.0);
    sim.send(
        conn,
        MoveRequest {
            pos: ahead,
            seq: MoveSeq(1),
        },
    );
    sim.tick();
    assert_eq!(pose(&sim), Some(Pose::Idle), "walking off leaves the pose");

    sim.run(3.0);
    ask(&mut sim, "nap");
    assert_eq!(
        pose(&sim),
        Some(Pose::Idle),
        "a pose the body has no clip for is not taken"
    );
    assert!(
        sim.errors(1).iter().any(|error| error.contains("can't")),
        "and its player is told so"
    );
}

fn emote(does: Value) -> Value {
    let mut emote = json!({ "name": "Emote", "icon": "icons/emote.png" });
    emote
        .as_object_mut()
        .expect("an object")
        .extend(does.as_object().expect("an object").clone());
    emote
}

/// How long sitting down and standing up take.
const SHIFT: Millis = Millis(400.0);

/// A room where a guard, placed in `placed`, takes `pose` as it wakes, waiting for it before it
/// arms itself; every body's skeleton sits down and stands up through clips of their own.
fn game(placed: &str, pose: &str) -> Game {
    let clips: Vec<(&str, Seconds)> = rig_clips()
        .into_iter()
        .chain([
            ("sit_down", SHIFT.seconds()),
            ("sit", SHIFT.seconds()),
            ("stand_up", SHIFT.seconds()),
            ("wave", SHIFT.seconds()),
        ])
        .collect();
    let base: serde_json::Map<String, Value> = actor::Action::VARIANTS
        .iter()
        .map(|action| (action.as_ref().to_owned(), json!(clip(action.as_ref()))))
        .collect();
    Game::new()
        .area(
            "room",
            &Map::open(9, 9),
            json!([{ "type": "resident", "npc": "guard.npc.json", "at": { "x": 6.0, "y": 6.0 }, "pose": placed }]),
        )
        .hero("room", 2.0, 2.0)
        .skeleton_of(
            BODY,
            &clips,
            json!({
                "base": {
                    "clips": base,
                    "poses": { "sit": clip("sit") },
                    "begins": { "sit": clip("sit_down") },
                    "ends": { "sit": clip("stand_up") },
                    "gestures": { "wave": clip("wave") },
                },
            }),
        )
        .sword()
        .npc(
            "guard",
            30.0,
            json!([{ "type": "script", "script": path("script", "guard_start") }]),
        )
        .script(
            "guard_start",
            "npc",
            json!({
                "started": { "type": "on", "event": "started", "next": "pose" },
                "pose": { "type": "set_pose", "pose": pose, "wait": true, "next": "arm" },
                "arm": { "type": "set_held", "items": [path("item", "sword")] },
            }),
        )
}
