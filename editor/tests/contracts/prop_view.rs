use editor::views::prop;
use engine::core::time::Seconds;
use engine::core::units::Meters;
use engine::systems::area::prop::{Footprint, PropDef};
use serde_json::json;
use testkit::path;

use crate::support;

const STONE: &str = "stone";

/// The loader's hash of the model of the prop at `key`, once the project settles.
fn model_hash(editor: &mut editor::app::Editor, key: &str) -> String {
    let project = support::settled(editor);
    let model = project
        .read::<PropDef>(key)
        .expect("the prop reads")
        .model
        .path();
    project
        .files
        .hash_of(&model)
        .expect("the model is hashed")
        .to_owned()
}

fn prop_of(editor: &editor::app::Editor, key: &str) -> PropDef {
    editor
        .read::<PropDef>(key)
        .expect("the prop is in the project")
        .expect("the prop reads")
        .as_ref()
        .clone()
}

#[test]
fn a_fitted_footprint_follows_its_model_when_the_model_is_replaced() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game().prop(STONE, json!({ "type": "box", "width": 1.0, "depth": 1.0 }));
    let mut editor = support::open(game, &scratch);
    let key = path("prop", STONE);

    assert!(prop::fit_to_mesh(&mut editor, &key));
    let fitted = model_hash(&mut editor, &key);
    match prop_of(&editor, &key).footprint {
        Footprint::Polygon { points, fitted_to } => {
            assert!(points.len() >= 3, "a polygon round the model's base");
            assert_eq!(
                fitted_to.as_ref().map(|hash| hash.0.as_str()),
                Some(fitted.as_str())
            );
        }
        other => panic!("fitting gives a polygon, not {other:?}"),
    }

    assert!(prop::replace_model(
        &mut editor,
        &key,
        testkit::glb(&[("turn", Seconds(1.0))])
    ));
    support::settled(&mut editor);
    let refitted = model_hash(&mut editor, &key);
    assert_ne!(refitted, fitted, "the model changed");
    match prop_of(&editor, &key).footprint {
        Footprint::Polygon { fitted_to, .. } => {
            assert_eq!(
                fitted_to.as_ref().map(|hash| hash.0.as_str()),
                Some(refitted.as_str()),
                "the footprint follows the model it was fitted to"
            );
        }
        other => panic!("the footprint stays a polygon, not {other:?}"),
    }
    editor.undo();
    support::settled(&mut editor);
    assert_eq!(
        model_hash(&mut editor, &key),
        fitted,
        "undoing brings the old model back"
    );
    assert_eq!(
        match prop_of(&editor, &key).footprint {
            Footprint::Polygon { fitted_to, .. } => fitted_to.map(|hash| hash.0),
            _ => None,
        }
        .as_deref(),
        Some(fitted.as_str()),
        "and the footprint fitted to it"
    );
}

#[test]
fn a_shaped_footprint_lands_in_the_prop_and_undoes() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game().prop(STONE, json!({ "type": "none" }));
    let mut editor = support::open(game, &scratch);
    let key = path("prop", STONE);
    let circle = Footprint::Circle {
        radius: Meters(0.75),
    };
    assert!(prop::shape(&mut editor, &key, circle.clone()));
    support::settled(&mut editor);
    assert_eq!(prop_of(&editor, &key).footprint, circle);
    assert!(
        !prop::shape(&mut editor, &key, circle.clone()),
        "the same footprint again is no step"
    );
    editor.undo();
    support::settled(&mut editor);
    assert_eq!(prop_of(&editor, &key).footprint, Footprint::None);
}
