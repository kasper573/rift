use std::time::{Duration, Instant};

use bevy::asset::{AssetPlugin, AssetServer, LoadState};
use bevy::image::{Image, ImageLoaderSettings};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use engine::core::assets::loader::PngLoader;
use engine::core::pace::Pace;
use testkit::png;

#[test]
fn a_png_loads_as_an_rgba_image_of_its_size_in_srgb_unless_asked_for_linear_light() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    std::fs::write(folder.path().join("face.png"), png(3, 2)).expect("a face");
    std::fs::write(folder.path().join("mask.png"), png(3, 2)).expect("a mask");
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin {
            file_path: folder.path().display().to_string(),
            ..Default::default()
        },
    ))
    .init_asset::<Image>()
    .register_asset_loader(PngLoader {
        pace: Pace::whole(),
    });
    let assets = app.world().resource::<AssetServer>();
    let face: Handle<Image> = assets.load("face.png");
    let mask: Handle<Image> = assets
        .load_builder()
        .with_settings(|settings: &mut ImageLoaderSettings| settings.is_srgb = false)
        .load("mask.png");
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.world().resource::<Assets<Image>>().len() < 2 && Instant::now() < deadline {
        let assets = app.world().resource::<AssetServer>();
        for handle in [&face, &mask] {
            assert!(
                !matches!(assets.load_state(handle), LoadState::Failed(_)),
                "the image loads: {:?}",
                assets.load_state(handle)
            );
        }
        app.update();
        std::thread::sleep(Duration::from_millis(5));
    }
    let images = app.world().resource::<Assets<Image>>();
    for (handle, format) in [
        (&face, TextureFormat::Rgba8UnormSrgb),
        (&mask, TextureFormat::Rgba8Unorm),
    ] {
        let image = images.get(handle).expect("the image loaded in time");
        assert_eq!((image.width(), image.height()), (3, 2));
        assert_eq!(image.texture_descriptor.format, format);
        assert_eq!(image.data.as_ref().map(Vec::len), Some(3 * 2 * 4));
    }
}
