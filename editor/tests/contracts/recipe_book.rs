use editor::views::recipe_book::{self, Picked, RecipeSide};
use engine::core::units::Seconds;
use engine::systems::actor::skeleton::MoveName;
use engine::systems::crafting::RecipeBookDef;
use engine::systems::item::{ItemDef, ItemStack};
use serde_json::json;
use testkit::{BODY, Game, Map, path, said};
use ui::egui;

use crate::support::{self, Frames};

const SCREEN: egui::Vec2 = egui::vec2(1600.0, 1000.0);
const NARROW: egui::Vec2 = egui::vec2(760.0, 1000.0);

#[test]
fn a_recipe_book_is_a_list_edited_a_recipe_at_a_time_and_refuses_what_the_loader_would() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(game(), &scratch);
    let key = path("recipe_book", "smithing");
    let item = |name: &str| path("item", name);

    let mut frames = Frames::new(SCREEN);
    let drawn = frames.run(Vec::new(), |ui| recipe_book::show(&mut editor, ui, &key));
    for shown in [
        "ingot ×1",
        "ore ×2",
        "coal ×1",
        "2.5",
        "1 recipes · 1 items made",
    ] {
        assert!(drawn.any(|text| text == shown), "{shown} in {drawn:?}");
    }
    assert_eq!(not_made(&editor, &key), ["ore", "coal"]);

    assert!(recipe_book::add_recipe(
        &mut editor,
        &key,
        &item("sword"),
        &item("ingot")
    ));
    for added in ["plank", "ingot"] {
        assert!(recipe_book::add_stack(
            &mut editor,
            &key,
            1,
            RecipeSide::From,
            &item(added)
        ));
    }
    assert!(recipe_book::set_amount(
        &mut editor,
        &key,
        0,
        (RecipeSide::Makes, 0),
        2
    ));
    assert!(recipe_book::add_stack(
        &mut editor,
        &key,
        0,
        RecipeSide::Makes,
        &item("slag")
    ));
    assert!(recipe_book::set_takes(&mut editor, &key, 0, Seconds(4.0)));
    support::settled(&mut editor);
    let listed = frames.run(Vec::new(), |ui| recipe_book::show(&mut editor, ui, &key));
    let grip = ui::glyph::Mark::Grip.text(1.0).text().to_owned();
    let handles: Vec<egui::Pos2> = listed
        .0
        .iter()
        .filter(|(text, _)| *text == grip)
        .map(|(_, drawn)| drawn.center())
        .collect();
    let [ingot, sword] = handles[..] else {
        panic!("a handle a recipe: {handles:?}");
    };
    let above = ingot - egui::vec2(0.0, 12.0);
    let button = |at, pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    for events in [
        vec![egui::Event::PointerMoved(sword)],
        vec![button(sword, true)],
        vec![egui::Event::PointerMoved(sword - egui::vec2(0.0, 10.0))],
        vec![egui::Event::PointerMoved(above)],
        vec![button(above, false)],
    ] {
        frames.run(events, |ui| recipe_book::show(&mut editor, ui, &key));
    }
    support::settled(&mut editor);
    assert_eq!(
        recipes(&editor, &key),
        [
            "1 sword from 2 ingot + 1 plank",
            "2 ingot + 1 slag from 2 ore + 1 coal"
        ],
        "an item added again adds to its amount, a recipe makes as many items as it is given, \
         and one dragged by its handle stands where it is let go"
    );
    let takes: Vec<f32> = book(&editor, &key)
        .recipes
        .iter()
        .map(|recipe| recipe.takes.0)
        .collect();
    assert_eq!(
        takes,
        [1.0, 4.0],
        "a recipe takes the time it is given, a second until it is"
    );
    assert_eq!(not_made(&editor, &key), ["plank", "ore", "coal"]);

    let narrow = Frames::new(NARROW).run(Vec::new(), |ui| recipe_book::show(&mut editor, ui, &key));
    let drawn = |text: &str| {
        narrow
            .rect(text)
            .unwrap_or_else(|| panic!("{text} in {narrow:?}"))
    };
    let (ingot, ore, slag) = (drawn("ingot ×2"), drawn("ore ×2"), drawn("slag ×1"));
    assert!(
        (ingot.left() - ore.left()).abs() < 1.0,
        "what each recipe is made from starts at one column: {ingot:?} {ore:?}"
    );
    assert!(
        slag.top() > ore.bottom() && slag.right() < ore.left(),
        "what a column has no room for goes on below in it: {slag:?} beside {ore:?}"
    );

    recipe_book::select(&mut editor, &key, Some(Picked::Item(item("ingot"))));
    let followed = frames.run(Vec::new(), |ui| recipe_book::show(&mut editor, ui, &key));
    for shown in ["2 ore + 1 coal", "2 into 1 sword"] {
        assert!(
            followed.any(|text| text == shown),
            "the inspector follows ingot to {shown}: {followed:?}"
        );
    }

    let whittled = Some(MoveName::new("whittle"));
    assert!(recipe_book::set_recipe_work(
        &mut editor,
        &key,
        1,
        whittled.clone()
    ));
    assert!(recipe_book::set_work(
        &mut editor,
        &key,
        MoveName::new("whittle")
    ));
    support::files(&mut editor);
    recipe_book::set_work(&mut editor, &key, MoveName::new("knit"));
    support::files(&mut editor);
    let refused = editor
        .project
        .as_mut()
        .and_then(|project| project.take_refused())
        .expect("a move no skeleton declares is refused as it is set");
    assert_eq!(refused.refusal.problem.rule, "undeclared move");
    let moved = book(&editor, &key);
    assert_eq!(
        (
            moved.work.as_str(),
            &moved.recipes[1].work,
            &moved.recipes[0].work
        ),
        ("whittle", &whittled, &None),
        "the book's move, and a recipe's own"
    );

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

    for (side, rule) in [
        (RecipeSide::From, "nothing to make it from"),
        (RecipeSide::Makes, "nothing made"),
    ] {
        assert!(recipe_book::remove_stack(&mut editor, &key, 1, (side, 0)));
        assert!(
            !recipe_book::remove_stack(&mut editor, &key, 1, (side, 0)),
            "a recipe is never left with {rule}"
        );
        let emptied = editor.last_refusal.as_ref().expect("refused");
        assert_eq!(emptied.problem.rule, rule);
    }
    support::settled(&mut editor);
    assert_eq!(recipes(&editor, &key)[1], "1 slag from 1 coal");

    assert!(recipe_book::remove_recipe(&mut editor, &key, 0));
    support::settled(&mut editor);
    assert_eq!(recipes(&editor, &key), ["1 slag from 1 coal"]);
}

/// A game whose smith opens a book that makes an ingot from ore and coal.
fn game() -> Game {
    let stack = |item: &str, count: u32| json!({ "item": path("item", item), "count": count });
    let mut game = support::game()
        .edit("skeleton", BODY, |body| {
            body["base"]["moves"] = json!({
                "forge": testkit::clip("attack"),
                "whittle": testkit::clip("attack"),
            });
        })
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
                "work": "forge",
                "recipes": [{ "makes": [stack("ingot", 1)], "from": [stack("ore", 2), stack("coal", 1)], "takes": 2.5 }],
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
    for item in ["ore", "coal", "ingot", "slag", "plank", "sword"] {
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
            let listed = |stacks: &[ItemStack]| {
                stacks
                    .iter()
                    .map(|stack| format!("{} {}", stack.count, name(stack.item)))
                    .collect::<Vec<_>>()
                    .join(" + ")
            };
            format!("{} from {}", listed(&recipe.makes), listed(&recipe.from))
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
