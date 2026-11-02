use std::sync::Arc;
use ui::egui;

use editor::app::{BottomTab, Editor};
use editor::project::RowSelection;
use editor::tabs::EditorTab;
use editor::views::script::{self, ScriptDoc, ScriptRow};
use editor::views::spreadsheet;
use engine::core::script::node::NEXT_EXIT;
use engine::core::script::{Fill, Node, Target, VariableType};
use engine::systems::combat::effects::HealBy;
use engine::systems::npc::NpcDef;
use engine::systems::script::{RiftEffect, RunnerKind, ScriptDef};
use serde_json::json;
use testkit::path;

use crate::support::{self, Drawn, Frames, click, key};

const SCREEN: egui::Vec2 = egui::vec2(1600.0, 1000.0);

#[test]
fn deleting_several_selected_nodes_deletes_every_one_of_them() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = open(support::game(), &scratch);
    let at = talk();
    let mut frames = ScriptFrames::new();
    frames.run(&mut editor, &at, Vec::new());
    select(&mut editor, &at, &["ask", "answer"]);

    frames.run(
        &mut editor,
        &at,
        key(egui::Key::Delete, egui::Modifiers::NONE),
    );

    let names: Vec<String> = read(&editor).nodes.keys().cloned().collect();
    assert_eq!(names, ["start"]);
}

#[test]
fn a_closed_script_view_leaves_its_keys_to_the_view_shown_where_it_was() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = open(support::game().npc("wolf", 10.0, json!([])), &scratch);
    let at = talk();
    let mut frames = Frames::new(SCREEN);
    let over = || vec![egui::Event::PointerMoved(egui::pos2(800.0, 400.0))];
    editor.open_tab(EditorTab::Script(at.clone()));
    editor.place_tabs();
    frames.run(over(), |ui| {
        editor::shell::shortcuts(&mut editor, ui.ctx());
        script::show(&mut editor, ui, &at);
    });
    let tab = editor
        .dock
        .find_tab(&EditorTab::Script(at.clone()))
        .expect("the script's tab");
    editor.dock.remove_tab(tab);

    let wolf = path("npc", "wolf");
    let mut sheet = |editor: &mut Editor, events: Vec<egui::Event>| {
        frames.run(events, |ui| {
            editor::shell::shortcuts(editor, ui.ctx());
            spreadsheet::show(editor, ui, "npc");
        });
    };
    sheet(&mut editor, over());
    editor.views.spreadsheet.view_or_default("npc").selected =
        Some((wolf.clone(), "name".to_owned()));
    for events in [
        key(egui::Key::Enter, egui::Modifiers::NONE),
        key(egui::Key::A, egui::Modifiers::COMMAND),
        vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
        vec![egui::Event::Text("Grey wolf".to_owned())],
        key(egui::Key::Enter, egui::Modifiers::NONE),
    ] {
        sheet(&mut editor, events);
        sheet(&mut editor, Vec::new());
    }

    let npc = editor
        .read::<NpcDef>(&wolf)
        .expect("a project")
        .expect("the npc reads");
    assert_eq!(npc.name, "Grey wolf");
}

#[test]
fn duplicating_several_selected_nodes_copies_every_one_of_them() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = open(support::game(), &scratch);
    let at = talk();
    let mut frames = ScriptFrames::new();
    frames.run(&mut editor, &at, Vec::new());
    select(&mut editor, &at, &["ask", "answer"]);

    frames.run(
        &mut editor,
        &at,
        key(egui::Key::D, egui::Modifiers::COMMAND),
    );

    let script = read(&editor);
    let saying = |said: &str| {
        script
            .nodes
            .values()
            .filter(|node| matches!(node, Node::Dialogue(dialogue) if dialogue.lines.iter().any(|line| editor::spans::plain(&line.text) == said)))
            .count()
    };
    assert_eq!((saying("Well?"), saying("Fine.")), (2, 2));
}

#[test]
fn enter_on_a_line_keeps_what_was_typed_into_it_and_adds_one_after() {
    for (caret, typed, lines) in [
        (egui::Key::End, " Speak up.", ["Well? Speak up.", ""]),
        (egui::Key::Home, "", ["Well?", ""]),
    ] {
        let scratch = tempfile::tempdir().expect("scratch");
        let mut editor = open(support::game(), &scratch);
        let at = talk();
        let mut frames = ScriptFrames::new();
        frames.run(&mut editor, &at, Vec::new());
        select(&mut editor, &at, &["ask"]);
        let line = frames
            .run(&mut editor, &at, Vec::new())
            .0
            .into_iter()
            .filter(|(text, _)| text == "Well?")
            .map(|(_, rect)| rect)
            .min_by(|a, b| a.left().total_cmp(&b.left()))
            .expect("the line is drawn on its card");

        for events in click(line.center())
            .into_iter()
            .chain([key(caret, egui::Modifiers::NONE)])
            .chain((!typed.is_empty()).then(|| vec![egui::Event::Text(typed.to_owned())]))
            .chain([key(egui::Key::Enter, egui::Modifiers::NONE)])
        {
            frames.run(&mut editor, &at, events);
        }

        let Some(Node::Dialogue(ask)) = read(&editor).nodes.get("ask").cloned() else {
            panic!("ask is a dialogue");
        };
        let shown: Vec<String> = ask
            .lines
            .iter()
            .map(|line| editor::spans::plain(&line.text))
            .collect();
        assert_eq!(shown, lines, "{caret:?}");
    }
}

#[test]
fn a_script_lists_every_module_running_it_with_the_values_it_gives() {
    let scratch = tempfile::tempdir().expect("scratch");
    let counted = |runs_on: &str, on: serde_json::Value| {
        json!({
            "runs_on": runs_on,
            "variables": { "times": { "type": "integer", "default": 1 } },
            "nodes": {
                "on": on,
                "check": {
                    "type": "if",
                    "test": { "type": "at_least", "a": { "var": "times" }, "b": 4 },
                    "yes": "rest",
                    "no": "rest",
                },
                "rest": { "type": "wait", "seconds": 0, "at_most": 1.0, "next": "rest" },
            },
        })
    };
    let game = support::game()
        .row(
            "script",
            "chime",
            counted("area", json!({ "type": "on", "event": "discovered", "next": "check" })),
        )
        .row(
            "script",
            "door",
            counted(
                "object",
                json!({ "type": "on", "event": "entered", "by": "characters", "next": "check" }),
            ),
        )
        .edit_scene("area", support::AREA, |scene| {
            let scene = scene.as_array_mut().expect("a scene");
            scene.push(json!({ "type": "script", "label": "Bells", "script": path("script", "chime"), "values": { "times": 5 } }));
            scene.push(json!({
                "type": "object",
                "name": "door",
                "shape": { "type": "rect", "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0 },
                "scripts": [{ "script": path("script", "door"), "values": { "times": 2 } }],
            }));
        });
    let mut editor = open(game, &scratch);

    for (script, given) in [("chime", "times 5"), ("door", "times 2")] {
        let at = ScriptRow::new(path("script", script));
        let mut frames = Frames::new(SCREEN);
        let drawn = frames.run(Vec::new(), |ui| {
            script::bottom(&mut editor, ui, &at, BottomTab::UsedBy)
        });
        assert!(drawn.any(|text| text == given), "{script}: {drawn:?}");
    }
}

#[test]
fn a_condition_inside_a_query_is_shown_as_a_condition_within_the_screen() {
    let watch = json!({
        "runs_on": "npc",
        "locals": { "target": "actor" },
        "nodes": {
            "start": { "type": "on", "event": "started", "next": "found" },
            "found": {
                "type": "if",
                "test": { "type": "present", "actor": { "type": "local", "local": "target" } },
                "yes": "rest",
                "no": "rest",
            },
            "rest": { "type": "wait", "seconds": 0, "at_most": 1.0, "next": "rest" },
        },
    });
    let steel = json!({
        "runs_on": "status_effect",
        "nodes": {
            "start": { "type": "on", "event": "activated", "next": "harden" },
            "harden": {
                "type": "boost",
                "stat": "max_health",
                "by": {
                    "type": "multiply",
                    "a": { "type": "status_power" },
                    "b": { "type": "add", "a": { "type": "status_power" }, "b": 12345 },
                },
            },
        },
    });
    for (script, nodes, node, last) in [
        ("watch", watch, "found", "target"),
        ("steel", steel, "harden", "12345"),
    ] {
        let scratch = tempfile::tempdir().expect("scratch");
        let mut editor = open(support::game().row("script", script, nodes), &scratch);
        let at = ScriptRow::new(path("script", script));
        let mut frames = ScriptFrames::new();
        frames.run(&mut editor, &at, Vec::new());
        select(&mut editor, &at, &[node]);

        let drawn = frames.run(&mut editor, &at, Vec::new());

        let shown = drawn
            .rect(last)
            .unwrap_or_else(|| panic!("{script}: {last} is drawn: {drawn:?}"));
        assert!(
            shown.right() <= SCREEN.x,
            "{script}: {last} fits on screen: {shown:?}"
        );
        assert!(!drawn.any(|text| text.contains("\"type\"")), "{drawn:?}");
    }
}

#[test]
fn a_variable_given_another_type_drops_the_default_it_had_as_the_old_one() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game().row(
        "script",
        "tally",
        json!({
            "runs_on": "npc",
            "variables": { "count": { "type": "integer", "default": 3 } },
            "nodes": {
                "start": { "type": "on", "event": "started", "next": "rest" },
                "rest": { "type": "wait", "seconds": 0, "at_most": 1.0, "next": "rest" },
            },
        }),
    );
    let mut editor = open(game, &scratch);
    let at = ScriptRow::new(path("script", "tally"));
    let mut frames = ScriptFrames::new();
    frames.run(&mut editor, &at, Vec::new());
    for chip in ["Integer", "Text"] {
        let at_chip = frames.run(&mut editor, &at, Vec::new()).at(chip);
        for events in click(at_chip) {
            frames.run(&mut editor, &at, events);
        }
    }

    let script = editor
        .read::<ScriptDef>(&at.key)
        .expect("a project")
        .expect("the script reads");
    let count = &script.variables["count"];
    assert_eq!(count.ty, VariableType::Text);
    assert!(count.default.is_none());
}

#[test]
fn every_node_a_script_offers_lands_after_its_event_or_is_refused_for_what_it_is() {
    let skill = json!({
        "name": "Lunge",
        "icon": "icons/cap.png",
        "target": "single_enemy",
        "levels": [{ "cost": 1, "stats": {} }],
        "modules": [{ "type": "script", "script": path("script", "lunge_used") }],
    });
    let wolf = json!([{ "type": "script", "script": path("script", "wolf_hunting") }]);
    let mut refused = Vec::new();
    for (name, kind) in [
        ("lunge_used", RunnerKind::Skill),
        ("wolf_hunting", RunnerKind::Npc),
    ] {
        let scratch = tempfile::tempdir().expect("scratch");
        let game = support::game()
            .file("icons/cap.png", testkit::png(16, 16))
            .sfx("swish")
            .vfx("spark")
            .row("skill", "lunge", skill.clone())
            .npc("wolf", 10.0, wolf.clone())
            .script(
                "lunge_used",
                "skill",
                json!({ "used": { "type": "on", "event": "used" } }),
            )
            .script(
                "wolf_hunting",
                "npc",
                json!({ "used": { "type": "on", "event": "started" } }),
            );
        let mut editor = support::open(game, &scratch);
        let at = ScriptRow::new(path("script", name));
        let offered: Vec<String> = editor
            .project
            .as_ref()
            .expect("a project")
            .catalogue
            .entries
            .iter()
            .filter(|entry| entry.fits(true))
            .filter(|entry| entry.owners.is_empty() || entry.owners.contains(&kind))
            .map(|entry| entry.kind.clone())
            .collect();

        for node in &offered {
            let doc = ScriptDoc::load(&editor, &at).expect("the script");
            editor.toast = None;
            script::add_node(
                &mut editor,
                &doc,
                node,
                None,
                Some(("used".to_owned(), NEXT_EXIT)),
            );
            if editor.toast.is_some() {
                continue;
            }
            support::files(&mut editor);
            if let Some(taken_back) = editor.project.as_mut().and_then(|p| p.take_refused()) {
                let refusal = &taken_back.refusal;
                let at = refusal.pointer.trim_start_matches("/nodes/");
                let its_own =
                    at != "used" && refusal.problem.found.starts_with(&format!("{at} in "));
                if !its_own {
                    refused.push(format!("{name} {node}: {}", refusal.sentence()));
                }
                continue;
            }
            let script = editor
                .read::<ScriptDef>(&at.key)
                .expect("a project")
                .expect("the script reads");
            if !matches!(&script.nodes["used"], Node::On { next, .. } if next.to().is_some()) {
                refused.push(format!("{name} {node}: not written"));
            }
            editor.undo();
        }
    }
    assert!(
        refused.is_empty(),
        "refused for another's fault: {refused:#?}"
    );
}

#[test]
fn a_node_field_made_a_variable_gives_the_script_that_variable_and_the_field_its_name() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game().row(
        "script",
        "door",
        json!({
            "runs_on": "object",
            "nodes": {
                "entered": { "type": "on", "event": "entered", "by": "characters", "next": "warp" },
                "warp": { "type": "warp", "to": { "area": path("area", support::AREA), "x": 1.0, "y": 1.0 } },
            },
        }),
    );
    let mut editor = support::open(game, &scratch);
    let at = ScriptRow::new(path("script", "door"));
    let selection = RowSelection::parts("script", &at.key, vec!["node:warp".to_owned()]);
    let mut frames = Frames::new(SCREEN);
    let mut inspect = |editor: &mut Editor, events: Vec<egui::Event>| {
        frames.run(events, |ui| {
            let doc = ScriptDoc::load(editor, &at).expect("the script");
            script::inspect::show(editor, ui, &doc, &selection);
        })
    };
    let at_text = |drawn: &Drawn, text: &str| {
        drawn
            .0
            .iter()
            .find(|(shown, _)| shown == text)
            .map(|(_, rect)| rect.center())
            .unwrap_or_else(|| panic!("{text} is drawn: {drawn:?}"))
    };

    let drawn = inspect(&mut editor, Vec::new());
    let hole = at_text(&drawn, "◇");
    for events in click(hole) {
        inspect(&mut editor, events);
    }
    let drawn = inspect(&mut editor, Vec::new());
    let make = at_text(&drawn, "Make variable");
    for events in click(make) {
        inspect(&mut editor, events);
    }

    let script = editor
        .read::<ScriptDef>(&at.key)
        .expect("a project")
        .expect("the script reads");
    assert_eq!(script.variables["to"].ty, VariableType::Place);
    let Node::Effect(RiftEffect::Warp(warp)) = &script.nodes["warp"] else {
        panic!("warp is a warp");
    };
    assert!(matches!(&warp.to, Fill::Var { var } if var == "to"));
}

#[test]
fn a_node_added_to_an_empty_script_shows_on_its_canvas() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game().row("script", "blank", json!({ "runs_on": "item", "nodes": {} }));
    let mut editor = support::open(game, &scratch);
    let at = ScriptRow::new(path("script", "blank"));
    let mut frames = ScriptFrames::new();
    let over_the_canvas = vec![egui::Event::PointerMoved(egui::pos2(800.0, 500.0))];
    frames.run(&mut editor, &at, over_the_canvas.clone());

    let tab = [over_the_canvas, key(egui::Key::Tab, egui::Modifiers::NONE)].concat();
    frames.run(&mut editor, &at, tab);
    frames.run(&mut editor, &at, vec![egui::Event::Text("on".into())]);
    frames.run(
        &mut editor,
        &at,
        key(egui::Key::Enter, egui::Modifiers::NONE),
    );
    support::files(&mut editor);
    let drawn = frames.run(&mut editor, &at, Vec::new());

    let rect = |text: &str| {
        drawn
            .0
            .iter()
            .find(|(shown, _)| shown.starts_with(text))
            .map(|(_, rect)| *rect)
            .unwrap_or_else(|| panic!("{text} is drawn: {drawn:?}"))
    };
    let canvas = egui::Rect::from_min_max(
        rect("blank.script · runs on").min,
        egui::pos2(rect("Inspector").min.x, SCREEN.y),
    );
    let card = rect("next · end");
    assert!(
        canvas.contains_rect(card),
        "{card:?} lies on the canvas {canvas:?}"
    );
}

#[test]
fn the_canvas_and_outline_chips_switch_the_view_by_a_click() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = open(support::game(), &scratch);
    let at = talk();
    let mut frames = ScriptFrames::new();
    let outlined = |drawn: &Drawn| drawn.0.iter().any(|(text, _)| text == " → ");
    let mut drawn = frames.run(&mut editor, &at, Vec::new());
    assert!(!outlined(&drawn), "a script opens on its canvas");

    for (chip, outline) in [("Outline", true), ("Canvas", false)] {
        let at_chip = drawn
            .0
            .iter()
            .find(|(text, _)| text == chip)
            .map(|(_, rect)| rect.center());
        for events in click(at_chip.unwrap_or_else(|| panic!("the {chip} chip is drawn"))) {
            frames.run(&mut editor, &at, events);
        }
        drawn = frames.run(&mut editor, &at, Vec::new());
        assert_eq!(outlined(&drawn), outline, "{chip} clicked");
    }
}

#[test]
fn a_heal_heals_fully_or_by_an_expression_picked_in_its_inspector() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game()
        .npc(
            "healer",
            10.0,
            json!([{ "type": "script", "script": path("script", "mend") }]),
        )
        .script(
            "mend",
            "npc",
            json!({
                "start": { "type": "on", "event": "interacted", "verb": "talk", "reach": 2.0, "next": "heal" },
                "heal": { "type": "heal", "by": "full" },
            }),
        );
    let mut editor = support::open(game, &scratch);
    let at = ScriptRow::new(path("script", "mend"));
    let selection = RowSelection::parts("script", &at.key, vec!["node:heal".to_owned()]);
    let mut frames = Frames::new(SCREEN);
    let mut inspect = |editor: &mut Editor, events: Vec<egui::Event>| {
        frames.run(events, |ui| {
            let doc = ScriptDoc::load(editor, &at).expect("the script");
            script::inspect::show(editor, ui, &doc, &selection);
        })
    };
    inspect(&mut editor, Vec::new());
    let full = inspect(&mut editor, Vec::new()).at("Full");
    assert!(
        !inspect(&mut editor, Vec::new())
            .0
            .iter()
            .filter(|(text, _)| text == "Full")
            .nth(1)
            .is_some(),
        "Full shows once"
    );
    for events in click(full) {
        inspect(&mut editor, events);
    }
    let expression = inspect(&mut editor, Vec::new()).at("Expression");

    for events in click(expression) {
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
    let script = editor
        .read::<ScriptDef>(&at.key)
        .expect("a project")
        .expect("the script reads");
    let Node::Effect(RiftEffect::Heal(heal)) = &script.nodes["heal"] else {
        panic!("heal is a heal");
    };
    assert!(matches!(heal.by, HealBy::Amount(_)));
}

#[test]
fn a_query_picked_as_an_if_test_starts_filled_in_with_values_of_the_types_it_takes() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game().memory("warned", "never").script(
        "watch",
        "npc",
        json!({
            "start": { "type": "on", "event": "started", "next": "found" },
            "found": { "type": "if", "test": { "type": "in_area", "area": support::area() } },
        }),
    );
    let mut editor = support::open(game, &scratch);
    let at = ScriptRow::new(path("script", "watch"));
    let selection = RowSelection::parts("script", &at.key, vec!["node:found".to_owned()]);
    let mut frames = Frames::new(SCREEN);
    let mut inspect = |editor: &mut Editor, events: Vec<egui::Event>| {
        frames.run(events, |ui| {
            let doc = ScriptDoc::load(editor, &at).expect("the script");
            script::inspect::show(editor, ui, &doc, &selection);
        })
    };

    let mut shown = "in area";
    let mut refused = Vec::new();
    for query in ["set within", "present", "is npc", "walking", "hostile"] {
        inspect(&mut editor, Vec::new());
        let chip = inspect(&mut editor, Vec::new()).at(shown);
        for events in click(chip) {
            inspect(&mut editor, events);
        }
        let mut entry = inspect(&mut editor, Vec::new()).rect(query);
        for _ in 0..20 {
            if entry.is_some() {
                break;
            }
            let wheel = vec![
                egui::Event::PointerMoved(chip + egui::vec2(0.0, 120.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -120.0),
                    modifiers: egui::Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                },
            ];
            inspect(&mut editor, wheel);
            for _ in 0..20 {
                entry = inspect(&mut editor, Vec::new()).rect(query);
            }
        }
        for events in click(entry.expect("the query is offered").center()) {
            inspect(&mut editor, events);
        }
        support::files(&mut editor);

        if let Some(taken) = editor.project.as_mut().and_then(|p| p.take_refused()) {
            refused.push(format!("{query}: {}", taken.refusal.sentence()));
        } else if !inspect(&mut editor, Vec::new()).any(|text| text == query) {
            refused.push(format!("{query}: not picked"));
        } else {
            shown = query;
        }
    }
    assert!(refused.is_empty(), "{refused:#?}");
}

#[test]
fn a_local_declared_in_the_scripts_header_is_what_a_fresh_set_value_writes() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game()
        .npc(
            "wolf",
            10.0,
            json!([{ "type": "script", "script": path("script", "hunt") }]),
        )
        .script(
            "hunt",
            "npc",
            json!({
                "start": { "type": "on", "event": "started", "next": "keep" },
                "keep": { "type": "set", "values": [] },
            }),
        );
    let mut editor = support::open(game, &scratch);
    let at = ScriptRow::new(path("script", "hunt"));
    let mut frames = Frames::new(SCREEN);
    let mut header = |editor: &mut Editor, events: Vec<egui::Event>| {
        frames.run(events, |ui| {
            let doc = ScriptDoc::load(editor, &at).expect("the script");
            script::inspect::script_row(editor, ui, &doc);
        })
    };
    header(&mut editor, Vec::new());
    let locals_key = header(&mut editor, Vec::new()).at("new key");
    for events in click(locals_key) {
        header(&mut editor, events);
    }
    header(&mut editor, vec![egui::Event::Text("prey".to_owned())]);
    let add = header(&mut editor, Vec::new()).at("+ add");
    for events in click(add) {
        header(&mut editor, events);
    }
    support::files(&mut editor);

    let selection = RowSelection::parts("script", &at.key, vec!["node:keep".to_owned()]);
    let mut frames = Frames::new(SCREEN);
    let mut inspect = |editor: &mut Editor, events: Vec<egui::Event>| {
        frames.run(events, |ui| {
            let doc = ScriptDoc::load(editor, &at).expect("the script");
            script::inspect::show(editor, ui, &doc, &selection);
        })
    };
    inspect(&mut editor, Vec::new());
    let value = inspect(&mut editor, Vec::new()).at("+ value");
    for events in click(value) {
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
    let script = editor
        .read::<ScriptDef>(&at.key)
        .expect("a project")
        .expect("the script reads");
    assert!(script.locals.contains_key("prey"), "{:?}", script.locals);
    let Node::Set { values, .. } = &script.nodes["keep"] else {
        panic!("keep is a set");
    };
    assert!(
        matches!(values.as_slice(), [only] if only.target == Target::Local("prey".to_owned())),
        "the set writes prey"
    );
    assert!(inspect(&mut editor, Vec::new()).any(|text| text == "local prey"));
}

#[test]
fn an_optional_amount_added_to_a_node_starts_as_a_number_the_game_takes() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game()
        .npc(
            "crier",
            10.0,
            json!([{ "type": "script", "script": path("script", "call") }]),
        )
        .script(
            "call",
            "npc",
            json!({
                "start": { "type": "on", "event": "started", "next": "shout" },
                "shout": { "type": "say", "text": ["Hear ye"], "next": "roam" },
                "roam": { "type": "wander", "priority": 1 },
            }),
        );
    let mut editor = support::open(game, &scratch);
    let at = ScriptRow::new(path("script", "call"));
    for (node, field) in [("shout", "+ within"), ("roam", "+ leash")] {
        let selection = RowSelection::parts("script", &at.key, vec![format!("node:{node}")]);
        let mut frames = Frames::new(SCREEN);
        let mut inspect = |editor: &mut Editor, events: Vec<egui::Event>| {
            frames.run(events, |ui| {
                let doc = ScriptDoc::load(editor, &at).expect("the script");
                script::inspect::show(editor, ui, &doc, &selection);
            })
        };
        inspect(&mut editor, Vec::new());
        let add = inspect(&mut editor, Vec::new()).at(field);
        for events in click(add) {
            inspect(&mut editor, events);
        }
        support::files(&mut editor);

        let refused = editor
            .project
            .as_mut()
            .and_then(|project| project.take_refused());
        assert!(
            refused.is_none(),
            "{field}: {:?}",
            refused.map(|taken| taken.refusal.sentence())
        );
        assert!(
            !inspect(&mut editor, Vec::new()).any(|text| text == field),
            "{field} is set"
        );
    }
}

fn talk() -> ScriptRow {
    ScriptRow::new(path("script", "talk"))
}

fn select(editor: &mut Editor, at: &ScriptRow, nodes: &[&str]) {
    let doc = ScriptDoc::load(editor, at).expect("the script");
    let nodes: Vec<String> = nodes.iter().map(|node| (*node).to_owned()).collect();
    script::select_nodes(editor, &doc, &nodes);
}

/// `game` with the `talk` script, open in the editor.
fn open(game: testkit::Game, scratch: &tempfile::TempDir) -> Editor {
    let line = |text: &str| json!({ "by": { "type": "narrator" }, "text": [text] });
    let game = game.script(
        "talk",
        "npc",
        json!({
            "start": { "type": "on", "event": "interacted", "verb": "talk", "reach": 2.0, "next": "ask" },
            "ask": { "type": "dialogue", "lines": [line("Well?")], "choices": [{ "label": ["Go on."], "to": "answer" }] },
            "answer": { "type": "dialogue", "lines": [line("Fine.")], "choices": [{ "label": ["Bye."] }] },
        }),
    );
    support::open(game, scratch)
}

fn read(editor: &Editor) -> Arc<ScriptDef> {
    editor
        .read::<ScriptDef>(&path("script", "talk"))
        .expect("a project")
        .expect("the script reads")
}

/// The script view drawn as the editor draws it in its tab, taking the editor's keys.
struct ScriptFrames(Frames);

impl ScriptFrames {
    fn new() -> ScriptFrames {
        ScriptFrames(Frames::new(SCREEN))
    }

    /// Draws a frame of `events` and one more to settle, giving what the last one drew.
    fn run(&mut self, editor: &mut Editor, at: &ScriptRow, events: Vec<egui::Event>) -> Drawn {
        editor.open_tab(EditorTab::Script(at.clone()));
        editor.place_tabs();
        let mut drawn = Drawn::default();
        for events in [events, Vec::new()] {
            drawn = self.0.run(events, |ui| {
                editor::shell::shortcuts(editor, ui.ctx());
                script::show(editor, ui, at);
            });
        }
        drawn
    }
}
