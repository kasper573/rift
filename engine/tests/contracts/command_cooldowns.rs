use bevy_ecs::message::MessageCursor;
use bevy_replication::ToClient;
use bevy_terminal::{LineKind, TerminalInput, TerminalLine};
use engine::core::content::Content;
use engine::core::math::Pos;
use engine::core::time::WallClock;
use engine::core::units::{MeterPos, Meters, Seconds};
use engine::systems::area::AreaDef;
use engine::systems::area::Place;
use engine::systems::chat::ChatChannel;
use engine::systems::npc::{self, NpcDef};
use engine::systems::player::snapshot::{CharacterSnapshot, JoinAs, JoinOrigin};
use engine::systems::player::{self, AreaDrawn};
use engine::systems::{actor, chat, combat, command, movement, skill, stat};
use serde_json::json;
use testkit::{Game, Map, Sim, path};

const SPAWN: Pos<Meters> = Pos::new(2.0, 2.0);
const AFIELD: Pos<Meters> = Pos::new(9.0, 9.0);

#[test]
fn a_command_cools_down_for_its_sender_once_it_runs_and_not_when_it_is_refused() {
    let mut sim = camp(game());
    let me = sim.join(1);
    let home = |sim: &mut Sim| {
        sim.run(0.5);
        let conn = sim.conn(1);
        sim.send(conn, AreaDrawn);
        sim.tick();
        movement::position(&sim.world, sim.player(1))
            .is_some_and(|at| at.distance(SPAWN) < Meters(1.0))
    };

    for _ in 0..2 {
        movement::relocate(&mut sim.world, me, AFIELD);
        send(&mut sim, "/home");
        assert!(
            home(&mut sim),
            "a command without a cooldown runs every time, here casting a skill its sender doesn't know"
        );
    }

    movement::relocate(&mut sim.world, me, AFIELD);
    let (dummy, camp_area) = (sim.id::<NpcDef>("dummy"), sim.id::<AreaDef>("camp"));
    let dummy = npc::spawn(&mut sim.world, dummy, Pos::new(9.0, 8.0), camp_area, None);
    combat::inflict(&mut sim.world, me, Some(dummy), 1.0);
    send(&mut sim, "/rest");
    assert_eq!(
        replies(&mut sim).last(),
        Some(&("/rest needs you out of combat".to_owned(), LineKind::Failed)),
        "what the command requires is checked before it runs"
    );
    assert!(!home(&mut sim));
    sim.run(2.5);
    send(&mut sim, "/rest");
    let said = replies(&mut sim).last().cloned();
    assert_eq!(
        (home(&mut sim), said),
        (
            true,
            Some((
                "/rest can be used again in 10 min.".to_owned(),
                LineKind::Notice
            ))
        ),
        "a refused command started no cooldown; one that runs starts it and says so"
    );

    movement::relocate(&mut sim.world, me, AFIELD);
    for (wait, left) in [(0.0, "10 min"), (60.0, "9 min"), (500.0, "38 s")] {
        sim.later(wait);
        send(&mut sim, "/rest");
        assert_eq!(
            replies(&mut sim).last(),
            Some(&(
                format!("/rest can be used again in {left}."),
                LineKind::Failed
            )),
            "the time left, to the minute above a minute and to the second below it"
        );
    }
    assert!(!home(&mut sim), "a command cooling down does nothing");
    send(&mut sim, "/commands");
    let listed: Vec<String> = replies(&mut sim)
        .into_iter()
        .map(|(text, _)| text)
        .collect();
    assert!(
        listed
            .iter()
            .any(|line| line == "/rest · every 10 min · ready in 38 s"),
        "the list of commands says each one's cooldown and the time left: {listed:?}"
    );

    let content = sim.world.resource::<Content>().clone();
    let snapshot = CharacterSnapshot::of_live(&sim.world, me).expect("a character to save");
    let camp_area = content
        .by_path::<AreaDef>(&path("area", "camp"))
        .expect("the camp");
    let now = sim.world.resource::<WallClock>().now;
    let mut next = camp(game());
    next.world.resource_mut::<WallClock>().now = now.after(Seconds(20.0));
    next.join_as(
        1,
        JoinAs {
            snapshot,
            start: Place {
                area: camp_area,
                at: AFIELD,
            },
            origin: JoinOrigin::Saved {
                at: now,
                told: Vec::new(),
            },
        },
    );
    next.run(0.5);
    send(&mut next, "/rest");
    let left = replies(&mut next).last().and_then(|(said, kind)| {
        let seconds = said
            .strip_prefix("/rest can be used again in ")?
            .strip_suffix(" s.")?;
        (*kind == LineKind::Failed).then(|| seconds.parse::<u32>().ok())?
    });
    assert!(
        left.is_some_and(|left| left < 20),
        "a cooldown is saved with the character, and runs on while they are away: {left:?}"
    );
    next.later(20.0);
    send(&mut next, "/rest");
    assert!(home(&mut next), "and once it runs out, the command runs");
}

/// A camp to stray from, a dummy to fight, and two commands that send their sender home by a
/// skill nobody knows: `/home` at any time, and `/rest` out of combat, every ten minutes.
fn game() -> Game {
    let homeward = json!({
        "runs_on": "command",
        "nodes": {
            "sent": { "type": "on", "event": "invoked", "next": "home" },
            "home": { "type": "use_skill", "skill": path("skill", "homeward"), "lent": true },
        },
    });
    Game::new()
        .area("camp", &Map::open(12, 12), json!([]))
        .hero("camp", SPAWN.x, SPAWN.y)
        .npc("dummy", 100.0, json!([]))
        .sfx("ui_refuse")
        .vfx("damage_taken")
        .shader("damage_pop")
        .asset("icons/skill.png", testkit::png(1, 1))
        .edit_settings(|settings| settings["claims_last"] = json!(2.0))
        .row(
            "skill",
            "homeward",
            json!({
                "name": "Homeward",
                "icon": "icons/skill.png",
                "target": "self",
                "levels": [{ "cost": 0, "stats": {} }],
                "modules": [{ "type": "script", "script": path("script", "homeward_used") }],
            }),
        )
        .script(
            "homeward_used",
            "skill",
            json!({
                "used": { "type": "on", "event": "used", "next": "back" },
                "back": { "type": "return_to_spawn" },
            }),
        )
        .row("script", "home", homeward.clone())
        .row("script", "rest", homeward)
        .row(
            "command",
            "home",
            json!({ "script": path("script", "home") }),
        )
        .row(
            "command",
            "rest",
            json!({
                "script": path("script", "rest"),
                "requires": { "type": "not", "of": { "type": "in_combat" } },
                "cooldown": 600.0,
            }),
        )
        .row(
            "script",
            "list",
            json!({
                "runs_on": "command",
                "nodes": {
                    "sent": { "type": "on", "event": "invoked", "next": "list" },
                    "list": { "type": "list_commands" },
                },
            }),
        )
        .row(
            "command",
            "commands",
            json!({ "script": path("script", "list") }),
        )
}

fn camp(game: Game) -> Sim {
    let mut sim = Sim::new(game.load());
    sim.serve("camp")
        .accept::<TerminalInput>()
        .accept::<AreaDrawn>()
        .message::<bevy_terminal::TerminalSaid>()
        .message::<bevy_terminal::TerminalCommanded>()
        .message::<ToClient<TerminalLine>>()
        .add((
            chat::hear,
            command::run,
            command::retire,
            player::arrival_holds,
            actor::playing::progress,
        ))
        .plan(skill::casting::StepCasts::default())
        .plan((
            engine::systems::roster::ListRoster::default(),
            engine::core::script::run::FlowRunner::default(),
            movement::AdvanceWalks::default(),
        ))
        .plan(stat::SettleVitals::default())
        .plan(actor::SettleActions::default());
    chat::route_lines(&mut sim.world);
    engine::systems::spawn_scripts(&mut sim.world);
    sim
}

fn send(sim: &mut Sim, line: &str) {
    let conn = sim.conn(1);
    sim.send(
        conn,
        TerminalInput {
            text: line.to_owned(),
            channel: ChatChannel::Area.terminal(),
            embeds: Vec::new(),
        },
    );
    sim.tick();
}

fn replies(sim: &mut Sim) -> Vec<(String, LineKind)> {
    let conn = sim.conn(1);
    sim.read(&mut MessageCursor::<ToClient<TerminalLine>>::default())
        .into_iter()
        .filter(|sent| sent.client == conn)
        .map(|sent| (sent.message.text, sent.message.kind))
        .collect()
}
