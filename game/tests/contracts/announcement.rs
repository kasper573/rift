use bevy_ecs::prelude::*;
use game::core::math::Offset;
use game::data;
use game::data::announcement::Id as AnnouncementId;
use game::data::memory::Id as MemoryId;
use game::data::npc::Id as NpcId;
use game::data::quest::Id as QuestId;
use game::systems::area::MarkerName;
use game::systems::area::transition::{self, Crossing};
use game::systems::dialogue::Conversation;
use game::systems::dialogue::announcement::{self, Announcements};
use game::systems::memory;
use game::systems::movement::{Position, position};
use game::systems::npc::{self, Pack};
use game::systems::player::{Xp, commands_locked};
use game::systems::quest::AcceptQuest;
use game::systems::rule::{Encounter, Terms};

use crate::support::{Sim, later, settle};

fn lane(sim: &mut Sim, player: Entity) -> Announcements {
    sim.world()
        .get::<Announcements>(player)
        .cloned()
        .unwrap_or_default()
}

fn showing(sim: &mut Sim, player: Entity) -> Option<AnnouncementId> {
    lane(sim, player).showing.map(|shown| shown.id)
}

fn next(sim: &mut Sim, player: Entity) -> Option<AnnouncementId> {
    lane(sim, player).next.map(|next| next.id)
}

fn shows(id: AnnouncementId) -> f32 {
    id.get().shows().0
}

#[test]
fn announcements_show_one_at_a_time_and_urgent_lines_go_first() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    for id in [
        AnnouncementId::HarbourBell,
        AnnouncementId::GullSails,
        AnnouncementId::ChiefChallenge,
        AnnouncementId::ChiefThreat,
    ] {
        announcement::announce(sim.world(), player, id);
    }
    settle(&mut sim);
    assert_eq!(showing(&mut sim, player), Some(AnnouncementId::HarbourBell));
    assert_eq!(next(&mut sim, player), Some(AnnouncementId::ChiefChallenge));

    later(&mut sim, shows(AnnouncementId::HarbourBell));
    assert_eq!(
        showing(&mut sim, player),
        Some(AnnouncementId::ChiefChallenge)
    );
    assert_eq!(next(&mut sim, player), Some(AnnouncementId::ChiefThreat));

    later(&mut sim, shows(AnnouncementId::ChiefChallenge));
    assert_eq!(showing(&mut sim, player), Some(AnnouncementId::ChiefThreat));
    assert_eq!(next(&mut sim, player), Some(AnnouncementId::GullSails));
}

#[test]
fn a_line_left_waiting_past_its_lifetime_is_missed() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    announcement::announce(sim.world(), player, AnnouncementId::HarbourBell);
    announcement::announce(sim.world(), player, AnnouncementId::Gulls);

    later(&mut sim, AnnouncementId::Gulls.get().lasts.0);

    let now = lane(&mut sim, player);
    assert_eq!(
        now.showing.map(|shown| shown.id),
        Some(AnnouncementId::HarbourBell)
    );
    assert_eq!(now.next, None);
    assert_eq!(
        now.missed
            .iter()
            .map(|missed| missed.id)
            .collect::<Vec<_>>(),
        vec![AnnouncementId::Gulls]
    );
}

#[test]
fn stepping_onto_the_pier_rings_the_harbour_bell_once_in_a_while() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let start = position(sim.world(), player).expect("position");
    let pier = sim
        .map()
        .marker(MarkerName("ferry-pier"))
        .expect("a pier on the map")
        .center();

    sim.world()
        .entity_mut(player)
        .insert(Position { pos: pier });
    settle(&mut sim);
    assert_eq!(showing(&mut sim, player), Some(AnnouncementId::HarbourBell));
    assert_eq!(next(&mut sim, player), Some(AnnouncementId::Gulls));
    assert!(!commands_locked(sim.world(), player));

    later(&mut sim, 30.0);
    sim.world()
        .entity_mut(player)
        .insert(Position { pos: start });
    settle(&mut sim);
    sim.world()
        .entity_mut(player)
        .insert(Position { pos: pier });
    settle(&mut sim);
    assert_eq!(showing(&mut sim, player), None);
    assert!(memory::recall(sim.world(), player, MemoryId::HeardHarbourBell).is_some());
}

#[test]
fn an_orc_chief_yells_at_players_on_maras_errand_without_stopping_them() {
    static ACCEPT: AcceptQuest = AcceptQuest(QuestId::TusksForTheChief);
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let bystander = sim.join(2);
    sim.world().get_mut::<Xp>(player).expect("xp").gain(100_000);
    Terms {
        requires: &[],
        costs: &[],
        outcomes: &[&ACCEPT],
    }
    .settle(sim.world(), player, Encounter::default())
    .expect("accepted");
    let at = position(sim.world(), player).expect("position");
    sim.world()
        .entity_mut(bystander)
        .insert(Position { pos: at });

    npc::spawn(
        sim.world(),
        NpcId::OrcChief,
        at + Offset::new(2.0, 0.0),
        data::area::SPAWN_ID,
        Pack(u32::MAX),
    );
    settle(&mut sim);

    assert_eq!(
        showing(&mut sim, player),
        Some(AnnouncementId::ChiefChallenge)
    );
    assert_eq!(next(&mut sim, player), Some(AnnouncementId::ChiefThreat));
    assert!(sim.world().get::<Conversation>(player).is_none());
    assert!(!commands_locked(sim.world(), player));
    assert_eq!(showing(&mut sim, bystander), None);
}

#[test]
fn the_lane_travels_with_the_character_and_the_forest_greets_arrivals() {
    let mut island = Sim::area(data::area::SPAWN_ID);
    let player = island.join(1);
    announcement::announce(island.world(), player, AnnouncementId::GullSails);
    let dest = island.map().spawn;
    island.world().entity_mut(player).insert(Crossing {
        dest_area: data::area::Id::Forest,
        dest,
    });
    let traveler = transition::departing(island.world())
        .pop()
        .expect("a traveler");

    let mut forest = Sim::area(data::area::Id::Forest);
    let arrived = transition::arrive(forest.world(), traveler);
    settle(&mut forest);

    assert_eq!(
        showing(&mut forest, arrived),
        Some(AnnouncementId::GullSails)
    );
    assert_eq!(
        next(&mut forest, arrived),
        Some(AnnouncementId::ForestArrival)
    );
}
