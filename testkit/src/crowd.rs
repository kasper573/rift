//! A game to bench a server on: one wide area crowded with monsters that hunt whoever comes near
//! and townsfolk who roam, and the test characters a bench's players join as, who fight with what
//! they hold and carry gold and goods to the market. Every file is a placeholder, as a
//! server's cost is in its counts and its simulation.

use serde_json::json;

use crate::script::hunting;
use crate::{Game, Map, path};

/// The area every player of the crowd starts in.
const CROWD_AREA: &str = "crowd";

const SIDE: usize = 96;
const RAIDERS: u32 = 36;
const TOWNSFOLK: u32 = 18;
const FIGHTERS: usize = 5;
/// What the market trades: names a search finds them by.
const GOODS: [&str; 12] = [
    "Iron Ore",
    "Copper Ore",
    "Wolf Hide",
    "Bear Hide",
    "Healing Herb",
    "Silver Trout",
    "Oak Timber",
    "Linen Cloth",
    "Rough Gem",
    "Old Bone",
    "Sheep Wool",
    "Sea Salt",
];

/// The crowd as a whole game, loadable on its own.
pub fn crowd() -> Game {
    let middle = SIDE as f32 / 2.0;
    let mut game = Game::new()
        .area(
            CROWD_AREA,
            &Map::open(SIDE, SIDE),
            json!([
                { "type": "spawner", "spawns": [{ "type": "npc", "npc": path("npc", "raider") }], "count": RAIDERS },
                { "type": "spawner", "spawns": [{ "type": "npc", "npc": path("npc", "townsfolk") }], "count": TOWNSFOLK },
            ]),
        )
        .hero(CROWD_AREA, middle, middle)
        .sword()
        .npc(
            "raider",
            60.0,
            json!([{ "type": "script", "script": path("script", "raider_hunt") }]),
        )
        .edit("npc", "raider", |raider| {
            raider["basic_attack"] = json!(path("skill", "strike"))
        })
        .row("script", "raider_hunt", hunting(10.0))
        .npc(
            "townsfolk",
            30.0,
            json!([{ "type": "script", "script": path("script", "townsfolk_roam") }]),
        )
        .script(
            "townsfolk_roam",
            "npc",
            json!({
                "spawned": { "type": "on", "event": "started", "next": "roam" },
                "roam": { "type": "wander", "priority": 1, "next": "rest" },
                "rest": { "type": "wait", "seconds": 1, "at_most": 3.0, "next": "roam" },
            }),
        );
    game = game
        .item(
            "gold",
            json!({ "name": "Gold", "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 100000 }] }),
        )
        .edit_settings(|settings| {
            settings["messages"] = json!({
                "inbox": 50, "attachments": 6, "title_length": 60, "text_length": 1000, "a_minute": 6
            });
            settings["market"] = json!({
                "default_currency": path("item", "gold"),
                "deposit_currency": path("item", "gold"),
                "max_offer_items": 3,
                "max_price_items": 3,
                "durations": [43200.0, 86400.0, 172800.0],
                "deposit": 5,
                "max_listings": 20,
            });
        });
    for good in GOODS {
        game = game.item(
            &file_name(good),
            json!({ "name": good, "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 50 }] }),
        );
    }
    for fighter in 0..FIGHTERS {
        let carried: Vec<_> = (0..3)
            .map(|nth| {
                let good = GOODS[(fighter * 3 + nth) % GOODS.len()];
                json!({ "item": path("item", &file_name(good)), "count": 50 })
            })
            .chain([json!({ "item": path("item", "gold"), "count": 5000 })])
            .collect();
        game = game.row(
            "test_character",
            &format!("fighter_{fighter}"),
            json!({
                "name": format!("Fighter {fighter}"),
                "class": path("class", "hero"),
                "appearance": { "model": path("model", "hero") },
                "equipped": [path("item", "sword")],
                "basic_attack": path("skill", "strike"),
                "hotbar": [{ "slot": 0, "entry": { "skill": path("skill", "strike") } }],
                "bag": carried,
                "start": "initial_spawn",
            }),
        );
    }
    game.engine_rows().whole()
}

fn file_name(good: &str) -> String {
    good.to_lowercase().replace(' ', "_")
}
