use engine::systems::actor::stance::HandIndex;
use engine::systems::actor::{Appearance, Hitbox, ModelDef, held::Held};
use engine::systems::item::ItemDef;
use engine::systems::{npc, script};
use serde_json::{Value, json};
use testkit::{Game, Map, Sim, path, rig_clips};

#[test]
fn an_npcs_script_dresses_it_anew_and_fills_its_hands() {
    struct Case {
        case: &'static str,
        nodes: Value,
        model: &'static str,
        height: f32,
        hands: &'static [(HandIndex, &'static str)],
    }
    let cases = [
        Case {
            case: "a new body, then a sword in hand",
            nodes: json!({
                "started": { "type": "on", "event": "started", "next": "dress" },
                "dress": { "type": "set_appearance", "appearance": { "model": path("model", "giant") }, "next": "arm" },
                "arm": { "type": "set_held", "items": [path("item", "sword")] },
            }),
            model: "giant",
            height: 4.0,
            hands: &[(HandIndex::MAIN, "sword")],
        },
        Case {
            case: "a body without hands shows nothing it held",
            nodes: json!({
                "started": { "type": "on", "event": "started", "next": "arm" },
                "arm": { "type": "set_held", "items": [path("item", "sword")], "next": "dress" },
                "dress": { "type": "set_appearance", "appearance": { "model": path("model", "slime") } },
            }),
            model: "slime",
            height: 1.0,
            hands: &[],
        },
    ];
    for case in cases {
        let mut sim = Sim::new(game(case.nodes).load());
        sim.serve("room").add(script::run_tick);
        script::spawn_stateless(&mut sim.world);
        npc::spawn_all(&mut sim.world);
        let guard = sim.npc("guard");
        sim.tick();

        let model = sim.id::<ModelDef>(case.model);
        assert_eq!(
            sim.world.get::<Appearance>(guard),
            Some(&Appearance::of(model)),
            "{}",
            case.case
        );
        assert_eq!(
            sim.world
                .get::<Hitbox>(guard)
                .map(|hitbox| hitbox.0.height.0),
            Some(case.height),
            "{}: hit as the new model is",
            case.case
        );
        let hands = case
            .hands
            .iter()
            .map(|(hand, item)| (*hand, sim.id::<ItemDef>(item)))
            .collect();
        assert_eq!(
            sim.world.get::<Held>(guard).map(|held| held.hands.clone()),
            Some(hands),
            "{}",
            case.case
        );
    }
}

#[test]
fn a_script_dressing_an_npc_in_a_look_its_model_cannot_wear_is_refused() {
    let report = game(json!({
        "started": { "type": "on", "event": "started", "next": "dress" },
        "dress": {
            "type": "set_appearance",
            "appearance": { "model": path("model", "giant"), "colours": { "fur": [1, 2, 3] } },
        },
    }))
    .try_load()
    .err()
    .expect("refused");
    let rules: Vec<&str> = report
        .errors
        .iter()
        .filter(|error| error.file == path("script", "guard_start"))
        .map(|error| error.problem.rule)
        .collect();
    assert_eq!(rules, ["unknown tint"], "{report}");
}

/// A room where a guard runs `nodes` as it wakes; a giant's body with hands and a slime's without
/// to dress it in, and a sword to show.
fn game(nodes: Value) -> Game {
    Game::new()
        .area(
            "room",
            &Map::open(9, 9),
            json!([{ "type": "resident", "npc": "guard.npc.json", "at": { "x": 6.0, "y": 6.0 } }]),
        )
        .hero("room", 2.0, 2.0)
        .skeleton("beast")
        .model_on("giant", "beast", 4.0, json!([]))
        .skeleton_of("blob", &rig_clips(), json!({ "hands": [] }))
        .model_on("slime", "blob", 1.0, json!([]))
        .sword()
        .npc(
            "guard",
            30.0,
            json!([{ "type": "script", "script": path("script", "guard_start") }]),
        )
        .script("guard_start", "npc", nodes)
}
