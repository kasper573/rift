//! The simulation layer stands on its own: given only an injected asset root — no process env, no
//! transport — its content validates and the authoritative app assembles, exactly as the server
//! boots it. Proves the host layer is exercisable in isolation, not only through the server binary.

#[test]
fn assembles_the_authoritative_app_from_an_injected_root() {
    world::assets::init(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"));
    world::sim::validate();
    let _app = world::sim::server_app();
}
