use std::path::Path;
use std::time::{Duration, Instant};

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replication::ClientLink;
use engine::core::content::Content;
use engine::core::time::{UnixMillis, UtcHour, WallClock};
use engine::core::units::{MeterPos, Seconds};
use engine::systems::area;
use engine::systems::launch::ServerDaylight;
use engine::systems::player::snapshot::CharacterSnapshot;
use engine::systems::player::{ClientId, JoinRequest, SpawnPolicy, SpawnRules};
use engine::systems::region;
use engine::systems::script::Place;
use engine::systems::worlds;
use engine::systems::{GameSettings, TICK_HZ, from_client, server_app, start_server};
use strum::AsRefStr;

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
    content: Content,
    ticks: usize,
}

impl Bench {
    pub fn new(areas: usize, layout: Layout, content: &Content) -> Bench {
        Bench::with_players_in(areas, PLAYERS_PER_AREA, layout, content, 1.0)
    }

    /// `areas` of the spawn area, each with `players` playing in it, each as one of the test
    /// characters in turn. Crowded players play only the characters starting within sight of the
    /// spawn; spread ones all of them. The players are in the first `share` of each area's ground
    /// alone, a band across it from its first row, the rest left to nobody.
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
        let (worlds, rosters) = (0..areas)
            .map(|ordinal| build_world(layout, content, ordinal as u64, joining.clone()))
            .unzip();
        Bench {
            worlds,
            rosters,
            content: content.clone(),
            ticks: 0,
        }
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
        match pool {
            Some(pool) => pool.install(|| worlds::tick(&mut views, clock)),
            None => worlds::tick(&mut views, clock),
        }
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
        for (app, roster) in self.worlds.iter_mut().zip(&self.rosters) {
            play::act(app.world_mut(), &self.content, roster, self.ticks);
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
    ordinal: u64,
    players: Vec<Option<(CharacterSnapshot, Place)>>,
) -> (App, Vec<Entity>) {
    let mut app = server_app(
        content.built::<GameSettings>().initial_spawn.area,
        ordinal,
        SEED,
        CLOCK,
        ServerDaylight::Turning,
        content,
    );
    app.insert_resource(SpawnRules {
        policy: layout.spawn_policy(),
        immortal: false,
    });
    start_server(&mut app);

    let world = app.world_mut();
    let mut roster = Vec::with_capacity(players.len());
    for (index, character) in players.iter().enumerate() {
        let conn = world.spawn(ClientId(index as u32 + 1)).id();
        if let Some(character) = character {
            play::join_as(world, conn, character);
        }
        from_client(world, conn, JoinRequest);
        roster.push(conn);
    }
    app.update(); // the real `player::join` system spawns each player's character.
    (app, roster)
}
