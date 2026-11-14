use engine::core::content::ErrorPlace;
use serde_json::json;
use testkit::{path, png};

use crate::support::game;

#[test]
fn a_card_style_picks_looks_it_can_paint_within_what_a_card_has_room_for() {
    let gradient = json!({ "from": [177, 132, 234], "to": [111, 58, 184] });
    let lacking = "look without what it paints with";
    let cases = [
        (
            "/border",
            lacking,
            json!({ "border": { "type": "gradient", "width": 3.0 } }),
        ),
        (
            "/crest",
            lacking,
            json!({ "crest": { "type": "icon", "size": 18.0 } }),
        ),
        (
            "/label_line",
            lacking,
            json!({ "label_line": { "type": "seal" } }),
        ),
        (
            "/name_band/icon",
            lacking,
            json!({ "name_band": { "type": "plain", "icon": true } }),
        ),
        (
            "/border/gap",
            "out of range",
            json!({
                "gradient": gradient,
                "border": { "type": "double", "width": 3.0, "gap": 6.0 },
            }),
        ),
        (
            "/corners/size",
            "out of range",
            json!({ "corners": { "type": "cut", "size": 40.0 } }),
        ),
        (
            "/paper/strength",
            "out of range",
            json!({ "gradient": gradient, "paper": { "type": "tinted", "strength": 1.5 } }),
        ),
        (
            "/material/gloss",
            "out of range",
            json!({ "material": { "type": "lacquer", "gloss": 2.0 } }),
        ),
    ];
    for (field, rule, fields) in cases {
        let report = game()
            .card_style("gilded", fields)
            .try_load()
            .err()
            .expect("refused");
        let found = report.errors.iter().any(|error| {
            error.problem.rule == rule
                && matches!(&error.at, ErrorPlace::Json { pointer, .. } if pointer == field)
        });
        assert!(found, "{field} {rule}\n{report}");
    }

    game()
        .asset("cards/star.png", png(16, 16))
        .asset("cards/weave.png", png(8, 8))
        .card_style("plain", json!({}))
        .card_style(
            "gilded",
            json!({
                "gradient": gradient,
                "icon": "cards/star.png",
                "label": "Gilded",
                "border": { "type": "patterned", "image": "cards/weave.png", "width": 6.0 },
                "corners": { "type": "notched", "size": 4.0 },
                "crest": { "type": "banner" },
                "name_band": { "type": "ribbon", "overhang": 6.0, "icon": true },
                "label_line": { "type": "strip" },
                "art": { "type": "inset", "height": 150.0, "margin": 6.0 },
                "material": { "type": "foil", "pattern": "cards/weave.png", "strength": 0.6 },
                "paper": { "type": "watermarked", "strength": 0.2 },
                "dividers": { "type": "ornament" },
                "halo": { "type": "glow", "spread": 12.0 },
            }),
        )
        .resource("ore", 5)
        .rarity(
            "fine",
            2,
            json!({ "card_style": path("card_style", "gilded") }),
        )
        .load();
}

#[test]
fn a_game_turns_its_cards_no_further_than_keeps_a_close_button_under_the_pointer() {
    let turned = |tilt: f32, eye: f32| {
        game()
            .edit_settings(|settings| {
                settings["cards"]["tilt"] = json!(tilt);
                settings["cards"]["eye"] = json!(eye);
            })
            .try_load()
    };

    assert!(turned(4.0, 1.5).is_ok());
    for (tilt, eye) in [(9.0, 1.5), (4.0, 0.4)] {
        let report = turned(tilt, eye).err().expect("refused");
        let found = report.errors.iter().any(|error| {
            error.problem.rule == "tilt too steep"
                && matches!(&error.at, ErrorPlace::Json { pointer, .. } if pointer == "/cards/tilt")
        });
        assert!(found, "{tilt} degrees seen from {eye}\n{report}");
    }
}
