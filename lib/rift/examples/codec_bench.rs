use std::hint::black_box;
use std::time::Instant;

use rift::{ClientId, Entity, Server, Wire};

#[derive(Wire, Clone, Debug, PartialEq)]
struct V2 {
    dx: f32,
    dy: f32,
}

#[derive(Wire, Clone, Debug, PartialEq)]
struct V3 {
    x: f32,
    y: f32,
    z: f32,
}

#[derive(Wire, Clone, Debug, PartialEq)]
struct Actor {
    pos: V3,
    vel: V2,
    name: String,
    alive: bool,
    hp: i32,
}

#[derive(Wire, Clone, Debug, PartialEq)]
struct Position {
    x: f32,
    y: f32,
}

#[derive(Wire, Clone, Debug, PartialEq)]
struct Velocity {
    x: f32,
    y: f32,
}

fn bench(label: &str, iters: u64, mut f: impl FnMut()) {
    for _ in 0..(iters / 10).max(1) {
        f();
    }
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let per = start.elapsed().as_secs_f64() / iters as f64;
    println!(
        "  {label:<32} {:>10.2} ns/op  {:>12.0} ops/s",
        per * 1e9,
        1.0 / per
    );
}

fn seeded_server(clients: u32) -> Server {
    let mut s = Server::new();
    for i in 1..=clients {
        s.connect(ClientId(i));
    }
    s
}

fn populate(s: &mut Server, n: usize) -> Vec<Entity> {
    (0..n)
        .map(|i| {
            let e = s.world.spawn();
            s.world.insert(
                e,
                Position {
                    x: i as f32,
                    y: 0.0,
                },
            );
            s.world.insert(e, Velocity { x: 1.0, y: 0.5 });
            e
        })
        .collect()
}

fn main() {
    let pos = V3 {
        x: 1.5,
        y: 2.5,
        z: 3.5,
    };
    let actor = Actor {
        pos: pos.clone(),
        vel: V2 { dx: 0.1, dy: 0.2 },
        name: "player_001".to_owned(),
        alive: true,
        hp: 100,
    };
    let arr: Vec<V3> = (0..100)
        .map(|i| V3 {
            x: i as f32,
            y: i as f32 * 2.0,
            z: i as f32 * 3.0,
        })
        .collect();
    let pos_bytes = {
        let mut o = Vec::new();
        pos.encode(&mut o);
        o
    };
    let actor_bytes = {
        let mut o = Vec::new();
        actor.encode(&mut o);
        o
    };
    let arr_bytes = {
        let mut o = Vec::new();
        arr.encode(&mut o);
        o
    };

    println!("codec encode:");
    bench("v3", 2_000_000, || {
        let mut o = Vec::with_capacity(16);
        pos.encode(&mut o);
        black_box(&o);
    });
    bench("actor", 1_000_000, || {
        let mut o = Vec::with_capacity(64);
        actor.encode(&mut o);
        black_box(&o);
    });
    bench("array[100]", 100_000, || {
        let mut o = Vec::with_capacity(2048);
        arr.encode(&mut o);
        black_box(&o);
    });

    println!("codec decode:");
    bench("v3", 2_000_000, || {
        black_box(V3::decode(&mut pos_bytes.as_slice()).unwrap());
    });
    bench("actor", 1_000_000, || {
        black_box(Actor::decode(&mut actor_bytes.as_slice()).unwrap());
    });
    bench("array[100]", 100_000, || {
        black_box(Vec::<V3>::decode(&mut arr_bytes.as_slice()).unwrap());
    });

    println!("world + replication:");
    {
        let mut s = seeded_server(0);
        let ids = populate(&mut s, 1000);
        let _ = s.flush(0.016);
        bench("world modify (1k)", 2000, || {
            for &e in &ids {
                s.world.modify::<Position>(e, |p| p.x += 1.0);
            }
        });
        bench("world iterate (1k)", 5000, || {
            let mut sum = 0.0f32;
            for (_, p) in s.world.iter::<Position>() {
                sum += p.x;
            }
            black_box(sum);
        });
    }
    for clients in [1u32, 16, 64] {
        let mut s = seeded_server(clients);
        let ids = populate(&mut s, 1000);
        let _ = s.flush(0.016);
        bench(&format!("tick {clients}c x 1000e"), 200, || {
            for &e in &ids {
                s.world.modify::<Position>(e, |p| {
                    p.x += 1.0;
                    p.y += 0.5;
                });
            }
            black_box(s.flush(0.016).len());
        });
    }
    bench("tick build+fill 1000e, 0 clients", 1000, || {
        let mut s = Server::new();
        for i in 0..1000 {
            let e = s.world.spawn();
            s.world.insert(
                e,
                Position {
                    x: i as f32,
                    y: 0.0,
                },
            );
        }
        let _ = s.flush(1.0 / 60.0);
        black_box(s.world.entity_count());
    });
}
