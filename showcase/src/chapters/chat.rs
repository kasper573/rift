use driver::{Player, UiTarget, input_name};
use engine::core::math::Offset;
use engine::systems::input::map::InputAction;

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "chat",
    title: "Chat",
    summary: "Meet other players and talk in the terminal",
    play,
};

fn play(shoot: &mut Shoot) {
    let player = shoot.join_fresh();
    let mut me = shoot.roll(player);
    me.caption("Other players share the world in real time…");
    let mut friend = shoot.join_fresh();
    me.caption("…like this one, who just signed in");
    let name = friend.snapshot().me.expect("the friend").name;
    friend.walk_by(Offset::new(-3.0, -1.0));
    me.caption("Every player's name floats over their head");
    me.find_ui(name.as_str());
    me.caption(&format!(
        "Press {} for the terminal, where everyone can chat: type, and {} sends",
        input_name(InputAction::ToggleTerminal),
        input_name(InputAction::SubmitText)
    ));
    me.act(InputAction::ToggleTerminal);
    submit(&mut me, "hello! anyone around?");
    friend.act(InputAction::ToggleTerminal);
    submit(&mut friend, "hi! let's go hunt some skeletons");
    me.find_ui(UiTarget::containing("let's go hunt some skeletons"));
    me.caption("It also takes commands — /commands lists them");
    submit(&mut me, "/commands");
    me.caption(&format!(
        "{} hands the keys back to the game, and {} closes the terminal",
        input_name(InputAction::Dismiss),
        input_name(InputAction::ToggleTerminal)
    ));
    me.act(InputAction::Dismiss);
    me.wait_until("the field to let go of the keys", |snapshot| {
        UiTarget::typing_field().find(snapshot).is_none()
    });
    me.act(InputAction::ToggleTerminal);
    me.wait_until("the terminal to close", |snapshot| {
        UiTarget::text_field().find(snapshot).is_none()
    });
}

// Keys typed before the field holds focus, or a submit sent before every key has landed, would be
// lost: each step waits for the game to catch up.
fn submit(player: &mut Player, text: &str) {
    player.find_ui(UiTarget::typing_field());
    let field = UiTarget::text_field();
    player.type_text(text, |snapshot| {
        field
            .find(snapshot)
            .is_some_and(|field| field.text.as_deref() == Some(text))
    });
    player.act(InputAction::SubmitText);
    player.wait_until("the line to be sent", |snapshot| {
        UiTarget::typing_field()
            .find(snapshot)
            .is_some_and(|field| field.text.is_none())
    });
}
