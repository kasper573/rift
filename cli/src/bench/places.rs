//! Places of a game the benches play at when the game names none itself: where its warps lead,
//! and open ground a walk away.

use engine::core::content::Content;
use engine::core::ground::TILE_SIDE;
use engine::core::math::{Pos, Rect};
use engine::core::render::transition::ScreenTransitionPhase;
use engine::core::units::{MeterPos, Meters};
use engine::systems::area;
use engine::systems::probe::ProbeSnapshot;
use engine::systems::script::{EffectReach, OwnerRow, Place, RiftScripts};
use engine::systems::sleep;
use engine::systems::staging::Staging;

use super::playtest::Playtest;

/// Where the player stands once the client shows it in the world.
pub fn settled_in_world(me: &mut Playtest) -> Place {
    let seen = me.wait_until("the player in the world", |snapshot| {
        snapshot.me.is_some() && snapshot.transition == ScreenTransitionPhase::Idle
    });
    Place {
        area: seen.area.expect("the player is in an area"),
        at: seen.me.expect("the player").at,
    }
}

/// Where the first warp out of `home` into another area leads.
pub fn away_from(content: &Content, home: Place) -> Option<Place> {
    warps_from(content, home.area).find(|to| to.area != home.area)
}

pub fn arrived(snapshot: &ProbeSnapshot, place: Place) -> bool {
    snapshot.area == Some(place.area)
        && snapshot.me.is_some()
        && snapshot.transition == ScreenTransitionPhase::Idle
}

pub fn staged(me: &Playtest, staging: Staging) {
    me.stage(&staging)
        .unwrap_or_else(|problem| panic!("the server didn't stage {staging:?}: {problem}"));
}

/// Where the warps out of `from` lead.
pub fn warps_from(content: &Content, from: area::AreaId) -> impl Iterator<Item = Place> + '_ {
    let scripts = content.built::<RiftScripts>();
    area::warp_objects(content, from)
        .into_iter()
        .flat_map(move |(index, _)| scripts.may_do(OwnerRow::of_object(from, index)).reaches)
        .filter_map(|reach| match reach {
            EffectReach::Warp(to) => Some(to),
            _ => None,
        })
}

/// The legs of a walk from `from` to `to` along the shortest walk there, each no longer than the
/// ground the server holds past a player, since a walk is routed only over held ground: a long
/// walk is asked for a leg at a time, as a player clicks ahead. Just `to` where no walk leads
/// there.
pub fn legs(content: &Content, from: Place, to: Pos<Meters>) -> Vec<Pos<Meters>> {
    let leg = sleep::awake_reach(content);
    let area = area::load(content, from.area);
    let around = Rect::from_points([from.at, to]).inflate(TILE_SIDE.0, TILE_SIDE.0);
    let _held = area.hold(area.ground_size().tiles_within(around));
    let mesh = area.mesh();
    let Some(corners) = mesh.route(from.at, to) else {
        return vec![to];
    };
    let mut legs = Vec::new();
    let mut last = from.at;
    let mut since_leg = 0.0;
    for corner in corners {
        let mut at = last;
        while at != corner {
            let left = at.distance(corner).0;
            let room = leg.0 - since_leg;
            if left <= room {
                since_leg += left;
                at = corner;
            } else {
                at = at.toward(corner, Meters(room));
                legs.push(mesh.nearest(at).unwrap_or(at));
                since_leg = 0.0;
            }
        }
        last = corner;
    }
    if legs.last() != Some(&to) {
        legs.push(to);
    }
    legs
}

/// Open ground a few steps from the first warp out of `area`, where a walk into it starts.
pub fn beside_warp(content: &Content, area: area::AreaId) -> Option<Place> {
    let (_, warp) = area::warp_objects(content, area).into_iter().next()?;
    let middle = warp.shape.center();
    let loaded = area::load(content, area);
    let _held = loaded.hold(loaded.tiles_around(middle, Meters(WARP_REACH)));
    let mesh = loaded.mesh();
    let at = (0..8).find_map(|step| {
        let angle = step as f32 * std::f32::consts::FRAC_PI_4;
        let beside = Pos::new(
            middle.x + angle.cos() * WARP_REACH,
            middle.y + angle.sin() * WARP_REACH,
        );
        mesh.walkable(beside).then_some(beside)
    })?;
    Some(Place { area, at })
}

/// How far from a warp a walk into it starts.
const WARP_REACH: f32 = 12.0;

/// Open ground a walk from `from` reaches, as far off as the first of `distances` that has some,
/// in the first of eight directions round from east that does.
pub fn walkable_toward(
    content: &Content,
    from: Place,
    distances: &[Meters],
) -> Option<Pos<Meters>> {
    let area = area::load(content, from.area);
    let farthest = distances
        .iter()
        .map(|distance| distance.0)
        .fold(0.0, f32::max);
    let _held = area.hold(area.tiles_around(from.at, Meters(farthest)));
    let mesh = area.mesh();
    let start = mesh.nearest(from.at)?;
    distances.iter().find_map(|distance| {
        (0..8).find_map(|step| {
            let angle = step as f32 * std::f32::consts::FRAC_PI_4;
            let to = Pos::new(
                start.x + angle.cos() * distance.0,
                start.y + angle.sin() * distance.0,
            );
            (mesh.walkable(to) && mesh.connects(start, to)).then_some(to)
        })
    })
}
