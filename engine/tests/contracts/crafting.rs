use bevy_ecs::prelude::*;
use engine::core::content::LoadReport;
use engine::core::units::Seconds;
use engine::systems::crafting::{self, CraftingRequest, CraftingView, RecipeBookDef, RecipeKey};
use engine::systems::dialogue::{self, ConversationRequest};
use engine::systems::interact::{self, InteractRequest};
use engine::systems::item::{ItemDef, ItemStack};
use engine::systems::prompt::{self, PromptRequest, PromptView};
use engine::systems::stat::{StatKind, Stats};
use engine::systems::{movement, region, script};
use serde_json::{Value, json};
use testkit::{Game, Map, Sim, path, said, sent};

const INGOT: (&str, usize) = ("smithing", 0);
const PLANK: (&str, usize) = ("smithing", 1);
const SWORD: (&str, usize) = ("smithing", 2);
const WHITTLED: (&str, usize) = ("smithing", 3);
const PARTS: (&str, usize) = ("salvage", 0);
const STRAPS: (&str, usize) = ("salvage", 1);

#[test]
fn a_crafting_prompt_makes_what_its_books_and_subsets_allow_and_nothing_else() {
    let mut sim = forge(game(|_| {}));
    let player = sim.join(1);
    sim.give(player, "ore", 10);
    sim.give(player, "coal", 10);
    sim.give(player, "log", 10);
    sim.give(player, "ingot", 3);
    sim.give(player, "plank", 3);

    let (asked, crafting) = open(&mut sim, 1, "smith");
    assert_eq!(asked.title, "Smithing");
    assert_eq!(
        asked.slots, 2,
        "as many slots as its biggest recipe has ingredients"
    );
    assert_eq!(made_by(&sim, &crafting), ["ingot", "plank", "sword"]);

    craft(&mut sim, 1, &[("coal", 1), ("ore", 2)], INGOT);
    assert_eq!(sim.count(player, "ingot"), 4, "whichever slot holds what");
    assert_eq!(
        (sim.count(player, "ore"), sim.count(player, "coal")),
        (8, 9)
    );
    let (again, after) = shown(&sim, player).expect("it stays open");
    assert_ne!(again.step, asked.step);
    assert_eq!(after.crafted, 1);
    assert_eq!(sim.count(player, "token"), 1, "the book heard of it");

    craft(&mut sim, 1, &[("log", 3)], PLANK);
    assert_eq!(
        (sim.count(player, "log"), sim.count(player, "plank")),
        (7, 9),
        "three crafts of two each at once"
    );

    sim.leave(1);
    assert_eq!(shown(&sim, player), None);
    assert_eq!(sim.count(player, "slip"), 1, "leaving goes down closed");

    let (_, crafting) = open(&mut sim, 1, "carpenter");
    assert_eq!(made_by(&sim, &crafting), ["plank"], "a subset of the book");
    craft(&mut sim, 1, &[("ore", 2)], INGOT);
    assert_eq!(sim.count(player, "ingot"), 4, "not this prompt's to make");
    assert_eq!(sim.errors(1), ["No recipe"]);
    sim.forget_notified();
    craft(&mut sim, 1, &[("log", 1)], PLANK);
    assert_eq!(sim.count(player, "plank"), 11);
    assert_eq!(sim.errors(1), Vec::<String>::new());

    let conn = sim.conn(1);
    let step = shown(&sim, player).expect("open").0.step;
    let offer = stacks(&sim, &[("log", 1)]);
    sim.send(conn, PromptRequest::Submit { step, offer });
    sim.run(0.5);
    assert!(
        shown(&sim, player).is_some(),
        "a crafting prompt hands no offer to its script"
    );
}

#[test]
fn with_several_recipes_the_one_chosen_is_made_with_everything_it_gives() {
    let mut sim = forge(game(|_| {}));
    let player = sim.join(1);
    sim.give(player, "blade", 4);
    let (_, crafting) = open(&mut sim, 1, "salvager");
    assert_eq!(made_by(&sim, &crafting), ["ingot + strap", "strap"]);

    craft(&mut sim, 1, &[("blade", 2)], STRAPS);
    assert_eq!(
        (sim.count(player, "strap"), sim.count(player, "ingot")),
        (6, 0)
    );
    craft(&mut sim, 1, &[("blade", 2)], PARTS);
    assert_eq!(
        (sim.count(player, "strap"), sim.count(player, "ingot")),
        (10, 2),
        "two crafts of an ingot and two straps each"
    );
    assert_eq!(sim.count(player, "blade"), 0);
}

#[test]
fn a_craft_spends_every_ingredient_and_fits_or_does_not_happen() {
    let bundle = json!({
        "makes": [{ "item": path("item", "gem") }, { "item": path("item", "strap"), "count": 2 }],
        "from": [{ "item": path("item", "log"), "count": 2 }],
        "takes": 0.1,
    });
    let whittling = game(|book| {
        book["recipes"]
            .as_array_mut()
            .expect("recipes")
            .push(bundle)
    });
    let mut sim = forge(whittling.edit_settings(|settings| settings["slots"]["bag"] = json!(3)));
    let player = sim.join(1);
    sim.give(player, "ore", 4);
    sim.give(player, "coal", 2);
    sim.give(player, "log", 2);
    let (asked, _) = open(&mut sim, 1, "smith");

    craft(&mut sim, 1, &[("ore", 2), ("coal", 1)], INGOT);
    assert_eq!(sim.errors(1), ["Needs 1 free slot"]);
    assert_eq!(
        (
            sim.count(player, "ore"),
            sim.count(player, "coal"),
            sim.count(player, "ingot")
        ),
        (4, 2, 0),
        "a product with no room for it takes nothing"
    );
    let (again, _) = shown(&sim, player).expect("still open");
    assert_ne!(again.step, asked.step, "the turn was answered");

    sim.forget_notified();
    craft(&mut sim, 1, &[("ore", 2), ("coal", 1)], SWORD);
    assert_eq!(sim.errors(1), ["No recipe"], "another recipe's result");
    sim.forget_notified();
    craft(&mut sim, 1, &[("ore", 4), ("coal", 2)], INGOT);
    assert_eq!(
        (
            sim.count(player, "ore"),
            sim.count(player, "coal"),
            sim.count(player, "ingot")
        ),
        (0, 0, 2),
        "room the ingredients leave is room for what they make"
    );

    for (offer, why) in [
        (vec![("log", 2), ("ore", 0)], "an empty stack"),
        (vec![("log", 3)], "more than the bag holds"),
        (vec![("log", 1), ("ingot", 1)], "more than the recipe takes"),
    ] {
        craft(&mut sim, 1, &offer, PLANK);
        assert_eq!(sim.count(player, "plank"), 0, "{why}");
        assert_eq!(sim.count(player, "log"), 2, "{why}");
    }
    sim.take(player, "token", 1);
    sim.give(player, "log", 2);
    sim.forget_notified();
    craft(&mut sim, 1, &[("log", 2)], WHITTLED);
    assert_eq!(sim.errors(1), ["Needs 1 free slot"]);
    assert_eq!(
        (
            sim.count(player, "log"),
            sim.count(player, "gem"),
            sim.count(player, "strap")
        ),
        (4, 0, 0),
        "a bundle the bag can't hold whole gives none of it"
    );
    craft(&mut sim, 1, &[("log", 4)], WHITTLED);
    assert_eq!(
        (
            sim.count(player, "log"),
            sim.count(player, "gem"),
            sim.count(player, "strap")
        ),
        (0, 2, 4)
    );
}

#[test]
fn a_craft_takes_its_recipes_time_at_the_crafters_speed_and_nothing_until_it_is_made() {
    let slow = game(|book| book["recipes"][0]["takes"] = json!(4.0));
    let mut sim = forge(slow);
    let player = sim.join(1);
    let conn = sim.conn(1);
    sim.give(player, "ore", 8);
    sim.give(player, "coal", 4);
    let held = |sim: &Sim| {
        (
            sim.count(player, "ore"),
            sim.count(player, "coal"),
            sim.count(player, "ingot"),
        )
    };
    let (asked, _) = open(&mut sim, 1, "smith");

    craft(&mut sim, 1, &[("ore", 4), ("coal", 2)], INGOT);
    let (during, crafting) = shown(&sim, player).expect("open");
    let making = crafting.making.expect("a craft under way");
    assert_eq!(
        making.until.since(making.since),
        Seconds(8.0),
        "two crafts of four seconds each"
    );
    assert_eq!(during.step, asked.step, "the turn waits for the craft");
    assert_eq!(held(&sim), (8, 4, 0), "nothing leaves the bag yet");
    craft(&mut sim, 1, &[("ore", 2), ("coal", 1)], INGOT);
    sim.run(5.0);
    assert_eq!(
        held(&sim),
        (8, 4, 0),
        "one craft at a time, and not yet due"
    );

    sim.send(conn, CraftingRequest::Cancel);
    sim.run(4.0);
    let (after, crafting) = shown(&sim, player).expect("still open");
    assert_eq!(crafting.making, None);
    assert_ne!(after.step, asked.step, "called off, the next turn begins");
    assert_eq!(held(&sim), (8, 4, 0), "a craft called off costs nothing");

    sim.world
        .get_mut::<Stats>(player)
        .expect("stats")
        .0
        .insert(StatKind::CraftingSpeed, 200.0);
    craft(&mut sim, 1, &[("ore", 2), ("coal", 1)], INGOT);
    sim.run(1.0);
    assert_eq!(held(&sim), (8, 4, 0), "twice as fast is still two seconds");
    sim.run(1.0);
    assert_eq!(held(&sim), (6, 3, 1));
    assert_eq!(shown(&sim, player).expect("open").1.crafted, 1);

    craft(&mut sim, 1, &[("ore", 2), ("coal", 1)], INGOT);
    sim.take(player, "coal", 3);
    sim.run(2.5);
    assert_eq!(sim.errors(1), ["Needs 1 more coal"]);
    assert_eq!(
        held(&sim),
        (6, 0, 1),
        "ingredients gone by the time it is due make nothing"
    );

    sim.give(player, "coal", 1);
    craft(&mut sim, 1, &[("ore", 2), ("coal", 1)], INGOT);
    sim.leave(1);
    sim.run(3.0);
    assert_eq!(held(&sim), (6, 1, 1), "leaving the prompt ends the craft");
}

#[test]
fn the_loader_refuses_recipes_that_loop_repeat_or_profit_and_books_no_script_opens() {
    let of = |item: &str, count: u32| json!({ "item": path("item", item), "count": count });
    let recipe =
        |makes: &str, from: Value| json!({ "makes": [of(makes, 1)], "from": from, "takes": 0.1 });
    let cases: [(&str, Value); 9] = [
        ("recipe loop", recipe("ore", json!([of("sword", 1)]))),
        (
            "made twice",
            recipe("sword", json!([of("plank", 4), of("ingot", 1)])),
        ),
        ("nothing to make it from", recipe("strap", json!([]))),
        (
            "nothing made",
            json!({ "makes": [], "from": [of("blade", 1)], "takes": 0.1 }),
        ),
        (
            "made item listed twice",
            json!({ "makes": [of("strap", 1), of("strap", 2)], "from": [of("blade", 1)], "takes": 0.1 }),
        ),
        (
            "out of range",
            json!({ "makes": [of("strap", 4)], "from": [of("blade", 1)], "takes": -1.0 }),
        ),
        ("out of range", recipe("strap", json!([of("blade", 0)]))),
        (
            "ingredient listed twice",
            recipe("strap", json!([of("blade", 1), of("blade", 2)])),
        ),
        (
            "crafting mints money",
            recipe("gem", json!([of("coal", 1)])),
        ),
    ];
    for (rule, added) in cases {
        let report = refused(game(|book| {
            book["recipes"]
                .as_array_mut()
                .expect("recipes")
                .push(added.clone());
        }));
        assert_eq!(rules(&report), [rule], "{added}");
    }

    let unopened = game(|_| {}).whole().row(
        "recipe_book",
        "spare",
        json!({ "mark": path("attention", "anvil"), "recipes": [] }),
    );
    let report = refused(unopened);
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.problem.rule == "never opened"
                && error.file == path("recipe_book", "spare")),
        "{report}"
    );

    let elsewhere = refused(game(|_| {}).edit("script", "carpenter_talk", |script| {
        script["nodes"]["craft"]["from"][0]["items"] = json!([path("item", "blade")]);
    }));
    assert_eq!(rules(&elsewhere), ["not in the book"]);
}

/// A forge whose smith crafts from all of the smithing book, whose carpenter crafts only its
/// planks, and whose salvager takes blades apart two ways, one of them into an ingot and straps
/// together. Every craft takes a tenth of a second. The smithing book hands a token for every
/// craft; leaving a prompt hands a slip.
fn game(change: impl FnOnce(&mut Value)) -> Game {
    let stack = |item: &str, count: u32| json!({ "item": path("item", item), "count": count });
    let mut smithing = json!({
        "mark": path("attention", "anvil"),
        "recipes": [
            { "makes": [stack("ingot", 1)], "from": [stack("ore", 2), stack("coal", 1)], "takes": 0.1 },
            { "makes": [stack("plank", 2)], "from": [stack("log", 1)], "takes": 0.1 },
            { "makes": [stack("sword", 1)], "from": [stack("ingot", 2), stack("plank", 1)], "takes": 0.1 },
        ],
        "modules": [{ "type": "script", "script": path("script", "smithing_crafted") }],
    });
    change(&mut smithing);
    let crafter = |title: &str, from: Value| {
        json!({
            "interacted": { "type": "on", "event": "interacted", "verb": "talk", "reach": 2.0, "next": "hello" },
            "hello": said("Lay it down.", "craft"),
            "craft": {
                "type": "open_crafting",
                "title": title,
                "from": from,
                "closed": "left",
            },
            "left": { "type": "items", "give": [{ "item": path("item", "slip") }] },
        })
    };
    let talks = |name: &str| json!([{ "type": "script", "script": path("script", &format!("{name}_talk")) }]);
    let resident = |name: &str, x: f32| json!({ "type": "resident", "npc": path("npc", name), "at": { "x": x, "y": 4.0 } });
    let mut game = Game::new()
        .area(
            "forge",
            &Map::open(16, 12),
            json!([resident("smith", 3.0), resident("carpenter", 8.0), resident("salvager", 13.0)]),
        )
        .hero("forge", 8.0, 6.0)
        .mark("anvil")
        .sfx("ui_refuse")
        .sfx("ui_close")
        .row("recipe_book", "smithing", smithing)
        .row(
            "recipe_book",
            "salvage",
            json!({
                "mark": path("attention", "anvil"),
                "recipes": [
                    { "makes": [stack("ingot", 1), stack("strap", 2)], "from": [stack("blade", 1)], "takes": 0.1 },
                    { "makes": [stack("strap", 3)], "from": [stack("blade", 1)], "takes": 0.1 },
                ],
            }),
        )
        .script(
            "smithing_crafted",
            "recipe_book",
            json!({
                "crafted": { "type": "on", "event": "crafted", "item": path("item", "ingot"), "next": "thank" },
                "thank": { "type": "items", "give": [{ "item": path("item", "token") }] },
            }),
        )
        .row(
            "shop",
            "stall",
            json!({
                "title": "Stall",
                "mark": path("attention", "anvil"),
                "modules": [
                    {
                        "type": "sells",
                        "item": path("item", "coal"),
                        "price": [stack("gold", 2)],
                        "stock": { "type": "unlimited" },
                    },
                    { "type": "buys", "what": { "type": "item", "item": path("item", "gem") }, "pays": [stack("gold", 5)] },
                ],
            }),
        )
        .npc("smith", 40.0, talks("smith"))
        .script(
            "smith_talk",
            "npc",
            crafter("Smithing", json!([{ "type": "book", "book": path("recipe_book", "smithing") }])),
        )
        .npc("carpenter", 40.0, talks("carpenter"))
        .script(
            "carpenter_talk",
            "npc",
            crafter(
                "Sawhorse",
                json!([{
                    "type": "subset",
                    "book": path("recipe_book", "smithing"),
                    "items": [path("item", "plank")],
                }]),
            ),
        )
        .npc("salvager", 40.0, talks("salvager"))
        .script(
            "salvager_talk",
            "npc",
            crafter("Salvage", json!([{ "type": "book", "book": path("recipe_book", "salvage") }])),
        );
    for item in [
        "ore", "coal", "log", "ingot", "plank", "sword", "blade", "strap", "gem", "gold", "token",
        "slip",
    ] {
        game = game.resource(item, 100);
    }
    game
}

fn forge(game: Game) -> Sim {
    let mut sim = Sim::new(game.load());
    sim.serve("forge")
        .accept::<InteractRequest>()
        .accept::<ConversationRequest>()
        .accept::<PromptRequest>()
        .accept::<CraftingRequest>()
        .add((
            interact::interact_request,
            dialogue::requests,
            prompt::requests,
            crafting::requests,
        ))
        .plan((
            script::RunFlows::default(),
            movement::AdvanceWalks::default(),
        ))
        .add((
            interact::interactions,
            dialogue::hold,
            dialogue::refresh,
            prompt::hold,
            crafting::hold,
            crafting::work,
        ));
    script::spawn_stateless(&mut sim.world);
    region::hold_whole(&mut sim.world);
    sim
}

fn refused(game: Game) -> LoadReport {
    game.try_load().err().expect("refused")
}

fn rules(report: &LoadReport) -> Vec<&str> {
    report
        .errors
        .iter()
        .map(|error| error.problem.rule)
        .collect()
}

/// Talks to `keeper` until their crafting prompt stands open.
fn open(sim: &mut Sim, client: u32, keeper: &str) -> (PromptView, CraftingView) {
    let player = sim.player(client);
    sim.talk(client, keeper);
    assert!(
        sim.run_until(20.0, |world| world.get::<CraftingView>(player).is_some()),
        "the {keeper} never opened a crafting prompt"
    );
    shown(sim, player).expect("the prompt is open")
}

/// The prompt as its player's client is shown it.
fn shown(sim: &Sim, player: Entity) -> Option<(PromptView, CraftingView)> {
    Some((
        sent(sim.world.get::<PromptView>(player)?),
        sent(sim.world.get::<CraftingView>(player)?),
    ))
}

/// What each of the prompt's recipes makes, in the order it lists them.
fn made_by(sim: &Sim, crafting: &CraftingView) -> Vec<String> {
    let content = sim.world.resource::<engine::core::content::Content>();
    crafting::craftables(content, &crafting.recipes)
        .iter()
        .map(|craftable| {
            let names: Vec<&str> = craftable
                .recipe
                .makes
                .iter()
                .map(|stack| stack.item.get(content).name.as_str())
                .collect();
            names.join(" + ")
        })
        .collect()
}

fn stacks(sim: &Sim, offer: &[(&str, u32)]) -> Vec<ItemStack> {
    offer
        .iter()
        .map(|(item, count)| ItemStack::new(sim.id::<ItemDef>(item), *count))
        .collect()
}

/// Asks the prompt open before the player to craft the recipe at `index` of `book` from `offer`,
/// in its current turn, and gives it half a second.
fn craft(sim: &mut Sim, client: u32, offer: &[(&str, u32)], (book, index): (&str, usize)) {
    let player = sim.player(client);
    let conn = sim.conn(client);
    let step = shown(sim, player).expect("a crafting prompt").0.step;
    let request = CraftingRequest::Craft {
        step,
        offer: stacks(sim, offer),
        recipe: RecipeKey {
            book: sim.id::<RecipeBookDef>(book),
            index,
        },
    };
    sim.send(conn, request);
    sim.run(0.5);
}
