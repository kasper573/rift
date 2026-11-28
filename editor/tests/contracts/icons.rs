use std::collections::HashSet;
use ui::egui;

use editor::icons;
use editor::project::Project;
use editor::thumbs::Thumbnails;
use engine::systems::registry;
use serde_json::json;
use testkit::Game;

use crate::support;

#[test]
fn every_type_of_content_has_an_icon_of_its_own() {
    let unknown = icons::of_table("no_such_type");
    let mut seen = HashSet::new();
    for table in registry().table_names() {
        let icon = icons::of_table(table);
        assert_ne!(icon, unknown, "{table} has no icon");
        assert!(seen.insert(format!("{icon:?}")), "{table} shares its icon");
    }
}

#[test]
fn a_small_image_or_a_content_file_naming_a_picture_stands_in_for_its_icon() {
    let ctx = egui::Context::default();
    let (_scratch, project) = open(
        support::game()
            .asset("cap.png", png(16, 16))
            .asset("map.png", png(128, 96))
            .item("cap", json!({ "kind": { "type": "resource" } }))
            .area("field", &testkit::Map::open(4, 4), json!([])),
    );
    let mut thumbs = Thumbnails::default();
    let mut budget = u32::MAX;
    let mut preview = |file: &str| icons::preview(&ctx, &mut thumbs, &project, file, &mut budget);

    assert_eq!(
        preview("cap.png").map(|cap| cap.size),
        Some(egui::vec2(16.0, 16.0))
    );
    assert!(preview("map.png").is_none(), "a big image keeps its icon");
    assert_eq!(
        preview(&testkit::path("item", "cap")).map(|cap| cap.size),
        Some(egui::vec2(32.0, 32.0)),
        "an item stands in by the picture it names"
    );
    let field = testkit::path("area", "field");
    assert!(
        preview(&field).is_none(),
        "a kind that keeps its icon never stands in by its files' pictures"
    );

    assert!(
        icons::picture(&ctx, &mut thumbs, &project, "map.png", &mut budget).is_some(),
        "a big picture still shows where pictures are large"
    );
    assert!(icons::picture(&ctx, &mut thumbs, &project, &field, &mut budget).is_none());
}

/// `game` written to a scratch folder and opened, loaded through.
fn open(game: Game) -> (tempfile::TempDir, Project) {
    let scratch = tempfile::tempdir().expect("scratch");
    let project = support::open(game, &scratch).project.expect("a project");
    (scratch, project)
}

fn png(width: u32, height: u32) -> Vec<u8> {
    encoded(&image::RgbaImage::from_pixel(
        width,
        height,
        image::Rgba([10, 20, 30, 255]),
    ))
}

fn encoded(image: &image::RgbaImage) -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageFormat::Png)
        .expect("encodes");
    bytes.into_inner()
}
