use engine::core::content::{ErrorPlace, LoadReport};
use engine::core::render::world::terrain::GROUND_PICTURE;
use engine::systems::item::{CARD_SIZE, ICON_SIZE};
use serde_json::json;
use testkit::{Map, path, png};

use crate::support::game;

fn resource() -> serde_json::Value {
    json!({ "type": "resource" })
}

#[test]
fn an_item_names_its_three_pictures_each_as_what_draws_it_takes_them() {
    let cases = [
        (
            "/look",
            "missing file",
            json!({ "look": "icons/cap.png", "kind": resource() }),
        ),
        (
            "/icon",
            "missing file",
            json!({ "icon": "icons/gone.png", "kind": resource() }),
        ),
        (
            "/card",
            "wrong size",
            json!({ "card": "icons/cap.png", "kind": resource() }),
        ),
    ];
    for (field, rule, fields) in cases {
        let report = game()
            .asset("icons/cap.png", png(ICON_SIZE.0, ICON_SIZE.1))
            .item("cap", json!({ "kind": resource() }))
            .item("hat", fields)
            .try_load()
            .err()
            .expect("refused");
        let at_the_item: Vec<(&str, &str)> = report
            .errors
            .iter()
            .filter(|error| error.file == path("item", "hat"))
            .map(|error| {
                let pointer = match &error.at {
                    ErrorPlace::Json { pointer, .. } => pointer.as_str(),
                    _ => "",
                };
                (error.problem.rule, pointer)
            })
            .collect();
        assert!(
            at_the_item
                .iter()
                .any(|(found, place)| *found == rule && *place == field),
            "{field} {rule}: {at_the_item:?}\n{report}"
        );
    }

    let painted = game()
        .asset("art/cap_card.png", png(CARD_SIZE.0, CARD_SIZE.1))
        .item(
            "cap",
            json!({ "card": "art/cap_card.png", "kind": resource() }),
        );
    assert!(painted.try_load().is_ok());
}

#[test]
fn a_grounds_picture_is_the_one_size_the_ground_draws_and_repeats_within_reach() {
    let pictured = |side: u32, tile: f32| {
        game()
            .asset("ground/earth.png", png(side, side))
            .area("meadow", &Map::open(8, 8), json!([]))
            .edit("terrain", testkit::TURF, |row| {
                row["picture"] = json!({ "image": "ground/earth.png", "tile": tile });
            })
            .try_load()
    };
    assert!(pictured(GROUND_PICTURE, 4.0).is_ok());
    for (side, tile, field, rule) in [
        (GROUND_PICTURE / 2, 4.0, "/picture/image", "wrong size"),
        (GROUND_PICTURE, 0.1, "/picture/tile", "out of range"),
        (GROUND_PICTURE, 100.0, "/picture/tile", "out of range"),
    ] {
        let report = pictured(side, tile).err().expect("refused");
        assert!(
            at_terrain(&report).contains(&(rule, field)),
            "{side} px every {tile} m: {report}"
        );
    }
}

fn at_terrain(report: &LoadReport) -> Vec<(&str, &str)> {
    report
        .errors
        .iter()
        .filter(|error| error.file == path("terrain", testkit::TURF))
        .map(|error| {
            let pointer = match &error.at {
                ErrorPlace::Json { pointer, .. } => pointer.as_str(),
                _ => "",
            };
            (error.problem.rule, pointer)
        })
        .collect()
}
