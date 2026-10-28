use engine::core::time::{Millis, Seconds};
use engine::systems::actor::playing::Playing;
use engine::systems::actor::skeleton::GestureName;
use engine::systems::actor::{self, Appearance, ModelDef, Motion};
use engine::systems::{npc, script};
use serde_json::{Value, json};
use testkit::{BODY, Game, Map, Sim, clip, path, rig_clips};

#[test]
fn a_script_makes_an_npc_gesture_where_its_body_names_the_gesture() {
    struct Case {
        case: &'static str,
        nodes: Value,
        waves: bool,
    }
    let cases = [
        Case {
            case: "a body whose sets name the wave waves, and the script waits for it",
            nodes: json!({
                "started": { "type": "on", "event": "started", "next": "wave" },
                "wave": { "type": "animate", "who": { "type": "owner" }, "gesture": "wave", "wait": "end", "next": "dress" },
                "dress": { "type": "set_appearance", "appearance": { "model": path("model", "giant") } },
            }),
            waves: true,
        },
        Case {
            case: "a body whose sets name no wave makes none, and the script goes on",
            nodes: json!({
                "started": { "type": "on", "event": "started", "next": "blob" },
                "blob": { "type": "set_appearance", "appearance": { "model": path("model", "slime") }, "next": "wave" },
                "wave": { "type": "animate", "who": { "type": "owner" }, "gesture": "wave", "wait": "end", "next": "dress" },
                "dress": { "type": "set_appearance", "appearance": { "model": path("model", "giant") } },
            }),
            waves: false,
        },
    ];
    for case in cases {
        let mut sim = Sim::new(game(case.nodes).load());
        sim.serve("room")
            .add((script::run_tick, actor::playing::progress));
        script::spawn_stateless(&mut sim.world);
        npc::spawn_all(&mut sim.world);
        let guard = sim.npc("guard");
        let giant = sim.id::<ModelDef>("giant");
        sim.tick();

        let dressed =
            |sim: &Sim| sim.world.get::<Appearance>(guard) == Some(&Appearance::of(giant));
        let playing = sim.world.get::<Playing>(guard).cloned();
        assert_eq!(
            playing.as_ref().map(|playing| playing.motion.clone()),
            case.waves
                .then(|| Motion::Gesture(GestureName::new("wave"))),
            "{}",
            case.case
        );
        if let Some(playing) = playing {
            assert_eq!(
                Millis((playing.ends.0 - playing.started.0) as f32),
                WAVE,
                "{}: the wave lasts as its clip does",
                case.case
            );
            assert!(
                !dressed(&sim),
                "{}: the script waits for the wave",
                case.case
            );
            assert!(
                sim.run_until(1.0, |world| world.get::<Playing>(guard).is_none()),
                "{}",
                case.case
            );
            sim.tick();
        }
        assert!(dressed(&sim), "{}: the script goes on", case.case);
    }
}

#[test]
fn a_script_asking_for_a_gesture_no_skeleton_declares_is_refused() {
    let report = game(json!({
        "started": { "type": "on", "event": "started", "next": "bow" },
        "bow": { "type": "animate", "who": { "type": "owner" }, "gesture": "bow" },
    }))
    .try_load()
    .err()
    .expect("refused");
    let rules: Vec<&str> = report
        .errors
        .iter()
        .filter(|error| error.file == path("script", "guard_start"))
        .map(|error| error.problem.rule)
        .collect();
    assert_eq!(rules, ["undeclared gesture"], "{report}");
}

/// How long a wave takes.
const WAVE: Millis = Millis(400.0);

/// A room where a guard runs `nodes` as it wakes; every body of the plain skeleton waves, a
/// slime's body of another makes no gestures, and a giant's body to dress the guard in.
fn game(nodes: Value) -> Game {
    let clips: Vec<(&str, Seconds)> = rig_clips()
        .into_iter()
        .chain([("wave", WAVE.seconds())])
        .collect();
    let base: serde_json::Map<String, Value> = <actor::Action as strum::VariantArray>::VARIANTS
        .iter()
        .map(|action| (action.as_ref().to_owned(), json!(clip(action.as_ref()))))
        .collect();
    Game::new()
        .area(
            "room",
            &Map::open(9, 9),
            json!([{ "type": "resident", "npc": "guard.npc.json", "at": { "x": 6.0, "y": 6.0 } }]),
        )
        .hero("room", 2.0, 2.0)
        .skeleton_of(
            BODY,
            &clips,
            json!({ "base": { "clips": base, "gestures": { "wave": clip("wave") } } }),
        )
        .model("giant", 4.0, json!([]))
        .skeleton_of("blob", &rig_clips(), json!({ "hands": [] }))
        .model_on("slime", "blob", 1.0, json!([]))
        .npc(
            "guard",
            30.0,
            json!([{ "type": "script", "script": path("script", "guard_start") }]),
        )
        .script("guard_start", "npc", nodes)
}
