//! Seats: what each lets its rider do, how a passenger comes by one, and how a rider is asked to
//! leave.

use bevy_ecs::entity::MapEntities;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::{MountDef, RiderAct, Riding, SeatIndex};
use crate::core::content::Content;
use crate::core::math::Pos;
use crate::core::units::{MeterPos, Meters};
use crate::systems::account::AccountId;
use crate::systems::actor::Name;
use crate::systems::history::HistoryTopic;
use crate::systems::movement;
use crate::systems::notification::{self, Notification};
use crate::systems::player::RequestNeeds;
use crate::systems::player::{self, PlayedBy, Players};
use crate::systems::reach::{self, Pursuit, ReachAct, ReachTarget};
use crate::systems::social_request::{
    RequestUnaccepted, SocialRequest, SocialRequestAsk, SocialRequestKind, SocialRequestSource,
    SocialRequestWords,
};
use crate::systems::society::{self, Settling, SocietyAsk, SocietyView};
use crate::systems::text::LineText;
use crate::systems::{GameSettings, area, visibility};

/// What a player asks of the mount they ride, or of one they would ride.
#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Copy, Debug, PartialEq)]
pub enum MountRequest {
    /// Leaves their seat: for the spot they picked, or for the ground nearest them.
    StepOff { at: Option<Pos<Meters>> },
    /// A driver sets down whoever sits in `seat`.
    SetDown { seat: SeatIndex },
    /// Walks into reach of the mount `driver` drives, and asks them for a seat on it.
    AskSeat {
        #[entities]
        driver: Entity,
    },
}

pub fn requests(world: &mut World) {
    player::player_requests::<MountRequest>(
        world,
        RequestNeeds::Nothing,
        |world, player, request| match request {
            MountRequest::StepOff { at } => super::step_off(world, player, at),
            MountRequest::SetDown { seat } => {
                set_passenger_down(world, player, seat);
                Ok(())
            }
            MountRequest::AskSeat { driver } => ask(world, player, driver),
        },
    );
}

/// Lets `rider` do `act` from where they sit: at once from a seat that lets them or on foot, and
/// from one that does not as their mount has it, stepping off first where they can stand or
/// refused in the game's words.
pub fn act(world: &mut World, rider: Entity, act: RiderAct) -> Result<(), LineText> {
    if let Some(refusal) = barred(world, rider, act) {
        return Err(refusal.into());
    }
    if lets(world, rider, act) {
        return Ok(());
    }
    super::step_off(world, rider, None)
}

/// What the seat `rider` holds has against them doing `act` from it, in the game's words:
/// nothing on foot, from a seat that lets them, or on a mount that has them step off for it and
/// walks the ground they would stand on. One that flies or floats refuses in its place.
pub fn barred(world: &World, rider: Entity, act: RiderAct) -> Option<String> {
    if lets(world, rider, act) {
        return None;
    }
    let def = world
        .get::<Riding>(rider)?
        .mount
        .get(world.resource::<Content>());
    let steps_off = match def.barred {
        super::BarredAct::StepOff => matches!(def.travels, super::Travels::Riding),
        super::BarredAct::Refuse => false,
    };
    let words = &world.resource::<GameSettings>().words;
    (!steps_off).then(|| words.not_from_mount.replace("{mount}", &def.name))
}

/// Whether `rider` may do `act` where they are with no more ado: on foot, or from a seat that
/// lets them.
fn lets(world: &World, rider: Entity, act: RiderAct) -> bool {
    world.get::<Riding>(rider).is_none_or(|riding| {
        let def = riding.mount.get(world.resource::<Content>());
        seat_of(def, riding.seat).is_some_and(|seat| seat.may.contains(&act))
    })
}

/// The first seat beside the driver's that nobody sits in, on the mount `driver` drives.
pub fn free_seat(world: &mut World, driver: Entity) -> Option<SeatIndex> {
    let riding = world
        .get::<Riding>(driver)
        .copied()
        .filter(Riding::drives)?;
    let seats = riding.mount.get(world.resource::<Content>()).seats.len();
    let taken = super::passengers(world, driver);
    (1..seats)
        .filter_map(|seat| u8::try_from(seat).ok().map(SeatIndex))
        .find(|seat| !taken.iter().any(|(_, held)| held == seat))
}

/// Every driver whose mount has a seat free.
pub fn drivers_with_room(world: &mut World) -> Vec<Entity> {
    let drivers: Vec<Entity> = world
        .query::<(Entity, &Riding)>()
        .iter(world)
        .filter(|(_, riding)| riding.drives())
        .map(|(driver, _)| driver)
        .collect();
    drivers
        .into_iter()
        .filter(|driver| free_seat(world, *driver).is_some())
        .collect()
}

/// Walks everyone on their way to ask for a seat into reach of the mount, and has them ask its
/// driver once there.
pub fn boarding(world: &mut World, intents: &mut reach::Intents) {
    let content = world.resource::<Content>().clone();
    for (asker, driver) in reach::intending(world, intents, ReachAct::Board) {
        let Some(riding) = world.get::<Riding>(driver).copied().filter(Riding::drives) else {
            reach::forget(world, asker);
            continue;
        };
        let def = riding.mount.get(&content);
        match reach::pursue(world, asker, def.reach) {
            Pursuit::Approaching => {}
            Pursuit::Lost => reach::forget(world, asker),
            Pursuit::Arrived => {
                reach::forget(world, asker);
                let client = world.get::<PlayedBy>(asker).map(|by| by.client);
                let of = world.get::<SocietyView>(driver).map(|view| view.account);
                if let (Some(client), Some(of)) = (client, of) {
                    let ask = SocialRequestAsk::Ask {
                        kind: SocialRequestKind::Seat,
                        of,
                        about: Some(def.name.clone()),
                    };
                    society::ask(world, client, SocietyAsk::Request(ask));
                }
            }
        }
    }
}

/// A seat on a mount as a kind of request: it lives as long as its card, needs both players
/// here, and seats the asker the moment the driver accepts.
pub fn request_source() -> SocialRequestSource {
    SocialRequestSource {
        kind: SocialRequestKind::Seat,
        lasts: |settings| Some(settings.requests.card_shows),
        reaches_offline: false,
        words: |words| SocialRequestWords {
            asks: &words.seat_asks,
            asked: &words.seat_asked,
            sent: None,
            accepted: None,
            denied: Some(&words.seat_denied),
            lapsed: Some(&words.seat_lapsed),
        },
        may_send: |_, _, _, _| Ok(()),
        accepted,
        stands,
    }
}

/// Sets down whichever of the two rides the other's mount: neither carries someone who keeps
/// them away, nor is carried by them.
pub(crate) fn part(settling: &mut Settling, one: &AccountId, other: &AccountId) {
    let Some((world, one, other)) = riders(settling, one, other) else {
        return;
    };
    for (rider, driver) in [(one, other), (other, one)] {
        let carried = world
            .get::<Riding>(rider)
            .is_some_and(|riding| riding.driver == driver);
        if carried && rider != driver {
            super::set_down(world, rider);
        }
    }
}

fn ask(world: &mut World, asker: Entity, driver: Entity) -> Result<(), LineText> {
    if !player::may_command(world, asker) || !visibility::in_sight(world, asker, driver) {
        return Ok(());
    }
    let words = &world.resource::<GameSettings>().words;
    let (riding, taken) = (words.step_off_first.clone(), words.seats_taken.clone());
    if world.get::<Riding>(asker).is_some() {
        return Err(riding.into());
    }
    if free_seat(world, driver).is_none() {
        return Err(taken.into());
    }
    reach::intend(world, asker, ReachTarget::Actor(driver), ReachAct::Board);
    Ok(())
}

/// A request for a seat stands while its asker is in reach of the mount, and the mount is there.
fn stands(settling: &mut Settling, request: &SocialRequest) -> bool {
    riders(settling, &request.asker, &request.asked)
        .is_some_and(|(world, asker, driver)| in_reach(world, asker, driver))
}

fn accepted(
    settling: &mut Settling,
    settings: &GameSettings,
    request: &SocialRequest,
) -> Result<(), RequestUnaccepted> {
    let apart = RequestUnaccepted::Over(settings.words.seat_out_of_reach.clone());
    let Some((world, asker, driver)) = riders(settling, &request.asker, &request.asked) else {
        return Err(apart);
    };
    if !in_reach(world, asker, driver) || world.get::<Riding>(asker).is_some() {
        return Err(apart);
    }
    let Some(seat) = free_seat(world, driver) else {
        return Err(RequestUnaccepted::Over(settings.words.seats_taken.clone()));
    };
    let Some(riding) = world.get::<Riding>(driver).copied() else {
        return Err(apart);
    };
    let seated = Riding {
        mount: riding.mount,
        seat,
        driver,
    };
    super::seat_beside(world, asker, seated);
    Ok(())
}

/// The world both accounts' characters stand in, and the two characters.
fn riders<'s>(
    settling: &'s mut Settling,
    one: &AccountId,
    other: &AccountId,
) -> Option<(&'s mut World, Entity, Entity)> {
    let one = settling.roster.account(one)?.client;
    let (world, other) = settling.reach(other)?;
    let one = *world.resource::<Players>().0.get(&one)?;
    Some((world, one, other))
}

/// Whether `asker` could take a seat on the mount `driver` drives from where they stand.
fn in_reach(world: &World, asker: Entity, driver: Entity) -> bool {
    let content = world.resource::<Content>();
    let Some(riding) = world.get::<Riding>(driver).filter(|riding| riding.drives()) else {
        return false;
    };
    let near = match (
        movement::position(world, asker),
        movement::position(world, driver),
    ) {
        (Some(asker), Some(driver)) => asker.distance(driver) <= riding.mount.get(content).reach,
        _ => false,
    };
    near && area::same_area(world, asker, driver) && super::may_ride(world, asker)
}

fn set_passenger_down(world: &mut World, driver: Entity, seat: SeatIndex) {
    let drives = world.get::<Riding>(driver).is_some_and(Riding::drives);
    let passenger = super::passengers(world, driver)
        .into_iter()
        .find(|(_, held)| *held == seat);
    let Some((passenger, _)) = passenger.filter(|_| drives) else {
        return;
    };
    super::set_down(world, passenger);
    let name = world
        .get::<Name>(driver)
        .map(|name| name.name.clone())
        .unwrap_or_default();
    let words = &world.resource::<GameSettings>().words;
    let said = words.set_down_by.replace("{name}", &name);
    let told = Notification::feed(HistoryTopic::Notification, said);
    notification::notify(world, passenger, None, told);
}

fn seat_of(def: &MountDef, seat: SeatIndex) -> Option<&super::MountSeat> {
    def.seats.get(usize::from(seat.0))
}
