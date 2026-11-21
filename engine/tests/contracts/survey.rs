use std::path::Path;
use std::sync::Arc;

use bevy::color::LinearRgba;
use bevy::math::Vec2;
use engine::core::content::{Content, FilesystemFolder, FolderReader, LoadPurpose, version};
use engine::core::ground::{GroundSize, WaterSurface};
use engine::core::render::world::gpu::impostor::{PLAN_PIXELS, PlanFromAbove};
use engine::core::units::{MeterSize, Meters};
use engine::systems::area::prop::PropDef;
use engine::systems::area::{self, AreaDef};
use engine::systems::area_map::bake;
use engine::systems::area_map::baked::{BakedMap, BakedMapSquare, MapPictureFile, MapTileKey};
use engine::systems::area_map::pictures::{MAP_PICTURE_PIXELS, MapDetail, MapTile};
use engine::systems::area_map::survey::{PropPlans, Survey};
use serde_json::json;
use testkit::{Declared, Game, HEIGHTS, Map, path};

/// Eight pixels a metre: a square of the sharpest level is 64 m across.
const DETAIL: MapDetail = MapDetail(8.0);

#[test]
fn a_map_draws_each_terrain_the_water_and_every_prop_from_its_plan_where_they_lie() {
    let content = field(160.0);
    let survey = survey_of(&content, &hut_plans(&content));
    let square = |column| {
        survey
            .draw(MapTile {
                level: 0,
                column,
                row: 0,
            })
            .expect("the field's ground reads")
    };
    let (west, middle, east) = (square(0), square(1), square(2));

    let turf = pixel(&middle, 1, 70.0, 32.0);
    let rock = pixel(&middle, 1, 120.0, 32.0);
    assert!(
        greenness(turf) > greenness(rock) + 20,
        "the turf {turf:?} draws greener than the rock {rock:?} beside it"
    );
    let pond = pixel(&west, 0, 32.0, 32.0);
    assert!(
        pond[2] > pond[0] + 20 && pond[2] > pond[1],
        "the pond {pond:?} draws in the blue of its depths"
    );
    let hut = pixel(&east, 2, 160.0, 32.0);
    assert!(
        hut[0] > hut[1] + 60 && hut[2] > hut[1] + 60,
        "the hut {hut:?} draws in its colour from above"
    );
    let beside = pixel(&east, 2, 160.0, 54.0);
    assert!(
        beside[0] < beside[1] + 30,
        "the rock beside the hut {beside:?} shows no hut"
    );
}

#[test]
fn a_map_picture_is_drawn_again_only_where_what_it_shows_changed() {
    let keys = |content: &Content| -> Vec<(MapTile, MapTileKey)> {
        let survey = survey_of(content, &PropPlans::new());
        let bounds = loaded(content).size.bounds();
        MapTile::within(0, bounds, bounds, DETAIL)
            .into_iter()
            .map(|tile| (tile, survey.key(tile)))
            .collect()
    };
    let before = keys(&field(10.0));

    assert_eq!(
        before.len(),
        6,
        "a 192 × 128 m field is six squares of 64 m at the sharpest level"
    );
    assert_eq!(
        before,
        keys(&field(10.0)),
        "the same files give every square the same key"
    );
    let changed: Vec<(u32, u32)> = before
        .iter()
        .zip(keys(&field(150.0)))
        .filter(|((_, was), (_, now))| was != now)
        .map(|((tile, _), _)| (tile.column, tile.row))
        .collect();
    assert_eq!(
        changed,
        [(0, 0), (2, 0)],
        "moving the hut changes the keys of the squares it left and came to, and no other"
    );
}

#[test]
fn a_baked_map_stands_current_in_its_bundle_until_what_a_square_shows_changes() {
    let folder = tempfile::tempdir().expect("a scratch game folder");
    field_game(10.0).write_to(folder.path());
    let unbaked = standing_in(folder.path());
    assert!(
        unbaked.unbaked() && !unbaked.current(),
        "an area never baked has no map: {unbaked:?}"
    );

    bake_keys(folder.path());
    let baked = standing_in(folder.path());
    assert!(
        baked.current(),
        "a map baked from the area as it is, read back from its bundle, is current: {baked:?}"
    );

    field_game(150.0).write_to(folder.path());
    let moved = standing_in(folder.path());
    assert_eq!(
        (moved.missing, moved.extra),
        (0, 0),
        "moving a prop leaves every square of the map: {moved:?}"
    );
    assert!(
        moved.out_of_date > 0 && moved.out_of_date < moved.squares,
        "moving the hut puts the squares it left and came to out of date, and no other: {moved:?}"
    );
}

/// A field of turf to the west and rock to the east, a pond in its west, and a hut standing at
/// `hut_at` along it.
fn field(hut_at: f32) -> Content {
    field_game(hut_at).load()
}

/// The game of [`field`].
fn field_game(hut_at: f32) -> Game {
    let rows: Vec<String> = (0..128)
        .map(|_| format!("{}{}", ".".repeat(96), "#".repeat(96)))
        .collect();
    let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
    let size = GroundSize {
        width: 192,
        height: 128,
    };
    let pond = WaterSurface::from_fn(size, HEIGHTS, |column, row| {
        let at = Vec2::new(column as f32, row as f32) / 2.0;
        Meters(if at.distance(Vec2::new(32.0, 32.0)) < 12.0 {
            1.0
        } else {
            0.0
        })
    });
    Game::new()
        .prop("hut", json!({ "type": "box", "width": 4.0, "depth": 4.0 }))
        .area(
            "field",
            &Map::of(&rows),
            json!([{ "type": "prop", "prop": path("prop", "hut"), "at": { "x": hut_at, "y": 32.0 } }]),
        )
        .water("field", &pond)
        .hero("field", 1.0, 1.0)
        .edit_settings(|settings| settings["map_detail"] = json!(DETAIL.0))
}

/// How the field's map in the game at `folder` stands.
fn standing_in(folder: &Path) -> bake::MapStanding {
    let content = loaded_from(folder);
    let id = content.declared::<AreaDef>("field").expect("the field");
    bake::standing(&content, id).expect("the field's map stands somehow")
}

/// Writes the field's map into its bundle in the game at `folder` as a bake draws it, keyed by
/// what each square shows, each picture a stand-in its keys don't read.
fn bake_keys(folder: &Path) {
    let content = loaded_from(folder);
    let id = content.declared::<AreaDef>("field").expect("the field");
    let survey = survey_of(&content, &PropPlans::new());
    let map = BakedMap {
        squares: bake::squares_of(loaded(&content), DETAIL)
            .into_iter()
            .map(|tile| {
                let square = BakedMapSquare {
                    picture: MapPictureFile::Drawn(Arc::from(&b"a picture"[..])),
                    key: Some(survey.key(tile)),
                };
                (tile, square)
            })
            .collect(),
    };
    let reader = FolderReader::new(
        field_game(0.0).registry(),
        Arc::new(FilesystemFolder(folder.to_path_buf())),
    );
    let path = id.path(&content);
    let mut row: AreaDef = reader.read(path).expect("the field reads");
    row.map = map;
    let written = reader.write(path, &row).expect("the field writes");
    for (file, bytes) in written.files {
        std::fs::create_dir_all(folder.join(&file).parent().expect("a folder")).expect("folders");
        std::fs::write(folder.join(file), bytes).expect("a file of the field");
    }
}

fn loaded_from(folder: &Path) -> Content {
    engine::core::content::load(
        field_game(0.0).registry(),
        Arc::new(FilesystemFolder(folder.to_path_buf())),
        version::build(),
        "the field did not load",
        LoadPurpose::Check,
    )
    .unwrap_or_else(|report| panic!("{report}"))
}

/// The hut seen from above: a magenta square 4 m across.
fn hut_plans(content: &Content) -> PropPlans {
    let hut = content.declared::<PropDef>("hut").expect("the hut");
    let side = PLAN_PIXELS as usize;
    let reach = 3.0;
    let cover = (0..side * side)
        .map(|at| {
            let texel = |along: usize| ((along as f32 + 0.5) / side as f32 * 2.0 - 1.0) * reach;
            let (x, z) = (texel(at % side), texel(at / side));
            f32::from(u8::from(x.abs() < 2.0 && z.abs() < 2.0))
        })
        .collect();
    PropPlans::from([(
        hut,
        Arc::new(PlanFromAbove {
            middle: Vec2::ZERO,
            reach: Meters(reach),
            cover,
            color: LinearRgba::rgb(0.8, 0.05, 0.6),
        }),
    )])
}

fn survey_of(content: &Content, plans: &PropPlans) -> Survey {
    Survey::new(content, loaded(content), DETAIL, plans)
}

fn loaded(content: &Content) -> &area::Area {
    let id = content.declared::<AreaDef>("field").expect("the field");
    area::loaded(content, id).expect("the field is loaded")
}

/// The colour the picture `pixels` of the square in `column` of the first row shows at `(x, y)`.
fn pixel(pixels: &[u8], column: u32, x: f32, y: f32) -> [u8; 3] {
    let side = 64.0;
    let per_meter = MAP_PICTURE_PIXELS as f32 / side;
    let (across, down) = (
        ((x - column as f32 * side) * per_meter) as usize,
        (y * per_meter) as usize,
    );
    let at = (down * MAP_PICTURE_PIXELS as usize + across) * 4;
    [pixels[at], pixels[at + 1], pixels[at + 2]]
}

fn greenness([r, g, b]: [u8; 3]) -> i32 {
    2 * i32::from(g) - i32::from(r) - i32::from(b)
}
