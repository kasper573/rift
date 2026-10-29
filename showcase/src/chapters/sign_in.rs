use driver::UiTarget;
use engine::core::math::Offset;

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "sign-in",
    title: "Sign in",
    summary: "The desktop client asks who's playing before it joins",
    play,
};

const NAME: &str = "Wren";

fn play(shoot: &mut Shoot) {
    let newcomer = shoot.start_signed_out();
    let mut me = shoot.roll(newcomer);
    me.caption(
        "Started on its own, the desktop client downloads the game, then asks who's playing",
    );
    me.find_ui(UiTarget::typing_field());
    me.caption("On a server with fake accounts, any name signs in, without a password");
    let field = UiTarget::text_field();
    me.type_text(NAME, |snapshot| {
        field
            .find(snapshot)
            .is_some_and(|field| field.text.as_deref() == Some(NAME))
    });
    me.hold();
    me.click_ui(UiTarget::button("Sign in"));
    me.caption("…and the player joins under that name");
    me.wait_until("the player in the world", |snapshot| {
        snapshot.me.as_ref().is_some_and(|me| me.name == NAME)
    });
    me.close_in();
    me.walk_by(Offset::new(3.0, 1.0));
}
