use editor::views::model::{self, import};
use engine::core::geometry::Capsule;
use engine::core::time::{Millis, Seconds};
use engine::core::units::Meters;
use engine::systems::actor::model::{MODEL_GLB, ModelDef};
use engine::systems::actor::skeleton::{ClipName, Cue};
use serde_json::json;
use testkit::{BODY, path};

use crate::support;

const HERO: &str = "hero";

#[test]
fn an_imported_skin_must_fit_the_skeleton_and_keeps_the_override_cues_that_still_fit() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game()
        .model(HERO, 1.8, json!([]))
        .part(
            "model",
            HERO,
            &MODEL_GLB.named("", "glb"),
            testkit::skin_glb(&testkit::rig(), &[("attack", Seconds(0.4))]),
        )
        .edit("model", HERO, |hero| {
            hero["overrides"] = json!({ "attack": [cue(100.0, true), cue(300.0, false)] })
        });
    let mut editor = support::open(game, &scratch);
    let key = path("model", HERO);
    let glb = format!("{key}/{}", MODEL_GLB.named("", "glb"));
    let before = support::settled(&mut editor).files.get(&glb).cloned();

    let mut handless = testkit::rig();
    handless.nodes.retain(|node| node.name != "hand.l");
    let refused = import::draft(
        &editor,
        &key,
        testkit::skin_glb(&handless, &[]),
        "handless.glb".to_owned(),
    )
    .expect("a glTF binary");
    let refusals = refused.refusals().join("\n");
    assert!(refusals.contains("no joint hand.l"), "{refusals}");
    assert!(
        refusals.contains(&format!("hand \"left\" of {BODY}.skeleton needs it")),
        "{refusals}"
    );
    assert!(!import::apply(&mut editor, &key, &refused));
    assert_eq!(
        support::settled(&mut editor).files.get(&glb).cloned(),
        before
    );

    let fitting = import::draft(
        &editor,
        &key,
        testkit::skin_glb(
            &testkit::rig(),
            &[("attack", Seconds(0.2)), ("dance", Seconds(1.0))],
        ),
        "fitting.glb".to_owned(),
    )
    .expect("a glTF binary");
    assert!(fitting.refusals().is_empty(), "{:?}", fitting.refusals());
    assert_eq!(
        fitting.carried,
        vec![
            (
                "attack".to_owned(),
                format!("would override clips/{}.glb's attack", testkit::LIBRARY)
            ),
            (
                "dance".to_owned(),
                "no set names it · never plays".to_owned()
            ),
        ]
    );
    assert!(import::apply(&mut editor, &key, &fitting));
    assert_eq!(
        overrides(&mut editor, &key),
        vec![(ClipName::new("attack"), vec![cue(100.0, true)])],
        "the step past the shorter attack is gone"
    );

    editor.undo();
    assert_eq!(
        support::settled(&mut editor).files.get(&glb).cloned(),
        before
    );
}

#[test]
fn override_cues_and_the_hitbox_land_in_the_model() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game().model(HERO, 1.8, json!([])).part(
        "model",
        HERO,
        &MODEL_GLB.named("", "glb"),
        testkit::skin_glb(&testkit::rig(), &[("walk", Seconds(0.4))]),
    );
    let mut editor = support::open(game, &scratch);
    let key = path("model", HERO);
    let walk = ClipName::new("walk");

    assert!(model::set_override_cues(
        &mut editor,
        &key,
        &walk,
        vec![cue(200.0, false), cue(120.0, false)],
    ));
    assert_eq!(
        overrides(&mut editor, &key),
        vec![(walk.clone(), vec![cue(120.0, false), cue(200.0, false)])],
        "cues are kept in order of time"
    );
    assert!(model::set_override_cues(
        &mut editor,
        &key,
        &walk,
        Vec::new()
    ));
    assert_eq!(overrides(&mut editor, &key), Vec::new());

    let hitbox = Capsule {
        radius: Meters(0.4),
        height: Meters(2.1),
    };
    assert!(model::set_hitbox(&mut editor, &key, hitbox));
    support::settled(&mut editor);
    assert_eq!(model_of(&editor, &key).hitbox, hitbox);
}

fn cue(at: f32, apex: bool) -> Cue {
    Cue {
        at: Millis(at),
        step: !apex,
        apex,
        sound: None,
    }
}

fn overrides(editor: &mut editor::app::Editor, key: &str) -> Vec<(ClipName, Vec<Cue>)> {
    support::settled(editor);
    model_of(editor, key).overrides.into_iter().collect()
}

fn model_of(editor: &editor::app::Editor, key: &str) -> ModelDef {
    editor
        .read::<ModelDef>(key)
        .expect("the model is in the project")
        .expect("the model reads")
        .as_ref()
        .clone()
}
