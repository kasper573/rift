use serde_json::json;
use testkit::{Game, glb, part_file, path, png, wav};

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
