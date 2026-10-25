use driver::{Player, UiTarget, id};
use engine::core::math::Offset;
use engine::systems::area::{AreaDef, AreaId};
use engine::systems::input::map::InputAction;

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "travel",
    title: "Travel",
    summary: "Warps connect the areas of the world, and each crossing plays a transition",
    play,
};

const TRANSITIONS: [(&str, &str); 6] = [
    (
        "Tile wave",
        "Tile wave: the tiles shrink to diamonds around you, and grow back where you land",
    ),
    (
        "Sweep",
        "Sweep: bands carry the old area off the way you walk, and bring the next in behind them",
    ),
    (
        "Iris",
        "Iris: a spotlight closes on you, and opens again where you arrive",
    ),
    (
        "Mosaic",
        "Mosaic: the area coarsens into blocks, steps over to the next, and resolves",
    ),
    (
        "Dither dissolve",
        "Dither dissolve: the area fizzles out pixel by pixel, and the next fizzles in around you",
    ),
    ("Fade", "Fade: plain and quick"),
];

fn play(shoot: &mut Shoot) {
    let player = shoot.join_as("traveler", None);
    let mut me = shoot.roll(player);
    me.caption("Warps lead to other areas — step onto one to cross");
    let home = id::<AreaDef>("island");
    let cottage = id::<AreaDef>("cottage");
    me.caption(
        "The woodsman's cottage stands on the rise north-east of town: its front door is a warp",
    );
    me.show_mixer();
    me.cross("front-door");
    me.caption("The island crumbles away, and the cottage rises around you: its own fire and ambience crossfade in over the town's");
    me.walk_by(Offset::new(-2.0, -1.0));
    me.walk_by(Offset::new(2.0, 0.0));
    me.hold();
    me.hide_mixer();
    me.caption("The door takes you back out, and you arrive beside it");
    cross_into(&mut me, home);
    me.hold();

    me.caption("Settings choose how a crossing looks");
    let mut current = "Crumble";
    for (style, says) in TRANSITIONS {
        choose_transition(&mut me, current, style);
        current = style;
        me.caption(says);
        cross_into(&mut me, cottage);
        cross_into(&mut me, home);
    }
}

fn choose_transition(me: &mut Player, current: &str, next: &str) {
    me.act(InputAction::ToggleSettings);
    me.scroll_until("Audio", current);
    me.click_ui(current);
    me.hover_ui(next);
    me.click_ui(next);
    me.wait_until(&format!("{next} to be picked"), |snapshot| {
        UiTarget::text(next).find(snapshot).is_some()
            && UiTarget::text(current).find(snapshot).is_none()
    });
    me.act(InputAction::ToggleSettings);
    me.wait_until("the settings to close", |snapshot| {
        UiTarget::text(next).find(snapshot).is_none()
    });
}

// Through the nearest warp that leads there.
fn cross_into(me: &mut Player, area: AreaId) {
    let seen = me.snapshot();
    let here = seen.me.as_ref().expect("the player").at;
    let warp = seen
        .warps
        .iter()
        .filter(|portal| portal.to == area)
        .min_by(|a, b| a.at.distance_to(here).total_cmp(&b.at.distance_to(here)))
        .expect("a warp there")
        .name
        .clone();
    me.cross(&warp);
}
