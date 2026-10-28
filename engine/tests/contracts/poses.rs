use engine::core::time::{Millis, Seconds};
use engine::systems::actor::held::Held;
use engine::systems::actor::playing::Playing;
use engine::systems::actor::{self, Actor, Motion, Pose};
use engine::systems::{npc, script};
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
            .add((script::run_tick, actor::playing::progress));
        script::spawn_stateless(&mut sim.world);
        npc::spawn_all(&mut sim.world);
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
        ])
        .collect();
    let mut base: serde_json::Map<String, Value> = actor::Action::VARIANTS
        .iter()
        .map(|action| (action.as_ref().to_owned(), json!(clip(action.as_ref()))))
        .collect();
    base.remove("sprint");
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
