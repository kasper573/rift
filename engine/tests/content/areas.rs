use engine::core::content::{Content, LoadReport, write};
use engine::core::math::{Pos, Rect, Size};
use engine::core::terrain::{self, Heightfield, Paint, TerrainSize};
use engine::core::units::{GROUND_ORIGIN, Meters};
use engine::systems::area::{self, AreaDef, AreaId};
use serde_json::{Value, json};
use testkit::{Game, Map, path};

use crate::support::game;

/// Eight metres square, flat in the west, rising half a metre per metre through the middle and
/// two metres per metre in the east.
fn hill() -> Heightfield {
    Heightfield::from_fn(
        TerrainSize {
            width: 8,
            height: 8,
        },
        |column, _| {
            let x = column as f32 / terrain::SAMPLES_PER_METER as f32 + GROUND_ORIGIN.x;
            Meters(match x {
                x if x <= 4.0 => 0.0,
                x if x <= 6.0 => (x - 4.0) * 0.5,
                x => 1.0 + (x - 6.0) * 2.0,
            })
        },
    )
}

#[test]
fn the_ground_walks_where_it_is_flat_enough_and_painted_walkable() {
    let map = Map::of(&[
        "........", "........", "........", "...#....", "........", "........", "........",
        "........",
    ]);
    let content = game()
        .area("hill", &map, json!([]))
        .part("area", "hill", "terrain/height.png", hill().encode())
        .load();
    let hill = area::load(&content, area_id(&content, "hill"));
    let walks = |x: f32, y: f32| hill.mesh.walkable(Pos::new(x, y));

    assert!(walks(2.0, 2.0), "flat ground");
    assert!(walks(4.0, 2.0), "a gentle slope");
    assert!(!walks(6.0, 2.0), "a slope past the area's max");
    assert!(
        !walks(3.0, 3.0),
        "ground painted with a material that blocks"
    );
    let halfway = hill.heights.height_at(Pos::new(5.0, 1.0));
    assert!((halfway.0 - 0.5).abs() < 0.001, "{halfway:?}");
}

#[test]
fn a_props_footprint_blocks_the_ground_under_it_and_a_scatter_strews_the_same_props_for_its_seed() {
    let field = || {
        game()
            .prop("boulder", json!({ "type": "circle", "radius": 0.4 }))
            .prop("tuft", json!({ "type": "none" }))
            .area(
                "field",
                &Map::open(8, 8),
                json!([
                    { "type": "prop", "prop": path("prop", "boulder"), "at": { "x": 4.0, "y": 4.0 } },
                    { "type": "prop", "prop": path("prop", "tuft"), "at": { "x": 1.0, "y": 6.0 } },
                    { "type": "scatter", "props": [path("prop", "tuft")], "region": { "x": -0.5, "y": -0.5, "w": 8.0, "h": 8.0 }, "per_square_meter": 0.5, "seed": 7 },
                ]),
            )
    };
    let content = field().load();
    let field_area = area::load(&content, area_id(&content, "field"));

    assert!(!field_area.mesh.walkable(Pos::new(4.0, 4.0)));
    assert!(field_area.mesh.walkable(Pos::new(5.0, 4.0)));
    assert!(
        field_area.mesh.walkable(Pos::new(1.0, 6.0)),
        "a tuft takes no ground"
    );
    assert_eq!(field_area.props.len(), 2 + 32);
    let region = Rect::new(GROUND_ORIGIN, Size::new(8.0, 8.0));
    assert!(field_area.props.iter().all(|prop| region.contains(prop.at)));

    let again = field().load();
    let again = area::load(&again, area_id(&again, "field"));
    assert_eq!(again.props, field_area.props, "the same seed strews alike");
}

#[test]
fn an_area_is_held_to_its_budgets_of_lights_and_props() {
    let light = |x: f32| json!({ "type": "light", "at": { "x": x, "y": 2.0 }, "color": [255, 200, 120], "intensity": 4.0, "radius": 3.0, "height": 2.0 });
    let lights: Vec<Value> = (0..9).map(|i| light(i as f32 + 1.0)).collect();
    let scatter = |density: f32| json!({ "type": "scatter", "props": [path("prop", "tuft")], "region": { "x": -0.5, "y": -0.5, "w": 16.0, "h": 16.0 }, "per_square_meter": density, "seed": 1 });
    let cases: [(&str, Value); 3] = [
        ("too many lights", Value::Array(lights)),
        ("too many props", json!([scatter(2.0)])),
        (
            "too many props",
            Value::Array((0..5).map(|_| scatter(1.95)).collect()),
        ),
    ];
    for (rule, modules) in cases {
        let report = game()
            .prop("tuft", json!({ "type": "none" }))
            .area("meadow", &Map::open(16, 16), modules.clone())
            .try_load()
            .err()
            .expect("refused");
        assert!(rules(&report).contains(&rule), "{modules}: {report}");
    }

    let content = game()
        .prop("tuft", json!({ "type": "none" }))
        .area("meadow", &Map::open(16, 16), json!([scatter(1.9)]))
        .load();
    assert_eq!(
        area::load(&content, area_id(&content, "meadow"))
            .props
            .len(),
        487
    );
}

#[test]
fn a_footprint_is_refused_when_it_crosses_itself_or_was_fitted_to_a_model_since_changed() {
    let polygon = |points: Value, fitted_to: Option<&str>| {
        let mut footprint = json!({ "type": "polygon", "points": points });
        if let Some(hash) = fitted_to {
            footprint["fitted_to"] = json!(hash);
        }
        footprint
    };
    let bowtie = json!([{ "x": -1.0, "y": -1.0 }, { "x": 1.0, "y": 1.0 }, { "x": 1.0, "y": -1.0 }, { "x": -1.0, "y": 1.0 }]);
    let square = json!([{ "x": -1.0, "y": -1.0 }, { "x": 1.0, "y": -1.0 }, { "x": 1.0, "y": 1.0 }, { "x": -1.0, "y": 1.0 }]);
    let cases = [
        ("polygon crosses itself", polygon(bowtie, None)),
        (
            "stale footprint",
            polygon(square.clone(), Some("0000000000000000")),
        ),
    ];
    for (rule, footprint) in cases {
        let report = game()
            .prop("hut", footprint)
            .try_load()
            .err()
            .expect("refused");
        let found: Vec<&str> = rules(&report);
        assert!(found.contains(&rule), "{rule}: {report}");
    }

    assert!(game().prop("hut", polygon(square, None)).try_load().is_ok());
}

#[test]
fn an_areas_heights_and_paint_write_back_as_they_were_read() {
    let size = TerrainSize {
        width: 8,
        height: 8,
    };
    let paint = Paint::from_fn(size, |column, row| {
        if column < 4 && row < 4 {
            [0, 0, 255, 0]
        } else {
            [255, 0, 0, 0]
        }
    });
    let game = game()
        .area("hill", &Map::open(8, 8), json!([]))
        .part("area", "hill", "terrain/height.png", hill().encode())
        .part("area", "hill", "terrain/paint.png", paint.encode());
    let out = tempfile::tempdir().expect("a folder");

    write(game.registry(), game.load(), out.path()).expect("written");

    let hill_area = out.path().join(path("area", "hill"));
    assert_eq!(
        std::fs::read(hill_area.join("terrain/height.png")).expect("the heights"),
        hill().encode()
    );
    assert_eq!(
        std::fs::read(hill_area.join("terrain/paint.png")).expect("the paint"),
        paint.encode()
    );
    let start = out.path().join(path("area", "start"));
    assert!(
        !start.join("terrain/height.png").exists(),
        "a flat area keeps no height file"
    );
}

#[test]
fn ground_nothing_reaches_fails_the_load_pointing_at_what_stands_there() {
    let split = Map::of(&[
        "......#.....",
        "......#.....",
        "......#.....",
        "......#.....",
        "......#.....",
        "......#.....",
        "......#.....",
        "......#.....",
    ]);
    let stranded = json!([
        { "type": "resident", "npc": path("npc", "hermit"), "at": { "x": 9.0, "y": 4.0 } },
        { "type": "population", "npc": path("npc", "hermit"), "count": 1, "roams": { "x": 8.0, "y": 1.0, "w": 3.0, "h": 3.0 } },
        { "type": "population", "npc": path("npc", "hermit"), "count": 1, "roams": { "x": 1.0, "y": 1.0, "w": 10.0, "h": 3.0 } },
    ]);
    let report = Game::new()
        .npc("hermit", 10.0, json!([]))
        .area("field", &split, stranded)
        .hero("field", 2.0, 4.0)
        .try_load()
        .err()
        .expect("refused");

    let unreachable: Vec<(&str, &str, Option<&str>)> = report
        .errors
        .iter()
        .filter(|error| error.problem.rule == "unreachable")
        .map(|error| {
            (
                error.at.pointer(),
                error.problem.found.as_str(),
                error.problem.nearest.as_deref(),
            )
        })
        .collect();
    let [
        (resident, hermit, near_resident),
        (population, strays, near_population),
    ] = unreachable.as_slice()
    else {
        panic!("{report}");
    };
    assert_eq!(
        (*resident, *hermit),
        ("/0/at", "hermit.npc.json at 9.00, 4.00")
    );
    assert_eq!(*population, "/1");
    assert!(strays.starts_with("hermit.npc.json at "), "{strays}");
    let west_of_the_wall = |nearest: Option<&str>| {
        nearest
            .and_then(|nearest| nearest.split(',').next()?.trim().parse::<f32>().ok())
            .is_some_and(|x| x < 5.5)
    };
    assert!(
        west_of_the_wall(*near_resident) && west_of_the_wall(*near_population),
        "{near_resident:?} {near_population:?}"
    );
}

fn area_id(content: &Content, name: &str) -> AreaId {
    content
        .by_path::<AreaDef>(&path("area", name))
        .expect("the area loads")
}

fn rules(report: &LoadReport) -> Vec<&str> {
    report
        .errors
        .iter()
        .map(|error| error.problem.rule)
        .collect()
}
