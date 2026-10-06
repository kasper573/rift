use std::borrow::Cow;

use crate::core::sfx::SfxId;
use crate::core::time::Seconds;
use crate::data::npc::Id as NpcId;
use crate::systems::actor::Rgba;
use crate::systems::notification::{IntroAppearance, NotificationDef, NotificationKind};
use crate::systems::text::{Fx, Ink, Motion, Voice, plain, styled};

crate::table! {
    ChiefChallenge: NotificationDef {
        kind: NotificationKind::Speech { speaker: NpcId::OrcChief },
        text: &[styled("WHO DARES STEAL TUSKS FROM MY CLAN?!", &[Fx::Voice(Voice::Shout), Fx::Motion(Motion::Shake)])],
        lasts: None,
    },
    ChiefThreat: NotificationDef {
        kind: NotificationKind::Speech { speaker: NpcId::OrcChief },
        text: &[plain("I'll grind your bones for "), styled("soup", &[Fx::Motion(Motion::Wave)]), plain("!")],
        lasts: None,
    },
    HarbourBell: NotificationDef {
        kind: NotificationKind::Narration { label: Cow::Borrowed("Harbour bell"), sfx: Some(SfxId::BellToll) },
        text: &[plain("The "), styled("Gull", &[Fx::Ink(Ink::Name)]), plain(" sails at the next bell.")],
        lasts: None,
    },
    Gulls: NotificationDef {
        kind: NotificationKind::Narration { label: Cow::Borrowed("Gulls"), sfx: Some(SfxId::GullCries) },
        text: &[plain("Gulls squabble over a fish head at the end of the pier.")],
        lasts: None,
    },
    GullSails: NotificationDef {
        kind: NotificationKind::Narration { label: Cow::Borrowed("The Gull"), sfx: Some(SfxId::HullScrape) },
        text: &[plain("The "), styled("Gull", &[Fx::Ink(Ink::Name)]), plain(" scrapes onto the "), styled("forest shore", &[Fx::Ink(Ink::Place)]), plain(".")],
        lasts: None,
    },
    ForestIntro: NotificationDef {
        kind: NotificationKind::Intro {
            title: Cow::Borrowed("The forest"),
            appearance: IntroAppearance {
                tint: Some(Rgba(0x8fd1_8bff)),
                motion: Some(Motion::Pulse),
                ..IntroAppearance::PLAIN
            },
            sfx: Some(SfxId::ForestDrums),
        },
        text: &[plain("Pines close in around you. Somewhere ahead, "), styled("drums", &[Fx::Ink(Ink::Magic)]), plain(".")],
        lasts: Some(Seconds(5.0)),
    },
    PellCallsGuards: NotificationDef {
        kind: NotificationKind::Speech { speaker: NpcId::Pell },
        text: &[styled("GUARDS!", &[Fx::Voice(Voice::Shout), Fx::Motion(Motion::Shake)]), plain(" This one's a cheat!")],
        lasts: None,
    },
    PellDrawsKnife: NotificationDef {
        kind: NotificationKind::Speech { speaker: NpcId::Pell },
        text: &[plain("Nobody calls "), styled("Pell", &[Fx::Ink(Ink::Name)]), plain(" a cheat "), styled("twice", &[Fx::Voice(Voice::Shout)]), plain("!")],
        lasts: None,
    },
}
