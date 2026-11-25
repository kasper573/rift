use bevy::prelude::*;
use engine::core::text::{KeyNames, LineText, SpanText};
use engine::systems::dialogue::sheet::{self, DialogueSheet, SheetChoice, SheetToCome};
use engine::systems::dialogue::{ChoiceTag, ChoiceView};
use serde_json::json;
use testkit::{Game, Map};
use ui::egui::{self, Pos2, Rect, Vec2};
use ui::motion::SheetPose;

/// The dialogue stands where it will stand for its whole step from the step's first letter: as
/// tall as the tallest of the step's lines over the choices that end it. A line typing out, the
/// next line of the step, and the choices showing once the last line is out never move it, so a
/// window or card set beside it at any moment of the step stays clear of it all along.
#[test]
fn the_dialogue_keeps_room_from_its_first_letter_for_every_line_and_choice_of_its_step() {
    let mut world = World::new();
    let content = Game::new()
        .area("start", &Map::open(4, 4), json!([]))
        .hero("start", 1.0, 1.0)
        .load();
    engine::systems::insert_content(&mut world, &content);
    world.insert_resource(KeyNames::default());
    let gui = egui::Context::default();
    ui::apply(&gui);
    let short = said("Good morning.");
    let long = said(
        "One more thing. Take this letter to the captain at the harbor, and mind the gulls on \
         the way, for they take anything that shines and a few things that don't.",
    );
    let step = SheetToCome {
        lines: vec![
            (Some(SPEAKER.to_owned()), short.clone()),
            (Some(SPEAKER.to_owned()), long.clone()),
        ],
        choices: vec![choice("Accept"), choice("Decline")],
    };
    let shows = |line: &LineText, typed: Option<usize>, choosing: bool| DialogueSheet {
        speaker: Some(SPEAKER.to_owned()),
        line: Some(line.clone()),
        typed,
        waits: !choosing,
        choices: if choosing {
            step.choices
                .iter()
                .map(|view| SheetChoice {
                    view: view.clone(),
                    lit: false,
                    refused: false,
                })
                .collect()
        } else {
            Vec::new()
        },
        to_come: step.clone(),
    };

    let choosing = settled(&gui, &mut world, &shows(&long, None, true));
    for (moment, sheet) in [
        ("its first letter out", shows(&short, Some(1), false)),
        ("its first line out", shows(&short, None, false)),
        ("its last line typing", shows(&long, Some(10), false)),
    ] {
        assert_eq!(
            settled(&gui, &mut world, &sheet),
            choosing,
            "with {moment}, the dialogue stands where it will with its choices showing"
        );
    }
}

const SPEAKER: &str = "Tobb";

/// Where the sheet stands once it has laid itself out at the height it needs.
fn settled(gui: &egui::Context, world: &mut World, sheet: &DialogueSheet) -> Rect {
    let mut height = 0.0;
    for _ in 0..8 {
        let mut drawn = None;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0))),
            ..egui::RawInput::default()
        };
        let mut output = gui.run_ui(input, |ui| {
            drawn = Some(sheet::draw(world, ui.ctx(), sheet, SheetPose::REST, height));
        });
        output.textures_delta.clear();
        let drawn = drawn.expect("the sheet drawn");
        if (drawn.height - height).abs() <= 0.5 {
            return drawn.rect;
        }
        height = drawn.height;
    }
    panic!("the sheet never settled on a height");
}

fn said(words: &str) -> LineText {
    LineText(vec![SpanText::Text {
        text: words.to_owned(),
        fx: Vec::new(),
    }])
}

fn choice(label: &str) -> ChoiceView {
    ChoiceView {
        label: said(label),
        tag: ChoiceTag::Dialogue,
        icon: None,
        chips: Vec::new(),
        locked: false,
        warn: None,
    }
}
