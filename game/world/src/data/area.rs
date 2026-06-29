use crate::core::assets::AssetRef;
use crate::data::npc::Id as NpcId;
use crate::systems::area::{AreaDef, Spawn};

// `Island`/`Forest` are the real areas; `BenchArea1..=768` are built-in
// load-test areas, so the benchmark scales area count without runtime
// instancing. Keep this at roughly 2x the bench's sustained capacity so the
// search always has headroom and never caps out. `seq!` only generates the
// bench rows; `table!` stays the single source of truth for the table itself.
seq_macro::seq!(N in 1..=768 {
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
                spawns: &[
                    Spawn { npc: NpcId::Orc, population: 6 },
                    Spawn { npc: NpcId::Skeleton, population: 8 },
                    Spawn { npc: NpcId::VampireBat, population: 5 },
                    Spawn { npc: NpcId::Bat, population: 4 },
                    Spawn { npc: NpcId::OrcChief, population: 2 },
                ],
            },
        )*
    }
});

/// The area new players spawn into.
pub const SPAWN_ID: Id = Id::Island;
