//! The content loaders read the asset root injected via `assets::init`, not the environment — so
//! `world` can load and validate content given only a path, with nothing set in the process env.

#[test]
fn loads_content_from_an_injected_root() {
    // No RIFT_ASSETS_DIR is read anywhere below; the root is the injected fixture path.
    world::assets::init(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"));

    assert!(!world::actors::models().is_empty());
    assert!(!world::area::areas().is_empty());
    assert!(!world::items::items().is_empty());
    assert!(!world::sfx::sfx_table().is_empty());
}
