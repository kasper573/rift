use std::path::Path;
use std::time::{Duration, Instant};

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replication::ClientLink;
use engine::core::content::Content;
use engine::core::time::{UnixMillis, UtcHour, WallClock};
use engine::core::units::{MeterPos, Seconds};
use engine::systems::account::{AccountId, AccountIdentity};
use engine::systems::area;
use engine::systems::area::Place;
use engine::systems::launch::{ServerDay, ServerWeather};
use engine::systems::player::snapshot::CharacterSnapshot;
use engine::systems::player::{ClientId, JoinRequest, SpawnPolicy, SpawnRules};
use engine::systems::region;
use engine::systems::save::Checkpoints;
use engine::systems::society::ServerSociety;
use engine::systems::worlds::{self, AboveWorlds};
use engine::systems::{GameSettings, TICK_HZ, from_client, server_app, start_server};
use strum::AsRefStr;

use super::market::{self, MarketAsks};
use super::play;
use crate::server::SEED;

pub const PLAYERS_PER_AREA: usize = 25;

const CLOCK: WallClock = WallClock {
    now: UnixMillis(0),
    reset: UtcHour::MIDNIGHT,
};

/// The wall clock ticks with the worlds, so cooldowns and played actions run their course as they
/// do on the live server.
fn clock_after(ticks: usize) -> WallClock {
    WallClock {
        now: CLOCK.now.after(Seconds(ticks as f32 / TICK_HZ.0)),
        ..CLOCK
    }
}

/// Ticks stepped before sampling, so spawn waves and connect churn settle out of the measurement.
pub const WARMUP: usize = 30;

/// How long the ground within reach of the players may take to be held before a bench gives up.
const SETTLE: Duration = Duration::from_secs(120);

/// Steps `bench` with `step` until the ground within reach of every player is held and what
/// stands on it has come to life, as a server's first second goes, then through the warmup.
pub fn warm_up(bench: &mut Bench, mut step: impl FnMut(&mut Bench)) {
    let deadline = Instant::now() + SETTLE;
    while !bench.settled() {
        assert!(
            Instant::now() < deadline,
            "the ground within reach of the players is held within {SETTLE:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
        step(bench);
    }
    for _ in 0..WARMUP {
        step(bench);
    }
}

/// How the area's players are positioned, which sets how much their areas-of-interest overlap. Maps
/// directly onto the server's [`SpawnPolicy`], so the bench spreads players exactly as production does.
#[derive(Clone, Copy, PartialEq, AsRefStr)]
#[strum(serialize_all = "lowercase")]
pub enum Layout {
    /// Every player spawns on the spawn point, so all clients share one identical area-of-interest —
    /// the worst case for replicating nearby entities to many overlapping viewers.
    Congested,
    /// Players spawn across the area's walkable ground, so clients have realistically distinct
    /// areas-of-interest.
    Distributed,
}

impl Layout {
    fn spawn_policy(self) -> SpawnPolicy {
        match self {
            Layout::Congested => SpawnPolicy::Map,
            Layout::Distributed => SpawnPolicy::Dist,
        }
    }
}

/// Areas of the content's spawn area, each with its players, stepped as the server steps its own.
pub struct Bench {
    worlds: Vec<App>,
    rosters: Vec<Vec<Entity>>,
    above: AboveWorlds,
    content: Content,
    ticks: usize,
    asks: MarketAsks,
}

impl Bench {
    pub fn new(areas: usize, layout: Layout, content: &Content) -> Bench {
        Bench::with_players_in(areas, PLAYERS_PER_AREA, layout, content, 1.0)
    }

    /// `areas` of the spawn area, each with `players` playing in it, each as one of the test
    /// characters in turn, signed in as an account of their own. Crowded players play only the
    /// characters starting within sight of the spawn; spread ones all of them. The players are in
    /// the first `share` of each area's ground alone, a band across it from its first row, the
    /// rest left to nobody. Society starts as a server's does that has run with them a while: its
    /// market holds their listings.
    pub fn with_players_in(
        areas: usize,
        players: usize,
        layout: Layout,
        content: &Content,
        share: f32,
    ) -> Bench {
        let spawn = content.built::<GameSettings>().initial_spawn;
        let peopled = area::load(content, spawn.area).size.height * share;
        let characters: Vec<Option<(CharacterSnapshot, Place)>> = {
            let mut all = play::characters_in(content, spawn.area);
            if layout == Layout::Congested {
                let sight = content.built::<GameSettings>().sight;
                all.retain(|(_, start)| start.at.distance(spawn.at) <= sight);
            }
            all.retain(|(_, start)| start.at.y < peopled);
            if all.is_empty() {
                vec![None]
            } else {
                all.into_iter().map(Some).collect()
            }
        };
        let joining: Vec<Option<(CharacterSnapshot, Place)>> = (0..players)
            .map(|index| characters[index % characters.len()].clone())
            .collect();
        let accounts: Vec<Vec<AccountIdentity>> = (0..areas)
            .map(|ordinal| {
                (0..players)
                    .map(|index| account(client_of(ordinal, players, index)))
                    .collect()
            })
            .collect();
        let mut above = AboveWorlds {
            roster: Default::default(),
            society: ServerSociety::of(market::society(content, &accounts.concat(), CLOCK.now)),
            checkpoints: Checkpoints::none(),
        };
        for identity in accounts.iter().flatten() {
            above
                .society
                .signed_in(identity, CLOCK.now, &mut above.checkpoints.unwritten);
        }
        let (worlds, rosters) = accounts
            .into_iter()
            .enumerate()
            .map(|(ordinal, accounts)| {
                build_world(layout, content, ordinal, accounts, joining.clone())
            })
            .unzip();
        Bench {
            worlds,
            rosters,
            above,
            content: content.clone(),
            ticks: 0,
            asks: MarketAsks::default(),
        }
    }

    /// How many listings the market holds now, what its traders have asked of it, and over how
    /// many ticks.
    pub fn market(&self) -> (usize, MarketAsks, usize) {
        (
            self.above.society.read().market().len(),
            self.asks,
            self.ticks,
        )
    }

    /// Whether every world holds the ground within reach of its players, with no hold under way.
    pub fn settled(&self) -> bool {
        self.worlds.iter().all(|app| region::settled(app.world()))
    }

    /// One tick of every world, as the server steps them: on `pool`'s threads alone, or on those
    /// of the pool the bench runs in.
    pub fn tick(&mut self, pool: Option<&rayon::ThreadPool>, clock: WallClock) {
        let mut views: Vec<Option<&mut World>> = self
            .worlds
            .iter_mut()
            .map(|app| Some(app.world_mut()))
            .collect();
        let above = &mut self.above;
        match pool {
            Some(pool) => pool.install(|| worlds::tick(&mut views, clock, above)),
            None => worlds::tick(&mut views, clock, above),
        };
    }

    pub fn connect(&mut self) {
        for (app, roster) in self.worlds.iter_mut().zip(&self.rosters) {
            let world = app.world_mut();
            for &conn in roster {
                world.entity_mut(conn).insert(ClientLink::default());
            }
        }
    }

    /// Has every player whose turn it is act, for the tick the worlds are about to step, and
    /// gives the wall clock that tick runs at.
    pub fn play(&mut self) -> WallClock {
        self.ticks += 1;
        for (ordinal, (app, roster)) in self.worlds.iter_mut().zip(&self.rosters).enumerate() {
            let world = app.world_mut();
            play::act(world, &self.content, roster, self.ticks);
            for (index, &conn) in roster.iter().enumerate().step_by(market::TRADER_EVERY) {
                if let Some(player) = play::player_of(world, conn) {
                    let trader = market::Trader {
                        conn,
                        player,
                        number: client_of(ordinal, roster.len(), index).0 as usize,
                    };
                    market::trade(world, &self.content, trader, self.ticks, &mut self.asks);
                }
            }
        }
        clock_after(self.ticks)
    }

    /// What the wire does with every client's stream: it goes out, as it must, or it would pile
    /// up in memory as no connected client lets it.
    pub fn send_out(&mut self) {
        for app in &mut self.worlds {
            let world = app.world_mut();
            for mut link in world.query::<&mut ClientLink>().iter_mut(world) {
                link.take_outgoing();
            }
        }
    }
}

pub fn load(content_dir: &Path) -> Content {
    crate::folder::load(
        content_dir,
        "the program stopped",
        engine::core::content::LoadPurpose::Play,
    )
    .unwrap_or_else(|report| {
        eprint!("{report}");
        std::process::exit(2);
    })
}

fn build_world(
    layout: Layout,
    content: &Content,
    ordinal: usize,
    accounts: Vec<AccountIdentity>,
    players: Vec<Option<(CharacterSnapshot, Place)>>,
) -> (App, Vec<Entity>) {
    let mut app = server_app(
        content.built::<GameSettings>().initial_spawn.area,
        ordinal as u64,
        SEED,
        CLOCK,
        ServerDay::TURNING.course(content, CLOCK.now),
        ServerWeather::Still.course(CLOCK.now),
        content,
    );
    app.insert_resource(SpawnRules {
        policy: layout.spawn_policy(),
        immortal: false,
    });
    start_server(&mut app);

    let world = app.world_mut();
    let mut roster = Vec::with_capacity(players.len());
    for (index, (character, identity)) in players.iter().zip(accounts).enumerate() {
        let client = client_of(ordinal, players.len(), index);
        let conn = world.spawn((client, identity)).id();
        if let Some(character) = character {
            play::join_as(world, conn, character);
        }
        from_client(world, conn, JoinRequest);
        roster.push(conn);
    }
    app.update();
    (app, roster)
}

/// The server-wide id of the `index`th player of the `ordinal`th world, `players` to a world.
fn client_of(ordinal: usize, players: usize, index: usize) -> ClientId {
    ClientId((ordinal * players + index) as u32 + 1)
}

/// The account the player of `client` signs in as.
fn account(client: ClientId) -> AccountIdentity {
    AccountIdentity {
        id: AccountId(format!("bench-{}", client.0)),
        name: format!("Bench {}", client.0),
        roles: Vec::new(),
    }
}
