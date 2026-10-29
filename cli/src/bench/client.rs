//! The desktop client's frame rate through the scenes a player meets, each staged on the server and
//! then only watched: what's measured is the client drawing the scene, not answering the bench.

use std::path::Path;
use std::time::{Duration, Instant};

use engine::core::content::Content;
use engine::core::math::Pos;
use engine::core::render::transition::ScreenTransitionPhase;
use engine::core::units::{MeterPos, Meters};
use engine::systems::area;
use engine::systems::probe::ProbeSnapshot;
use engine::systems::script::{OwnerRow, Place, Reach, RiftScripts};
use engine::systems::staging::Staging;

use super::playtest::Playtest;

pub fn run(game: &Path, content: &Content) {
    let mut me = Playtest::start(game, content);
    let home = settled_in_world(&mut me);
    let mut scenes = vec![watch(&mut me, "town", Duration::from_secs(4))];

    let walk = walkable_near(content, home, Meters(5.0)..Meters(8.0));
    match walk {
        Some(to) => {
            staged(&me, Staging::Walk { to });
            scenes.push(watch(&mut me, "walk", Duration::from_secs(3)));
        }
        None => skipped("walk", "no open ground a few metres off"),
    }
    match me.stage(Staging::Fight) {
        Ok(()) => scenes.push(watch(&mut me, "combat", Duration::from_secs(4))),
        Err(problem) => skipped("combat", &problem),
    }
    staged(&me, Staging::Place { place: home });
    match me.stage(Staging::Talk) {
        Ok(()) => scenes.push(watch(&mut me, "conversation", Duration::from_secs(4))),
        Err(problem) => skipped("conversation", &problem),
    }
    match away_from(&me, home) {
        Some(away) => {
            staged(&me, Staging::Place { place: away });
            scenes.push(watch(&mut me, "crossing", Duration::from_millis(2500)));
            me.wait_until("the other area", |snapshot| arrived(snapshot, away));
            scenes.push(watch(&mut me, "away", Duration::from_secs(4)));
            staged(&me, Staging::Place { place: home });
            me.wait_until("back home", |snapshot| arrived(snapshot, home));
            scenes.push(watch(&mut me, "back", Duration::from_secs(2)));
        }
        None => skipped("crossing", "no warp out of the spawn area"),
    }
    report(&scenes);
}

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
pub fn away_from(me: &Playtest, home: Place) -> Option<Place> {
    let content = me.content();
    let scripts = content.built::<RiftScripts>();
    area::warp_objects(content, home.area)
        .into_iter()
        .flat_map(|(index, _)| {
            scripts
                .may_do(OwnerRow::of_object(home.area, index))
                .reaches
        })
        .find_map(|reach| match reach {
            Reach::Warp(to) if to.area != home.area => Some(to),
            _ => None,
        })
}

pub fn arrived(snapshot: &ProbeSnapshot, place: Place) -> bool {
    snapshot.area == Some(place.area)
        && snapshot.me.is_some()
        && snapshot.transition == ScreenTransitionPhase::Idle
}

pub fn staged(me: &Playtest, staging: Staging) {
    me.stage(staging)
        .unwrap_or_else(|problem| panic!("the server didn't stage {staging:?}: {problem}"));
}

struct Scene {
    name: &'static str,
    frames: u32,
    lasted: Duration,
}

impl Scene {
    fn fps(&self) -> f64 {
        f64::from(self.frames) / self.lasted.as_secs_f64()
    }
}

fn watch(me: &mut Playtest, name: &'static str, lasting: Duration) -> Scene {
    let first = me.snapshot().frame;
    let since = Instant::now();
    me.linger(lasting);
    let last = me.snapshot().frame;
    Scene {
        name,
        frames: last - first,
        lasted: since.elapsed(),
    }
}

fn skipped(scene: &str, why: &str) {
    println!("[client-bench] {scene:<12} not benched: {why}");
}

fn report(scenes: &[Scene]) {
    for scene in scenes {
        println!("[client-bench] {:<12} {:>6.0} fps", scene.name, scene.fps());
    }
    let frames: u32 = scenes.iter().map(|scene| scene.frames).sum();
    let lasted: Duration = scenes.iter().map(|scene| scene.lasted).sum();
    let lowest = scenes
        .iter()
        .min_by(|a, b| a.fps().total_cmp(&b.fps()))
        .expect("scenes were watched");
    println!(
        "[client-bench] RESULT fps={:.0} lowest={:.0} ({})",
        f64::from(frames) / lasted.as_secs_f64(),
        lowest.fps(),
        lowest.name
    );
}

/// The walkable ground farthest within `within` of `from` that a walk from it reaches.
fn walkable_near(
    content: &Content,
    from: Place,
    within: std::ops::Range<Meters>,
) -> Option<Pos<Meters>> {
    let mesh = &area::load(content, from.area).mesh;
    let start = mesh.nearest(from.at)?;
    let reach = within.end.0.ceil() as i32;
    (-reach..=reach)
        .flat_map(|dy| (-reach..=reach).map(move |dx| (dx, dy)))
        .map(|(dx, dy)| Pos::new(start.x.round() + dx as f32, start.y.round() + dy as f32))
        .filter(|point| within.contains(&point.distance(start)))
        .filter(|point| mesh.walkable(*point) && mesh.connects(start, *point))
        .max_by(|a, b| a.distance(start).0.total_cmp(&b.distance(start).0))
}
