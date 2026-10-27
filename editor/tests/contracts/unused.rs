use editor::rows;
use editor::unused::{self, UnusedRow};
use editor::views::area::nodes;
use engine::systems::area::AreaModule;
use serde_json::json;
use testkit::{Game, path, png};

use crate::support;

/// Wren lives in the start, behind a rock blueprint placing a boulder prop. The hermit lives
/// nowhere, a cap is worn by no one, and a stray icon is named by nothing.
fn hermitage() -> Game {
    let rock = json!([
        { "type": "prop", "prop": path("prop", "boulder"), "at": { "x": 0.0, "y": 0.0 } }
    ]);
    support::game()
        .npc("wren", 10.0, json!([]))
        .npc("hermit", 10.0, json!([]))
        .item("cap", json!({ "kind": { "type": "resource" } }))
        .prop("boulder", json!({ "type": "circle", "radius": 0.5 }))
        .blueprint("rock", (1, 1), json!({}), rock)
        .asset("icons/stray.png", png(16, 16))
        .edit_scene("area", support::AREA, |scene| {
            let nodes = scene.as_array_mut().expect("a scene");
            nodes.push(json!({ "type": "resident", "npc": path("npc", "wren"), "at": { "x": 2.0, "y": 2.0 } }));
            nodes.push(json!({ "type": "blueprint", "label": "rock", "blueprint": path("blueprint", "rock"), "at": { "x": 4.0, "y": 4.0 } }));
        })
}

#[test]
fn what_nothing_in_use_reaches_is_unused_at_any_depth() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(hermitage(), &scratch);
    let project = support::settled(&mut editor);

    let unused: Vec<(&str, &UnusedRow)> = project
        .unused
        .rows()
        .map(|(row, why)| (row.as_str(), why))
        .collect();

    assert_eq!(
        unused,
        vec![
            (path("item", "cap").as_str(), &UnusedRow::Unnamed),
            (
                path("prop", "drop").as_str(),
                &UnusedRow::NamedByUnused(vec![path("item", "cap")])
            ),
            (
                path("model", "hermit").as_str(),
                &UnusedRow::NamedByUnused(vec![path("npc", "hermit")])
            ),
            (
                path("model_part", "hermit").as_str(),
                &UnusedRow::NamedByUnused(vec![path("model", "hermit")])
            ),
            (path("npc", "hermit").as_str(), &UnusedRow::Unnamed),
            (
                path("sfx", "item").as_str(),
                &UnusedRow::NamedByUnused(vec![path("item", "cap")])
            ),
        ]
    );
    let assets: Vec<&str> = project.unused.assets().collect();
    assert_eq!(
        assets,
        ["cards/cap_card.png", "icons/cap.png", "icons/stray.png"]
    );
}

#[test]
fn deleting_unused_content_takes_the_files_and_their_parts_in_one_undo_step() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(hermitage(), &scratch);
    let project = support::settled(&mut editor);
    let rows: Vec<String> = project.unused.rows().map(|(row, _)| row.clone()).collect();
    let assets: Vec<String> = project.unused.assets().map(str::to_owned).collect();
    let sprites = format!("{}/mesh.glb", path("model_part", "hermit"));

    let deletion = unused::delete(project, &rows, &assets).expect("deletes");
    editor.try_apply(deletion.edit).expect("applies");

    let project = support::settled(&mut editor);
    assert!(project.unused.is_empty());
    assert!(!project.files.contains(&path("npc", "hermit")));
    assert!(!project.files.contains(&sprites));
    assert!(!project.files.contains("icons/stray.png"));
    assert!(project.files.contains(&path("npc", "wren")));
    editor.undo();
    let project = support::settled(&mut editor);
    assert!(project.files.contains(&path("npc", "hermit")));
    assert!(project.files.contains(&sprites));
}

#[test]
fn deleting_is_refused_while_something_left_behind_names_what_goes() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(hermitage(), &scratch);
    let project = support::settled(&mut editor);
    let (model, npc, cap) = (
        path("model", "hermit"),
        path("npc", "hermit"),
        path("item", "cap"),
    );

    let model_alone = unused::delete(project, std::slice::from_ref(&model), &[]).err();
    let icon_alone = unused::delete(project, &[], &["icons/cap.png".to_owned()]).err();

    assert!(
        model_alone
            .as_ref()
            .is_some_and(|refusal| refusal.contains(&npc) && refusal.contains(&model)),
        "{model_alone:?}"
    );
    assert!(
        icon_alone
            .as_ref()
            .is_some_and(|refusal| refusal.contains(&cap) && refusal.contains("icons/cap.png")),
        "{icon_alone:?}"
    );
}

#[test]
fn renaming_a_prop_rewrites_the_blueprint_scenes_that_name_it() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(hermitage(), &scratch);

    rows::rename(&mut editor, &path("prop", "boulder"), "stone").expect("renamed");

    support::settled(&mut editor);
    let reader = editor.reader().expect("a reader");
    let props: Vec<Option<&str>> =
        nodes::modules(&nodes::scene_of(&editor, &path("blueprint", "rock")))
            .iter()
            .filter_map(|module| match module.module() {
                AreaModule::Prop(prop) => Some(reader.path_of(prop.prop)),
                _ => None,
            })
            .collect();
    assert_eq!(props, [Some(path("prop", "stone").as_str())]);
}
