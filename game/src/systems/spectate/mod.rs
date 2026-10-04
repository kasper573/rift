pub mod widget;

use std::collections::HashMap;

use bevy_app::App;
use bevy_ecs::lifecycle::Remove;
use bevy_ecs::message::Message;
use bevy_ecs::observer::On;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{SendTargets, ToClients};
use serde::{Deserialize, Serialize};

use crate::systems::account::identity::Identity;
use crate::systems::account::role::Role;
use crate::systems::player::{ClientId, Players};

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.add_client_message::<SpectateRequest>(Channel::Ordered)
        .add_server_message::<Spectating>(Channel::Ordered);
}

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpectateRequest {
    Start,
    Next,
    Previous,
}

/// Whom a spectator watches, told whenever it changes. A spectator watches nobody only while nobody
/// plays.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spectating {
    Nobody,
    Player(Watched),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Watched {
    pub player: ClientId,
    /// Counts from 1.
    pub place: u32,
    pub online: u32,
}

#[derive(Resource, Default)]
pub struct Spectators(pub HashMap<ClientId, Spectator>);

#[derive(Clone, Copy, Debug, Default)]
pub struct Spectator {
    watching: Option<ClientId>,
    step: Option<Step>,
    told: Option<Spectating>,
}

impl Spectator {
    pub fn watching(&self) -> Option<ClientId> {
        self.watching
    }
}

pub struct SpectatorTransfer {
    pub client: ClientId,
    pub dest: usize,
    spectator: Spectator,
}

pub fn requests(world: &mut World) {
    for request in crate::systems::requests::<SpectateRequest>(world) {
        let Some(conn) = request.client_id.entity() else {
            continue;
        };
        let Some(&client) = world.get::<ClientId>(conn) else {
            continue;
        };
        let step = match request.message {
            SpectateRequest::Start => {
                if allowed(world, conn, client) {
                    world
                        .resource_mut::<Spectators>()
                        .0
                        .entry(client)
                        .or_default();
                }
                continue;
            }
            SpectateRequest::Next => Step::Next,
            SpectateRequest::Previous => Step::Previous,
        };
        if let Some(spectator) = world.resource_mut::<Spectators>().0.get_mut(&client) {
            spectator.step = Some(step);
        }
    }
}

/// Points every spectator at a player, wherever in `worlds` that player is. `arriving` are the players
/// between worlds, by the index of the world they're bound for. A spectator whose player is in another
/// world leaves theirs, and sees nothing until the transfer lands them there.
pub fn coordinate(
    worlds: &mut [App],
    arriving: impl Iterator<Item = (ClientId, usize)>,
) -> Vec<SpectatorTransfer> {
    if worlds
        .iter()
        .all(|app| app.world().resource::<Spectators>().0.is_empty())
    {
        return Vec::new();
    }
    let roster = roster(worlds, arriving);
    let mut transfers = Vec::new();
    for (index, app) in worlds.iter_mut().enumerate() {
        let world = app.world_mut();
        let clients: Vec<ClientId> = world.resource::<Spectators>().0.keys().copied().collect();
        for client in clients {
            let Some(mut spectator) = world.resource_mut::<Spectators>().0.remove(&client) else {
                continue;
            };
            spectator.watching = pick(&roster, spectator.watching, spectator.step.take());
            let status = match spectator.watching.and_then(|player| roster.watched(player)) {
                Some(watched) => Spectating::Player(watched),
                None => Spectating::Nobody,
            };
            if spectator.told != Some(status) {
                tell(world, client, status);
                spectator.told = Some(status);
            }
            match spectator
                .watching
                .and_then(|player| roster.world_of(player))
            {
                Some(dest) if dest != index => transfers.push(SpectatorTransfer {
                    client,
                    dest,
                    spectator,
                }),
                _ => {
                    world
                        .resource_mut::<Spectators>()
                        .0
                        .insert(client, spectator);
                }
            }
        }
    }
    transfers
}

pub fn arrive(world: &mut World, transfer: SpectatorTransfer) {
    // Told again from the destination: the departing world may never have sent the last word.
    let spectator = Spectator {
        told: None,
        ..transfer.spectator
    };
    world
        .resource_mut::<Spectators>()
        .0
        .insert(transfer.client, spectator);
}

pub fn client_left(
    remove: On<Remove, ClientId>,
    clients: Query<(Entity, &ClientId)>,
    mut spectators: ResMut<Spectators>,
) {
    let Ok((_, id)) = clients.get(remove.entity) else {
        return;
    };
    if clients
        .iter()
        .any(|(entity, other)| entity != remove.entity && other == id)
    {
        return;
    }
    spectators.0.remove(id);
}

#[derive(Clone, Copy, Debug)]
enum Step {
    Next,
    Previous,
}

struct Roster(Vec<Online>);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Online {
    player: ClientId,
    world: usize,
}

impl Roster {
    fn place(&self, player: ClientId) -> Result<usize, usize> {
        self.0.binary_search_by_key(&player, |online| online.player)
    }

    fn world_of(&self, player: ClientId) -> Option<usize> {
        self.place(player).ok().map(|place| self.0[place].world)
    }

    fn watched(&self, player: ClientId) -> Option<Watched> {
        let place = self.place(player).ok()?;
        Some(Watched {
            player,
            place: place as u32 + 1,
            online: self.0.len() as u32,
        })
    }
}

fn roster(worlds: &[App], arriving: impl Iterator<Item = (ClientId, usize)>) -> Roster {
    let mut online: Vec<Online> = worlds
        .iter()
        .enumerate()
        .flat_map(|(index, app)| {
            app.world()
                .resource::<Players>()
                .0
                .keys()
                .map(move |&client| (client, index))
        })
        .chain(arriving)
        .map(|(player, world)| Online { player, world })
        .collect();
    online.sort_unstable();
    Roster(online)
}

/// Keeps watching the same player while they're online. One who left hands over to the next in the
/// roster, so stepping from a player who left still lands next to where they were.
fn pick(roster: &Roster, watching: Option<ClientId>, step: Option<Step>) -> Option<ClientId> {
    let count = roster.0.len();
    if count == 0 {
        return None;
    }
    let index = match watching.map(|player| roster.place(player)) {
        None => 0,
        Some(Ok(place)) => match step {
            None => place,
            Some(Step::Next) => (place + 1) % count,
            Some(Step::Previous) => (place + count - 1) % count,
        },
        Some(Err(after)) => match step {
            None | Some(Step::Next) => after % count,
            Some(Step::Previous) => (after + count - 1) % count,
        },
    };
    Some(roster.0[index].player)
}

fn tell(world: &mut World, client: ClientId, status: Spectating) {
    let conns: Vec<Entity> = world
        .query::<(Entity, &ClientId)>()
        .iter(world)
        .filter(|(_, id)| **id == client)
        .map(|(conn, _)| conn)
        .collect();
    for conn in conns {
        world.write_message(ToClients {
            targets: SendTargets::Single(bevy_replicon::prelude::ClientId::Client(conn)),
            message: status,
        });
    }
}

fn allowed(world: &World, conn: Entity, client: ClientId) -> bool {
    let playing = world.resource::<Players>().0.contains_key(&client);
    let entitled = world
        .get::<Identity>(conn)
        .is_some_and(|identity| identity.has_role(Role::Spectate));
    !playing && entitled
}
