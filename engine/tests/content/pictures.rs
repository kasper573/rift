use engine::core::content::ErrorPlace;
use engine::systems::item::{CARD_SIZE, ICON_SIZE};
use serde_json::json;
use testkit::{path, png};

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
