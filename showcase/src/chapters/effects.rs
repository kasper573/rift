use driver::{UiTarget, aiming, holding, resident_home, status_on_me};
use engine::systems::input::map::InputAction;

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "effects",
    title: "Status effects",
    summary: "Buffs and debuffs from items, skills and gear: what they do, how they look, and how they end",
    play,
};

const BOG_WATER: &str = "icons/potion/water_bottle.png";
const ANTIDOTE: &str = "icons/potion/blue_potion.png";

fn play(shoot: &mut Shoot) {
    let player = shoot.join_as(
        "arcanist",
        Some(resident_home("glimmercap", "gnome_alchemist")),
    );
    let mut me = shoot.roll(player);
    me.caption_top(
        "Buffs and debuffs are status effects. Bog water is a bad idea: Poisoned bites every two seconds",
    );
    me.act(InputAction::ToggleInventory);
    me.act_on_ui(InputAction::UseItem, UiTarget::image(BOG_WATER));
    me.wait_until("poisoned", |snapshot| status_on_me(snapshot, "poisoned"));
    me.act(InputAction::ToggleInventory);
    let full = me.snapshot().me.expect("the player").health;
    me.wait_until("the poison to bite", |snapshot| {
        snapshot.me.as_ref().is_some_and(|me| me.health < full)
    });
    me.caption_top(
        "Whatever is on you lines up under your character info, ringed red for a debuff. Rest on one for its card",
    );
    me.hover_ui(UiTarget::starting("Status Poisoned"));
    me.hold();
    me.caption_top(
        "Gear holds its boosts while it is worn, and lets go the moment it comes off: the wand's card lists them",
    );
    me.act(InputAction::ToggleEquipment);
    me.act_on_ui(
        InputAction::InspectItem,
        UiTarget::starting("Worn as Weapon"),
    );
    me.find_ui(UiTarget::containing("+2 Damage"));
    me.hold();
    me.act(InputAction::Dismiss);
    me.act(InputAction::ToggleEquipment);
    me.caption_top(
        "Skills apply them as well: Ward is a buff, ringed blue, and wraps your sprite in a shimmering outline for as long as it holds",
    );
    me.act(InputAction::Hotbar6);
    me.wait_until("warded", |snapshot| status_on_me(snapshot, "warded"));
    me.hover_ui(UiTarget::starting("Status Warded"));
    me.hold();
    me.caption_top(
        "Buff or debuff is only a label, but content can filter by it: the antidote clears every debuff and spares the ward",
    );
    me.act(InputAction::ToggleInventory);
    me.act_on_ui(InputAction::UseItem, UiTarget::image(ANTIDOTE));
    me.wait_until("cleansed", |snapshot| {
        !status_on_me(snapshot, "poisoned")
            && status_on_me(snapshot, "warded")
            && holding(snapshot, "antidote") == 0
    });
    me.act(InputAction::ToggleInventory);
    me.hold();
    me.caption_top(
        "A status can act on a clock: Sanctuary's blessing heals whoever stands in it every two seconds",
    );
    me.act(InputAction::Hotbar5);
    me.wait_until("Sanctuary armed", aiming);
    let here = me.snapshot().me.expect("the player").at;
    me.point_at_tile(here);
    me.click();
    me.wait_until("sanctified", |snapshot| {
        status_on_me(snapshot, "sanctified")
    });
    let bitten = me.snapshot().me.expect("the player").health;
    me.wait_until("the blessing to heal", |snapshot| {
        snapshot.me.as_ref().is_some_and(|me| me.health > bitten)
    });
    me.hold();
    me.caption_top("Time runs out on its own: the ward breaks, and its icon goes with it");
    me.wait_until("the ward broken", |snapshot| {
        !status_on_me(snapshot, "warded")
    });
    me.hold();
}
