use std::sync::Arc;

use engine::core::content::id::LeafKind;
use engine::core::content::{FolderReader, fields_file};
use engine::systems::registry;
use serde_json::json;

use testkit::path;

use crate::support::game;

#[test]
fn a_reference_is_placed_where_it_is_read_not_where_its_key_is_first_written() {
    let game = game().npc("wren", 10.0, json!([]));
    let file = fields_file(&path("npc", "wren"));
    let (reader, text) = read_with_text(game, &file);

    let references = registry().references(&reader, &file, &text);

    let models: Vec<&str> = references
        .iter()
        .filter(|reference| reference.kind == LeafKind::Row("model"))
        .map(|reference| reference.pointer.as_str())
        .collect();
    assert_eq!(models, ["/appearance/model"]);
}

#[test]
fn a_blueprint_part_names_its_rows_even_around_a_hole_with_no_default() {
    let game = game().prop("rock", json!({ "type": "none" })).blueprint(
        "rock",
        (1, 1),
        json!({ "x": { "type": "number" } }),
        json!([{
            "type": "prop",
            "prop": path("prop", "rock"),
            "at": { "x": { "var": "x" }, "y": 0.0 },
        }]),
    );
    let scene = format!("{}/scene.json", path("blueprint", "rock"));
    let (reader, text) = read_with_text(game, &scene);

    let references = registry().references(&reader, &scene, &text);

    let props: Vec<(&str, &str)> = references
        .iter()
        .filter(|reference| reference.kind == LeafKind::Row("prop"))
        .map(|reference| (reference.value.as_str(), reference.pointer.as_str()))
        .collect();
    assert_eq!(props, [("rock.prop", "/0/prop")]);
}

/// What reads `game`'s files, and the text of its file at `file`.
fn read_with_text(game: testkit::Game, file: &str) -> (FolderReader, String) {
    let folder = game.folder();
    let text = String::from_utf8(folder.files[file].to_vec()).expect("a text file");
    (FolderReader::new(registry(), Arc::new(folder)), text)
}
