use std::sync::Arc;
use ui::egui;

use editor::project::ProjectFiles;
use editor::thumbs::Thumbnails;

#[test]
fn an_undecodable_asset_is_decoded_once_until_its_bytes_change() {
    let ctx = egui::Context::default();
    let mut files = ProjectFiles::default();
    files.insert("broken.png".to_owned(), Arc::from(&b"not an image"[..]));
    let mut thumbs = Thumbnails::default();

    let mut budget = 1;
    assert!(
        thumbs
            .image_within(&ctx, &files, "broken.png", &mut budget)
            .is_none()
    );
    assert_eq!(budget, 0, "the first attempt spends the decode budget");

    let mut budget = 1;
    assert!(
        thumbs
            .image_within(&ctx, &files, "broken.png", &mut budget)
            .is_none()
    );
    assert_eq!(budget, 1, "a known failure costs nothing on later frames");

    files.insert("broken.png".to_owned(), png(2, 2));
    let mut budget = 1;
    assert!(
        thumbs
            .image_within(&ctx, &files, "broken.png", &mut budget)
            .is_some()
    );
    assert_eq!(budget, 0, "changed bytes are decoded again");
}

#[test]
fn an_icon_fits_its_longest_side_and_keeps_the_aspect_ratio() {
    let ctx = egui::Context::default();
    let mut files = ProjectFiles::default();
    files.insert("wide.png".to_owned(), png(64, 16));
    let mut thumbs = Thumbnails::default();
    let mut budget = u32::MAX;
    let icon = thumbs
        .icon(&ctx, &files, "wide.png", &mut budget)
        .expect("decodes");
    assert_eq!(icon.size, [editor::thumbs::ICON as usize, 8]);
}

fn png(width: u32, height: u32) -> Arc<[u8]> {
    let image = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageFormat::Png)
        .expect("encodes");
    Arc::from(bytes.into_inner())
}
