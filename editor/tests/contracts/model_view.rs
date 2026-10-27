use editor::views::model;
use editor::views::model_part::{self, import};
use engine::core::geometry::Capsule;
use engine::core::time::Seconds;
use engine::core::units::Meters;
use engine::systems::actor::ModelDef;
use engine::systems::actor::part::PART_MESH;
use serde_json::json;
use testkit::{BODY, path};

use crate::support;

const HERO: &str = "hero";

#[test]
fn an_imported_mesh_must_fit_the_skeleton_and_carry_no_clips() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game().model(HERO, 1.8, json!([]));
    let mut editor = support::open(game, &scratch);
    let key = path("model_part", HERO);
    let row = editor
        .read::<engine::systems::actor::ModelPartDef>(&key)
        .expect("a project")
        .expect("the part reads");
    model_part::ensure_view(&mut editor, &key, &row);
    let glb = format!("{key}/{}", PART_MESH.named("", "glb"));
    let before = support::settled(&mut editor).files.get(&glb).cloned();

    let mut crossed = testkit::rig();
    crossed.nodes[3].parent = Some(2);
    let refusals = |bytes: Vec<u8>| {
        import::draft(&editor, &key, bytes, "dropped.glb".to_owned())
            .expect("a glTF binary")
            .refusals()
            .join("\n")
    };
    let misfit = refusals(testkit::skin_glb(&crossed, &[]));
    assert!(
        misfit.contains("joint hand.l has parent hand.r"),
        "{misfit}"
    );
    assert!(
        misfit.contains(&format!("{BODY}.skeleton expects body")),
        "{misfit}"
    );
    let clipped = refusals(testkit::skin_glb(
        &testkit::rig(),
        &[("dance", Seconds(1.0))],
    ));
    assert!(clipped.contains("carries clips (dance)"), "{clipped}");

    let refused = import::draft(
        &editor,
        &key,
        testkit::skin_glb(&crossed, &[]),
        "crossed.glb".to_owned(),
    )
    .expect("a glTF binary");
    assert!(!import::apply(&mut editor, &key, &refused));
    assert_eq!(
        support::settled(&mut editor).files.get(&glb).cloned(),
        before
    );

    let mut fewer = testkit::rig();
    fewer.nodes.pop();
    let fitting = import::draft(
        &editor,
        &key,
        testkit::skin_glb(&fewer, &[]),
        "fitting.glb".to_owned(),
    )
    .expect("a glTF binary");
    assert!(fitting.refusals().is_empty(), "{:?}", fitting.refusals());
    assert!(import::apply(&mut editor, &key, &fitting));
    assert_eq!(
        support::settled(&mut editor).files.get(&glb).cloned(),
        Some(fitting.bytes.clone().into())
    );

    editor.undo();
    assert_eq!(
        support::settled(&mut editor).files.get(&glb).cloned(),
        before
    );
}

#[test]
fn the_hitbox_lands_in_the_model() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(support::game().model(HERO, 1.8, json!([])), &scratch);
    let key = path("model", HERO);

    let hitbox = Capsule {
        radius: Meters(0.4),
        height: Meters(2.1),
    };
    assert!(model::set_hitbox(&mut editor, &key, hitbox));
    support::settled(&mut editor);
    let model = editor
        .read::<ModelDef>(&key)
        .expect("the model is in the project")
        .expect("the model reads");
    assert_eq!(model.hitbox, hitbox);
}
