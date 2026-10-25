use std::time::{Duration, Instant};

use driver::{
    Player, UiTarget, arrived_in, content, erred, holding, input_name, marker_at, nearest_npc,
    noticed, npc_is, told_by,
};
use engine::core::math::Offset;
use engine::systems::history::HistoryTopic;
use engine::systems::input::map::InputAction;
use engine::systems::interface::InterfaceIcon;
use engine::systems::probe::ProbeSnapshot;

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "notifications",
    title: "Notifications",
    summary: "Every notification where it belongs: bubbles, captions, alerts, errors, milestones and intros",
    play,
};

const HELMET: &str = "tribal_helmet";
const CUTLASS: &str = "corsair_cutlass";

fn play(shoot: &mut Shoot) {
    let player = shoot.join_as("forest_bound", None);
    let mut me = shoot.roll(player);
    me.caption("Some places start a conversation on their own: Ilsa watches the north road");
    let road = marker_at("island", "forest-road");
    me.travel_to(|_| Some(road), |snapshot| snapshot.stage.is_some());
    me.on_stage("halt");
    me.caption("Without a pass the road is just ground. Standing on it, she stops you and your commands lock");
    me.read_to_choices();
    me.caption("Ilsa shows only what she means to: the road opens at level 3");
    me.hover_choice("I'm ready for the north road.");
    me.hold();
    me.pick("I'll turn back.");
    me.conversation_over();

    me.caption("Talk to her, though, and a fish changes her mind");
    me.talk_to("ilsa");
    me.read_to_choices();
    me.hover_choice("Would a fish change your mind?");
    me.hold();
    me.pick("Would a fish change your mind?");
    me.on_stage("fish");
    me.wait_until("the pass in the feed", |snapshot| {
        noticed(snapshot, "+1 Road Pass")
    });
    me.caption("The trade rises over your head: the fish goes, the pass arrives");
    me.read_to_choices();
    me.pick("Enjoy it.");
    me.conversation_over();
    me.wait_until("the pass in the bag", |snapshot| {
        holding(snapshot, "road_pass") == 1
    });

    me.caption(
        "Narration plays as captions at the top and never blocks: gulls squabble at the pier",
    );
    // With the pass in the bag the road would take the player now, so the way to the pier keeps clear
    // of it.
    me.walk_near(road + Offset::new(0.0, 3.0));
    // The gulls squabble once in a long while, and only briefly: the caption is caught as it comes
    // up, the walk onto the pier finishing under the pointer.
    let squabble = || UiTarget::containing("squabble");
    me.travel_to(
        |_| Some(marker_at("island", "ferry-pier")),
        |snapshot| squabble().find(snapshot).is_some(),
    );
    me.hover_ui(squabble());
    me.caption("Hovering holds the captions, so you can finish reading");
    me.hold();
    me.caption(&format!(
        "Every line is kept in history as well — {} shows it. A window lies over the captions: only alerts and errors rise above windows",
        input_name(InputAction::ToggleHistory)
    ));
    me.act(InputAction::ToggleHistory);
    me.wait_until("the history to open", |snapshot| snapshot.history.open);
    let caption = me.find_ui(squabble()).rect;
    let title = me.find_ui("History").rect;
    me.drag_ui(
        "History",
        Offset::new(
            caption.center().x - title.center().x,
            caption.center().y - title.min_y(),
        ),
    );
    me.hold();
    me.act(InputAction::ToggleHistory);
    me.wait_until("the history to close", |snapshot| !snapshot.history.open);

    me.caption("What you can't do is an error, in red at the top: the Tribal Helmet wants level 3");
    me.click_ui(UiTarget::interface_icon(InterfaceIcon::Bag));
    me.act_on_ui(InputAction::UseItem, UiTarget::item(HELMET));
    me.wait_until("the helmet refused", |snapshot| {
        erred(snapshot, "Tribal Helmet")
    });
    me.caption("Only the latest error shows: the Corsair's Cutlass wants level 3 too");
    me.act_on_ui(InputAction::UseItem, UiTarget::item(CUTLASS));
    me.wait_until("the cutlass refused over the helmet", |snapshot| {
        erred(snapshot, "Corsair's Cutlass")
    });
    me.caption("Trying again keeps that one error up, with nothing piling on");
    me.act_on_ui(InputAction::UseItem, UiTarget::item(CUTLASS));
    me.caption("Errors it replaced are kept in history");
    me.act(InputAction::ToggleHistory);
    me.wait_until("the history to open", |snapshot| snapshot.history.open);
    me.open_history_tab("Errors");
    me.wait_until("both errors in the history", |snapshot| {
        snapshot
            .history
            .records
            .iter()
            .filter(|record| record.topic == HistoryTopic::Error)
            .count()
            >= 2
    });
    me.hold();
    me.act(InputAction::ToggleHistory);
    me.wait_until("the history to close", |snapshot| !snapshot.history.open);
    me.act(InputAction::ToggleInventory);

    me.caption("The woodsman's cottage is up the hill: its door is a warp");
    me.cross("front-door");
    // Intros stay up only a while: each is waited for before a caption holds the chapter.
    me.wait_until("the cottage's intro", |snapshot| {
        arrived_in(snapshot, "cottage") && told_by(snapshot, "The woodsman's cottage")
    });
    me.caption("A first visit opens on the area's own title, to its own drums");
    me.walk_by(Offset::new(-2.0, -1.0));
    me.hold();
    me.cross("front-door");

    me.caption(
        "The skeletons in the bone camp are worth experience, and one more brings a new level: a milestone, drawn over every other notice",
    );
    fight_until(&mut me, "skeleton", |snapshot| {
        told_by(snapshot, "Level up")
    });
    me.hold();

    me.caption(
        "You're on Ilsa's errand, and the camp's warrior knows it: come near him and he has words for you",
    );
    // The warrior taunts once and then holds his tongue for a while, so one that wandered past
    // earlier has already had his say; the bubble is captioned only while it is up.
    let taunted = |snapshot: &ProbeSnapshot| {
        yelled(snapshot, "skeleton_warrior")
            || snapshot
                .history
                .records
                .iter()
                .any(|record| record.by.as_deref() == Some("Skeleton Warrior"))
    };
    seek(&mut me, "skeleton_warrior", taunted);
    if yelled(&me.snapshot(), "skeleton_warrior") {
        me.caption(
            "Voices speak from where they stand: a bubble over the warrior, in his own voice",
        );
        me.hold();
    }
    me.caption("You keep fighting: no tether, no lock, and your foe's bubble stays up front");
    let until = Instant::now() + Duration::from_secs(9);
    me.fight(
        |_, body| npc_is(body, "skeleton_warrior"),
        |snapshot| Instant::now() >= until || nearest_npc(snapshot, "skeleton_warrior").is_none(),
    );
}

fn yelled(snapshot: &ProbeSnapshot, npc: &str) -> bool {
    snapshot
        .notifications
        .bubbles
        .iter()
        .any(|bubble| bubble.npc.name(content()) == npc)
}

fn fight_until(me: &mut Player, npc: &str, done: impl Fn(&ProbeSnapshot) -> bool) {
    me.look_out(npc, |snapshot| {
        done(snapshot) || nearest_npc(snapshot, npc).is_some()
    });
    me.fight(|_, body| npc_is(body, npc), done);
}

// Walks toward the nearest of them until `done`.
fn seek(me: &mut Player, npc: &str, done: impl Fn(&ProbeSnapshot) -> bool) {
    let ground = me.look_out(npc, |snapshot| {
        done(snapshot) || nearest_npc(snapshot, npc).is_some()
    });
    me.travel_to(
        |snapshot| Some(nearest_npc(snapshot, npc).map_or(ground, |foe| foe.at)),
        done,
    );
}
