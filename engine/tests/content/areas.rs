use std::io;
use std::sync::Arc;

use engine::core::content::{
    Content, ContentFolder, FileHash, FolderEntry, LoadPurpose, LoadReport, MemoryFolder,
    SETTINGS_FILE, load, version, write,
};
use engine::core::ground::{
    self as ground, GroundSize, Heightfield, MIN_SIDE, PaintMap, ShareMap, TileAt, WaterSurface,
};
use engine::core::math::{Pos, Rect, Size};
use engine::core::units::{GROUND_ORIGIN, Meters};
use engine::core::vfx::shader::Stage;
use engine::systems::area::scenery::{Sources, prop_transform};
use engine::systems::area::{self, AreaDef, AreaId};
use serde_json::{Value, json};
use testkit::{Game, HEIGHTS, Map, path};

use crate::support::game;

/// The one tile of a ground up to a tile wide, as its file holds it.
fn tile(heights: &Heightfield) -> Vec<u8> {
    heights
        .encode_tile(TileAt { column: 0, row: 0 })
        .expect("heights that aren't flat")
        .to_vec()
}

/// Eight metres square, flat in the west, rising half a metre per metre through the middle and
/// two metres per metre in the east.
fn hill() -> Heightfield {
    Heightfield::from_fn(
        GroundSize {
            width: 8,
            height: 8,
        },
        HEIGHTS,
        |column, _| {
            let x = column as f32 / ground::SAMPLES_PER_METER as f32 + GROUND_ORIGIN.x;
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
        .part("area", "hill", "height/0_0.png", tile(&hill()))
        .part("area", "hill", "height_overview.png", overview(&hill()))
        .load();
    let hill = area::load(&content, area_id(&content, "hill"));
    let _held = hill.hold(hill.ground_size().each_tile());
    let walks = |x: f32, y: f32| hill.mesh().walkable(Pos::new(x, y));

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
    let _held = field_area.hold(field_area.ground_size().each_tile());

    assert!(!field_area.mesh().walkable(Pos::new(4.0, 4.0)));
    assert!(field_area.mesh().walkable(Pos::new(5.0, 4.0)));
    assert!(
        field_area.mesh().walkable(Pos::new(1.0, 6.0)),
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
fn a_prop_stands_on_the_ground_under_it_sunk_as_deep_as_it_is_placed() {
    let content = game()
        .prop("boulder", json!({ "type": "none" }))
        .area(
            "hill",
            &Map::open(8, 8),
            json!([
                { "type": "prop", "prop": path("prop", "boulder"), "at": { "x": 2.0, "y": 3.0 } },
                { "type": "prop", "prop": path("prop", "boulder"), "at": { "x": 7.0, "y": 3.0 }, "sunk": 1.5 },
            ]),
        )
        .part("area", "hill", "height/0_0.png", tile(&hill()))
        .part("area", "hill", "height_overview.png", overview(&hill()))
        .load();
    let hill_area = area::load(&content, area_id(&content, "hill"));

    let standing: Vec<f32> = hill_area
        .props
        .iter()
        .map(|placed| prop_transform(&hill_area.heights, placed).translation.y)
        .collect();
    let expected = [0.0, 3.0 - 1.5];
    assert!(
        standing
            .iter()
            .zip(expected)
            .all(|(stands, ground)| (stands - ground).abs() < 0.01),
        "a prop stands on the ground, a sunk one that far under it: {standing:?}"
    );
}

#[test]
fn a_walk_leads_to_every_point_the_ground_holds() {
    let rocks: Vec<Value> = [(2.0, 2.0), (5.5, 3.0), (3.0, 6.0), (6.5, 6.5), (1.0, 4.5), (4.2, 4.6)]
        .iter()
        .map(|(x, y)| json!({ "type": "prop", "prop": path("prop", "boulder"), "at": { "x": x, "y": y } }))
        .collect();
    let content = game()
        .prop("boulder", json!({ "type": "circle", "radius": 0.4 }))
        .area("field", &Map::open(9, 9), json!(rocks))
        .load();
    let field = area::load(&content, area_id(&content, "field"));
    let _held = field.hold(field.ground_size().each_tile());
    let mesh = field.mesh();
    let from = Pos::new(0.5, 0.5);
    assert!(mesh.walkable(from));
    let step = 0.02;
    let held: Vec<Pos<Meters>> = (0..(9.0 / step) as i32)
        .flat_map(|column| (0..(9.0 / step) as i32).map(move |row| (column, row)))
        .map(|(column, row)| Pos::new(column as f32 * step - 0.5, row as f32 * step - 0.5))
        .filter(|&point| mesh.walkable(point) && mesh.connects(from, point))
        .collect();
    assert!(held.len() > 100_000);
    let unreachable: Vec<Pos<Meters>> = held
        .into_iter()
        .filter(|&point| mesh.route(from, point).is_none())
        .collect();
    assert!(unreachable.is_empty(), "{unreachable:?}");
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
fn fog_lies_over_the_ground_where_it_is_placed_and_no_further_than_the_ground_reaches() {
    let bank = |x: f32, w: f32, thickest: f32| {
        json!({
            "type": "fog",
            "rect": { "x": x, "y": 2.0, "w": w, "h": 4.0 },
            "height": 12.0,
            "haze": {
                "clear": 5.0, "thick": 30.0, "thickest": thickest, "moist_closer": 0.2,
                "rain_closer": 0.1, "dry_air": 0.001, "moist_air": 0.004,
            },
        })
    };
    let content = game()
        .area("meadow", &Map::open(16, 16), json!([bank(1.0, 6.0, 0.5)]))
        .load();
    let meadow = area::load(&content, area_id(&content, "meadow"));
    let lying: Vec<(Rect<Meters>, Meters, f32)> = meadow
        .fogs
        .iter()
        .map(|fog| (fog.rect, fog.height, fog.haze.thickest))
        .collect();
    assert_eq!(
        lying,
        [(
            Rect::new(Pos::new(1.0, 2.0), Size::new(6.0, 4.0)),
            Meters(12.0),
            0.5
        )]
    );

    for (rule, refused, told) in [
        (
            "outside the terrain",
            bank(12.0, 8.0, 0.5),
            "lands outside the terrain at 20, 6",
        ),
        ("out of range", bank(1.0, 6.0, 1.5), ""),
    ] {
        let report = game()
            .area("meadow", &Map::open(16, 16), json!([refused.clone()]))
            .try_load()
            .err()
            .expect("refused");
        assert!(rules(&report).contains(&rule), "{refused}: {report}");
        assert!(report.to_string().contains(told), "{refused}: {report}");
    }
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
fn ground_reads_in_blocks_and_overviews_that_follow_its_edits_each_tile_versioned_by_its_values() {
    let size = GroundSize {
        width: 512,
        height: 256,
    };
    let pixel = |column: u32, row: u32| ((column / 7 + row / 5) % 4) as u8;
    let paint = PaintMap::from_fn(size, pixel);
    let (west, east) = (TileAt { column: 0, row: 0 }, TileAt { column: 1, row: 0 });
    let read = [paint.version(west), paint.version(east)];
    let mut edited = paint.clone();
    edited.set(596, 4, 9);
    assert_eq!(
        [paint.version(west), paint.version(east)],
        read,
        "the copy set apart"
    );
    assert_eq!(edited.version(west), read[0], "a tile nothing was set in");
    assert_ne!(edited.version(east), read[1], "the tile set");

    assert_eq!(
        edited.block((-1, -1), 3, 3),
        [
            [pixel(0, 0), pixel(0, 0), pixel(1, 0)],
            [pixel(0, 0), pixel(0, 0), pixel(1, 0)],
            [pixel(0, 1), pixel(0, 1), pixel(1, 1)],
        ]
        .concat(),
        "from before the start, the nearest past the edge"
    );
    assert_eq!(
        edited.block((1023, 10), 3, 1),
        [pixel(1023, 10); 3],
        "past the far edge"
    );
    assert_eq!(
        edited.block((595, 4), 3, 1),
        [pixel(595, 4), 9, pixel(597, 4)]
    );

    assert_eq!(
        edited.overview_at(&[74], &[0]),
        [9],
        "the overview follows the edit"
    );
    assert_eq!(paint.overview_at(&[74], &[0]), [pixel(596, 4)]);
    assert_eq!(edited.overview_span(east), ((64, 0), 64, 64));
    assert_eq!(
        Heightfield::flat(size, HEIGHTS).overview_span(west),
        ((0, 0), 65, 65),
        "heights' tiles share the samples on their seam"
    );
}

#[test]
fn an_areas_heights_paint_water_and_rain_write_back_as_they_were_read() {
    let size = GroundSize {
        width: 8,
        height: 8,
    };
    let paint = PaintMap::from_fn(size, |column, row| u8::from(column < 4 && row < 4));
    let pond = WaterSurface::from_fn(size, HEIGHTS, |column, _| {
        Meters(if column < 6 { 1.5 } else { 0.0 })
    });
    let pond_tile = pond
        .encode_tile(TileAt { column: 0, row: 0 })
        .expect("water lies on the tile")
        .to_vec();
    let pond_overview = pond.encode_overview().expect("water lies somewhere");
    let rain = ShareMap::from_fn(3, 2, |across, down| across * (1.0 - down));
    let game = game()
        .area("hill", &Map::open(8, 8), json!([]))
        .part("area", "hill", "height/0_0.png", tile(&hill()))
        .part("area", "hill", "paint/0_0.png", tile_paint(&paint))
        .part("area", "hill", "water/0_0.png", pond_tile.clone())
        .part("area", "hill", "height_overview.png", overview(&hill()))
        .part("area", "hill", "paint_overview.png", overview_paint(&paint))
        .part("area", "hill", "water_overview.png", pond_overview.clone())
        .part("area", "hill", "rain.png", rain.encode());
    let out = tempfile::tempdir().expect("a folder");
    let registry = game.registry();
    let content = game.load();
    let read = area::load(&content, area_id(&content, "hill"));
    assert_eq!(read.water, pond);
    assert_eq!(read.rain.as_ref(), Some(&rain));

    write(registry, content, out.path()).expect("written");

    let hill_area = out.path().join(path("area", "hill"));
    assert_eq!(
        std::fs::read(hill_area.join("water/0_0.png")).expect("the water"),
        pond_tile
    );
    assert_eq!(
        std::fs::read(hill_area.join("water_overview.png")).expect("the water overview"),
        pond_overview
    );
    assert_eq!(
        std::fs::read(hill_area.join("rain.png")).expect("the rain map"),
        rain.encode()
    );
    assert_eq!(
        std::fs::read(hill_area.join("height/0_0.png")).expect("the heights"),
        tile(&hill())
    );
    assert_eq!(
        std::fs::read(hill_area.join("paint/0_0.png")).expect("the paint"),
        tile_paint(&paint)
    );
    assert_eq!(
        std::fs::read(hill_area.join("height_overview.png")).expect("the height overview"),
        overview(&hill())
    );
    assert_eq!(
        std::fs::read(hill_area.join("paint_overview.png")).expect("the paint overview"),
        overview_paint(&paint)
    );
    let start = out.path().join(path("area", "start"));
    assert!(
        !start.join("height").exists() && !start.join("height_overview.png").exists(),
        "a flat area keeps no height tiles and no overview"
    );
    assert!(
        !start.join("water").exists() && !start.join("rain.png").exists(),
        "a dry area keeps no water tiles, and one where rain falls alike no rain map"
    );
    let played = crate::support::game()
        .area(
            "meadow",
            &Map::open(MIN_SIDE as usize, MIN_SIDE as usize),
            json!([]),
        )
        .load_to_play();
    let flat = area::load(&played, area_id(&played, "meadow"));
    assert!(
        flat.heights
            .size()
            .each_tile()
            .all(|at| flat.heights.is_loaded(at) && flat.paint.is_loaded(at)),
        "a flat area's ground is loaded at once, with nothing to read"
    );

    // A ground wider than a tile is held tile by tile, the seams shared, and reads back whole.
    let wide = GroundSize {
        width: 300,
        height: 20,
    };
    let mut ridge = Heightfield::flat(wide, HEIGHTS);
    for column in 0..=wide.width * ground::SAMPLES_PER_METER {
        for row in 0..=wide.height * ground::SAMPLES_PER_METER {
            ridge.set_sample(column, row, Meters((column % 7) as f32 * 0.25));
        }
    }
    let striped = PaintMap::from_fn(wide, |column, _| (column / 100 % 2) as u8);
    let ridge_game = || {
        wide.each_tile().fold(
            crate::support::game()
                .area("ridge", &Map::open(300, 20), json!([]))
                .part("area", "ridge", "height_overview.png", overview(&ridge))
                .part(
                    "area",
                    "ridge",
                    "paint_overview.png",
                    overview_paint(&striped),
                ),
            |game, at| {
                let game = match ridge.encode_tile(at) {
                    Some(bytes) => game.part(
                        "area",
                        "ridge",
                        &format!("height/{}.png", at.key()),
                        bytes.to_vec(),
                    ),
                    None => game,
                };
                match striped.encode_tile(at) {
                    Some(bytes) => game.part(
                        "area",
                        "ridge",
                        &format!("paint/{}.png", at.key()),
                        bytes.to_vec(),
                    ),
                    None => game,
                }
            },
        )
    };
    let content = ridge_game().load();
    let read = area::load(&content, area_id(&content, "ridge"));
    assert_eq!(wide.tiles(), (2, 1));
    assert_eq!(read.heights, ridge);
    assert_eq!(read.paint, striped);
    let seam = Pos::new(255.5, 3.0);
    assert_eq!(read.heights.height_at(seam), ridge.height_at(seam));

    // Its tiles hold levels across the area's range: twice the range is the same land twice as
    // high.
    let content = ridge_game()
        .edit("area", "ridge", |ridge| {
            ridge["height_range"] = json!({ "lowest": -16.0, "highest": 32.0 });
        })
        .load();
    let stretched = area::load(&content, area_id(&content, "ridge"));
    for at in [Pos::new(3.5, 3.0), seam] {
        let (was, is) = (ridge.height_at(at).0, stretched.heights.height_at(at).0);
        assert!(
            (is - 2.0 * was).abs() < 1e-3,
            "{at:?}: {was} m stretched to {is} m"
        );
    }
}

fn tile_paint(paint: &PaintMap) -> Vec<u8> {
    paint
        .encode_tile(TileAt { column: 0, row: 0 })
        .expect("paint that isn't plain")
        .to_vec()
}

fn overview(heights: &Heightfield) -> Vec<u8> {
    heights.encode_overview().expect("heights that aren't flat")
}

fn overview_paint(paint: &PaintMap) -> Vec<u8> {
    paint.encode_overview().expect("paint that isn't plain")
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

#[test]
fn a_grounds_cover_names_a_cover_stage_shader_fills_only_what_it_declares_and_grows_no_thicker_than_drawn()
 {
    let green = json!({ "from": [60, 120, 50, 255], "to": [90, 160, 70, 255] });
    let blade = |width: f32, ratio: f32| json!({ "shape": { "type": "blade", "width": width, "height": 0.3 }, "ratio": ratio, "color": green });
    let pebble = |ratio: f32| json!({ "shape": { "type": "prop", "prop": path("prop", "pebble") }, "ratio": ratio, "size": { "min": 0.2, "max": 0.4 }, "color": green });
    let cover = |shader: &str, params: Value, body: Value| {
        let mut cover = json!({ "shader": path("vfx_shader", shader), "params": params });
        for (key, value) in body.as_object().expect("a cover body") {
            cover[key] = value.clone();
        }
        cover
    };
    let game = || {
        game()
            .shader_on("blades", Stage::Cover)
            .edit("vfx_shader", "blades", |row| {
                row["params"] =
                    json!([{ "name": "tip", "kind": "color", "default": [255, 255, 255, 255] }]);
            })
            .shader("flash")
            .prop("pebble", json!({ "type": "none" }))
            .area("meadow", &Map::open(8, 8), json!([]))
    };
    let pieces = |pieces: Vec<Value>| json!({ "pieces": pieces });
    let cases = [
        (
            "a shader of another stage",
            cover("flash", json!({}), pieces(vec![blade(0.1, 0.5)])),
        ),
        (
            "unknown shader parameter",
            cover("blades", json!({ "glow": 1.0 }), pieces(vec![])),
        ),
        (
            "wrong parameter kind",
            cover("blades", json!({ "tip": 1.0 }), pieces(vec![])),
        ),
        (
            "out of range",
            cover("blades", json!({}), pieces(vec![blade(0.1, 1.5)])),
        ),
        (
            "too thick",
            cover("blades", json!({}), pieces(vec![blade(0.02, 1.0)])),
        ),
        (
            "too thick",
            cover("blades", json!({}), pieces(vec![pebble(1.0)])),
        ),
        (
            "too many pieces",
            cover("blades", json!({}), pieces(vec![blade(0.1, 0.1); 9])),
        ),
    ];
    for (rule, grown) in cases {
        let report = game()
            .edit("terrain", testkit::TURF, |row| row["cover"] = grown.clone())
            .try_load()
            .err()
            .expect("refused");
        assert!(rules(&report).contains(&rule), "{grown}: {report}");
    }
    for body in [
        json!({ "sheet": "ground" }),
        json!({ "sheet": "water" }),
        json!({ "seed": 42, "pieces": [blade(0.1, 0.8), pebble(0.001)] }),
    ] {
        game()
            .edit("terrain", testkit::TURF, |row| {
                row["cover"] = cover(
                    "blades",
                    json!({ "tip": [120, 200, 90, 255] }),
                    body.clone(),
                )
            })
            .load();
    }
}

#[test]
fn an_areas_scenery_stands_again_only_when_a_file_it_is_made_from_changes() {
    let sources = |game: Game| {
        let content = game.load();
        let id = content
            .by_path::<AreaDef>(&path("area", "start"))
            .expect("the area loads");
        let picture =
            area::picture(&content, area::Pictured::Area(id), area::Kept::All).expect("pictured");
        Sources::of(&content, &picture.area)
    };
    let as_it_was = sources(game());
    let cases = [
        (
            "an item no area shows",
            game().item("cap", json!({ "kind": { "type": "resource" } })),
            false,
        ),
        (
            "the colour of a terrain the area lays",
            game().row(
                "terrain",
                testkit::TURF,
                json!({
                    "name": "Turf",
                    "color": [120, 150, 60, 255],
                    "surface": path("surface", testkit::TURF),
                    "walk": "walkable",
                }),
            ),
            true,
        ),
    ];
    for (case, changed, stands_again) in cases {
        assert_eq!(sources(changed) != as_it_was, stands_again, "{case}");
    }
}

/// A folder vouching for every file it holds but those under `unvouched`, as one whose files
/// loaded whole elsewhere does.
struct Vouching {
    folder: MemoryFolder,
    unvouched: Vec<String>,
}

impl ContentFolder for Vouching {
    fn name(&self) -> String {
        self.folder.name()
    }

    fn entries(&self, dir: &str) -> Vec<FolderEntry> {
        self.folder.entries(dir)
    }

    fn read(&self, file: &str) -> io::Result<Vec<u8>> {
        self.folder.read(file)
    }

    fn checked(&self, file: &str) -> Option<FileHash> {
        let unvouched = self
            .unvouched
            .iter()
            .any(|path| file == path || file.starts_with(&format!("{path}/")));
        if unvouched {
            return None;
        }
        Some(FileHash::of(&self.folder.read(file).ok()?))
    }
}

#[test]
fn a_check_holds_an_area_to_the_rules_its_ground_and_paths_need_again_only_when_it_could_break_them()
 {
    let split = Map::of(&[
        "......#.....",
        "......#.....",
        "......#.....",
        "......#.....",
    ]);
    let open = Map::of(&["........", "........", "........", "........"]);
    let stranded = json!([
        { "type": "resident", "npc": path("npc", "hermit"), "at": { "x": 9.0, "y": 2.0 } },
    ]);
    let game = Game::new()
        .npc("hermit", 10.0, json!([]))
        .area("field", &split, stranded)
        .area("meadow", &open, json!([]))
        .hero("field", 2.0, 2.0);
    let refuses = |unvouched: &[&str]| {
        let folder = Vouching {
            folder: game.folder(),
            unvouched: unvouched.iter().map(|path| path.to_string()).collect(),
        };
        let loaded = load(
            game.registry(),
            Arc::new(folder),
            version::build(),
            "the test game did not load",
            LoadPurpose::Check,
        );
        loaded.err().is_some_and(|report| {
            report
                .errors
                .iter()
                .any(|error| error.problem.rule == "unreachable")
        })
    };
    let field = path("area", "field");
    let meadow = path("area", "meadow");
    let hermit = path("npc", "hermit");

    assert!(refuses(&[field.as_str()]), "the area itself changed");
    assert!(refuses(&[hermit.as_str()]), "what the area names changed");
    assert!(refuses(&[SETTINGS_FILE]), "the game's settings changed");
    assert!(!refuses(&[meadow.as_str()]), "only another area changed");
    assert!(
        !refuses(&[]),
        "nothing changed since the folder loaded whole"
    );
}
