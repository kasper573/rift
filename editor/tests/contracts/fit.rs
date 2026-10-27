use editor::app::Editor;
use editor::project::RowSelection;
use editor::shell::row_form;
use editor::views::script::inspect;
use editor::views::script::{ScriptDoc, ScriptRow};
use serde_json::json;
use testkit::path;
use ui::egui;

use crate::support;

const WIDTH: f32 = 300.0;
const GUARD: &str = "world/areas/island/npcs/a_rather_long_named_harbour_guard";

fn open(scratch: &tempfile::TempDir) -> Editor {
    let line = |face: &str, text: &str| json!({ "by": { "type": "npc", "npc": path("npc", GUARD), "face": face }, "text": [text] });
    let game = support::game()
        .npc(GUARD, 10.0, json!([]))
        .bust_of(GUARD)
        .script(
            "greeting",
            "npc",
            json!({
                "start": { "type": "on", "event": "interacted", "verb": "talk", "reach": 2.0, "next": "cold" },
                "cold": {
                    "type": "dialogue",
                    "lines": [
                        line("angry", "Short."),
                        line("neutral", "A line long enough that it has to wrap to stay inside the card it is written in, twice over at least."),
                    ],
                    "choices": [{ "label": ["A choice long enough that it has to wrap to stay inside its card as well."] }],
                },
            }),
        );
    let game = game.row(
        "script",
        "watch",
        json!({
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
        }),
    );
    support::open(game, scratch)
}

/// How wide `body` lays itself out when it is given [`WIDTH`], once remembered sizes settle.
fn laid_out_width(editor: &mut Editor, body: impl Fn(&mut Editor, &mut egui::Ui)) -> f32 {
    let ctx = egui::Context::default();
    ui::apply(&ctx);
    let mut width = 0.0;
    for _ in 0..3 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 1200.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            ui.allocate_ui(egui::vec2(WIDTH, 1200.0), |ui| {
                ui.set_max_width(WIDTH);
                body(editor, ui);
                width = ui.min_rect().width();
            });
        });
        output.textures_delta.clear();
    }
    width
}

#[test]
fn a_node_inspector_fits_the_width_it_is_given() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = open(&scratch);
    for (script, node) in [("greeting", "cold"), ("watch", "found")] {
        let at = ScriptRow {
            key: path("script", script),
        };
        let width = laid_out_width(&mut editor, |editor, ui| {
            let doc = ScriptDoc::load(editor, &at).expect("the script");
            let selection = RowSelection::parts("script", &at.key, vec![format!("node:{node}")]);
            inspect::show(editor, ui, &doc, &selection);
        });
        assert!(width <= WIDTH, "{node}: {width}");
    }
}

#[test]
fn a_row_naming_a_long_path_fits_the_width_it_is_given() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = open(&scratch);
    let width = laid_out_width(&mut editor, |editor, ui| {
        row_form::show_row(editor, ui, "npc", &path("npc", GUARD));
    });
    assert!(width <= WIDTH, "{width}");
}
