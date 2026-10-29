// The memory the game's processes hold as the system sees it, and what the client holds on the GPU,
// while a player crosses back and forth between two areas and then stands still: an area's
// footprint must go when the player leaves it, so once a few crossings have grown the caches and
// pools that are sized to the busiest moment, each visit to an area holds what the first did, and
// standing still costs nothing. Standing still may still grow the GPU's mesh buffers once, to fit
// a mesh first seen there.

use std::path::Path;
use std::time::{Duration, SystemTime};

use driver::{Pace, Player, Playtest, PlaytestSetup};
use engine::core::units::Bytes;
use engine::systems::area::AreaId;

// Runs play out alike, so they compare.
const SEED: u64 = 1931;
const WARM_UP: usize = 3;
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
// What the GPU's buffers and textures may differ by from visit to visit, as a frame's draws differ.
const GPU_SLACK: Bytes = Bytes(256 * 1024);

fn main() {
    let mut playtest = Playtest::start(PlaytestSetup {
        server_binary: env!("CARGO_BIN_EXE_game-server").into(),
        client_binary: env!("CARGO_BIN_EXE_desktop-client").into(),
        content: Path::new(env!("CARGO_MANIFEST_DIR")).join("../example_game"),
        folder: Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("memory-bench")
            .join(std::process::id().to_string()),
        pace: Pace::Brisk,
        seed: Some(SEED),
        recorded: false,
    });
    let mut me = playtest.join_as("traveler", None);
    let home = me.in_world().area.expect("the player is in an area");
    let away = me
        .snapshot()
        .warps
        .iter()
        .map(|warp| warp.to)
        .find(|area| *area != home)
        .expect("a warp to another area");
    for _ in 0..WARM_UP {
        cross_into(&mut me, away);
        cross_into(&mut me, home);
    }
    let mut visits: Vec<(AreaId, Reading)> = vec![(home, settle(&mut me, &playtest))];
    for _ in 0..ROUND_TRIPS {
        for area in [away, home] {
            cross_into(&mut me, area);
            visits.push((area, settle(&mut me, &playtest)));
        }
    }
    let still = {
        let before = settle(&mut me, &playtest);
        me.linger_until(SystemTime::now() + STILL);
        (before, read(&mut me, &playtest))
    };
    let mut broken = Vec::new();
    for area in [home, away] {
        let held: Vec<Reading> = visits
            .iter()
            .filter(|(visited, _)| *visited == area)
            .map(|(_, reading)| *reading)
            .collect();
        let (first, last) = (held[0], held[held.len() - 1]);
        let name = &area.get(driver::content()).name;
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
    assert!(broken.is_empty(), "memory grows:\n{}", broken.join("\n"));
    println!("[memory-bench] RESULT ok");
}

#[derive(Clone, Copy)]
struct Reading {
    client: Bytes,
    server: Bytes,
    gpu: Bytes,
}

fn settle(me: &mut Player, playtest: &Playtest) -> Reading {
    me.linger_until(SystemTime::now() + SETTLE);
    read(me, playtest)
}

fn read(me: &mut Player, playtest: &Playtest) -> Reading {
    Reading {
        gpu: me.snapshot().gpu_memory,
        client: me.client_memory(),
        server: playtest.server_memory(),
    }
}

fn grown(what: &str, first: Bytes, last: Bytes) -> Option<String> {
    let slack = SLACK_LEAST.max(Bytes((first.0 as f64 * SLACK_SHARE) as u64));
    (last.0 > first.0 + slack.0).then(|| {
        format!(
            "{what}: {:.0} MB, from {:.0} MB (at most {:.0} MB more)",
            last.megabytes(),
            first.megabytes(),
            slack.megabytes()
        )
    })
}

fn gpu_grown(what: &str, first: Bytes, last: Bytes) -> Option<String> {
    (last.0 > first.0 + GPU_SLACK.0).then(|| {
        format!(
            "{what}: {} bytes, from {} bytes (at most {} more)",
            last.0, first.0, GPU_SLACK.0
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

// Through the nearest warp that leads there.
fn cross_into(me: &mut Player, area: AreaId) {
    let seen = me.snapshot();
    let here = seen.me.as_ref().expect("the player").at;
    let warp = seen
        .warps
        .iter()
        .filter(|warp| warp.to == area)
        .min_by(|a, b| a.at.distance_to(here).total_cmp(&b.at.distance_to(here)))
        .expect("a warp there")
        .name
        .clone();
    me.cross(&warp);
}
