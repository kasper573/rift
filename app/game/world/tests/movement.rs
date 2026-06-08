use rift::{App, ClientId, Server};
use world::core::area;
use world::core::protocol::is_dead;
use world::features;
use world::features::movement::{MoveTarget, Path};
use world::features::npc::Npc;
use world::{AttackRequest, JoinRequest, MoveRequest, Owner, Position};

// The movement invariant: an actor only ever comes to rest on an exact tile. Stopping a chase
// mid-step (the historic bug), clearing a path, or issuing a new move order must each finish the
// current step first. Driven through real gameplay — npcs wander, aggro, chase, and attack, and the
// player chases and attacks back — every actor that is not moving must sit on a tile center, every
// tick.
#[test]
fn a_resting_actor_is_always_on_a_tile() {
    let dt = 1.0 / 30.0;
    let mut app = App::new(&features());
    let mut server = Server::new();
    app.start(&mut server);

    let client = ClientId(0);
    server.connect(client);
    server.inject(client, &JoinRequest {});
    app.tick(&mut server, dt);

    // Make the player chase and attack an npc, so the player-side stop is exercised too, not only
    // npcs aggroing the idle player.
    if let Some(target) = server.world.ids::<Npc>().into_iter().next() {
        server.inject(client, &AttackRequest { target });
    }

    for _ in 0..600 {
        app.tick(&mut server, dt);
        for (id, position) in server.world.iter::<Position>() {
            if is_dead(&server.world, id)
                || server.world.has::<Path>(id)
                || server.world.has::<MoveTarget>(id)
            {
                continue;
            }
            assert_eq!(
                position.pos.x.0.fract(),
                0.5,
                "a resting actor stranded in x at {}",
                position.pos.x.0
            );
            assert_eq!(
                position.pos.y.0.fract(),
                0.5,
                "a resting actor stranded in y at {}",
                position.pos.y.0
            );
        }
    }
}

// A second move order while already moving must redirect to the new goal, not stop at the end of the
// current step. (Regression: routing a redirect through `halt`, which keeps the in-flight tile, let
// `advance` drop the new goal once that tile was reached, so a second click stopped the actor.)
#[test]
fn a_second_move_order_redirects_rather_than_stopping() {
    let dt = 1.0 / 30.0;
    let mut app = App::new(&features());
    let mut server = Server::new();
    app.start(&mut server);

    let client = ClientId(0);
    server.connect(client);
    server.inject(client, &JoinRequest {});
    app.tick(&mut server, dt);

    let player = server
        .world
        .iter::<Owner>()
        .find(|(_, owner)| owner.client == client)
        .map(|(entity, _)| entity)
        .expect("the join spawns a player");
    let start = server
        .world
        .get::<Position>(player)
        .expect("player position");
    let from_spawn = |p: &Position| (p.pos.x.0 - start.pos.x.0).hypot(p.pos.y.0 - start.pos.y.0);

    // Set off for the far side of the spawn area.
    let spawn_area = &area::areas()[world::spawn_zone() as usize];
    let &node = spawn_area
        .walkable_nodes
        .iter()
        .max_by(|&&a, &&b| {
            let here = (a.x.0 + 0.5 - start.pos.x.0).hypot(a.y.0 + 0.5 - start.pos.y.0);
            let there = (b.x.0 + 0.5 - start.pos.x.0).hypot(b.y.0 + 0.5 - start.pos.y.0);
            here.total_cmp(&there)
        })
        .expect("the spawn area has walkable nodes");
    server.inject(
        client,
        &MoveRequest {
            pos: node.map(|t| t + 0.5),
        },
    );
    for _ in 0..3 {
        app.tick(&mut server, dt);
    }
    let mid = server
        .world
        .get::<Position>(player)
        .expect("player position");
    assert!(from_spawn(&mid) > 0.0, "the player should have set off");

    // Redirect back to spawn: the player must turn around, not stop where its step ended.
    server.inject(client, &MoveRequest { pos: start.pos });
    for _ in 0..10 {
        app.tick(&mut server, dt);
    }
    let after = server
        .world
        .get::<Position>(player)
        .expect("player position");
    assert!(
        from_spawn(&after) < from_spawn(&mid),
        "a second order must redirect the player back toward spawn",
    );
}
