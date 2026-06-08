use rift::{App, Client, ClientId, Server};
use world::features;
use world::features::combat::{AttackTarget, Stats};
use world::features::movement::Speed;
use world::features::npc::Npc;
use world::{Actor, JoinRequest, Position};

// The single-World design keeps server-only components in the same world as the replicated ones;
// only `replication::feature`'s declared types may reach a client. This guards against the leak.
#[test]
fn server_only_components_do_not_replicate() {
    let dt = 1.0 / 30.0;
    let mut app = App::new(&features());
    let mut server = Server::new();
    app.start(&mut server);

    let client_id = ClientId(0);
    server.connect(client_id);
    server.inject(client_id, &JoinRequest {});
    app.tick(&mut server, dt);

    let mut mirror = Client::new();
    for (id, packet) in server.flush(dt) {
        if id == client_id {
            mirror.receive(&packet);
        }
    }

    let entities: Vec<_> = mirror.world.all_entities().collect();
    assert!(!entities.is_empty(), "client should see at least itself");
    for entity in entities {
        assert!(
            mirror.world.has::<Actor>(entity) && mirror.world.has::<Position>(entity),
            "visible entities must carry the render components",
        );
        assert!(
            !mirror.world.has::<Npc>(entity),
            "Npc must stay server-side"
        );
        assert!(
            !mirror.world.has::<Stats>(entity),
            "Stats must stay server-side"
        );
        assert!(
            !mirror.world.has::<AttackTarget>(entity),
            "AttackTarget must stay server-side",
        );
        assert!(
            !mirror.world.has::<Speed>(entity),
            "Speed must stay server-side"
        );
    }
}
