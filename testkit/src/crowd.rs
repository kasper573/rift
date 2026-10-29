//! A game to bench a server on: one wide area crowded with monsters that hunt whoever comes near
//! and townsfolk who roam, and the test characters a bench's players join as, who fight with what
//! they hold. Every file is a placeholder, as a server's cost is in its counts and its simulation.

use serde_json::json;

use crate::script::hunting;
use crate::{Game, Map, path};

/// The area every player of the crowd starts in.
pub const CROWD_AREA: &str = "crowd";

const SIDE: usize = 96;
const RAIDERS: u32 = 36;
const TOWNSFOLK: u32 = 18;
const FIGHTERS: usize = 5;

/// The crowd as a whole game, loadable on its own.
pub fn crowd() -> Game {
    let middle = SIDE as f32 / 2.0;
    let mut game = Game::new()
        .area(
            CROWD_AREA,
            &Map::open(SIDE, SIDE),
            json!([
                { "type": "population", "npc": path("npc", "raider"), "count": RAIDERS },
                { "type": "population", "npc": path("npc", "townsfolk"), "count": TOWNSFOLK },
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
    for fighter in 0..FIGHTERS {
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
                "start": "initial_spawn",
            }),
        );
    }
    game.engine_rows().whole()
}
