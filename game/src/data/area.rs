use crate::core::assets::AssetRef;
use crate::data::memory::Id as MemoryId;
use crate::data::notification::Id as NotificationId;
use crate::data::npc::Id as NpcId;
use crate::data::prop::Id as PropId;
use crate::systems::area::{AreaDef, MarkerName, Population, Resident, Zone};
use crate::systems::memory::{Remember, Remembers};
use crate::systems::notification::Notify;
use crate::systems::prop::Fixture;
use crate::systems::rule::Not;

crate::table! {
    Island: AreaDef {
        name: "The island",
        map: AssetRef("maps/island.tmx"),
        populations: &[
            Population { npc: NpcId::Orc, count: 6, roams: None },
            Population { npc: NpcId::Skeleton, count: 8, roams: None },
            Population { npc: NpcId::VampireBat, count: 5, roams: None },
            Population { npc: NpcId::Bat, count: 4, roams: None },
            Population { npc: NpcId::OrcChief, count: 2, roams: None },
        ],
        residents: &[
            Resident { npc: NpcId::Grisha, at: MarkerName("inn"), shown: &[] },
            Resident { npc: NpcId::Mara, at: MarkerName("market-stall"), shown: &[] },
            Resident { npc: NpcId::Tobb, at: MarkerName("fishing-pier"), shown: &[] },
            Resident { npc: NpcId::Bram, at: MarkerName("ferry-landing"), shown: &[] },
            Resident { npc: NpcId::Wren, at: MarkerName("bone-heap"), shown: &[] },
            Resident {
                npc: NpcId::Pell,
                at: MarkerName("dice-table"),
                shown: &[&Not(&Remembers(MemoryId::PellFighting)), &Not(&Remembers(MemoryId::PellDead))],
            },
            Resident { npc: NpcId::Ilsa, at: MarkerName("road-warden"), shown: &[] },
        ],
        props: &[
            Fixture { prop: PropId::HarbourBoard, at: MarkerName("notice-board"), shown: &[] },
            Fixture { prop: PropId::TideChest, at: MarkerName("tide-chest"), shown: &[] },
            Fixture { prop: PropId::HonestyBox, at: MarkerName("honesty-box"), shown: &[] },
        ],
        zones: &[
            Zone {
                at: MarkerName("ferry-pier"),
                with: None,
                requires: &[&Not(&Remembers(MemoryId::HeardGulls))],
                then: &[&Remember(MemoryId::HeardGulls), &Notify(NotificationId::Gulls)],
            },
        ],
        intro: None,
    },
    Forest: AreaDef {
        name: "The forest",
        map: AssetRef("maps/forest.tmx"),
        populations: &[
            Population { npc: NpcId::Orc, count: 8, roams: None },
            Population { npc: NpcId::OrcChief, count: 3, roams: None },
            Population { npc: NpcId::Skeleton, count: 5, roams: None },
            Population { npc: NpcId::Bat, count: 6, roams: None },
        ],
        residents: &[Resident { npc: NpcId::Ugra, at: MarkerName("ugra-camp"), shown: &[] }],
        props: &[Fixture { prop: PropId::StandingStone, at: MarkerName("standing-stone"), shown: &[] }],
        zones: &[],
        intro: Some(NotificationId::ForestIntro),
    },
}

pub const BENCH_ID: Id = Id::Island;

/// The area new players spawn into.
pub const SPAWN_ID: Id = Id::Island;
