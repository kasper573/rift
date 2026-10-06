use crate::core::time::Seconds;
use crate::data::attention::Id as AttentionId;
use crate::data::dialogue::Id as DialogueId;
use crate::data::item::Id as ItemId;
use crate::data::memory::Id as MemoryId;
use crate::data::npc::Id as NpcId;
use crate::data::prop::Id as PropId;
use crate::systems::actor::bust::Face::{Generic, Individual};
use crate::systems::actor::bust::GenericExpression::{Angry, Happy, Thinking};
use crate::systems::actor::bust::IndividualExpression::{Counting, Smug};
use crate::systems::dialogue::Line;
use crate::systems::dialogue::Speaker::Npc;
use crate::systems::interact::Counterpart;
use crate::systems::item::{ItemCategory, ItemStack};
use crate::systems::memory::Remembers;
use crate::systems::rule::Not;
use crate::systems::shop::{Buys, ShopBuys, ShopDef, ShopOffer, ShopReactions, Stock};
use crate::systems::text::plain;

crate::table! {
    MaraWares: ShopDef {
        title: "Mara's Wares",
        keeper: Counterpart::Npc(NpcId::Mara),
        requires: &[],
        mark: AttentionId::Merchant,
        ask: Some(&[plain("Show me your wares.")]),
        browsing: DialogueId::MaraShopping,
        sells: &[
            ShopOffer { item: ItemId::HealthPotion, count: 1, price: &[ItemStack::new(ItemId::Gold, 6)], stock: Stock::Unlimited, requires: &[] },
            ShopOffer {
                item: ItemId::GreaterHealthPotion,
                count: 1,
                price: &[ItemStack::new(ItemId::Gold, 10), ItemStack::new(ItemId::BatWing, 2)],
                stock: Stock::Limited { count: 3, restock: Seconds(600.0) },
                requires: &[],
            },
            ShopOffer { item: ItemId::FishSteak, count: 2, price: &[ItemStack::new(ItemId::Gold, 5)], stock: Stock::Unlimited, requires: &[] },
            ShopOffer {
                item: ItemId::BoneShield,
                count: 1,
                price: &[ItemStack::new(ItemId::Gold, 25)],
                stock: Stock::Limited { count: 1, restock: Seconds(3600.0) },
                requires: &[&Not(&Remembers(MemoryId::SidedWithOrcs))],
            },
        ],
        buys: &[
            ShopBuys {
                what: Buys::Item(ItemId::OrcTusk),
                pays: &[ItemStack::new(ItemId::Gold, 1)],
                requires: &[&Not(&Remembers(MemoryId::SidedWithOrcs))],
            },
            ShopBuys { what: Buys::Category(ItemCategory::Consumable), pays: &[ItemStack::new(ItemId::Gold, 2)], requires: &[] },
            ShopBuys { what: Buys::Category(ItemCategory::Equipment), pays: &[ItemStack::new(ItemId::Gold, 6)], requires: &[] },
            ShopBuys { what: Buys::Category(ItemCategory::Material), pays: &[ItemStack::new(ItemId::Gold, 1)], requires: &[] },
        ],
        reactions: Some(ShopReactions {
            bought: &[
                Line { by: Npc(NpcId::Mara), face: Some(Generic(Happy)), cue: true, text: &[plain("A fine choice. You won't find better this side of the strait.")] },
                Line { by: Npc(NpcId::Mara), face: Some(Individual(Counting)), cue: true, text: &[plain("Pleasure doing business.")] },
            ],
            sold: &[
                Line { by: Npc(NpcId::Mara), face: Some(Generic(Thinking)), cue: true, text: &[plain("Hm. I'll find a buyer for that.")] },
                Line { by: Npc(NpcId::Mara), face: Some(Individual(Smug)), cue: true, text: &[plain("It'll do. Don't tell anyone what I paid.")] },
            ],
            cant_afford: &[
                Line { by: Npc(NpcId::Mara), face: Some(Generic(Angry)), cue: true, text: &[plain("Come back when your purse is heavier.")] },
            ],
        }),
    },
    BoneExchange: ShopDef {
        title: "Bone Exchange",
        keeper: Counterpart::Npc(NpcId::Wren),
        requires: &[],
        mark: AttentionId::Collector,
        ask: Some(&[plain("I've brought bones.")]),
        browsing: DialogueId::WrenShopping,
        sells: &[
            ShopOffer { item: ItemId::GreaterHealthPotion, count: 1, price: &[ItemStack::new(ItemId::BoneToken, 3)], stock: Stock::Unlimited, requires: &[] },
            ShopOffer {
                item: ItemId::TribalHelmet,
                count: 1,
                price: &[ItemStack::new(ItemId::BoneToken, 12), ItemStack::new(ItemId::Gold, 20)],
                stock: Stock::Limited { count: 1, restock: Seconds(3600.0) },
                requires: &[],
            },
        ],
        buys: &[
            ShopBuys { what: Buys::Item(ItemId::Bone), pays: &[ItemStack::new(ItemId::BoneToken, 1)], requires: &[] },
            ShopBuys { what: Buys::Item(ItemId::BatWing), pays: &[ItemStack::new(ItemId::BoneToken, 1)], requires: &[] },
        ],
        reactions: None,
    },
    HonestyBox: ShopDef {
        title: "Tobb's honesty box",
        keeper: Counterpart::Prop(PropId::HonestyBox),
        requires: &[],
        mark: AttentionId::Merchant,
        ask: None,
        browsing: DialogueId::HonestyBoxShopping,
        sells: &[ShopOffer {
            item: ItemId::FishSteak,
            count: 1,
            price: &[ItemStack::new(ItemId::Gold, 3)],
            stock: Stock::Limited { count: 5, restock: Seconds(1800.0) },
            requires: &[],
        }],
        buys: &[],
        reactions: None,
    },
    UgraRemedies: ShopDef {
        title: "Clan remedies",
        keeper: Counterpart::Npc(NpcId::Ugra),
        requires: &[&Remembers(MemoryId::SidedWithOrcs)],
        mark: AttentionId::Merchant,
        ask: Some(&[plain("Show me the clan's remedies.")]),
        browsing: DialogueId::UgraShopping,
        sells: &[
            ShopOffer {
                item: ItemId::GreaterHealthPotion,
                count: 1,
                price: &[ItemStack::new(ItemId::Bone, 4), ItemStack::new(ItemId::BatWing, 2)],
                stock: Stock::Unlimited,
                requires: &[],
            },
            ShopOffer {
                item: ItemId::TribalHelmet,
                count: 1,
                price: &[ItemStack::new(ItemId::OrcTusk, 8)],
                stock: Stock::Limited { count: 1, restock: Seconds(3600.0) },
                requires: &[],
            },
        ],
        buys: &[ShopBuys { what: Buys::Item(ItemId::OrcTusk), pays: &[ItemStack::new(ItemId::Gold, 2)], requires: &[] }],
        reactions: None,
    },
}
