use bevy_ecs::prelude::*;
use engine::core::content::Content;
use engine::core::math::Pos;
use engine::core::time::WallClock;
use engine::core::units::{MeterPos, Meters, Seconds};
use engine::systems::area::{self, AreaDef, AreaId, transition};
use engine::systems::area_instance::{
    self, AreaInstanceGiven, AreaInstanceShown, AreaInstanceWorld, LeaveInstance,
};
use engine::systems::inbox::InboxView;
use engine::systems::item::{self, Inventory, ItemDef, ItemStack, UseItemRequest};
use engine::systems::movement::position;
use engine::systems::npc::{self, NpcDef};
use engine::systems::party::PartyRequest;
use engine::systems::player::snapshot::{CharacterStart, SavedCharacter};
use engine::systems::player::{self, AreaDrawn, ClientId, Players, SpawnPoint, Xp};
use engine::systems::script::{self, Place};
use engine::systems::social_request::RequestAnswer;
use engine::systems::society::SocietyView;
use engine::systems::{combat, request_sources, stat};
use serde_json::{Value, json};
use testkit::{Game, Map, Sim, path};

use crate::saves::confer;
use crate::server::{Server, serving};

const ENTRANCE: Pos<Meters> = Pos::new(3.0, 3.0);
const BY_THE_DOOR: Pos<Meters> = Pos::new(9.0, 6.0);

#[test]
fn an_instance_is_its_partys_own_and_lets_in_who_it_has_room_for() {
    let mut host = Host::of(game(2), 2);
    for (client, name) in [
        (1, "Wren"),
        (2, "Hale"),
        (3, "Marrow"),
        (4, "Osric"),
        (5, "Tamsin"),
    ] {
        host.sign_in(client, name);
    }
    party(&mut host, 1, &[(2, "Hale"), (3, "Marrow")]);

    uses(&mut host, 1, "vault_key");
    let theirs = host
        .run_of(1)
        .expect("the door opens an instance for her party");
    let village = host.server.village.id::<AreaDef>("village");
    assert_eq!(
        theirs.return_to,
        Place {
            area: village,
            at: BY_THE_DOOR
        }
    );
    assert!(
        host.at(1).distance(ENTRANCE) < Meters(1.5),
        "she appears where the door leads"
    );
    let plate = host.shown(1).expect("a plate inside");
    assert_eq!(
        (goals(&plate), plate.inside, plate.holds),
        (
            vec![
                ("Defeat the warden".to_owned(), 0, 2),
                ("Seal the gate".to_owned(), 0, 1)
            ],
            1,
            2
        ),
        "the area says what it asks of those inside"
    );

    uses(&mut host, 2, "token");
    assert_eq!(
        host.run_of(2),
        Some(theirs),
        "another way into the area leads a member to the party's own, as it was opened"
    );
    uses(&mut host, 3, "lantern");
    assert_eq!(
        host.count(3, "pebble"),
        1,
        "a script reads how many of the party are inside"
    );
    uses(&mut host, 3, "vault_key");
    assert_eq!(
        (host.run_of(3), host.errors(3)),
        (None, vec!["vault holds no more than 2".to_owned()]),
        "a member who finds it full is refused, with the number"
    );
    let marrow = host.sim(3).player(3);
    let below = Place {
        area: host.server.village.id::<AreaDef>("vault"),
        at: ENTRANCE,
    };
    assert!(
        area::warp::warp(&mut host.sim(3).world, marrow, below).is_err(),
        "nothing else leads into an instanced area"
    );

    uses(&mut host, 4, "vault_key");
    let his = host
        .run_of(4)
        .expect("a player in no party gets one of their own");
    assert_ne!(his.id, theirs.id);
    let inside = |host: &mut Host, client| host.shown(client).map(|plate| plate.inside);
    assert_eq!(
        (inside(&mut host, 1), inside(&mut host, 4)),
        (Some(2), Some(1)),
        "nothing of one is seen in the other"
    );
    uses(&mut host, 4, "compass");
    uses(&mut host, 3, "compass");
    assert_eq!(
        (host.count(4, "pebble"), host.count(3, "pebble")),
        (1, 1),
        "a script reads whether someone is in an instance"
    );

    uses(&mut host, 5, "vault_key");
    assert_eq!(
        (host.run_of(5), host.errors(5)),
        (None, vec!["There is no room right now".to_owned()]),
        "the server runs only as many as it is set to"
    );
    uses(&mut host, 5, "token");
    assert_eq!(
        (host.count(5, "pebble"), host.errors(5).len()),
        (1, 1),
        "a door that has its own say of a refusal says it alone"
    );
}

#[test]
fn clearing_an_instance_rewards_everyone_who_was_there_once() {
    let mut host = Host::of(game(5), 2);
    let names = ["Wren", "Hale", "Isolde", "Tamsin", "Corvin"];
    for (client, name) in (1..).zip(names) {
        host.sign_in(client, name);
    }
    party(
        &mut host,
        1,
        &[(2, "Hale"), (3, "Isolde"), (4, "Tamsin"), (5, "Corvin")],
    );
    for client in 1..=5 {
        uses(&mut host, client, "vault_key");
    }
    let content = host.content.clone();
    let stack = |name: &str, count: u32| {
        let item = content
            .by_path::<ItemDef>(&path("item", name))
            .expect("an item of the test game");
        ItemStack::new(item, count)
    };
    let spoils = [
        stack("potion", 2),
        stack("gold", 30),
        stack("warden_key", 1),
    ];

    let isolde = host.sim(3).player(3);
    host.sim(3).disconnect(3);
    host.step();
    let saved = SavedCharacter::of_live(&host.sim(3).world, isolde).expect("her character");
    let village = host.server.village.id::<AreaDef>("village");
    assert_eq!(
        saved.character.start,
        CharacterStart::At(Place {
            area: village,
            at: BY_THE_DOOR
        }),
        "a character saved inside is saved where leaving leads"
    );
    host.later(31.0);
    host.send(4, LeaveInstance);
    run(&mut host, 0.5);
    assert_eq!(host.run_of(4), None);
    assert!(
        host.at(4).distance(BY_THE_DOOR) < Meters(1.5),
        "leaving leads back to the door"
    );

    host.carry(
        1,
        &[stack("gold", 5), stack("plank", 50), stack("plank", 50)],
    );
    let wren = host.sim(1).player(1);
    for fallen in 1..=2 {
        let warden = warden(&mut host, 1);
        combat::inflict(&mut host.sim(1).world, warden, Some(wren), 1_000_000.0);
        run(&mut host, 0.5);
        let plate = host.shown(2).expect("a plate inside");
        assert_eq!(
            goals(&plate)[0],
            ("Defeat the warden".to_owned(), fallen, 2),
            "a goal is counted for everyone inside, whoever struck the blow"
        );
    }
    let corvin = host.sim(5).player(5);
    combat::inflict(&mut host.sim(5).world, corvin, None, 1_000_000.0);
    run(&mut host, 0.5);
    assert!(stat::is_dead(&host.sim(5).world, corvin));
    assert_eq!(host.xp(1), 0, "nothing is given before every goal is met");

    uses(&mut host, 2, "seal");
    assert_eq!(
        [1, 2, 5, 4].map(|client| (host.bag(client), host.xp(client))),
        [
            (
                vec![
                    stack("gold", 35),
                    stack("plank", 50),
                    stack("plank", 50),
                    stack("potion", 2)
                ],
                50
            ),
            (spoils.to_vec(), 50),
            (spoils.to_vec(), 50),
            (Vec::new(), 0),
        ],
        "each stack goes where it fits, to whoever was there, standing or fallen"
    );
    let sent = |host: &mut Host, client| -> Vec<(String, bool, Vec<ItemStack>)> {
        host.inbox(client)
            .rows
            .into_iter()
            .map(|row| (row.title, row.from.is_none(), row.attached))
            .collect()
    };
    assert_eq!(
        (sent(&mut host, 1), sent(&mut host, 2)),
        (
            vec![(
                "Your rewards from vault".to_owned(),
                true,
                vec![stack("warden_key", 1)]
            )],
            Vec::new()
        ),
        "only what a bag had no room for is sent, by the game"
    );
    let cleared = host.shown(1).and_then(|plate| plate.cleared);
    assert_eq!(
        cleared.map(|cleared| cleared.given),
        Some(AreaInstanceGiven {
            xp: 50,
            kept: vec![stack("potion", 2), stack("gold", 30)],
            sent: vec![stack("warden_key", 1)],
        }),
        "each participant is shown what clearing it gave them"
    );

    uses(&mut host, 2, "seal");
    let warden = warden(&mut host, 1);
    combat::inflict(&mut host.sim(1).world, warden, Some(wren), 1_000_000.0);
    run(&mut host, 0.5);
    assert_eq!(
        (host.bag(2), host.xp(2), sent(&mut host, 1).len()),
        (spoils.to_vec(), 50, 1),
        "an instance is cleared once"
    );

    host.later(31.0);
    run(&mut host, 0.5);
    assert_eq!(
        [1, 2, 5].map(|client| (
            host.run_of(client),
            host.at(client).distance(BY_THE_DOOR) < Meters(2.5)
        )),
        [(None, true); 3],
        "a cleared instance closes, and everyone inside is returned"
    );
    assert_eq!(host.instances(), 0);

    host.sign_in(3, "Isolde");
    run(&mut host, 0.5);
    assert_eq!(
        sent(&mut host, 3),
        [("Your rewards from vault".to_owned(), true, spoils.to_vec())],
        "whoever lost their player inside and is still of the party gets everything sent"
    );
}

#[test]
fn an_instance_lives_while_it_is_someones_to_come_back_to() {
    let mut host = Host::of(game(5), 2);
    for (client, name) in [(1, "Wren"), (2, "Hale"), (3, "Marrow"), (4, "Osric")] {
        host.sign_in(client, name);
    }
    party(&mut host, 1, &[(2, "Hale"), (3, "Marrow")]);
    uses(&mut host, 1, "vault_key");
    uses(&mut host, 2, "vault_key");
    let first = host.run_of(1).expect("an instance");
    let wren = host.sim(1).player(1);
    assert_eq!(
        host.sim(1)
            .world
            .get::<SpawnPoint>(wren)
            .map(|point| point.0),
        Some(ENTRANCE),
        "whoever falls inside rises at the entrance"
    );

    host.send(2, PartyRequest::Leave);
    run(&mut host, 0.5);
    let now = host.now();
    let out_at = host.shown(2).and_then(|plate| plate.sent_out);
    assert_eq!(
        (
            host.told(2).last().cloned(),
            out_at.is_some_and(|at| at > now && at <= now.after(Seconds(30.0))),
            host.run_of(2)
        ),
        (
            Some("You are no longer of this party".to_owned()),
            true,
            Some(first)
        ),
        "whoever stops being of the party is told why, and has the area's time to go"
    );
    host.later(31.0);
    run(&mut host, 0.5);
    assert_eq!(host.run_of(2), None);
    assert!(host.at(2).distance(BY_THE_DOOR) < Meters(1.5));
    uses(&mut host, 2, "vault_key");
    assert_ne!(
        host.run_of(2).map(|run| run.id),
        Some(first.id),
        "there is no way into a party's instance but to be of the party"
    );
    host.send(2, LeaveInstance);
    run(&mut host, 0.5);

    let warden = warden(&mut host, 1);
    combat::inflict(&mut host.sim(1).world, warden, Some(wren), 1_000_000.0);
    run(&mut host, 0.5);
    uses(&mut host, 1, "rope");
    assert_eq!(host.run_of(1), None, "a script leads out as the plate does");
    uses(&mut host, 1, "vault_key");
    let plate = host.shown(1).expect("a plate inside");
    assert_eq!(
        (host.run_of(1), goals(&plate)[0].1),
        (Some(first), 1),
        "leaving does not close the door: a member goes back in to what was done"
    );
    assert!(host.at(1).distance(ENTRANCE) < Meters(1.5));

    uses(&mut host, 1, "rope");
    host.later(301.0);
    uses(&mut host, 1, "vault_key");
    let second = host.run_of(1).expect("an instance");
    let plate = host.shown(1).expect("a plate inside");
    assert_eq!(
        (second.id == first.id, goals(&plate)[0].1),
        (false, 0),
        "one nobody came back to in the area's time is gone, and the door opens another"
    );

    host.send(1, PartyRequest::Disband);
    run(&mut host, 0.5);
    assert_eq!(
        host.told(1).last().map(String::as_str),
        Some("You are no longer of this party")
    );
    host.later(31.0);
    run(&mut host, 0.5);
    assert_eq!(
        (host.run_of(1), host.instances()),
        (None, 0),
        "a disbanded party's instance sends everyone out, and is gone"
    );

    uses(&mut host, 4, "trial_key");
    assert!(host.run_of(4).is_some());
    host.later(601.0);
    run(&mut host, 0.5);
    assert_eq!(
        (
            host.told(4).last().map(String::as_str),
            host.run_of(4),
            host.xp(4),
            host.instances()
        ),
        (Some("Time ran out in trial"), None, 0, 0),
        "an instance that only lasts so long returns everyone, with no rewards"
    );
}

#[test]
fn a_game_whose_instances_cannot_be_played_is_refused() {
    let rules = |game: Game| -> Vec<&'static str> {
        let report = game.try_load().err().expect("refused");
        report
            .errors
            .iter()
            .map(|error| error.problem.rule)
            .collect()
    };
    let door = |change: &dyn Fn(&mut Value)| {
        game(2).edit("script", "vault_door", |script| {
            change(&mut script["nodes"]["enter"])
        })
    };
    let place = |area: &str| json!({ "area": path("area", area), "x": 3.0, "y": 3.0 });

    assert_eq!(
        rules(door(
            &|enter| *enter = json!({ "type": "warp", "to": place("vault") })
        )),
        ["warps into an instance"]
    );
    assert_eq!(
        rules(door(&|enter| enter["to"] = place("forest"))),
        ["not an instanced area"]
    );
    assert_eq!(
        rules(door(&|enter| enter["return_to"] = place("trial"))),
        ["returns into an instance"]
    );
    assert_eq!(
        rules(game(2).edit_settings(|settings| {
            settings["initial_spawn"]["area"] = json!(path("area", "vault"))
        })),
        ["starts in an instance"]
    );
    let scout = json!({
        "name": "Scout",
        "class": path("class", "hero"),
        "appearance": { "model": path("model", "hero") },
        "start": place("vault"),
    });
    assert_eq!(
        rules(game(2).row("test_character", "scout", scout)),
        ["starts in an instance"]
    );
    let mut unsent = rules(game(2).edit_settings(|settings| {
        settings
            .as_object_mut()
            .expect("settings")
            .remove("messages");
    }));
    unsent.dedup();
    assert_eq!(
        unsent,
        ["pays items without messages"],
        "what a bag has no room for needs somewhere to go"
    );
    assert_eq!(
        rules(game(2).edit("area", "vault", |vault| {
            vault["instance"]["goals"][0] = vault["instance"]["goals"][1].clone()
        })),
        ["task named twice"]
    );
}

/// A server whose instances each get a world of their own as someone is let in, the way a
/// game server makes them.
struct Host {
    server: Server,
    runs: Vec<Sim>,
    content: Content,
    /// What the feed told each player in the worlds they have left, by their client.
    told_before: Vec<(u32, String)>,
}

impl Host {
    /// A server of `game` that runs at most `instances` instances at once.
    fn of(game: Game, instances: u32) -> Host {
        let mut server = Server::hosting(game, request_sources(), instances);
        plays(&mut server.village);
        plays(&mut server.forest);
        let content = server.village.world.resource::<Content>().clone();
        Host {
            server,
            runs: Vec::new(),
            content,
            told_before: Vec::new(),
        }
    }

    fn sims(&mut self) -> Vec<&mut Sim> {
        [&mut self.server.village, &mut self.server.forest]
            .into_iter()
            .chain(&mut self.runs)
            .collect()
    }

    /// The worlds confer, each plays a tick, and whoever is on their way arrives.
    fn step(&mut self) {
        let Server {
            above,
            village,
            forest,
        } = &mut self.server;
        let mut sims: Vec<&mut Sim> = [village, forest]
            .into_iter()
            .chain(&mut self.runs)
            .collect();
        confer(&mut sims, above);
        for sim in sims {
            sim.tick();
        }
        self.travel();
        let society = self.server.above.society.read();
        self.runs.retain(|sim| {
            let run = sim.world.resource::<AreaInstanceWorld>().id;
            society.instances().get(run).is_some()
        });
    }

    fn travel(&mut self) {
        let mut travelers = Vec::new();
        let mut told = Vec::new();
        for sim in self.sims() {
            for traveler in transition::departing(&mut sim.world) {
                let client = traveler.played.client.0;
                let connected = connection(sim, client).is_some();
                if connected {
                    told.extend(told_in(sim, client).into_iter().map(|said| (client, said)));
                    sim.disconnect(client);
                }
                travelers.push((traveler, connected));
            }
        }
        self.told_before.extend(told);
        for (traveler, connected) in travelers {
            let client = traveler.played.client.0;
            let to = self.world_for(traveler.dest_area, traveler.instance);
            if connected {
                to.connect(client);
            }
            transition::arrive(&mut to.world, traveler);
            if connected {
                let conn = to.conn(client);
                to.send(conn, AreaDrawn);
            }
        }
    }

    /// The world of `area` everyone shares, or of its instance `run`, made now if it wasn't.
    fn world_for(&mut self, area: AreaId, run: Option<AreaInstanceWorld>) -> &mut Sim {
        let Some(run) = run else {
            let village = self.server.village.id::<AreaDef>("village");
            return if area == village {
                &mut self.server.village
            } else {
                &mut self.server.forest
            };
        };
        let made = self
            .runs
            .iter()
            .position(|sim| sim.world.resource::<AreaInstanceWorld>().id == run.id);
        let index = made.unwrap_or_else(|| {
            let key = ["vault", "trial"]
                .into_iter()
                .find(|key| self.server.village.id::<AreaDef>(key) == area)
                .expect("an instanced area of the test game");
            let mut sim = serving(&self.content, key);
            plays(&mut sim);
            sim.world.insert_resource(run);
            let now = *self.server.village.world.resource::<WallClock>();
            sim.world.insert_resource(now);
            self.runs.push(sim);
            self.runs.len() - 1
        });
        &mut self.runs[index]
    }

    fn later(&mut self, seconds: f32) {
        for sim in self.sims() {
            let mut clock = sim.world.resource_mut::<WallClock>();
            clock.now = clock.now.after(Seconds(seconds));
        }
        self.step();
        self.step();
    }

    fn now(&self) -> engine::core::time::UnixMillis {
        self.server.now()
    }

    /// The account called `name` signs in and joins the village as `client`.
    fn sign_in(&mut self, client: u32, name: &str) -> Entity {
        let character = self.server.sign_in(client, name, "village");
        self.step();
        character
    }

    /// The world the character of `client` stands in.
    fn sim(&mut self, client: u32) -> &mut Sim {
        self.sims()
            .into_iter()
            .find(|sim| {
                sim.world
                    .resource::<Players>()
                    .0
                    .contains_key(&ClientId(client))
            })
            .expect("a character in a world")
    }

    fn send<M: Message>(&mut self, client: u32, message: M) {
        let sim = self.sim(client);
        let conn = sim.conn(client);
        sim.send(conn, message);
        sim.tick();
    }

    /// The instance the character of `client` stands in.
    fn run_of(&mut self, client: u32) -> Option<AreaInstanceWorld> {
        self.sim(client)
            .world
            .get_resource::<AreaInstanceWorld>()
            .copied()
    }

    fn at(&mut self, client: u32) -> Pos<Meters> {
        let sim = self.sim(client);
        position(&sim.world, sim.player(client)).expect("a character somewhere")
    }

    /// The instance the player of `client` is shown they are inside.
    fn shown(&mut self, client: u32) -> Option<AreaInstanceShown> {
        let sim = self.sim(client);
        let character = sim.player(client);
        sim.world.get::<SocietyView>(character)?.instance.clone()
    }

    fn inbox(&mut self, client: u32) -> InboxView {
        let sim = self.sim(client);
        let character = sim.player(client);
        sim.world
            .get::<InboxView>(character)
            .cloned()
            .unwrap_or_default()
    }

    fn bag(&mut self, client: u32) -> Vec<ItemStack> {
        let sim = self.sim(client);
        let character = sim.player(client);
        sim.world
            .get::<Inventory>(character)
            .expect("a bag")
            .slots
            .clone()
    }

    /// The character of `client` carries `stacks` and nothing else.
    fn carry(&mut self, client: u32, stacks: &[ItemStack]) {
        let sim = self.sim(client);
        let character = sim.player(client);
        let mut bag = sim.world.get_mut::<Inventory>(character).expect("a bag");
        bag.slots = stacks.to_vec();
    }

    fn count(&mut self, client: u32, item: &str) -> u32 {
        let sim = self.sim(client);
        sim.count(sim.player(client), item)
    }

    fn xp(&mut self, client: u32) -> u32 {
        let sim = self.sim(client);
        let character = sim.player(client);
        sim.world.get::<Xp>(character).map_or(0, |xp| xp.amount)
    }

    fn errors(&mut self, client: u32) -> Vec<String> {
        self.sim(client).errors(client)
    }

    /// What the feed told the player so far, in every world they have been in, refusals aside.
    fn told(&mut self, client: u32) -> Vec<String> {
        let here = told_in(self.sim(client), client);
        let before = self.told_before.iter();
        before
            .filter(|(told, _)| *told == client)
            .map(|(_, said)| said.clone())
            .chain(here)
            .collect()
    }

    /// How many instances the server runs.
    fn instances(&self) -> usize {
        self.server.above.society.read().instances().iter().count()
    }
}

/// Has a world play what instances take: items used, the scripts they run, falls, and leaving.
fn plays(sim: &mut Sim) {
    sim.accept::<AreaDrawn>()
        .accept::<UseItemRequest>()
        .accept::<LeaveInstance>()
        .add((
            player::arrival_holds,
            item::use_item,
            area_instance::requests,
        ))
        .plan((
            script::query::ListRoster::default(),
            script::RunFlows::default(),
        ))
        .plan(stat::SettleVitals::default())
        .add((combat::defeats, area_instance::count_defeats).chain());
    script::spawn_stateless(&mut sim.world);
}

/// What the feed told the player of `client` in this world, refusals aside.
fn told_in(sim: &mut Sim, client: u32) -> Vec<String> {
    let errors = sim.errors(client);
    sim.told(client)
        .into_iter()
        .map(|told| told.text.plain_words())
        .filter(|said| !errors.contains(said))
        .collect()
}

fn connection(sim: &mut Sim, client: u32) -> Option<Entity> {
    let mut conns = sim.world.query::<(Entity, &ClientId)>();
    conns
        .iter(&sim.world)
        .find(|(_, id)| id.0 == client)
        .map(|(conn, _)| conn)
}

fn run(host: &mut Host, seconds: f32) {
    let ticks = (seconds * engine::systems::TICK_HZ.0).ceil() as usize;
    for _ in 0..ticks {
        host.step();
    }
}

/// The player of `leader` makes a party of themselves and every one of `members`.
fn party(host: &mut Host, leader: u32, members: &[(u32, &str)]) {
    for (client, name) in members {
        let invited = host.server.number(name);
        host.send(leader, PartyRequest::Invite(invited));
        host.step();
        host.server.answer(*client, RequestAnswer::Accept);
        host.step();
    }
}

/// The player of `client` is handed `item`, uses it, and no longer has it once the world has
/// played what that started.
fn uses(host: &mut Host, client: u32, item: &str) {
    let sim = host.sim(client);
    let character = sim.player(client);
    sim.give(character, item, 1);
    let slot = sim.slot_of(character, item);
    host.send(client, UseItemRequest { slot });
    run(host, 0.5);
    let sim = host.sim(client);
    sim.take(sim.player(client), item, 1);
}

/// A warden beside the character of `client`.
fn warden(host: &mut Host, client: u32) -> Entity {
    let at = host.at(client);
    let sim = host.sim(client);
    let (area, warden) = (sim.id::<AreaDef>("vault"), sim.id::<NpcDef>("warden"));
    npc::spawn(&mut sim.world, warden, at, area, None)
}

fn goals(plate: &AreaInstanceShown) -> Vec<(String, u32, u32)> {
    plate
        .goals
        .iter()
        .map(|goal| (goal.label.clone(), goal.done, goal.of))
        .collect()
}

/// A game of a village and a forest everyone shares and two instanced areas: a vault holding
/// `holds`, cleared by two wardens falling and its gate being sealed, and a trial that lasts ten
/// minutes. Things a player uses lead in and out and do what the scripts of instances do.
fn game(holds: u32) -> Game {
    let stacked = |max: u32| json!({ "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": max }] });
    let place =
        |area: &str, at: Pos<Meters>| json!({ "area": path("area", area), "x": at.x, "y": at.y });
    let used = |game: Game, item: &str, script: &str, nodes: Value| {
        game.item(
            item,
            json!({
                "kind": { "type": "usable" },
                "modules": [{ "type": "script", "script": path("script", script) }],
            }),
        )
        .script(script, "item", nodes)
    };
    let pebble = json!({ "type": "items", "give": [{ "item": path("item", "pebble") }] });
    let spoils = json!({
        "xp": 50,
        "items": [
            { "item": path("item", "potion"), "amount": 2 },
            { "item": path("item", "gold"), "amount": 30 },
            { "item": path("item", "warden_key") },
        ],
    });
    let instanced = |game: Game, area: &str, rules: Value| {
        game.area(area, &Map::open(12, 12), json!([]))
            .edit("area", area, |row| row["instance"] = rules)
    };
    let game = Game::new()
        .area("village", &Map::open(12, 12), json!([]))
        .area("forest", &Map::open(12, 12), json!([]))
        .hero("village", 4.0, 4.0)
        .sfx("ui_refuse")
        .interface_icon("map")
        .npc("warden", 25.0, json!([]))
        .vfx("damage_taken")
        .shader("damage_pop")
        .vfx("defeated")
        .item("potion", stacked(20))
        .item("gold", stacked(99))
        .item("plank", stacked(50))
        .item("warden_key", json!({ "kind": { "type": "resource" } }))
        .item("pebble", stacked(20))
        .edit_settings(|settings| {
            settings["lingers_for"] = json!(30.0);
            settings["slots"]["bag"] = json!(4);
            settings["party"] = json!({
                "size": 5,
                "invite_lasts": 60.0,
                "absent_for": 3600.0,
                "experience": { "split": "even", "range": 40.0, "bonus": 0.0 },
            });
            settings["messages"] = json!({
                "inbox": 2, "attachments": 6, "title_length": 60, "text_length": 1000, "a_minute": 2
            });
        });
    let game = instanced(
        game,
        "vault",
        json!({
            "holds": holds,
            "goals": [
                { "type": "defeat", "npc": path("npc", "warden"), "count": 2 },
                { "type": "task", "task": "seal", "label": "Seal the gate" },
            ],
            "empties_after": 300.0,
            "closes_after": 30.0,
        }),
    );
    let game = instanced(
        game,
        "trial",
        json!({
            "holds": 1,
            "goals": [{ "type": "task", "task": "endure", "label": "Endure" }],
            "empties_after": 300.0,
            "closes_after": 30.0,
            "lasts": 600.0,
        }),
    );
    let enter = |area: &str, rewards: Value, back: Value| {
        json!({
            "type": "enter_instance",
            "to": place(area, ENTRANCE),
            "rewards": rewards,
            "return_to": back,
        })
    };
    let on_use = |next: &str| json!({ "type": "on", "event": "used", "next": next });
    let game = used(
        game,
        "vault_key",
        "vault_door",
        json!({
            "used": on_use("enter"),
            "enter": enter("vault", spoils, place("village", BY_THE_DOOR)),
        }),
    );
    let mut by_token = enter(
        "vault",
        json!({ "xp": 5 }),
        place("forest", Pos::new(6.0, 6.0)),
    );
    by_token["refused"] = json!("sorry");
    let game = used(
        game,
        "token",
        "vault_token",
        json!({ "used": on_use("enter"), "enter": by_token, "sorry": pebble }),
    );
    let game = used(
        game,
        "trial_key",
        "trial_door",
        json!({
            "used": on_use("enter"),
            "enter": enter("trial", json!({ "xp": 50 }), place("village", BY_THE_DOOR)),
        }),
    );
    let game = used(
        game,
        "seal",
        "gate_sealed",
        json!({
            "used": on_use("seal"),
            "seal": { "type": "complete_goal", "task": "seal" },
        }),
    );
    let game = used(
        game,
        "rope",
        "way_out",
        json!({ "used": on_use("leave"), "leave": { "type": "leave_instance" } }),
    );
    let game = used(
        game,
        "lantern",
        "look_below",
        json!({
            "used": on_use("look"),
            "look": {
                "type": "if",
                "test": {
                    "type": "at_least",
                    "a": { "type": "instance_holds", "area": path("area", "vault") },
                    "b": 2,
                },
                "yes": "seen",
            },
            "seen": pebble,
        }),
    );
    used(
        game,
        "compass",
        "where_am_i",
        json!({
            "used": on_use("look"),
            "look": { "type": "if", "test": { "type": "in_instance" }, "yes": "inside" },
            "inside": pebble,
        }),
    )
}
