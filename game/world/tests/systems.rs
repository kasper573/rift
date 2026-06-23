//! The systems layer stands on its own: with content baked into the binary — no process env, no
//! transport — it validates and the authoritative app assembles, exactly as the server boots it.
//! Proves the systems layer is exercisable in isolation, not only through the server binary.

#[test]
fn assembles_the_authoritative_app() {
    world::validate();
    let _app = world::server_app(world::area::spawn_zone());
}
