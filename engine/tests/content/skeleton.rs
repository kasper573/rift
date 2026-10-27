use engine::core::content::LoadReport;
use engine::core::time::Seconds;
use serde_json::{Value, json};
use testkit::{BODY, CLIP, Game, LIBRARY, library_glb, rig, rig_clips};

use crate::support::game;

#[test]
fn a_skeleton_is_held_to_the_clips_its_libraries_carry() {
    struct Case {
        case: &'static str,
        change: fn(&mut Value),
        found: &'static str,
    }
    let cases = [
        Case {
            case: "a base set missing an engine action",
            change: |skeleton| {
                skeleton["base"]["clips"]
                    .as_object_mut()
                    .expect("clips")
                    .remove("run");
            },
            found: "base set has no run clip",
        },
        Case {
            case: "a set naming a clip no library carries",
            change: |skeleton| skeleton["base"]["clips"]["talk"] = json!("stroll"),
            found: "base set names stroll: no such clip in clips/*.glb",
        },
        Case {
            case: "a gait whose clip covers no stated ground",
            change: |skeleton| skeleton["base"]["clips"]["run"] = json!("talk"),
            found: "talk has no pace",
        },
        Case {
            case: "a cell of no clips",
            change: |skeleton| skeleton["base"]["clips"]["talk"] = json!({ "clips": [] }),
            found: "an empty list",
        },
        Case {
            case: "a gait taking turns with a clip that covers no stated ground",
            change: |skeleton| {
                skeleton["base"]["clips"]["run"] = json!({ "clips": ["run", "talk"] })
            },
            found: "talk has no pace",
        },
        Case {
            case: "a pace covering no ground",
            change: |skeleton| skeleton["paces"]["walk"] = json!(0.0),
            found: "0 m/s",
        },
        Case {
            case: "an attack landing nowhere",
            change: |skeleton| skeleton["cues"]["attack"] = json!([]),
            found: "attack plays as an attack but has 0 apex cues",
        },
        Case {
            case: "an attack landing twice",
            change: |skeleton| {
                skeleton["cues"]["attack"] =
                    json!([{ "at": 0, "apex": true }, { "at": 50, "apex": true }])
            },
            found: "attack plays as an attack but has 2 apex cues",
        },
        Case {
            case: "a move landing nowhere",
            change: |skeleton| skeleton["base"]["moves"] = json!({ "thrust": "talk" }),
            found: "talk plays as an attack but has 0 apex cues",
        },
        Case {
            case: "a cue after the clip ends",
            change: |skeleton| skeleton["cues"]["attack"][0]["at"] = json!(CLIP.0 * 2.0),
            found: "600 ms into a clip of 300 ms",
        },
        Case {
            case: "a pose the idle action already plays",
            change: |skeleton| skeleton["base"]["poses"] = json!({ "idle": "idle" }),
            found: "a clip for the idle pose",
        },
        Case {
            case: "a clip kept to a group the skeleton lacks",
            change: |skeleton| {
                skeleton["base"]["clips"]["talk"] = json!({ "clip": "talk", "only": "upper" })
            },
            found: "upper",
        },
        Case {
            case: "a hand clasping as a clip no library carries",
            change: |skeleton| {
                skeleton["hands"][0]["clasp"] = json!({ "clip": "fist", "fingers": ["finger.r"] })
            },
            found: "hand \"right\" clasps as fist: no such clip in clips/*.glb",
        },
        Case {
            case: "a clasp naming a finger the rig lacks",
            change: |skeleton| {
                skeleton["hands"][0]["clasp"] = json!({ "clip": "idle", "fingers": ["thumb.r"] })
            },
            found: "thumb.r is not in rig.glb",
        },
        Case {
            case: "a clasp whose clip turns none of its fingers",
            change: |skeleton| {
                skeleton["hands"][0]["clasp"] = json!({ "clip": "idle", "fingers": ["finger.r"] })
            },
            found: "idle turns none of hand \"right\"'s fingers",
        },
    ];
    for case in cases {
        let report = game()
            .edit("skeleton", BODY, case.change)
            .try_load()
            .err()
            .unwrap_or_else(|| panic!("{}: refused", case.case));
        let found: Vec<String> = problems_in(&report, "body.skeleton/config.json")
            .into_iter()
            .map(|(found, _)| found)
            .collect();
        assert_eq!(found, [case.found], "{}", case.case);
    }

    let mut short = rig_clips();
    short[0].1 = Seconds(0.05);
    let report = game()
        .part(
            "skeleton",
            BODY,
            &format!("clips/{LIBRARY}.glb"),
            library_glb(&rig(), &short),
        )
        .try_load()
        .err()
        .expect("refused");
    assert_eq!(
        problems_in(&report, &format!("body.skeleton/clips/{LIBRARY}.glb")),
        [(
            "idle plays 50 ms".to_owned(),
            "100 ms to 60000 ms".to_owned()
        )]
    );
}

#[test]
fn a_clip_name_carried_by_two_libraries_is_refused() {
    let report = game()
        .part(
            "skeleton",
            BODY,
            "clips/extra.glb",
            library_glb(&rig(), &[("walk", Seconds(1.0))]),
        )
        .try_load()
        .err()
        .expect("refused");
    assert_eq!(
        problems_in(&report, "body.skeleton/clips/moves.glb"),
        [(
            "clip walk is in clips/extra.glb and clips/moves.glb".to_owned(),
            "one library per name".to_owned()
        )]
    );
}

#[test]
fn a_skill_asking_for_a_move_no_skeleton_declares_is_refused() {
    let volley = |game: Game| {
        game.row(
            "skill",
            "volley",
            json!({
                "name": "Volley",
                "icon": "icons/volley.png",
                "target": "single_enemy",
                "levels": [{ "cost": 1 }],
                "modules": [{ "type": "script", "script": testkit::path("script", "volley") }],
            }),
        )
        .asset("icons/volley.png", testkit::png(1, 1))
        .script(
            "volley",
            "skill",
            json!({
                "used": { "type": "on", "event": "used", "next": "draw" },
                "draw": { "type": "animate", "move": "shoot", "wait": "apex" },
            }),
        )
    };
    let report = volley(game()).try_load().err().expect("refused");
    let refused: Vec<&str> = report
        .errors
        .iter()
        .filter(|error| error.file == "volley.script.json")
        .map(|error| error.problem.rule)
        .collect();
    assert_eq!(refused, ["undeclared move"]);

    volley(game().edit("skeleton", BODY, |skeleton| {
        skeleton["base"]["moves"] = json!({ "shoot": "attack" })
    }))
    .load();
}

fn problems_in(report: &LoadReport, file: &str) -> Vec<(String, String)> {
    report
        .errors
        .iter()
        .filter(|error| error.file == file)
        .map(|error| (error.problem.found.clone(), error.problem.expected.clone()))
        .collect()
}
