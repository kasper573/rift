use std::sync::Arc;

use engine::core::content::{Content, ContentBuilder, ContentScope, Table, with_content};
use engine::core::math::Pos;
use engine::core::script::compile::compile_owner;
use engine::core::script::{EffectDef, Expr, ScriptError};
use engine::systems::item::ItemId;
use engine::systems::memory::MemoryId;
use engine::systems::script::{
    EventKind, OwnerKind, Place, Procedure, Reach, RiftScripting, ScriptDef, ScriptModule,
};
use engine::systems::shop::ShopId;
use serde_json::{Value, json};

use crate::json::{effect, inline, inline_with, parse};
use testkit::{content, row};

const OWNERS: [OwnerKind; 6] = [
    OwnerKind::Object,
    OwnerKind::Npc,
    OwnerKind::Item,
    OwnerKind::Area,
    OwnerKind::Quest,
    OwnerKind::Shop,
];

const FUNCTIONS: [&str; 2] = ["helper", "loops"];

#[test]
fn every_capability_compiles_for_exactly_the_owners_it_lists() {
    let capabilities = every_capability();
    assert_eq!(capabilities.len(), 38);
    for owner in OWNERS {
        for value in &capabilities {
            let parsed = effect(value.clone());
            let kind = parsed.kind();
            let allowed = parsed.owners().contains(&owner);
            let script = inline(json!({
                "on": entry(owner, "do"),
                "do": value,
            }));
            let compiled = compile(owner, &[&script]);
            assert_eq!(
                compiled.is_ok(),
                allowed,
                "{kind} for {owner:?}: {:?}",
                compiled.err().map(|error| error.to_string())
            );
        }
    }
}

#[test]
fn may_do_lists_exactly_what_a_script_reaches() {
    let gold: ItemId = row("Gold");
    let pass: ItemId = row("RoadPass");
    let bribed: MemoryId = row("BribedIlsa");
    let honesty_box: ShopId = row("HonestyBox");
    let place = Place {
        area: row("Forest"),
        at: Pos::new(3.0, 18.0),
    };
    let script = inline(json!({
        "on": entry(OwnerKind::Npc, "hello"),
        "hello": {
            "type": "dialogue",
            "lines": [{ "by": { "type": "narrator" }, "text": ["Pay or go."] }],
            "choices": [
                { "label": ["Pay."], "to": "pay" },
                { "label": ["Go."], "to": "go" }
            ]
        },
        "pay": {
            "type": "items",
            "take": [{ "item": "gold", "count": 5 }],
            "give": [{ "item": "road_pass" }],
            "done": "warp"
        },
        "warp": { "type": "warp", "to": { "area": "forest", "x": 3.0, "y": 18.0 } },
        "go": { "type": "wait", "seconds": 1, "at_most": 5.0, "next": "mark" },
        "mark": { "type": "remember", "memory": "bribed_ilsa", "next": "shop" },
        "shop": { "type": "open_shop", "shop": "honesty_box" },
    }));

    let procedures = compile(OwnerKind::Npc, &[&script]).expect("compiles");
    let may_do = &procedures[0].may_do;

    assert_eq!(may_do.answers, vec![EventKind::Interacted]);
    assert_eq!(may_do.dialogue, vec!["hello"]);
    assert!(may_do.calls.is_empty());
    assert!(may_do.waits);
    let expected = [
        Reach::Takes(gold),
        Reach::Gives(pass),
        Reach::Warp(place),
        Reach::Remembers(bribed),
        Reach::OpensShop(honesty_box),
    ];
    assert_eq!(may_do.reaches.len(), expected.len(), "{:?}", may_do.reaches);
    for reach in expected {
        assert!(may_do.reaches.contains(&reach), "{reach:?} missing");
    }
}

#[test]
fn a_loop_without_a_wait_is_refused() {
    let script = inline(json!({
        "on": entry(OwnerKind::Npc, "a"),
        "a": { "type": "if", "test": true, "yes": "b" },
        "b": { "type": "if", "test": true, "yes": "a" },
    }));

    let error = compile(OwnerKind::Npc, &[&script]).err().expect("refused");

    assert_eq!(error.rule, "loop without a wait");
    assert_eq!(error.expected, "a → b → a");
}

#[test]
fn too_many_steps_between_pauses_are_refused() {
    let script = inline(json!({
        "on": entry(OwnerKind::Npc, "outer"),
        "outer": { "type": "repeat", "times": 1000, "at_most": 1000, "each": "inner" },
        "inner": { "type": "repeat", "times": 1000, "at_most": 1000, "each": "check" },
        "check": { "type": "if", "test": true },
    }));

    let error = compile(OwnerKind::Npc, &[&script]).err().expect("refused");

    assert_eq!(error.rule, "too many steps");
}

#[test]
fn a_decision_reaches_as_deciding_a_path_not_completing_a_task() {
    let script = inline(json!({
        "on": entry(OwnerKind::Npc, "decide"),
        "decide": {
            "type": "decide",
            "quest": "shamans_plea",
            "path": "Keep them",
        },
    }));

    let procedures = compile(OwnerKind::Npc, &[&script]).expect("compiles");
    let reaches = &procedures[0].may_do.reaches;

    assert!(
        reaches.contains(&Reach::Decides(row("ShamansPlea"))),
        "{reaches:?}"
    );
    assert!(
        !reaches.contains(&Reach::Completes(row("ShamansPlea"))),
        "{reaches:?}"
    );
}

#[test]
fn an_effect_or_query_refuses_by_its_own_rule() {
    let effect = inline(json!({
        "on": entry(OwnerKind::Npc, "a"),
        "a": { "type": "complete_task", "quest": "x_marks_the_spot", "task": "no_such_task" },
    }));
    let query = inline(json!({
        "on": entry(OwnerKind::Npc, "a"),
        "a": {
            "type": "if",
            "test": { "type": "task_done", "quest": "x_marks_the_spot", "task": "no_such_task" },
        },
    }));

    for script in [effect, query] {
        let error = compile(OwnerKind::Npc, &[&script]).err().expect("refused");

        assert_eq!(error.rule, "unknown task");
        assert_eq!(error.node.as_deref(), Some("a"));
    }
}

#[test]
fn a_script_that_runs_itself_is_refused() {
    let error = compile_owner(&functions(), OwnerKind::Npc, "test", &[&calling("loops")])
        .err()
        .expect("refused")
        .remove(0)
        .1;

    assert_eq!(error.rule, "runs itself");
    assert!(error.expected.contains("loops"), "{}", error.expected);
}

#[test]
fn may_do_names_the_functions_a_script_calls() {
    let procedures = compile_owner(&functions(), OwnerKind::Npc, "test", &[&calling("helper")])
        .expect("compiles");

    let may_do = &procedures[0].may_do;
    assert_eq!(may_do.calls, vec!["helper".to_owned()]);
    assert_eq!(may_do.answers, vec![EventKind::Started]);
    assert!(may_do.reaches.is_empty());
    assert!(!may_do.waits);
}

#[test]
fn an_if_always_taken_still_compiles_both_arrows() {
    let script = inline(json!({
        "on": entry(OwnerKind::Area, "check"),
        "check": { "type": "if", "test": { "type": "not", "of": true }, "yes": "yes", "no": "no" },
        "yes": { "type": "notify", "notification": "gulls" },
        "no": { "type": "notify", "notification": "forest_intro" },
    }));

    let procedures = compile(OwnerKind::Area, &[&script]).expect("compiles");

    assert_eq!(procedures[0].may_do.reaches.len(), 2);
}

#[test]
fn every_expression_shape_serializes_back_to_the_json_it_was_read_from() {
    let shapes = json!([
        true,
        3,
        2.5,
        "text",
        { "var": "aggro" },
        { "type": "local", "local": "target" },
        { "type": "state", "state": "count" },
        { "type": "event", "event": "started" },
        { "type": "all", "of": [true, { "type": "not", "of": false }] },
        { "type": "any", "of": [false] },
        { "type": "join", "of": ["a", "b"] },
        { "type": "round", "of": 2.5 },
        { "type": "not_equal", "a": { "var": "aggro" }, "b": 4 },
        { "type": "at_least", "a": { "var": "aggro" }, "b": 4 },
        { "type": "divide", "a": 9, "b": 3 }
    ]);
    for shape in shapes.as_array().expect("a list") {
        let expr: Expr<RiftScripting> = parse(shape.clone());
        assert_eq!(serde_json::to_value(&expr).expect("json"), *shape);
    }
}

#[test]
fn a_shared_script_row_reads_back_as_written() {
    let def: ScriptDef = parse(json!({
        "owner": "npc",
        "variables": { "aggro": { "type": "number", "default": 4 } },
        "locals": { "target": "actor" },
        "nodes": {
            "spawned": { "type": "on", "event": "started", "next": "look" },
            "look": {
                "type": "set",
                "values": [{ "local": "target", "to": { "type": "nearest", "among": "characters", "within": { "var": "aggro" } } }]
            }
        }
    }));

    assert_eq!(def.owner, OwnerKind::Npc);
    assert_eq!(def.variables.len(), 1);
    assert_eq!(def.nodes.len(), 2);
}

fn compile(
    owner: OwnerKind,
    modules: &[&ScriptModule],
) -> Result<Vec<Arc<Procedure>>, ScriptError> {
    compile_owner(content(), owner, "test", modules).map_err(|mut errors| errors.remove(0).1)
}

fn entry(owner: OwnerKind, next: &str) -> Value {
    match owner {
        OwnerKind::Object | OwnerKind::Npc => json!({
            "type": "on", "event": "interacted", "verb": "talk", "reach": 2.0, "next": next
        }),
        OwnerKind::Item | OwnerKind::Skill => {
            json!({ "type": "on", "event": "used", "next": next })
        }
        OwnerKind::Area => json!({ "type": "on", "event": "discovered", "next": next }),
        OwnerKind::Quest => json!({ "type": "on", "event": "accepted", "next": next }),
        OwnerKind::Shop => json!({ "type": "on", "event": "bought", "next": next }),
        OwnerKind::StatusEffect => json!({ "type": "on", "event": "activated", "next": next }),
        OwnerKind::Blueprint | OwnerKind::Any => unreachable!("no entry for {owner:?}"),
    }
}

fn every_capability() -> Vec<Value> {
    vec![
        json!({ "type": "warp", "to": { "area": "forest", "x": 3.0, "y": 18.0 } }),
        json!({ "type": "heal", "by": "full" }),
        json!({ "type": "damage", "by": 10.0 }),
        json!({ "type": "apply_status", "effect": "guarded", "power": 10.0, "seconds": 10.0 }),
        json!({ "type": "remove_status", "category": "debuff" }),
        json!({ "type": "items", "take": [{ "item": "gold" }] }),
        json!({ "type": "use_item", "item": "tattered_map" }),
        json!({ "type": "equip", "item": "road_pass" }),
        json!({ "type": "unequip", "slot": "weapon" }),
        json!({ "type": "drop_items", "item": "gold", "count": 1 }),
        json!({ "type": "spawn_ground_items", "items": [{ "item": "gold" }], "for": "subject" }),
        json!({ "type": "clear_ground_items", "within": 10.0 }),
        json!({
            "type": "move_items", "from": { "type": "subject" }, "to": { "type": "subject" },
            "items": [{ "item": "gold" }]
        }),
        json!({ "type": "grant_xp", "xp": 1 }),
        json!({ "type": "remember", "memory": "bribed_ilsa" }),
        json!({ "type": "forget", "memories": ["bribed_ilsa"] }),
        json!({ "type": "offer_quest", "quest": "x_marks_the_spot" }),
        json!({ "type": "accept_quest", "quest": "x_marks_the_spot" }),
        json!({ "type": "turn_in", "quest": "x_marks_the_spot" }),
        json!({ "type": "fail_quest", "quest": "x_marks_the_spot" }),
        json!({ "type": "complete_task", "quest": "x_marks_the_spot", "task": "find_stone" }),
        json!({ "type": "decide", "quest": "shamans_plea", "path": "Keep them" }),
        json!({ "type": "open_shop", "shop": "honesty_box" }),
        json!({ "type": "notify", "notification": "gulls" }),
        json!({ "type": "say", "text": ["Hm."] }),
        json!({ "type": "wander", "priority": 1 }),
        json!({ "type": "walk_to", "to": { "type": "subject" }, "within": 1, "priority": 3 }),
        json!({ "type": "attack", "target": { "type": "subject" }, "within": 10.0, "priority": 5 }),
        json!({ "type": "stop", "priority": 9 }),
        json!({ "type": "face", "toward": { "type": "subject" } }),
        json!({ "type": "animate", "action": "attack", "wait": "apex" }),
        json!({ "type": "use_skill", "skill": "slash", "at": { "type": "subject" } }),
        json!({ "type": "reset_skills" }),
        json!({ "type": "vfx", "vfx": "damage_taken", "params": { "amount": 3.0 } }),
        json!({ "type": "set_attitude", "attitude": "hostile" }),
        json!({ "type": "spawn_npcs", "npc": "orc", "count": 1, "near": "subject", "shown": "everyone" }),
        json!({ "type": "despawn" }),
        json!({ "type": "play_sound", "sfx": "ui_pick", "for": "subject" }),
    ]
}

fn functions() -> Content {
    let mut scope = ContentScope::new();
    scope.table::<ScriptDef>(FUNCTIONS.map(str::to_owned).to_vec());
    let parse = |value: Value| -> ScriptDef {
        let (parsed, _) = with_content(scope.clone(), || serde_json::from_value(value));
        parsed.expect("a script row")
    };
    let helper = parse(json!({
        "owner": "any",
        "nodes": {
            "run": { "type": "on", "event": "run", "next": "done" },
            "done": { "type": "return" }
        }
    }));
    let loops = parse(json!({
        "owner": "any",
        "nodes": {
            "run": { "type": "on", "event": "run", "next": "again" },
            "again": { "type": "run", "script": "loops" }
        }
    }));
    let mut tables = ContentBuilder::default();
    tables.insert_table(Table::new(vec![
        ("helper".to_owned(), helper),
        ("loops".to_owned(), loops),
    ]));
    tables.build()
}

fn calling(script: &str) -> ScriptModule {
    let mut scope = ContentScope::new();
    scope.table::<ScriptDef>(FUNCTIONS.map(str::to_owned).to_vec());
    let (parsed, _) = with_content(scope, || {
        serde_json::from_value::<ScriptModule>(json!({
            "type": "inline_script",
            "nodes": {
                "on": { "type": "on", "event": "started", "next": "call" },
                "call": { "type": "run", "script": script }
            }
        }))
    });
    parsed.expect("a module")
}

#[test]
fn a_trade_field_reads_where_every_entry_reaching_the_node_raises_it() {
    let script = inline_with(
        json!({ "price": "integer" }),
        json!({
            "on": { "type": "on", "event": "bought", "next": "read" },
            "read": { "type": "set", "values": [{ "local": "price", "to": { "type": "event", "event": "count" } }], "next": "check" },
            "check": { "type": "if", "test": { "type": "more", "a": { "type": "local", "local": "price" }, "b": 0 } },
        }),
    );

    compile(OwnerKind::Shop, &[&script]).expect("compiles");
}

#[test]
fn an_event_field_is_refused_where_the_entry_reaching_it_raises_no_fields() {
    let script = inline_with(
        json!({ "price": "integer" }),
        json!({
            "on": { "type": "on", "event": "started", "next": "read" },
            "read": { "type": "set", "values": [{ "local": "price", "to": { "type": "event", "event": "count" } }], "next": "check" },
            "check": { "type": "if", "test": { "type": "more", "a": { "type": "local", "local": "price" }, "b": 0 } },
        }),
    );

    let error = compile(OwnerKind::Object, &[&script])
        .err()
        .expect("refused");

    assert_eq!(error.rule, "unknown field");
}
