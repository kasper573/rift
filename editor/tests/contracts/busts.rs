use editor::app::Editor;
use editor::views::bust::faces::{self, BustFace};
use engine::systems::dialogue::bust::{BustDef, GenericExpression};
use serde_json::json;
use strum::VariantArray;
use testkit::{Game, path, png};

use crate::support;

fn open(scratch: &tempfile::TempDir) -> Editor {
    let game: Game = support::game()
        .npc("orc", 10.0, json!([]))
        .bust_of("orc")
        .part("bust", "orc", "faces/smug.png", png(4, 4));
    support::open(game, scratch)
}

fn faces(editor: &Editor) -> Vec<BustFace> {
    faces_of(editor, &path("bust", "orc"))
}

#[test]
fn a_bust_has_a_face_for_every_picture_its_bundle_keeps() {
    let scratch = tempfile::tempdir().expect("scratch");
    let editor = open(&scratch);
    let faces = faces(&editor);

    assert_eq!(faces.len(), GenericExpression::VARIANTS.len() + 1);
    let smug = faces.iter().find(|face| face.name == "smug").expect("smug");
    assert!(smug.own);
    assert_eq!(smug.file, format!("{}/faces/smug.png", path("bust", "orc")));
    assert!(
        faces
            .iter()
            .filter(|face| face.name != "smug")
            .all(|face| !face.own)
    );
}

#[test]
fn renaming_an_own_face_moves_its_picture_and_the_game_still_loads() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = open(&scratch);
    let smug = faces(&editor)
        .into_iter()
        .find(|face| face.name == "smug")
        .expect("smug");

    assert!(faces::rename(
        &mut editor,
        &path("bust", "orc"),
        &smug,
        "sly"
    ));

    support::settled(&mut editor);
    let names: Vec<String> = faces(&editor).into_iter().map(|face| face.name).collect();
    assert!(names.contains(&"sly".to_owned()), "{names:?}");
    assert!(!names.contains(&"smug".to_owned()), "{names:?}");
}

#[test]
fn a_face_keeps_its_name_when_asked_for_one_no_file_can_hold() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = open(&scratch);
    let smug = faces(&editor)
        .into_iter()
        .find(|face| face.name == "smug")
        .expect("smug");

    for wanted in ["grin!", "a/b", "happy (2) x"] {
        assert!(
            !faces::rename(&mut editor, &path("bust", "orc"), &smug, wanted),
            "{wanted}"
        );
    }
    let names: Vec<String> = faces(&editor).into_iter().map(|face| face.name).collect();
    assert!(names.contains(&"smug".to_owned()), "{names:?}");
}

#[test]
fn removing_an_own_face_removes_its_picture_and_a_generic_face_stays() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = open(&scratch);
    let faces_before = faces(&editor);
    let bust = path("bust", "orc");
    let smug = faces_before
        .iter()
        .find(|face| face.own)
        .expect("an own face");
    let happy = faces_before
        .iter()
        .find(|face| face.name == "happy")
        .expect("a generic face");

    assert!(faces::remove(&mut editor, &bust, smug, "happy"));
    assert!(!faces::remove(&mut editor, &bust, happy, "sad"));

    support::settled(&mut editor);
    assert_eq!(faces(&editor).len(), GenericExpression::VARIANTS.len());
}

#[test]
fn renaming_a_face_leaves_the_lines_of_speakers_shown_in_another_bust_alone() {
    let scratch = tempfile::tempdir().expect("scratch");
    let line = |npc: &str| json!({ "by": { "type": "npc", "npc": path("npc", npc), "face": "smug" }, "text": ["Hm."] });
    let game = ["grunt", "lady"].iter().fold(support::game(), |game, npc| {
        game.npc(npc, 10.0, json!([]))
            .bust_of(npc)
            .part("bust", npc, "faces/smug.png", png(4, 4))
    });
    let game = game.script(
        "talk",
        "npc",
        json!({
            "start": { "type": "on", "event": "interacted", "verb": "talk", "reach": 2.0, "next": "said" },
            "said": { "type": "dialogue", "lines": [line("grunt"), line("lady")], "choices": [{ "label": ["Bye."] }] },
        }),
    );
    let mut editor = support::open(game, &scratch);
    let grunt = path("bust", "grunt");
    let smug = faces_of(&editor, &grunt)
        .into_iter()
        .find(|face| face.name == "smug")
        .expect("smug");

    assert!(faces::rename(&mut editor, &grunt, &smug, "sly"));

    support::settled(&mut editor);
    let asked = |bust: &str| -> Vec<String> {
        faces::lines(&editor, &path("bust", bust))
            .into_iter()
            .map(|line| line.face)
            .collect()
    };
    assert_eq!(asked("grunt"), ["sly"]);
    assert_eq!(asked("lady"), ["smug"]);
}

fn faces_of(editor: &Editor, bust: &str) -> Vec<BustFace> {
    let bust = editor
        .read::<BustDef>(bust)
        .expect("a project")
        .expect("the bust reads");
    faces::faces(&bust)
}
