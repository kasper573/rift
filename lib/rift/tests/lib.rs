use std::time::{Duration, Instant};

use rift::{
    App, Builder, Client, ClientId, Entity, Link, LinkStatus, Server, TcpCluster, Wire, World,
};

#[derive(Wire, Clone, Debug, PartialEq)]
struct Pos {
    x: f32,
    y: f32,
}

#[derive(Wire, Clone, Debug, PartialEq)]
struct Name {
    v: String,
}

#[derive(Wire, Clone, Debug, PartialEq)]
struct Ping {
    note: String,
}

#[derive(Wire, Clone, Debug, PartialEq)]
struct Gold {
    amount: u32,
}

#[derive(Wire, Clone, Debug, PartialEq)]
struct OwnedBy {
    client: u32,
}

fn deliver(s: &mut Server, c: &mut Client) {
    for (_, pkt) in s.flush(0.0) {
        c.receive(&pkt);
    }
}

fn deliver_each(s: &mut Server, clients: &mut [(ClientId, &mut Client)]) {
    for (id, pkt) in s.flush(0.0) {
        if let Some((_, client)) = clients.iter_mut().find(|(cid, _)| *cid == id) {
            client.receive(&pkt);
        }
    }
}

fn owner_of(world: &World, entity: Entity) -> Option<ClientId> {
    world.get::<OwnedBy>(entity).map(|o| ClientId(o.client))
}

fn server() -> Server {
    let mut s = Server::new();
    s.replicate::<Pos>();
    s.replicate::<Name>();
    s
}

#[test]
fn snapshot_mirrors_world() {
    let mut s = server();
    let mut c = Client::new();
    s.connect(ClientId(1));
    let e = s.world.spawn();
    s.world.insert(e, Pos { x: 4.0, y: 5.0 });
    s.world.insert(
        e,
        Name {
            v: "alpha".to_owned(),
        },
    );
    deliver(&mut s, &mut c);
    assert_eq!(c.id, Some(ClientId(1)));
    assert_eq!(c.world.get::<Pos>(e), Some(Pos { x: 4.0, y: 5.0 }));
    assert_eq!(
        c.world.get::<Name>(e),
        Some(Name {
            v: "alpha".to_owned()
        })
    );
}

#[test]
fn update_and_despawn_propagate() {
    let mut s = server();
    let mut c = Client::new();
    s.connect(ClientId(1));
    let e = s.world.spawn();
    s.world.insert(e, Pos { x: 1.0, y: 2.0 });
    deliver(&mut s, &mut c);
    s.world.modify::<Pos>(e, |p| p.x = 99.0);
    deliver(&mut s, &mut c);
    assert_eq!(c.world.get::<Pos>(e).map(|p| p.x), Some(99.0));
    s.world.despawn(e);
    deliver(&mut s, &mut c);
    assert!(!c.world.alive(e));
}

#[test]
fn component_removal_propagates() {
    let mut s = server();
    let mut c = Client::new();
    s.connect(ClientId(1));
    let e = s.world.spawn();
    s.world.insert(e, Pos { x: 1.0, y: 2.0 });
    s.world.insert(
        e,
        Name {
            v: "alpha".to_owned(),
        },
    );
    deliver(&mut s, &mut c);
    assert!(c.world.get::<Name>(e).is_some());
    s.world.remove::<Name>(e);
    deliver(&mut s, &mut c);
    assert_eq!(c.world.get::<Name>(e), None);
    assert_eq!(c.world.get::<Pos>(e), Some(Pos { x: 1.0, y: 2.0 }));
    assert!(c.world.alive(e));
}

#[test]
fn events_both_ways() {
    let mut s = server();
    let mut c = Client::new();
    s.connect(ClientId(1));
    let up = c.send(&Ping {
        note: "hi".to_owned(),
    });
    s.receive(ClientId(1), &up);
    assert_eq!(
        s.drain_events::<Ping>(),
        vec![(
            ClientId(1),
            Ping {
                note: "hi".to_owned()
            }
        )]
    );
    s.broadcast(&Ping {
        note: "yo".to_owned(),
    });
    deliver(&mut s, &mut c);
    assert_eq!(
        c.drain_events::<Ping>(),
        vec![Ping {
            note: "yo".to_owned()
        }]
    );
}

#[test]
fn visibility_filters_entities() {
    let mut s = server();
    let mut c = Client::new();
    s.connect(ClientId(1));
    let vis = s.world.spawn();
    s.world.insert(vis, Pos { x: 1.0, y: 1.0 });
    let hid = s.world.spawn();
    s.world.insert(hid, Pos { x: 2.0, y: 2.0 });
    s.set_visibility(ClientId(1), Some([vis].into_iter().collect()));
    deliver(&mut s, &mut c);
    assert!(c.world.alive(vis));
    assert!(!c.world.alive(hid));
}

#[test]
fn owner_only_components_replicate_only_to_the_owner() {
    let mut s = server();
    s.replicate_to_owner::<Gold>();
    s.owned_by(owner_of);
    let mut a = Client::new();
    let mut b = Client::new();
    s.connect(ClientId(1));
    s.connect(ClientId(2));
    let e = s.world.spawn();
    s.world.insert(e, Pos { x: 1.0, y: 2.0 });
    s.world.insert(e, OwnedBy { client: 1 });
    s.world.insert(e, Gold { amount: 100 });
    deliver_each(&mut s, &mut [(ClientId(1), &mut a), (ClientId(2), &mut b)]);
    assert_eq!(a.world.get::<Gold>(e), Some(Gold { amount: 100 }));
    assert_eq!(a.world.get::<Pos>(e), Some(Pos { x: 1.0, y: 2.0 }));
    assert_eq!(b.world.get::<Gold>(e), None);
    assert_eq!(b.world.get::<Pos>(e), Some(Pos { x: 1.0, y: 2.0 }));

    s.world.modify::<Gold>(e, |g| g.amount = 250);
    deliver_each(&mut s, &mut [(ClientId(1), &mut a), (ClientId(2), &mut b)]);
    assert_eq!(a.world.get::<Gold>(e), Some(Gold { amount: 250 }));
    assert_eq!(b.world.get::<Gold>(e), None);

    s.world.despawn(e);
    deliver_each(&mut s, &mut [(ClientId(1), &mut a), (ClientId(2), &mut b)]);
    assert!(!a.world.alive(e));
    assert!(!b.world.alive(e));
}

#[test]
fn unowned_owner_only_components_reach_no_one() {
    let mut s = server();
    s.replicate_to_owner::<Gold>();
    s.owned_by(owner_of);
    let mut c = Client::new();
    s.connect(ClientId(1));
    let e = s.world.spawn();
    s.world.insert(e, Pos { x: 0.0, y: 0.0 });
    s.world.insert(e, Gold { amount: 5 });
    deliver(&mut s, &mut c);
    assert_eq!(c.world.get::<Gold>(e), None);
    assert_eq!(c.world.get::<Pos>(e), Some(Pos { x: 0.0, y: 0.0 }));
}

#[test]
fn builder_wires_owner_only_replication() {
    fn feature(b: &mut Builder) {
        b.replicate::<Pos>();
        b.replicate_to_owner::<Gold>();
        b.owned_by(owner_of);
    }
    let mut app = App::new(&[feature]);
    let mut s = Server::new();
    app.start(&mut s);
    let mut a = Client::new();
    let mut b = Client::new();
    s.connect(ClientId(1));
    s.connect(ClientId(2));
    let e = s.world.spawn();
    s.world.insert(e, Pos { x: 3.0, y: 4.0 });
    s.world.insert(e, OwnedBy { client: 1 });
    s.world.insert(e, Gold { amount: 7 });
    app.tick(&mut s, 0.1);
    deliver_each(&mut s, &mut [(ClientId(1), &mut a), (ClientId(2), &mut b)]);
    assert_eq!(a.world.get::<Gold>(e), Some(Gold { amount: 7 }));
    assert_eq!(b.world.get::<Gold>(e), None);
    assert_eq!(b.world.get::<Pos>(e), Some(Pos { x: 3.0, y: 4.0 }));
}

#[test]
fn codec_roundtrips() {
    fn rt<T: Wire + PartialEq + std::fmt::Debug>(v: T) {
        let mut o = Vec::new();
        v.encode(&mut o);
        assert_eq!(T::decode(&mut o.as_slice()), Some(v));
    }
    rt(42u32);
    rt(-7i32);
    rt(3.5f32);
    rt(true);
    rt(false);
    rt("héllo r:ft 🌌".to_owned());
    rt(vec![1u16, 2, 3]);
    rt(Some(9u8));
    rt(Option::<u8>::None);
}

/// The game-defined session type rift carries opaquely; tests use a plain name.
#[derive(Debug, PartialEq, Eq)]
struct User(String);

fn seeded_host(authenticated: bool) -> (TcpCluster, rift::Entity) {
    let mut host = TcpCluster::bind("127.0.0.1:0", &[], &[0], 0).expect("bind");
    if authenticated {
        host.authenticate_with(Box::new(|token| match token.strip_prefix("good:") {
            Some(name) => Ok(std::sync::Arc::new(User(name.to_owned())) as rift::Session),
            None => Err("bad token".to_owned()),
        }));
    }
    let entity = {
        let server = host.cluster.server_mut(0).expect("zone 0 shard");
        server.replicate::<Pos>();
        let entity = server.world.spawn();
        server.world.insert(entity, Pos { x: 3.0, y: 7.0 });
        entity
    };
    (host, entity)
}

fn purift_until(host: &mut TcpCluster, link: &mut Link, ready: impl Fn(&Link) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        host.poll();
        host.tick(0.0);
        link.poll();
        if ready(link) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

#[test]
fn full_stack_over_tcp() {
    let (mut host, entity) = seeded_host(false);
    let addr = host.local_addr().to_string();
    let mut link = Link::tcp(&addr, "").expect("connect");
    let replicated = purift_until(&mut host, &mut link, |link| {
        link.client.world.get::<Pos>(entity) == Some(Pos { x: 3.0, y: 7.0 })
    });
    assert!(replicated, "entity never replicated over tcp");
}

#[test]
fn tcp_auth_accepts_valid_token_and_exposes_session() {
    let (mut host, entity) = seeded_host(true);
    let addr = host.local_addr().to_string();
    let mut link = Link::tcp(&addr, "good:kasper").expect("connect");
    let replicated = purift_until(&mut host, &mut link, |link| {
        link.client.world.get::<Pos>(entity).is_some()
    });
    assert!(replicated, "authenticated client never got a snapshot");
    let client_id = link.client.id.expect("snapshot carries the client id");
    assert_eq!(
        host.session::<User>(client_id),
        Some(&User("kasper".to_owned()))
    );
    assert_eq!(
        host.session::<String>(client_id),
        None,
        "wrong downcast type yields None"
    );
}

#[test]
fn tcp_auth_rejects_invalid_token() {
    let (mut host, entity) = seeded_host(true);
    let addr = host.local_addr().to_string();
    let mut link = Link::tcp(&addr, "wrong").expect("connect");
    let replicated = purift_until(&mut host, &mut link, |link| {
        link.client.world.get::<Pos>(entity).is_some() || link.status() == LinkStatus::Closed
    });
    assert!(replicated, "rejected client neither replicated nor closed");
    assert_eq!(link.status(), LinkStatus::Closed);
    assert!(link.client.world.get::<Pos>(entity).is_none());
}

#[test]
fn full_stack_over_websocket() {
    let (mut host, entity) = seeded_host(true);
    let port = host.local_addr().port();

    let client = std::thread::spawn(move || {
        let stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("tcp connect");
        let (mut socket, _) = tungstenite::client(
            format!("ws://127.0.0.1:{port}/ws?accessToken=good%3Abrowser"),
            stream,
        )
        .expect("websocket handshake");
        // The first binary message is a snapshot; receiving one proves the full path works.
        loop {
            match socket.read().expect("read message") {
                tungstenite::Message::Binary(data) => return data,
                _ => continue,
            }
        }
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    while !client.is_finished() && Instant::now() < deadline {
        host.poll();
        host.tick(0.0);
        std::thread::sleep(Duration::from_millis(5));
    }
    let snapshot = client.join().expect("client thread");
    let mut mirror = Client::new();
    mirror.receive(&snapshot);
    assert_eq!(
        mirror.world.get::<Pos>(entity),
        Some(Pos { x: 3.0, y: 7.0 }),
        "websocket snapshot should mirror the world"
    );
}

#[test]
fn websocket_rejects_invalid_token() {
    let (mut host, _) = seeded_host(true);
    let port = host.local_addr().port();

    let client = std::thread::spawn(move || {
        let stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("tcp connect");
        tungstenite::client(
            format!("ws://127.0.0.1:{port}/ws?accessToken=wrong"),
            stream,
        )
        .map(|_| ())
        .map_err(|_| ())
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    while !client.is_finished() && Instant::now() < deadline {
        host.poll();
        host.tick(0.0);
        std::thread::sleep(Duration::from_millis(5));
    }
    let result = client.join().expect("client thread");
    assert!(result.is_err(), "handshake must fail with a bad token");
}

#[test]
fn health_and_metrics_endpoints() {
    use std::io::{Read, Write};

    let (mut host, _) = seeded_host(false);
    let port = host.local_addr().port();

    let mut http_get = |path: &'static str| {
        let client = std::thread::spawn({
            move || {
                let mut stream =
                    std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
                stream
                    .write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
                    .expect("write request");
                let mut response = String::new();
                stream.read_to_string(&mut response).expect("read response");
                response
            }
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while !client.is_finished() && Instant::now() < deadline {
            host.poll();
            host.tick(0.0);
            std::thread::sleep(Duration::from_millis(5));
        }
        client.join().expect("client thread")
    };

    let health = http_get("/health");
    assert!(health.starts_with("HTTP/1.1 200"), "health: {health}");
    assert!(health.ends_with("ok"), "health: {health}");

    let metrics = http_get("/metrics");
    assert!(metrics.starts_with("HTTP/1.1 200"), "metrics: {metrics}");
    assert!(metrics.contains("rift_ticks_total"), "metrics: {metrics}");
    assert!(
        metrics.contains("rift_clients_connected"),
        "metrics: {metrics}"
    );

    let missing = http_get("/nope");
    assert!(missing.starts_with("HTTP/1.1 404"), "missing: {missing}");
}
