use driver::{UiTarget, content, holding, marker_at, resident_home};
use engine::core::math::{Offset, Pos};
use engine::core::units::Meters;
use engine::systems::input::map::InputAction;

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "explore",
    title: "Explore",
    summary: "Point and click to roam the island",
    play,
};

const ROUTE: [((f32, f32), &str); 4] = [
    (
        (4.0, 1.0),
        "Click the ground to walk there — the camera follows, and the grass parts underfoot",
    ),
    (
        (5.0, -2.0),
        "Paths route around the well, the stalls and anything in the way",
    ),
    (
        (-3.0, -4.0),
        "A harbour, a cottage on the rise, dead trees, a bone camp and bat rocks — the island is open to explore",
    ),
    (
        (0.0, 0.0),
        "Town is safe ground: monsters roam the wilds beyond it, and fight back",
    ),
];

const SWORD: &str = "rusty_sword";

/// Round the cottage's west gable: the way behind it that keeps off its front door.
fn north_of_the_cottage() -> Pos<Meters> {
    Pos::new(82.0, 34.0)
}

fn play(shoot: &mut Shoot) {
    let player = shoot.join_as("swordsman", None);
    let mut me = shoot.roll(player);
    let spawn = me.come_to_rest();
    for ((x, y), says) in ROUTE {
        me.caption(says);
        me.walk_near(spawn + Offset::new(x, y));
    }
    me.caption("Things you drop lie on the ground as themselves");
    me.act(InputAction::ToggleInventory);
    me.act_on_ui(InputAction::DropItem, UiTarget::item(SWORD));
    me.wait_until("the sword to drop", |snapshot| {
        holding(snapshot, SWORD) == 0
    });
    me.act(InputAction::ToggleInventory);
    let sword = me.wait_for("the sword on the ground", |snapshot| {
        snapshot
            .items
            .iter()
            .find(|item| item.item.name(content()) == SWORD)
            .cloned()
    });
    me.walk_near(sword.at + Offset::new(0.0, 2.0));
    me.hold();

    me.caption("The path north-east climbs the rise to the woodsman's cottage");
    me.travel_to(
        |_| Some(resident_home("island", "grisha").at + Offset::new(2.0, 2.0)),
        |_| false,
    );
    me.hold();
    me.caption("Behind the cottage a standing stone looks out over the sea");
    me.walk_near(north_of_the_cottage());
    me.travel_to(
        |_| Some(marker_at("island", "standing-stone") + Offset::new(0.0, 3.0)),
        |_| false,
    );
    me.hold();
    me.caption("The sun turns through the day as you play, and the light with it");
    me.hold();
}
