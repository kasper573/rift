use std::collections::BTreeSet;
use ui::egui;

use editor::views::area::grid::GridDraft;
use editor::views::area::{AreaAction, AreaSnapshot, run_action};
use editor::views::tileset::{SheetRect, TileBlueprintDialog, TileSheetDialog};
use engine::core::math::Pos;
use engine::core::units::{CellPos, Meters};
use serde_json::json;
use testkit::path;

use crate::support::{self, Frames, key};

#[test]
fn ctrl_and_the_wheel_over_a_tilesets_sheet_zoom_it_in_and_out() {
    let scratch = tempfile::tempdir().expect("scratch");
    let game = support::game()
        .row(
            "tileset",
            "rocks",
            serde_json::json!({ "tile": testkit::TILE }),
        )
        .part("tileset", "rocks", "image.png", testkit::png(256, 256));
    let mut editor = support::open(game, &scratch);
    let tileset = path("tileset", "rocks");
    let mut frames = Frames::new(egui::vec2(1600.0, 900.0));
    let mut show = |editor: &mut editor::app::Editor, events: Vec<egui::Event>| {
        frames.run(events, |ui| {
            editor::views::tileset::show(editor, ui, &tileset)
        })
    };
    show(&mut editor, Vec::new());
    show(&mut editor, Vec::new());
    let zoom =
        |editor: &editor::app::Editor| editor.views.tileset.view(&path("tileset", "rocks")).zoom;

    for (notches, zooms_in) in [(1.0, true), (-1.0, false)] {
        let before = zoom(&editor);
        let ctrl = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
        let wheel = vec![
            egui::Event::ModifiersChanged(ctrl),
            egui::Event::PointerMoved(egui::pos2(800.0, 450.0)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, notches),
                phase: egui::TouchPhase::Move,
                modifiers: ctrl,
            },
        ];
        show(&mut editor, wheel);
        show(
            &mut editor,
            vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
        );
        assert_eq!(zoom(&editor) > before, zooms_in, "{notches} notch");
    }
}

#[test]
fn a_blueprint_cut_from_a_sheet_blocks_the_cells_painted_on_it_once_it_makes_them_its_own() {
    let scratch = tempfile::tempdir().expect("scratch");
    let walkable: serde_json::Map<String, serde_json::Value> = (0..8)
        .map(|tile| (tile.to_string(), json!({ "walk": "walkable" })))
        .collect();
    let game = support::game()
        .row(
            "tileset",
            "huts",
            json!({ "tile": testkit::TILE, "tiles": walkable }),
        )
        .part("tileset", "huts", "image.png", testkit::png(64, 32));
    let mut editor = support::open(game, &scratch);
    let tileset = path("tileset", "huts");
    let mut frames = Frames::new(egui::vec2(1600.0, 900.0));
    editor::views::tileset::ensure_view(&mut editor, &tileset);
    let view = editor.views.tileset.view_mut(&tileset);
    view.selection = Some(SheetRect {
        x: 0,
        y: 0,
        w: 3,
        h: 2,
    });
    view.dialog = Some(TileSheetDialog::MakeBlueprint(TileBlueprintDialog {
        name: "hut".to_owned(),
        error: None,
    }));
    for events in [
        Vec::new(),
        key(egui::Key::Enter, egui::Modifiers::NONE),
        Vec::new(),
    ] {
        frames.run(events, |ui| {
            editor::views::tileset::show(&mut editor, ui, &tileset)
        });
    }
    support::files(&mut editor);
    let hut = path("blueprint", "hut");
    let painted: BTreeSet<(i32, i32)> = [(1, 1), (2, 0)].into();
    let mut draft = GridDraft::own(&editor, &hut).expect("the blueprint is made");
    for &(x, y) in &painted {
        draft.set_walk(CellPos::new(x, y), Some(false));
    }
    draft.commit(&mut editor, "Walk".to_owned());
    support::files(&mut editor);
    let blocked = |editor: &mut editor::app::Editor| -> BTreeSet<(i32, i32)> {
        let snapshot = AreaSnapshot::for_bundle(editor, &hut).expect("the blueprint opens");
        let grid = &snapshot.area().grid;
        (0..2)
            .flat_map(|y| (0..3).map(move |x| (x, y)))
            .filter(|&(x, y)| !grid.walkable(Pos::<Meters>::new(x as f32, y as f32)))
            .collect()
    };
    assert_eq!(blocked(&mut editor), painted, "painted on the walk grid");

    let snapshot = AreaSnapshot::for_bundle(&mut editor, &hut).expect("the blueprint opens");
    run_action(
        &mut editor,
        &snapshot,
        AreaAction::AdoptBlocks("/0".to_owned()),
    );
    support::files(&mut editor);

    assert_eq!(blocked(&mut editor), painted, "made the scenery's own");
    let draft = GridDraft::own(&editor, &hut).expect("the blueprint");
    assert!(
        draft.cells().all(|cell| draft.walk_at(cell).is_none()),
        "no cell is left forced"
    );
}
