use bevy_ecs::prelude::*;
use engine::core::content::LoadReport;
use engine::systems::dialogue::{self, ConversationRequest};
use engine::systems::interact::{self, InteractRequest};
use engine::systems::item::{ItemDef, ItemStack};
use engine::systems::prompt::{self, PromptRequest, PromptStep, PromptView};
use engine::systems::{movement, region, script};
use serde_json::{Value, json};
use testkit::{Game, Map, Sim, dialogue as asks, lines, path, said};

#[test]
fn what_is_placed_reaches_the_script_as_offered_and_nothing_leaves_the_bag_until_it_is_taken() {
    let mut sim = gate();
    let player = sim.join(1);
    sim.give(player, "key", 1);
    sim.give(player, "coin", 3);

    let asked = approach(&mut sim, 1);
    assert_eq!(asked.title, "The lock");
    assert_eq!(asked.slots, 2);
    assert!(asked.text.is_some(), "the script's own words");

    submit(&mut sim, 1, asked.step, &[("coin", 2)]);
    let again = shown(&sim, player).expect("the lock asks again");
    assert_ne!(again.step, asked.step);
    assert_eq!(heard(&sim, 1), ["Nothing happens."]);
    assert_eq!(sim.count(player, "coin"), 3, "an offer not wanted is kept");

    submit(&mut sim, 1, asked.step, &[("key", 1)]);
    assert_eq!(
        shown(&sim, player),
        Some(again.clone()),
        "an answer to a turn that is over"
    );

    submit(&mut sim, 1, again.step, &[("key", 1), ("coin", 1)]);
    assert_eq!(shown(&sim, player), None, "submitting closes the prompt");
    assert_eq!(heard(&sim, 1), ["The key fits."]);
    assert_eq!(
        (sim.count(player, "key"), sim.count(player, "coin")),
        (1, 3),
        "nothing leaves the bag before the script takes it"
    );

    sim.pick(1, "Turn it.");
    assert_eq!(
        (sim.count(player, "key"), sim.count(player, "coin")),
        (0, 2),
        "all that was offered is taken"
    );
    assert_eq!(sim.count(player, "pass"), 1);
    assert_eq!(sim.errors(1), Vec::<String>::new());
}

#[test]
fn an_offer_the_bag_does_not_cover_is_never_handed_on() {
    let mut sim = gate();
    let player = sim.join(1);
    sim.give(player, "key", 1);
    sim.give(player, "coin", 3);
    sim.give(player, "pebble", 1);
    let asked = approach(&mut sim, 1);

    submit(&mut sim, 1, asked.step, &[("key", 2)]);
    assert_eq!(sim.errors(1), ["Needs 1 more key"]);
    let asked = shown(&sim, player).expect("still asking");

    sim.forget_notified();
    submit(
        &mut sim,
        1,
        asked.step,
        &[("key", 1), ("coin", 1), ("pebble", 1)],
    );
    assert_eq!(
        shown(&sim, player),
        Some(asked.clone()),
        "more stacks than slots"
    );

    submit(&mut sim, 1, asked.step, &[("key", 1), ("coin", 3)]);
    assert_eq!(heard(&sim, 1), ["The key fits."]);
    sim.take(player, "coin", 1);
    sim.pick(1, "Turn it.");
    assert_eq!(
        (sim.count(player, "key"), sim.count(player, "coin")),
        (1, 2),
        "an offer the bag stopped covering is taken in no part"
    );
    assert_eq!(sim.count(player, "pass"), 0);
    let again = shown(&sim, player).expect("the lock asks again");

    sim.leave(1);
    assert_eq!(shown(&sim, player), None);
    assert_eq!(sim.count(player, "slip"), 1, "leaving goes down closed");
    submit(&mut sim, 1, again.step, &[("key", 1)]);
    assert_eq!(
        sim.count(player, "pass"),
        0,
        "a prompt left takes no answer"
    );
}

#[test]
fn the_loader_refuses_a_prompt_without_slots_and_an_offer_used_where_no_prompt_asks_for_one() {
    let slotless = refused(|nodes| nodes["ask"]["slots"] = json!(0));
    assert_eq!(rules(&slotless), ["no slots"]);

    for (unasked, leads) in [("fits", "sure"), ("take", "open")] {
        let report = refused(|nodes| {
            let node = nodes[unasked].take();
            *nodes = json!({
                "interacted": { "type": "on", "event": "interacted", "verb": "talk", "reach": 2.0, "next": unasked },
                "wrong": said("Nothing happens.", "end"),
                "end": { "type": "items", "give": [{ "item": path("item", "pass") }] },
            });
            nodes[leads] = said("The key fits.", "end");
            nodes[unasked] = node;
        });
        assert_eq!(rules(&report), ["offered without a prompt"], "{unasked}");
    }
}

/// A gate whose keeper's lock asks for items in two slots: an offer holding a key fits, and is
/// taken for a pass once the player says to turn it; any other goes back to the lock; leaving it
/// hands over a slip.
fn gate() -> Sim {
    let mut sim = Sim::new(game(|_| {}).load());
    sim.serve("gate")
        .accept::<InteractRequest>()
        .accept::<ConversationRequest>()
        .accept::<PromptRequest>()
        .add((
            interact::interact_request,
            dialogue::requests,
            prompt::requests,
        ))
        .plan((
            script::RunFlows::default(),
            movement::AdvanceWalks::default(),
        ))
        .add((
            interact::interactions,
            dialogue::hold,
            dialogue::refresh,
            prompt::hold,
        ));
    script::spawn_stateless(&mut sim.world);
    region::hold_whole(&mut sim.world);
    sim
}

fn game(change: impl FnOnce(&mut Value)) -> Game {
    let mut nodes = json!({
        "interacted": { "type": "on", "event": "interacted", "verb": "talk", "reach": 2.0, "next": "hello" },
        "hello": said("The lock is empty.", "ask"),
        "ask": {
            "type": "item_prompt",
            "title": "The lock",
            "text": ["What fits?"],
            "slots": 2,
            "submitted": "fits",
            "closed": "left",
        },
        "fits": {
            "type": "if",
            "test": { "type": "at_least", "a": { "type": "offered", "item": path("item", "key") }, "b": 1 },
            "yes": "sure",
            "no": "wrong",
        },
        "sure": asks("The key fits.", json!([{ "label": ["Turn it."], "to": "take" }])),
        "take": { "type": "take_offer", "done": "open", "refused": "wrong" },
        "open": { "type": "items", "give": [{ "item": path("item", "pass") }] },
        "wrong": said("Nothing happens.", "ask"),
        "left": { "type": "items", "give": [{ "item": path("item", "slip") }] },
    });
    change(&mut nodes);
    Game::new()
        .area(
            "gate",
            &Map::open(12, 12),
            json!([{ "type": "resident", "npc": path("npc", "keeper"), "at": { "x": 4.0, "y": 4.0 } }]),
        )
        .hero("gate", 4.0, 5.0)
        .resource("key", 10)
        .resource("coin", 10)
        .resource("pebble", 10)
        .resource("pass", 10)
        .resource("slip", 10)
        .npc(
            "keeper",
            40.0,
            json!([{ "type": "script", "script": path("script", "lock") }]),
        )
        .script("lock", "npc", nodes)
        .sfx("ui_refuse")
        .sfx("ui_close")
}

fn refused(change: impl FnOnce(&mut Value)) -> LoadReport {
    game(change).try_load().err().expect("refused")
}

fn rules(report: &LoadReport) -> Vec<&str> {
    report
        .errors
        .iter()
        .map(|error| error.problem.rule)
        .collect()
}

/// Talks to the keeper until the lock's prompt stands open.
fn approach(sim: &mut Sim, client: u32) -> PromptView {
    let player = sim.player(client);
    sim.talk(client, "keeper");
    assert!(
        sim.run_until(20.0, |world| world.get::<PromptView>(player).is_some()),
        "the lock never asked"
    );
    shown(sim, player).expect("the lock asks")
}

fn submit(sim: &mut Sim, client: u32, step: PromptStep, offer: &[(&str, u32)]) {
    let conn = sim.conn(client);
    let offer = offer
        .iter()
        .map(|(item, count)| ItemStack::new(sim.id::<ItemDef>(item), *count))
        .collect();
    sim.send(conn, PromptRequest::Submit { step, offer });
    sim.run(0.5);
}

fn shown(sim: &Sim, player: Entity) -> Option<PromptView> {
    sim.world.get::<PromptView>(player).cloned()
}

fn heard(sim: &Sim, client: u32) -> Vec<String> {
    lines(&sim.conversation(client).expect("talking"))
}
