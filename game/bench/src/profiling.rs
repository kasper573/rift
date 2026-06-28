use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

use bevy_app::App;
use bevy_ecs::prelude::{Entity, With};
use bevy_replicon::prelude::Replicated;
use world::data;
use world::systems::movement::Position;
use world::systems::visibility::seen_by;

const AREAS: usize = 250; // near the current budget crossover: the representative heavy workload
const WARMUP: usize = 30;
const MEASURE: usize = 400;

/// Profiles a fixed `AREAS`-area workload with clients connected (the replication-heavy "full"
/// config), writing a CPU flamegraph + pprof protobuf and reporting allocation churn per tick. Uses
/// the exact same [`crate::sim`] world construction the benchmark does, so the two never diverge.
pub fn run(out_dir: &Path) {
    let npc = data::npc::Id::Orc;
    let assets = crate::assets::service();
    let (mut worlds, rosters) = crate::sim::worlds(AREAS, npc, &assets);
    crate::sim::connect(&mut worlds, &rosters);

    for _ in 0..WARMUP {
        crate::sim::step(&mut worlds);
    }

    report_aoi(&mut worlds);

    let guard = pprof::ProfilerGuardBuilder::default()
        .frequency(999)
        .blocklist(&["libc", "libgcc", "pthread", "vdso"])
        .build()
        .expect("start pprof profiler");
    ALLOCS.store(0, Relaxed);
    BYTES.store(0, Relaxed);
    let started = Instant::now();
    for _ in 0..MEASURE {
        crate::sim::step(&mut worlds);
    }
    let elapsed = started.elapsed();
    let allocs = ALLOCS.load(Relaxed);
    let bytes = BYTES.load(Relaxed);

    let report = guard.report().build().expect("build pprof report");
    print_hotspots(&report);
    let svg = out_dir.join("flamegraph.svg");
    report
        .flamegraph(File::create(&svg).expect("create flamegraph"))
        .expect("write flamegraph");
    {
        use pprof::protos::Message;
        let profile = report.pprof().expect("pprof profile");
        let buf = profile.write_to_bytes().expect("encode pprof");
        File::create(out_dir.join("profile.pb"))
            .expect("create profile.pb")
            .write_all(&buf)
            .expect("write profile.pb");
    }

    let ticks = MEASURE as f64;
    let per_tick = elapsed.as_secs_f64() * 1000.0 / ticks;
    println!("[profile] {AREAS} areas, full config (clients connected), {MEASURE} ticks");
    println!(
        "[profile] {per_tick:.2}ms/tick wall ({:.2}s total)",
        elapsed.as_secs_f64()
    );
    println!(
        "[profile] allocations: {:.0}/tick ({} total), {:.2} MiB/tick churn",
        allocs as f64 / ticks,
        allocs,
        bytes as f64 / ticks / (1024.0 * 1024.0),
    );
    if let Some(hwm) = peak_rss_kib() {
        println!("[profile] peak RSS: {:.1} MiB", hwm as f64 / 1024.0);
    }
    println!("[profile] flamegraph: {}", svg.display());
}

/// Quantifies area-of-interest overlap in the (distributed-player) scenario: how many clients see
/// each replicated entity, and how many distinct AOIs exist. High avg viewers/entity means the
/// per-(client,entity) replication work is heavily redundant and an AOI-dedup optimization would
/// pay; ~1 means it would not. Uses the real `visibility::seen_by`, so it reflects the live rules.
fn report_aoi(worlds: &mut [App]) {
    let mut subjects = 0u64;
    let mut viewer_pairs = 0u64; // sum over entities of viewer count = sum over clients of AOI size
    let mut max_viewers = 0usize;
    let mut clients = 0u64;
    let mut distinct_aois = 0u64;
    for app in worlds.iter_mut() {
        let world = app.world_mut();
        let entities: Vec<Entity> = world
            .query_filtered::<Entity, (With<Replicated>, With<Position>)>()
            .iter(world)
            .collect();
        let mut aoi: HashMap<Entity, Vec<u64>> = HashMap::new();
        for &entity in &entities {
            let viewers = seen_by(world, entity);
            subjects += 1;
            viewer_pairs += viewers.len() as u64;
            max_viewers = max_viewers.max(viewers.len());
            for client in viewers {
                aoi.entry(client).or_default().push(entity.to_bits());
            }
        }
        clients += aoi.len() as u64;
        let mut sets: Vec<Vec<u64>> = aoi
            .into_values()
            .map(|mut set| {
                set.sort_unstable();
                set
            })
            .collect();
        sets.sort();
        sets.dedup();
        distinct_aois += sets.len() as u64;
    }
    let avg_viewers = viewer_pairs as f64 / subjects.max(1) as f64;
    let avg_aoi = viewer_pairs as f64 / clients.max(1) as f64;
    println!(
        "[aoi] avg viewers/entity: {avg_viewers:.2}  (per-entity replication work repeated this many times)"
    );
    println!(
        "[aoi] avg AOI size: {avg_aoi:.1} entities/client, max viewers on one entity: {max_viewers}"
    );
    println!(
        "[aoi] distinct AOI permutations: {distinct_aois} of {clients} viewing clients ({:.1}% unique)",
        100.0 * distinct_aois as f64 / clients.max(1) as f64,
    );
}

/// Aggregates the pprof samples into self time per owning system and per leaf symbol — the CPU side
/// of the profile in text form.
fn print_hotspots(report: &pprof::Report) {
    let mut total = 0i64;
    let mut self_time: HashMap<String, i64> = HashMap::new();
    let mut by_system: HashMap<String, i64> = HashMap::new();
    for (frames, &count) in &report.data {
        let count = count as i64;
        total += count;
        if let Some(leaf) = frames.frames.first().and_then(|group| group.first()) {
            *self_time.entry(symbol(leaf)).or_default() += count;
        }
        let owner = frames
            .frames
            .iter()
            .rev()
            .flat_map(|group| group.iter().rev())
            .find_map(|sym| attribute(&symbol(sym)))
            .unwrap_or_else(|| "other".to_string());
        *by_system.entry(owner).or_default() += count;
    }
    println!("\n[profile] CPU self time by owning system (of {total} samples):");
    print_ranked(&by_system, total);
    println!("\n[profile] CPU self time by leaf (top 25):");
    print_ranked(&self_time, total);
    println!();
}

/// The owning system/subsystem for a frame, scanning a stack root-first so the highest-level owner
/// wins (e.g. nav::astar samples attribute to movement::advance).
fn attribute(name: &str) -> Option<String> {
    for prefix in ["world::systems::", "world::core::"] {
        if let Some(rest) = name.strip_prefix(prefix) {
            let trimmed: String = rest.split('<').next().unwrap_or(rest).to_string();
            return Some(format!("{prefix}{trimmed}"));
        }
    }
    if name.starts_with("bevy_replicon::") {
        return Some("bevy_replicon (replication)".to_string());
    }
    None
}

fn symbol(sym: &pprof::Symbol) -> String {
    let name = sym.name();
    name.split("::{{closure}}")
        .next()
        .unwrap_or(&name)
        .to_string()
}

fn print_ranked(map: &HashMap<String, i64>, total: i64) {
    let mut ranked: Vec<_> = map.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1));
    for (name, &count) in ranked.into_iter().take(25) {
        println!(
            "[profile]   {:5.1}%  {}",
            100.0 * count as f64 / total as f64,
            name
        );
    }
}

fn peak_rss_kib() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("VmHWM:"))
                .and_then(|rest| rest.split_whitespace().next())
                .and_then(|kib| kib.parse().ok())
        })
}

#[global_allocator]
static ALLOC: Counting = Counting;

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(layout.size() as u64, Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        if new_size > layout.size() {
            BYTES.fetch_add((new_size - layout.size()) as u64, Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}
