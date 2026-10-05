use crate::core::assets::AssetRef;
use crate::data::announcement::Id as AnnouncementId;
use crate::data::dialogue::Id as DialogueId;
use crate::data::item::Id as ItemId;
use crate::data::memory::Id as MemoryId;
use crate::data::npc::Id as NpcId;
use crate::data::prop::Id as PropId;
use crate::systems::area::{AreaDef, MarkerName, Population, Resident, Zone};
use crate::systems::dialogue::announcement::Announce;
use crate::systems::dialogue::{BusyPolicy, StartConversation};
use crate::systems::item::{Holding, ItemStack};
use crate::systems::memory::{Remember, Remembers};
use crate::systems::movement::HeadingTo;
use crate::systems::prop::Fixture;
use crate::systems::rule::Not;

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
            Resident { npc: NpcId::Ilsa, at: MarkerName("forest-road"), shown: &[] },
        ],
        props: &[
            Fixture { prop: PropId::HarbourNotices, at: MarkerName("notice-board"), shown: &[] },
            Fixture { prop: PropId::TideChest, at: MarkerName("tide-chest"), shown: &[] },
            Fixture { prop: PropId::HonestyBox, at: MarkerName("honesty-box"), shown: &[] },
        ],
        zones: &[
            Zone {
                at: MarkerName("forest-gate"),
                with: Some(NpcId::Ilsa),
                requires: &[
                    &HeadingTo(Id::Forest),
                    &Not(&Remembers(MemoryId::IlsaHaltedYou)),
                    &Not(&Holding(ItemStack::new(ItemId::RoadPass, 1))),
                ],
                then: &[
                    &StartConversation { node: DialogueId::IlsaHalt, busy: BusyPolicy::Skip },
                    &Remember(MemoryId::IlsaHaltedYou),
                ],
            },
            Zone {
                at: MarkerName("ferry-pier"),
                with: None,
                requires: &[&Not(&Remembers(MemoryId::HeardHarbourBell))],
                then: &[
                    &Remember(MemoryId::HeardHarbourBell),
                    &Announce(AnnouncementId::HarbourBell),
                    &Announce(AnnouncementId::Gulls),
                ],
            },
        ],
        intro: None,
    },
    Forest: AreaDef {
        map: AssetRef("maps/forest.tmx"),
        populations: &[
            Population { npc: NpcId::Orc, count: 8 },
            Population { npc: NpcId::OrcChief, count: 3 },
            Population { npc: NpcId::Skeleton, count: 5 },
            Population { npc: NpcId::Bat, count: 6 },
        ],
        residents: &[Resident { npc: NpcId::Ugra, at: MarkerName("ugra-camp"), shown: &[] }],
        props: &[Fixture { prop: PropId::StandingStone, at: MarkerName("standing-stone"), shown: &[] }],
        zones: &[],
        intro: Some(AnnouncementId::ForestArrival),
    },
}

pub const BENCH_ID: Id = Id::Island;

/// The area new players spawn into.
pub const SPAWN_ID: Id = Id::Island;
