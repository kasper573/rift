use crate::data::area::Id as AreaId;
use crate::data::npc::Id as NpcId;
use crate::systems::npc::SpawnRow;

crate::table! {
    IslandOrc: SpawnRow {
        npc: NpcId::Orc,
        area: AreaId::Island,
        population: 6,
    },
    IslandSkeleton: SpawnRow {
        npc: NpcId::Skeleton,
        area: AreaId::Island,
        population: 8,
    },
    IslandVampireBat: SpawnRow {
        npc: NpcId::VampireBat,
        area: AreaId::Island,
        population: 5,
    },
    IslandBat: SpawnRow {
        npc: NpcId::Bat,
        area: AreaId::Island,
        population: 4,
    },
    IslandOrcChief: SpawnRow {
        npc: NpcId::OrcChief,
        area: AreaId::Island,
        population: 2,
    },
    ForestOrc: SpawnRow {
        npc: NpcId::Orc,
        area: AreaId::Forest,
        population: 8,
    },
    ForestOrcChief: SpawnRow {
        npc: NpcId::OrcChief,
        area: AreaId::Forest,
        population: 3,
    },
    ForestSkeleton: SpawnRow {
        npc: NpcId::Skeleton,
        area: AreaId::Forest,
        population: 5,
    },
    ForestBat: SpawnRow {
        npc: NpcId::Bat,
        area: AreaId::Forest,
        population: 6,
    },
}
