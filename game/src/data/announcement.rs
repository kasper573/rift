use crate::core::time::Seconds;
use crate::data::npc::Id as NpcId;
use crate::systems::announcement::{AnnouncementDef, Announcer};
use crate::systems::text::{Fx, Ink, Motion, Voice, plain, styled};

crate::table! {
    ChiefChallenge: AnnouncementDef {
        by: Announcer::Npc(NpcId::OrcChief),
        text: &[styled("WHO DARES STEAL TUSKS FROM MY CLAN?!", &[Fx::Voice(Voice::Shout), Fx::Motion(Motion::Shake)])],
        lasts: Seconds(20.0),
        urgent: true,
    },
    ChiefThreat: AnnouncementDef {
        by: Announcer::Npc(NpcId::OrcChief),
        text: &[plain("I'll grind your bones for "), styled("soup", &[Fx::Motion(Motion::Wave)]), plain("!")],
        lasts: Seconds(20.0),
        urgent: true,
    },
    HarbourBell: AnnouncementDef {
        by: Announcer::narrator("Harbour bell"),
        text: &[plain("The "), styled("Gull", &[Fx::Ink(Ink::Name)]), plain(" sails at the next bell.")],
        lasts: Seconds(30.0),
        urgent: false,
    },
    Gulls: AnnouncementDef {
        by: Announcer::narrator("Gulls"),
        text: &[plain("Gulls squabble over a fish head at the end of the pier.")],
        lasts: Seconds(3.0),
        urgent: false,
    },
    GullSails: AnnouncementDef {
        by: Announcer::narrator("The Gull"),
        text: &[plain("The "), styled("Gull", &[Fx::Ink(Ink::Name)]), plain(" scrapes onto the "), styled("forest shore", &[Fx::Ink(Ink::Place)]), plain(".")],
        lasts: Seconds(30.0),
        urgent: false,
    },
    ForestArrival: AnnouncementDef {
        by: Announcer::narrator("The forest"),
        text: &[plain("Pines close in around you. Somewhere ahead, "), styled("drums", &[Fx::Ink(Ink::Magic), Fx::Motion(Motion::Pulse)]), plain(".")],
        lasts: Seconds(30.0),
        urgent: false,
    },
    PellCallsGuards: AnnouncementDef {
        by: Announcer::Npc(NpcId::Pell),
        text: &[styled("GUARDS!", &[Fx::Voice(Voice::Shout), Fx::Motion(Motion::Shake)]), plain(" This one's a cheat!")],
        lasts: Seconds(10.0),
        urgent: true,
    },
    PellDrawsKnife: AnnouncementDef {
        by: Announcer::Npc(NpcId::Pell),
        text: &[plain("Nobody calls "), styled("Pell", &[Fx::Ink(Ink::Name)]), plain(" a cheat "), styled("twice", &[Fx::Voice(Voice::Shout)]), plain("!")],
        lasts: Seconds(10.0),
        urgent: true,
    },
}
