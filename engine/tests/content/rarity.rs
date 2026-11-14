use engine::core::content::ErrorPlace;
use engine::systems::item::rarity::RARITY_ICON_SIZE;
use serde_json::json;
use testkit::png;

use crate::support::game;

#[test]
fn a_rarity_holds_a_rank_of_its_own_and_an_icon_the_size_it_is_drawn() {
    let (width, height) = RARITY_ICON_SIZE;
    let cases = [
        ("/rank", "rank twice", 1, json!({})),
        (
            "/icon",
            "wrong size",
            2,
            json!({ "icon": "rarities/large.png" }),
        ),
    ];
    for (field, rule, rank, fields) in cases {
        let report = game()
            .asset("rarities/large.png", png(width * 2, height * 2))
            .resource("ore", 5)
            .rarity("fine", rank, fields)
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
        .asset("rarities/gem.png", png(width, height))
        .resource("ore", 5)
        .rarity("fine", 2, json!({ "icon": "rarities/gem.png" }))
        .load();
}
