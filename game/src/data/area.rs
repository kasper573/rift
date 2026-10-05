use crate::core::assets::AssetRef;
use crate::data::npc::Id as NpcId;
use crate::systems::area::{AreaDef, Population};

crate::table! {
    Island: AreaDef {
        map: AssetRef("maps/island.tmx"),
        populations: &[
            Population { npc: NpcId::Orc, count: 6 },
            Population { npc: NpcId::Skeleton, count: 8 },
            Population { npc: NpcId::VampireBat, count: 5 },
            Population { npc: NpcId::Bat, count: 4 },
            Population { npc: NpcId::OrcChief, count: 2 },
        ],
        residents: &[],
    },
    Forest: AreaDef {
        map: AssetRef("maps/forest.tmx"),
        populations: &[
            Population { npc: NpcId::Orc, count: 8 },
            Population { npc: NpcId::OrcChief, count: 3 },
            Population { npc: NpcId::Skeleton, count: 5 },
            Population { npc: NpcId::Bat, count: 6 },
        ],
        residents: &[],
    },
}

pub const BENCH_ID: Id = Id::Island;

/// The area new players spawn into.
pub const SPAWN_ID: Id = Id::Island;
