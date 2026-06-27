use crate::core::assets::AssetRef;
use crate::data::npc::Id as NpcId;
use crate::systems::area::{AreaDef, Spawn};

// `Island`/`Forest` are the real areas; `BenchArea1..=256` are built-in
// load-test areas, so the benchmark scales area count without runtime
// instancing. `seq!` only generates the bench rows; `table!` stays the
// single source of truth for the table itself.
seq_macro::seq!(N in 1..=256 {
    crate::table! {
        Island: AreaDef {
            map: AssetRef("maps/island.tmx"),
            bench: false,
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
            bench: false,
            spawns: &[
                Spawn { npc: NpcId::Orc, population: 8 },
                Spawn { npc: NpcId::OrcChief, population: 3 },
                Spawn { npc: NpcId::Skeleton, population: 5 },
                Spawn { npc: NpcId::Bat, population: 6 },
            ],
        },
        #(
            BenchArea~N: AreaDef {
                map: AssetRef("maps/island.tmx"),
                bench: true,
                spawns: &[],
            },
        )*
    }
});

/// The area new players spawn into.
pub const SPAWN_ID: Id = Id::Island;
