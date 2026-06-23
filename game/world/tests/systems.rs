//! The systems layer stands on its own: with content baked into the binary — no process env, no
//! transport — it validates and the authoritative app assembles, exactly as the server boots it.
//! Proves the systems layer is exercisable in isolation, not only through the server binary.

#[test]
fn assembles_the_authoritative_app() {
    world::systems::validate();
    let _app = world::systems::server_app(world::content::area::spawn_zone());
}
