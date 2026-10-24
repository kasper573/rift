use editor::rows;
use editor::unused::{self, UnusedRow};
use editor::views::area::nodes;
use engine::systems::area::AreaModule;
use serde_json::json;
use testkit::{Game, path, png};

use crate::support;

/// Wren lives in the start, behind a rock blueprint drawn from its own tileset. The hermit lives
/// nowhere, a cap is worn by no one, and a stray icon is named by nothing.
fn hermitage() -> Game {
    let rock = json!([
        { "type": "scenery", "at": { "x": 0.0, "y": 0.0 }, "tileset": path("tileset", "rocks"), "tiles": [[0]] }
    ]);
    support::game()
        .npc("wren", 10.0, json!([]))
        .npc("hermit", 10.0, json!([]))
        .item("cap", json!({ "kind": { "type": "resource" } }))
        .row("tileset", "rocks", json!({ "tile": 16, "tiles": {} }))
        .part("tileset", "rocks", "image.png", png(16, 16))
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
                path("model", "hermit").as_str(),
                &UnusedRow::NamedByUnused(vec![path("npc", "hermit")])
            ),
            (path("npc", "hermit").as_str(), &UnusedRow::Unnamed),
            (
                path("sfx", "item").as_str(),
                &UnusedRow::NamedByUnused(vec![path("item", "cap")])
            ),
        ]
    );
    let assets: Vec<&str> = project.unused.assets().collect();
    assert_eq!(assets, ["icons/cap.png", "icons/stray.png"]);
}

#[test]
fn a_tileset_named_only_by_a_palette_or_a_blueprint_scene_is_used_there() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(hermitage(), &scratch);
    let project = support::settled(&mut editor);

    assert!(
        project
            .unused
            .row(&path("tileset", "start_ground"))
            .is_none()
    );
    assert!(project.unused.row(&path("tileset", "rocks")).is_none());
}

#[test]
fn deleting_unused_content_takes_the_files_and_their_parts_in_one_undo_step() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(hermitage(), &scratch);
    let project = support::settled(&mut editor);
    let rows: Vec<String> = project.unused.rows().map(|(row, _)| row.clone()).collect();
    let assets: Vec<String> = project.unused.assets().map(str::to_owned).collect();
    let sprites = format!("{}/model.glb", path("model", "hermit"));

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
fn renaming_a_tileset_rewrites_the_palettes_and_blueprint_scenes_that_name_it() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(hermitage(), &scratch);

    rows::rename(&mut editor, &path("tileset", "start_ground"), "meadow").expect("renamed");
    rows::rename(&mut editor, &path("tileset", "rocks"), "boulders").expect("renamed");

    let project = support::settled(&mut editor);
    let palette = project
        .files
        .text(&format!("{}/palette.csv", path("area", "start")))
        .expect("the palette");
    assert_eq!(
        palette,
        format!("tileset,tile,flip\n{},0,\n", path("tileset", "meadow"))
    );
    let reader = editor.reader().expect("a reader");
    let tilesets: Vec<Option<&str>> =
        nodes::modules(&nodes::scene_of(&editor, &path("blueprint", "rock")))
            .iter()
            .filter_map(|module| match module.module() {
                AreaModule::Scenery(scenery) => Some(reader.path_of(scenery.tileset)),
                _ => None,
            })
            .collect();
    assert_eq!(tilesets, [Some(path("tileset", "boulders").as_str())]);
}
