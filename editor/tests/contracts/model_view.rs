use editor::views::model;
use editor::views::model_part;
use engine::core::geometry::Capsule;
use engine::core::units::Meters;
use engine::systems::actor::ModelDef;
use serde_json::json;
use testkit::{part_file, path};
use ui::egui;

use crate::support::{self, Frames};

const HERO: &str = "hero";

#[test]
fn the_fit_page_tells_whether_a_parts_mesh_fits_its_skeleton() {
    let mut crossed = testkit::rig();
    crossed.nodes[3].parent = Some(2);
    let mut fewer = testkit::rig();
    fewer.nodes.pop();
    for (case, rig, fits) in [("crossed", crossed, false), ("fewer joints", fewer, true)] {
        let scratch = tempfile::tempdir().expect("scratch");
        let game = support::game()
            .model(HERO, 1.8, json!([]))
            .asset(&part_file(HERO), testkit::skin_glb(&rig, &[]));
        let mut editor = support::open_as_is(game, &scratch);
        let key = path("model_part", HERO);
        let mut frames = Frames::new(egui::vec2(1400.0, 900.0));
        frames.run(Vec::new(), |ui| model_part::show(&mut editor, ui, &key));
        let drawn = frames.run(Vec::new(), |ui| model_part::show(&mut editor, ui, &key));
        assert!(drawn.any(|text| text == "hand.l"), "{case}: {drawn:?}");
        assert_eq!(
            editor
                .project
                .as_ref()
                .is_some_and(|project| project.loads()),
            fits,
            "{case}"
        );
        assert_eq!(
            !drawn.any(|text| text.contains("do not fit")),
            fits,
            "{case}: {drawn:?}"
        );
    }
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
