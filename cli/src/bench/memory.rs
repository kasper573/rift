//! The memory the game's processes hold as the system sees it, and what the client holds on the GPU,
//! while the player crosses back and forth between the spawn area and the area its first warp leads
//! to, and then stands still: an area's footprint must go when the player leaves it, so once a few
//! crossings have grown the caches and pools that are sized to the busiest moment, each visit to an
//! area holds what the first did, and standing still costs nothing. Standing still may still grow
//! the GPU's mesh buffers once, to fit a mesh first seen there.

use std::path::Path;
use std::time::Duration;

use engine::core::content::Content;
use engine::core::units::Bytes;
use engine::systems::script::Place;
use engine::systems::staging::Staging;

use super::client::{arrived, away_from, settled_in_world, staged};
use super::playtest::Playtest;

// The caches and pools sized to the busiest moment fill over a varying number of crossings, the
// graphics driver's command memory slowest of all, so the visits are measured once a few round
// trips in a row grow the client by no more than the measured ones may on average: one quiet
// trip proves nothing while the pools still fill. One that never settles leaks.
const MOST_WARM_UPS: usize = 12;
const QUIET_ROUND_TRIPS: usize = 2;
const ROUND_TRIPS: usize = 4;
// Long enough for an arrival's loading to pass, and for the allocator to hand back what the loading
// churned through.
const SETTLE: Duration = Duration::from_secs(12);
const STILL: Duration = Duration::from_secs(20);
// What the process may keep beyond what the game holds: what its allocator holds on to, and the
// command memory the graphics driver keeps for each of wgpu's pooled command encoders at the most
// it ever recorded, which only fills up over dozens of crossings.
const SLACK_SHARE: f64 = 0.05;
const SLACK_LEAST: Bytes = Bytes(64 * 1024 * 1024);
// What the GPU's buffers and textures may differ by from visit to visit: a frame's draws differ,
// and the meshes stand in slabs that grow by half when what a visit stands outgrows one, which
// differs with where the view is from, so a visit may hold a slab's growth more than another.
const GPU_SLACK_SHARE: f64 = 0.02;
const GPU_SLACK_LEAST: Bytes = Bytes(256 * 1024);

/// Fails naming what grew.
pub fn run(game: &Path, content: &Content) -> Result<(), String> {
    let mut me = Playtest::start(game, content);
    let home = settled_in_world(&mut me);
    let Some(away) = away_from(&me, home) else {
        println!("[memory-bench] not benched: no warp out of the spawn area");
        return Ok(());
    };
    let mut warm = settle(&mut me);
    let mut warm_ups = 0;
    let mut quiet = 0;
    loop {
        if warm_ups == MOST_WARM_UPS {
            return Err(format!(
                "[memory-bench] memory never settles: client {:.0} MB after {MOST_WARM_UPS} round trips",
                warm.client.megabytes()
            ));
        }
        let before = warm;
        for place in [away, home] {
            cross_into(&mut me, place);
            warm = settle(&mut me);
        }
        warm_ups += 1;
        let allowed = slack(before.client).0 / ROUND_TRIPS as u64;
        quiet = if warm.client.0 <= before.client.0 + allowed {
            quiet + 1
        } else {
            0
        };
        if quiet == QUIET_ROUND_TRIPS {
            break;
        }
    }
    println!("[memory-bench] settled after {warm_ups} round trips");
    let mut visits: Vec<(Place, Reading)> = vec![(home, warm)];
    for _ in 0..ROUND_TRIPS {
        for place in [away, home] {
            cross_into(&mut me, place);
            visits.push((place, settle(&mut me)));
        }
    }
    let still = {
        let before = settle(&mut me);
        me.linger(STILL);
        (before, read(&mut me))
    };
    let mut broken = Vec::new();
    for place in [home, away] {
        let held: Vec<Reading> = visits
            .iter()
            .filter(|(visited, _)| visited.area == place.area)
            .map(|(_, reading)| *reading)
            .collect();
        let (first, last) = (held[0], held[held.len() - 1]);
        let name = &place.area.get(content).name;
        report(&format!("{name}, visit by visit"), &held);
        broken.extend(grown(&format!("{name}: client"), first.client, last.client));
        broken.extend(grown(&format!("{name}: server"), first.server, last.server));
        broken.extend(gpu_grown(&format!("{name}: gpu"), first.gpu, last.gpu));
    }
    report(
        &format!("standing still for {STILL:?}"),
        &[still.0, still.1],
    );
    broken.extend(grown(
        "standing still: client",
        still.0.client,
        still.1.client,
    ));
    broken.extend(grown(
        "standing still: server",
        still.0.server,
        still.1.server,
    ));
    if !broken.is_empty() {
        return Err(format!(
            "[memory-bench] memory grows:\n{}",
            broken.join("\n")
        ));
    }
    println!("[memory-bench] RESULT ok");
    Ok(())
}

#[derive(Clone, Copy)]
struct Reading {
    client: Bytes,
    server: Bytes,
    gpu: Bytes,
}

fn cross_into(me: &mut Playtest, place: Place) {
    staged(me, Staging::Place { place });
    me.wait_until("the crossing", |snapshot| arrived(snapshot, place));
}

fn settle(me: &mut Playtest) -> Reading {
    me.linger(SETTLE);
    read(me)
}

fn read(me: &mut Playtest) -> Reading {
    Reading {
        gpu: me.snapshot().gpu_memory,
        client: me.client_memory(),
        server: me.server_memory(),
    }
}

fn grown(what: &str, first: Bytes, last: Bytes) -> Option<String> {
    let slack = slack(first);
    (last.0 > first.0 + slack.0).then(|| {
        format!(
            "{what}: {:.0} MB, from {:.0} MB (at most {:.0} MB more)",
            last.megabytes(),
            first.megabytes(),
            slack.megabytes()
        )
    })
}

fn slack(held: Bytes) -> Bytes {
    SLACK_LEAST.max(Bytes((held.0 as f64 * SLACK_SHARE) as u64))
}

fn gpu_grown(what: &str, first: Bytes, last: Bytes) -> Option<String> {
    let slack = GPU_SLACK_LEAST.max(Bytes((first.0 as f64 * GPU_SLACK_SHARE) as u64));
    (last.0 > first.0 + slack.0).then(|| {
        format!(
            "{what}: {} bytes, from {} bytes (at most {} more)",
            last.0, first.0, slack.0
        )
    })
}

fn report(what: &str, held: &[Reading]) {
    let column = |of: fn(&Reading) -> Bytes| {
        held.iter()
            .map(|reading| format!("{:.0}", of(reading).megabytes()))
            .collect::<Vec<_>>()
            .join(" ")
    };
    println!(
        "[memory-bench] {what}: client {} MB, server {} MB, gpu {} MB",
        column(|reading| reading.client),
        column(|reading| reading.server),
        column(|reading| reading.gpu)
    );
}
