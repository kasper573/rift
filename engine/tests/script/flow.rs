use std::sync::Arc;

use bevy_ecs::prelude::*;
use engine::core::script::compile::compile_owner;
use engine::core::tiling::Tiles;
use engine::systems::dialogue::Conversation;
use engine::systems::item::ItemId;
use engine::systems::movement::position;
use engine::systems::notification::NotificationId;
use engine::systems::npc::body::{self, Act, Priority};
use engine::systems::npc::{self, NpcId, Pack};
use engine::systems::script::{
    EventKind, Owner, OwnerKind, Procedure, RiftInstances, ScriptModule,
};
use engine::systems::stat::{self, StatKind};
use serde_json::{Value, json};

use crate::json::{inline, inline_with};
use testkit::{
    Sim, content, conversation, count, errors, give, health, labels, later, leave, pick, place,
    remembered, row, spawn_area, spoken,
};

#[test]
fn a_wait_pauses_the_flow_and_an_effect_takes_the_output_it_earns() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    let gold: ItemId = row("Gold");
    let script = inline(json!({
        "interacted": interacted("wait"),
        "wait": { "type": "wait", "seconds": 1, "at_most": 5.0, "next": "hello" },
        "hello": say("Pay up.", vec![("Pay.", Some("pay"))]),
        "pay": {
            "type": "items",
            "take": [{ "item": "gold", "count": 5 }],
            "done": "paid",
            "refused": "broke"
        },
        "paid": say("Thanks.", vec![("Bye.", None)]),
        "broke": say("No coin?", vec![("Bye.", None)]),
    }));
    let (_, owner) = scripted_npc(&mut sim, player, &[&script]);

    raise(&mut sim, owner, EventKind::Interacted, player);
    assert!(!sim.run_until(0.5, |world| world.get::<Conversation>(player).is_some()));
    assert!(sim.run_until(1.0, |world| world.get::<Conversation>(player).is_some()));
    assert_eq!(
        conversation(&mut sim, player).map(|now| now.node),
        Some("hello".to_owned())
    );

    let broke = pick(&mut sim, 1, player, "Pay.").expect("talking");
    assert_eq!(broke.node, "broke");
    leave(&mut sim, 1, player);

    give(&mut sim, player, gold, 5);
    raise(&mut sim, owner, EventKind::Interacted, player);
    assert!(sim.run_until(2.0, |world| world.get::<Conversation>(player).is_some()));
    let paid = pick(&mut sim, 1, player, "Pay.").expect("talking");
    assert_eq!(paid.node, "paid");
    assert_eq!(count(&mut sim, player, gold), 0);
}

#[test]
fn flows_run_in_the_order_their_events_were_raised() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    let gulls: NotificationId = row("Gulls");
    let sails: NotificationId = row("GullSails");
    let script = inline(json!({
        "interacted": interacted("first"),
        "first": { "type": "notify", "notification": "gulls" },
        "came_near": {
            "type": "on", "event": "came_near", "within": 3.0, "by": "characters", "next": "second"
        },
        "second": { "type": "notify", "notification": "gull_sails" },
    }));
    let (_, owner) = scripted_npc(&mut sim, player, &[&script]);
    sim.forget_notified();

    raise(&mut sim, owner, EventKind::Interacted, player);
    raise(&mut sim, owner, EventKind::CameNear, player);
    sim.tick();
    assert_eq!(notified(&sim, 1, &[gulls, sails]), vec![gulls, sails]);

    sim.forget_notified();
    later(&mut sim, 61.0);
    raise(&mut sim, owner, EventKind::CameNear, player);
    raise(&mut sim, owner, EventKind::Interacted, player);
    sim.tick();
    assert_eq!(notified(&sim, 1, &[gulls, sails]), vec![sails, gulls]);
}

#[test]
fn a_lower_priority_act_waits_while_an_equal_one_interrupts() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    let script = inline(json!({
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
        "stopped": remember("bribed_ilsa", None),
        "lost": remember("sided_with_orcs", None),
    }));
    let (npc, owner) = scripted_npc(&mut sim, player, &[&script]);
    let full = health(&mut sim, player);

    raise(&mut sim, owner, EventKind::Interacted, player);
    assert!(sim.run_until(5.0, |world| {
        stat::effective(world, player, StatKind::Health) < full
    }));
    assert_eq!(body::target(sim.world(), npc), Some(player));

    body::order(
        sim.world(),
        npc,
        Act::Wander { leash: None },
        Priority(1),
        None,
    );
    let hurt = health(&mut sim, player);
    assert!(sim.run_until(5.0, |world| {
        stat::effective(world, player, StatKind::Health) < hurt
    }));
    assert_eq!(body::target(sim.world(), npc), Some(player));
    assert!(!remembered(&mut sim, player, row("BribedIlsa")));

    body::order(sim.world(), npc, Act::Stop, Priority(5), None);
    sim.tick();
    assert!(remembered(&mut sim, player, row("BribedIlsa")));
    assert_eq!(body::target(sim.world(), npc), None);
    assert!(!remembered(&mut sim, player, row("SidedWithOrcs")));
}

#[test]
fn an_attack_loses_a_target_that_leaves_its_range() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    let script = inline(json!({
        "interacted": interacted("hostile"),
        "hostile": { "type": "set_attitude", "attitude": "hostile", "next": "attack" },
        "attack": {
            "type": "attack",
            "target": { "type": "subject" },
            "within": 10.0,
            "priority": 5,
            "lost": "lost"
        },
        "lost": remember("sided_with_orcs", None),
    }));
    let (npc, owner) = scripted_npc(&mut sim, player, &[&script]);

    raise(&mut sim, owner, EventKind::Interacted, player);
    assert!(sim.run_until(5.0, |world| body::target(world, npc) == Some(player)));

    let here = position(sim.world(), player).expect("position");
    let far = sim.walkable_near(here, Tiles(14.0), Tiles(30.0));
    place(&mut sim, player, far);
    assert!(sim.run_until(5.0, |world| body::target(world, npc).is_none()));
    sim.tick();
    assert!(remembered(&mut sim, player, row("SidedWithOrcs")));
}

#[test]
fn a_capability_aimed_at_nobody_takes_refused_or_tells_the_subject_why() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    let gold: ItemId = row("Gold");
    let nobody = json!({ "type": "local", "local": "nobody" });
    let script = inline_with(
        json!({ "nobody": "actor" }),
        json!({
            "interacted": interacted("give"),
            "give": {
                "type": "items",
                "who": nobody,
                "give": [{ "item": "gold" }],
                "done": "gave",
                "refused": "refused"
            },
            "gave": remember("sided_with_orcs", None),
            "refused": remember("bribed_ilsa", Some("heal")),
            "heal": { "type": "heal", "who": nobody, "by": "full", "next": "despawn" },
            "despawn": { "type": "despawn", "who": nobody, "done": "gone" },
            "gone": remember("tide_chest_looted", None),
        }),
    );
    let (_, owner) = scripted_npc(&mut sim, player, &[&script]);
    stat::apply_damage(sim.world(), player, 1.0);

    raise(&mut sim, owner, EventKind::Interacted, player);
    sim.tick();

    assert!(remembered(&mut sim, player, row("BribedIlsa")));
    assert!(!remembered(&mut sim, player, row("SidedWithOrcs")));
    assert!(!remembered(&mut sim, player, row("TideChestLooted")));
    assert_eq!(count(&mut sim, player, gold), 0);
    assert_eq!(errors(&sim, 1).len(), 1);
    assert!(
        errors(&sim, 1)[0].contains("Nobody"),
        "{:?}",
        errors(&sim, 1)
    );
}

#[test]
fn an_npc_can_be_the_subject_of_a_flow() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    let script = inline(json!({
        "came_near": { "type": "on", "event": "came_near", "within": 3.0, "by": "npcs", "next": "heal" },
        "heal": { "type": "heal", "by": "full" },
    }));
    let (_, healer) = scripted_npc(&mut sim, player, &[&script]);
    let here = position(sim.world(), player).expect("position");
    let spot = sim.walkable_near(here, Tiles(1.0), Tiles(2.0));
    let patient = npc::spawn(
        sim.world(),
        row::<NpcId>("Orc"),
        spot,
        spawn_area(),
        Pack(u32::MAX),
    );
    sim.tick();
    let full = health(&mut sim, patient);
    stat::apply_damage(sim.world(), patient, 3.0);
    assert!(health(&mut sim, patient) < full);

    raise(&mut sim, healer, EventKind::CameNear, patient);
    sim.tick();

    assert_eq!(health(&mut sim, patient), full);
}

#[test]
fn a_choice_label_fills_its_values_like_a_line_does() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    let script = inline(json!({
        "talk": interacted("hello"),
        "hello": {
            "type": "dialogue",
            "lines": [{ "by": { "type": "narrator" }, "text": ["Pay ", { "value": "Gold" }, " or go."] }],
            "choices": [{ "label": ["Pay ", { "value": "Gold" }] }]
        },
    }));
    let (_, healer) = scripted_npc(&mut sim, player, &[&script]);

    raise(&mut sim, healer, EventKind::Interacted, player);
    sim.tick();

    let conversation = conversation(&mut sim, player).expect("in conversation");
    assert_eq!(labels(&conversation), vec!["Pay Gold".to_owned()]);
}

fn say(text: &str, choices: Vec<(&str, Option<&str>)>) -> Value {
    let choices: Vec<Value> = choices
        .into_iter()
        .map(|(label, to)| match to {
            Some(to) => json!({ "label": [label], "to": to }),
            None => json!({ "label": [label] }),
        })
        .collect();
    json!({
        "type": "dialogue",
        "lines": [{ "by": { "type": "narrator" }, "text": [text] }],
        "choices": choices,
    })
}

fn interacted(next: &str) -> Value {
    json!({ "type": "on", "event": "interacted", "verb": "talk", "reach": 2.0, "next": next })
}

fn remember(key: &str, next: Option<&str>) -> Value {
    match next {
        Some(next) => json!({ "type": "remember", "memory": key, "next": next }),
        None => json!({ "type": "remember", "memory": key }),
    }
}

fn compiled(modules: &[&ScriptModule]) -> Vec<Arc<Procedure>> {
    compile_owner(content(), OwnerKind::Npc, "test/npc", modules).expect("compiles")
}

fn scripted_npc(sim: &mut Sim, beside: Entity, modules: &[&ScriptModule]) -> (Entity, Owner) {
    let at = position(sim.world(), beside).expect("position");
    let spot = sim.walkable_near(at, Tiles(1.0), Tiles(1.5));
    let npc = npc::spawn(
        sim.world(),
        row::<NpcId>("Orc"),
        spot,
        spawn_area(),
        Pack(u32::MAX),
    );
    let owner = Owner::Entity(OwnerKind::Npc, npc);
    let procedures = compiled(modules);
    let mut instances = sim.world().resource_mut::<RiftInstances>();
    instances.remove_owner(owner);
    instances.spawn_owner(owner, procedures);
    instances.raise(owner, EventKind::Started, None, Vec::new(), |_| true);
    sim.tick();
    (npc, owner)
}

fn raise(sim: &mut Sim, owner: Owner, kind: EventKind, subject: Entity) {
    sim.world().resource_mut::<RiftInstances>().raise(
        owner,
        kind,
        Some(subject),
        Vec::new(),
        |_| true,
    );
}

fn notified(sim: &Sim, client: u32, ids: &[NotificationId]) -> Vec<NotificationId> {
    sim.notified(client)
        .into_iter()
        .filter_map(|sent| {
            ids.iter()
                .copied()
                .find(|id| sent.notification.text == spoken(*id).text)
        })
        .collect()
}

#[test]
fn a_dialogue_behind_a_random_pick_is_found_before_it_is_heard() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    let script = inline(json!({
        "interacted": interacted("roll"),
        "roll": { "type": "random", "weights": [1, 1], "outputs": ["hello", "bye"] },
        "hello": say("Hi.", vec![("Bye.", None)]),
        "bye": { "type": "wait", "seconds": 1, "at_most": 5.0 },
    }));
    let (_, owner) = scripted_npc(&mut sim, player, &[&script]);

    let world: &World = sim.world();
    let found = world.resource::<RiftInstances>().next_dialogue(
        world,
        owner,
        EventKind::Interacted,
        player,
    );

    assert_eq!(found, vec![("test/npc".to_owned(), "hello".to_owned())]);
}
