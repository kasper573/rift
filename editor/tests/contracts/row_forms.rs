use std::sync::Arc;
use ui::egui;

use editor::app::Editor;
use editor::project::RowSelection;
use editor::shell::row_form;
use editor::views::class::TREE_PART;
use editor::views::class::inspect as class_inspect;
use editor::views::{character, spreadsheet};
use engine::core::audio::playback::{SfxDef, SfxScalar};
use engine::core::content::Fixture;
use engine::core::math::Pos;
use engine::core::units::{MeterPos, Meters};
use engine::core::vfx::{ParamFill, ParamValue, VfxDef, VfxLayer};
use engine::systems::area::world_map::{WorldMap, WorldMapDef};
use engine::systems::class::{AccessRequirement, ClassDef, NodeKey};
use engine::systems::item::{ItemDef, ItemKind, ItemModule};
use engine::systems::memory::{MemoryDef, Resets};
use engine::systems::movement;
use engine::systems::notification::{NotificationDef, NotificationDefKind};
use engine::systems::npc::NpcDef;
use engine::systems::player::snapshot::{CharacterSnapshot, CharacterStart};
use engine::systems::script::{RunnerKind, ScriptDef, VarValue};
use engine::systems::skill::{SkillDef, Targeting};
use engine::systems::stat::StatKind;
use serde_json::{Value, json};
use testkit::{Game, Map, path};

use crate::support::{self, Drawn, Frames, click, key};

#[test]
fn a_test_character_shows_where_it_starts_in_an_area_of_any_size() {
    for (width, height, x, y) in [(8, 8, 1.0, 1.0), (16, 12, 8.0, 6.0), (90, 40, 85.0, 2.0)] {
        let scratch = tempfile::tempdir().expect("scratch");
        let game = Game::new()
            .area(support::AREA, &Map::open(width, height), json!([]))
            .hero(support::AREA, x, y)
            .row(
                "test_character",
                "tester",
                json!({ "name": "Tester", "class": path("class", "hero"), "start": "initial_spawn" }),
            )
            .engine_rows()
            .whole();
        let mut editor = support::open(game, &scratch);
        let tester = path("test_character", "tester");

        let drawn = Frames::new(egui::vec2(1600.0, 1000.0))
            .run(Vec::new(), |ui| character::show(&mut editor, ui, &tester));

        assert!(
            drawn.rect(&format!("{x}, {y}")).is_some(),
            "{width}×{height}: {drawn:?}"
        );
    }
}

#[test]
fn a_test_character_set_up_for_a_quest_moment_starts_beside_whom_it_is_with() {
    let (giver, taker) = (Pos::<Meters>::new(2.0, 2.0), Pos::<Meters>::new(6.0, 5.0));
    for (moment, with) in [("just offered", giver), ("ready to hand in", taker)] {
        let scratch = tempfile::tempdir().expect("scratch");
        let resident = |npc: &str, at: Pos<Meters>| json!({ "type": "resident", "npc": path("npc", npc), "at": { "x": at.x, "y": at.y } });
        let talks = |quest: Value| {
            json!({
                "interacted": { "type": "on", "event": "interacted", "verb": "talk", "reach": 2.0, "next": "quest" },
                "quest": quest,
            })
        };
        let letter = path("quest", "letter");
        let game = Game::new()
            .area(
                support::AREA,
                &Map::open(8, 8),
                json!([resident("giver", giver), resident("taker", taker)]),
            )
            .hero(support::AREA, 1.0, 1.0)
            .script("giver_talk", "npc", talks(json!({ "type": "offer_quest", "quest": letter })))
            .script("taker_talk", "npc", talks(json!({ "type": "turn_in", "quest": letter })))
            .npc("giver", 40.0, json!([{ "type": "script", "script": path("script", "giver_talk") }]))
            .npc("taker", 40.0, json!([{ "type": "script", "script": path("script", "taker_talk") }]))
            .row(
                "quest",
                "letter",
                json!({
                    "title": "A Letter",
                    "category": "Errands",
                    "blurb": "A letter",
                    "xp": 10,
                    "modules": [
                        { "type": "task", "task": "read", "label": "Read the letter" },
                        { "type": "hand_over", "text": ["I read it."] },
                    ],
                }),
            )
            .row(
                "test_character",
                "tester",
                json!({ "name": "Tester", "class": path("class", "hero"), "start": "initial_spawn" }),
            )
            .engine_rows()
            .whole();
        let mut editor = support::open(game, &scratch);
        let tester = path("test_character", "tester");
        let mut frames = Frames::new(egui::vec2(1600.0, 1000.0));
        let mut draw = |editor: &mut Editor, events: Vec<egui::Event>| {
            frames.run(events, |ui| character::show(editor, ui, &tester))
        };

        let quests = draw(&mut editor, Vec::new()).at("Quests");
        for events in click(quests) {
            draw(&mut editor, events);
        }
        let set_up = draw(&mut editor, Vec::new()).at("Set up for…");
        for events in click(set_up) {
            draw(&mut editor, events);
        }
        let picked = draw(&mut editor, Vec::new()).at(moment);
        for events in click(picked) {
            draw(&mut editor, events);
        }
        draw(&mut editor, Vec::new());
        support::files(&mut editor);

        let character = editor
            .read::<CharacterSnapshot>(&tester)
            .expect("a project")
            .expect("the test character reads");
        let CharacterStart::At(start) = character.start else {
            panic!("{moment}: the character starts at a place");
        };
        let apart = start.at.distance(with);
        assert!(
            apart >= movement::SPACING && apart <= Meters(movement::SPACING.0 * 3.0),
            "{moment}: starts at {:?}, beside {with:?}",
            start.at
        );
    }
}

#[test]
fn a_multi_row_form_of_a_type_without_modules_says_nothing_of_modules() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game()
        .memory("taunted", "never")
        .memory("bribed", "never");
    let mut editor = support::open(game, &scratch);
    let keys = [path("memory", "taunted"), path("memory", "bribed")];

    let drawn = rows_form(&mut editor, "memory", &keys, Vec::new());

    assert!(drawn.rect("label").is_some(), "{drawn:?}");
    assert!(
        !drawn.any(|text| text.contains("modules don't read") || text.contains("Add module")),
        "{drawn:?}"
    );
}

#[test]
fn an_edit_to_a_field_the_rows_hold_apart_sets_it_in_every_row_unless_it_holds_more() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game()
        .npc("wolf", 10.0, json!([]))
        .npc("bear", 10.0, json!([]))
        .item(
            "cap",
            json!({ "kind": { "type": "equipment", "slot": "head" } }),
        )
        .item("ore", json!({ "kind": { "type": "resource" } }));
    let mut editor = support::open(game, &scratch);
    let npcs = [path("npc", "wolf"), path("npc", "bear")];
    let items = [path("item", "cap"), path("item", "ore")];
    let items_before: Vec<_> = items
        .iter()
        .map(|key| editor.fields_json(key).expect("the item"))
        .collect();

    let drawn = rows_form(&mut editor, "npc", &npcs, Vec::new());
    let name = drawn.rect("name").expect("the name field");
    let apart = drawn
        .0
        .iter()
        .filter(|(text, _)| text.contains("an edit sets them all"))
        .map(|(_, rect)| *rect)
        .min_by(|a, b| {
            (a.top() - name.top())
                .abs()
                .total_cmp(&(b.top() - name.top()).abs())
        })
        .expect("the name field says the rows hold it apart");
    type_below(&mut editor, "npc", &npcs, apart, "1234");

    let kind = rows_form(&mut editor, "item", &items, Vec::new())
        .0
        .into_iter()
        .find(|(text, _)| text.contains("edit the rows one by one"))
        .map(|(_, rect)| rect)
        .expect("the kind field says to edit each row");
    type_below(&mut editor, "item", &items, kind, "x");

    for key in &npcs {
        let npc = editor
            .read::<NpcDef>(key)
            .expect("a project")
            .unwrap_or_else(|error| panic!("{key} reads as an npc: {error:?}"));
        assert_eq!(npc.name, "1234", "{key}");
    }
    let items_after: Vec<_> = items
        .iter()
        .map(|key| editor.fields_json(key).expect("the item"))
        .collect();
    assert_eq!(items_after, items_before);
}

#[test]
fn the_spreadsheet_names_each_row_by_its_file_and_shows_an_edit_on_the_next_frame() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game().npc("wolf", 10.0, json!([]));
    let mut editor = support::open(game, &scratch);
    let mut frames = Frames::new(egui::vec2(1200.0, 900.0));
    let drawn = frames.run(Vec::new(), |ui| spreadsheet::show(&mut editor, ui, "npc"));
    assert!(drawn.rect("wolf").is_some(), "{drawn:?}");
    let wolf = path("npc", "wolf");
    let mut npc = (*editor
        .read::<NpcDef>(&wolf)
        .expect("a project")
        .expect("the npc reads"))
    .clone();
    npc.name = "Grey wolf".to_owned();

    assert!(editor.write_content("Rename the wolf", &wolf, &npc));

    let drawn = frames.run(Vec::new(), |ui| spreadsheet::show(&mut editor, ui, "npc"));
    assert!(drawn.rect("Grey wolf").is_some(), "{drawn:?}");
    assert!(drawn.rect("wolf").is_some(), "{drawn:?}");
}

#[test]
fn a_spreadsheet_cell_is_edited_from_the_keyboard_alone() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game()
        .npc("wolf", 10.0, json!([]))
        .model("bear", 1.5, json!([]));
    let mut editor = support::open(game, &scratch);
    let wolf = path("npc", "wolf");
    let mut frames = Frames::new(egui::vec2(1200.0, 900.0));
    let mut press = |editor: &mut Editor, events: Vec<egui::Event>| {
        frames.run(events, |ui| spreadsheet::show(editor, ui, "npc"))
    };
    let enter = || key(egui::Key::Enter, egui::Modifiers::NONE);
    let typed = |text: &str| vec![egui::Event::Text(text.to_owned())];
    press(&mut editor, Vec::new());

    let tab = || key(egui::Key::Tab, egui::Modifiers::NONE);
    editor.views.spreadsheet.state_mut("npc").selected = Some((wolf.clone(), "name".to_owned()));
    for events in [
        enter(),
        key(egui::Key::A, egui::Modifiers::COMMAND),
        vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
        typed("Grey wolf"),
        enter(),
        tab(),
        typed("wol"),
        key(egui::Key::Escape, egui::Modifiers::NONE),
        enter(),
        typed("bear"),
        enter(),
        tab(),
        key(egui::Key::A, egui::Modifiers::COMMAND),
        vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
        typed("12"),
        tab(),
    ] {
        press(&mut editor, events);
        press(&mut editor, Vec::new());
    }

    let npc = editor
        .read::<NpcDef>(&wolf)
        .expect("a project")
        .expect("the npc reads");
    let reader = editor.reader().expect("a project");
    assert_eq!(npc.name, "Grey wolf");
    assert_eq!(
        reader.path_of(npc.model),
        Some(path("model", "bear").as_str())
    );
    assert_eq!(npc.stats.get(StatKind::AttackDelay), 12.0);
}

#[test]
fn a_sounds_volume_slides_only_as_far_as_the_editor_lets_it_in_one_undo_step() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(support::game().sfx("chime"), &scratch);
    let chime = path("sfx", "chime");
    let mut frames = Frames::new(egui::vec2(900.0, 700.0));
    let mut show = |editor: &mut Editor, events: Vec<egui::Event>| {
        frames.run(events, |ui| row_form::show_row(editor, ui, "sfx", &chime))
    };
    let drawn = show(&mut editor, Vec::new());
    let volume = drawn
        .0
        .iter()
        .filter(|(text, _)| text == "at random")
        .map(|(_, rect)| *rect)
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .expect("the volume's shapes are drawn");
    let handle = egui::pos2(volume.right() + 60.0, volume.center().y);
    let button = |pressed| egui::Event::PointerButton {
        pos: handle,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let far = egui::pos2(handle.x + 2000.0, handle.y);

    for events in [
        vec![egui::Event::PointerMoved(handle)],
        vec![button(true)],
        vec![egui::Event::PointerMoved(handle + egui::vec2(40.0, 0.0))],
        vec![egui::Event::PointerMoved(far)],
        vec![egui::Event::PointerButton {
            pos: far,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    ] {
        show(&mut editor, events);
    }

    let volume = |editor: &mut Editor| {
        editor
            .read::<SfxDef>(&path("sfx", "chime"))
            .expect("a project")
            .expect("the sound reads")
            .volume
    };
    assert_eq!(volume(&mut editor), SfxScalar::Fixed(2.0));
    editor.undo();
    assert_eq!(volume(&mut editor), SfxScalar::Fixed(1.0));
}

#[test]
fn pasting_into_the_spreadsheet_writes_each_cell_its_column_reads_and_skips_the_rest() {
    let text = "daily\nsometimes".to_owned();
    let shortcut = egui::Event::Key {
        key: egui::Key::V,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    };
    for paste in [
        vec![egui::Event::Paste(text.clone())],
        vec![shortcut, egui::Event::Text(text.clone())],
    ] {
        let scratch = tempfile::tempdir().expect("scratch");
        let game = support::game()
            .memory("bribed", "never")
            .memory("taunted", "never");
        let mut editor = support::open(game, &scratch);
        let (bribed, taunted) = (path("memory", "bribed"), path("memory", "taunted"));
        let mut frames = Frames::new(egui::vec2(1200.0, 900.0));
        frames.run(Vec::new(), |ui| {
            spreadsheet::show(&mut editor, ui, "memory")
        });
        editor.views.spreadsheet.state_mut("memory").selected =
            Some((bribed.clone(), "resets".to_owned()));

        frames.run(paste, |ui| spreadsheet::show(&mut editor, ui, "memory"));

        let resets = |editor: &mut Editor, memory: &str| {
            editor
                .read::<MemoryDef>(memory)
                .expect("a project")
                .expect("the memory reads")
                .resets
        };
        assert_eq!(resets(&mut editor, &bribed), Resets::Daily);
        assert_eq!(resets(&mut editor, &taunted), Resets::Never);
    }
}

#[test]
fn a_cells_picker_makes_a_new_file_beside_the_row_and_names_it_in_the_cell() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game().npc("orc", 10.0, json!([]));
    let mut editor = support::open(game, &scratch);
    let orc = path("npc", "orc");
    let mut frames = Frames::new(egui::vec2(1400.0, 900.0));
    let mut sheet = |editor: &mut Editor, events: Vec<egui::Event>| {
        frames.run(events, |ui| spreadsheet::show(editor, ui, "npc"))
    };
    sheet(&mut editor, Vec::new());
    editor.views.spreadsheet.state_mut("npc").selected = Some((orc.clone(), "model".to_owned()));
    sheet(&mut editor, key(egui::Key::Enter, egui::Modifiers::NONE));
    sheet(&mut editor, Vec::new());
    let drawn = sheet(&mut editor, Vec::new());
    let new = drawn.at("New model beside it");
    for events in click(new) {
        sheet(&mut editor, events);
    }

    let npc = editor
        .read::<NpcDef>(&orc)
        .expect("a project")
        .expect("the npc reads");
    let reader = editor.reader().expect("a project");
    assert_eq!(
        reader.path_of(npc.model),
        Some(path("model", "model").as_str())
    );
}

#[test]
fn a_script_module_added_to_a_row_runs_a_script_of_its_kind_made_for_it_when_none_is_there() {
    let skill_script = || json!({ "used": { "type": "on", "event": "used" } });
    let asking_a_value = json!({
        "runs_on": "item",
        "variables": { "strength": { "type": "number" } },
        "nodes": {},
    });
    for (skill_scripts, item_scripts, named) in [
        (vec![], vec![], path("script", "potion")),
        (vec!["strike_used"], vec![], path("script", "potion")),
        (
            vec!["strike_used"],
            vec!["potion_used"],
            path("script", "potion_used"),
        ),
        (
            vec!["strike_used"],
            vec!["a_brew", "potion_used"],
            path("script", "potion_used"),
        ),
        (
            vec!["strike_used"],
            vec!["a_brew"],
            path("script", "a_brew"),
        ),
    ] {
        let scratch = tempfile::tempdir().expect("scratch");
        let mut game = support::game().item("potion", json!({ "kind": { "type": "resource" } }));
        for name in &skill_scripts {
            game = game.script(name, "skill", skill_script());
        }
        for name in &item_scripts {
            game = match *name {
                "a_brew" => game.row("script", name, asking_a_value.clone()),
                _ => game.script(name, "item", json!({})),
            };
        }
        let mut editor = support::open(game, &scratch);
        let potion = path("item", "potion");
        let mut form = RowFrames::new("item", &potion);
        form.run(&mut editor, Vec::new());
        let add = form.run(&mut editor, Vec::new()).at("+ Add module");
        for events in click(add) {
            form.run(&mut editor, events);
        }
        let script = form.run(&mut editor, Vec::new()).at("script");
        for events in click(script) {
            form.run(&mut editor, events);
        }
        form.run(&mut editor, Vec::new());

        let item = editor
            .read::<ItemDef>(&potion)
            .expect("a project")
            .expect("the item reads");
        let reader = editor.reader().expect("a project");
        let scripts: Vec<&str> = item
            .modules
            .iter()
            .filter_map(|module| match module {
                ItemModule::Script(module) => reader.path_of(module.script),
                _ => None,
            })
            .collect();
        assert_eq!(
            scripts,
            [named.as_str()],
            "{skill_scripts:?} {item_scripts:?}"
        );
        let made = editor
            .read::<ScriptDef>(&named)
            .expect("a project")
            .expect("the script reads");
        assert_eq!(made.runs_on, RunnerKind::Item);
        let module = item.modules.iter().find_map(|module| match module {
            ItemModule::Script(module) => Some(module),
            _ => None,
        });
        let asked: Vec<&String> = made
            .variables
            .iter()
            .filter(|(_, variable)| variable.default.is_none())
            .map(|(name, _)| name)
            .collect();
        assert!(
            module.is_some_and(|module| asked.iter().all(|name| module.values.contains_key(*name))),
            "{named} starts with every value it asks for"
        );
    }

    for (table, kind) in [
        ("npc", RunnerKind::Npc),
        ("item", RunnerKind::Item),
        ("status_effect", RunnerKind::StatusEffect),
        ("quest", RunnerKind::Quest),
        ("shop", RunnerKind::Shop),
    ] {
        let scratch = tempfile::tempdir().expect("scratch");
        let game = support::game()
            .file("icons/cap.png", testkit::png(32, 32))
            .file("art/cap_card.png", testkit::png(256, 180))
            .prop("drop", json!({ "type": "none" }))
            .npc("orc", 10.0, json!([]));
        let mut editor = support::open(game, &scratch);
        let row = editor::rows::new_file(&mut editor, "", table)
            .unwrap_or_else(|why| panic!("a fresh {table}: {why}"));
        support::files(&mut editor);

        let made = editor::rows::new_script_module(&mut editor, table, &row)
            .unwrap_or_else(|why| panic!("a fresh {table} takes a script: {why}"));
        support::files(&mut editor);

        let taken_back = editor
            .project
            .as_mut()
            .and_then(|project| project.take_refused());
        assert!(
            taken_back.is_none(),
            "a fresh {table} holds with its script: {:?}",
            taken_back.map(|taken| taken.refusal.sentence())
        );
        let script = editor
            .read::<ScriptDef>(&made)
            .expect("a project")
            .expect("the script reads");
        assert_eq!(script.runs_on, kind, "the script made for a fresh {table}");
    }
}

#[test]
fn a_module_typed_into_the_add_module_filter_is_added_by_return() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game().item("potion", json!({ "kind": { "type": "resource" } }));
    let mut editor = support::open(game, &scratch);
    let potion = path("item", "potion");
    let mut form = RowFrames::new("item", &potion);
    form.run(&mut editor, Vec::new());
    let add = form.run(&mut editor, Vec::new()).at("+ Add module");
    for events in click(add) {
        form.run(&mut editor, events);
    }
    form.run(&mut editor, vec![egui::Event::Text("bound".to_owned())]);
    form.run(&mut editor, key(egui::Key::Enter, egui::Modifiers::NONE));
    form.run(&mut editor, Vec::new());

    let item = editor
        .read::<ItemDef>(&potion)
        .expect("a project")
        .expect("the item reads");
    assert!(
        matches!(item.modules.as_slice(), [ItemModule::Bound]),
        "{} modules",
        item.modules.len()
    );
}

#[test]
fn a_map_keyed_by_files_shows_each_key_as_its_file_and_typing_never_renames_one() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(world_with_a_card(), &scratch);
    let world = WorldMap::Game.path();
    let mut form = RowFrames::new("world_map", &world);
    form.run(&mut editor, Vec::new());
    let shown = editor::tabs::shown_path(&support::area()).to_owned();
    let card = form.run(&mut editor, Vec::new()).at(&shown);

    form.type_into(&mut editor, card, "x");

    let map = world_map(&editor);
    let reader = editor.reader().expect("a project");
    let keys: Vec<Option<&str>> = map.cards.keys().map(|area| reader.path_of(*area)).collect();
    assert_eq!(keys, [Some(support::area().as_str())]);
}

#[test]
fn a_whole_number_field_refuses_a_fraction_instead_of_rounding_it() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(world_with_a_card(), &scratch);
    let world = WorldMap::Game.path();
    let mut form = RowFrames::new("world_map", &world);
    form.run(&mut editor, Vec::new());
    let size = form.run(&mut editor, Vec::new()).at("100");

    let drawn = form.type_into(&mut editor, size, "5.5");

    let sizes: Vec<u32> = world_map(&editor)
        .cards
        .values()
        .map(|card| card.size)
        .collect();
    assert_eq!(sizes, [100]);
    assert!(
        drawn.any(|text| text.contains("a whole number")),
        "{drawn:?}"
    );
}

#[test]
fn a_layers_shader_parameter_lands_in_its_file_as_it_was_typed() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game()
        .shader("shake")
        .edit("vfx_shader", "shake", |shader| {
            shader["params"] = json!([{
                "name": "amplitude",
                "kind": "number",
                "default": 0.04,
                "min": 0.0,
                "max": 0.5,
            }]);
        })
        .row(
            "vfx",
            "jolt",
            json!({
                "name": "jolt",
                "layers": [{
                    "type": "shader",
                    "shader": path("vfx_shader", "shake"),
                    "duration": 0.2,
                }],
            }),
        );
    let mut editor = support::open(game, &scratch);
    let jolt = path("vfx", "jolt");
    let mut form = RowFrames::new("vfx", &jolt);
    form.run(&mut editor, Vec::new());
    let amplitude = form.run(&mut editor, Vec::new()).at("0.04");

    form.type_into(&mut editor, amplitude, "0.05");

    let vfx = editor
        .read::<VfxDef>(&jolt)
        .expect("a project")
        .expect("the effect reads");
    let Some(VfxLayer::Shader { params, .. }) = vfx.layers.first() else {
        panic!("a shader layer");
    };
    assert_eq!(
        params.get("amplitude"),
        Some(&ParamFill::Value(ParamValue::Number(0.05)))
    );
}

#[test]
fn an_item_made_usable_answers_used_with_a_script_made_for_it_in_the_same_step() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game().item("potion", json!({ "kind": { "type": "resource" } }));
    let mut editor = support::open(game, &scratch);
    let potion = path("item", "potion");
    let mut form = RowFrames::new("item", &potion);
    form.run(&mut editor, Vec::new());
    let kind = form.run(&mut editor, Vec::new()).at("Resource");
    for events in click(kind) {
        form.run(&mut editor, events);
    }
    let usable = form.run(&mut editor, Vec::new()).at("Usable");

    for events in click(usable) {
        form.run(&mut editor, events);
    }
    form.run(&mut editor, Vec::new());
    support::files(&mut editor);

    let item = editor
        .read::<ItemDef>(&potion)
        .expect("a project")
        .expect("the item reads");
    assert!(matches!(item.kind, ItemKind::Usable));
    let reader = editor.reader().expect("a project");
    let scripts: Vec<&str> = item
        .modules
        .iter()
        .filter_map(|module| match module {
            ItemModule::Script(module) => reader.path_of(module.script),
            _ => None,
        })
        .collect();
    assert_eq!(scripts, [path("script", "potion_used").as_str()]);
    editor.undo();
    let item = editor
        .read::<ItemDef>(&potion)
        .expect("a project")
        .expect("the item reads");
    assert!(
        matches!(item.kind, ItemKind::Resource),
        "one undo takes both back"
    );
}

#[test]
fn a_script_module_pointed_at_a_script_asking_for_values_starts_them_filled_in() {
    let walled_corner = Map::of(&[
        "##......", "#.......", "........", "........", "........", "........", "........",
        "........",
    ]);
    for (variable, kind, effect) in [
        (
            "strength",
            "number",
            json!({ "type": "heal", "by": { "var": "strength" } }),
        ),
        (
            "to",
            "place",
            json!({ "type": "warp", "to": { "var": "to" } }),
        ),
    ] {
        let scratch = tempfile::tempdir().expect("scratch");
        let asking = json!({
            "runs_on": "item",
            "variables": { variable: { "type": kind } },
            "nodes": {
                "used": { "type": "on", "event": "used", "next": "effect" },
                "effect": effect,
            },
        });
        let game = Game::new()
            .area(support::AREA, &walled_corner, json!([]))
            .hero(support::AREA, 4.0, 4.0)
            .engine_rows()
            .whole()
            .script(
                "potion_used",
                "item",
                json!({ "used": { "type": "on", "event": "used" } }),
            )
            .row("script", "brew", asking)
            .item(
                "potion",
                json!({
                    "kind": { "type": "usable" },
                    "modules": [{ "type": "script", "script": path("script", "potion_used") }],
                }),
            );
        let mut editor = support::open(game, &scratch);
        let potion = path("item", "potion");
        let mut form = RowFrames::new("item", &potion);
        form.run(&mut editor, Vec::new());
        let card = form
            .run(&mut editor, Vec::new())
            .at("POTION_USED.SCRIPT.JSON");
        for events in click(card) {
            form.run(&mut editor, events);
        }
        let chip = form.run(&mut editor, Vec::new()).at("potion_used.script");
        for events in click(chip) {
            form.run(&mut editor, events);
        }
        let brew = form.run(&mut editor, Vec::new()).at("brew.script");

        for events in click(brew) {
            form.run(&mut editor, events);
        }
        form.run(&mut editor, Vec::new());
        support::files(&mut editor);

        let item = editor
            .read::<ItemDef>(&potion)
            .expect("a project")
            .expect("the item reads");
        let reader = editor.reader().expect("a project");
        let modules: Vec<(&str, Vec<&String>)> = item
            .modules
            .iter()
            .filter_map(|module| match module {
                ItemModule::Script(module) => Some((
                    reader.path_of(module.script)?,
                    module.values.keys().collect(),
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            modules,
            [(path("script", "brew").as_str(), vec![&variable.to_owned()])],
            "a {kind}"
        );
    }
}

#[test]
fn a_place_moved_to_another_area_lands_on_walkable_ground_there() {
    let scratch = tempfile::tempdir().expect("scratch");
    let walled_middle = Map::of(&[
        "........", "........", "........", "........", "....#...", "........", "........",
        "........",
    ]);
    let warp = json!({
        "runs_on": "item",
        "variables": { "to": { "type": "place" } },
        "nodes": {
            "used": { "type": "on", "event": "used", "next": "warp" },
            "warp": { "type": "warp", "to": { "var": "to" } },
        },
    });
    let game = support::game()
        .area("hall", &walled_middle, json!([]))
        .row("script", "warp", warp)
        .item(
            "stone",
            json!({
                "kind": { "type": "usable" },
                "modules": [{
                    "type": "script",
                    "script": path("script", "warp"),
                    "values": { "to": { "area": support::area(), "x": 4.0, "y": 4.0 } },
                }],
            }),
        );
    let mut editor = support::open(game, &scratch);
    let stone = path("item", "stone");
    let mut form = RowFrames::new("item", &stone);
    form.run(&mut editor, Vec::new());
    let card = form
        .run(&mut editor, Vec::new())
        .at("WARP.SCRIPT.JSON · TO START.AREA 4, 4");
    for events in click(card) {
        form.run(&mut editor, events);
    }
    let chip = form.run(&mut editor, Vec::new()).at("start.area");
    for events in click(chip) {
        form.run(&mut editor, events);
    }
    let hall = form.run(&mut editor, Vec::new()).at("hall.area");

    for events in click(hall) {
        form.run(&mut editor, events);
    }
    form.run(&mut editor, Vec::new());
    support::files(&mut editor);

    let refused = editor
        .project
        .as_mut()
        .and_then(|project| project.take_refused());
    assert!(
        refused.is_none(),
        "{:?}",
        refused.map(|taken| taken.refusal.sentence())
    );
    let item = editor
        .read::<ItemDef>(&stone)
        .expect("a project")
        .expect("the item reads");
    let reader = editor.reader().expect("a project");
    let place = item.modules.iter().find_map(|module| match module {
        ItemModule::Script(module) => match module.values.get("to") {
            Some(VarValue::Place(place)) => Some(place),
            _ => None,
        },
        _ => None,
    });
    assert!(
        place.is_some_and(|place| reader.path_of(place.area)
            == Some(path("area", "hall").as_str())
            && (place.at.x, place.at.y) != (4.0, 4.0)),
        "the place moved into the hall, off its wall"
    );
}

#[test]
fn a_skills_target_and_area_change_together_from_either_side() {
    for (click_first, then) in [("Single enemy", Some("Area enemy")), ("+ area", None)] {
        let scratch = tempfile::tempdir().expect("scratch");
        let game = support::game()
            .file("icons/cap.png", testkit::png(16, 16))
            .row(
                "skill",
                "lunge",
                json!({
                    "name": "Lunge",
                    "icon": "icons/cap.png",
                    "target": "single_enemy",
                    "levels": [{ "cost": 1, "stats": {} }],
                    "modules": [{ "type": "script", "script": path("script", "lunge_used") }],
                }),
            )
            .script(
                "lunge_used",
                "skill",
                json!({ "used": { "type": "on", "event": "used" } }),
            );
        let mut editor = support::open(game, &scratch);
        let lunge = path("skill", "lunge");
        let mut form = RowFrames::new("skill", &lunge);
        form.run(&mut editor, Vec::new());
        let first = form.run(&mut editor, Vec::new()).at(click_first);
        for events in click(first) {
            form.run(&mut editor, events);
        }
        if let Some(then) = then {
            let option = form.run(&mut editor, Vec::new()).at(then);
            for events in click(option) {
                form.run(&mut editor, events);
            }
        }
        form.run(&mut editor, Vec::new());
        support::files(&mut editor);

        let refused = editor
            .project
            .as_mut()
            .and_then(|project| project.take_refused());
        assert!(
            refused.is_none(),
            "{click_first}: {:?}",
            refused.map(|t| t.refusal.sentence())
        );
        let skill = editor
            .read::<SkillDef>(&lunge)
            .expect("a project")
            .expect("the skill reads");
        assert_eq!(skill.target, Targeting::AreaEnemy, "{click_first}");
        assert!(skill.area.is_some(), "{click_first}");
    }
}

#[test]
fn the_keys_walk_a_picker_list_past_what_it_shows_and_pick_what_they_reach() {
    let scratch = tempfile::tempdir().expect("scratch");
    let voice = json!([{ "type": "voice", "babble": path("babble", "crier") }]);
    let mut game = support::game().babble("crier");
    for index in 0..30 {
        game = game.npc(&format!("npc_{index:02}"), 10.0, voice.clone());
    }
    let game = game.row(
        "notification",
        "shout",
        json!({ "kind": { "type": "speech", "speaker": path("npc", "npc_00") }, "text": [] }),
    );
    let mut editor = support::open(game, &scratch);
    let mut form = RowFrames::new("notification", &path("notification", "shout"));
    form.run(&mut editor, Vec::new());
    let chip = form.run(&mut editor, Vec::new()).at("npc_00.npc");
    for events in click(chip) {
        form.run(&mut editor, events);
    }

    let mut drawn = Drawn::default();
    for events in std::iter::repeat_n(key(egui::Key::ArrowDown, egui::Modifiers::NONE), 20)
        .chain([Vec::new(), Vec::new()])
    {
        drawn = form.run(&mut editor, events);
    }
    form.run(&mut editor, key(egui::Key::Enter, egui::Modifiers::NONE));

    assert!(drawn.rect("npc_20.npc").is_some(), "{drawn:?}");
    let shout = editor
        .read::<NotificationDef>(&path("notification", "shout"))
        .expect("a project")
        .expect("the notification reads");
    let NotificationDefKind::Speech { speaker } = &shout.kind else {
        panic!("a speech");
    };
    let reader = editor.reader().expect("a project");
    assert_eq!(
        reader.path_of(*speaker),
        Some(path("npc", "npc_20").as_str())
    );
}

fn world_with_a_card() -> Game {
    support::game().edit("world_map", WorldMap::Game.name(), |map| {
        map["cards"] = json!({ support::area(): { "at": { "x": 0.0, "y": 0.0 }, "size": 100 } });
    })
}

fn world_map(editor: &Editor) -> Arc<WorldMapDef> {
    editor
        .read::<WorldMapDef>(&WorldMap::Game.path())
        .expect("a project")
        .expect("the world map reads")
}

/// The form of one row drawn frame after frame, as the editor draws it.
struct RowFrames {
    frames: Frames,
    table: String,
    key: String,
}

impl RowFrames {
    fn new(table: &str, key: &str) -> RowFrames {
        RowFrames {
            frames: Frames::new(egui::vec2(900.0, 1200.0)),
            table: table.to_owned(),
            key: key.to_owned(),
        }
    }

    fn run(&mut self, editor: &mut Editor, events: Vec<egui::Event>) -> Drawn {
        self.frames.run(events, |ui| {
            row_form::show_row(editor, ui, &self.table, &self.key)
        })
    }

    /// Clicks at `at`, types `text` over what is there and presses Enter, giving what the form
    /// draws once that settles.
    fn type_into(&mut self, editor: &mut Editor, at: egui::Pos2, text: &str) -> Drawn {
        for events in click(at).into_iter().chain([
            key(egui::Key::A, egui::Modifiers::COMMAND),
            vec![egui::Event::Text(text.to_owned())],
            key(egui::Key::Enter, egui::Modifiers::NONE),
            Vec::new(),
        ]) {
            self.run(editor, events);
        }
        self.run(editor, Vec::new())
    }
}

/// What the multi-row form of `keys` draws once a frame of `events` settles.
fn rows_form(editor: &mut Editor, table: &str, keys: &[String], events: Vec<egui::Event>) -> Drawn {
    let mut frames = Frames::new(egui::vec2(1200.0, 900.0));
    let mut drawn = Drawn::default();
    for events in [Vec::new(), Vec::new(), events, Vec::new(), Vec::new()] {
        drawn = frames.run(events, |ui| {
            row_form::show_rows_tab(editor, ui, table, keys)
        });
    }
    drawn
}

/// Clicks just below `label` in the multi-row form of `keys`, into the field it labels, types
/// `text` and presses Enter.
fn type_below(editor: &mut Editor, table: &str, keys: &[String], label: egui::Rect, text: &str) {
    let field = egui::pos2(label.left() + 20.0, label.bottom() + 12.0);
    let mut frames = Frames::new(egui::vec2(1200.0, 900.0));
    for events in [Vec::new(), Vec::new()]
        .into_iter()
        .chain(click(field))
        .chain([
            vec![egui::Event::Text(text.to_owned())],
            key(egui::Key::Enter, egui::Modifiers::NONE),
            Vec::new(),
            Vec::new(),
        ])
    {
        frames.run(events, |ui| {
            row_form::show_rows_tab(editor, ui, table, keys)
        });
    }
}

#[test]
fn a_skill_requirement_added_to_a_tree_node_starts_at_a_node_it_may_require() {
    let scratch = tempfile::tempdir().expect("scratch");
    let passive = |name: &str| {
        json!({
            "name": name,
            "icon": "icons/cap.png",
            "kind": "passive",
            "target": "self",
            "levels": [{ "cost": 1, "stats": {} }],
        })
    };
    let game = support::game()
        .file("icons/cap.png", testkit::png(16, 16))
        .row("skill", "guard", passive("Guard"))
        .row("skill", "brace", passive("Brace"))
        .row(
            "class",
            "hero",
            json!({
                "name": "Hero",
                "icon": "icons/hero.png",
                "levels": [{ "xp": 0, "skill_points": 1 }],
                "tree": {
                    "guard": { "skill": path("skill", "guard"), "at": { "x": 0.0, "y": 0.0 } },
                    "brace": { "skill": path("skill", "brace"), "at": { "x": 0.0, "y": 100.0 } },
                },
            }),
        );
    let mut editor = support::open(game, &scratch);
    let class = path("class", "hero");
    let selection = RowSelection::parts("class", &class, vec![format!("{TREE_PART}brace")]);
    let mut frames = Frames::new(egui::vec2(1600.0, 1000.0));
    let mut inspect = |editor: &mut Editor, events: Vec<egui::Event>| {
        frames.run(events, |ui| class_inspect::show(editor, ui, &selection))
    };
    inspect(&mut editor, Vec::new());
    let add = inspect(&mut editor, Vec::new()).at("+ add requirement");
    for events in click(add) {
        inspect(&mut editor, events);
    }
    let skill = inspect(&mut editor, Vec::new()).at("Skill");
    for events in click(skill) {
        inspect(&mut editor, events);
    }
    support::files(&mut editor);

    let refused = editor
        .project
        .as_mut()
        .and_then(|project| project.take_refused());
    assert!(
        refused.is_none(),
        "{:?}",
        refused.map(|taken| taken.refusal.sentence())
    );
    let hero = editor
        .read::<ClassDef>(&class)
        .expect("a project")
        .expect("the class reads");
    let brace = &hero.tree.0[&NodeKey("brace".to_owned())];
    assert!(
        matches!(brace.requires.as_slice(), [AccessRequirement::Skill { node, .. }] if node.as_str() == "guard"),
        "brace requires guard"
    );
}
