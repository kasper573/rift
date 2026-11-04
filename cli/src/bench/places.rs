//! Places of a game the benches play at when the game names none itself: where the player
//! starts, where its warps lead, the open ground near the middle of each area, and open ground a
//! walk away.

use std::path::Path;

use engine::core::content::Content;
use engine::core::math::Pos;
use engine::core::render::transition::ScreenTransitionPhase;
use engine::core::units::{MeterSize, Meters};
use engine::systems::area::{self, AreaDef};
use engine::systems::probe::ProbeSnapshot;
use engine::systems::script::{EffectReach, OwnerRow, Place, RiftScripts};
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

/// A place in each of the game's areas, each named by its file: home's where the player starts,
/// another's the first a warp leads into it at, or the open ground nearest its middle.
pub fn arrivals(content: &Content, home: Place) -> Vec<(String, Place)> {
    let mut places = vec![home];
    for area in content.ids::<AreaDef>() {
        for to in warps_from(content, area) {
            if places.iter().all(|place| place.area != to.area) {
                places.push(to);
            }
        }
    }
    for area in content.ids::<AreaDef>() {
        if places.iter().all(|place| place.area != area)
            && let Some(at) = open_middle(content, area)
        {
            places.push(Place { area, at });
        }
    }
    places
        .into_iter()
        .map(|place| {
            let path = place.area.path(content);
            let name = Path::new(path).file_stem().map_or_else(
                || path.to_owned(),
                |stem| stem.to_string_lossy().into_owned(),
            );
            (name, place)
        })
        .collect()
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

/// The open ground nearest the middle of `area`.
pub fn open_middle(content: &Content, area: area::AreaId) -> Option<Pos<Meters>> {
    let loaded = area::load(content, area);
    let middle = loaded.size.bounds().center();
    let _held = loaded.hold(loaded.tiles_around(middle, Meters(MIDDLE_REACH)));
    loaded.mesh().nearest(middle)
}

/// How far from an area's middle its open ground is looked for.
const MIDDLE_REACH: f32 = 64.0;

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
