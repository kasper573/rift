use editor::views::model::clips::ClipSlot;
use editor::views::model::{self, import};
use engine::core::geometry::Capsule;
use engine::core::time::{Millis, Seconds};
use engine::core::units::Meters;
use engine::systems::actor::model::{Cue, ModelDef};
use engine::systems::actor::{Action, Pose};
use serde_json::json;
use testkit::path;

use crate::support;

const HERO: &str = "hero";

fn model_of(editor: &editor::app::Editor, key: &str) -> ModelDef {
    editor
        .read::<ModelDef>(key)
        .expect("the model is in the project")
        .expect("the model reads")
        .as_ref()
        .clone()
}

#[test]
fn an_imported_rig_plays_the_clips_named_after_the_actions_and_keeps_the_cues_that_fit() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game().model(HERO, 1.8, json!([])).busts(HERO);
    let mut editor = support::open(game, &scratch);
    let key = path("model", HERO);
    assert!(model::set_cues(
        &mut editor,
        &key,
        ClipSlot::Action(Action::Walk),
        vec![
            Cue {
                at: Millis(50.0),
                step: true,
                apex: false,
                sound: None
            },
            Cue {
                at: Millis(250.0),
                step: true,
                apex: false,
                sound: None
            },
        ],
    ));
    support::settled(&mut editor);

    let rig = testkit::glb(&[
        ("Idle_Loop", Seconds(1.0)),
        ("Walking", Seconds(0.2)),
        ("Running", Seconds(0.6)),
        ("Attack01", Seconds(0.15)),
        ("Dying", Seconds(1.2)),
        ("Talking", Seconds(0.8)),
        ("Sitting", Seconds(2.0)),
        ("Dance", Seconds(3.0)),
    ]);
    let draft = import::draft(&editor, &key, rig, "hero.glb".to_owned()).expect("a glTF binary");
    let chosen = |action: Action| draft.actions[&action].as_deref();
    assert_eq!(chosen(Action::Idle), Some("Idle_Loop"));
    assert_eq!(chosen(Action::Walk), Some("Walking"));
    assert_eq!(chosen(Action::Run), Some("Running"));
    assert_eq!(chosen(Action::Attack), Some("Attack01"));
    assert_eq!(chosen(Action::Dead), Some("Dying"));
    assert_eq!(chosen(Action::Talk), Some("Talking"));
    assert_eq!(draft.poses[&Pose::Sit].as_deref(), Some("Sitting"));
    assert_eq!(draft.unmatched(), vec!["Dance"]);
    assert!(draft.refusals().is_empty(), "{:?}", draft.refusals());

    assert!(import::apply(&mut editor, &key, &draft));
    support::settled(&mut editor);
    let model = model_of(&editor, &key);
    assert_eq!(model.clips[&Action::Walk].name, "Walking");
    assert_eq!(
        model.clips[&Action::Walk]
            .cues
            .iter()
            .map(|cue| cue.at)
            .collect::<Vec<Millis>>(),
        vec![Millis(50.0)],
        "the step past the shorter walk is gone"
    );
    let apex: Vec<&Cue> = model.clips[&Action::Attack]
        .cues
        .iter()
        .filter(|cue| cue.apex)
        .collect();
    assert_eq!(apex.len(), 1, "an attack keeps exactly one apex");
    assert!(
        apex[0].at <= Millis(150.0),
        "the apex lands within the shorter attack: {:?}",
        apex[0].at
    );
    assert_eq!(model.poses[&Pose::Sit].name, "Sitting");
    assert_eq!(
        editor::views::model::busts::faces(&model).len(),
        6,
        "the busts are untouched"
    );

    editor.undo();
    support::settled(&mut editor);
    assert_eq!(model_of(&editor, &key).clips[&Action::Walk].name, "walk");
}

#[test]
fn cues_a_clip_and_the_hitbox_land_in_the_model() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game().model(HERO, 1.8, json!([]));
    let mut editor = support::open(game, &scratch);
    let key = path("model", HERO);
    let slot = ClipSlot::Action(Action::Attack);
    let cues = vec![
        Cue {
            at: Millis(200.0),
            step: true,
            apex: false,
            sound: None,
        },
        Cue {
            at: Millis(120.0),
            step: false,
            apex: true,
            sound: None,
        },
    ];
    assert!(model::set_cues(&mut editor, &key, slot, cues));
    support::settled(&mut editor);
    let attack = model_of(&editor, &key).clips[&Action::Attack].clone();
    assert_eq!(
        attack
            .cues
            .iter()
            .map(|cue| cue.at)
            .collect::<Vec<Millis>>(),
        vec![Millis(120.0), Millis(200.0)],
        "cues are kept in order of time"
    );

    assert!(model::set_clip(
        &mut editor,
        &key,
        ClipSlot::Pose(Pose::Sit),
        "idle"
    ));
    support::settled(&mut editor);
    assert_eq!(model_of(&editor, &key).poses[&Pose::Sit].name, "idle");
    assert!(model::remove_clip(
        &mut editor,
        &key,
        ClipSlot::Pose(Pose::Sit)
    ));
    support::settled(&mut editor);
    assert!(!model_of(&editor, &key).poses.contains_key(&Pose::Sit));

    let hitbox = Capsule {
        radius: Meters(0.4),
        height: Meters(2.1),
    };
    assert!(model::set_hitbox(&mut editor, &key, hitbox));
    support::settled(&mut editor);
    assert_eq!(model_of(&editor, &key).hitbox, hitbox);
}
