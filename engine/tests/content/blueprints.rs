use std::sync::Arc;

use engine::core::content::{ContentFolder, FolderReader};
use engine::core::geometry::Degrees;
use engine::core::math::{Pos, Rect, Size};
use engine::core::units::Meters;
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::{self, AreaDef, AreaModule};
use serde_json::{Value, json};
use testkit::{Game, path};

use crate::support::game;

#[test]
fn a_placement_flattens_to_exactly_its_parts_nested_ones_included() {
    let content = placing(
        game()
            .prop("rock", json!({ "type": "none" }))
            .blueprint("tree", (1, 1), json!({}), json!([rock_at_x(0.0)]))
            .blueprint("grove", (4, 2), json!({}), grove()),
        json!([
            placement("Grove", "grove", 6.0, 4.0, 0.0),
            placement("Tree", "tree", 2.0, 2.0, 0.0),
        ]),
    )
    .load();
    let start = start(&content);
    let placed = area::placed(&content, start);

    let parts: Vec<(&str, usize, Pos<Meters>)> = placed
        .iter()
        .map(|part| {
            (
                part.origin.as_str(),
                part.chain.len(),
                prop_at(&part.module).0,
            )
        })
        .collect();
    assert_eq!(
        parts,
        vec![
            ("/1", 1, Pos::new(6.0, 4.0)),
            ("/1", 2, Pos::new(8.0, 5.0)),
            ("/2", 1, Pos::new(2.0, 2.0)),
        ]
    );
}

#[test]
fn folders_dissolve_into_their_modules_in_tree_order() {
    let content = placing(
        game().prop("rock", json!({ "type": "none" })),
        json!([
            { "type": "folder", "label": "Rocks", "hidden": true, "children": [
                rock_at_x(4.0),
                { "type": "folder", "label": "More", "locked": true, "children": [rock_at_x(5.0)] },
            ] },
            rock_at_x(6.0),
        ]),
    )
    .load();
    let start = start(&content);

    let order: Vec<(f32, &str)> = area::placed(&content, start)
        .iter()
        .map(|part| (prop_at(&part.module).0.x, part.origin.as_str()))
        .collect();
    assert_eq!(
        order,
        [
            (4.0, "/1/children/0"),
            (5.0, "/1/children/1/children/0"),
            (6.0, "/2")
        ]
    );
}

#[test]
fn a_placement_turned_by_quarter_turns_turns_what_it_places_about_its_spot() {
    let turned = |yaw: f32| {
        let content = placing(
            game().prop("rock", json!({ "type": "none" })).blueprint(
                "camp",
                (4, 4),
                json!({}),
                json!([
                    { "type": "prop", "prop": path("prop", "rock"), "at": { "x": 2.0, "y": 1.0 }, "yaw": 30.0 },
                    { "type": "safe_zone", "rect": { "x": 1.0, "y": 0.0, "w": 2.0, "h": 1.0 } },
                    { "type": "resident", "npc": path("npc", "wren"), "at": { "x": 1.0, "y": 0.0 }, "facing": 350.0 },
                ]),
            ),
            json!([placement("Camp", "camp", 4.0, 4.0, yaw)]),
        )
        .npc("wren", 10.0, json!([]))
        .load();
        let start = start(&content);
        let placed = area::placed(&content, start);
        let (at, yaw) = prop_at(&placed[0].module);
        let zone = match &placed[1].module {
            AreaModule::SafeZone(zone) => zone.rect,
            _ => panic!("not a safe zone"),
        };
        let (home, facing) = match &placed[2].module {
            AreaModule::Resident(resident) => (resident.at, resident.facing),
            _ => panic!("not a resident"),
        };
        (at, yaw, zone, home, facing)
    };

    assert_eq!(
        turned(0.0),
        (
            Pos::new(6.0, 5.0),
            Degrees(30.0),
            Rect::new(Pos::new(5.0, 4.0), Size::new(2.0, 1.0)),
            Pos::new(5.0, 4.0),
            Degrees(350.0),
        )
    );
    assert_eq!(
        turned(90.0),
        (
            Pos::new(3.0, 6.0),
            Degrees(120.0),
            Rect::new(Pos::new(3.0, 5.0), Size::new(1.0, 2.0)),
            Pos::new(4.0, 5.0),
            Degrees(80.0),
        )
    );
    assert_eq!(
        turned(180.0),
        (
            Pos::new(2.0, 3.0),
            Degrees(210.0),
            Rect::new(Pos::new(1.0, 3.0), Size::new(2.0, 1.0)),
            Pos::new(3.0, 4.0),
            Degrees(170.0),
        )
    );
}

#[test]
fn a_placement_turned_by_anything_but_a_quarter_turn_is_refused() {
    let report = placing(
        game().prop("rock", json!({ "type": "none" })).blueprint(
            "tree",
            (1, 1),
            json!({}),
            json!([rock_at_x(0.0)]),
        ),
        json!([placement("Tree", "tree", 2.0, 2.0, 45.0)]),
    )
    .try_load()
    .err()
    .expect("refused");

    let refused: Vec<(&str, &str)> = report
        .errors
        .iter()
        .filter(|error| error.problem.rule == "not a quarter turn")
        .map(|error| (error.file.as_str(), pointer(&error.at)))
        .collect();
    assert_eq!(refused, [("start.area/scene.json", "/1/yaw")], "{report}");
}

#[test]
fn a_placement_whose_parts_land_past_the_edge_is_refused() {
    let report = placing(
        game().prop("rock", json!({ "type": "none" })).blueprint(
            "tree",
            (1, 1),
            json!({}),
            json!([rock_at_x(0.0)]),
        ),
        json!([placement("Tree", "tree", 12.0, 3.0, 0.0)]),
    )
    .try_load()
    .err()
    .expect("refused");

    assert!(rules(&report).contains(&"outside the terrain"), "{report}");
}

#[test]
fn a_placement_needs_a_label_of_its_own_among_its_siblings() {
    let trees = || {
        game().prop("rock", json!({ "type": "none" })).blueprint(
            "tree",
            (1, 1),
            json!({}),
            json!([rock_at_x(0.0)]),
        )
    };
    let mut unlabelled = placement("Tree", "tree", 2.0, 2.0, 0.0);
    unlabelled.as_object_mut().expect("a node").remove("label");
    let report = placing(trees(), json!([unlabelled]))
        .try_load()
        .err()
        .expect("refused");
    assert!(rules(&report).contains(&"missing field"), "{report}");

    let report = placing(
        trees(),
        json!([
            placement("Tree", "tree", 2.0, 2.0, 0.0),
            placement("tree", "tree", 4.0, 2.0, 0.0),
        ]),
    )
    .try_load()
    .err()
    .expect("refused");

    assert!(rules(&report).contains(&"duplicate name"), "{report}");
}

#[test]
fn an_area_with_variables_is_refused() {
    let report = game()
        .edit("area", "start", |area| {
            area["variables"] = json!({ "x": { "type": "number", "default": 1.0 } });
        })
        .try_load()
        .err()
        .expect("refused");

    assert!(rules(&report).contains(&"unknown field"), "{report}");
}

#[test]
fn a_blueprint_holding_itself_is_refused() {
    let report = placing(
        game().prop("rock", json!({ "type": "none" })).blueprint(
            "loop",
            (2, 1),
            json!({}),
            json!([rock_at_x(0.0), placement("Loop", "loop", 1.0, 0.0, 0.0),]),
        ),
        json!([placement("Loop", "loop", 2.0, 2.0, 0.0)]),
    )
    .try_load()
    .err()
    .expect("refused");

    assert!(
        rules(&report).contains(&"blueprint holds itself"),
        "{report}"
    );
}

#[test]
fn nesting_loads_up_to_eight_levels_and_is_refused_past_them() {
    let content = nest(1).load();
    assert_eq!(area::placed(&content, start(&content)).len(), 1);

    let report = nest(0).try_load().err().expect("refused");
    let rules = rules(&report);
    assert!(rules.contains(&"nested too deep"), "{report}");
    assert!(!rules.contains(&"blueprint holds itself"), "{report}");
}

#[test]
fn a_placed_blueprint_naming_an_unknown_file_is_reported_once_at_the_blueprint() {
    let mut bad = rock_at_x(0.0);
    bad["prop"] = json!("nowhere.prop");
    let report = placing(
        game().blueprint("bad", (1, 1), json!({}), json!([bad])),
        json!([placement("Bad", "bad", 2.0, 2.0, 0.0)]),
    )
    .try_load()
    .err()
    .expect("refused");

    let unknown: Vec<&str> = report
        .errors
        .iter()
        .filter(|error| error.problem.rule == "missing file")
        .map(|error| error.file.as_str())
        .collect();
    assert_eq!(unknown, ["bad.blueprint/scene.json"], "{report}");
}

#[test]
fn a_placement_fills_each_hole_with_the_value_it_gives_or_its_variables_default() {
    let rock = |variables: Value, values: Value| {
        let content = placing(
            game().prop("rock", json!({ "type": "none" })).blueprint(
                "rock",
                (1, 1),
                variables,
                json!([holed_rock()]),
            ),
            json!([with_values(
                placement("Rock", "rock", 2.0, 2.0, 0.0),
                values
            )]),
        )
        .load();
        area::placed(&content, start(&content))
            .iter()
            .map(|part| prop_at(&part.module).0)
            .collect::<Vec<Pos<Meters>>>()
    };

    assert_eq!(
        rock(json!({ "x": { "type": "number" } }), json!({ "x": 3.0 })),
        [Pos::new(5.0, 2.0)]
    );
    assert_eq!(
        rock(
            json!({ "x": { "type": "number", "default": 1.0 } }),
            json!({})
        ),
        [Pos::new(3.0, 2.0)]
    );
}

#[test]
fn a_hole_naming_no_variable_of_its_blueprint_is_reported_at_the_hole() {
    let report = game()
        .prop("rock", json!({ "type": "none" }))
        .blueprint(
            "rock",
            (1, 1),
            json!({ "y": { "type": "number" } }),
            json!([holed_rock()]),
        )
        .try_load()
        .err()
        .expect("refused");

    let unknown: Vec<(&str, &str)> = report
        .errors
        .iter()
        .filter(|error| error.problem.rule == "unknown variable")
        .map(|error| (error.file.as_str(), pointer(&error.at)))
        .collect();
    assert_eq!(
        unknown,
        [("rock.blueprint/scene.json", "/0/at/x/var")],
        "{report}"
    );
}

#[test]
fn a_blueprint_writes_its_holes_back_where_it_read_them() {
    let game = game().prop("rock", json!({ "type": "none" })).blueprint(
        "rock",
        (1, 1),
        json!({ "x": { "type": "number" }, "look": { "type": "row", "table": "prop" } }),
        json!([{
            "type": "prop",
            "prop": { "var": "look" },
            "at": { "x": { "var": "x" }, "y": 0.0 },
            "label": "Rock",
        }]),
    );
    let folder = Arc::new(game.folder());
    let reader = FolderReader::new(game.registry(), folder.clone());
    let rock = path("blueprint", "rock");
    let blueprint: BlueprintDef = reader.read(&rock).expect("reads");

    let scene = format!("{rock}/scene.json");
    let written = reader.write(&rock, &blueprint).expect("writes").files;
    let read: Value =
        serde_json::from_slice(&folder.read(&scene).expect("the scene")).expect("json");
    let wrote: Value = serde_json::from_slice(&written[&scene]).expect("json");
    assert_eq!(wrote, read);
}

fn holed_rock() -> Value {
    json!({
        "type": "prop",
        "prop": path("prop", "rock"),
        "at": { "x": { "var": "x" }, "y": 0.0 },
    })
}

fn with_values(mut placement: Value, values: Value) -> Value {
    placement["values"] = values;
    placement
}

fn grove() -> Value {
    json!([rock_at_x(0.0), placement("Tree", "tree", 2.0, 1.0, 0.0),])
}

fn rock_at_x(x: f32) -> Value {
    json!({
        "type": "prop",
        "prop": path("prop", "rock"),
        "at": { "x": x, "y": 0.0 },
    })
}

fn placement(label: &str, blueprint: &str, x: f32, y: f32, yaw: f32) -> Value {
    let mut node = json!({
        "type": "blueprint",
        "label": label,
        "blueprint": path("blueprint", blueprint),
        "at": { "x": x, "y": y },
    });
    if yaw != 0.0 {
        node["yaw"] = json!(yaw);
    }
    node
}

/// Blueprints nested from `level{top}` down to the deepest level allowed, placed in the start.
fn nest(top: usize) -> Game {
    let mut game = game().prop("rock", json!({ "type": "none" }));
    for depth in top..=area::blueprint::MAX_DEPTH {
        let nodes = if depth == area::blueprint::MAX_DEPTH {
            json!([rock_at_x(0.0)])
        } else {
            let next = format!("level{}", depth + 1);
            json!([placement(&next, &next, 0.0, 0.0, 0.0)])
        };
        game = game.blueprint(&format!("level{depth}"), (1, 1), json!({}), nodes);
    }
    let top = format!("level{top}");
    placing(game, json!([placement(&top, &top, 2.0, 2.0, 0.0)]))
}

/// The start area with `nodes` in its scene after an empty folder, so every node's pointer starts
/// at one.
fn placing(game: Game, nodes: Value) -> Game {
    game.edit_scene("area", "start", |scene| {
        let mut nodes = nodes.as_array().expect("nodes").clone();
        nodes.insert(0, json!({ "type": "folder", "label": "Ground" }));
        *scene = Value::Array(nodes);
    })
}

fn start(content: &engine::core::content::Content) -> engine::systems::area::AreaId {
    content
        .by_path::<AreaDef>(&path("area", "start"))
        .expect("start")
}

fn pointer(at: &engine::core::content::ErrorPlace) -> &str {
    match at {
        engine::core::content::ErrorPlace::Json { pointer, .. } => pointer,
        _ => "",
    }
}

fn rules(report: &engine::core::content::LoadReport) -> Vec<&str> {
    report
        .errors
        .iter()
        .map(|error| error.problem.rule)
        .collect()
}

fn prop_at(module: &AreaModule) -> (Pos<Meters>, Degrees) {
    match module {
        AreaModule::Prop(prop) => (prop.at, prop.yaw),
        other => panic!("not a prop: {}", serde_json::to_value(other).expect("json")),
    }
}
