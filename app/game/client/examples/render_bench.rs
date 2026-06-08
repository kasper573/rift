//! Renderer benchmarks mirroring the world bench: the frame pipeline a playing client runs
//! every frame (`build_scene` + `rasterize`), measured headless over a live replicated
//! session — everything except the GPU texture upload, which needs a window.
//! Run: `cargo run --release --example render_bench -p client`
//! Profilers: `cargo run --release --example render_bench -p client prof [frames]`

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::hint::black_box;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use client::render::{self, Animator};
use rift::{ClientId, Cluster};
use world::features::npc;
use world::{Identity, LinkStatus, MmoClient, Owner, Transport, Vitals};

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
const WARMUP_FRAMES: u32 = 200;
const FRAMES: u32 = 2000;
// NPCs spawned across the spawn area; the view only replicates the nearby subset.
const NPC_SCENARIOS: [usize; 4] = [0, 25, 100, 200];
// Enough ticks for join, NPC spread, and replication to settle.
const WARMUP_TICKS: u32 = 60;

fn main() {
    if let Some(pos) = std::env::args().position(|a| a == "prof") {
        let frames: u32 = std::env::args()
            .nth(pos + 1)
            .and_then(|a| a.parse().ok())
            .unwrap_or(100_000);
        let (client, mut animator) = warmed(200);
        let mut frame = image::Image::new(render::VIEW.x.0 as u32, render::VIEW.y.0 as u32);
        for i in 0..frames {
            let time = i as f32 / world::TICK_HZ;
            let scene = render::build_scene(&client, time, &mut animator);
            render::rasterize(&scene, &mut frame);
            black_box(&frame);
        }
        return;
    }

    println!(
        "== RENDER  (build_scene + rasterize, {}x{}; {FRAMES} frames; spawn area) ==",
        render::VIEW.x.0,
        render::VIEW.y.0,
    );
    for &npcs in &NPC_SCENARIOS {
        let (client, mut animator) = warmed(npcs);
        let actors = client.world().iter::<world::Actor>().count();
        let mut frame = image::Image::new(render::VIEW.x.0 as u32, render::VIEW.y.0 as u32);

        let mut scene_ns = Vec::with_capacity(FRAMES as usize);
        let mut raster_ns = Vec::with_capacity(FRAMES as usize);
        let mut a0 = (0, 0);
        for i in 0..WARMUP_FRAMES + FRAMES {
            if i == WARMUP_FRAMES {
                a0 = (
                    ALLOCS.load(Ordering::Relaxed),
                    ALLOC_BYTES.load(Ordering::Relaxed),
                );
            }
            let time = i as f32 / world::TICK_HZ;
            let t = Instant::now();
            let scene = render::build_scene(&client, time, &mut animator);
            let built = t.elapsed();
            render::rasterize(&scene, &mut frame);
            if i >= WARMUP_FRAMES {
                scene_ns.push(built.as_nanos());
                raster_ns.push((t.elapsed() - built).as_nanos());
            }
            black_box(&frame);
        }
        let allocs = (ALLOCS.load(Ordering::Relaxed) - a0.0) / u64::from(FRAMES);
        let abytes = (ALLOC_BYTES.load(Ordering::Relaxed) - a0.1) / u64::from(FRAMES);
        scene_ns.sort_unstable();
        raster_ns.sort_unstable();
        println!(
            "  {npcs:>3} NPCs (actors in view {actors:>3}): scene mean {:>5.1}us p95 {:>5.1} | raster mean {:>6.1}us p95 {:>6.1} | {allocs:>3} allocs {abytes:>7}B/frame",
            mean(&scene_ns),
            percentile(&scene_ns, 95.0),
            mean(&raster_ns),
            percentile(&raster_ns, 95.0),
        );
    }
}

/// A spawned, immortal player on a live cluster with `npcs` NPCs in its area, replication
/// settled and the world then frozen: samples measure rendering, not simulation.
fn warmed(npcs: usize) -> (MmoClient, Animator) {
    let dt = 1.0 / world::TICK_HZ;
    let mut cluster = Cluster::new(&world::features(), &world::zones(), world::spawn_zone());
    let id = ClientId(1);
    cluster.connect_as(
        id,
        Some(Arc::new(Identity {
            id: "bench".into(),
            name: "bench".into(),
            roles: vec![],
        })),
    );
    let zone = world::spawn_zone();
    if let Some(server) = cluster.server_mut(zone) {
        npc::spawn_npcs(
            &mut server.world,
            SEED,
            npcs,
            &[world::core::area::AreaId(zone)],
        );
    }

    let shared = Rc::new(RefCell::new(Shared::default()));
    let mut client = MmoClient::with_transport(Box::new(Loopback(Rc::clone(&shared))));
    client.join();
    for _ in 0..WARMUP_TICKS {
        loop {
            let packet = shared.borrow_mut().to_server.pop_front();
            match packet {
                Some(packet) => cluster.receive(id, &packet),
                None => break,
            }
        }
        for (to, packet) in cluster.tick(dt) {
            if to == id {
                shared.borrow_mut().to_client.push_back(packet);
            }
        }
        client.poll();
        if let Some(server) = cluster.server_mut(zone) {
            for entity in server.world.ids::<Owner>() {
                server.world.modify::<Vitals>(entity, |v| {
                    v.health = f32::MAX;
                    v.max = f32::MAX;
                });
            }
        }
    }
    assert!(client.my_entity().is_some(), "bench player must spawn");
    (client, Animator::default())
}

#[derive(Default)]
struct Shared {
    to_server: VecDeque<Vec<u8>>,
    to_client: VecDeque<Vec<u8>>,
}

struct Loopback(Rc<RefCell<Shared>>);

impl Transport for Loopback {
    fn send(&mut self, packet: &[u8]) {
        self.0.borrow_mut().to_server.push_back(packet.to_vec());
    }
    fn poll(&mut self, sink: &mut dyn FnMut(&[u8])) {
        loop {
            let packet = self.0.borrow_mut().to_client.pop_front();
            match packet {
                Some(packet) => sink(&packet),
                None => break,
            }
        }
    }
    fn status(&self) -> LinkStatus {
        LinkStatus::Open
    }
}

fn mean(sorted: &[u128]) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted.iter().sum::<u128>() as f64 / sorted.len() as f64 / 1000.0
}

fn percentile(sorted: &[u128], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)] as f64 / 1000.0 // ns -> us
}
