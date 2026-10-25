use serde_json::{Value, json};

/// A dialogue node in which the narrator says `text` and the player picks one of `choices`.
pub fn dialogue(text: &str, choices: Value) -> Value {
    json!({
        "type": "dialogue",
        "lines": [{ "by": { "type": "narrator" }, "text": [text] }],
        "choices": choices,
    })
}

/// A dialogue node in which the narrator says `text`, going on to `next`.
pub fn said(text: &str, next: &str) -> Value {
    json!({
        "type": "dialogue",
        "lines": [{ "by": { "type": "narrator" }, "text": [text] }],
        "next": next,
    })
}

/// An npc script row: hostile from the start, it fights the nearest character within `within`
/// metres, and looks again at least once a second while there is none.
pub fn hunting(within: f32) -> Value {
    json!({
        "runs_on": "npc",
        "locals": { "target": "actor" },
        "nodes": {
            "spawned": { "type": "on", "event": "started", "next": "hostile" },
            "hostile": { "type": "set_attitude", "attitude": "hostile", "next": "look" },
            "look": {
                "type": "set",
                "values": [{
                    "local": "target",
                    "to": { "type": "nearest", "among": "characters", "within": within },
                }],
                "next": "found",
            },
            "found": {
                "type": "if",
                "test": { "type": "present", "actor": { "type": "local", "local": "target" } },
                "yes": "fight",
                "no": "tick",
            },
            "fight": {
                "type": "attack",
                "target": { "type": "local", "local": "target" },
                "within": within,
                "priority": 5,
                "defeated": "look",
                "lost": "look",
                "interrupted": "look",
            },
            "tick": { "type": "wait", "seconds": 0, "at_most": 1.0, "next": "look" },
        },
    })
}
