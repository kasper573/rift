use bevy_ecs::message::MessageCursor;
use bevy_ecs::prelude::*;
use bevy_replication::ToClient;
use engine::core::content::{Content, ContentScope};
use engine::core::math::Pos;
use engine::core::store::{Record, RecordRow};
use engine::core::time::{UnixMillis, UtcHour, WallClock};
use engine::core::units::Seconds;
use engine::systems::account::{AccountId, AccountIdentity};
use engine::systems::actor::plate;
use engine::systems::area::{self, AreaDef};
use engine::systems::class::NodeKey;
use engine::systems::history::{History, HistoryNews};
use engine::systems::item::{Inventory, ItemDef, ItemStack};
use engine::systems::movement::{self, Position};
use engine::systems::player::snapshot::{
    CharacterSnapshot, CharacterStart, JoinAs, JoinOrigin, QuestState, SnapshotQuest, SnapshotSkill,
};
use engine::systems::player::{
    self, Away, CharacterState, ClientId, Departures, Players, SpawnPoint, SpawnPolicy, SpawnRules,
    Xp,
};
use engine::systems::quest::QuestDef;
use engine::systems::save::load::{self, LoadedCharacter};
use engine::systems::save::notice::{self, ChangeLine, GameChanged, Returned, SavingNews};
use engine::systems::save::{self, GameChange, SavedCharacter, ServerSaving};
use engine::systems::script::Place;
use engine::systems::skill::{self, Rank};
use engine::systems::society::{Account, Society};
use engine::systems::stat::{self, StatKind};
use engine::systems::worlds::{self, AboveWorlds};
use engine::systems::{GameSettings, combat};
use serde_json::json;
use testkit::{Game, Map, Sim};

use crate::saves::{Saves, checkpoint, confer};

#[test]
fn what_a_server_saved_as_it_stopped_is_what_the_next_one_plays_on() {
    let saves = Saves::in_a_file();
    let content = game(Some(&Map::open(9, 9))).load();
    let mut village = serving(&content, "village");
    let mut above = saves.above(&content);

    let wren = village.sign_in(1, "wren", None);
    let now = village.world.resource::<WallClock>().now;
    above
        .society
        .signed_in(&identity("wren"), now, &mut above.checkpoints.unwritten);
    village.give(wren, "token", 7);
    village.world.get_mut::<Xp>(wren).expect("xp").gain(40);
    movement::relocate(&mut village.world, wren, Pos::new(6.0, 2.0));
    let staged = JoinAs {
        snapshot: CharacterSnapshot::of_player(&content, "Stand-in", player::def(&content)),
        start: content.built::<GameSettings>().initial_spawn,
        origin: JoinOrigin::Staged,
    };
    village.sign_in(2, "hale", Some(staged));
    let left = state(&village, wren);

    let now = village.world.resource::<WallClock>().now;
    save::save_all(&mut [Some(&mut village.world)], &mut above.checkpoints, now)
        .expect("the store takes the game");
    assert_eq!(saves.character("hale"), None, "a stand-in is never saved");
    let held = saves.character("wren").expect("the account's own is");

    let content = game(Some(&Map::open(9, 9))).load();
    let society = saves
        .runtime
        .block_on(Society::read(
            &saves.store,
            &ContentScope::of(&content),
            Vec::new(),
        ))
        .expect("the store is read");
    let known = society.accounts().get(&AccountId("wren".to_owned()));
    assert_eq!(
        known.map(|account| (account.name.as_str(), account.first_seen)),
        Some(("wren", now)),
        "who signed in is known to the next server"
    );
    let mut next = serving(&content, "village");
    next.later(3600.0);
    let loaded = load::load(&content, &held).expect("a character this build saved");
    assert_eq!(loaded.changes, [], "the game is as it was");
    let back = next.sign_in(1, "wren", Some(joining(loaded)));
    assert_eq!(state(&next, back), left);
    let stands = next.world.get::<Position>(back).expect("a position").pos;
    assert!(
        stands.distance_to(Pos::new(6.0, 2.0)) <= 1.5,
        "she stands where she stood, not at {stands:?}"
    );
}

#[test]
fn a_checkpoint_the_store_refuses_is_made_good_by_the_next_and_players_are_told_meanwhile() {
    let saves = Saves::in_a_file();
    let content = game(Some(&Map::open(9, 9))).load();
    let mut village = serving(&content, "village");
    let mut above = saves.above(&content);
    let wren = village.sign_in(1, "wren", None);
    let began = village.world.resource::<WallClock>().now;
    above
        .society
        .signed_in(&identity("wren"), began, &mut above.checkpoints.unwritten);
    confer(&mut [&mut village], &mut above);
    let mut told = MessageCursor::default();

    saves.refuse_characters(true);
    for failed in 1..=3 {
        let report = checkpoint(&mut [&mut village], &mut above);
        assert!(report.outcome.is_err(), "the store refuses it");
        assert_eq!(
            above.checkpoints.waiting(),
            1,
            "the account still waits to be written"
        );
        assert_eq!(
            saves.all(Account::KIND),
            Vec::<String>::new(),
            "nothing of a checkpoint is written unless all of it is"
        );
        village.tick();
        let news: Vec<SavingNews> = heard(&mut village, &mut told);
        let expected = match failed {
            3 => vec![SavingNews(ServerSaving {
                failing_since: Some(began),
            })],
            _ => Vec::new(),
        };
        assert_eq!(news, expected, "after {failed} in a row");
    }
    village.connect(2);
    village.tick();
    assert_eq!(
        heard::<SavingNews>(&mut village, &mut told).len(),
        1,
        "whoever connects meanwhile is told as well"
    );

    saves.refuse_characters(false);
    let report = checkpoint(&mut [&mut village], &mut above);
    assert_eq!(report.outcome, Ok(2), "the account and her character");
    assert_eq!(above.checkpoints.waiting(), 0);
    village.tick();
    assert_eq!(
        heard::<SavingNews>(&mut village, &mut told),
        vec![SavingNews(ServerSaving::default()); 2],
        "both clients are told the game saves again"
    );
    assert_eq!(saves.all(Account::KIND).len(), 1);

    let report = checkpoint(&mut [&mut village], &mut above);
    assert_eq!(
        report.outcome,
        Ok(0),
        "a character is written when it changed"
    );
    village.world.get_mut::<Xp>(wren).expect("xp").gain(5);
    let report = checkpoint(&mut [&mut village], &mut above);
    assert_eq!(report.outcome, Ok(1));
    let held = saves.character("wren").expect("saved");
    let saved = load::load(&content, &held).expect("a character this build saved");
    assert_eq!(saved.saved.character.xp, 5);
}

#[test]
fn a_character_that_left_is_read_from_memory_until_the_store_has_it() {
    let saves = Saves::in_a_file();
    let content = game(Some(&Map::open(9, 9))).load();
    let mut village = serving(&content, "village");
    let mut above = saves.above(&content);
    let wren = village.sign_in(1, "wren", None);
    let staged = JoinAs {
        snapshot: CharacterSnapshot::of_player(&content, "Stand-in", player::def(&content)),
        start: content.built::<GameSettings>().initial_spawn,
        origin: JoinOrigin::Staged,
    };
    village.sign_in(2, "hale", Some(staged));
    village.world.get_mut::<Xp>(wren).expect("xp").gain(40);
    confer(&mut [&mut village], &mut above);

    village.disconnect(1);
    village.disconnect(2);
    let gone = confer(&mut [&mut village], &mut above);
    let mut clients: Vec<ClientId> = gone.iter().map(|departure| departure.client).collect();
    clients.sort();
    assert_eq!(clients, [ClientId(1), ClientId(2)]);
    let account = AccountId("wren".to_owned());
    let kept = above.checkpoints.departed(&account);
    assert_eq!(
        kept.map(|kept| kept.character.xp),
        Some(40),
        "memory holds her as she left"
    );
    assert_eq!(saves.character("wren"), None, "before the store does");
    let stand_in = AccountId("hale".to_owned());
    assert!(above.checkpoints.departed(&stand_in).is_none());

    let report = checkpoint(&mut [&mut village], &mut above);
    assert_eq!(report.outcome, Ok(1), "her, and no stand-in");
    assert!(
        above.checkpoints.departed(&account).is_none(),
        "once written, the store is where she is read from"
    );
    let held = saves.character("wren").expect("saved");
    let saved = load::load(&content, &held).expect("a character this build saved");
    assert_eq!(saved.saved.character.xp, 40);
}

#[test]
fn a_character_comes_back_whole_to_a_game_that_changed_and_is_told_once_what_became_of_it() {
    let before = game(Some(&Map::open(9, 9))).load();
    let forest = id::<AreaDef>(&before, "forest");
    let stood = Place {
        area: forest,
        at: Pos::new(6.0, 4.0),
    };
    let token = ItemStack::new(id::<ItemDef>(&before, "token"), 3);
    let plank = |count| ItemStack::new(id::<ItemDef>(&before, "plank"), count);
    let wren = SavedCharacter {
        account: AccountId("wren".to_owned()),
        saved_at: UnixMillis(1_000_000_000),
        character: CharacterSnapshot {
            skills: vec![SnapshotSkill {
                node: NodeKey("guard".to_owned()),
                rank: Rank::FIRST,
            }],
            bag: vec![token, plank(50), plank(50), plank(8)].into(),
            quests: vec![SnapshotQuest {
                progress: vec![0],
                ..SnapshotQuest::of(id::<QuestDef>(&before, "errand"), QuestState::Active)
            }]
            .into(),
            start: CharacterStart::At(stood),
            ..CharacterSnapshot::of_player(&before, "Wren", player::def(&before))
        },
        told: Vec::new(),
    };
    let saved = RecordRow::of(&wren, &ContentScope::of(&before)).body;

    let walled = Map::of(&[
        ".........",
        ".........",
        ".........",
        ".....###.",
        ".....###.",
        ".....###.",
        ".........",
        ".........",
        ".........",
    ]);
    let changed = game(Some(&walled))
        .remove("errand.quest.json")
        .remove("token.item.json")
        .edit_settings(|settings| settings["slots"]["bag"] = json!(2))
        .edit("class", "hero", |hero| hero["tree"] = json!({}))
        .load();
    let loaded = load::load(&changed, &saved).expect("she still loads");
    let changes = vec![
        GameChange::Gone {
            table: "item".to_owned(),
            name: "token".to_owned(),
        },
        GameChange::Gone {
            table: "quest".to_owned(),
            name: "errand".to_owned(),
        },
        GameChange::DidNotFit {
            slots: 2,
            stacks: 1,
        },
        GameChange::SkillsReset {
            class: "Hero".to_owned(),
            points: 2,
        },
    ];
    assert_eq!(loaded.changes, changes);
    assert_eq!(loaded.start.area, id::<AreaDef>(&changed, "forest"));
    assert!(
        area::check_place(&changed, loaded.start).is_ok()
            && loaded.start.at.distance_to(stood.at) <= 3.0,
        "she starts on the nearest ground to stand on, not at {:?}",
        loaded.start.at
    );

    let mut woods = serving(&changed, "forest");
    let back = woods.sign_in(1, "wren", Some(joining(loaded)));
    let conn = woods.conn(1);
    woods.world.entity_mut(conn).insert(Returned { changes });
    let mut told = MessageCursor::default();
    woods.tick();
    let kept = "Kept aside for you, in case it returns.";
    let line = |what: &str, became: &str| ChangeLine {
        what: what.to_owned(),
        became: became.to_owned(),
    };
    let lines = vec![
        line("The item token is no longer in the game.", kept),
        line("The quest errand is no longer in the game.", kept),
        line("Your inventory now holds 2. One stack did not fit.", kept),
        line(
            "The Hero's skills were rearranged.",
            "Your 2 skill points are yours to spend again.",
        ),
    ];
    assert_eq!(
        heard::<GameChanged>(&mut woods, &mut told),
        vec![GameChanged {
            lines: lines.clone()
        }],
        "told whole"
    );
    woods.tick();
    assert_eq!(heard::<GameChanged>(&mut woods, &mut told), [], "and once");
    let recorded: Vec<String> = woods
        .world
        .get::<History>(back)
        .expect("a history")
        .records()
        .map(|record| record.entry.text.plain_words())
        .collect();
    let written: Vec<String> = lines
        .iter()
        .map(|line| format!("{} {}", line.what, line.became))
        .collect();
    assert_eq!(recorded, written, "and it is in her history afterwards");
    let bag = &woods.world.get::<Inventory>(back).expect("a bag").slots;
    let planks = id::<ItemDef>(&changed, "plank");
    assert_eq!(
        bag,
        &vec![ItemStack::new(planks, 50), ItemStack::new(planks, 50)]
    );
    assert!(skill::known(&woods.world, back).is_empty());

    let again = SavedCharacter::of_live(&woods.world, back).expect("her own character");
    let saved = RecordRow::of(&again, &ContentScope::of(&changed)).body;
    let reread = load::load(&changed, &saved).expect("she still loads");
    assert_eq!(
        reread.changes,
        [],
        "what she was told she is not told again"
    );

    let restored = load::load(&before, &saved).expect("she loads under the game as it was");
    assert_eq!(restored.changes, []);
    let mut carried = restored.saved.character.bag.to_vec();
    carried.sort_by_key(|stack| stack.count);
    assert_eq!(
        carried,
        [token, plank(8), plank(50), plank(50)],
        "what was kept aside is hers again once the game has it"
    );
    let quests: Vec<_> = restored
        .saved
        .character
        .quests
        .iter()
        .map(|quest| quest.quest)
        .collect();
    assert_eq!(quests, [id::<QuestDef>(&before, "errand")]);

    let without_forest = game(None).load();
    let moved = load::load(&without_forest, &saved).expect("she still loads");
    assert_eq!(
        moved.start,
        without_forest.built::<GameSettings>().initial_spawn,
        "a character whose area is gone starts where the game does"
    );
}

#[test]
fn a_character_stays_in_the_world_a_while_after_its_player_is_gone() {
    let content = game(Some(&Map::open(9, 9)))
        .edit_settings(|settings| settings["lingers_for"] = json!(30.0))
        .load();
    let mut village = serving(&content, "village");
    let mut above = AboveWorlds::unsaved();
    let wren = village.sign_in(1, "wren", None);
    movement::goto(&mut village.world, wren, Pos::new(8.0, 8.0));
    village.run(0.2);

    village.disconnect(1);
    let world = &mut village.world;
    assert!(world.get::<Away>(wren).is_some() && !player::attended(world, wren));
    assert!(!movement::moving(world, wren), "its walk stops");
    let label = plate::label_of(world, &content, wren).expect("a plate");
    assert_eq!(label.state.as_deref(), Some("Away"), "others see it is");
    let health = |world: &World| stat::effective(world, wren, StatKind::Health);
    let whole = health(world);
    combat::inflict(world, wren, None, 4.0);
    assert_eq!(health(world), whole - 4.0, "it is hurt as it was before");
    let clock = *world.resource::<WallClock>();
    let later = WallClock {
        now: clock.now.after(Seconds(10.0)),
        reset: clock.reset,
    };
    let gone = worlds::tick(&mut [Some(&mut *world)], later, &mut above);
    assert!(gone.is_empty());
    assert_eq!(
        world.resource::<WallClock>().now,
        later.now,
        "its world goes on with nobody connected to it"
    );

    let back = village.sign_in(1, "wren", None);
    assert_eq!(back, wren, "its player takes it back as it stands");
    assert!(village.world.get::<Away>(wren).is_none());
    assert_eq!(health(&village.world), whole - 4.0);

    village.disconnect(1);
    village.later(29.0);
    assert!(
        village
            .world
            .resource::<Players>()
            .0
            .contains_key(&ClientId(1))
    );
    village.later(2.0);
    assert!(
        village.world.resource::<Players>().0.is_empty(),
        "when the time is up it leaves the world"
    );
    let departures = &village.world.resource::<Departures>().0;
    let left: Vec<_> = departures
        .iter()
        .filter_map(|departure| departure.left.as_ref())
        .map(|left| {
            (
                left.account.0.as_str(),
                left.character.stats.get(StatKind::Health),
            )
        })
        .collect();
    assert_eq!(left, [("wren", whole - 4.0)], "as it then is");
}

/// What `sim`'s clients were sent of `M` since `cursor` last read.
fn heard<M: Message + Clone>(sim: &mut Sim, cursor: &mut MessageCursor<ToClient<M>>) -> Vec<M> {
    sim.read(cursor)
        .into_iter()
        .map(|sent| sent.message)
        .collect()
}

fn identity(account: &str) -> AccountIdentity {
    AccountIdentity {
        id: AccountId(account.to_owned()),
        name: account.to_owned(),
        roles: Vec::new(),
    }
}

/// The saved character a sign-in read, as the connection it joins with.
fn joining(loaded: LoadedCharacter) -> JoinAs {
    JoinAs {
        snapshot: loaded.saved.character,
        start: loaded.start,
        origin: JoinOrigin::Saved {
            at: loaded.saved.saved_at,
            told: loaded.saved.told,
        },
    }
}

/// Everything `character` is, as it would cross to another world.
fn state(sim: &Sim, character: Entity) -> CharacterState {
    let rises = sim
        .world
        .get::<SpawnPoint>(character)
        .expect("a spawn point");
    CharacterState::of(&sim.world, character, rises.0).expect("a character")
}

fn id<Row: engine::core::content::ContentFile>(
    content: &Content,
    name: &str,
) -> engine::core::content::ContentId<Row> {
    content
        .by_path::<Row>(&testkit::path(Row::TABLE, name))
        .unwrap_or_else(|| panic!("the test game has no {} `{name}`", Row::TABLE))
}

/// A world of the game `content` holds serving `area`, with mortal players, where characters
/// linger and players are told of saving as on a server.
fn serving(content: &Content, area: &str) -> Sim {
    let mut sim = Sim::new(content.clone());
    sim.serve(area)
        .message::<ToClient<SavingNews>>()
        .message::<ToClient<GameChanged>>()
        .message::<ToClient<HistoryNews>>()
        .add((notice::tell_returned, notice::tell_saving, player::linger));
    sim.world.insert_resource(WallClock {
        now: UnixMillis(1_000_000_000),
        reset: UtcHour::MIDNIGHT,
    });
    sim.world.insert_resource(SpawnRules {
        policy: SpawnPolicy::Map,
        immortal: false,
    });
    sim
}

/// A village to start in and, with `forest`, a forest laid out so; two things to carry, an
/// errand that asks for one of them, and a class with a skill to learn.
fn game(forest: Option<&Map>) -> Game {
    let stacked =
        json!({ "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 50 }] });
    let game = Game::new()
        .area("village", &Map::open(9, 9), json!([]))
        .hero("village", 4.0, 4.0)
        .vfx("damage_taken")
        .shader("damage_pop")
        .item("token", stacked.clone())
        .item("plank", stacked)
        .quest(
            "errand",
            json!({
                "title": "An errand",
                "category": "Errands",
                "blurb": "Bring five tokens.",
                "xp": 120,
                "modules": [
                    { "type": "hold", "item": "token.item.json", "count": 5 },
                    { "type": "hand_over", "text": ["Done."] },
                ],
            }),
        )
        .edit("class", "hero", |hero| {
            hero["tree"] = json!({
                "guard": { "skill": "guard.skill.json", "at": { "x": 0.0, "y": 0.0 } },
            });
            hero["levels"] = json!([{ "xp": 0, "skill_points": 2 }]);
        })
        .row(
            "skill",
            "guard",
            json!({
                "name": "Guard",
                "icon": "icons/hero.png",
                "target": "self",
                "cooldown": 600.0,
                "levels": [{ "cost": 1 }],
                "modules": [{ "type": "script", "script": "mends.script.json" }],
            }),
        )
        .script(
            "mends",
            "skill",
            json!({
                "used": { "type": "on", "event": "used", "next": "mend" },
                "mend": { "type": "heal", "who": { "type": "skill_targets" }, "by": 1.0 },
            }),
        );
    match forest {
        Some(map) => game.area("forest", map, json!([])),
        None => game,
    }
}
