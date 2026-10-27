use bevy_ecs::prelude::*;
use engine::systems::combat::{self, AttackRequest};
use engine::systems::dialogue::{self, ConversationRequest};
use engine::systems::interact::{self, InteractRequest};
use engine::systems::memory::{self, MemoryDef};
use engine::systems::npc::{self, NpcDef, summon};
use engine::systems::player::ClientId;
use engine::systems::stat::{self, StatKind};
use engine::systems::visibility;
use engine::systems::{actor, movement, rewards, script, skill};
use serde_json::{Value, json};
use testkit::{Game, Map, Sim, dialogue, hunting, lines, path};

#[test]
fn a_gamblers_dice_take_ten_gold_a_roll_and_pay_twenty_on_a_win() {
    let mut sim = harbour();
    let player = sim.join(1);
    sim.equip(player, "sword");
    sim.give(player, "gold", 400);

    let mut roll = "Roll the dice.";
    sim.talk(1, "gambler");
    let (mut won, mut lost) = (false, false);
    for _ in 0..30 {
        let before = sim.count(player, "gold");
        let said = lines(&sim.pick(1, roll).expect("at the table"));
        let after = sim.count(player, "gold");
        if said == ["Beginner's luck."] {
            assert_eq!(after, before + 10);
            won = true;
        } else if said == ["The house wins!"] {
            assert_eq!(after, before - 10);
            lost = true;
        } else {
            panic!("the dice landed on {said:?}");
        }
        roll = "Roll again.";
    }
    assert!(won && lost, "thirty rolls went only one way");
}

#[test]
fn calling_the_gambler_a_cheat_sends_guards_after_you_alone() {
    let mut sim = harbour();
    let player = sim.join(1);
    sim.equip(player, "sword");
    let bystander = sim.join(2);
    sim.give(player, "gold", 400);

    lose_at_dice(&mut sim, 1);
    assert!(sim.pick(1, "You're cheating.").is_none());

    let guards = sim.npcs("guard");
    assert_eq!(guards.len(), 2);
    for &guard in &guards {
        assert!(visibility::present(&sim.world, guard, player));
        assert!(!visibility::present(&sim.world, guard, bystander));
    }
    assert!(sim.remembers(player, "grudge"));

    let full = stat::effective(&sim.world, player, StatKind::Health);
    let calm = stat::effective(&sim.world, bystander, StatKind::Health);
    assert!(sim.run_until(15.0, |world| stat::effective(
        world,
        player,
        StatKind::Health
    ) < full));
    assert_eq!(
        stat::effective(&sim.world, bystander, StatKind::Health),
        calm
    );

    assert_eq!(
        lines(&sim.talk(1, "gambler")),
        vec!["You've got some nerve.".to_owned()]
    );
}

#[test]
fn fighting_the_gambler_leaves_him_dead_in_your_world_only() {
    let mut sim = harbour();
    let player = sim.join(1);
    sim.equip(player, "sword");
    let bystander = sim.join(2);
    let grudge = sim.id::<MemoryDef>("grudge");
    memory::set(
        &mut sim.world,
        player,
        grudge,
        engine::core::script::Value::Bool(true),
    );
    let gambler = sim.npc("gambler");

    sim.talk(1, "gambler");
    assert!(sim.pick(1, "Then let's settle this.").is_none());
    let copies = sim.npcs("gambler_hostile");
    assert_eq!(copies.len(), 1);
    let copy = copies[0];
    assert!(!visibility::present(&sim.world, gambler, player));
    assert!(visibility::present(&sim.world, gambler, bystander));
    assert!(!visibility::present(&sim.world, copy, bystander));

    sim.run(0.5);
    let conn = sim.conn(1);
    sim.send(conn, AttackRequest { target: copy });
    assert!(
        sim.run_until(60.0, |world| stat::is_dead(world, copy)),
        "the copy never fell"
    );
    sim.run(0.5);
    assert!(sim.remembers(player, "dead"));
    assert!(!sim.remembers(player, "fighting"));
    assert!(!visibility::present(&sim.world, gambler, player));
    assert!(visibility::present(&sim.world, gambler, bystander));

    sim.later(1800.0);
    assert!(visibility::present(&sim.world, gambler, player));
}

#[test]
fn summons_leave_with_their_owner() {
    let mut sim = harbour();
    let player = sim.join(1);
    sim.equip(player, "sword");
    let guard = sim.id::<NpcDef>("guard");
    summon(
        &mut sim.world,
        guard,
        2,
        player,
        Some(ClientId(1)),
        Some(player),
    );
    assert_eq!(sim.npcs("guard").len(), 2);

    sim.world.despawn(player);
    sim.tick();
    assert!(sim.npcs("guard").is_empty());
}

#[test]
fn a_townsperson_turned_hostile_can_be_fought_until_they_return_friendly() {
    let mut sim = harbour();
    let player = sim.join(1);
    sim.equip(player, "sword");
    let baker = sim.npc("baker");
    assert!(!combat::attackable(&sim.world, player, baker));

    sim.talk(1, "baker");
    sim.pick(1, "Your bread is stale.");
    sim.run(0.5);
    assert!(combat::attackable(&sim.world, player, baker));

    stat::apply_damage(&mut sim.world, baker, 10_000.0);
    sim.tick();
    assert!(stat::is_dead(&sim.world, baker));
    assert!(sim.run_until(10.0, |world| !stat::is_dead(world, baker)));
    assert!(!combat::attackable(&sim.world, player, baker));
}

fn harbour() -> Sim {
    let content = Game::new()
        .area(
            "harbour",
            &Map::open(15, 9),
            json!([
                {
                    "type": "resident",
                    "npc": "gambler.npc.json",
                    "at": { "x": 7.0, "y": 4.0 },
                    "shown": {
                        "type": "all",
                        "of": [
                            { "type": "not", "of": { "type": "memory", "memory": "fighting.memory.json" } },
                            {
                                "type": "not",
                                "of": { "type": "set_within", "memory": "dead.memory.json", "seconds": 1800 },
                            },
                        ],
                    },
                },
                { "type": "resident", "npc": "baker.npc.json", "at": { "x": 10.0, "y": 6.0 } },
            ]),
        )
        .hero("harbour", 5.0, 4.0)
        .sword()
        .item(
            "gold",
            json!({ "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 100_000 }] }),
        )
        .npc("gambler", 40.0, json!([{ "type": "script", "script": "gambler_roll.script.json" }]))
        .script("gambler_roll", "npc", gambling())
        .npc(
            "gambler_hostile",
            40.0,
            json!([
                { "type": "script", "script": "gambler_hostile_hunt.script.json" },
                { "type": "script", "script": "gambler_hostile_defeat.script.json" },
            ]),
        )
        .row("script", "gambler_hostile_hunt", hunting(10.0))
        .script(
            "gambler_hostile_defeat",
            "npc",
            json!({
                "defeated": { "type": "on", "event": "defeated", "next": "over" },
                "over": { "type": "forget", "memories": ["fighting.memory.json"], "next": "dead" },
                "dead": { "type": "remember", "memory": "dead.memory.json" },
            }),
        )
        .edit("npc", "gambler_hostile", |copy| copy["basic_attack"] = json!(path("skill", "strike")))
        .npc("guard", 30.0, json!([{ "type": "script", "script": "guard_hunt.script.json" }]))
        .row("script", "guard_hunt", hunting(10.0))
        .edit("npc", "guard", |guard| guard["basic_attack"] = json!(path("skill", "strike")))
        .npc(
            "baker",
            20.0,
            json!([
                { "type": "respawn", "after": 5.0 },
                { "type": "script", "script": "baker_talk.script.json" },
            ]),
        )
        .script(
            "baker_talk",
            "npc",
            json!({
                "interacted": {
                    "type": "on",
                    "event": "interacted",
                    "verb": "talk",
                    "reach": 2.0,
                    "next": "hello",
                },
                "hello": {
                    "type": "dialogue",
                    "lines": [{ "by": { "type": "narrator" }, "text": ["Fresh bread!"] }],
                    "choices": [{ "label": ["Your bread is stale."], "to": "angry" }],
                },
                "angry": { "type": "set_attitude", "attitude": "hostile" },
            }),
        )
        .memory("grudge", "never")
        .memory("fighting", "leaving_area")
        .memory("dead", "never")
        .vfx("damage_taken")
        .shader("damage_pop")
        .vfx("defeated")
        .load();
    let mut sim = Sim::new(content);
    sim.serve("harbour")
        .accept::<InteractRequest>()
        .accept::<ConversationRequest>()
        .accept::<AttackRequest>()
        .add((
            interact::interact_request,
            dialogue::requests,
            combat::request,
            actor::playing::progress,
            skill::casting::casts,
        ))
        .add((
            (rewards::grant, npc::defeated).chain(),
            script::query::take_tick_roster,
            script::run_tick,
            npc::body::drive,
            movement::advance,
            interact::interactions,
            (dialogue::hold, dialogue::refresh).chain(),
            (npc::run_respawn, npc::dismiss).chain(),
        ))
        .add((
            stat::settle_health,
            combat::defeats,
            skill::hotbar::settle_basic_attack,
            actor::settle,
        ));
    script::spawn_stateless(&mut sim.world);
    npc::spawn_all(&mut sim.world);
    sim
}

fn gambling() -> Value {
    json!({
            "interacted": {
                "type": "on",
                "event": "interacted",
                "verb": "talk",
                "reach": 2.0,
                "next": "holds_grudge",
            },
            "holds_grudge": {
                "type": "if",
                "test": { "type": "set_within", "memory": "grudge.memory.json", "seconds": 1800 },
                "yes": "grudging",
                "no": "hello",
            },
            "hello": dialogue("Ten gold a roll.", json!([
                { "label": ["Roll the dice."], "to": "stake" },
                { "label": ["Not today."] },
            ])),
            "stake": { "type": "items", "take": [{ "item": "gold.item.json", "count": 10 }], "done": "roll" },
            "roll": { "type": "random", "weights": [45, 55], "outputs": ["payout", "wins"] },
            "payout": { "type": "items", "give": [{ "item": "gold.item.json", "count": 20 }], "done": "loses" },
            "loses": dialogue("Beginner's luck.", json!([
                { "label": ["Roll again."], "to": "stake" },
                { "label": ["Quit."] },
            ])),
            "wins": dialogue("The house wins!", json!([
                { "label": ["Roll again."], "to": "stake" },
                { "label": ["You're cheating."], "to": "remember_grudge" },
                { "label": ["Leave."] },
            ])),
            "remember_grudge": { "type": "remember", "memory": "grudge.memory.json", "next": "guards" },
            "guards": {
                "type": "spawn_npcs",
                "npc": "guard.npc.json",
                "count": 2,
                "near": "owner",
                "shown": "subject",
            },
            "grudging": dialogue("You've got some nerve.", json!([
                { "label": ["Then let's settle this."], "to": "remember_fight" },
                { "label": ["I'm leaving."] },
            ])),
            "remember_fight": { "type": "remember", "memory": "fighting.memory.json", "next": "copy" },
            "copy": {
                "type": "spawn_npcs",
                "npc": "gambler_hostile.npc.json",
                "count": 1,
                "near": "owner",
                "shown": "subject",
            },
    })
}

fn lose_at_dice(sim: &mut Sim, client: u32) {
    sim.talk(client, "gambler");
    let mut roll = "Roll the dice.";
    for _ in 0..40 {
        let at_the_table = sim.pick(client, roll).expect("still at the table");
        if lines(&at_the_table) == ["The house wins!"] {
            return;
        }
        roll = "Roll again.";
    }
    panic!("never lost a roll");
}
