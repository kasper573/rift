use engine::core::content::gltf::externalize;
use engine::systems::area::prop::{PROP_GLB, PROP_TEXTURES};
use serde_json::json;
use testkit::{Game, glb, glb_naming_image, glb_with_image_inside, part_file, path, png, wav};

/// The rule and file of every error the game is refused for.
fn refusals(game: Game) -> Vec<(&'static str, String)> {
    let report = game.try_load().err().expect("refused");
    report
        .errors
        .iter()
        .map(|error| (error.problem.rule, error.file.clone()))
        .collect()
}

fn damaged(mut bytes: Vec<u8>, from_end: usize) -> Vec<u8> {
    let at = bytes.len() - from_end;
    for byte in &mut bytes[at..at + 16] {
        *byte ^= 0xff;
    }
    bytes
}

#[test]
fn an_image_damaged_past_its_header_or_cut_short_is_refused() {
    let mut cut = png(32, 32);
    cut.truncate(cut.len() - 20);
    let game = Game::new()
        .item("cap", json!({ "kind": { "type": "resource" } }))
        .item("orc", json!({ "kind": { "type": "resource" } }))
        .file("icons/orc.png", damaged(png(32, 32), 30))
        .file("icons/cap.png", cut);
    assert_eq!(
        refusals(game),
        [
            ("unreadable image", "icons/cap.png".to_owned()),
            ("unreadable image", "icons/orc.png".to_owned()),
        ]
    );
}

#[test]
fn a_model_that_does_not_read_as_a_gltf_binary_is_refused() {
    let mut cut = glb(&[]);
    cut.truncate(cut.len() - 20);
    let game = Game::new()
        .model("orc", 1.0, json!([]))
        .file(&part_file("orc"), cut);
    assert_eq!(refusals(game), [("unreadable model", part_file("orc"))]);
}

#[test]
fn a_models_images_are_png_files_beside_it_that_the_game_holds_never_kept_inside() {
    let model = format!("{}/model.glb", path("prop", "oak"));
    let oak = || Game::new().prop("oak", json!({ "type": "none" }));
    let with_model = |game: Game, glb: Vec<u8>| game.part("prop", "oak", "model.glb", glb);
    let bark = png(2, 2);

    let inside = with_model(oak(), glb_with_image_inside("Oak Bark", &bark));
    assert_eq!(refusals(inside), [("embedded image", model.clone())]);

    let naming = with_model(oak(), glb_naming_image("textures/oak-bark.png"));
    assert_eq!(refusals(naming), [("missing image", model.clone())]);
    with_model(oak(), glb_naming_image("textures/oak-bark.png"))
        .part("prop", "oak", "textures/oak-bark.png", bark.clone())
        .load();

    let taken = externalize(&glb_with_image_inside("Oak Bark", &bark), PROP_TEXTURES)
        .expect("the image comes out");
    assert_eq!(
        taken.images,
        [("textures/oak-bark.png".to_owned(), bark)],
        "the image comes out as a PNG part named after it"
    );
    let mut externalized = with_model(oak(), taken.glb);
    for (part, bytes) in taken.images {
        externalized = externalized.part("prop", "oak", &part, bytes);
    }
    externalized.load();
    assert_eq!(PROP_GLB.named("", "glb"), "model.glb");
}

#[test]
fn media_no_content_names_is_never_read() {
    let mut cut = glb(&[]);
    cut.truncate(cut.len() - 20);
    Game::new()
        .file("downloads/Broken Model.glb", cut)
        .file("downloads/Noise.wav", b"not a sound".to_vec())
        .load();
}

#[test]
fn a_sound_cut_short_or_that_the_game_cannot_play_is_refused() {
    let clip = format!("{}/clip.wav", path("sfx", "hit"));
    let mut cut = wav();
    cut.truncate(cut.len() - 10);
    let mut garbled = wav();
    garbled[20] = 0x77;
    let game = Game::new()
        .sfx("hit")
        .sfx("hum")
        .file(&clip, cut)
        .file(&format!("{}/clip.wav", path("sfx", "hum")), garbled);
    assert_eq!(
        refusals(game),
        [
            ("unreadable sound", clip),
            (
                "unreadable sound",
                format!("{}/clip.wav", path("sfx", "hum"))
            ),
        ]
    );
}
