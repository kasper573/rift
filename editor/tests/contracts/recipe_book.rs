use editor::views::recipe_book::{self, Picked};
use engine::systems::crafting::RecipeBookDef;
use engine::systems::item::ItemDef;
use serde_json::json;
use testkit::{Game, Map, path, said};
use ui::egui;

use crate::support::{self, Frames};

const SCREEN: egui::Vec2 = egui::vec2(1600.0, 1000.0);

#[test]
fn a_recipe_book_is_a_list_edited_a_recipe_at_a_time_and_refuses_what_the_loader_would() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(game(), &scratch);
    let key = path("recipe_book", "smithing");
    let item = |name: &str| path("item", name);

    let mut frames = Frames::new(SCREEN);
    let drawn = frames.run(Vec::new(), |ui| recipe_book::show(&mut editor, ui, &key));
    for shown in ["ingot", "ore ×2", "coal ×1", "1 recipes · 1 items made"] {
        assert!(drawn.any(|text| text == shown), "{shown} in {drawn:?}");
    }
    assert_eq!(not_made(&editor, &key), ["ore", "coal"]);

    assert!(recipe_book::add_recipe(
        &mut editor,
        &key,
        &item("sword"),
        &item("ingot")
    ));
    assert!(recipe_book::add_ingredient(
        &mut editor,
        &key,
        1,
        &item("plank")
    ));
    assert!(recipe_book::add_ingredient(
        &mut editor,
        &key,
        1,
        &item("ingot")
    ));
    assert!(recipe_book::set_count(&mut editor, &key, 0, 2));
    assert!(recipe_book::move_recipe(&mut editor, &key, 1, 0));
    support::settled(&mut editor);
    assert_eq!(
        recipes(&editor, &key),
        [
            "1 sword from 2 ingot + 1 plank",
            "2 ingot from 2 ore + 1 coal"
        ],
        "an item added again adds to its amount, and the order kept is the order given"
    );
    assert_eq!(not_made(&editor, &key), ["plank", "ore", "coal"]);

    recipe_book::select(&mut editor, &key, Some(Picked::Item(item("ingot"))));
    let followed = frames.run(Vec::new(), |ui| recipe_book::show(&mut editor, ui, &key));
    for shown in ["2 ore + 1 coal", "sword ×2"] {
        assert!(
            followed.any(|text| text == shown),
            "the inspector follows ingot to {shown}: {followed:?}"
        );
    }

    let before = recipes(&editor, &key);
    recipe_book::add_recipe(&mut editor, &key, &item("ore"), &item("sword"));
    support::files(&mut editor);
    let refused = editor
        .project
        .as_mut()
        .and_then(|project| project.take_refused())
        .expect("a loop is refused as it is added");
    assert_eq!(refused.refusal.problem.rule, "recipe loop");
    assert_eq!(recipes(&editor, &key), before);

    assert!(recipe_book::remove_ingredient(&mut editor, &key, 1, 0));
    assert!(
        !recipe_book::remove_ingredient(&mut editor, &key, 1, 0),
        "a recipe is never left with nothing to make it from"
    );
    let emptied = editor.last_refusal.as_ref().expect("refused");
    assert_eq!(emptied.problem.rule, "nothing to make it from");
    support::settled(&mut editor);
    assert_eq!(recipes(&editor, &key)[1], "2 ingot from 1 coal");

    assert!(recipe_book::remove_recipe(&mut editor, &key, 0));
    support::settled(&mut editor);
    assert_eq!(recipes(&editor, &key), ["2 ingot from 1 coal"]);
}

/// A game whose smith opens a book that makes an ingot from ore and coal.
fn game() -> Game {
    let stack = |item: &str, count: u32| json!({ "item": path("item", item), "count": count });
    let mut game = support::game()
        .area(
            support::AREA,
            &Map::open(8, 8),
            json!([{ "type": "resident", "npc": path("npc", "smith"), "at": { "x": 4.0, "y": 4.0 } }]),
        )
        .mark("anvil")
        .row(
            "recipe_book",
            "smithing",
            json!({
                "mark": path("attention", "anvil"),
                "recipes": [{ "makes": stack("ingot", 1), "from": [stack("ore", 2), stack("coal", 1)] }],
            }),
        )
        .npc(
            "smith",
            40.0,
            json!([{ "type": "script", "script": path("script", "smith_talk") }]),
        )
        .script(
            "smith_talk",
            "npc",
            json!({
                "interacted": { "type": "on", "event": "interacted", "verb": "talk", "reach": 2.0, "next": "hello" },
                "hello": said("Lay it down.", "craft"),
                "craft": {
                    "type": "open_crafting",
                    "title": "Smithing",
                    "from": [{ "type": "book", "book": path("recipe_book", "smithing") }],
                },
            }),
        );
    for item in ["ore", "coal", "ingot", "plank", "sword"] {
        game = game.resource(item, 100);
    }
    game
}

/// The book's recipes as sentences, in its order.
fn recipes(editor: &editor::app::Editor, key: &str) -> Vec<String> {
    let book = book(editor, key);
    let name = |item| name_of(editor, item);
    book.recipes
        .iter()
        .map(|recipe| {
            let from: Vec<String> = recipe
                .from
                .iter()
                .map(|stack| format!("{} {}", stack.count, name(stack.item)))
                .collect();
            format!(
                "{} {} from {}",
                recipe.makes.count,
                name(recipe.makes.item),
                from.join(" + ")
            )
        })
        .collect()
}

fn not_made(editor: &editor::app::Editor, key: &str) -> Vec<String> {
    recipe_book::not_made(&book(editor, key))
        .into_iter()
        .map(|item| name_of(editor, item))
        .collect()
}

fn book(editor: &editor::app::Editor, key: &str) -> std::sync::Arc<RecipeBookDef> {
    editor
        .read::<RecipeBookDef>(key)
        .expect("a project")
        .expect("the book reads")
}

fn name_of(editor: &editor::app::Editor, item: engine::systems::item::ItemId) -> String {
    let reader = editor.reader().expect("a project");
    let path = reader.path_of(item).expect("an item file").to_owned();
    editor
        .read::<ItemDef>(&path)
        .expect("a project")
        .expect("the item reads")
        .name
        .clone()
}
