use driver::{UiTarget, erred, input_name, input_names, noticed, open_ground_beside};
use engine::systems::input::gestures::ClickClaim;
use engine::systems::input::map::InputAction;
use engine::systems::probe::ProbeSnapshot;

use crate::chapters::Chapter;
use crate::shoot::{Shoot, Star};

pub const CHAPTER: Chapter = Chapter {
    name: "conversation",
    title: "Conversation",
    summary: "Talk to the townsfolk: busts, choices, prices, refusals, history",
    play,
};

fn play(shoot: &mut Shoot) {
    let player = shoot.join_as("talker", None);
    let mut me = shoot.roll(player);
    me.caption(&format!(
        "{} a townsperson to talk — you walk into reach and the conversation opens",
        input_name(InputAction::Interact)
    ));
    me.talk_to("tobb");
    me.caption(&format!(
        "Busts show who is speaking. {} reads on",
        spoken_list(&input_names(InputAction::Advance))
    ));
    let stage = me.read_to_choices();
    me.caption(&format!(
        "No buttons on the frame: {} leaves, and the choices say the rest",
        input_name(InputAction::Dismiss)
    ));
    me.caption("A choice shows its words and nothing more, unless the writer reveals more");
    me.caption(&format!(
        "{} {} choose and {} picks, and a pointer at rest never takes the choice back",
        input_name(InputAction::ChoosePrevious),
        input_name(InputAction::ChooseNext),
        input_name(InputAction::PickChoice)
    ));
    me.hover_choice(&stage.choices[0].label);
    let catch = stage
        .choices
        .iter()
        .position(|choice| choice.label == "Caught anything good today?")
        .expect("the catch choice");
    for _ in 0..=catch {
        me.act(InputAction::ChooseNext);
    }
    me.act(InputAction::ChoosePrevious);
    me.act(InputAction::PickChoice);
    me.on_stage("catch");
    me.read_to_choices();
    me.caption("…or pick it with the pointer");
    me.pick("Good luck out there.");
    me.on_stage("news");
    me.caption("Tobb held his news while you talked — once you're free, he calls you over");
    me.read_to_choices();
    me.act(InputAction::Choice1);
    me.conversation_over();

    me.caption("While you talk, the map ignores the mouse: a plain pointer, and clicks do nothing");
    me.talk_to("grisha");
    me.come_to_rest();
    let pointed = point_off_stage(&mut me);
    assert_eq!(pointed.claim, ClickClaim::Locked);
    me.click();
    me.read_to_choices();
    me.caption(
        "Over the conversation the cursor is its own: an arrow on the words, a hand on the choices",
    );
    me.hover_ui(UiTarget::containing("Driftwood Inn"));
    me.hover_choice("Chat.");
    point_off_stage(&mut me);
    me.caption("Choices branch into a tree: Chat opens a menu of its own");
    for (pick, next) in [("Chat.", "chat"), ("Tell me about this island.", "island")] {
        me.pick(pick);
        me.on_stage(next);
        me.read_to_choices();
    }
    me.caption("Ok leads back up to the chat menu");
    for (pick, next) in [("Ok.", "chat"), ("Any rumours?", "rumour_wren")] {
        me.pick(pick);
        me.on_stage(next);
        me.read_to_choices();
    }
    me.caption("…and a branch can go deeper still");
    for (pick, next) in [
        ("What does she do with them?", "wren_secret"),
        ("Ok.", "chat"),
    ] {
        me.pick(pick);
        me.on_stage(next);
        me.read_to_choices();
    }
    me.caption("Never mind goes back to the top");
    me.pick("Never mind.");
    me.on_stage("anything_else");
    me.read_to_choices();
    me.caption("An inn has its prices on the wall: Grisha's menu shows what each choice costs");
    me.hover_choice("Buy a round for the room.");
    me.hold();
    me.pick("Buy a round for the room.");
    me.on_stage("round");
    me.wait_until("the round's price in the feed", |snapshot| {
        noticed(snapshot, "−10 Gold")
    });
    me.caption("What it took rises over your head");
    me.read_to_choices();
    me.caption(
        "Can't afford it? Pick it anyway: it turns rubric, an error says why, and you can pick again",
    );
    me.act(InputAction::Choice1);
    me.wait_until("the refused pick to say why", |snapshot| {
        erred(snapshot, "Needs")
    });
    me.caption(&format!(
        "History is a window like any other: {} opens it here too, over the conversation",
        input_name(InputAction::ToggleHistory)
    ));
    me.act(InputAction::ToggleHistory);
    me.wait_until("the history to open", |snapshot| snapshot.history.open);
    me.caption(&format!(
        "{} closes the history, then leaves the conversation",
        input_name(InputAction::Dismiss)
    ));
    me.act(InputAction::Dismiss);
    me.wait_until("the history to close", |snapshot| !snapshot.history.open);
    me.act(InputAction::Dismiss);
    me.conversation_over();

    me.caption("Things on the map can start conversations too — a chest washed up by the tide");
    me.interact_with("tide-chest");
    me.read_to_choices();
    me.pick("Close the lid.");
    me.conversation_over();
}

/// Points at open ground beside the conversation, found as the screen stands now: what covers the
/// map changes as the conversation goes on.
fn point_off_stage(me: &mut Star) -> ProbeSnapshot {
    let off_stage = *open_ground_beside(&me.snapshot())
        .first()
        .expect("open ground in view beside the conversation");
    me.point_at_ground(off_stage)
}

fn spoken_list(names: &[String]) -> String {
    match names {
        [rest @ .., last] if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        _ => names.concat(),
    }
}
