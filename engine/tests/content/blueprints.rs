use std::sync::Arc;

use engine::core::content::{ContentFolder, ContentId, FolderReader};
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::grid::Flip;
use engine::systems::area::{self, AreaDef, AreaModule, Scenery, SceneryTile, walk};
use serde_json::{Value, json};
use testkit::{Game, path, png};

use crate::support::game;

#[test]
fn a_placement_flattens_to_exactly_its_parts_nested_ones_included() {
    let content = placing(
        game()
            .blueprint("tree", (1, 1), json!({}), json!([tree()]))
            .blueprint("grove", (4, 2), json!({}), grove()),
        json!([
            placement("Grove", "grove", 6.0, 4.0, "horizontal"),
            placement("Tree", "tree", 2.0, 2.0, "none"),
        ]),
    )
    .load();
    let start = start(&content);
    let placed = area::placed(&content, start);

    let parts: Vec<(&str, usize, Pos<Meters>, Flip)> = placed
        .iter()
        .map(|part| {
            let (at, flip) = scenery_at(&part.module);
            (part.origin.as_str(), part.chain.len(), at, flip)
        })
        .collect();
    assert_eq!(
        parts,
        vec![
            ("/1", 1, Pos::new(6.0, 4.0), Flip::Horizontal),
            ("/1", 2, Pos::new(4.0, 5.0), Flip::Horizontal),
            ("/2", 1, Pos::new(2.0, 2.0), Flip::None),
        ]
    );
}

#[test]
fn folders_dissolve_into_their_modules_in_tree_order() {
    let content = placing(
        game(),
        json!([
            { "type": "folder", "label": "Rocks", "hidden": true, "children": [
                scenery_at_x(4.0),
                { "type": "folder", "label": "More", "locked": true, "children": [scenery_at_x(5.0)] },
            ] },
            scenery_at_x(6.0),
        ]),
    )
    .load();
    let start = start(&content);

    let order: Vec<(f32, &str)> = area::placed(&content, start)
        .iter()
        .map(|part| (scenery_at(&part.module).0.x, part.origin.as_str()))
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
fn a_placed_blueprint_paints_its_ground_into_the_areas_layer_of_the_same_name() {
    let content = placing(
        rugs(game()),
        json!([
            placement("Rug", "rug", 2.0, 3.0, "none"),
            placement("Rug (1)", "rug", 5.0, 3.0, "horizontal"),
            placement("Rug (2)", "rug", 4.0, 3.0, "none"),
            placement("Rug (3)", "rug", 8.0, 3.0, "horizontal"),
        ]),
    )
    .load();
    let start = start(&content);
    let grids = &area::load(&content, start).grids;
    let layer = &grids.ground[0];
    let at = |x: usize, y: usize| {
        grids
            .entry(layer.cells[y * 10 + x])
            .map(|entry| (entry.tile, entry.flip))
    };

    assert_eq!(
        at(1, 3),
        Some((0, Flip::None)),
        "the area's own ground stays"
    );
    assert_eq!(at(2, 3), Some((1, Flip::None)));
    assert_eq!(
        at(3, 3),
        Some((0, Flip::None)),
        "an empty cell keeps what's under it"
    );
    assert_eq!(at(4, 3), Some((1, Flip::None)), "the later placement wins");
    assert_eq!(
        at(7, 3),
        Some((1, Flip::Horizontal)),
        "a flip mirrors cells and tiles"
    );
}

#[test]
fn a_placed_blueprints_forced_walk_cells_merge_into_the_areas() {
    let content = placing(
        rugs(game()).part("blueprint", "rug", "walk.csv", "0,\n"),
        json!([placement("Rug", "rug", 6.0, 6.0, "none")]),
    )
    .load();
    let start = start(&content);
    let grid = &area::load(&content, start).grid;

    assert!(!grid.walkable(Pos::new(6.0, 6.0)));
    assert!(grid.walkable(Pos::new(7.0, 6.0)));
}

#[test]
fn a_placed_blueprints_scenery_blocks_the_cells_it_names_wherever_it_lands() {
    for (flip, blocked, open) in [
        ("none", (6.0, 5.0), [(5.0, 5.0), (6.0, 4.0), (7.0, 5.0)]),
        (
            "horizontal",
            (4.0, 5.0),
            [(5.0, 5.0), (4.0, 4.0), (3.0, 5.0)],
        ),
    ] {
        let mut hut = scenery(json!([[0, 0], [0, 0]]));
        hut["at"] = json!({ "x": -0.5, "y": -0.5 });
        hut["blocks"] = json!([[1, 1]]);
        let game = game()
            .row(
                "tileset",
                "start_ground",
                json!({ "tile": testkit::TILE, "tiles": { "0": { "walk": "walkable" } } }),
            )
            .remove(&format!("{}/walk.csv", path("area", "start")))
            .blueprint("hut", (2, 2), json!({}), json!([hut]));
        let content = placing(game, json!([placement("Hut", "hut", 5.0, 4.0, flip)])).load();
        let grid = &area::load(&content, start(&content)).grid;

        assert!(!grid.walkable(Pos::new(blocked.0, blocked.1)), "{flip}");
        for (x, y) in open {
            assert!(grid.walkable(Pos::new(x, y)), "{flip} {x}, {y}");
        }
    }
}

#[test]
fn a_placement_painting_a_layer_the_area_lacks_is_refused_at_the_placement() {
    let report = placing(
        rugs(game()).edit_scene("blueprint", "rug", |scene| {
            scene[0]["name"] = json!("rug");
        }),
        json!([placement("Rug", "rug", 2.0, 3.0, "none")]),
    )
    .part("blueprint", "rug", "layers/rug.csv", "0,\n")
    .remove(&format!("{}/layers/ground.csv", path("blueprint", "rug")))
    .try_load()
    .err()
    .expect("refused");

    let missing: Vec<(&str, &str)> = report
        .errors
        .iter()
        .filter(|error| error.problem.rule == "missing layer")
        .map(|error| (error.file.as_str(), pointer(&error.at)))
        .collect();
    assert_eq!(missing, [("start.area/scene.json", "/1")], "{report}");
}

#[test]
fn a_placement_whose_ground_lands_past_the_edge_is_refused() {
    let report = placing(
        rugs(game()),
        json!([placement("Rug", "rug", 10.0, 3.0, "none")]),
    )
    .try_load()
    .err()
    .expect("refused");

    assert!(rules(&report).contains(&"outside the map"), "{report}");
}

#[test]
fn a_placement_needs_a_label_of_its_own_among_its_siblings() {
    let mut unlabelled = placement("Tree", "tree", 2.0, 2.0, "none");
    unlabelled.as_object_mut().expect("a node").remove("label");
    let report = placing(
        game().blueprint("tree", (1, 1), json!({}), json!([tree()])),
        json!([unlabelled]),
    )
    .try_load()
    .err()
    .expect("refused");
    assert!(rules(&report).contains(&"missing field"), "{report}");

    let report = placing(
        game().blueprint("tree", (1, 1), json!({}), json!([tree()])),
        json!([
            placement("Tree", "tree", 2.0, 2.0, "none"),
            placement("tree", "tree", 4.0, 2.0, "none"),
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
            area["blueprint"]["variables"] = json!({ "x": { "type": "number", "default": 1.0 } });
        })
        .try_load()
        .err()
        .expect("refused");

    assert!(rules(&report).contains(&"out of range"), "{report}");
}

#[test]
fn a_blueprint_holding_itself_is_refused() {
    let report = placing(
        game().blueprint(
            "loop",
            (2, 1),
            json!({}),
            json!([
                scenery(json!([[0]])),
                placement("Loop", "loop", 1.0, 0.0, "none"),
            ]),
        ),
        json!([placement("Loop", "loop", 2.0, 2.0, "none")]),
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
fn a_scenerys_bounds_cover_exactly_its_tiles_however_it_is_flipped() {
    for flip in [Flip::None, Flip::Horizontal, Flip::Vertical, Flip::Both] {
        let tile = SceneryTile {
            tile: 338,
            flip: Flip::None,
        };
        let scenery = Scenery {
            at: Pos::new(5.0, 7.0),
            tileset: ContentId::new(0),
            tiles: vec![vec![Some(tile); 3]; 2],
            sort: None,
            flip,
            blocks: Vec::new(),
        };

        let covered = (0..2)
            .flat_map(|row| (0..3).map(move |column| (column, row)))
            .map(|(column, row)| walk::tile_square(&scenery, column, row))
            .reduce(|a, b| a.union(&b));

        assert_eq!(covered, Some(scenery.bounds()), "{flip:?}");
    }
}

#[test]
fn a_placed_blueprint_naming_an_unknown_file_is_reported_once_at_the_blueprint() {
    let mut bad = scenery(json!([[0]]));
    bad["tileset"] = json!("nowhere.tileset");
    let report = placing(
        game().blueprint("bad", (1, 1), json!({}), json!([bad])),
        json!([placement("Bad", "bad", 2.0, 2.0, "none")]),
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
            game().blueprint("rock", (1, 1), variables, json!([holed_rock()])),
            json!([with_values(
                placement("Rock", "rock", 2.0, 2.0, "none"),
                values
            )]),
        )
        .load();
        area::placed(&content, start(&content))
            .iter()
            .map(|part| scenery_at(&part.module).0)
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
    let game = game().blueprint(
        "rock",
        (1, 1),
        json!({ "x": { "type": "number" }, "set": { "type": "row", "table": "tileset" } }),
        json!([{
            "type": "scenery",
            "at": { "x": { "var": "x" }, "y": 0.0 },
            "tileset": { "var": "set" },
            "tiles": [[0]],
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
        "type": "scenery",
        "at": { "x": { "var": "x" }, "y": 0.0 },
        "tileset": path("tileset", "start_ground"),
        "tiles": [[0]],
    })
}

fn with_values(mut placement: Value, values: Value) -> Value {
    placement["values"] = values;
    placement
}

fn tree() -> Value {
    let mut trunk = scenery(json!([[0]]));
    trunk["sort"] = json!(0.5);
    trunk
}

fn grove() -> Value {
    json!([
        scenery(json!([[0, 0]])),
        placement("Tree", "tree", 2.0, 1.0, "none"),
    ])
}

fn scenery(tiles: Value) -> Value {
    json!({
        "type": "scenery",
        "at": { "x": 0.0, "y": 0.0 },
        "tileset": path("tileset", "start_ground"),
        "tiles": tiles,
    })
}

fn scenery_at_x(x: f32) -> Value {
    let mut scenery = scenery(json!([[0]]));
    scenery["at"]["x"] = json!(x);
    scenery
}

fn placement(label: &str, blueprint: &str, x: f32, y: f32, flip: &str) -> Value {
    let mut node = json!({
        "type": "blueprint",
        "label": label,
        "blueprint": path("blueprint", blueprint),
        "at": { "x": x, "y": y },
    });
    if flip != "none" {
        node["flip"] = json!(flip);
    }
    node
}

/// A tileset of two tiles, and a rug two tiles wide whose left cell paints its second one into
/// the ground layer, its right cell left empty.
fn rugs(game: Game) -> Game {
    game.row(
        "tileset",
        "rugs",
        json!({ "tile": testkit::TILE, "tiles": {} }),
    )
    .part(
        "tileset",
        "rugs",
        "image.png",
        png(2 * testkit::TILE, testkit::TILE),
    )
    .blueprint(
        "rug",
        (2, 1),
        json!({}),
        json!([{ "type": "ground_layer", "name": "ground" }]),
    )
    .part(
        "blueprint",
        "rug",
        "palette.csv",
        format!("tileset,tile,flip\n{},1,\n", path("tileset", "rugs")),
    )
    .part("blueprint", "rug", "layers/ground.csv", "0,\n")
}

/// Blueprints nested from `level{top}` down to the deepest level allowed, placed in the start.
fn nest(top: usize) -> Game {
    let mut game = game();
    for depth in top..=area::blueprint::MAX_DEPTH {
        let nodes = if depth == area::blueprint::MAX_DEPTH {
            json!([scenery(json!([[0]]))])
        } else {
            let next = format!("level{}", depth + 1);
            json!([placement(&next, &next, 0.0, 0.0, "none")])
        };
        game = game.blueprint(&format!("level{depth}"), (1, 1), json!({}), nodes);
    }
    let top = format!("level{top}");
    placing(game, json!([placement(&top, &top, 2.0, 2.0, "none")]))
}

/// The start area with `nodes` after its ground layer in its scene.
fn placing(game: Game, nodes: Value) -> Game {
    game.edit_scene("area", "start", |scene| {
        let scene = scene.as_array_mut().expect("a scene");
        scene.truncate(1);
        scene.extend(nodes.as_array().expect("nodes").iter().cloned());
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

fn scenery_at(module: &AreaModule) -> (Pos<Meters>, Flip) {
    match module {
        AreaModule::Scenery(scenery) => (scenery.at, scenery.flip),
        other => panic!(
            "not scenery: {}",
            serde_json::to_value(other).expect("json")
        ),
    }
}
