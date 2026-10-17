use driver::{LONG, UiTarget, aiming, holding, played, resident_home, status_on_me};
use engine::systems::input::map::InputAction;
use engine::systems::status::StatusCategory;

use crate::playtest;

const BOG_WATER: &str = "icons/potion/water_bottle.png";
const ANTIDOTE: &str = "icons/potion/blue_potion.png";

#[test]
fn statuses_come_from_items_skills_and_gear_and_go_by_category_or_time() {
    let mut playtest = playtest();
    let mut player = playtest.join_as(
        "arcanist",
        Some(resident_home("glimmercap", "gnome_alchemist")),
    );
    player.in_world();

    player.act(InputAction::ToggleInventory);
    player.act_on_ui(InputAction::UseItem, UiTarget::image(BOG_WATER));
    let poisoned = player.wait_until("poisoned", |snapshot| status_on_me(snapshot, "poisoned"));
    assert_eq!(holding(&poisoned, "bog_water"), 1);
    assert!(
        poisoned
            .statuses
            .iter()
            .any(|status| status.effect == "poisoned" && status.category == StatusCategory::Debuff)
    );
    player.act(InputAction::ToggleInventory);
    let full = poisoned.me.expect("the player").health;
    player.wait_until("the poison to bite", |snapshot| {
        snapshot.me.as_ref().is_some_and(|me| me.health < full)
    });
    player.find_ui(UiTarget::starting("Status Poisoned"));

    player.act(InputAction::Hotbar6);
    let warded = player.wait_until("warded", |snapshot| status_on_me(snapshot, "warded"));
    assert!(
        warded
            .statuses
            .iter()
            .any(|status| status.effect == "warded" && status.category == StatusCategory::Buff)
    );
    player.find_ui(UiTarget::starting("Status Warded"));
    assert!(played(&warded, "ward_bubble").len() + played(&warded, "ward_up").len() >= 1);

    player.act(InputAction::ToggleInventory);
    player.act_on_ui(InputAction::UseItem, UiTarget::image(ANTIDOTE));
    player.wait_until("cleansed", |snapshot| {
        !status_on_me(snapshot, "poisoned")
            && status_on_me(snapshot, "warded")
            && holding(snapshot, "antidote") == 0
    });
    player.act(InputAction::ToggleInventory);

    player.act(InputAction::ToggleEquipment);
    player.act_on_ui(
        InputAction::InspectItem,
        UiTarget::starting("Worn as Weapon"),
    );
    player.find_ui(UiTarget::containing("+2 Damage"));
    player.act(InputAction::Dismiss);
    player.act(InputAction::ToggleEquipment);

    player.act(InputAction::Hotbar5);
    player.wait_until("Sanctuary armed", aiming);
    let here = player.snapshot().me.expect("the player").at;
    player.point_at_tile(here);
    player.click();
    player.wait_until("sanctified", |snapshot| {
        status_on_me(snapshot, "sanctified")
    });
    let bitten = player.snapshot().me.expect("the player").health;
    player.wait_until("the blessing to heal", |snapshot| {
        snapshot.me.as_ref().is_some_and(|me| me.health > bitten)
    });

    player.wait_within("the ward broken", LONG, |snapshot| {
        (!status_on_me(snapshot, "warded")).then_some(())
    });
}
