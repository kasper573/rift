use driver::{UiTarget, aiming, blocked, cooling, holding, pressed, resident_home, slot_item};
use engine::core::math::Offset;
use engine::systems::input::map::InputAction;
use engine::systems::skill::bar::HotbarPress;

use crate::playtest;

const POTION: &str = "icons/potion/red_potion.png";

#[test]
fn the_hotbar_fires_refuses_aims_quick_casts_and_takes_items() {
    let mut playtest = playtest();
    let mut player = playtest.join_as(
        "armsman",
        Some(resident_home("glimmercap", "gnome_alchemist")),
    );
    player.in_world();

    player.act(InputAction::Hotbar2);
    player.wait_until("Guard cooling", |snapshot| {
        cooling(snapshot, 1) && pressed(snapshot, 1) == Some(HotbarPress::Used)
    });
    player.act(InputAction::Hotbar2);
    player.wait_until("the press refused", |snapshot| {
        pressed(snapshot, 1) == Some(HotbarPress::Denied)
    });

    player.wait_until("Throw Knife blocked without a knife", |snapshot| {
        blocked(snapshot, 3)
    });
    player.act(InputAction::Hotbar4);
    player.wait_until("the knife refused", |snapshot| {
        pressed(snapshot, 3) == Some(HotbarPress::Denied)
    });

    player.act(InputAction::Hotbar1);
    player.wait_until("Cleave armed", aiming);
    let at = player.snapshot().me.expect("the player").at + Offset::new(2.0, 0.0);
    player.point_at_tile(at);
    player.click();
    player.wait_until("Cleave cast", |snapshot| {
        cooling(snapshot, 0) && !aiming(snapshot)
    });
    player.act(InputAction::ToggleSettings);
    player.click_ui(UiTarget::text("Quick cast skills"));
    player.act(InputAction::ToggleSettings);
    player.wait_until("Cleave ready", |snapshot| !cooling(snapshot, 0));
    player.point_at_tile(at);
    player.act(InputAction::Hotbar1);
    player.wait_until("Cleave cast at once", |snapshot| {
        cooling(snapshot, 0) && !aiming(snapshot)
    });
    player.point_at_tile(at);
    player.act(InputAction::Hotbar3);
    player.wait_until("Sunder refused over bare ground", |snapshot| {
        pressed(snapshot, 2) == Some(HotbarPress::Denied)
    });
    assert!(!cooling(&player.snapshot(), 2));

    player.act(InputAction::ToggleInventory);
    player.act_on_ui(InputAction::PinToHotbar, UiTarget::image(POTION));
    player.wait_until("the potion in the first free slot", |snapshot| {
        slot_item(snapshot, 4) == Some("health_potion")
    });
    player.act(InputAction::ToggleInventory);
    player.act(InputAction::Hotbar5);
    player.wait_until("a potion drunk", |snapshot| {
        holding(snapshot, "health_potion") == 1
    });

    player.act_on_ui(
        InputAction::InspectItem,
        UiTarget::starting("Hotbar slot 2"),
    );
    player.click_ui(UiTarget::button("Rebind key"));
    player.wait_until("the slot waiting for a key", |snapshot| {
        snapshot.hotbar.get(1).is_some_and(|slot| slot.rebinding)
    });
    player.type_text("q", |snapshot| {
        snapshot
            .hotbar
            .get(1)
            .is_some_and(|slot| slot.key.as_deref() == Some("Q"))
    });
    player.act_on_ui(
        InputAction::InspectItem,
        UiTarget::starting("Hotbar slot 5"),
    );
    player.click_ui(UiTarget::button("Clear slot"));
    player.wait_until("slot 5 cleared", |snapshot| {
        slot_item(snapshot, 4).is_none()
    });

    player.drag_ui("Resize", Offset::new(-200.0, -44.0));
    player.wait_until("another shape", |snapshot| snapshot.hotbar.len() == 8);
    player.drag_ui("Resize", Offset::new(200.0, 44.0));
    player.wait_until("one row of nine", |snapshot| snapshot.hotbar.len() == 9);
}
