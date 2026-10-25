use driver::{UiTarget, finished, holding, input_name, noticed, on_quest};
use engine::core::units::Meters;
use engine::systems::input::map::InputAction;
use engine::systems::quest::QuestResult;

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "quest-choices",
    title: "Quest choices",
    summary: "A quest found in a chest, a place to explore, and a choice you can't take back",
    play,
};

const MAP: &str = "tattered_map";

fn play(shoot: &mut Shoot) {
    let player = shoot.join_as("bones_midway", None);
    let mut me = shoot.roll(player);
    me.caption("The tide chest hides more than gold");
    me.interact_with("tide-chest");
    me.on_stage("map");
    me.read_to_choices();
    me.pick("Close the lid.");
    me.conversation_over();
    me.wait_until("the map in the bag", |snapshot| {
        holding(snapshot, "tattered_map") == 1
    });

    me.caption(&format!(
        "The map begins a quest — {} it in your bag to use it",
        input_name(InputAction::UseItem)
    ));
    me.act(InputAction::ToggleInventory);
    me.hover_ui(UiTarget::item(MAP));
    me.hold();
    me.act_on_ui(InputAction::UseItem, UiTarget::item(MAP));
    me.on_stage("offer");
    me.caption("No one to talk to: just you, and the map");
    me.read_to_choices();
    me.hold();
    me.pick("Accept");
    me.wait_until("the map's quest", |snapshot| {
        on_quest(snapshot, "x_marks_the_spot").is_some()
    });
    me.act(InputAction::ToggleInventory);

    me.caption("The cross is on the rise behind the woodsman's cottage");

    me.caption("Finding the standing stone completes the objective");
    me.walk_into("standing-stone", Meters(4.0));
    me.interact_with("standing-stone");
    me.wait_until("the stone found", |snapshot| {
        on_quest(snapshot, "x_marks_the_spot").is_some_and(|quest| quest.ready)
    });
    me.on_stage("matches");
    me.caption("A map object takes the hand-in, like any townsperson");
    me.read_to_choices();
    me.hold();
    me.pick("Dig where the cross says.");
    me.wait_until("the cross dug up", |snapshot| {
        finished(snapshot, "x_marks_the_spot") == Some(QuestResult::Completed)
    });

    me.caption("The warriors' cairn among the dead trees knows about Ilsa's errand");
    me.walk_into("old-cairn", Meters(3.0));
    me.interact_with("old-cairn");
    me.read_to_choices();
    me.pick("The Fallen's Plea");
    me.on_stage("plea_offer");
    me.read_to_choices();
    me.pick("I'm listening.");
    me.on_stage("plea_answer");
    me.caption(
        "Both answers are permanent, so this one shows everything: what it needs, what it costs, and a warning",
    );
    me.read_to_choices();
    me.hover_choice("Lay the bones to rest.");
    me.hold();
    me.pick("Lay the bones to rest.");
    me.on_stage("grateful");
    me.wait_until("Ilsa's quest to fail", |snapshot| {
        finished(snapshot, "bones_for_the_warden") == Some(QuestResult::Failed)
    });
    me.wait_until("the failed quest in the feed", |snapshot| {
        noticed(snapshot, "Quest failed · Bones for the Warden")
    });
    me.caption("Ilsa's quest fails, in red over your head, and the choice is remembered");
    me.read_to_choices();
    me.pick("Sleep well.");
    me.conversation_over();

    me.caption("Now the cairn greets a friend, with work of its own");
    me.interact_with("old-cairn");
    me.on_stage("friend");
    me.read_to_choices();
    me.hold();
    me.act(InputAction::Dismiss);
    me.conversation_over();
}
