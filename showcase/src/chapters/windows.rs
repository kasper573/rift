use std::f32::consts::TAU;

use driver::{Player, UiTarget, content, held_in_stance, holding, input_name};
use engine::core::math::{Offset, Pos, WindowPx};
use engine::systems::actor::stance::HandIndex;
use engine::systems::input::map::InputAction;
use engine::systems::probe::ProbeSnapshot;

use crate::chapters::Chapter;
use crate::shoot::{Shoot, Star};

pub const CHAPTER: Chapter = Chapter {
    name: "windows",
    title: "Windows",
    summary: "Inventory, equipment, stats and settings, laid out your way",
    play,
};

// Each window opens at the same spot; these spread them over the corners of a 1280×720 view.
const LAYOUT: [(InputAction, &str, (f32, f32)); 4] = [
    (InputAction::ToggleInventory, "Inventory", (-330.0, -230.0)),
    (InputAction::ToggleEquipment, "Equipment", (440.0, -230.0)),
    (InputAction::ToggleStats, "Stats", (-330.0, 40.0)),
    (InputAction::ToggleSettings, "Settings", (440.0, 40.0)),
];

const SWORD: &str = "rusty_sword";

fn play(shoot: &mut Shoot) {
    let player = shoot.join_as("armed", None);
    let mut me = shoot.roll(player);
    let shield = me
        .wait_for("a shield in the bag", |seen| {
            seen.bag
                .iter()
                .map(|stack| stack.item)
                .find(|&item| held_in_stance(item, "shield"))
        })
        .name(content());
    me.caption(&format!(
        "Every window has a hotkey: {}. A window drops onto the table as it opens",
        [
            InputAction::ToggleInventory,
            InputAction::ToggleEquipment,
            InputAction::ToggleStats,
            InputAction::ToggleQuestLog,
            InputAction::ToggleSettings,
            InputAction::ToggleTerminal,
        ]
        .map(input_name)
        .join(", ")
    ));
    for (toggle, title, (x, y)) in LAYOUT {
        me.act(toggle);
        if title == "Inventory" {
            fling_and_resize(&mut me);
            me.caption("Drag a window by its title to move it");
        }
        me.drag_ui(title, Offset::new(x, y));
    }
    me.caption("Rest the pointer on an item to see what it is");
    me.hover_ui(outside_card(shield));
    me.hold();
    me.caption(&format!(
        "{} an item and its card is dealt from the slot: what it does, and a line of lore",
        input_name(InputAction::InspectItem)
    ));
    inspect(&mut me, shield);
    me.caption("One card at a time: the next is dealt on top, and the old one drops away under it");
    inspect(&mut me, SWORD);
    me.caption(&format!(
        "{} an item to use it — or, for gear, to wear it",
        input_name(InputAction::UseItem)
    ));
    me.act_on_ui(InputAction::UseItem, outside_card(SWORD));
    me.wait_until("the sword to be worn", |snapshot| {
        holding(snapshot, "rusty_sword") == 0
    });
    me.caption("Worn gear has a card too…");
    inspect(&mut me, shield);
    inspect(&mut me, SWORD);
    me.caption(&format!(
        "…and {} takes it off",
        input_name(InputAction::UseItem)
    ));
    me.act_on_ui(InputAction::UseItem, outside_card(SWORD));
    me.wait_until("the sword to be taken off", |snapshot| {
        holding(snapshot, "rusty_sword") == 1
    });
    hands(&mut me, shield);
    me.caption(&format!(
        "{} brings the card in front, and {} flicks away what's in front",
        input_name(InputAction::InspectItem),
        input_name(InputAction::Dismiss)
    ));
    inspect(&mut me, shield);
    me.wait_until("the card in front", card_in_front);
    me.hold();
    me.act(InputAction::Dismiss);
    me.wait_until("the item card to close", |snapshot| {
        snapshot.item_card.is_none()
    });

    me.caption("Windows snap to a grid — toggle it in Settings");
    me.click_ui(UiTarget::starting("UI snapping"));
    me.hold();
    me.click_ui(UiTarget::starting("UI snapping"));
    me.caption("Text speed is a slider, in letters a second");
    let before = me.find_ui(slider("Text speed")).slider;
    me.drag_ui(slider("Text speed"), Offset::new(70.0, 0.0));
    me.wait_until("the text speed to change", |snapshot| {
        slider("Text speed")
            .find(snapshot)
            .map(|speed| speed.slider)
            != Some(before)
    });
    me.caption(
        "Show FPS puts a meter in the corner: the frame rate now, and over the last ten seconds",
    );
    me.click_ui(UiTarget::starting("Show FPS"));
    me.find_ui(UiTarget::containing(" FPS ("));
    me.hold();
    me.click_ui(UiTarget::starting("Show FPS"));
    me.wait_until("the meter to go", |snapshot| {
        UiTarget::containing(" FPS (").find(snapshot).is_none()
    });
    me.caption(
        "Sound has a volume each: master, music, ambience, effects and voice, all halfway to start",
    );
    me.scroll_until("Audio", "Voice");
    me.caption("Halfway is the mix as balanced, with room to turn any of them up or down");
    me.show_mixer();
    me.caption("Music is the soundscape's first channel: turned down, the theme fades out");
    set_fader(&mut me, "Music", 0.0);
    me.caption("Ambience is every other channel: turned down, the world falls silent");
    set_fader(&mut me, "Ambience", 0.0);
    me.caption("Effects are the game's sounds: in the quiet, your footsteps…");
    let home = me.come_to_rest();
    me.walk_by(Offset::new(3.0, 0.0));
    me.caption("…which go quiet too with effects turned down");
    set_fader(&mut me, "Effects", 0.0);
    me.walk_to(home);
    me.caption("Back to halfway, everything plays as balanced again");
    for fader in ["Music", "Ambience", "Effects"] {
        set_fader(&mut me, fader, 0.5);
    }
    me.hold();
    me.hide_mixer();
    me.caption("Where you put them is remembered for next time");
    me.caption(&format!(
        "{} closes the window you used last, and it lifts off the table",
        input_name(InputAction::Dismiss)
    ));
    for _ in LAYOUT {
        me.act(InputAction::Dismiss);
    }
    me.wait_until("every window to close", |snapshot| {
        LAYOUT
            .iter()
            .all(|(_, title, _)| UiTarget::text(title).find(snapshot).is_none())
    });

    me.caption("Voice is how characters speak: at halfway, Grisha's greeting babbles…");
    hear_out_greeting(&mut me);
    me.act(InputAction::ToggleSettings);
    me.scroll_until("Audio", "Voice");
    set_fader(&mut me, "Voice", 0.0);
    me.act(InputAction::Dismiss);
    me.caption("…and with voice turned down, the same greeting is silent");
    hear_out_greeting(&mut me);
}

/// Gear in hand: what the equipment window's hands show as a sword, a dagger beside it, and a
/// greatsword that takes both are worn in turn.
fn hands(me: &mut Star, shield: &str) {
    me.caption("The equipment window shows your hands: a sword takes the main hand");
    me.act_on_ui(InputAction::UseItem, outside_card(SWORD));
    me.wait_until("the sword in hand", |snapshot| wears(snapshot, SWORD));
    me.caption("A dagger goes to the other hand, and you still fight as a swordsman");
    me.act_on_ui(InputAction::UseItem, outside_card(DAGGER));
    me.wait_until(
        "the dagger beside the sword, in the sword's stance",
        |snapshot| {
            wears(snapshot, SWORD) && wears(snapshot, DAGGER) && in_stance(snapshot, "sword")
        },
    );
    me.hold();
    me.caption(
        "A greatsword spans both hands: the sword and the dagger come back to the bag together",
    );
    me.act_on_ui(InputAction::UseItem, outside_card(GREATSWORD));
    me.wait_until("the greatsword in both hands", |snapshot| {
        wears(snapshot, GREATSWORD)
            && holding(snapshot, SWORD) == 1
            && holding(snapshot, DAGGER) == 1
    });
    me.hold();
    me.act_on_ui(InputAction::UseItem, outside_card(GREATSWORD));
    me.wait_until("the greatsword back in the bag", |snapshot| {
        holding(snapshot, GREATSWORD) == 1
    });
    me.caption("Anything held goes in any hand: a shield dropped on the main hand is held there");
    me.carry_onto(outside_card(shield), UiTarget::starting("Empty: "));
    me.wait_until("the shield in the main hand", |snapshot| {
        holds_in(snapshot, shield, HandIndex::MAIN)
    });
    me.hold();
    me.act_on_ui(InputAction::UseItem, outside_card(shield));
    me.wait_until("the shield back in the bag", |snapshot| {
        holding(snapshot, shield) == 1
    });
}

fn holds_in(snapshot: &ProbeSnapshot, item: &str, hand: HandIndex) -> bool {
    snapshot.me.as_ref().is_some_and(|me| {
        me.wearing
            .iter()
            .any(|(held, worn)| *held == hand && worn.name(content()) == item)
    })
}

const DAGGER: &str = "dagger";
const GREATSWORD: &str = "greatsword";

fn wears(snapshot: &ProbeSnapshot, item: &str) -> bool {
    snapshot.me.as_ref().is_some_and(|me| {
        me.wearing
            .iter()
            .any(|(_, worn)| worn.name(content()) == item)
    })
}

fn in_stance(snapshot: &ProbeSnapshot, stance: &str) -> bool {
    snapshot
        .me
        .as_ref()
        .and_then(|me| me.stance)
        .is_some_and(|worn| worn.name(content()) == stance)
}

fn fling_and_resize(me: &mut Star) {
    me.caption("Grab a window by its title and fling it about: the spot you grabbed stays under the pointer");
    me.dragging_ui("Inventory", |me| {
        let grabbed = me.snapshot().input.cursor.expect("the pointer");
        let path: Vec<Pos<WindowPx>> = (1..=FLING_STEPS)
            .map(|step| {
                let turn = TAU * 2.0 * step as f32 / FLING_STEPS as f32;
                grabbed + Offset::new(240.0 * turn.sin(), 140.0 * (turn * 1.5).sin())
            })
            .collect();
        me.sweep_pointer(&path)
    });
    me.hold();
    me.caption("Pull its corner to resize it");
    me.drag_ui("Resize", Offset::new(120.0, 90.0));
    me.hold();
    me.drag_ui("Resize", Offset::new(-120.0, -90.0));
}

const FLING_STEPS: usize = 240;

fn inspect(me: &mut Player, item: &str) {
    me.act_on_ui(InputAction::InspectItem, outside_card(item));
    me.wait_until(&format!("the card of {item}"), |snapshot| {
        snapshot
            .item_card
            .is_some_and(|card| card.item.name(content()) == item)
    });
}

fn card_in_front(snapshot: &ProbeSnapshot) -> bool {
    snapshot.item_card.is_some_and(|card| {
        let name = &card.item.get(content()).name;
        snapshot
            .ui
            .iter()
            .filter(|element| element.text.as_ref() == Some(name))
            .filter(|element| card.rect.contains(element.rect.center()))
            .all(|element| !element.click_through)
    })
}

// The item card repeats the icon of the item it shows; only the copies outside it count.
fn outside_card(item: &str) -> UiTarget {
    UiTarget::item(item).and("outside the item card", |element, snapshot| {
        snapshot
            .item_card
            .is_none_or(|card| !card.rect.contains(element.rect.center()))
    })
}

fn hear_out_greeting(me: &mut Player) {
    me.talk_to("grisha");
    me.read_to_choices();
    me.leave_conversation();
}

// Pressing a slider moves it to the press, so a drag from the middle of its track lands a value
// at the same share of the track's width away.
const TOLERANCE: f32 = 0.02;

fn set_fader(me: &mut Player, fader: &str, value: f32) {
    let track = me.find_ui(slider(fader)).rect.size.width;
    let by = match value {
        0.0 => -track,
        1.0 => track,
        _ => (value - 0.5) * track,
    };
    me.drag_ui(slider(fader), Offset::new(by, 0.0));
    let reached = slider(fader);
    me.wait_for(
        &format!("the {fader} volume to reach {value}"),
        |snapshot| {
            reached
                .find(snapshot)
                .filter(|track| {
                    track
                        .slider
                        .is_some_and(|now| (now - value).abs() <= TOLERANCE)
                })
                .cloned()
        },
    );
}

fn slider(name: &str) -> UiTarget {
    let name = name.to_owned();
    UiTarget::new(format!("the {name} slider"), move |element, _| {
        element.slider.is_some() && element.text.as_deref() == Some(name.as_str())
    })
}
