//! Content is baked into the binary, so `world` loads and validates it with nothing set in the
//! process environment and no path supplied.

#[test]
fn loads_embedded_content() {
    assert!(!world::content::actors::models().is_empty());
    assert!(!world::content::area::areas().is_empty());
    assert!(!world::content::items::items().is_empty());
    assert!(!world::content::sfx::sfx_table().is_empty());
}
