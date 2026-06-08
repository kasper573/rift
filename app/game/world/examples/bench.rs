use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rift::{ClientId, Cluster, Feature};
use world::core::area;
use world::features::{actions, combat, movement, npc, player, regen, replication, visibility};
use world::{JoinRequest, Owner, Vitals};

// Counting allocator for memory profiling (bench-only): forwards to System, tallying allocations.
#[global_allocator]
static ALLOC: Counting = Counting;
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

struct Counting;
// SAFETY: every method forwards unchanged to the System allocator; we only bump relaxed counters.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

const SEED: u64 = 0x1234_5678_9abc_def0;
const TICK_HZ: f64 = 20.0;
const WARMUP: u32 = 120;
const SIM_TICKS: u32 = 200; // 10 s @ 20 Hz
const NPC_SCENARIOS: [usize; 3] = [25, 100, 200];
// How many areas the sim world has; scatter scenarios spread entities across all of them.
const MAX_AREAS: usize = 16;
// Upper-bound probe: scale areas (each 25 players + 75 NPCs) until a tick misses the frame budget.
const BOUND_MAX_AREAS: usize = 512;
// (NPCs, clients, areas) — congestion (1 area) vs scatter (MAX_AREAS) at low/mid/high scale.
const PERF_SCENARIOS: [(usize, u32, usize); 8] = [
    (200, 8, 1),
    (200, 8, MAX_AREAS),
    (1000, 50, 1),
    (1000, 50, MAX_AREAS),
    (5000, 200, 1),
    (5000, 200, MAX_AREAS),
    (5000, 1, 1),
    (1000, 200, MAX_AREAS),
];

fn dt() -> f32 {
    (1.0 / TICK_HZ) as f32
}

// The full gameplay feature set minus `npc::spawner` — the bench picks the NPC count itself.
fn bench_features() -> Vec<Feature> {
    vec![
        replication::feature,
        actions::feature,
        regen::feature,
        npc::ai,
        movement::input,
        combat::feature,
        movement::step,
        player::feature,
        npc::respawn,
        visibility::feature,
    ]
}

// A cluster of `spread` shards (one area each), holding `npc_count` NPCs spread evenly across them
// and `players` immortal observers distributed round-robin. spread=1 is congestion (one shard, no
// parallelism); spread>1 is scatter (shards tick in parallel).
fn warmed(npc_count: usize, players: u32, warmup: u32, spread: usize) -> Cluster {
    let dt = dt();
    let spread = spread.max(1);
    let zones: Vec<u32> = (0..spread as u32).collect();
    let mut cluster = Cluster::new(&bench_features(), &zones, 0);
    for (i, &zone) in zones.iter().enumerate() {
        let count = npc_count / spread + usize::from(i < npc_count % spread);
        if let Some(server) = cluster.server_mut(zone) {
            npc::spawn_npcs(&mut server.world, SEED, count, &[area::AreaId(zone)]);
        }
    }
    for i in 0..players {
        let client = ClientId(i + 1);
        let zone = i % spread as u32;
        cluster.connect_to(client, zone);
        if let Some(server) = cluster.server_mut(zone) {
            server.inject(client, &JoinRequest {});
        }
    }
    let _ = cluster.tick(dt); // join intents spawn the players
    for &zone in &zones {
        if let Some(server) = cluster.server_mut(zone) {
            for entity in server.world.ids::<Owner>() {
                server.world.modify::<Vitals>(entity, |v| {
                    v.health = f32::MAX;
                    v.max = f32::MAX;
                });
            }
        }
    }
    for _ in 0..warmup {
        let _ = cluster.tick(dt);
    }
    cluster
}

fn entity_count(cluster: &Cluster, spread: usize) -> usize {
    (0..spread)
        .filter_map(|z| cluster.server(z as u32))
        .map(|server| server.world.entity_count())
        .sum()
}

fn percentile(sorted: &[u128], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)] as f64 / 1000.0 // ns -> us
}

fn main() {
    let dt = dt();
    if std::env::args().any(|a| a == "bound") {
        run_bound();
        return;
    }
    let full = std::env::args().any(|a| a == "--full");
    area::configure_areas(MAX_AREAS);

    if let Some(pos) = std::env::args().position(|a| a == "prof") {
        // `prof [npcs] [clients] [spread] [ticks]` — a long single-scenario run for profilers.
        let nums: Vec<u32> = std::env::args()
            .skip(pos + 1)
            .filter_map(|a| a.parse().ok())
            .collect();
        let npc = nums.first().copied().unwrap_or(5000) as usize;
        let clients = nums.get(1).copied().unwrap_or(1);
        let spread = nums.get(2).copied().unwrap_or(1) as usize;
        let ticks = nums.get(3).copied().unwrap_or(2000);
        let mut cluster = warmed(npc, clients, 60, spread);
        for _ in 0..ticks {
            let _ = cluster.tick(dt);
        }
        return;
    }

    println!(
        "== PERF  (cluster.tick, shards in parallel; {SIM_TICKS} ticks @ {TICK_HZ:.0}Hz; NPC x clients x areas) =="
    );
    for &(npc, clients, spread) in &PERF_SCENARIOS {
        let mut cluster = warmed(npc, clients, WARMUP, spread);
        let mut samples = Vec::with_capacity(SIM_TICKS as usize);
        let (a0, by0) = (
            ALLOCS.load(Ordering::Relaxed),
            ALLOC_BYTES.load(Ordering::Relaxed),
        );
        for _ in 0..SIM_TICKS {
            let a = Instant::now();
            let _ = cluster.tick(dt);
            samples.push(a.elapsed().as_nanos());
        }
        let allocs = (ALLOCS.load(Ordering::Relaxed) - a0) / u64::from(SIM_TICKS);
        let abytes = (ALLOC_BYTES.load(Ordering::Relaxed) - by0) / u64::from(SIM_TICKS);
        samples.sort_unstable();
        let mean = samples.iter().sum::<u128>() as f64 / samples.len() as f64 / 1000.0;
        println!(
            "  {npc:>4} NPC x {clients:>3} cli x {spread:>2} area (ents {:>5}): mean {mean:8.1}us  p95 {:7.1}  | {allocs:>5} allocs {abytes:>8}B/tick",
            entity_count(&cluster, spread),
            percentile(&samples, 95.0),
        );
    }

    println!(
        "\n== PACKETS  (outbound replication bytes; {SIM_TICKS} ticks @ {TICK_HZ:.0}Hz, 1 player) =="
    );
    for &n in &NPC_SCENARIOS {
        let mut cluster = warmed(n, 1, WARMUP, 1);
        let (mut bytes, mut packets, mut max) = (0usize, 0usize, 0usize);
        for _ in 0..SIM_TICKS {
            for (_id, packet) in cluster.tick(dt) {
                bytes += packet.len();
                packets += 1;
                max = max.max(packet.len());
            }
        }
        let secs = f64::from(SIM_TICKS) / TICK_HZ;
        println!(
            "  {n:>4} NPCs: {packets} pkts, {bytes}B total, avg {:.0}B/pkt, max {max}B, {:.0}B/s",
            bytes as f64 / packets.max(1) as f64,
            bytes as f64 / secs,
        );
    }

    if !full {
        return;
    }
    println!("\n== BOUNDS  (max NPCs holding p95 tick <= 50ms, doubling probe) ==");
    for &players in &[1u32, 25, 100] {
        let cap = 6_400usize;
        let mut bound = 0usize;
        let mut probe = 25usize;
        while probe <= cap {
            if probe_p95(probe, players) > Duration::from_millis(50) {
                break;
            }
            bound = probe;
            probe *= 2;
        }
        let suffix = if bound >= cap { "+ (probe cap)" } else { "" };
        println!("  {players:>3} players: ~{bound}{suffix} NPCs");
    }
}

fn probe_p95(npc_count: usize, players: u32) -> Duration {
    let dt = dt();
    let mut cluster = warmed(npc_count, players, 20, 1);
    let mut samples = Vec::with_capacity(30);
    for _ in 0..30 {
        let t = Instant::now();
        let _ = cluster.tick(dt);
        samples.push(t.elapsed());
    }
    samples.sort_unstable();
    samples[((0.95 * (samples.len() as f64 - 1.0)).round() as usize).min(samples.len() - 1)]
}

// Scale the world by `areas`, each populated with 25 players + 75 NPCs, until a tick's p95 misses
// the frame budget. Reports the largest world (and so the most concurrent players) that holds up.
fn run_bound() {
    area::configure_areas(BOUND_MAX_AREAS);
    println!("== UPPER BOUND  (25 players + 75 NPCs per area; frame budget 50ms @ 20Hz) ==");
    let mut areas = 1;
    while areas <= BOUND_MAX_AREAS {
        let p95 = probe_areas(areas);
        let drops = p95 > Duration::from_millis(50);
        println!(
            "  {areas:>4} areas | {:>6} players + {:>6} NPCs = {:>7} ents | p95 {:8.2}ms {}",
            25 * areas,
            75 * areas,
            100 * areas,
            p95.as_secs_f64() * 1000.0,
            if drops { "<-- FRAME DROP" } else { "ok" },
        );
        if drops {
            break;
        }
        areas *= 2;
    }
}

fn probe_areas(areas: usize) -> Duration {
    let dt = dt();
    let mut cluster = warmed(75 * areas, (25 * areas) as u32, 40, areas);
    let mut samples = Vec::with_capacity(60);
    for _ in 0..60 {
        let t = Instant::now();
        let _ = cluster.tick(dt);
        samples.push(t.elapsed());
    }
    samples.sort_unstable();
    samples[((0.95 * (samples.len() as f64 - 1.0)).round() as usize).min(samples.len() - 1)]
}
