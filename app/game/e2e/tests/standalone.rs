use e2e::sim::SimStage;
use e2e::{Player as _, Stage as _, eventually, scenarios};
use world::MoveRequest;
use world::core::math::{Pos, Tiles};

macro_rules! bind {
    (players_only: $($name:ident $(($($arg:expr),*))?),* $(,)?) => {$(
        #[test]
        fn $name() {
            scenarios::$name(&mut SimStage::players_only() $($(, $arg)*)?);
        }
    )*};
    (full: $($name:ident $(($($arg:expr),*))?),* $(,)?) => {$(
        #[test]
        fn $name() {
            scenarios::$name(&mut SimStage::full() $($(, $arg)*)?);
        }
    )*};
}

e2e::for_each_scenario!(bind);

// Protocol-level contracts: requests no real client UI can send.

#[test]
fn a_playing_client_cannot_become_a_spectator() {
    let mut stage = SimStage::players_only();
    let mut a = stage.connect(&[]);
    a.join();
    assert!(
        eventually(&mut stage, 5.0, || a.view().me().is_some()),
        "player spawns"
    );
    a.spectate(None);
    stage.step(1.0);
    let view = a.view();
    assert!(
        view.me().is_some_and(|me| me.actor),
        "a playing client stays a rendered player"
    );
    assert!(
        !view.spectating,
        "a playing client never becomes a spectator"
    );
}

#[test]
fn a_spectator_cannot_join_move_or_attack() {
    use world::{AttackRequest, Entity, SPECTATE_ROLE};

    let mut stage = SimStage::players_only();
    let mut a = stage.connect(&[]);
    a.join();
    let mut s = stage.connect(&[SPECTATE_ROLE]);
    s.spectate(None);
    assert!(
        eventually(&mut stage, 5.0, || s.view().spectating),
        "spectator admitted"
    );

    s.join();
    stage.step(1.0);
    let view = s.view();
    assert!(
        view.spectating && view.me().is_some_and(|me| !me.actor),
        "joining while spectating must be ignored",
    );

    let before = s.view().me().map(|me| me.pos);
    s.send(&MoveRequest {
        pos: Pos::new(Tiles(18.5), Tiles(24.5)),
    });
    stage.step(2.0);
    assert_eq!(
        s.view().me().map(|me| me.pos),
        before,
        "spectators cannot move their anchor",
    );

    let a_id = a.client_id();
    let target = s
        .view()
        .player_of(a_id)
        .expect("spectator sees the player")
        .entity;
    let full = a
        .view()
        .me()
        .and_then(|me| me.health)
        .expect("player healthy");
    s.send(&AttackRequest {
        target: Entity(target),
    });
    stage.step(5.0);
    assert_eq!(
        a.view().me().and_then(|me| me.health),
        Some(full),
        "spectators cannot attack",
    );
}

#[test]
fn connecting_without_joining_spawns_nothing() {
    let mut stage = SimStage::players_only();
    let mut raw = stage.connect(&[]);
    stage.step(0.5);
    assert!(
        raw.view().me().is_none(),
        "no entity may exist for a client that never joined"
    );
}

#[test]
fn an_open_server_lets_anyone_spectate() {
    use rift::{Client, ClientId, Cluster};
    use world::SpectateRequest;

    let mut cluster = Cluster::new(&world::features(), &world::zones(), world::spawn_zone());
    let id = ClientId(1);
    cluster.connect(id);
    let mut client = Client::new();
    let packet = client.send(&SpectateRequest { watch: None });
    cluster.receive(id, &packet);
    let dt = 1.0 / world::TICK_HZ;
    let mut admitted = false;
    for _ in 0..10 {
        for (to, bytes) in cluster.tick(dt) {
            if to == id {
                client.receive(&bytes);
            }
        }
        admitted = client.world.iter::<world::Spectate>().next().is_some();
        if admitted {
            break;
        }
    }
    assert!(admitted, "spectating works without auth");
}

#[test]
fn stepping_onto_a_portal_without_intent_does_not_teleport() {
    let mut stage = SimStage::players_only();
    let mut a = stage.connect(&[]);
    a.join();
    assert!(
        eventually(&mut stage, 5.0, || a.view().me().is_some()),
        "player spawns"
    );
    let portal = scenarios::home_portal();
    a.send(&MoveRequest { pos: portal });
    stage.step(30.0);
    assert_eq!(
        a.view().me().and_then(|me| me.area),
        Some(world::core::area::spawn_zone()),
        "walking over a portal must not cross without explicit intent",
    );
}
