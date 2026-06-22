//! Content is baked into the binary, so `world` loads and validates it with nothing set in the
//! process environment and no path supplied.

#[test]
fn loads_embedded_content() {
    assert!(!world::actors::models().is_empty());
    assert!(!world::area::areas().is_empty());
    assert!(!world::items::items().is_empty());
    assert!(!world::sfx::sfx_table().is_empty());
}
