//! What no character is near stands still: every entity in a chunk out of every character's reach is
//! disabled, so no mechanic's query finds it and no script of its runs, until a character comes near
//! again. A land costs only what its characters are near, however much of it there is.

use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::prelude::*;
use bevy_platform::collections::HashSet;

use crate::core::chunk::{Chunk, ChunkIndex, ChunkSpan, InChunk};
use crate::core::content::Content;
use crate::core::units::Meters;
use crate::systems::GameSettings;
use crate::systems::movement::Position;
use crate::systems::player::PlayedBy;
use crate::systems::script::RiftScripts;
use crate::systems::script::event::RiftEvent;
use crate::systems::skill::SkillDef;

/// The chunks awake, and how far from a character they reach.
#[derive(Resource)]
pub struct Sleep {
    reach: Meters,
    stood: Vec<Chunk>,
    /// The chunks within reach of each character as of the last settle, sorted, each span once: they
    /// alone decide what is awake.
    reached: Vec<ChunkSpan>,
    awake: HashSet<Chunk>,
}

impl Sleep {
    /// The chunks characters stood in as of the last settle, sorted, each once.
    pub fn stood(&self) -> &[Chunk] {
        &self.stood
    }

    pub fn of(content: &Content) -> Sleep {
        Sleep {
            reach: awake_reach(content),
            stood: Vec::new(),
            reached: Vec::new(),
            awake: HashSet::default(),
        }
    }
}

/// Wakes the chunks a character came within reach of and puts to sleep those every character left, with
/// whatever walked or was put into a chunk asleep.
pub fn settle(world: &mut World, players: &mut PlayerPositions, filed: &mut Filed) {
    let reach = world.resource::<Sleep>().reach;
    let (mut stood, mut reached): (Vec<Chunk>, Vec<ChunkSpan>) = players
        .iter(world)
        .map(|position| {
            (
                Chunk::at(position.pos),
                ChunkSpan::around(position.pos, reach),
            )
        })
        .unzip();
    stood.sort_unstable();
    stood.dedup();
    reached.sort_unstable();
    reached.dedup();
    let (mut woken, mut falling) = (Vec::new(), Vec::new());
    let sleep = world.resource::<Sleep>();
    if reached != sleep.reached {
        let awake: HashSet<Chunk> = reached.iter().flat_map(|span| span.chunks()).collect();
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
        sleep.reached = reached;
        sleep.awake = awake;
    }
    if stood != world.resource::<Sleep>().stood {
        world.resource_mut::<Sleep>().stood = stood;
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
        {
            entity.insert(Disabled);
        }
    }
}

/// Awake is as far as a player sees, and as far again as anything there can reach out to what it
/// sees: a skill landing, a script listening.
pub fn awake_reach(content: &Content) -> Meters {
    Meters(content.built::<GameSettings>().sight.0 + acting_reach(content).0)
}

/// How far anything acts on what it sees: as far as a client sees, a skill lands or a script's
/// event listens. What stands that far past a player's sight must be awake for what the player
/// sees to act as it would.
fn acting_reach(content: &Content) -> Meters {
    let mut reach = content.built::<GameSettings>().sight;
    for (_, skill) in content.table::<SkillDef>().iter() {
        let lands = skill.range.map_or(0.0, |range| range.0)
            + skill.area.as_ref().map_or(0.0, |area| area.reach().0);
        reach = Meters(reach.0.max(lands));
    }
    for (_, procedures) in content.built::<RiftScripts>().owners() {
        for procedure in procedures {
            for (_, event) in &procedure.entries {
                if let RiftEvent::CameNear { within, .. } | RiftEvent::Heard { within, .. } = event
                {
                    reach = Meters(reach.0.max(within.0));
                }
            }
        }
    }
    reach
}

type PlayerPositions = QueryState<&'static Position, With<PlayedBy>>;

/// What was filed under a chunk since the last settle, of what may sleep.
type Filed = QueryState<(Entity, &'static InChunk), (Changed<InChunk>, Without<PlayedBy>)>;
