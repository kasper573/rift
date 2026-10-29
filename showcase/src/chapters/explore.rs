use driver::{UiTarget, content, holding, marker_at, pictured_ground, props_near, resident_home};
use engine::core::math::{Offset, Pos};
use engine::core::units::Meters;
use engine::systems::actor::Action;
use engine::systems::input::map::InputAction;
use engine::systems::script::Place;

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "explore",
    title: "Explore",
    summary: "Point and click to roam the world",
    play,
};

const ROUTE: [((f32, f32), &str); 4] = [
    (
        (4.0, 1.0),
        "Click the ground to walk there — the camera follows, and the grass parts underfoot",
    ),
    ((5.0, -2.0), "Paths route around anything in the way"),
    ((-3.0, -4.0), "Beyond town the world is open to explore"),
    (
        (0.0, 0.0),
        "Town is safe ground: monsters roam the wilds beyond it, and fight back",
    ),
];

const SWORD: &str = "rusty_sword";

/// Round the cottage's west gable from where the woodsman stands: the way behind it that keeps
/// off its front door.
fn north_of_the_cottage() -> Pos<Meters> {
    resident_home("grisha").at + Offset::new(-1.0, -7.5)
}

fn play(shoot: &mut Shoot) {
    let player = shoot.join_as("swordsman", None);
    let mut me = shoot.roll(player);
    let spawn = me.come_to_rest();
    for ((x, y), says) in ROUTE {
        me.caption(says);
        me.walk_near(spawn + Offset::new(x, y));
    }
    me.caption(
        "Your pace picks the gait whose clip covers ground nearest it, played as fast as the ground it covers, so feet keep to the grass instead of sliding",
    );
    for _ in 0..2 {
        me.tilt_camera(InputAction::TiltUp);
    }
    me.point_at_ground(spawn + Offset::new(4.0, 1.0));
    me.click();
    me.wait_for(
        "a gait played as fast as the ground it covers",
        |snapshot| {
            snapshot
                .me
                .as_ref()
                .filter(|me| Action::GAITS.contains(&me.action) && me.rate.0 != 1.0)
                .map(|_| ())
        },
    );
    me.come_to_rest();
    me.walk_near(spawn);
    me.close_in();
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

    let here = Place {
        area: me.snapshot().area.expect("an area"),
        at: spawn,
    };
    me.caption(
        "Each ground can lay a picture of its own over the land, repeating under its colour",
    );
    me.travel_to(|_| Some(pictured_ground(here)), |_| false);
    me.hold();
    me.caption("Plants sway in the wind, and the sun glows through leaves that let it in");
    let leafy = props_near(here, |prop| {
        prop.materials
            .values()
            .any(|material| material.get(content()).translucency > 0.0)
    });
    let plant = *leafy
        .first()
        .expect("a plant whose leaves let the light in");
    me.travel_to(|_| Some(plant + Offset::new(0.0, 3.0)), |_| false);
    me.hold();

    me.caption("A path leads to the woodsman's cottage");
    me.travel_to(
        |_| Some(resident_home("grisha").at + Offset::new(2.0, 2.0)),
        |_| false,
    );
    me.hold();
    me.caption("Behind the cottage rises a standing stone");
    me.walk_near(north_of_the_cottage());
    me.travel_to(
        |_| Some(marker_at("standing-stone").at + Offset::new(0.0, 3.0)),
        |_| false,
    );
    me.hold();
    me.caption("The sun turns through the day as you play, and the light with it");
    me.hold();
}
