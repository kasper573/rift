use driver::{UiTarget, aiming, blocked, cooling, holding, pressed, resident_home, slot_item};
use engine::core::math::Offset;
use engine::systems::input::map::InputAction;
use engine::systems::skill::bar::HotbarPress;

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "hotbar",
    title: "Hotbar",
    summary: "Nine keys for skills and items: cooldowns, refusals, aiming, quick cast and a bar of any shape",
    play,
};

const POTION: &str = "icons/potion/red_potion.png";

fn play(shoot: &mut Shoot) {
    let player = shoot.join_as(
        "armsman",
        Some(resident_home("glimmercap", "gnome_alchemist")),
    );
    let mut me = shoot.roll(player);
    me.caption_top("The hotbar: nine slots on the keys 1 to 9, holding skills and items alike");
    me.hover_ui(UiTarget::starting("Hotbar slot 1"));
    me.hold();
    me.caption_top(
        "Press a slot's key: a skill on yourself fires at once, and its cooldown drains off the slot",
    );
    me.act(InputAction::Hotbar2);
    me.wait_until("Guard cooling", |snapshot| {
        cooling(snapshot, 1) && pressed(snapshot, 1) == Some(HotbarPress::Used)
    });
    me.hold();
    me.caption_top("Every press ripples its slot: blue when it fired, red when it could not");
    me.act(InputAction::Hotbar2);
    me.wait_until("the press refused", |snapshot| {
        pressed(snapshot, 1) == Some(HotbarPress::Denied)
    });
    me.hold();
    me.caption_top(
        "A slot whose skill you cannot use stays filled: Throw Knife wants a knife you don't carry",
    );
    me.hover_ui(UiTarget::starting("Hotbar slot 4"));
    me.wait_until("Throw Knife blocked", |snapshot| blocked(snapshot, 3));
    me.hold();
    me.act(InputAction::Hotbar4);
    me.wait_until("the knife refused", |snapshot| {
        pressed(snapshot, 3) == Some(HotbarPress::Denied)
    });
    me.caption_top(
        "An area skill is aimed: its shape follows the pointer over the ground, and a click casts it",
    );
    me.act(InputAction::Hotbar1);
    me.wait_until("Cleave armed", aiming);
    let at = me.snapshot().me.expect("the player").at + Offset::new(2.0, 0.0);
    me.point_at_tile(at);
    me.hold();
    me.click();
    me.wait_until("Cleave cast", |snapshot| cooling(snapshot, 0));
    me.hold();
    me.caption_top(
        "Quick cast, in Settings, skips the aiming: the press casts at once, at where the pointer rests",
    );
    me.act(InputAction::ToggleSettings);
    me.click_ui(UiTarget::text("Quick cast skills"));
    me.act(InputAction::ToggleSettings);
    me.wait_until("Cleave ready", |snapshot| !cooling(snapshot, 0));
    me.point_at_tile(at);
    me.act(InputAction::Hotbar1);
    me.wait_until("Cleave cast at once", |snapshot| {
        cooling(snapshot, 0) && !aiming(snapshot)
    });
    me.caption_top(
        "A skill for one target wants someone under the pointer: Sunder over bare ground is refused",
    );
    me.point_at_tile(at);
    me.act(InputAction::Hotbar3);
    me.wait_until("Sunder refused", |snapshot| {
        pressed(snapshot, 2) == Some(HotbarPress::Denied)
    });
    me.hold();
    me.caption_top("Items go on the bar too: shift-click one in the bag. Its count stays live");
    me.act(InputAction::ToggleInventory);
    me.act_on_ui(InputAction::PinToHotbar, UiTarget::image(POTION));
    me.wait_until("the potion on the bar", |snapshot| {
        slot_item(snapshot, 4) == Some("health_potion")
    });
    me.act(InputAction::ToggleInventory);
    me.act(InputAction::Hotbar5);
    me.wait_until("a potion drunk", |snapshot| {
        holding(snapshot, "health_potion") == 1
    });
    me.caption_top("Right-click a slot to rebind its key, or to clear it");
    me.act_on_ui(
        InputAction::InspectItem,
        UiTarget::starting("Hotbar slot 2"),
    );
    me.click_ui(UiTarget::button("Rebind key"));
    me.wait_until("the slot waiting for a key", |snapshot| {
        snapshot.hotbar.get(1).is_some_and(|slot| slot.rebinding)
    });
    me.hold();
    me.type_text("q", |snapshot| {
        snapshot
            .hotbar
            .get(1)
            .is_some_and(|slot| slot.key.as_deref() == Some("Q"))
    });
    me.caption_top("The slot's tooltip names its key");
    me.hover_ui(UiTarget::starting("Hotbar slot 2"));
    me.hold();
    me.act_on_ui(
        InputAction::InspectItem,
        UiTarget::starting("Hotbar slot 5"),
    );
    me.click_ui(UiTarget::button("Clear slot"));
    me.wait_until("slot 5 cleared", |snapshot| {
        slot_item(snapshot, 4).is_none()
    });
    me.caption_top("The bar is a grid of any shape: pull its corner, and it snaps to whole slots");
    me.drag_ui("Resize", Offset::new(-200.0, -44.0));
    me.wait_until("another shape", |snapshot| snapshot.hotbar.len() != 9);
    me.hold();
    me.drag_ui("Resize", Offset::new(200.0, 44.0));
    me.wait_until("one row of nine", |snapshot| snapshot.hotbar.len() == 9);
    me.caption_top(
        "Left of the bar sits the basic attack slot: a click on an enemy uses it, a weapon that grants one fills it, and no press ripples it",
    );
    me.hover_ui(UiTarget::starting("Basic attack slot"));
    me.hold();
}
