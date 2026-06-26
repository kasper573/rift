//! Content is baked into the binary, so `world` loads and validates it with nothing set in the
//! process environment and no path supplied.

#[test]
fn loads_embedded_content() {
    assert!(!world::systems::actor::models().is_empty());
    assert!(!world::systems::area::areas().is_empty());
    assert!(!world::systems::item::items().is_empty());
    assert!(!world::systems::sfx::sfx_table().is_empty());
}
