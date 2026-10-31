use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::ConnectedClient;
use bevy_replicon::shared::backend::channels::{ClientChannel, ServerChannel};
use bevy_replicon::shared::backend::server_messages::ServerMessages;
use engine::core::content::{Content, FilesystemFolder};
use engine::core::net::MAX_MESSAGE_BYTES;
use engine::core::time::{Seconds, UnixMillis, UtcHour, WallClock};
use engine::systems::area::{self, Cell};
use engine::systems::launch::ServerDaylight;
use engine::systems::player::{ClientId, JoinRequest, SpawnPolicy, SpawnRules};
use engine::systems::worlds::{self, Worlds};
use engine::systems::{GameSettings, TICK_HZ, from_client, load_content, server_app, start_server};
use strum::AsRefStr;

use super::play;

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

// Runs play out alike, so they compare.
pub const SEED: u64 = 1931;

/// How the area's players are positioned, which sets how much their areas-of-interest overlap. Maps
/// directly onto the server's [`SpawnPolicy`], so the bench spreads players exactly as production does.
#[derive(Clone, Copy, PartialEq, AsRefStr)]
#[strum(serialize_all = "lowercase")]
pub enum Layout {
    /// Every player spawns on the spawn tile, so all clients share one identical area-of-interest —
    /// the worst case for replicating nearby entities to many overlapping viewers.
    Congested,
    /// Players spawn across the area's walkable cells, so clients have realistically distinct
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
    pub worlds: Vec<App>,
    cells: Worlds,
    rosters: Vec<Vec<Entity>>,
    content: Content,
    ticks: usize,
}

impl Bench {
    pub fn new(areas: usize, layout: Layout, content: &Content) -> Bench {
        Bench::with_players(areas, PLAYERS_PER_AREA, layout, content)
    }

    /// `areas` of the spawn area, each with `players` playing in it.
    pub fn with_players(areas: usize, players: usize, layout: Layout, content: &Content) -> Bench {
        let spawn = content.built::<GameSettings>().initial_spawn;
        let cells = area::load(content, spawn.area).cells;
        let (worlds, rosters) = Worlds::copies(content, spawn.area, areas)
            .iter()
            .enumerate()
            .map(|(ordinal, (_, cell))| {
                let copy = ordinal / cells.count();
                // Crowded players all join the spawn's cell; spread ones share the cells out.
                let joining = match layout {
                    Layout::Congested if cells.at(spawn.at) == cell => players,
                    Layout::Congested => 0,
                    Layout::Distributed => {
                        let share = players / cells.count();
                        share
                            + usize::from(cell.0 as usize == copy % cells.count())
                                * (players - share * cells.count())
                    }
                };
                build_world(layout, content, cell, ordinal as u64, joining)
            })
            .unzip();
        Bench {
            worlds,
            cells: Worlds::copies(content, spawn.area, areas),
            rosters,
            content: content.clone(),
            ticks: 0,
        }
    }

    /// One tick of every world, as the server steps them.
    pub fn tick(&mut self, clock: WallClock) {
        let mut views: Vec<&mut World> = self.worlds.iter_mut().map(App::world_mut).collect();
        worlds::tick(&self.cells, &mut views, clock);
    }

    /// One tick on `pool`'s threads alone.
    pub fn tick_on(&mut self, pool: &rayon::ThreadPool, clock: WallClock) {
        let mut views: Vec<&mut World> = self.worlds.iter_mut().map(App::world_mut).collect();
        pool.install(|| worlds::tick(&self.cells, &mut views, clock));
    }

    pub fn connect(&mut self) {
        for (app, roster) in self.worlds.iter_mut().zip(&self.rosters) {
            let world = app.world_mut();
            for &conn in roster {
                world.entity_mut(conn).insert(ConnectedClient {
                    max_size: MAX_MESSAGE_BYTES,
                });
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

    /// What the wire and the clients do with the server's outgoing messages: they go out, and
    /// every mutate message comes back acknowledged, as a client acknowledges each one it
    /// receives. Left unacknowledged, the server would resend every changed component each
    /// replication tick and keep the bookkeeping for all of them, a load no connected client puts
    /// on it.
    pub fn acknowledge_sent(&mut self) {
        for app in &mut self.worlds {
            let mut messages = app.world_mut().resource_mut::<ServerMessages>();
            let mut acks: HashMap<Entity, Vec<u8>> = HashMap::new();
            for (client, channel, message) in messages.drain_sent() {
                if channel == usize::from(ServerChannel::Mutations) {
                    acks.entry(client)
                        .or_default()
                        .extend_from_slice(&message[MUTATE_INDEX]);
                }
            }
            for (client, ack) in acks {
                messages.insert_received(client, ClientChannel::MutationAcks, ack);
            }
        }
    }
}

pub fn load(content_dir: &Path) -> Content {
    load_content(
        Arc::new(FilesystemFolder(content_dir.to_path_buf())),
        "the program stopped",
    )
    .unwrap_or_else(|report| {
        eprint!("{report}");
        std::process::exit(2);
    })
}

/// Where a mutate message carries its index, which is what an acknowledgement quotes: right after
/// the flags byte.
const MUTATE_INDEX: std::ops::Range<usize> = 1..3;

fn build_world(
    layout: Layout,
    content: &Content,
    cell: Cell,
    ordinal: u64,
    players: usize,
) -> (App, Vec<Entity>) {
    let mut app = server_app(
        content.built::<GameSettings>().initial_spawn.area,
        cell,
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

    let zone = content.built::<GameSettings>().initial_spawn.area;
    let world = app.world_mut();
    let mut roster = Vec::with_capacity(players);
    for index in 0..players {
        let conn = world.spawn(ClientId(index as u32 + 1)).id();
        play::join_as(world, content, zone, conn, index);
        from_client(world, conn, JoinRequest);
        roster.push(conn);
    }
    app.update(); // the real `player::join` system spawns each player's character.
    (app, roster)
}
