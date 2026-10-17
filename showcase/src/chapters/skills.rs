use driver::{
    UiTarget, bare_spot, holding, learned, planned, reset_prompted, resident_home, slot_skill,
    to_spend, tree_view,
};
use engine::core::math::{Offset, Pos};
use engine::systems::input::map::InputAction;
use engine::systems::skill::HotbarEntry;

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "skills",
    title: "Skill trees",
    summary: "Spend your class's points in its tree, actives and passives alike, and win them back",
    play,
};

const DRAUGHT: &str = "icons/potion/green_potion_3.png";

fn play(shoot: &mut Shoot) {
    let player = shoot.join_as(
        "unspent",
        Some(resident_home("glimmercap", "gnome_alchemist")),
    );
    let mut me = shoot.roll(player);
    me.caption_top("Your class grants skill points as you level: P opens your skills");
    me.act(InputAction::ToggleSkills);
    me.find_ui("Skills");
    me.find_ui(UiTarget::containing("to spend"));
    me.caption_top(
        "The tree is laid out as the designer drew it, over the class's own art; arrows lead from a node to what it unlocks",
    );
    me.hold();
    me.caption_top(
        "Rest the pointer on a node for its card: every rank, its cost, and what it needs",
    );
    me.hover_ui(UiTarget::starting("Cleave node"));
    me.hold();
    me.caption_top("A locked node shows the crosses still to clear");
    me.hover_ui(UiTarget::starting("War Cry node"));
    me.hold();
    me.caption_top(
        "Click a node to pick a rank; the pill counts it. Confirm spends the points for good",
    );
    me.click_ui(UiTarget::starting("Cleave node"));
    me.click_ui(UiTarget::starting("Guard node"));
    me.wait_until("two picks", |snapshot| planned(snapshot) == 2);
    me.hold();
    me.click_ui(UiTarget::starting("Confirm"));
    me.wait_until("the ranks learned", |snapshot| {
        learned(snapshot, "cleave") == 1 && learned(snapshot, "guard") == 1
    });
    me.caption_top("Learned. Second Wind, which needed both, is open now, and one point is left");
    me.hold();
    me.caption_top(
        "Skills come in two kinds. Toughness is passive: nothing to cast, it simply holds while it is known",
    );
    me.hover_ui(UiTarget::starting("Toughness node"));
    me.hold();
    me.click_ui(UiTarget::starting("Toughness node"));
    me.click_ui(UiTarget::starting("Confirm"));
    me.wait_until("Toughness learned", |snapshot| {
        learned(snapshot, "toughness") == 1
    });
    me.caption_top("Drag a learned active onto the hotbar…");
    me.carry_onto(
        UiTarget::starting("Cleave node"),
        UiTarget::starting("Hotbar slot 1"),
    );
    me.wait_until("Cleave in slot 1", |snapshot| {
        slot_skill(snapshot, 0) == Some("cleave")
    });
    me.caption_top("…or shift-click it to land in the first free slot");
    me.act_on_ui(InputAction::PinToHotbar, UiTarget::starting("Guard node"));
    me.wait_until("Guard in slot 2", |snapshot| {
        slot_skill(snapshot, 1) == Some("guard")
    });
    me.caption_top(
        "A passive never goes on the bar: drag Toughness onto a slot and the slot stays empty",
    );
    me.carry_onto(
        UiTarget::starting("Toughness node"),
        UiTarget::starting("Hotbar slot 3"),
    );
    me.hold();
    let seen = me.snapshot();
    assert_eq!(
        seen.hotbar.get(2).map(|slot| slot.entry),
        Some(HotbarEntry::Empty),
        "a passive landed on the bar"
    );
    me.caption_top("A tree larger than its window pans by dragging, and never past the art's edge");
    let view = tree_view(&seen).expect("the skills window").clone();
    let grab = bare_spot(&seen, view.viewport).expect("a bare spot of the tree");
    me.drag_from(grab, Offset::new(-140.0, -90.0));
    me.wait_until("the tree panned", |snapshot| {
        tree_view(snapshot).is_some_and(|now| now.panned.x > view.panned.x)
    });
    me.hold();
    me.caption_top("Pull the window's corner as far as you like: it stops where the tree ends");
    me.drag_ui("Skills", Pos::new(20.0, 20.0) - view.window.origin);
    let moved = me.wait_until("the window moved", |snapshot| {
        tree_view(snapshot).is_some_and(|view| view.window.origin.x < 40.0)
    });
    let before = tree_view(&moved).expect("the skills window").window;
    let corner = before.max() - Offset::new(4.0, 4.0);
    let screen = moved.screen.expect("the screen");
    me.drag_from(
        corner,
        Pos::new(screen.width - 2.0, screen.height - 2.0) - corner,
    );
    me.wait_until("the window at its largest", |snapshot| {
        tree_view(snapshot).is_some_and(|view| view.window.size.width > before.size.width + 100.0)
    });
    me.hold();
    me.act(InputAction::ToggleSkills);
    me.caption_top(
        "A reset is content's to offer, here a draught. The game always asks first, and Keep is the default",
    );
    me.act(InputAction::ToggleInventory);
    me.act_on_ui(InputAction::UseItem, UiTarget::image(DRAUGHT));
    me.wait_until("the question", reset_prompted);
    me.hold();
    me.click_ui(UiTarget::button("Reset"));
    me.wait_until("the points back", |snapshot| {
        learned(snapshot, "cleave") == 0
            && to_spend(snapshot) == 3
            && holding(snapshot, "shamans_draught") == 0
    });
    me.act(InputAction::ToggleInventory);
    me.caption_top(
        "Every point is back. The slots that held the skills stay, dimmed, until they are learned again",
    );
    me.hover_ui(UiTarget::starting("Hotbar slot 1"));
    me.hold();
}
