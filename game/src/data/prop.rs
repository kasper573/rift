use crate::core::assets::AssetRef;
use crate::core::tiling::Tiles;
use crate::data::dialogue::Id as DialogueId;
use crate::data::item::Id as ItemId;
use crate::data::memory::Id as MemoryId;
use crate::systems::dialogue::{BusyPolicy, StartConversation};
use crate::systems::interact::{Interaction, Response, Verb};
use crate::systems::item::{GiveItems, ItemStack};
use crate::systems::memory::{Remember, Remembers};
use crate::systems::prop::PropDef;
use crate::systems::rule::Not;
use crate::systems::shop::{OpenShop, ShopId};

crate::table! {
    HarbourNotices: PropDef {
        display_name: "Harbour notices",
        look: Some(AssetRef("icons/misc/scroll.png")),
        interaction: Some(Interaction {
            verb: Verb::Read,
            reach: Tiles(1.5),
            responses: &[Response {
                requires: &[],
                then: &[&StartConversation { node: DialogueId::HarbourNotices, busy: BusyPolicy::Replace }],
                news: false,
            }],
            marks: &[],
        }),
    },
    TideChest: PropDef {
        display_name: "Tide chest",
        look: Some(AssetRef("icons/misc/chest.png")),
        interaction: Some(Interaction {
            verb: Verb::Open,
            reach: Tiles(1.5),
            responses: &[
                Response {
                    requires: &[&Not(&Remembers(MemoryId::FoundTatteredMap))],
                    then: &[
                        &GiveItems(&[ItemStack::new(ItemId::Gold, 5), ItemStack::new(ItemId::TatteredMap, 1)]),
                        &Remember(MemoryId::TideChestLooted),
                        &Remember(MemoryId::FoundTatteredMap),
                        &StartConversation { node: DialogueId::TideChestMap, busy: BusyPolicy::Replace },
                    ],
                    news: false,
                },
                Response {
                    requires: &[&Not(&Remembers(MemoryId::TideChestLooted))],
                    then: &[
                        &GiveItems(&[ItemStack::new(ItemId::Gold, 5)]),
                        &Remember(MemoryId::TideChestLooted),
                        &StartConversation { node: DialogueId::TideChestFound, busy: BusyPolicy::Replace },
                    ],
                    news: false,
                },
                Response {
                    requires: &[],
                    then: &[&StartConversation { node: DialogueId::TideChestEmpty, busy: BusyPolicy::Replace }],
                    news: false,
                },
            ],
            marks: &[],
        }),
    },
    HonestyBox: PropDef {
        display_name: "Honesty box",
        look: Some(AssetRef("icons/misc/crate.png")),
        interaction: Some(Interaction {
            verb: Verb::Use,
            reach: Tiles(1.5),
            responses: &[Response { requires: &[], then: &[&OpenShop(ShopId::HonestyBox)], news: false }],
            marks: &[],
        }),
    },
    StandingStone: PropDef {
        display_name: "Standing stone",
        look: Some(AssetRef("icons/misc/rune_stone.png")),
        interaction: Some(Interaction {
            verb: Verb::Use,
            reach: Tiles(1.5),
            responses: &[Response {
                requires: &[],
                then: &[&StartConversation { node: DialogueId::StandingStone, busy: BusyPolicy::Replace }],
                news: false,
            }],
            marks: &[],
        }),
    },
}
