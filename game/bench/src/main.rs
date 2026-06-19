//! Zero-config load benchmark for the authoritative `world` simulation: it finds, on its own, the
//! highest number of areas the server sustains within the per-tick frame budget and reports it.
//!
//! The world is modelled as A areas (the content maps reused under fresh ids via
//! `world::area::configure_areas`) of 25 NPCs + 25 players each, every player backed by an in-process
//! `ConnectedClient` (replicon's headless-test shape — exercises the real area+range visibility cull and
//! replicon serialization with no socket). The program builds a fresh app per candidate A, drives
//! `app.update()` like the real server loop (`game/server/src/main.rs::simulate`), times the ticks, and
//! ramps A (exponential bracket, then binary search) until the mean tick crosses the budget. It reports
//! the largest sustained A and its area/player/NPC totals, plus the sim-vs-replication split at that point.
//!
//! Players spread evenly across areas, so the per-area replication cull is exercised: this blueprint
//! server culls by area+range, the golden reference the (unculled, ~O(A^2)) rift server is compared to.
//!
//! Run:      cargo run -p bench --release          (no arguments)
//! Profile:  cargo flamegraph -p bench             ;  heaptrack target/release/bench

use std::time::Instant;

use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{ConnectedClient, Replicated, ServerState};
use bevy_state::prelude::NextState;
use world::area::{self, Area};
use world::math::{Direction, Pos, Tiles};
use world::sim::Character;
use world::sim::combat::Stats;
use world::sim::movement::Speed;
use world::sim::npc::{self, Npc, NpcDef};
use world::sim::player::Players;
use world::sim::visibility::OwnedBy;
use world::table::Id;
use world::{
    ACTION_IDLE, Actor, AreaTag, ClientId, Hitbox, Inventory, Name, Owner, Position, Vitals, Xp,
};

const NPCS_PER_AREA: usize = 25;
const PLAYERS_PER_AREA: usize = 25;
const BUDGET_MS: f64 = 40.0;
const MAX_AREAS: usize = 128; // size of the reusable area pool; the ramp never needs more than this here
const WARMUP: usize = 30;
const MEASURE: usize = 200; // one window for every candidate, so the reported number is the one decided on

fn main() {
    world::assets::init(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"));
    area::configure_areas(MAX_AREAS);
    world::sim::validate();

    println!("[bench] finding the highest A sustained within the {BUDGET_MS:.0}ms budget...");

    let mut good = 0usize;
    let mut bad = 0usize;
    let mut best: Option<Point> = None;
    let consider = |areas: usize, good: &mut usize, best: &mut Option<Point>| -> bool {
        let p = point(areas, WARMUP, MEASURE);
        let pass = p.full <= BUDGET_MS;
        println!(
            "[bench]   A={areas:<4} mean={:6.2}ms  {}",
            p.full,
            verdict(p.full)
        );
        if pass {
            *good = areas;
            *best = Some(p);
        }
        pass
    };

    // Exponential bracket: double A until the mean tick exceeds budget.
    let mut a = 1usize;
    loop {
        if consider(a, &mut good, &mut best) {
            if a >= MAX_AREAS {
                break;
            }
            a = (a * 2).min(MAX_AREAS);
        } else {
            bad = a;
            break;
        }
    }

    // Binary search the crossover between the last good and first bad A.
    while bad > good + 1 {
        let mid = (good + bad) / 2;
        if !consider(mid, &mut good, &mut best) {
            bad = mid;
        }
    }

    let areas = good.max(1);
    let r = best.unwrap_or_else(|| point(areas, WARMUP, MEASURE));
    println!("\n[bench] areas,npcs,players,clients,mean_ms,p50_ms,p99_ms,max_ms,sim_ms,repl_ms");
    println!(
        "[bench] RESULT {},{},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3}",
        areas,
        NPCS_PER_AREA * areas,
        PLAYERS_PER_AREA * areas,
        PLAYERS_PER_AREA * areas,
        r.full,
        r.p50,
        r.p99,
        r.max,
        r.sim,
        (r.full - r.sim).max(0.0),
    );
    println!(
        "[bench] capacity: {} areas = {} NPCs + {} players sustained at {:.1}ms/tick (budget {:.0}ms)",
        areas,
        NPCS_PER_AREA * areas,
        PLAYERS_PER_AREA * areas,
        r.full,
        BUDGET_MS,
    );
}

fn verdict(mean: f64) -> &'static str {
    if mean <= BUDGET_MS { "ok" } else { "over" }
}

struct Point {
    full: f64,
    sim: f64,
    p50: f64,
    p99: f64,
    max: f64,
}

// Build a fresh server app populated with `areas` areas (the first `areas` of the configured pool) of 25
// NPCs + 25 players each, measure the sim alone, then attach a client per player and measure the full
// sim+replication tick.
fn point(areas: usize, warmup: usize, ticks: usize) -> Point {
    let mut app = world::sim::server_app();
    app.finish();
    app.cleanup();
    app.world_mut()
        .resource_mut::<NextState<ServerState>>()
        .set(ServerState::Running);
    app.update(); // run Startup (content NPC spawn) + apply the Running transition

    let npc = Id::<NpcDef>::by_name(&npc::defs()[0].id).expect("first npc def exists");
    let pool = area::areas();

    let assignments: Vec<(ClientId, Entity)> = {
        let world = app.world_mut();
        let content: Vec<Entity> = world
            .query_filtered::<Entity, With<Npc>>()
            .iter(world)
            .collect();
        for entity in content {
            world.despawn(entity);
        }

        let mut assignments = Vec::new();
        let mut next_client = 0u32;
        for area in pool.iter().take(areas) {
            for _ in 0..NPCS_PER_AREA {
                let entity = spawn_character(world, area, npc, wander_pos(area));
                world.entity_mut(entity).insert(Npc {
                    def: npc,
                    group: area.id.index() as u32,
                });
            }
            for _ in 0..PLAYERS_PER_AREA {
                next_client += 1;
                let client = ClientId(next_client);
                let player = spawn_character(world, area, npc, area.spawn);
                world.entity_mut(player).insert((
                    Owner { client },
                    Inventory { items: Vec::new() },
                    Xp { amount: 0 },
                ));
                world.resource_mut::<Players>().0.insert(client, player);
                assignments.push((client, player));
            }
        }
        assignments
    };

    let sim = measure(&mut app, warmup, ticks);

    {
        let world = app.world_mut();
        for &(client, player) in &assignments {
            let client_entity = world
                .spawn((ConnectedClient { max_size: 1200 }, client))
                .id();
            world.entity_mut(player).insert(OwnedBy(client_entity));
        }
    }

    let full = measure(&mut app, warmup, ticks);
    Point {
        full: full.0,
        sim: sim.0,
        p50: full.1,
        p99: full.2,
        max: full.3,
    }
}

fn measure(app: &mut bevy_app::App, warmup: usize, ticks: usize) -> (f64, f64, f64, f64) {
    for _ in 0..warmup {
        app.update();
    }
    let mut samples = Vec::with_capacity(ticks);
    for _ in 0..ticks {
        let started = Instant::now();
        app.update();
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = samples.len();
    (
        samples.iter().sum::<f64>() / n as f64,
        samples[n / 2],
        samples[(n as f64 * 0.99) as usize],
        samples[n - 1],
    )
}

fn wander_pos(area: &Area) -> Pos<Tiles> {
    area.walkable_nodes.first().copied().unwrap_or(area.spawn)
}

fn spawn_character(world: &mut World, area: &Area, def_id: Id<NpcDef>, at: Pos<Tiles>) -> Entity {
    let def = def_id.get();
    world
        .spawn(Character {
            replicated: Replicated,
            position: Position { pos: at },
            name: Name {
                name: def.display_name.clone(),
            },
            actor: Actor {
                color: def.tint,
                dir: Direction::S as u8,
                action: ACTION_IDLE,
                model: def.model,
                attack_rate: def.attack_speed,
            },
            hitbox: Hitbox {
                size: def.model.get().hitbox(),
            },
            vitals: Vitals {
                health: def.health,
                max: def.health,
            },
            area: AreaTag { area: area.id },
            stats: Stats {
                damage: def.damage,
                attack_speed: def.attack_speed,
                attack_delay: def.attack_delay,
                range: def.range,
            },
            speed: Speed { value: def.speed },
        })
        .id()
}
