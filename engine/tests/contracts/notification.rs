use bevy_ecs::prelude::*;
use engine::core::content::Content;
use engine::core::math::{Offset, Pos};
use engine::core::units::Meters;
use engine::core::vfx::{VfxParams, VfxShaderDef, VfxShaderId};
use engine::systems::area::{self, AreaDef};
use engine::systems::combat::{self, Attitude};
use engine::systems::dialogue::{self, Conversation, ConversationRequest};
use engine::systems::history::{History, HistoryTopic};
use engine::systems::interact::{self, InteractRequest};
use engine::systems::movement::{self, position};
use engine::systems::notification::{self, Notification, NotificationKind, NotificationSent};
use engine::systems::npc::{self, NpcDef};
use engine::systems::player::{AreaDrawn, commands_locked};
use engine::systems::quest::{self, QuestDef};
use engine::systems::status::{self, StatusEffectDef};
use engine::systems::text::LineText;
use engine::systems::{actor, belongings, class, script, stat, vfx};
use serde_json::json;
use testkit::{Game, Map, Sim, path};

const BESIDE_PIER: Pos<Meters> = Pos::new(8.0, 6.0);
const ON_PIER: Pos<Meters> = Pos::new(10.0, 6.0);

#[test]
fn stepping_onto_the_pier_narrates_the_gulls_once_even_during_a_conversation() {
    let mut sim = island(&game());
    let player = sim.join(1);
    let talker = sim.npc("talker");
    movement::relocate(&mut sim.world, talker, BESIDE_PIER);
    movement::relocate(&mut sim.world, player, BESIDE_PIER);
    sim.run(0.5);
    sim.open_with(1, talker);

    movement::relocate(&mut sim.world, player, ON_PIER);
    sim.run(0.5);
    assert_eq!(sim.told(1), vec![sim.spoken("gulls")]);

    sim.later(30.0);
    sim.forget_notified();
    movement::relocate(&mut sim.world, player, BESIDE_PIER);
    sim.run(0.5);
    movement::relocate(&mut sim.world, player, ON_PIER);
    sim.run(0.5);
    assert_eq!(sim.told(1), Vec::new());
    assert!(sim.remembers(player, "heard_gulls"));
    assert!(sim.conversation(1).is_some());
}

#[test]
fn history_keeps_every_notice_and_refusal_raised_and_never_a_pop() {
    let mut sim = island(&game());
    let player = sim.join(1);
    sim.run(0.5);
    let gulls = sim.spoken("gulls");
    let refused = refusal(&sim);

    notification::notify(&mut sim.world, player, gulls);
    notification::notify(&mut sim.world, player, refused);
    notification::pop(&mut sim.world, player, "Ouch", None, VfxParams::default());
    sim.tick();

    let records = recorded(&sim, player);
    assert!(records.contains(&(
        HistoryTopic::Notification,
        Some("Gulls".to_owned()),
        "Gulls squabble over a fish head.".to_owned(),
        1
    )));
    assert!(records.contains(&(HistoryTopic::Error, None, "Needs 10 Gold".to_owned(), 1)));
    assert!(!records.iter().any(|(_, _, text, _)| text == "Ouch"));
    assert_eq!(pops(&mut sim, 1).len(), 1);
}

#[test]
fn a_character_speaks_to_players_on_its_errand_from_its_own_body() {
    let mut sim = island(&game());
    let player = sim.join(1);
    let bystander = sim.join(2);
    let quest = sim.id::<QuestDef>("tusks");
    quest::accept(&mut sim.world, player, quest).expect("accepted");
    let at = position(&sim.world, player).expect("position");
    movement::relocate(&mut sim.world, bystander, at);
    sim.forget_notified();

    let area = sim.id::<AreaDef>("island");
    let def = sim.id::<NpcDef>("chief");
    let chief = npc::spawn(&mut sim.world, def, at + Offset::new(2.0, 0.0), area, None);
    sim.run(0.5);

    let said: Vec<NotificationSent> = sim
        .notified(1)
        .into_iter()
        .filter(|sent| matches!(sent.notification.kind, NotificationKind::Speech { .. }))
        .collect();
    assert_eq!(
        said.iter()
            .map(|sent| sent.notification.clone())
            .collect::<Vec<_>>(),
        vec![sim.spoken("challenge"), sim.spoken("threat")]
    );
    assert!(said.iter().all(|sent| sent.actor == Some(chief)));
    assert!(sim.world.get::<Conversation>(player).is_none());
    assert!(!commands_locked(&sim.world, player));
    assert!(
        !sim.told(2)
            .iter()
            .any(|notification| matches!(notification.kind, NotificationKind::Speech { .. }))
    );

    sim.forget_notified();
    let talker = sim.npc("talker");
    let challenge = sim.spoken("challenge");
    notification::notify_from(&mut sim.world, player, Some(talker), challenge);
    assert_eq!(
        sim.notified(1)
            .into_iter()
            .map(|sent| sent.actor)
            .collect::<Vec<_>>(),
        vec![None],
        "a line someone else speaks names no body"
    );
}

#[test]
fn an_area_intro_plays_on_the_first_arrival_only_and_never_for_the_start_zone() {
    let game = game();
    let mut island = island(&game);
    let player = island.join(1);
    island.run(0.5);
    assert_eq!(intros(&mut island, 1), 0);

    let mut forest = area(&game, "forest");
    let player = travel(&mut island, player, &mut forest);
    let intros_told: Vec<Notification> = forest
        .told(1)
        .into_iter()
        .filter(|notification| matches!(notification.kind, NotificationKind::Intro { .. }))
        .collect();
    assert_eq!(intros_told, vec![forest.spoken("forest_intro")]);

    let mut island = area(&game, "island");
    let player = travel(&mut forest, player, &mut island);
    assert_eq!(intros(&mut island, 1), 0);

    let mut forest = area(&game, "forest");
    travel(&mut island, player, &mut forest);
    assert_eq!(intros(&mut forest, 1), 0);
}

#[test]
fn completing_a_quest_and_reaching_a_level_are_milestones() {
    let mut sim = island(&game());
    let player = sim.join(1);
    let quest = sim.id::<QuestDef>("tusks");
    quest::accept(&mut sim.world, player, quest).expect("accepted");

    quest::complete(&mut sim.world, player, quest);
    class::grant_xp(&mut sim.world, player, 30);
    sim.run(0.5);

    let milestones: Vec<(String, String, HistoryTopic)> = sim
        .told(1)
        .into_iter()
        .filter_map(|notification| match notification.kind {
            NotificationKind::Milestone { label, topic, .. } => {
                Some((label.into_owned(), notification.text.plain_words(), topic))
            }
            _ => None,
        })
        .collect();
    assert!(milestones.contains(&(
        "Level up".to_owned(),
        "Level 2".to_owned(),
        HistoryTopic::Notification
    )));
    assert!(milestones.contains(&(
        "Quest complete".to_owned(),
        "Tusks".to_owned(),
        HistoryTopic::Quest
    )));
}

#[test]
fn the_same_line_from_the_same_speaker_goes_out_once_per_window_and_merges_in_history() {
    let mut sim = island(&game());
    let player = sim.join(1);
    let gulls = sim.spoken("gulls");
    for _ in 0..25 {
        notification::notify(&mut sim.world, player, gulls.clone());
    }
    assert_eq!(sim.told(1), vec![gulls.clone()]);
    let repeats = |sim: &Sim| {
        recorded(sim, player)
            .into_iter()
            .filter(|(_, by, ..)| by.as_deref() == Some("Gulls"))
            .map(|(.., repeats)| repeats)
            .collect::<Vec<_>>()
    };
    assert_eq!(repeats(&sim), vec![25]);

    sim.later(11.0);
    notification::notify(&mut sim.world, player, gulls.clone());
    assert_eq!(sim.told(1), vec![gulls.clone(), gulls]);
}

#[test]
fn a_flood_goes_out_in_a_burst_and_then_at_the_refill_rate() {
    let mut sim = island(&game());
    let player = sim.join(1);
    for line in 0..15 {
        notification::notify(&mut sim.world, player, narration(format!("Beat {line}")));
    }
    assert_eq!(sim.told(1).len(), 10);
    assert_eq!(
        recorded(&sim, player)
            .iter()
            .filter(|(_, by, ..)| by.as_deref() == Some("Drums"))
            .count(),
        15
    );

    sim.later(1.0);
    for line in 15..20 {
        notification::notify(&mut sim.world, player, narration(format!("Beat {line}")));
    }
    assert_eq!(sim.told(1).len(), 12);
}

#[test]
fn every_refusal_answers_its_action_however_many_come() {
    let mut sim = island(&game());
    let player = sim.join(1);
    let refused = refusal(&sim);
    for _ in 0..20 {
        notification::notify(&mut sim.world, player, refused.clone());
    }
    assert_eq!(sim.told(1), vec![refused; 20]);
}

#[test]
fn a_hit_pops_its_amount_out_of_the_one_struck_for_everyone_who_sees_it() {
    let mut sim = island(&game());
    let player = sim.join(1);
    sim.join(2);
    let at = position(&sim.world, player).expect("position");
    let area = sim.id::<AreaDef>("island");
    let def = sim.id::<NpcDef>("dummy");
    let dummy = npc::spawn(&mut sim.world, def, at + Offset::new(1.0, 0.0), area, None);
    sim.world.entity_mut(dummy).insert(Attitude::Hostile);
    sim.run(0.5);
    sim.forget_notified();

    combat::inflict(&mut sim.world, dummy, Some(player), 4.0);
    sim.tick();

    let damage_pop = Some(sim.id::<VfxShaderDef>("damage_pop"));
    for client in [1, 2] {
        assert_eq!(
            pops(&mut sim, client),
            vec![(Some(dummy), "4".to_owned(), damage_pop)]
        );
    }
}

#[test]
fn healing_a_level_and_a_status_each_pop_with_their_own_look() {
    let mut sim = island(&game());
    let player = sim.join(1);
    stat::apply_damage(&mut sim.world, player, 6.0);
    sim.tick();
    sim.forget_notified();

    stat::heal(&mut sim.world, player, 3.0);
    class::grant_xp(&mut sim.world, player, 30);
    let guarded = sim.id::<StatusEffectDef>("guarded");
    status::apply(&mut sim.world, player, guarded, 10.0, None);
    sim.run(0.5);

    let shader = |key: &str| Some(sim.id::<VfxShaderDef>(key));
    let looks = vec![
        (Some(player), "+3".to_owned(), shader("heal_pop")),
        (Some(player), "Level 2".to_owned(), shader("level_pop")),
        (Some(player), "Guarded".to_owned(), shader("status_pop")),
    ];
    assert_eq!(pops(&mut sim, 1), looks);
}

/// An island with a pier the gulls haunt, a forest across the water that greets newcomers, and a
/// chief who taunts whoever is after his tusks.
fn game() -> Content {
    let intro = |notice: &str| {
        json!({
            "discovered": { "type": "on", "event": "discovered", "next": "intro" },
            "intro": { "type": "notify", "notification": path("notification", notice) },
        })
    };
    let island_modules = json!([
        { "type": "script", "script": "island_intro.script.json" },
        { "type": "resident", "npc": "talker.npc.json", "at": { "x": 2.0, "y": 2.0 } },
        {
            "type": "object",
            "name": "pier",
            "shape": { "type": "rect", "x": 9.5, "y": 5.5, "w": 2.0, "h": 2.0 },
            "scripts": [{ "script": "pier_gulls.script.json" }],
        },
    ]);
    Game::new()
        .area("island", &Map::open(12, 12), island_modules)
        .script("island_intro", "area", intro("island_intro"))
        .script(
            "pier_gulls",
            "object",
            json!({
                "entered": { "type": "on", "event": "entered", "by": "characters", "next": "heard" },
                "heard": { "type": "if", "test": { "type": "set_within", "memory": "heard_gulls.memory.json", "seconds": 600 }, "no": "remember" },
                "remember": { "type": "remember", "memory": "heard_gulls.memory.json", "next": "gulls" },
                "gulls": { "type": "notify", "notification": "gulls.notification.json" },
            }),
        )
        .area("forest", &Map::open(8, 8), json!([{ "type": "script", "script": "forest_intro.script.json" }]))
        .script("forest_intro", "area", intro("forest_intro"))
        .hero("island", 3.0, 8.0)
        .edit("class", "hero", |hero| {
            hero["levels"] = json!([{ "xp": 0, "skill_points": 1 }, { "xp": 20 }])
        })
        .npc(
            "talker",
            40.0,
            json!([{ "type": "script", "script": "talker_talk.script.json" }]),
        )
        .script(
            "talker_talk",
            "npc",
            json!({
                "interacted": { "type": "on", "event": "interacted", "verb": "talk", "reach": 3.0, "next": "hello" },
                "hello": {
                    "type": "dialogue",
                    "lines": [{ "by": { "type": "narrator" }, "text": ["Fine weather."] }],
                    "choices": [{ "label": ["Goodbye."] }],
                },
            }),
        )
        .babble("chief")
        .npc(
            "chief",
            40.0,
            json!([{ "type": "voice", "babble": "chief.babble.json" }, { "type": "script", "script": "chief_taunt.script.json" }]),
        )
        .script(
            "chief_taunt",
            "npc",
            json!({
                "came_near": { "type": "on", "event": "came_near", "within": 8.0, "by": "characters", "next": "hunting_me" },
                "hunting_me": {
                    "type": "if",
                    "test": { "type": "all", "of": [
                        { "type": "on_quest", "quest": "tusks.quest.json" },
                        { "type": "not", "of": { "type": "set_within", "memory": "taunted.memory.json", "seconds": 120 } },
                    ] },
                    "yes": "taunted",
                },
                "taunted": { "type": "remember", "memory": "taunted.memory.json", "next": "challenge" },
                "challenge": { "type": "notify", "notification": "challenge.notification.json", "next": "threat" },
                "threat": { "type": "notify", "notification": "threat.notification.json" },
            }),
        )
        .npc("dummy", 40.0, json!([]))
        .item("tusk", json!({ "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 50 }] }))
        .row(
            "quest",
            "tusks",
            json!({
                "title": "Tusks",
                "category": "Errands",
                "blurb": "Bring tusks.",
                "xp": 10,
                "modules": [
                    { "type": "hold", "item": "tusk.item.json", "count": 5 },
                    { "type": "hand_in", "items": [{ "item": "tusk.item.json", "count": 5 }] },
                    { "type": "hand_over", "text": ["Here are your tusks."] },
                ],
            }),
        )
        .row(
            "status_effect",
            "guarded",
            json!({
                "name": "Guarded",
                "icon": "icons/status.png",
                "category": "buff",
                "description": "Guarded.",
                "vfx": {},
                "modules": [],
            }),
        )
        .asset("icons/status.png", testkit::png(1, 1))
        .row(
            "notification",
            "gulls",
            json!({
                "kind": { "type": "narration", "label": "Gulls", "sfx": "gull_cries.sfx" },
                "text": ["Gulls squabble over a fish head."],
            }),
        )
        .row(
            "notification",
            "challenge",
            json!({ "kind": { "type": "speech", "speaker": "chief.npc.json" }, "text": ["WHO DARES?!"] }),
        )
        .row(
            "notification",
            "threat",
            json!({ "kind": { "type": "speech", "speaker": "chief.npc.json" }, "text": ["I'll grind your bones!"] }),
        )
        .row("notification", "island_intro", intro_row("The island"))
        .row("notification", "forest_intro", intro_row("The forest"))
        .memory("heard_gulls", "never")
        .memory("taunted", "never")
        .sfx("gull_cries")
        .sfx("drums")
        .sfx("ui_refuse")
        .sfx("ui_close")
        .sfx("ui_chime")
        .sfx("rising_chime")
        .sfx("tally_tick")
        .sfx("quest_accepted")
        .sfx("quest_completed")
        .vfx("damage_taken")
        .vfx("healed")
        .vfx("level_up")
        .vfx("defeated")
        .shader("pop")
        .shader("damage_pop")
        .shader("heal_pop")
        .shader("level_pop")
        .shader("status_pop")
        .mark("quest_ready")
        .mark("quest_offered")
        .mark("repeatable_ready")
        .mark("repeatable_offered")
        .mark("quest_in_progress")
        .mark("quest_locked")
        .load()
}

fn intro_row(title: &str) -> serde_json::Value {
    json!({
        "kind": { "type": "intro", "title": title, "tint": [143, 209, 139, 255], "motion": "pulse", "sfx": "drums.sfx" },
        "text": ["Pines close in around you."],
        "lasts": 5.0,
    })
}

fn island(game: &Content) -> Sim {
    let mut sim = area(game, "island");
    npc::spawn_all(&mut sim.world);
    area::object::spawn_all(&mut sim.world);
    sim
}

/// The server of the area under `key`, running what raises and carries notifications.
fn area(game: &Content, key: &str) -> Sim {
    let mut sim = Sim::new(game.clone());
    sim.serve(key)
        .accept::<AreaDrawn>()
        .accept::<InteractRequest>()
        .accept::<ConversationRequest>()
        .add((
            (status::expire, status::aspect::settle, status::aspect::tick).chain(),
            vfx::expire,
            engine::systems::player::arrival_holds,
            interact::interact_request,
            dialogue::requests,
        ))
        .add((
            script::query::take_tick_roster,
            area::object::track,
            npc::approach,
            script::run_tick,
            movement::advance,
            interact::interactions,
            dialogue::hold,
            dialogue::refresh,
        ))
        .add((
            stat::settle_health,
            combat::defeats,
            actor::settle,
            belongings::notify_changes,
        ));
    script::spawn_stateless(&mut sim.world);
    sim
}

/// Hands `player` over from one area's server to the other's, and lets it settle in.
fn travel(from: &mut Sim, player: Entity, to: &mut Sim) -> Entity {
    let arrived = from.cross(player, to, Pos::new(3.0, 3.0));
    to.run(0.5);
    arrived
}

fn refusal(sim: &Sim) -> Notification {
    Notification::new(
        NotificationKind::error(sim.world.resource::<Content>()),
        LineText::plain("Needs 10 Gold"),
    )
}

fn pops(sim: &mut Sim, client: u32) -> Vec<(Option<Entity>, String, Option<VfxShaderId>)> {
    sim.notified(client)
        .into_iter()
        .filter_map(|sent| match sent.notification.kind {
            NotificationKind::Pop { shader, .. } => {
                Some((sent.actor, sent.notification.text.plain_words(), shader))
            }
            _ => None,
        })
        .collect()
}

fn recorded(sim: &Sim, player: Entity) -> Vec<(HistoryTopic, Option<String>, String, u32)> {
    sim.world
        .get::<History>(player)
        .map(|history| {
            history
                .records()
                .map(|record| {
                    let entry = &record.entry;
                    (
                        entry.topic,
                        entry.by.clone(),
                        entry.text.plain_words(),
                        entry.repeats,
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn narration(text: String) -> Notification {
    Notification::new(
        NotificationKind::Narration {
            label: "Drums".into(),
            sfx: None,
        },
        LineText::plain(text),
    )
}

fn intros(sim: &mut Sim, client: u32) -> usize {
    sim.told(client)
        .iter()
        .filter(|notification| matches!(notification.kind, NotificationKind::Intro { .. }))
        .count()
}
