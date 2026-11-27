//! A game to bench a server on: one wide area crowded with monsters that hunt whoever comes near
//! and townsfolk who roam and stop to talk, and the test characters a bench's players join as,
//! who fight with what they hold, drink potions, and carry gold and goods to the market. Every
//! file is a placeholder, as a server's cost is in its counts and its simulation.

use serde_json::json;

use crate::script::{dialogue, hunting, said};
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
                { "type": "spawner", "populations": [{ "spawns": [{ "type": "npc", "npc": path("npc", "raider") }], "max_spawned_at_once": RAIDERS }] },
                { "type": "spawner", "populations": [{ "spawns": [{ "type": "npc", "npc": path("npc", "townsfolk") }], "max_spawned_at_once": TOWNSFOLK }] },
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
            json!([
                { "type": "script", "script": path("script", "townsfolk_roam") },
                { "type": "script", "script": path("script", "townsfolk_talk") },
            ]),
        )
        .script(
            "townsfolk_talk",
            "npc",
            json!({
                "talked": { "type": "on", "event": "interacted", "verb": "talk", "reach": 2.0, "next": "greet" },
                "greet": dialogue(
                    "Fine weather for it.",
                    json!([{ "label": ["What news?"], "to": "news" }, { "label": ["Goodbye."] }]),
                ),
                "news": said("Raiders on the north road again.", "parting"),
                "parting": dialogue("Mind how you go.", json!([{ "label": ["Goodbye."] }])),
            }),
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
            "potion",
            json!({
                "name": "Potion",
                "kind": { "type": "usable" },
                "modules": [{ "type": "stack", "max": 50 }, { "type": "script", "script": path("script", "potion_used") }],
            }),
        )
        .script(
            "potion_used",
            "item",
            json!({
                "used": { "type": "on", "event": "used", "next": "heal" },
                "heal": { "type": "heal", "by": 25.0 },
            }),
        )
        .item(
            "gold",
            json!({ "name": "Gold", "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 100000 }] }),
        )
        .edit_settings(|settings| {
            settings["mail"] = json!({
                "inbox": 50, "attachments": 6, "title_length": 60, "notice_title_length": 25, "text_length": 1000, "a_minute": 6
            });
            settings["market"] = json!({
                "default_currency": path("item", "gold"),
                "deposit_currency": path("item", "gold"),
                "max_goods_items": 3,
                "max_price_items": 3,
                "durations": [43200.0, 86400.0, 172800.0],
                "deposit": 5,
                "max_listings": 20,
                "page_rows": 50,
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
                json!({ "slot": nth, "item": path("item", &file_name(good)), "count": 50 })
            })
            .chain([
                json!({ "slot": 3, "item": path("item", "gold"), "count": 5000 }),
                json!({ "slot": 4, "item": path("item", "potion"), "count": 50 }),
            ])
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
