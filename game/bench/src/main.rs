use std::time::Instant;

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{ConnectedClient, Replicated, ServerState};
use bevy_state::prelude::NextState;
use world::content::area::{self, Area};
use world::core::math::{Direction, Pos};
use world::core::table::Id;
use world::core::tiling::Tiles;
use world::sim::Character;
use world::sim::combat::Stats;
use world::sim::movement::Speed;
use world::sim::npc::{self, Npc, NpcDef};
use world::sim::player::Players;
use world::sim::visibility::OwnedBy;
use world::{
    ACTION_IDLE, Actor, AreaTag, ClientId, Hitbox, Inventory, Name, Owner, Position, Vitals, Xp,
};

const NPCS_PER_AREA: usize = 25;
const PLAYERS_PER_AREA: usize = 25;
const BUDGET_MS: f64 = 40.0;
const MAX_AREAS: usize = 256; // must exceed the crossover and the probe that overshoots it
const WARMUP: usize = 30;
const MEASURE: usize = 200;

fn main() {
    area::configure_areas(MAX_AREAS);
    world::sim::validate();

    println!("[bench] finding the highest A sustained within the {BUDGET_MS:.0}ms budget...");

    let mut best: Option<(usize, Point)> = None;
    let mut under: Option<(usize, f64)> = None;
    let mut over: Option<(usize, f64)> = None;
    let mut previous: Option<(usize, f64)> = None;
    let mut next = Some(1usize);
    while let Some(areas) = next {
        let p = point(areas, WARMUP, MEASURE);
        let mean = p.full;
        println!(
            "[bench]   A={areas:<4} mean={mean:6.2}ms  {}",
            verdict(mean)
        );
        let last = (areas, mean);
        if mean <= BUDGET_MS {
            if under.is_none_or(|(highest, _)| areas >= highest) {
                under = Some(last);
                best = Some((areas, p));
            }
        } else if over.is_none_or(|(lowest, _)| areas <= lowest) {
            over = Some(last);
        }
        next = project_areas(under, over, previous, last);
        previous = Some(last);
    }

    let (areas, r) = best.unwrap_or_else(|| (1, point(1, WARMUP, MEASURE)));
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
        "[bench] capacity: {} isolated areas = {} NPCs + {} players sustained at {:.1}ms/tick (budget {:.0}ms)",
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

/// The next area count to probe while root-finding the budget crossover, or `None` to stop. Stops once
/// the projected next probe is only one area past the best sustained one — a finer answer isn't worth a
/// probe. With the budget bracketed it uses false position; before that it leaps straight at the budget
/// (secant of the last two probes, else one proportional guess), so a fast world is bracketed at once.
fn project_areas(
    under: Option<(usize, f64)>,
    over: Option<(usize, f64)>,
    previous: Option<(usize, f64)>,
    last: (usize, f64),
) -> Option<usize> {
    let (ua, ut) = under?;
    if ua >= MAX_AREAS {
        return None;
    }
    let next = match over {
        Some((oa, ot)) => {
            if oa <= ua + 1 {
                return None;
            }
            let guess = if ot > ut {
                ua as f64 + (BUDGET_MS - ut) * (oa - ua) as f64 / (ot - ut)
            } else {
                (ua + oa) as f64 / 2.0
            };
            (guess.round() as usize).clamp(ua + 1, oa - 1)
        }
        None => {
            let projected = match previous {
                Some((pa, pt)) if last.0 != pa && last.1 > pt => {
                    let slope = (last.1 - pt) / (last.0 - pa) as f64;
                    last.0 as f64 + (BUDGET_MS - last.1) / slope
                }
                _ => ua as f64 * BUDGET_MS / ut,
            };
            (projected.round() as usize).clamp(ua + 1, MAX_AREAS)
        }
    };
    (next > ua + 1).then_some(next)
}

struct Point {
    full: f64,
    sim: f64,
    p50: f64,
    p99: f64,
    max: f64,
}

fn point(areas: usize, warmup: usize, ticks: usize) -> Point {
    let npc = Id::<NpcDef>::by_name(&npc::defs()[0].id).expect("first npc def exists");
    let pool = area::areas();

    let mut worlds: Vec<App> = Vec::with_capacity(areas);
    let mut rosters: Vec<Vec<(ClientId, Entity)>> = Vec::with_capacity(areas);
    for area in pool.iter().take(areas) {
        let (app, roster) = build_world(area, npc);
        worlds.push(app);
        rosters.push(roster);
    }

    let sim = measure(&mut worlds, warmup, ticks);

    for (app, roster) in worlds.iter_mut().zip(&rosters) {
        let world = app.world_mut();
        for &(client, player) in roster {
            world.spawn((ConnectedClient { max_size: 1200 }, client));
            world.entity_mut(player).insert(OwnedBy(client));
        }
    }

    let full = measure(&mut worlds, warmup, ticks);
    Point {
        full: full.0,
        sim: sim.0,
        p50: full.1,
        p99: full.2,
        max: full.3,
    }
}

fn build_world(area: &Area, npc: Id<NpcDef>) -> (App, Vec<(ClientId, Entity)>) {
    let mut app = world::sim::server_app(area.id);
    app.finish();
    app.cleanup();
    app.world_mut()
        .resource_mut::<NextState<ServerState>>()
        .set(ServerState::Running);
    app.update();

    let world = app.world_mut();
    let content: Vec<Entity> = world
        .query_filtered::<Entity, With<Npc>>()
        .iter(world)
        .collect();
    for entity in content {
        world.despawn(entity);
    }

    let mut roster = Vec::with_capacity(PLAYERS_PER_AREA);
    for _ in 0..NPCS_PER_AREA {
        let entity = spawn_character(world, area, npc, wander_pos(area));
        world.entity_mut(entity).insert(Npc {
            def: npc,
            group: area.id.index() as u32,
        });
    }
    for index in 0..PLAYERS_PER_AREA {
        let client = ClientId(index as u32 + 1);
        let player = spawn_character(world, area, npc, area.spawn);
        world.entity_mut(player).insert((
            Owner { client },
            Inventory { items: Vec::new() },
            Xp { amount: 0 },
        ));
        world.resource_mut::<Players>().0.insert(client, player);
        roster.push((client, player));
    }
    (app, roster)
}

fn measure(worlds: &mut [App], warmup: usize, ticks: usize) -> (f64, f64, f64, f64) {
    for _ in 0..warmup {
        for app in worlds.iter_mut() {
            app.update();
        }
    }
    let mut samples = Vec::with_capacity(ticks);
    for _ in 0..ticks {
        let started = Instant::now();
        for app in worlds.iter_mut() {
            app.update();
        }
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
