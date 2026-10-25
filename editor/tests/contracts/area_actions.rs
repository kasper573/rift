use ui::egui;

use editor::app::Editor;
use editor::project::RowSelection;
use editor::views::area::nodes::SceneTree;
use editor::views::area::scene_tree;
use editor::views::area::{AreaAction, AreaSnapshot, nodes, run_action};
use engine::core::content::ContentFile;
use engine::core::math::{Pos, Rect, Size};
use engine::core::units::{MeterPos, Meters};
use engine::systems::area::object::ObjectShape;
use engine::systems::area::scene::{ModuleNode, SceneNode};
use engine::systems::area::{AreaDef, AreaModule};
use engine::systems::movement;
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

/// The door's node: the first of the start area's scene.
const DOOR: &str = "/0";

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
    scene_tree::copy_rows(&mut editor, &snapshot, &["/9".to_owned(), "/1".to_owned()]);

    let snapshot = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");
    scene_tree::paste_rows(&mut editor, &snapshot, "");

    let names = support::names(&nodes::scene_of(&editor, &area()));
    assert_eq!(names[names.len() - 2..], ["door-2", "door-10"]);
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
    editor.selection = RowSelection::parts(AreaDef::TABLE, &area(), vec![DOOR.to_owned()]);
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
    assert_eq!(nodes::children_of(&scene[0]).len(), 1);
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
        editor.selection = RowSelection::parts(AreaDef::TABLE, area, vec!["/0".to_owned()]);
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
        .area(support::AREA, &Map::open(10, 8), json!([wolves]))
        .hero(support::AREA, 1.0, 1.0)
        .engine_rows()
        .whole();
    let mut editor = support::open(game, &scratch);
    editor.selection = RowSelection::parts(AreaDef::TABLE, &area(), vec!["/0".to_owned()]);
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
    let whole = Rect::new(Pos::new(-0.5, -0.5), Size::new(10.0, 8.0));
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
        AreaAction::ReturnDoor("/0".to_owned()),
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
    let door = Pos::<Meters>::new(3.0, 3.0);
    assert!(
        back.distance(door) >= movement::SPACING
            && back.distance(door) <= Meters(movement::SPACING.0 * 3.0),
        "{back:?}"
    );
}
