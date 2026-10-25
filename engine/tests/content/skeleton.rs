use bevy_math::Quat;
use engine::core::content::LoadReport;
use engine::core::time::{PlaybackRate, Seconds};
use engine::systems::actor::{Action, ActorModel, ModelDef, Motion};
use serde_json::{Value, json};
use testkit::{APEX, BODY, CLIP, Declared, Game, LIBRARY, library_glb, rig, rig_clips, skin_glb};

use crate::support::game;

#[test]
fn a_model_that_does_not_fit_its_skeleton_is_refused_naming_the_joint() {
    struct Case {
        case: &'static str,
        change: fn(&mut engine::core::content::gltf::NodeTree),
        found: &'static str,
        expected: &'static str,
    }
    let cases = [
        Case {
            case: "a hand's joint missing",
            change: |tree| {
                tree.nodes.pop();
            },
            found: "model.glb: no joint hand.l",
            expected: "hand \"left\" of body.skeleton needs it",
        },
        Case {
            case: "a joint under another parent",
            change: |tree| tree.nodes[3].parent = Some(2),
            found: "model.glb: joint hand.l has parent hand.r",
            expected: "body.skeleton expects body",
        },
        Case {
            case: "a joint resting turned",
            change: |tree| tree.nodes[3].rest.rotation = Quat::from_rotation_z(10f32.to_radians()),
            found: "model.glb: joint hand.l rests 10.0° and 0.0 mm from body.skeleton",
            expected: "at most 2° and 2 mm, so library clips play true",
        },
        Case {
            case: "a joint resting moved",
            change: |tree| tree.nodes[2].rest.translation.x += 0.01,
            found: "model.glb: joint hand.r rests 0.0° and 10.0 mm from body.skeleton",
            expected: "at most 2° and 2 mm, so library clips play true",
        },
    ];
    for case in cases {
        let mut tree = rig();
        (case.change)(&mut tree);
        let report = game()
            .part("model", "hero", "model.glb", skin_glb(&tree, &[]))
            .try_load()
            .err()
            .unwrap_or_else(|| panic!("{}: refused", case.case));
        assert_eq!(
            problems_in(&report, "hero.model/config.json"),
            [(case.found.to_owned(), case.expected.to_owned())],
            "{}",
            case.case
        );
    }

    let mut extra = rig();
    extra.nodes.push(engine::core::content::gltf::TreeNode {
        name: "tail".to_owned(),
        parent: Some(1),
        ..rig().nodes[2].clone()
    });
    game()
        .part("model", "hero", "model.glb", skin_glb(&extra, &[]))
        .try_load()
        .unwrap_or_else(|report| panic!("a model with a joint the rig lacks loads: {report}"));
}

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
            change: |skeleton| skeleton["base"]["clips"]["walk"] = json!("stroll"),
            found: "base set names stroll: no such clip in clips/*.glb or any model that wears this skeleton",
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
fn a_models_own_clip_plays_in_place_of_the_librarys_cues_and_all() {
    let content = game()
        .part(
            "model",
            "hero",
            "model.glb",
            skin_glb(&rig(), &[("attack", Seconds(0.6))]),
        )
        .edit("model", "hero", |hero| {
            hero["overrides"] = json!({ "attack": [{ "at": 500.0, "apex": true }] })
        })
        .model("brute", 2.0, json!([]))
        .load();
    let timing = |name: &str| {
        let model = content.declared::<ModelDef>(name).expect("declared");
        ActorModel::get(&content, model).timing(
            None,
            &Motion::Action(Action::Attack),
            PlaybackRate(1.0),
        )
    };

    let own = timing("hero");
    assert_eq!((own.duration, own.apex), (Seconds(0.6), Seconds(0.5)));
    let library = timing("brute");
    assert_eq!(
        (library.duration, library.apex),
        (CLIP.seconds(), APEX.seconds())
    );

    let report = game()
        .part(
            "model",
            "hero",
            "model.glb",
            skin_glb(&rig(), &[("attack", Seconds(0.6))]),
        )
        .try_load()
        .err()
        .expect("an own attack without its apex is refused");
    assert_eq!(
        problems_in(&report, "hero.model/config.json"),
        [(
            "attack plays as an attack but has 0 apex cues".to_owned(),
            "exactly one, where the hit lands".to_owned()
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
