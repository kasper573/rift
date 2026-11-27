use bevy_ecs::prelude::*;
use engine::core::math::{Offset, Pos};
use engine::core::script::EventKind;
use engine::core::script::Instances;
use engine::core::script::Owner;
use engine::systems::dialogue::Conversation;
use engine::systems::interact::Interacted;
use engine::systems::movement;
use engine::systems::notification::Notification;
use engine::systems::npc::CameNear;
use engine::systems::npc::body::{self, Act, Priority};
use engine::systems::stat::{self, StatKind};
use serde_json::{Value, json};
use testkit::{Sim, dialogue, path};

use crate::game::{field, health, module, scripted_npc, spawn};

#[test]
fn a_wait_pauses_the_flow_and_an_effect_takes_the_output_it_earns() {
    let (mut sim, player) = field();
    let (_, owner) = scripted_npc(&mut sim, player, &[&module("flow_pay_later")]);

    raise(&mut sim, owner, EventKind::of::<Interacted>(), player);
    assert!(!sim.run_until(0.5, |world| world.get::<Conversation>(player).is_some()));
    assert!(sim.run_until(1.0, |world| world.get::<Conversation>(player).is_some()));
    assert_eq!(
        sim.conversation(1).map(|now| sim.lines(&now)),
        Some(vec!["Pay up.".to_owned()])
    );

    let broke = sim.pick(1, "Pay.").expect("talking");
    assert_eq!(sim.lines(&broke), vec!["No coin?".to_owned()]);
    sim.leave(1);

    sim.give(player, "gold", 5);
    raise(&mut sim, owner, EventKind::of::<Interacted>(), player);
    assert!(sim.run_until(2.0, |world| world.get::<Conversation>(player).is_some()));
    let paid = sim.pick(1, "Pay.").expect("talking");
    assert_eq!(sim.lines(&paid), vec!["Thanks.".to_owned()]);
    assert_eq!(sim.count(player, "gold"), 0);
}

#[test]
fn flows_run_in_the_order_their_events_were_raised() {
    let (mut sim, player) = field();
    let (gulls, sails) = (sim.spoken("gulls"), sim.spoken("sails"));
    let (_, owner) = scripted_npc(&mut sim, player, &[&module("flow_notices")]);
    sim.forget_notified();

    raise(&mut sim, owner, EventKind::of::<Interacted>(), player);
    raise(&mut sim, owner, EventKind::of::<CameNear>(), player);
    sim.tick();
    assert_eq!(told(&mut sim, &[&gulls, &sails]), vec![&gulls, &sails]);

    sim.forget_notified();
    sim.later(61.0);
    raise(&mut sim, owner, EventKind::of::<CameNear>(), player);
    raise(&mut sim, owner, EventKind::of::<Interacted>(), player);
    sim.tick();
    assert_eq!(told(&mut sim, &[&gulls, &sails]), vec![&sails, &gulls]);
}

#[test]
fn a_lower_priority_act_waits_while_an_equal_one_interrupts() {
    let (mut sim, player) = field();
    let (npc, owner) = scripted_npc(&mut sim, player, &[&module("flow_interruptible")]);
    let full = health(&sim, player);

    raise(&mut sim, owner, EventKind::of::<Interacted>(), player);
    assert!(sim.run_until(5.0, |world| {
        stat::effective(world, player, StatKind::Health) < full
    }));
    assert_eq!(body::target(&sim.world, npc), Some(player));

    body::order(
        &mut sim.world,
        npc,
        Act::Wander { leash: None },
        Priority(1),
        None,
    );
    let hurt = health(&sim, player);
    assert!(sim.run_until(5.0, |world| {
        stat::effective(world, player, StatKind::Health) < hurt
    }));
    assert_eq!(body::target(&sim.world, npc), Some(player));
    assert!(!sim.remembers(player, "bribed"));

    body::order(&mut sim.world, npc, Act::Stop, Priority(5), None);
    sim.tick();
    assert!(sim.remembers(player, "bribed"));
    assert_eq!(body::target(&sim.world, npc), None);
    assert!(!sim.remembers(player, "sided"));
}

#[test]
fn an_attack_loses_a_target_that_leaves_its_range() {
    let (mut sim, player) = field();
    let (npc, owner) = scripted_npc(&mut sim, player, &[&module("flow_leash")]);

    raise(&mut sim, owner, EventKind::of::<Interacted>(), player);
    assert!(sim.run_until(5.0, |world| body::target(world, npc) == Some(player)));

    movement::relocate(&mut sim.world, player, Pos::new(28.0, 28.0));
    assert!(sim.run_until(5.0, |world| body::target(world, npc).is_none()));
    sim.tick();
    assert!(sim.remembers(player, "sided"));
}

#[test]
fn a_capability_aimed_at_nobody_takes_refused_or_tells_the_subject_why() {
    let (mut sim, player) = field();
    let (_, owner) = scripted_npc(&mut sim, player, &[&module("flow_nobody")]);
    stat::apply_damage(&mut sim.world, player, 1.0);

    raise(&mut sim, owner, EventKind::of::<Interacted>(), player);
    sim.tick();

    assert!(sim.remembers(player, "bribed"));
    assert!(!sim.remembers(player, "sided"));
    assert!(!sim.remembers(player, "looted"));
    assert_eq!(sim.count(player, "gold"), 0);
    let errors = sim.errors(1);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("Nobody"), "{errors:?}");
}

#[test]
fn an_npc_can_be_the_subject_of_a_flow() {
    let (mut sim, player) = field();
    let (_, healer) = scripted_npc(&mut sim, player, &[&module("flow_healer")]);
    let patient = spawn(&mut sim, "fighter", player, Offset::new(0.0, 2.0), u32::MAX);
    sim.tick();
    let full = health(&sim, patient);
    stat::apply_damage(&mut sim.world, patient, 3.0);
    assert!(health(&sim, patient) < full);

    raise(&mut sim, healer, EventKind::of::<CameNear>(), patient);
    sim.tick();

    assert_eq!(health(&sim, patient), full);
}

#[test]
fn a_choice_label_fills_its_values_like_a_line_does() {
    let (mut sim, player) = field();
    let (_, healer) = scripted_npc(&mut sim, player, &[&module("flow_label")]);

    raise(&mut sim, healer, EventKind::of::<Interacted>(), player);
    sim.tick();

    let conversation = sim.conversation(1).expect("in conversation");
    assert_eq!(sim.labels(&conversation), vec!["Pay Gold".to_owned()]);
}

fn interacted(next: &str) -> Value {
    json!({ "type": "on", "event": "interacted", "verb": "talk", "reach": 2.0, "next": next })
}

fn remember(key: &str, next: Option<&str>) -> Value {
    match next {
        Some(next) => json!({ "type": "remember", "memory": path("memory", key), "next": next }),
        None => json!({ "type": "remember", "memory": path("memory", key) }),
    }
}

fn raise(sim: &mut Sim, owner: Owner, kind: EventKind, subject: Entity) {
    sim.world
        .resource_mut::<Instances>()
        .raise(owner, kind, Some(subject), Vec::new(), |_| true);
}

/// Which of `lines` the first client was told, in the order told.
fn told<'a>(sim: &mut Sim, lines: &[&'a Notification]) -> Vec<&'a Notification> {
    sim.told(1)
        .into_iter()
        .filter_map(|told| lines.iter().copied().find(|line| line.text == told.text))
        .collect()
}

pub fn scripts() -> Vec<(&'static str, Value)> {
    let npc = |nodes: Value| json!({ "runs_on": "npc", "nodes": nodes });
    vec![
        (
            "flow_pay_later",
            npc(json!({
                "interacted": interacted("wait"),
                "wait": { "type": "wait", "seconds": 1, "at_most": 5.0, "next": "hello" },
                "hello": dialogue("Pay up.", json!([{ "label": ["Pay."], "to": "pay" }])),
                "pay": {
                    "type": "items",
                    "take": [{ "item": "gold.item.json", "count": 5 }],
                    "done": "paid",
                    "refused": "broke"
                },
                "paid": dialogue("Thanks.", json!([{ "label": ["Bye."] }])),
                "broke": dialogue("No coin?", json!([{ "label": ["Bye."] }])),
            })),
        ),
        (
            "flow_notices",
            npc(json!({
                "interacted": interacted("first"),
                "first": { "type": "notify", "notification": "gulls.notification.json" },
                "came_near": {
                    "type": "on", "event": "came_near", "within": 3.0, "by": "characters", "fliers": false, "next": "second"
                },
                "second": { "type": "notify", "notification": "sails.notification.json" },
            })),
        ),
        (
            "flow_interruptible",
            npc(json!({
                "interacted": interacted("hostile"),
                "hostile": { "type": "set_attitude", "attitude": "hostile", "next": "attack" },
                "attack": {
                    "type": "attack",
                    "target": { "type": "subject" },
                    "within": 10.0,
                    "priority": 5,
                    "lost": "lost",
                    "interrupted": "stopped"
                },
                "stopped": remember("bribed", None),
                "lost": remember("sided", None),
            })),
        ),
        (
            "flow_leash",
            npc(json!({
                "interacted": interacted("hostile"),
                "hostile": { "type": "set_attitude", "attitude": "hostile", "next": "attack" },
                "attack": {
                    "type": "attack",
                    "target": { "type": "subject" },
                    "within": 10.0,
                    "priority": 5,
                    "lost": "lost"
                },
                "lost": remember("sided", None),
            })),
        ),
        (
            "flow_nobody",
            json!({
                "runs_on": "npc",
                "locals": { "nobody": "actor" },
                "nodes": {
                    "interacted": interacted("give"),
                    "give": {
                        "type": "items",
                        "who": { "type": "local", "local": "nobody" },
                        "give": [{ "item": "gold.item.json" }],
                        "done": "gave",
                        "refused": "refused"
                    },
                    "gave": remember("sided", None),
                    "refused": remember("bribed", Some("heal")),
                    "heal": { "type": "heal", "who": { "type": "local", "local": "nobody" }, "by": "full", "next": "despawn" },
                    "despawn": { "type": "despawn", "who": { "type": "local", "local": "nobody" }, "done": "gone" },
                    "gone": remember("looted", None),
                },
            }),
        ),
        (
            "flow_healer",
            npc(json!({
                "came_near": { "type": "on", "event": "came_near", "within": 3.0, "by": "npcs", "fliers": false, "next": "heal" },
                "heal": { "type": "heal", "by": "full" },
            })),
        ),
        (
            "flow_label",
            npc(json!({
                "talk": interacted("hello"),
                "hello": {
                    "type": "dialogue",
                    "lines": [{ "by": { "type": "narrator" }, "text": ["Pay ", { "value": "Gold" }, " or go."] }],
                    "choices": [{ "label": ["Pay ", { "value": "Gold" }] }]
                },
            })),
        ),
    ]
}
