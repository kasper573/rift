use std::time::Duration;

use driver::{
    Player, UiTarget, WaitBudget, content, erred, finished, held_in_stance, holding, id,
    input_name, marked, noticed, on_quest, told_by,
};
use engine::systems::attention::AttentionMark;
use engine::systems::input::map::InputAction;
use engine::systems::quest::{QuestDef, QuestResult, pick_label};

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "quests",
    title: "Quests",
    summary: "Offers, a tracker and a log, deliveries, timers, and a reward that needs room",
    play,
};

const LETTER: &str = "tobbs_letter";
const SWORD: &str = "rusty_sword";
const BAIT: &str = "fishing_bait";

fn play(shoot: &mut Shoot) {
    let player = shoot.join_as("warrior_fallen", None);
    let mut me = shoot.roll(player);
    me.caption("A ! over a townsperson means they have work for you");
    me.approach("tobb");
    me.wait_until("! over Tobb", |snapshot| {
        marked(snapshot, "tobb", AttentionMark::QuestOffered)
    });
    let heard_news = talk_past_news(&mut me);
    me.caption("Quests join the conversation as topics");
    me.hold();
    me.pick("A Letter for the Captain");
    me.on_stage("letter_offer");
    me.caption(
        "A quest card docks above the conversation with its objectives and rewards. Accept or decline below",
    );
    me.read_to_choices();
    me.hold();
    me.pick("I'll take it.");
    me.wait_until("the letter quest", |snapshot| {
        on_quest(snapshot, "letter_for_the_captain").is_some()
    });
    me.wait_until("the accepted quest in the feed", |snapshot| {
        noticed(snapshot, "Quest accepted · A Letter for the Captain")
    });
    me.caption(
        "Accepted: a line over your head says so, the letter is in your bag, the quest on your tracker",
    );
    if !heard_news {
        hear_out_tobb(&mut me, Duration::from_secs(12));
    }

    me.caption(
        "Quest items are marked: they can't be sold or dropped, and abandoning takes them back",
    );
    me.act(InputAction::ToggleInventory);
    me.hover_ui(UiTarget::item(LETTER));
    me.hold();
    me.act(InputAction::ToggleInventory);

    me.caption("Some quests run against the clock");
    talk_past_news(&mut me);
    me.pick("Low Tide");
    me.on_stage("low_tide_offer");
    me.read_to_choices();
    me.pick("I'll hurry.");
    me.wait_until("Low Tide's clock", |snapshot| {
        on_quest(snapshot, "low_tide").is_some_and(|quest| quest.left.is_some())
    });
    me.caption("The tracker counts it down");

    me.caption(&format!(
        "{} opens the quest log, grouped by category; the chosen quest lies on the page as a card",
        input_name(InputAction::ToggleQuestLog)
    ));
    me.act(InputAction::ToggleQuestLog);
    me.find_ui("Active quests");
    me.click_ui(UiTarget::button("Low Tide"));
    me.caption("A long card scrolls its plate under the name band, down to the rewards");
    me.scroll_until("objectives", UiTarget::item(BAIT));
    me.caption(&format!(
        "{} a reward for its item card",
        input_name(InputAction::InspectItem)
    ));
    me.act_on_ui(InputAction::InspectItem, UiTarget::item(BAIT));
    me.wait_until("the bait's card", |snapshot| {
        snapshot
            .item_card
            .is_some_and(|card| card.item.name(content()) == "fishing_bait")
    });
    me.hold();
    me.act(InputAction::Dismiss);
    me.wait_until("the item card to close", |snapshot| {
        snapshot.item_card.is_none()
    });
    me.caption("A quest can keep its rewards to itself: the letter's entry has none to show");
    me.click_ui(UiTarget::button("A Letter for the Captain"));
    me.hold();
    me.click_ui(UiTarget::button("Low Tide"));
    me.caption(&format!(
        "Abandoning always asks, and Keep quest is the default — {} keeps it",
        input_name(InputAction::TakeDefault)
    ));
    me.click_ui("Abandon");
    me.find_ui("Abandon Low Tide?");
    me.hold();
    me.act(InputAction::TakeDefault);
    me.wait_until("Enter to keep the quest", |snapshot| {
        UiTarget::text("Abandon Low Tide?").find(snapshot).is_none()
    });
    me.click_ui("Abandon");
    me.click_ui("Abandon quest");
    me.wait_until("Low Tide to be abandoned", |snapshot| {
        on_quest(snapshot, "low_tide").is_none()
    });
    me.act(InputAction::Dismiss);

    me.caption("Bram's next errand stays out of sight until you're ready for it");
    me.hold();
    me.talk_to("bram");
    me.read_to_choices();
    me.hold();
    me.act(InputAction::Dismiss);
    me.conversation_over();

    me.caption("The letter is for Bram: a ? marks where a quest is handed in");
    me.wait_until("? over Bram", |snapshot| {
        marked(snapshot, "bram", AttentionMark::QuestReady)
    });
    me.talk_to("bram");
    me.read_to_choices();
    me.pick("A Letter for the Captain");
    me.caption("Nobody named Bram's pay, so the choice keeps it to itself");
    me.hold();
    me.pick("Tobb asked me to bring you this.");
    me.on_stage("letter_thanks");
    me.wait_until("the letter delivered", |snapshot| {
        finished(snapshot, "letter_for_the_captain") == Some(QuestResult::Completed)
    });
    me.wait_until("the pay in the feed", |snapshot| {
        noticed(snapshot, "+10 Gold")
    });
    me.caption(
        "Delivered: the letter goes and the pay arrives over your head, the finished quest lands in the middle",
    );
    me.wait_until("the finished quest", |snapshot| {
        told_by(snapshot, "Quest complete")
    });
    me.caption("The XP brought a new level, which joins it right below");
    me.wait_until("the level up", |snapshot| told_by(snapshot, "Level up"));
    me.hold();
    me.pick("Never mind.");
    me.conversation_over();
    me.caption("Now at level 3, he has work for you");
    me.wait_until("! over Bram", |snapshot| {
        marked(snapshot, "bram", AttentionMark::QuestOffered)
    });
    me.talk_to("bram");
    me.read_to_choices();
    me.hover_choice("Bats in the Belfry");
    me.hold();
    me.act(InputAction::Dismiss);
    me.conversation_over();

    me.caption("Ilsa pays for bones — and the warrior that leads them. You came prepared: the warrior is down, the bones in the bag");
    me.wait_until("the bone quest ready", |snapshot| {
        on_quest(snapshot, "bones_for_the_warden").is_some_and(|quest| quest.ready)
    });
    me.wait_until("? over Ilsa", |snapshot| {
        marked(snapshot, "ilsa", AttentionMark::QuestReady)
    });
    me.talk_to("ilsa");
    me.read_to_choices();
    me.pick("Bones for the Warden");
    let shield = *id::<QuestDef>("bones_for_the_warden")
        .get(content())
        .pick_one()
        .iter()
        .find(|pick| held_in_stance(pick.item, "shield"))
        .expect("a shield among the bone quest's picks");
    let take_shield = pick_label(content(), shield);
    me.caption("Rewards can include a pick, chosen when you hand in");
    me.caption("Rest the pointer on a reward to see what it is");
    me.hover_ui(reward_on(&take_shield));
    me.hold();
    me.caption(&format!(
        "{} it for its card",
        input_name(InputAction::InspectItem)
    ));
    me.act_on_ui(InputAction::InspectItem, reward_on(&take_shield));
    me.wait_until("the shield's card", |snapshot| {
        snapshot
            .item_card
            .is_some_and(|card| card.item == shield.item)
    });
    me.hold();
    me.act(InputAction::Dismiss);
    me.wait_until("the card to close", |snapshot| snapshot.item_card.is_none());
    me.caption(
        "With a full bag the pick is refused with an error — space is counted after the bones are handed in",
    );
    me.pick(&take_shield);
    me.wait_until("the full bag to refuse the pick", |snapshot| {
        erred(snapshot, "free slot")
    });
    me.hold();
    me.pick("Not yet.");
    me.conversation_over();

    me.caption(&format!(
        "Make some room: {} drops",
        input_name(InputAction::DropItem)
    ));
    me.act(InputAction::ToggleInventory);
    for _ in 0..2 {
        let before = holding(&me.snapshot(), "rusty_sword");
        me.act_on_ui(InputAction::DropItem, UiTarget::item(SWORD));
        me.wait_until("a sword to drop", |snapshot| {
            holding(snapshot, "rusty_sword") < before
        });
    }
    me.act(InputAction::ToggleInventory);

    me.talk_to("ilsa");
    me.read_to_choices();
    me.pick("Bones for the Warden");
    me.caption("Now there's room for the pick");
    me.pick(&take_shield);
    me.on_stage("bones_thanks");
    me.wait_until("the bones handed in", |snapshot| {
        finished(snapshot, "bones_for_the_warden") == Some(QuestResult::Completed)
    });

    me.caption("Finished quests move to Completed in the log");
    me.act(InputAction::ToggleQuestLog);
    me.click_ui("Completed");
    me.hold();
    me.act(InputAction::Dismiss);
}

// Once a day Tobb calls over whoever comes near him after a first visit, holding his news while they
// talk: it shows when he strolls back, or opens the next talk with him instead.
fn hear_out_tobb(me: &mut Player, within: Duration) {
    let budget = WaitBudget {
        time: within,
        frames: (within.as_secs_f32() * 60.0) as u32,
    };
    let news = me.try_wait_for(budget, |snapshot| {
        snapshot
            .stage
            .as_ref()
            .is_some_and(|stage| stage.node == "news")
            .then_some(())
    });
    if news.is_some() {
        me.answer_first_choices();
    }
}

// A choice shows its item beside its words too; the reward is the chip that counts it.
fn reward_on(choice: &str) -> UiTarget {
    let choice = choice.to_owned();
    UiTarget::text("+1").and(&format!("on {choice:?}"), move |element, snapshot| {
        snapshot.stage.as_ref().is_some_and(|stage| {
            stage.choices.iter().any(|shown| {
                shown.label == choice
                    && shown
                        .rect
                        .is_some_and(|rect| rect.contains(element.rect.center()))
            })
        })
    })
}

fn talk_past_news(me: &mut Player) -> bool {
    let mut heard = false;
    loop {
        me.talk_to("tobb");
        if me.read_to_choices().node != "news" {
            return heard;
        }
        me.answer_first_choices();
        heard = true;
    }
}
