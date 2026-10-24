use std::collections::HashSet;
use ui::egui;

use editor::app::Editor;
use editor::project::RowSelection;
use editor::views::area::brush::{BrushStroke, BrushStrokeKind};
use editor::views::area::grid::GridDraft;
use editor::views::area::nodes::SceneTree;
use editor::views::area::scene_tree;
use editor::views::area::terrain::{self, TerrainBrush};
use editor::views::area::{AreaAction, AreaSnapshot, AreaTool, nodes, run_action};
use engine::core::content::ContentFile;
use engine::core::math::{Pos, Rect, Rng, Size};
use engine::core::units::{CellPos, TilePos};
use engine::systems::area::object::ObjectShape;
use engine::systems::area::scene::{ModuleNode, SceneNode};
use engine::systems::area::tileset::TilesetDef;
use engine::systems::area::{AreaDef, AreaModule};
use serde_json::{Value, json};

use crate::support::{self, Frames, area, key};
use engine::systems::script::{RunnerKind, ScriptDef};
use testkit::{Game, Map, path};

fn barn_door() -> Value {
    json!({
        "type": "object",
        "name": "barn-doors",
        "shape": { "type": "rect", "x": 2.5, "y": 2.5, "w": 2.0, "h": 2.0 }
    })
}

fn editor_with_a_door(scratch: &tempfile::TempDir) -> Editor {
    let mut editor = support::open(support::game(), scratch);
    let reader = editor.reader().expect("a reader");
    let door: AreaModule = reader.from_json(&barn_door()).expect("a door");
    assert!(nodes::change_scene(&mut editor, &area(), |scene| {
        SceneTree::new(scene).append("", vec![SceneNode::Module(ModuleNode::new(door))]);
        Some("Add a door".to_owned())
    }));
    editor
}

/// The door's node: after the ground layer the area starts with.
const DOOR: &str = "/1";

/// The name and shape of each object of the start area, in the order its scene lists them.
fn doors(editor: &Editor) -> Vec<(Option<String>, ObjectShape)> {
    nodes::modules(&nodes::scene_of(editor, &area()))
        .iter()
        .filter_map(|module| match module.module() {
            AreaModule::Object(object) => Some((object.name.clone(), object.shape)),
            _ => None,
        })
        .collect()
}

fn duplicate(editor: &mut Editor, rows: Vec<String>) {
    editor.selection = RowSelection::parts(AreaDef::TABLE, &area(), rows);
    let snapshot = AreaSnapshot::for_bundle(editor, &area()).expect("the area is open");
    run_action(editor, &snapshot, AreaAction::Duplicate);
}

#[test]
fn duplicating_a_module_places_its_copy_one_tile_on_under_a_fresh_name_in_one_undo_step() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = editor_with_a_door(&scratch);
    let original = doors(&editor);

    duplicate(&mut editor, Vec::new());
    assert_eq!(doors(&editor), original, "nothing selected, nothing copied");

    duplicate(&mut editor, vec![DOOR.to_owned()]);
    let copies = doors(&editor);
    assert_eq!(copies.len(), 2);
    assert_eq!(copies[0], original[0]);
    let (ObjectShape::Rect(was), ObjectShape::Rect(copy)) = (copies[0].1, copies[1].1) else {
        panic!("the doors are rects");
    };
    assert_eq!(
        (copy.origin.x - was.origin.x, copy.origin.y - was.origin.y),
        (1.0, 1.0),
        "the copy sits one tile on"
    );
    assert_ne!(copies[1].0, copies[0].0);

    editor.undo();
    assert_eq!(doors(&editor), original);
}

#[test]
fn a_duplicated_ground_layer_takes_a_fresh_name_and_its_cells_along() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = editor_with_a_door(&scratch);

    duplicate(&mut editor, vec!["/0".to_owned(), DOOR.to_owned()]);

    assert_eq!(doors(&editor).len(), 2, "the duplicate applies");
    let bundle = nodes::bundle_of(&editor, &area()).expect("the area reads");
    let grids = &bundle.blueprint().scene.grids;
    let names: Vec<&String> = grids.layers.keys().collect();
    assert_eq!(names.len(), 2);
    assert_eq!(grids.layers[names[0]], grids.layers[names[1]]);
}

#[test]
fn copied_rows_paste_in_the_order_the_scene_lists_them() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = editor_with_a_door(&scratch);
    let reader = editor.reader().expect("a reader");
    let named = |name: &str| {
        let mut door = barn_door();
        door["name"] = json!(name);
        SceneNode::Module(ModuleNode::new(reader.from_json(&door).expect("a door")))
    };
    let more: Vec<SceneNode> = (2..=10).map(|n| named(&format!("door-{n}"))).collect();
    assert!(nodes::change_scene(&mut editor, &area(), |scene| {
        SceneTree::new(scene).append("", more);
        Some("Add doors".to_owned())
    }));
    let snapshot = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");
    scene_tree::copy_rows(&mut editor, &snapshot, &["/10".to_owned(), "/2".to_owned()]);

    let snapshot = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");
    scene_tree::paste_rows(&mut editor, &snapshot, "");

    let names = support::names(&nodes::scene_of(&editor, &area()));
    assert_eq!(names[names.len() - 2..], ["door-2", "door-10"]);
}

#[test]
fn switching_tools_or_escape_over_the_area_ends_the_stroke_held() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = editor_with_a_door(&scratch);
    hold_a_stroke(&mut editor);
    view(&mut editor).set_tool(AreaTool::Select);
    assert!(view(&mut editor).stroke().is_none());

    let mut frames = Frames::new(egui::vec2(1600.0, 900.0));
    let over_the_area = vec![egui::Event::PointerMoved(egui::pos2(800.0, 450.0))];
    frames.run(over_the_area.clone(), |ui| {
        editor::views::area::show(&mut editor, ui, &area())
    });
    hold_a_stroke(&mut editor);
    frames.run(over_the_area, |ui| {
        editor::views::area::show(&mut editor, ui, &area())
    });
    assert!(view(&mut editor).stroke().is_some());

    frames.run(key(egui::Key::Escape, egui::Modifiers::NONE), |ui| {
        editor::views::area::show(&mut editor, ui, &area())
    });
    assert!(view(&mut editor).stroke().is_none());
}

#[test]
fn ctrl_g_over_the_scene_graph_groups_the_selection_into_a_new_folder() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = editor_with_a_door(&scratch);
    let mut frames = Frames::new(egui::vec2(1600.0, 900.0));
    let over_the_panel = vec![egui::Event::PointerMoved(egui::pos2(150.0, 400.0))];
    frames.run(over_the_panel.clone(), |ui| {
        editor::views::area::show(&mut editor, ui, &area())
    });
    editor.selection = RowSelection::parts(
        AreaDef::TABLE,
        &area(),
        vec!["/0".to_owned(), DOOR.to_owned()],
    );
    frames.run(over_the_panel, |ui| {
        editor::views::area::show(&mut editor, ui, &area())
    });

    let ctrl = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
    let ctrl_g = [egui::Event::ModifiersChanged(ctrl)]
        .into_iter()
        .chain(key(egui::Key::G, ctrl))
        .collect();
    frames.run(ctrl_g, |ui| {
        editor::views::area::show(&mut editor, ui, &area())
    });

    let scene = nodes::scene_of(&editor, &area());
    assert_eq!(scene.len(), 1);
    assert!(nodes::is_folder(&scene[0]));
    assert_eq!(nodes::children_of(&scene[0]).len(), 2);
}

#[test]
fn a_terrain_stroke_turns_the_cells_it_covers_into_its_terrain_and_leaves_other_tiles_be() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let drift = testkit::path("tileset", "drift");
    let game = support::game()
        .row(
            "tileset",
            "drift",
            json!({
                "tile": testkit::TILE,
                "terrains": { "drift": ["ground", "snow"] },
                "tiles": {
                    "0": { "corners": { "drift": ["snow", "snow", "snow", "snow"] } },
                    "1": { "corners": { "drift": ["ground", "ground", "ground", "snow"] } },
                },
            }),
        )
        .part("tileset", "drift", "image.png", testkit::png(32, 16));
    let mut editor = support::open(game, &scratch);
    let snapshot = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");
    let def = {
        let content = support::content(&mut editor);
        content
            .by_path::<TilesetDef>(&drift)
            .expect("the drift tileset")
            .get(content)
            .clone()
    };
    let tilesets = |draft: &GridDraft| -> Vec<(CellPos, String)> {
        let mut cells: Vec<(CellPos, String)> = draft
            .cells()
            .filter_map(|cell| {
                let row = draft.entry(draft.get(0, cell)?)?;
                Some((cell, row.tileset.clone()))
            })
            .collect();
        cells.sort_by_key(|(cell, _)| (cell.y, cell.x));
        cells
    };

    for (terrain, snowed) in [(0, 0), (1, 5)] {
        let mut draft = GridDraft::own(&editor, &snapshot.bundle).expect("the area's grids");
        let before = tilesets(&draft);
        let brush = TerrainBrush {
            def: &def,
            tileset: &drift,
            set: "drift",
            terrain,
            size: 2,
        };
        terrain::paint(
            &mut draft,
            0,
            &brush,
            CellPos::new(3, 3),
            &mut Rng::new(1),
            &mut HashSet::new(),
        );

        let after = tilesets(&draft);
        assert_eq!(
            after.len(),
            before.len(),
            "terrain {terrain} cleared no cell"
        );
        let drifts = after.iter().filter(|(_, tileset)| *tileset == drift);
        assert_eq!(drifts.count(), snowed, "terrain {terrain}");
        if terrain == 1 {
            let above_left = draft
                .get(0, CellPos::new(1, 1))
                .and_then(|index| draft.entry(index))
                .map(|row| row.tile);
            assert_eq!(
                above_left,
                Some(1),
                "the cell up and left of the stroke is snowed only in its south-east corner"
            );
        }
    }
}

#[test]
fn an_optional_field_set_to_what_the_game_reads_without_it_stays_shown_to_edit_in_that_file_alone()
{
    let scratch = tempfile::tempdir().expect("scratch");
    let wind = json!({
        "type": "soundscape",
        "shape": { "type": "rect", "x": -0.5, "y": -0.5, "w": 8.0, "h": 8.0 },
        "emit": 0.0,
        "channels": [{ "channel": 1, "file": "audio/wind.wav" }],
    });
    let game = Game::new()
        .file("audio/wind.wav", testkit::wav())
        .area(support::AREA, &Map::open(8, 8), json!([wind.clone()]))
        .area("cellar", &Map::open(8, 8), json!([wind]))
        .hero(support::AREA, 1.0, 1.0)
        .engine_rows()
        .whole();
    let mut editor = support::open(game, &scratch);
    let mut frames = Frames::new(egui::vec2(1600.0, 900.0));
    let mut show = |editor: &mut Editor, area: &str, events: Vec<egui::Event>| {
        editor.selection = RowSelection::parts(AreaDef::TABLE, area, vec!["/1".to_owned()]);
        frames.run(events, |ui| editor::views::area::show(editor, ui, area))
    };
    show(&mut editor, &area(), Vec::new());
    let set_volume = show(&mut editor, &area(), Vec::new()).at("+ volume");

    for events in support::click(set_volume) {
        show(&mut editor, &area(), events);
    }

    let drawn = show(&mut editor, &area(), Vec::new());
    assert!(!drawn.any(|text| text == "+ volume"), "{drawn:?}");
    assert!(drawn.any(|text| text == "volume"), "{drawn:?}");
    let cellar = path("area", "cellar");
    show(&mut editor, &cellar, Vec::new());
    let drawn = show(&mut editor, &cellar, Vec::new());
    assert!(drawn.any(|text| text == "+ volume"), "{drawn:?}");
}

#[test]
fn an_object_given_a_script_runs_the_nearest_object_script_or_one_made_for_it() {
    let warp = json!({
        "runs_on": "object",
        "variables": { "to": { "type": "place" } },
        "nodes": {
            "entered": { "type": "on", "event": "entered", "by": "characters", "next": "warp" },
            "warp": { "type": "warp", "to": { "var": "to" } },
        },
    });
    for (object_scripts, named) in [
        (vec![], path("script", "barn-doors")),
        (vec!["creak"], path("script", "creak")),
        (vec!["warp"], path("script", "warp")),
    ] {
        let scratch = tempfile::tempdir().expect("scratch");
        let mut game = support::game().script("a_talk", "npc", json!({}));
        for name in &object_scripts {
            let entered =
                json!({ "entered": { "type": "on", "event": "entered", "by": "characters" } });
            game = match *name {
                "warp" => game.row("script", name, warp.clone()),
                _ => game.script(name, "object", entered),
            };
        }
        let mut editor = support::open(game, &scratch);
        let reader = editor.reader().expect("a reader");
        let door: AreaModule = reader.from_json(&barn_door()).expect("a door");
        assert!(nodes::change_scene(&mut editor, &area(), |scene| {
            SceneTree::new(scene).append("", vec![SceneNode::Module(ModuleNode::new(door))]);
            Some("Add a door".to_owned())
        }));
        editor.selection = RowSelection::parts(AreaDef::TABLE, &area(), vec![DOOR.to_owned()]);
        let mut frames = Frames::new(egui::vec2(1600.0, 900.0));
        let mut show = |editor: &mut Editor, events: Vec<egui::Event>| {
            frames.run(events, |ui| editor::views::area::show(editor, ui, &area()))
        };
        show(&mut editor, Vec::new());
        let add = show(&mut editor, Vec::new()).at("+ Add script");

        for events in support::click(add) {
            show(&mut editor, events);
        }
        support::files(&mut editor);

        let refused = editor
            .project
            .as_mut()
            .and_then(|project| project.take_refused());
        assert!(
            refused.is_none(),
            "{object_scripts:?}: {:?}",
            refused.map(|taken| taken.refusal.sentence())
        );
        let reader = editor.reader().expect("a reader");
        let scripts: Vec<String> = nodes::modules(&nodes::scene_of(&editor, &area()))
            .iter()
            .filter_map(|module| match module.module() {
                AreaModule::Object(object) => Some(object.scripts.clone()),
                _ => None,
            })
            .flatten()
            .filter_map(|module| reader.path_of(module.script).map(str::to_owned))
            .collect();
        assert_eq!(scripts, std::slice::from_ref(&named), "{object_scripts:?}");
        let script = editor
            .read::<ScriptDef>(&named)
            .expect("a project")
            .expect("the script reads");
        assert_eq!(script.runs_on, RunnerKind::Object);
    }
}

#[test]
fn a_population_given_a_roam_rect_first_roams_the_whole_area_it_roamed_without_one() {
    let scratch = tempfile::tempdir().expect("scratch");
    let wolves = json!({ "type": "population", "npc": path("npc", "wolf"), "count": 2 });
    let game = Game::new()
        .npc("wolf", 20.0, json!([]))
        .area(support::AREA, &Map::open(8, 6), json!([wolves]))
        .hero(support::AREA, 1.0, 1.0)
        .engine_rows()
        .whole();
    let mut editor = support::open(game, &scratch);
    editor.selection = RowSelection::parts(AreaDef::TABLE, &area(), vec!["/1".to_owned()]);
    let mut frames = Frames::new(egui::vec2(1600.0, 900.0));
    let mut show = |editor: &mut Editor, events: Vec<egui::Event>| {
        frames.run(events, |ui| editor::views::area::show(editor, ui, &area()))
    };
    show(&mut editor, Vec::new());
    let set_roams = show(&mut editor, Vec::new()).at("+ roams");

    for events in support::click(set_roams) {
        show(&mut editor, events);
    }
    support::files(&mut editor);

    let roams: Vec<_> = nodes::modules(&nodes::scene_of(&editor, &area()))
        .iter()
        .filter_map(|module| match module.module() {
            AreaModule::Population(population) => Some(population.roams),
            _ => None,
        })
        .collect();
    let whole = Rect::new(Pos::new(-0.5, -0.5), Size::new(8.0, 6.0));
    assert_eq!(roams, [Some(whole)]);
}

#[test]
fn a_door_back_from_a_warps_destination_stands_beside_where_the_warp_lands() {
    let scratch = tempfile::tempdir().expect("scratch");
    let hall = path("area", "hall");
    let door = json!({
        "type": "object",
        "name": "door",
        "shape": { "type": "point", "x": 4.0, "y": 4.0 },
        "scripts": [{
            "script": path("script", "warp"),
            "values": { "to": { "area": hall, "x": 3.0, "y": 3.0 } },
        }],
    });
    let warp = json!({
        "runs_on": "object",
        "variables": { "to": { "type": "place" } },
        "nodes": {
            "entered": { "type": "on", "event": "entered", "by": "characters", "next": "warp" },
            "warp": { "type": "warp", "to": { "var": "to" } },
        },
    });
    let game = Game::new()
        .row("script", "warp", warp)
        .area(support::AREA, &Map::open(8, 8), json!([door]))
        .area("hall", &Map::open(8, 8), json!([]))
        .hero(support::AREA, 1.0, 1.0)
        .engine_rows()
        .whole();
    let mut editor = support::open(game, &scratch);
    let snapshot = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");

    run_action(
        &mut editor,
        &snapshot,
        AreaAction::ReturnDoor("/1".to_owned()),
    );
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
    let doors: Vec<ObjectShape> = nodes::modules(&nodes::scene_of(&editor, &hall))
        .iter()
        .filter_map(|module| match module.module() {
            AreaModule::Object(object) => Some(object.shape),
            _ => None,
        })
        .collect();
    let [ObjectShape::Point(back)] = doors.as_slice() else {
        panic!("one door back: {doors:?}");
    };
    assert_ne!(
        back.cell(),
        Pos::<engine::core::units::Meters>::new(3.0, 3.0).cell()
    );
    assert!(
        (back.x - 3.0).abs() <= 1.0 && (back.y - 3.0).abs() <= 1.0,
        "{back:?}"
    );
}

#[test]
fn a_quick_drag_of_the_eraser_clears_every_cell_it_crosses_from_where_it_was_pressed() {
    for flick in [false, true] {
        let scratch = tempfile::tempdir().expect("scratch");
        let mut editor = support::open(support::game(), &scratch);
        editor.selection = RowSelection::parts(AreaDef::TABLE, &area(), vec!["/0".to_owned()]);
        view(&mut editor).set_tool(AreaTool::Erase);
        let mut frames = Frames::new(egui::vec2(1600.0, 900.0));
        let mut show = |editor: &mut Editor, events: Vec<egui::Event>| {
            frames.run(events, |ui| editor::views::area::show(editor, ui, &area()));
        };
        show(&mut editor, Vec::new());
        show(&mut editor, Vec::new());
        let (from, to) = if flick {
            let edge = cell_edge_after(&mut editor, &mut show, egui::pos2(650.0, 450.0));
            (edge - egui::vec2(2.0, 0.0), edge + egui::vec2(3.0, 0.0))
        } else {
            (egui::pos2(650.0, 450.0), egui::pos2(950.0, 450.0))
        };
        let press = |at, pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(from)],
            vec![press(from, true)],
            vec![egui::Event::PointerMoved(to)],
            vec![press(to, false)],
            Vec::new(),
        ] {
            show(&mut editor, events);
        }
        support::files(&mut editor);

        let snapshot = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");
        let ground = GridDraft::own(&editor, &snapshot.bundle).expect("the area's grids");
        let mut cleared: Vec<i32> = ground
            .cells()
            .filter(|cell| ground.get(0, *cell).is_none())
            .map(|cell| cell.x)
            .collect();
        cleared.sort();
        cleared.dedup();
        if flick {
            assert_eq!(
                cleared.len(),
                2,
                "the cell pressed and the one let go over: {cleared:?}"
            );
        } else {
            assert!(cleared.len() >= 3, "a stroke, not a dab: {cleared:?}");
        }
        assert!(
            cleared.windows(2).all(|pair| pair[1] == pair[0] + 1),
            "no gaps: {cleared:?}"
        );
    }
}

/// The first point right of `from` that the map reads as the next cell over.
fn cell_edge_after(
    editor: &mut Editor,
    show: &mut impl FnMut(&mut Editor, Vec<egui::Event>),
    from: egui::Pos2,
) -> egui::Pos2 {
    let mut cell_at = |editor: &mut Editor, at: egui::Pos2| {
        show(editor, vec![egui::Event::PointerMoved(at)]);
        let hover = editor.views.area.hover.as_ref();
        hover.expect("the pointer is over the map").tile.cell()
    };
    let first = cell_at(editor, from);
    (1..200)
        .map(|step| from + egui::vec2(step as f32, 0.0))
        .find(|&at| cell_at(editor, at) != first)
        .expect("a cell edge within 200 points")
}

fn hold_a_stroke(editor: &mut Editor) {
    let snapshot = AreaSnapshot::for_bundle(editor, &area()).expect("the area is open");
    let draft = GridDraft::own(editor, &snapshot.bundle).expect("the area's grids");
    view(editor).set_stroke(BrushStroke {
        draft,
        kind: BrushStrokeKind::Paint,
        layer: 0,
        cells: HashSet::new(),
        anchor: CellPos::new(0, 0),
        last: CellPos::new(0, 0),
        rng: Rng::new(1),
    });
}

fn view(editor: &mut Editor) -> &mut editor::views::area::AreaViewState {
    editor.views.area.ensure_view(&area())
}
