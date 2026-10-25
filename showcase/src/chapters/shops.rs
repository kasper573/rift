use driver::{Player, UiTarget, content, holding, input_name};
use engine::core::math::Offset;
use engine::systems::input::map::InputAction;
use engine::systems::probe::ProbeSnapshot;

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "shops",
    title: "Shops",
    summary: "Buy, sell and buy back — from the townsfolk, and from a box on the pier",
    play,
};

fn play(shoot: &mut Shoot) {
    let player = shoot.join_as("shopper", None);
    let mut me = shoot.roll(player);
    me.caption("Mara keeps a shop: ask, and her wares open above the conversation");
    me.talk_to("mara");
    me.read_to_choices();
    me.pick("Show me your wares.");
    me.wait_until("Mara's shop", |snapshot| shop_is(snapshot, "mara_wares"));
    me.caption("It belongs to the conversation: fixed above it, with no close button");
    let before = me.find_ui("Mara's Wares").rect;
    me.drag_ui("Mara's Wares", Offset::new(-300.0, 0.0));
    let after = me.settled();
    assert_eq!(
        UiTarget::text("Mara's Wares")
            .find(&after)
            .map(|title| title.rect),
        Some(before),
        "the shop panel stays put"
    );

    me.caption(
        "The chosen ware lies on the page as its own card, the trade button under it; a price you can't cover turns rubric in the list",
    );
    me.click_ui("Greater Health Potion");
    me.caption("The list stays as narrow as its wares, and browsing never shifts it");
    let listed = row_x(&mut me, "Bone Shield");
    for ware in ["Bone Shield", "Health Potion", "Greater Health Potion"] {
        me.click_ui(UiTarget::button(ware));
        me.settled();
        assert_eq!(
            row_x(&mut me, "Bone Shield"),
            listed,
            "the ware list holds still"
        );
    }
    me.caption("Mara's shop opts into reactions — try buying anyway");
    me.click_ui("Can't afford");
    me.find_ui(UiTarget::containing("purse is heavier"));

    me.caption("Buying trades the price for the goods");
    me.click_ui("Health Potion");
    me.click_ui("Buy");
    me.wait_until("the potion in the bag", |snapshot| {
        holding(snapshot, "health_potion") == 1
    });

    me.caption("Limited stock is yours alone, and restocks on a timer");
    me.click_ui("Bone Shield");
    me.click_ui("Buy");
    me.wait_until("the shield to sell out", |snapshot| {
        snapshot.shop.as_ref().is_some_and(|shop| {
            shop.offers
                .iter()
                .any(|offer| offer.item.name(content()) == "bone_shield" && offer.left == Some(0))
        })
    });

    me.caption("With a shop open, your bag shows what sells and dims what doesn't");
    me.act(InputAction::ToggleInventory);
    me.drag_ui("Inventory", Offset::new(384.0, -260.0));
    me.caption(&format!("{} to sell…", input_name(InputAction::SellItem)));
    me.act_on_ui(InputAction::SellItem, UiTarget::item("bat_wing"));
    me.wait_until("the wings to sell", |snapshot| {
        holding(snapshot, "bat_wing") == 0
    });
    me.caption(&format!(
        "…or {} onto the shop. Equipment asks first",
        input_name(InputAction::CarryItem)
    ));
    me.carry_onto(UiTarget::item("rusty_sword"), UiTarget::starting("Buys "));
    me.find_ui("Sell Rusty Sword?");
    me.click_ui("Sell");
    me.wait_until("the sword to sell", |snapshot| {
        holding(snapshot, "rusty_sword") == 0
    });

    me.caption("Changed your mind? Buy it back for exactly what she paid");
    me.click_ui(UiTarget::starting("Buy back ("));
    me.click_ui("Rusty Sword");
    me.click_ui("Buy back");
    me.wait_until("the sword back in the bag", |snapshot| {
        holding(snapshot, "rusty_sword") == 1
    });

    close_bag(&mut me);
    me.caption("Leaving the conversation closes the shop with it");
    me.act(InputAction::Dismiss);
    me.conversation_over();
    me.wait_until("the shop to close", |snapshot| snapshot.shop.is_none());

    me.caption("Wren collects bones and wings, and pays in her own Bone Tokens");
    me.talk_to("wren");
    me.read_to_choices();
    me.pick("I've brought bones.");
    me.wait_until("Wren's exchange", |snapshot| {
        shop_is(snapshot, "bone_exchange")
    });
    me.caption(
        "Her shop has no reactions: trades pass in silence, and your bag lights up what she takes",
    );
    me.act(InputAction::ToggleInventory);
    me.hold();
    me.act_on_ui(InputAction::SellItem, UiTarget::item("bone"));
    me.wait_until("the bones to sell", |snapshot| {
        holding(snapshot, "bone_token") == 4
    });
    close_bag(&mut me);
    me.act(InputAction::Dismiss);
    me.wait_until("the exchange to close", |snapshot| snapshot.shop.is_none());

    me.caption("Even a box at the harbour can keep a shop, inside a conversation of its own");
    me.shop_at("honesty-box");
    me.click_ui("Fish Steak");
    me.click_ui("Buy");
    me.wait_until("the fish steak in the bag", |snapshot| {
        holding(snapshot, "fish_steak") == 1
    });
}

fn shop_is(snapshot: &ProbeSnapshot, shop: &str) -> bool {
    snapshot
        .shop
        .as_ref()
        .is_some_and(|open| open.shop.name(content()) == shop)
}

// Esc closes the bag before the conversation under it.
fn close_bag(me: &mut Player) {
    me.act(InputAction::ToggleInventory);
    me.wait_until("the bag to close", |snapshot| {
        UiTarget::text("Inventory").find(snapshot).is_none()
    });
}

// The ware's row in the list, rather than its name repeated on the details beside it.
fn row_x(me: &mut Player, ware: &str) -> f32 {
    let seen = me.settled();
    UiTarget::button(ware)
        .find(&seen)
        .unwrap_or_else(|| panic!("{ware} is not listed"))
        .rect
        .min_x()
}
