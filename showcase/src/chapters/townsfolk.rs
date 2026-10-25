use driver::{
    Player, UiTarget, content, holding, input_name, object_named, resident_home, townsperson,
};
use engine::core::math::{Offset, Pos};
use engine::core::units::Meters;
use engine::systems::input::gestures::ClickClaim;
use engine::systems::input::map::InputAction;
use engine::systems::probe::ProbeSnapshot;

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "townsfolk",
    title: "Townsfolk",
    summary: "Who lives on the island, and what they have for you",
    play,
};

const SWORD: &str = "rusty_sword";

fn play(shoot: &mut Shoot) {
    let player = shoot.join_as("swordsman", Some(resident_home("island", "mara")));
    let mut me = shoot.roll(player);
    me.caption(&format!(
        "Loot at a townsperson's feet? {} picks it up before it talks",
        input_name(InputAction::Interact)
    ));
    let host = me.wait_for("Mara", |snapshot| townsperson(snapshot, "mara").cloned());
    me.caption(&format!(
        "You arrived right where {} stands: drop a sword at her feet",
        host.name
    ));
    me.act(InputAction::ToggleInventory);
    me.act_on_ui(InputAction::DropItem, UiTarget::item(SWORD));
    me.wait_until("the sword to drop", |snapshot| {
        holding(snapshot, "rusty_sword") == 0
    });
    me.act(InputAction::ToggleInventory);
    walk_below(&mut me, |_| Some(host.at));
    let sword = me.wait_for("the sword on the ground", |snapshot| {
        snapshot
            .items
            .iter()
            .find(|item| item.item.name(content()) == "rusty_sword")
            .cloned()
    });
    let under_foot = sword.at - Offset::new(0.0, 0.3);
    let pointed = me.point_at_ground(under_foot);
    assert_eq!(
        pointed.claim,
        ClickClaim::Pickup(sword.id),
        "the sword under her feet takes the click"
    );
    me.hold();
    me.click();
    me.wait_until(
        "the sword back in the bag, with no conversation",
        |snapshot| holding(snapshot, "rusty_sword") == 1 && snapshot.stage.is_none(),
    );

    me.caption("Eight townsfolk live on the island — name plates show who they are");
    walk_below(&mut me, |snapshot| {
        townsperson(snapshot, "mara").map(|mara| mara.at)
    });
    me.caption("An icon over a head is the most important thing they have for you");
    me.caption("Things on the map can be used too: a notice board, a chest by the tide");
    me.approach("tide-chest");
    walk_below(&mut me, |snapshot| {
        object_named(snapshot, "tide-chest").map(|chest| chest.at)
    });
    let chest = me.wait_for("the tide chest", |snapshot| {
        object_named(snapshot, "tide-chest").cloned()
    });
    me.point_at_ground(chest.aim);
    me.hold();
    me.approach("notice-board");
    let board = me.wait_for("the notice board", |snapshot| {
        object_named(snapshot, "notice-board").cloned()
    });
    walk_below(&mut me, |_| Some(board.at));
    me.point_at_ground(board.aim);
    me.hold();
}

// Clear of what it stands below, so the click walks there instead of using it.
fn walk_below(me: &mut Player, target: impl Fn(&ProbeSnapshot) -> Option<Pos<Meters>>) {
    me.walk_toward(|snapshot| target(snapshot).map(|at| at + Offset::new(0.0, 2.5)));
}
