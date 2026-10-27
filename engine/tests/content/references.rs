use engine::core::content::id::LeafKind;
use engine::core::content::{Content, fields_file};
use engine::systems::registry;
use serde_json::json;

use testkit::path;

use crate::support::game;

#[test]
fn a_reference_is_placed_where_it_is_read_not_where_its_key_is_first_written() {
    let game = game().npc("wren", 10.0, json!([]));
    let folder = tempfile::tempdir().expect("a scratch folder");
    game.write_to(folder.path());
    let file = fields_file(&path("npc", "wren"));
    let text = std::fs::read_to_string(folder.path().join(&file)).expect("the npc's file");
    let content = game.load();

    let references = registry().references(&content, &file, &text);

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
    let (content, file, text) = loaded_with_text(game, &scene);

    let references = registry().references(&content, &file, &text);

    let props: Vec<(&str, &str)> = references
        .iter()
        .filter(|reference| reference.kind == LeafKind::Row("prop"))
        .map(|reference| (reference.value.as_str(), reference.pointer.as_str()))
        .collect();
    assert_eq!(props, [("rock.prop", "/0/prop")]);
}

fn loaded_with_text(game: testkit::Game, file: &str) -> (Content, String, String) {
    let folder = tempfile::tempdir().expect("a scratch folder");
    game.write_to(folder.path());
    let text = std::fs::read_to_string(folder.path().join(file)).expect("the file");
    (game.load(), file.to_owned(), text)
}
