use crate::core::assets::AssetRef;
use crate::data::npc::Id as NpcId;
use crate::systems::area::{AreaDef, Spawn};

crate::table! {
    Island: AreaDef {
        map: AssetRef("maps/island.tmx"),
        spawns: &[
            Spawn { npc: NpcId::Orc, population: 6 },
            Spawn { npc: NpcId::Skeleton, population: 8 },
            Spawn { npc: NpcId::VampireBat, population: 5 },
            Spawn { npc: NpcId::Bat, population: 4 },
            Spawn { npc: NpcId::OrcChief, population: 2 },
        ],
    },
    Forest: AreaDef {
        map: AssetRef("maps/forest.tmx"),
        spawns: &[
            Spawn { npc: NpcId::Orc, population: 8 },
            Spawn { npc: NpcId::OrcChief, population: 3 },
            Spawn { npc: NpcId::Skeleton, population: 5 },
            Spawn { npc: NpcId::Bat, population: 6 },
        ],
    },
}

pub const BENCH_ID: Id = Id::Island;

/// The area new players spawn into.
pub const SPAWN_ID: Id = Id::Island;
