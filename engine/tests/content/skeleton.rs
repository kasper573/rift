use bevy_math::{Quat, Vec3};
use engine::core::content::LoadReport;
use engine::core::content::gltf::{NodeTree, Rest, TreeNode};
use engine::core::time::Seconds;
use engine::systems::actor::model::Skeletons;
use serde_json::{Value, json};
use testkit::{
    BODY, CLIP, Declared, Game, clip, library_glb, rig, rig_clips, rig_file, rig_glb, skin_glb,
};

use crate::support::game;

const SKELETON: &str = "body.skeleton.json";

#[test]
fn a_skeleton_is_held_to_the_clips_it_names() {
    struct Case {
        case: &'static str,
        change: fn(&mut Value),
        found: String,
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
            found: "base set has no run clip".to_owned(),
        },
        Case {
            case: "a set naming a file the folder lacks",
            change: |skeleton| skeleton["base"]["clips"]["talk"] = json!(clip("stroll")),
            found: format!("{:?}", clip("stroll")),
        },
        Case {
            case: "a gait whose clip covers no stated ground",
            change: |skeleton| skeleton["base"]["clips"]["run"] = json!(clip("talk")),
            found: format!("{} has no pace", clip("talk")),
        },
        Case {
            case: "a cell of no clips",
            change: |skeleton| skeleton["base"]["clips"]["talk"] = json!({ "clips": [] }),
            found: "an empty list".to_owned(),
        },
        Case {
            case: "a gait taking turns with a clip that covers no stated ground",
            change: |skeleton| {
                skeleton["base"]["clips"]["run"] = json!({ "clips": [clip("run"), clip("talk")] })
            },
            found: format!("{} has no pace", clip("talk")),
        },
        Case {
            case: "a pace covering no ground",
            change: |skeleton| skeleton["paces"][0]["pace"] = json!(0.0),
            found: "0 m/s".to_owned(),
        },
        Case {
            case: "an attack landing nowhere",
            change: |skeleton| skeleton["cues"][0]["cues"] = json!([]),
            found: format!("{} plays as an attack but has 0 apex cues", clip("attack")),
        },
        Case {
            case: "an attack landing twice",
            change: |skeleton| {
                skeleton["cues"][0]["cues"] =
                    json!([{ "at": 0, "apex": true }, { "at": 50, "apex": true }])
            },
            found: format!("{} plays as an attack but has 2 apex cues", clip("attack")),
        },
        Case {
            case: "a move landing nowhere",
            change: |skeleton| skeleton["base"]["moves"] = json!({ "thrust": clip("talk") }),
            found: format!("{} plays as an attack but has 0 apex cues", clip("talk")),
        },
        Case {
            case: "a cue after the clip ends",
            change: |skeleton| skeleton["cues"][0]["cues"][0]["at"] = json!(CLIP.0 * 2.0),
            found: "600 ms into a clip of 300 ms".to_owned(),
        },
        Case {
            case: "a pose the idle action already plays",
            change: |skeleton| skeleton["base"]["poses"] = json!({ "idle": clip("idle") }),
            found: "a clip for the idle pose".to_owned(),
        },
        Case {
            case: "a clip kept to a group the skeleton lacks",
            change: |skeleton| {
                skeleton["base"]["clips"]["talk"] = json!({ "clip": clip("talk"), "only": "upper" })
            },
            found: "upper".to_owned(),
        },
        Case {
            case: "a clasp naming a finger the rig lacks",
            change: |skeleton| {
                skeleton["hands"][0]["clasp"] =
                    json!({ "clip": clip("idle"), "fingers": ["thumb.r"] })
            },
            found: "thumb.r is not a joint of our rig".to_owned(),
        },
        Case {
            case: "a clasp whose clip turns none of its fingers",
            change: |skeleton| {
                skeleton["hands"][0]["clasp"] =
                    json!({ "clip": clip("idle"), "fingers": ["finger.r"] })
            },
            found: format!("{} turns none of hand \"right\"'s fingers", clip("idle")),
        },
        Case {
            case: "a take its file lacks",
            change: |skeleton| {
                skeleton["base"]["clips"]["talk"] = json!({ "file": clip("talk"), "take": "chat" })
            },
            found: format!("{} holds no animation named chat", clip("talk")),
        },
    ];
    for case in cases {
        let report = game()
            .edit("skeleton", BODY, case.change)
            .try_load()
            .err()
            .unwrap_or_else(|| panic!("{}: loaded", case.case));
        let found: Vec<String> = problems_in(&report, SKELETON)
            .into_iter()
            .map(|(found, _)| found)
            .collect();
        assert_eq!(found, [case.found], "{}", case.case);
    }

    let mut short = rig_clips();
    short[0].1 = Seconds(0.05);
    let report = game()
        .asset(&clip("idle"), library_glb(&rig(), &short[..1]))
        .try_load()
        .err()
        .expect("refused");
    assert_eq!(
        problems_in(&report, SKELETON),
        [(
            format!("{} plays 50 ms", clip("idle")),
            "100 ms to 60000 ms".to_owned()
        )]
    );
}

#[test]
fn a_file_of_several_clips_plays_the_take_a_cell_names() {
    let both = library_glb(&rig(), &[("chat", Seconds(1.0)), ("shout", Seconds(2.0))]);
    let report = game()
        .asset("clips/talking.glb", both.clone())
        .edit("skeleton", BODY, |skeleton| {
            skeleton["base"]["clips"]["talk"] = json!("clips/talking.glb")
        })
        .try_load()
        .err()
        .expect("refused");
    assert_eq!(
        problems_in(&report, SKELETON),
        [(
            "clips/talking.glb holds 2 animations".to_owned(),
            "the take named, one of chat, shout".to_owned()
        )]
    );

    let content = game()
        .asset("clips/talking.glb", both)
        .edit("skeleton", BODY, |skeleton| {
            skeleton["base"]["clips"]["talk"] =
                json!({ "file": "clips/talking.glb", "take": "shout" })
        })
        .load();
    let skeleton = body(&content);
    let talk = skeleton
        .clips
        .iter()
        .find(|played| played.clip.file().as_str() == "clips/talking.glb")
        .expect("the talk clip plays");
    assert_eq!(talk.length, Seconds(2.0));
}

#[test]
fn our_rig_is_every_joint_its_files_agree_on() {
    let cape = || {
        tree(&[
            ("cape", None, [0.0, 1.0, 0.0]),
            ("cape.tip", Some(0), [0.0, -0.4, -0.1]),
        ])
    };
    let rigged = |files: Value, hangs: Value| {
        move |skeleton: &mut Value| {
            skeleton["rig"] = json!({ "files": files, "hangs": hangs });
        }
    };

    let mut moved = rig();
    moved.nodes[1].rest.translation += Vec3::new(0.0, 0.01, 0.0);
    let report = game()
        .asset("rigs/moved.glb", rig_glb(&moved))
        .edit(
            "skeleton",
            BODY,
            rigged(json!([rig_file(BODY), "rigs/moved.glb"]), json!({})),
        )
        .try_load()
        .err()
        .expect("refused");
    assert_eq!(
        problems_in(&report, SKELETON),
        [(
            format!(
                "body rests 0.0° and 10.0 mm apart in \"rigs/moved.glb\" and {:?}",
                rig_file(BODY)
            ),
            "every file placing each joint under the same parent, resting alike within 2° and \
             2 mm"
                .to_owned()
        )]
    );

    let report = game()
        .asset("rigs/cape.glb", skin_glb(&cape(), &[]))
        .edit(
            "skeleton",
            BODY,
            rigged(json!([rig_file(BODY), "rigs/cape.glb"]), json!({})),
        )
        .try_load()
        .err()
        .expect("refused");
    assert_eq!(
        problems_in(&report, SKELETON),
        [(
            "model, cape have no parent".to_owned(),
            "one top joint carrying the body, every other hung on a joint of ours".to_owned()
        )]
    );

    let content = game()
        .asset("rigs/cape.glb", skin_glb(&cape(), &[]))
        .edit(
            "skeleton",
            BODY,
            rigged(
                json!([rig_file(BODY), "rigs/cape.glb"]),
                json!({ "cape": "hand.r" }),
            ),
        )
        .load();
    let rig = &body(&content).rig;
    let worlds = rig.worlds();
    let at = |name: &str| worlds[rig.named(name).expect(name)].w_axis.truncate();
    assert_eq!(rig.parent_name(rig.named("cape").expect("cape")), "hand.r");
    assert!(at("cape").abs_diff_eq(at("hand.r"), 1.0e-5));
    assert!(at("cape.tip").abs_diff_eq(at("hand.r") + Vec3::new(0.0, -0.4, -0.1), 1.0e-5));
}

#[test]
fn a_clip_made_on_a_source_rig_plays_once_the_skeleton_names_his_joints() {
    let his = tree(&[
        ("his.root", None, [0.0, 0.0, 0.0]),
        ("his.hips", Some(0), [0.0, 1.0, 0.0]),
        ("his.hand", Some(1), [-0.3, 0.2, 0.0]),
    ]);
    let wave = library_glb(&his, &[("wave", Seconds(1.0))]);
    let waving = |game: Game| {
        game.asset("clips/wave.glb", wave.clone())
            .edit("skeleton", BODY, |skeleton| {
                skeleton["base"]["gestures"] = json!({ "wave": "clips/wave.glb" })
            })
    };
    let report = waving(game()).try_load().err().expect("refused");
    assert_eq!(
        problems_in(&report, SKELETON),
        [(
            "clips/wave.glb moves his.hips".to_owned(),
            "a clip moving only joints of our rig, or only joints of one source rig".to_owned()
        )]
    );

    let content = waving(game())
        .edit("skeleton", BODY, |skeleton| {
            skeleton["sources"] = json!([{
                "name": "his",
                "joints": ["his.root", "his.hips", "his.hand"],
                "pairs": { "his.hips": "body", "his.hand": "hand.r" },
            }])
        })
        .load();
    let skeleton = body(&content);
    let wave = skeleton
        .clips
        .iter()
        .find(|played| played.clip.file().as_str() == "clips/wave.glb")
        .expect("the wave plays");
    assert_eq!(wave.source, Some(0));
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
        skeleton["base"]["moves"] = json!({ "shoot": clip("attack") })
    }))
    .load();
}

fn body(
    content: &engine::core::content::Content,
) -> &engine::systems::actor::rigging::BuiltSkeleton {
    let id = content.declared(BODY).expect("the body skeleton");
    Skeletons::loaded(content, id).expect("the body skeleton loads")
}

fn tree(joints: &[(&str, Option<usize>, [f32; 3])]) -> NodeTree {
    NodeTree {
        nodes: joints
            .iter()
            .map(|&(name, parent, at)| TreeNode {
                name: name.to_owned(),
                parent,
                rest: Rest {
                    translation: Vec3::from(at),
                    rotation: Quat::IDENTITY,
                    scale: Vec3::ONE,
                },
                mesh: false,
            })
            .collect(),
    }
}

fn problems_in(report: &LoadReport, file: &str) -> Vec<(String, String)> {
    report
        .errors
        .iter()
        .filter(|error| error.file == file)
        .map(|error| (error.problem.found.clone(), error.problem.expected.clone()))
        .collect()
}
