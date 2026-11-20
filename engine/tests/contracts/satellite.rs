use engine::core::content::Content;
use engine::core::units::MeterSize;
use engine::systems::area::{self, AreaDef};
use engine::systems::area_map::satellite::{SatelliteDensity, SatelliteKey, SatelliteTile};
use serde_json::json;
use testkit::{Declared, Game, Map, path};

#[test]
fn a_satellite_picture_is_drawn_again_only_where_what_it_shows_changed() {
    let density = SatelliteDensity(8.0);
    let field = |rock_at: f32| {
        Game::new()
            .prop("rock", json!({ "type": "none" }))
            .area(
                "field",
                &Map::open(192, 128),
                json!([{ "type": "prop", "prop": path("prop", "rock"), "at": { "x": rock_at, "y": 10.0 } }]),
            )
            .hero("field", 1.0, 1.0)
            .load()
    };
    let keys = |content: &Content| -> Vec<(SatelliteTile, SatelliteKey)> {
        let id = content.declared::<AreaDef>("field").expect("the field");
        let area = area::loaded(content, id).expect("the field is loaded");
        SatelliteTile::within(0, area.size.bounds(), area.size.bounds(), density)
            .into_iter()
            .map(|tile| (tile, SatelliteKey::of(content, area, tile, density)))
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
        "moving the rock changes the keys of the squares it left and came to, and no other"
    );
}
