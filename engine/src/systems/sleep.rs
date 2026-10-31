//! What no player is near stands still: every entity in a chunk out of every player's reach is
//! disabled, so no mechanic's query finds it and no script of its runs, until a player comes near
//! again. A land costs only what its players are near, however much of it there is.

use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::prelude::*;
use bevy_platform::collections::HashSet;

use crate::core::chunk::{Chunk, ChunkIndex, InChunk};
use crate::core::content::Content;
use crate::core::units::Meters;
use crate::systems::mirror::{self, Ghost};
use crate::systems::movement::Position;
use crate::systems::player::PlayedBy;
use crate::systems::visibility::VIEW_DISTANCE;

/// The chunks awake, and how far from a player's chunk they reach.
#[derive(Resource)]
pub struct Sleep {
    reach: Meters,
    /// The chunks players stood in as of the last settle, which alone decide what is awake.
    stood: Vec<Chunk>,
    awake: HashSet<Chunk>,
}

impl Sleep {
    /// Awake is as far as a player sees, and as far again as anything there can reach out to
    /// what it sees: a skill landing, a script listening.
    pub fn of(content: &Content) -> Sleep {
        Sleep {
            reach: Meters(VIEW_DISTANCE.0 + mirror::halo(content).0.0),
            stood: Vec::new(),
            awake: HashSet::default(),
        }
    }
}

/// Wakes the chunks a player came near and puts to sleep those every player left, with whatever
/// walked or was put into a chunk asleep.
pub fn settle(world: &mut World, players: &mut PlayerPositions, filed: &mut Filed) {
    let mut stood: Vec<Chunk> = players
        .iter(world)
        .map(|position| Chunk::at(position.pos))
        .collect();
    stood.sort_unstable();
    stood.dedup();
    let (mut woken, mut falling) = (Vec::new(), Vec::new());
    let sleep = world.resource::<Sleep>();
    if stood != sleep.stood {
        let awake: HashSet<Chunk> = stood
            .iter()
            .flat_map(|chunk| chunk.within(sleep.reach))
            .collect();
        let index = world.resource::<ChunkIndex>();
        woken.extend(
            awake
                .difference(&sleep.awake)
                .flat_map(|&chunk| index.in_chunk(chunk).iter().copied()),
        );
        falling.extend(
            sleep
                .awake
                .difference(&awake)
                .flat_map(|&chunk| index.in_chunk(chunk).iter().copied()),
        );
        let mut sleep = world.resource_mut::<Sleep>();
        sleep.stood = stood;
        sleep.awake = awake;
    }
    let awake = &world.resource::<Sleep>().awake;
    falling.extend(
        filed
            .iter(world)
            .filter(|(_, filed)| !awake.contains(&filed.0))
            .map(|(entity, _)| entity),
    );
    for entity in woken {
        if let Ok(mut entity) = world.get_entity_mut(entity) {
            entity.remove::<Disabled>();
        }
    }
    for entity in falling {
        if let Ok(mut entity) = world.get_entity_mut(entity)
            && !entity.contains::<PlayedBy>()
            && !entity.contains::<Ghost>()
        {
            entity.insert(Disabled);
        }
    }
}

pub type PlayerPositions = QueryState<&'static Position, With<PlayedBy>>;

/// What was filed under a chunk since the last settle, of what may sleep.
pub type Filed =
    QueryState<(Entity, &'static InChunk), (Changed<InChunk>, Without<PlayedBy>, Without<Ghost>)>;
