//! The desktop client's frame rate through the scenes a player meets, at the lowest and the highest
//! video settings, each staged on the server and then watched: every area the game's warps lead to
//! is crossed into, stood in and walked across into ground the view has yet to take in, and seen
//! again from the widest zoom. What a scene reports is its frames, slowest included, and how long
//! what the view takes in went missing while the scenery stood it.

use std::path::Path;
use std::time::{Duration, Instant};

use engine::core::content::Content;
use engine::core::math::Pos;
use engine::core::render::camera::Zoom;
use engine::core::render::transition::ScreenTransitionPhase;
use engine::core::units::{MeterPos, MeterSize, Meters};
use engine::systems::area::AreaDef;
use engine::systems::probe::ProbeSnapshot;
use engine::systems::script::{EffectReach, OwnerRow, Place, RiftScripts};
use engine::systems::staging::Staging;
use engine::systems::{GameSettings, area};

use super::frames::{Frames, Video};
use super::playtest::Playtest;

pub fn run(game: &Path, content: &Content) {
    for video in Video::ALL {
        let scenes = play(game, content, video);
        report(video, &scenes);
    }
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
    warps_from(me.content(), home.area).find(|to| to.area != home.area)
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

fn play(game: &Path, content: &Content, video: Video) -> Vec<Scene> {
    let mut me = Playtest::start(game, content, video.preferences());
    let home = settled_in_world(&mut me);
    let mut scenes = vec![watch(&mut me, "town".to_owned(), Duration::from_secs(4))];
    match walkable_toward(content, home, &[Meters(8.0), Meters(5.0)]) {
        Some(to) => {
            staged(&me, Staging::Walk { to });
            scenes.push(watch(&mut me, "walk".to_owned(), Duration::from_secs(3)));
        }
        None => skipped("walk", "no open ground a few metres off"),
    }
    match me.stage(&Staging::Fight) {
        Ok(()) => scenes.push(watch(&mut me, "combat".to_owned(), Duration::from_secs(4))),
        Err(problem) => skipped("combat", &problem),
    }
    staged(&me, Staging::Place { place: home });
    match me.stage(&Staging::Talk) {
        Ok(()) => scenes.push(watch(
            &mut me,
            "conversation".to_owned(),
            Duration::from_secs(4),
        )),
        Err(problem) => skipped("conversation", &problem),
    }
    let areas = arrivals(content, home);
    for (name, place) in &areas {
        scenes.push(arrive(&mut me, format!("{name} arrive"), *place));
        scenes.push(watch(&mut me, name.clone(), Duration::from_secs(4)));
        match walkable_toward(content, *place, &WALKS) {
            Some(to) => scenes.push(walk(&mut me, format!("{name} walk"), to)),
            None => skipped(&format!("{name} walk"), "no open ground to walk to"),
        }
    }
    zoom_out(&mut me);
    for (name, place) in &areas {
        arrive(&mut me, String::new(), *place);
        scenes.push(watch(
            &mut me,
            format!("{name} wide"),
            Duration::from_secs(4),
        ));
    }
    scenes
}

/// How far a walk across an area goes, the farthest first that reaches open ground.
const WALKS: [Meters; 3] = [Meters(60.0), Meters(40.0), Meters(20.0)];

/// A place in each of the game's areas, each named by its file: home's where the player starts,
/// another's the first a warp leads into it at, or the open ground nearest its middle.
fn arrivals(content: &Content, home: Place) -> Vec<(String, Place)> {
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
fn warps_from(content: &Content, from: area::AreaId) -> impl Iterator<Item = Place> + '_ {
    let scripts = content.built::<RiftScripts>();
    area::warp_objects(content, from)
        .into_iter()
        .flat_map(move |(index, _)| scripts.may_do(OwnerRow::of_object(from, index)).reaches)
        .filter_map(|reach| match reach {
            EffectReach::Warp(to) => Some(to),
            _ => None,
        })
}

/// A scene the client draws, and how long what its view takes in was missing while the scenery
/// stood it, where that was watched.
struct Scene {
    name: String,
    frames: Frames,
    missing: Option<Duration>,
}

/// Lets the client play on for `lasting` without asking it anything, so what it does meanwhile is
/// drawing, not answering.
fn watch(me: &mut Playtest, name: String, lasting: Duration) -> Scene {
    me.snapshot();
    me.linger(lasting);
    Scene {
        name,
        frames: Frames::of(me.snapshot().frame_times),
        missing: None,
    }
}

/// Puts the player at `place`, from wherever they are, and watches until the scenery stands all
/// the view takes in there.
fn arrive(me: &mut Playtest, name: String, place: Place) -> Scene {
    me.snapshot();
    staged(me, Staging::Place { place });
    let mut frames = Frames::default();
    let mut missing = Duration::ZERO;
    let deadline = Instant::now() + ARRIVE_WITHIN;
    loop {
        let since = Instant::now();
        std::thread::sleep(POLL);
        let seen = me.snapshot();
        frames.extend(seen.frame_times.iter().copied());
        if !arrived(&seen, place) {
            assert!(Instant::now() < deadline, "never arrived at {place:?}");
            continue;
        }
        if !seen.scenery_standing {
            break;
        }
        missing += since.elapsed();
        assert!(
            Instant::now() < deadline,
            "the scenery never stood at {place:?}"
        );
    }
    Scene {
        name,
        frames,
        missing: Some(missing),
    }
}

/// Walks the player to `to` and watches them go, until they stop.
fn walk(me: &mut Playtest, name: String, to: Pos<Meters>) -> Scene {
    me.snapshot();
    staged(me, Staging::Walk { to });
    let mut frames = Frames::default();
    let mut missing = Duration::ZERO;
    let mut last = None;
    let deadline = Instant::now() + WALK_WITHIN;
    while Instant::now() < deadline {
        let since = Instant::now();
        std::thread::sleep(POLL);
        let seen = me.snapshot();
        frames.extend(seen.frame_times.iter().copied());
        if seen.scenery_standing {
            missing += since.elapsed();
        }
        let at = seen.me.map(|body| body.at);
        if at.is_some_and(|at| at.distance(to).0 < ARRIVED_WITHIN) || (at.is_some() && at == last) {
            break;
        }
        last = at;
    }
    Scene {
        name,
        frames,
        missing: Some(missing),
    }
}

const POLL: Duration = Duration::from_millis(100);
const ARRIVE_WITHIN: Duration = Duration::from_secs(120);
const WALK_WITHIN: Duration = Duration::from_secs(60);
const ARRIVED_WITHIN: f32 = 1.5;

/// Turns the mouse wheel away from the player past the widest zoom, and waits for the view to have
/// come to it.
fn zoom_out(me: &mut Playtest) {
    me.scroll(-i32::from(Zoom::STEPS));
    me.wait_until("the camera's widest zoom", |snapshot| {
        snapshot
            .view
            .as_ref()
            .is_some_and(|view| view.zoom == Zoom::Wide.step())
    });
    let ease = me.content().built::<GameSettings>().look.camera.zoom_ease;
    me.linger(Duration::from_secs_f32(ease.0 * EASES_TO_ARRIVE));
}

/// How many times its ease the camera takes to come within a hundredth of a zoom it eases to.
const EASES_TO_ARRIVE: f32 = 5.0;

fn skipped(scene: &str, why: &str) {
    println!("[client-bench] {scene:<16} not benched: {why}");
}

fn report(video: Video, scenes: &[Scene]) {
    for scene in scenes {
        let missing = scene
            .missing
            .map(|missing| format!("  missing {:.1} s", missing.as_secs_f64()))
            .unwrap_or_default();
        println!(
            "[client-bench] {:<8} {:<20} {}{missing}",
            video.name(),
            scene.name,
            scene.frames.summary()
        );
    }
    let slowest = scenes
        .iter()
        .filter(|scene| !scene.frames.is_empty())
        .min_by(|a, b| a.frames.slowest().total_cmp(&b.frames.slowest()))
        .expect("scenes were watched");
    let mut all = Frames::default();
    for scene in scenes {
        all.join(&scene.frames);
    }
    let missing: Duration = scenes.iter().filter_map(|scene| scene.missing).sum();
    println!(
        "[client-bench] RESULT video={} mean={:.0} low={:.0} slowest={:.0} ({}) floor={:.0} missing={:.1}s",
        video.name(),
        all.mean(),
        all.low(),
        slowest.frames.slowest(),
        slowest.name,
        video.floor(),
        missing.as_secs_f64()
    );
}

/// The open ground nearest the middle of `area`.
fn open_middle(content: &Content, area: area::AreaId) -> Option<Pos<Meters>> {
    let loaded = area::load(content, area);
    let middle = loaded.size.bounds().center();
    let _held = loaded.hold(loaded.tiles_around(middle, Meters(MIDDLE_REACH)));
    loaded.mesh().nearest(middle)
}

/// How far from an area's middle its open ground is looked for.
const MIDDLE_REACH: f32 = 64.0;

/// Open ground a walk from `from` reaches, as far off as the first of `distances` that has some,
/// in the first of eight directions round from east that does.
fn walkable_toward(content: &Content, from: Place, distances: &[Meters]) -> Option<Pos<Meters>> {
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
