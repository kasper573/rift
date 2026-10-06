use bevy_ecs::prelude::*;
use game::core::tiling::Tiles;
use game::core::time::{Seconds, WallClock};
use game::data;
use game::data::dialogue::Id as DialogueId;
use game::data::item::Id as ItemId;
use game::data::prop::Id as PropId;
use game::systems::dialogue::{Conversation, ConversationRequest, Speaker};
use game::systems::input::map::InputMap;
use game::systems::interact::InteractRequest;
use game::systems::item::{Inventory, ItemStack};
use game::systems::movement::{Position, position};
use game::systems::npc::Npc;
use game::systems::prop::Prop;
use game::systems::shop::{ShopId, ShopRequest, ShopView};

use crate::support::Sim;

fn townsperson(sim: &mut Sim, who: data::npc::Id) -> Entity {
    let world = sim.world();
    world
        .query::<(Entity, &Npc)>()
        .iter(world)
        .find(|(_, npc)| npc.def == who)
        .map(|(entity, _)| entity)
        .expect("a resident")
}

fn prop(sim: &mut Sim, which: PropId) -> Entity {
    let world = sim.world();
    world
        .query::<(Entity, &Prop)>()
        .iter(world)
        .find(|(_, prop)| prop.def == which)
        .map(|(entity, _)| entity)
        .expect("a fixture on the map")
}

fn conversation(sim: &mut Sim, player: Entity) -> Option<Conversation> {
    sim.world().get::<Conversation>(player).cloned()
}

fn shown(sim: &mut Sim, player: Entity) -> Option<ShopView> {
    sim.world().get::<ShopView>(player).cloned()
}

fn browse(sim: &mut Sim, client: u32, player: Entity, keeper: data::npc::Id) -> ShopView {
    let target = townsperson(sim, keeper);
    sim.send(client, InteractRequest { target });
    assert!(
        sim.run_until(20.0, |world| world.get::<Conversation>(player).is_some()),
        "the conversation with {keeper:?} never opened"
    );
    let greeting = conversation(sim, player).expect("talking");
    let ask = greeting
        .choices
        .iter()
        .position(|choice| choice.label.words(&InputMap::default()) == ask_of(keeper))
        .expect("the shop's choice in the greeting") as u32;
    sim.send(
        client,
        ConversationRequest::Pick {
            step: greeting.step,
            choice: ask,
        },
    );
    sim.tick();
    shown(sim, player).expect("the shop opened")
}

fn ask_of(keeper: data::npc::Id) -> &'static str {
    match keeper {
        data::npc::Id::Mara => "Show me your wares.",
        data::npc::Id::Wren => "I've brought bones.",
        other => panic!("{other:?} keeps no shop"),
    }
}

fn give(sim: &mut Sim, player: Entity, item: ItemId, count: u32) {
    sim.world()
        .get_mut::<Inventory>(player)
        .expect("bag")
        .exchange(&[], &[ItemStack::new(item, count)])
        .expect("room");
}

fn count(sim: &mut Sim, player: Entity, item: ItemId) -> u32 {
    sim.world()
        .get::<Inventory>(player)
        .expect("bag")
        .count(item)
}

fn slot_of(sim: &mut Sim, player: Entity, item: ItemId) -> u32 {
    sim.world()
        .get::<Inventory>(player)
        .expect("bag")
        .slots
        .iter()
        .position(|stack| stack.item == item)
        .expect("in the bag") as u32
}

fn offer_of(shop: ShopId, item: ItemId) -> u32 {
    shop.get()
        .sells
        .iter()
        .position(|offer| offer.item == item)
        .expect("on sale") as u32
}

fn later(sim: &mut Sim, seconds: f32) {
    let clock = *sim.world().resource::<WallClock>();
    sim.world().insert_resource(WallClock {
        now: clock.now.after(Seconds(seconds)),
        ..clock
    });
    sim.tick();
}

#[test]
fn asking_a_keeper_opens_the_shop_and_buying_trades_the_price_for_the_goods() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    give(&mut sim, player, ItemId::Gold, 10);

    let opened = browse(&mut sim, 1, player, data::npc::Id::Mara);
    assert_eq!(opened.shop, ShopId::MaraWares);
    assert_eq!(
        conversation(&mut sim, player).map(|now| now.node),
        Some(DialogueId::MaraShopping)
    );

    sim.send(
        1,
        ShopRequest::Buy {
            offer: offer_of(ShopId::MaraWares, ItemId::HealthPotion),
        },
    );
    sim.tick();

    assert_eq!(count(&mut sim, player, ItemId::Gold), 4);
    assert_eq!(count(&mut sim, player, ItemId::HealthPotion), 1);
}

#[test]
fn a_purchase_you_cannot_cover_changes_nothing_and_the_keeper_says_so() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    give(&mut sim, player, ItemId::Gold, 12);
    let mara = townsperson(&mut sim, data::npc::Id::Mara);
    let opened = browse(&mut sim, 1, player, data::npc::Id::Mara);
    let potion = offer_of(ShopId::MaraWares, ItemId::GreaterHealthPotion);
    assert!(opened.offers[potion as usize].refusal.is_some());

    sim.send(1, ShopRequest::Buy { offer: potion });
    sim.tick();

    assert_eq!(count(&mut sim, player, ItemId::Gold), 12);
    assert_eq!(count(&mut sim, player, ItemId::GreaterHealthPotion), 0);
    let talking = conversation(&mut sim, player).expect("still browsing");
    assert_eq!(talking.with, Some(mara));
    let remark = talking.remark.expect("Mara reacts");
    assert_eq!(remark.line.by, Speaker::Npc(data::npc::Id::Mara));
}

#[test]
fn limited_stock_is_kept_per_player_and_restocks_on_its_timer() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let first = sim.join(1);
    let second = sim.join(2);
    for player in [first, second] {
        give(&mut sim, player, ItemId::Gold, 100);
        give(&mut sim, player, ItemId::BatWing, 20);
    }
    browse(&mut sim, 1, first, data::npc::Id::Mara);
    browse(&mut sim, 2, second, data::npc::Id::Mara);
    let potion = offer_of(ShopId::MaraWares, ItemId::GreaterHealthPotion);

    for _ in 0..4 {
        sim.send(1, ShopRequest::Buy { offer: potion });
        sim.tick();
    }
    assert_eq!(count(&mut sim, first, ItemId::GreaterHealthPotion), 3);
    let sold_out = shown(&mut sim, first).expect("open");
    assert_eq!(sold_out.offers[potion as usize].left, Some(0));

    sim.send(2, ShopRequest::Buy { offer: potion });
    sim.tick();
    assert_eq!(count(&mut sim, second, ItemId::GreaterHealthPotion), 1);

    later(&mut sim, 600.0);
    sim.send(1, ShopRequest::Buy { offer: potion });
    sim.tick();
    assert_eq!(count(&mut sim, first, ItemId::GreaterHealthPotion), 4);
}

#[test]
fn a_collector_pays_in_his_own_currency_and_buyback_costs_exactly_what_he_paid() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    give(&mut sim, player, ItemId::Bone, 7);
    give(&mut sim, player, ItemId::OrcTusk, 1);

    let opened = browse(&mut sim, 1, player, data::npc::Id::Wren);
    assert_eq!(opened.shop, ShopId::BoneExchange);
    assert_eq!(
        conversation(&mut sim, player).map(|now| now.node),
        Some(DialogueId::WrenShopping),
        "a shop without reactions still keeps its conversation"
    );

    let tusk = slot_of(&mut sim, player, ItemId::OrcTusk);
    sim.send(
        1,
        ShopRequest::Sell {
            slot: tusk,
            stack: ItemStack::new(ItemId::OrcTusk, 1),
        },
    );
    sim.tick();
    assert_eq!(count(&mut sim, player, ItemId::OrcTusk), 1);

    let bones = slot_of(&mut sim, player, ItemId::Bone);
    let all_bones = ItemStack::new(ItemId::Bone, 7);
    sim.send(
        1,
        ShopRequest::Sell {
            slot: bones,
            stack: all_bones,
        },
    );
    sim.send(
        1,
        ShopRequest::Sell {
            slot: bones,
            stack: all_bones,
        },
    );
    sim.tick();
    assert_eq!(count(&mut sim, player, ItemId::Bone), 0);
    assert_eq!(count(&mut sim, player, ItemId::BoneToken), 7);
    let sold = shown(&mut sim, player).expect("open");
    assert_eq!(sold.buyback.len(), 1);

    sim.send(1, ShopRequest::Buyback { sale: 0 });
    sim.tick();
    assert_eq!(count(&mut sim, player, ItemId::Bone), 7);
    assert_eq!(count(&mut sim, player, ItemId::BoneToken), 0);
    assert!(shown(&mut sim, player).expect("open").buyback.is_empty());
}

#[test]
fn walking_out_of_reach_closes_the_shop() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    browse(&mut sim, 1, player, data::npc::Id::Wren);
    let at = position(sim.world(), player).expect("position");

    let away = sim.walkable_near(at, Tiles(8.0), Tiles(12.0));
    sim.world()
        .entity_mut(player)
        .insert(Position { pos: away });
    sim.tick();

    assert!(shown(&mut sim, player).is_none());
}

#[test]
fn leaving_the_conversation_closes_the_shop() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    browse(&mut sim, 1, player, data::npc::Id::Mara);
    let step = conversation(&mut sim, player).expect("browsing").step;

    sim.send(1, ConversationRequest::Leave { step });
    sim.tick();

    assert!(conversation(&mut sim, player).is_none());
    assert!(shown(&mut sim, player).is_none());
}

#[test]
fn a_map_object_can_keep_a_shop() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    give(&mut sim, player, ItemId::Gold, 3);
    let honesty_box = prop(&mut sim, PropId::HonestyBox);

    sim.send(
        1,
        InteractRequest {
            target: honesty_box,
        },
    );
    assert!(sim.run_until(20.0, |world| world.get::<ShopView>(player).is_some()));
    assert_eq!(
        shown(&mut sim, player).map(|opened| opened.shop),
        Some(ShopId::HonestyBox)
    );
    assert_eq!(
        conversation(&mut sim, player).map(|now| now.node),
        Some(DialogueId::HonestyBoxShopping),
        "the box keeps its shop inside a conversation"
    );

    sim.send(
        1,
        ShopRequest::Buy {
            offer: offer_of(ShopId::HonestyBox, ItemId::FishSteak),
        },
    );
    sim.tick();
    assert_eq!(count(&mut sim, player, ItemId::FishSteak), 1);
    assert_eq!(count(&mut sim, player, ItemId::Gold), 0);
}
