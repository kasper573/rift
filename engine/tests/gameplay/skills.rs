use driver::{
    UiTarget, bare_spot, holding, id, learned, planned, reset_prompted, resident_home, slot_skill,
    to_spend, tree_view,
};
use engine::core::math::{Offset, Pos};
use engine::systems::input::map::InputAction;
use engine::systems::skill::{HotbarEntry, SkillDef};

use crate::playtest;

const DRAUGHT: &str = "icons/potion/green_potion_3.png";

#[test]
fn skills_are_learned_in_the_tree_pinned_on_the_bar_and_given_back_by_a_reset() {
    let mut playtest = playtest();
    let mut player = playtest.join_as(
        "unspent",
        Some(resident_home("glimmercap", "gnome_alchemist")),
    );

    player.act(InputAction::ToggleSkills);
    player.find_ui("Skills");
    player.click_ui(UiTarget::starting("Cleave node"));
    player.click_ui(UiTarget::starting("Guard node"));
    player.wait_until("two picks", |snapshot| planned(snapshot) == 2);
    player.click_ui(UiTarget::starting("Confirm"));
    player.wait_until("the ranks learned", |snapshot| {
        learned(snapshot, "cleave") == 1
            && learned(snapshot, "guard") == 1
            && to_spend(snapshot) == 1
    });
    player.click_ui(UiTarget::starting("Toughness node"));
    player.click_ui(UiTarget::starting("Confirm"));
    player.wait_until("Toughness learned", |snapshot| {
        learned(snapshot, "toughness") == 1 && to_spend(snapshot) == 0
    });

    player.carry_onto(
        UiTarget::starting("Cleave node"),
        UiTarget::starting("Hotbar slot 1"),
    );
    player.wait_until("Cleave in slot 1", |snapshot| {
        slot_skill(snapshot, 0) == Some("cleave")
    });
    player.act_on_ui(InputAction::PinToHotbar, UiTarget::starting("Guard node"));
    player.wait_until("Guard in slot 2", |snapshot| {
        slot_skill(snapshot, 1) == Some("guard")
    });
    player.carry_onto(
        UiTarget::starting("Toughness node"),
        UiTarget::starting("Hotbar slot 3"),
    );
    player.act_on_ui(
        InputAction::PinToHotbar,
        UiTarget::starting("Toughness node"),
    );
    let opened = player.settled();
    assert_eq!(
        opened.hotbar.get(2).map(|slot| slot.entry),
        Some(HotbarEntry::Empty),
        "a passive never lands on the bar"
    );
    assert!(
        opened
            .hotbar
            .iter()
            .all(|slot| slot.entry != HotbarEntry::Skill(id::<SkillDef>("toughness"))),
        "a passive never lands on the bar: {:?}",
        opened.hotbar
    );

    let view = tree_view(&opened).expect("the skills window");
    assert!(
        view.canvas.width > view.viewport.size.width
            && view.canvas.height > view.viewport.size.height,
        "the tree outgrows its window, so panning has somewhere to go: {view:?}"
    );
    let at_first = view.panned;
    let grab = bare_spot(&opened, view.viewport).expect("a bare spot of the tree");
    player.drag_from(grab, Offset::new(-240.0, -160.0));
    let panned = player.wait_until("the tree panned", |snapshot| {
        tree_view(snapshot)
            .is_some_and(|view| view.panned.x > at_first.x && view.panned.y > at_first.y)
    });
    let view = tree_view(&panned).expect("the skills window");
    assert!(
        view.panned.x <= view.canvas.width - view.viewport.size.width + 0.5
            && view.panned.y <= view.canvas.height - view.viewport.size.height + 0.5,
        "panning stops at the canvas's edge: {view:?}"
    );
    let grab = bare_spot(&panned, view.viewport).expect("a bare spot of the tree");
    let screen = panned.screen.expect("the screen");
    player.drag_from(
        grab,
        Offset::new(
            (view.panned.x + 20.0).min(screen.width - grab.x - 10.0),
            (view.panned.y + 20.0).min(screen.height - grab.y - 10.0),
        ),
    );
    player.wait_until("the tree panned back to its origin", |snapshot| {
        tree_view(snapshot).is_some_and(|view| view.panned.x == 0.0 && view.panned.y == 0.0)
    });
    player.drag_ui("Skills", Pos::new(20.0, 20.0) - view.window.origin);
    let moved = player.wait_until("the window moved", |snapshot| {
        tree_view(snapshot).is_some_and(|view| view.window.origin.x < 40.0)
    });
    let before = tree_view(&moved).expect("the skills window").window;
    let corner = before.max() - Offset::new(4.0, 4.0);
    let screen = moved.screen.expect("the screen");
    let pulled_to = Pos::new(screen.width - 2.0, screen.height - 2.0);
    player.drag_from(corner, pulled_to - corner);
    let grown = player.wait_until("the window grown", |snapshot| {
        tree_view(snapshot).is_some_and(|view| view.window.size.width > before.size.width + 100.0)
    });
    let view = tree_view(&grown).expect("the skills window");
    let max = view.max.expect("a bounded window");
    assert!(
        view.window.size.width <= max.width + 0.5 && view.window.size.height <= max.height + 0.5,
        "the window never outgrows the canvas: {view:?}"
    );
    assert!(
        view.window.max().x < pulled_to.x - 100.0,
        "the window stopped short of where the corner was pulled: {view:?}"
    );
    assert!(
        view.viewport.size.width <= view.canvas.width + 0.5
            && view.viewport.size.height <= view.canvas.height + 0.5,
        "the viewport is always full of canvas: {view:?}"
    );
    player.act(InputAction::ToggleSkills);

    player.act(InputAction::ToggleInventory);
    player.act_on_ui(InputAction::UseItem, UiTarget::image(DRAUGHT));
    player.wait_until("the question", reset_prompted);
    player.click_ui(UiTarget::button("Keep"));
    player.wait_until("the question gone", |snapshot| !reset_prompted(snapshot));
    assert_eq!(learned(&player.snapshot(), "cleave"), 1);
    assert_eq!(holding(&player.snapshot(), "shamans_draught"), 1);
    player.act_on_ui(InputAction::UseItem, UiTarget::image(DRAUGHT));
    player.wait_until("the question", reset_prompted);
    player.click_ui(UiTarget::button("Reset"));
    let snapshot = player.wait_until("the points back", |snapshot| {
        learned(snapshot, "cleave") == 0
            && to_spend(snapshot) == 3
            && holding(snapshot, "shamans_draught") == 0
    });
    assert_eq!(slot_skill(&snapshot, 0), Some("cleave"));
    assert!(snapshot.hotbar[0].dimmed);
}
