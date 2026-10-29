use driver::{
    UiTarget, aiming, blocked, content, cooling, holding, input_name, open_ground_beside, pressed,
    resident_home, slot_item,
};
use engine::core::math::Offset;
use engine::systems::input::map::InputAction;
use engine::systems::probe::ProbeSnapshot;
use engine::systems::skill::bar::HotbarPress;

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "hotbar",
    title: "Hotbar",
    summary: "Nine keys for skills and items: cooldowns, refusals, aiming, quick cast and a bar of any shape",
    play,
};

const POTION: &str = "health_potion";
const SWORD: &str = "iron_sword";

fn play(shoot: &mut Shoot) {
    let player = shoot.join_as("armsman", Some(resident_home("grisha")));
    let mut me = shoot.roll(player);
    me.caption(&format!(
        "The hotbar: nine slots on the keys {} to {}, holding skills and items alike",
        input_name(InputAction::Hotbar1),
        input_name(InputAction::Hotbar9)
    ));
    me.hover_ui(UiTarget::starting("Hotbar slot 1"));
    me.hold();
    me.caption(
        "Press a slot's key: a skill on yourself fires at once, and its cooldown drains off the slot",
    );
    me.act(InputAction::Hotbar2);
    me.wait_until("Guard cooling", |snapshot| {
        cooling(snapshot, 1) && pressed(snapshot, 1) == Some(HotbarPress::Used)
    });
    me.hold();
    me.caption(
        "Every press rings its slot for a moment: gold when it fired, rubric when it could not",
    );
    me.act(InputAction::Hotbar2);
    me.wait_until("the press refused", |snapshot| {
        pressed(snapshot, 1) == Some(HotbarPress::Denied)
    });
    me.hold();
    me.caption(
        "A slot whose skill you cannot use stays filled: Throw Knife wants a knife you don't carry",
    );
    me.hover_ui(UiTarget::starting("Hotbar slot 4"));
    me.wait_until("Throw Knife blocked", |snapshot| blocked(snapshot, 3));
    me.hold();
    me.act(InputAction::Hotbar4);
    me.wait_until("the knife refused", |snapshot| {
        pressed(snapshot, 3) == Some(HotbarPress::Denied)
    });
    me.caption(&format!(
        "An area skill is aimed: its shape follows the pointer over the ground, and {} casts it",
        input_name(InputAction::Interact)
    ));
    me.act(InputAction::Hotbar1);
    me.wait_until("Cleave armed", aiming);
    let at = me.snapshot().me.expect("the player").at + Offset::new(2.0, 0.0);
    me.point_at_ground(at);
    me.hold();
    me.click();
    me.wait_until("Cleave cast", |snapshot| cooling(snapshot, 0));
    me.hold();
    me.caption(
        "Quick cast, in Settings, skips the aiming: the press casts at once, at where the pointer rests",
    );
    me.act(InputAction::ToggleSettings);
    me.click_ui(UiTarget::text("Quick cast skills"));
    me.act(InputAction::ToggleSettings);
    me.wait_until("Cleave ready", |snapshot| !cooling(snapshot, 0));
    me.point_at_ground(at);
    me.act(InputAction::Hotbar1);
    me.wait_until("Cleave cast at once", |snapshot| {
        cooling(snapshot, 0) && !aiming(snapshot)
    });
    me.caption(
        "A skill for one target wants someone under the pointer: Sunder over bare ground is refused",
    );
    let bare = *open_ground_beside(&me.snapshot())
        .first()
        .expect("bare ground in view");
    me.point_at_ground(bare);
    me.act(InputAction::Hotbar3);
    me.wait_until("Sunder refused", |snapshot| {
        pressed(snapshot, 2) == Some(HotbarPress::Denied)
    });
    me.hold();
    me.caption(&format!(
        "Items go on the bar too: {} one in the bag. Its count stays live",
        input_name(InputAction::PinToHotbar)
    ));
    me.act(InputAction::ToggleInventory);
    me.act_on_ui(InputAction::PinToHotbar, UiTarget::item(POTION));
    me.wait_until("the potion on the bar", |snapshot| {
        slot_item(snapshot, 4) == Some("health_potion")
    });
    me.act(InputAction::ToggleInventory);
    me.act(InputAction::Hotbar5);
    me.wait_until("a potion drunk", |snapshot| {
        holding(snapshot, "health_potion") == 1
    });
    me.caption("Each skill or item holds one slot: carry it onto another, and it moves there");
    me.act(InputAction::ToggleInventory);
    me.carry_onto(in_bag(POTION), UiTarget::starting("Hotbar slot 7"));
    me.wait_until("the potion moved to slot 7", |snapshot| {
        slot_item(snapshot, 6) == Some("health_potion") && slot_item(snapshot, 4).is_none()
    });
    me.act(InputAction::ToggleInventory);
    me.caption(&format!(
        "{} a slot to rebind its key, or to clear it",
        input_name(InputAction::SlotOptions)
    ));
    me.act_on_ui(
        InputAction::SlotOptions,
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
    me.caption("The slot's tooltip names its key");
    me.hover_ui(UiTarget::starting("Hotbar slot 2"));
    me.hold();
    me.act_on_ui(
        InputAction::SlotOptions,
        UiTarget::starting("Hotbar slot 7"),
    );
    me.click_ui(UiTarget::button("Clear slot"));
    me.wait_until("slot 7 cleared", |snapshot| {
        slot_item(snapshot, 6).is_none()
    });
    me.caption("The bar is a grid of any shape: pull its corner, and it snaps to whole slots");
    me.drag_ui("Resize", Offset::new(-200.0, -44.0));
    me.wait_until("another shape", |snapshot| snapshot.hotbar.len() != 9);
    me.hold();
    me.drag_ui("Resize", Offset::new(200.0, 44.0));
    me.wait_until("one row of nine", |snapshot| snapshot.hotbar.len() == 9);
    me.caption(&format!(
        "Left of the bar sits the basic attack slot: {} on an enemy uses it, a weapon that grants one fills it, and no press rings it",
        input_name(InputAction::Interact)
    ));
    me.hover_ui(UiTarget::starting("Basic attack slot"));
    me.hold();
    me.caption(&format!(
        "{} empties it; carry the weapon you wear onto it, from Equipment, and its attack is back",
        input_name(InputAction::PinToHotbar)
    ));
    me.act_on_ui(
        InputAction::PinToHotbar,
        UiTarget::starting("Basic attack slot"),
    );
    me.wait_until("the basic attack slot empty", |snapshot| {
        snapshot.basic_attack.is_none()
    });
    me.act(InputAction::ToggleEquipment);
    me.carry_onto(
        UiTarget::starting("Worn as"),
        UiTarget::starting("Basic attack slot"),
    );
    me.wait_until("Slash back in the slot", slashing);
    me.hold();
    me.caption("Carry a weapon onto it from the bag, and you put it on to attack with it");
    me.act_on_ui(InputAction::UseItem, UiTarget::starting("Worn as"));
    me.wait_until("the sword taken off", |snapshot| {
        holding(snapshot, "iron_sword") == 1 && snapshot.basic_attack.is_none()
    });
    me.act(InputAction::ToggleInventory);
    me.carry_onto(in_bag(SWORD), UiTarget::starting("Basic attack slot"));
    me.wait_until("the sword worn, swinging Slash", |snapshot| {
        holding(snapshot, "iron_sword") == 0 && slashing(snapshot)
    });
    me.hold();
}

fn slashing(snapshot: &ProbeSnapshot) -> bool {
    snapshot
        .basic_attack
        .is_some_and(|skill| skill.name(content()) == "slash")
}

fn in_bag(item: &str) -> UiTarget {
    UiTarget::item(item).and("in the bag", |element, snapshot| {
        !snapshot.ui.iter().any(|slot| {
            slot.text.as_deref().is_some_and(|text| {
                text.starts_with("Hotbar slot") || text.starts_with("Basic attack slot")
            }) && slot.rect.contains(element.rect.center())
        })
    })
}
