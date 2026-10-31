use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use engine::core::content::{Content, ContentRegistry, FilesystemFolder, version, write};
use serde_json::json;
use testkit::Game;
use walkdir::WalkDir;

use crate::support::game;

#[test]
fn a_folder_written_from_scratch_lists_every_file_of_a_written_kind() {
    let game = rich_game();
    let declared = written_kinds(&game);
    let out = tempfile::tempdir().expect("a temp folder");

    let writer = write(game.registry(), game.load(), out.path()).expect("written");

    let written: BTreeSet<String> = writer.written().iter().cloned().collect();
    assert_eq!(written, declared);
    assert_eq!(on_disk(out.path()), declared);
}

#[test]
fn writing_a_written_folder_back_changes_no_bytes() {
    let game = rich_game();
    let out = tempfile::tempdir().expect("a temp folder");
    for (path, bytes) in game.folder().files {
        if !game.registry().writes(&path) {
            let file = out.path().join(&path);
            std::fs::create_dir_all(file.parent().expect("a parent folder")).expect("folders");
            std::fs::write(file, bytes).expect("an asset");
        }
    }
    let registry = game.registry();
    let first = write(registry, game.load(), out.path()).expect("written");

    let reread = load(registry, out.path());
    let again = write(registry, reread, out.path()).expect("written again");

    assert_eq!(again.written(), &[] as &[String], "rewritten files");
    assert_eq!(again.kept(), first.written().len());
}

/// A game of files of many types, bundles among them, and the terrain of its area.
fn rich_game() -> Game {
    game()
        .sword()
        .bust("hero")
        .babble("brute")
        .npc(
            "brute",
            30.0,
            json!([{ "type": "voice", "babble": testkit::path("babble", "brute") }]),
        )
        .edit("npc", "brute", |brute| {
            brute["held"] = json!([testkit::path("item", "sword")])
        })
        .item(
            "bone",
            json!({ "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 20 }] }),
        )
        .row(
            "memory",
            "met",
            json!({ "label": "Met", "type": "boolean", "resets": "never" }),
        )
        .vfx("sparkle")
        .shader("glow")
        .mark("news")
}

fn load(registry: &ContentRegistry, folder: &Path) -> Content {
    engine::core::content::load(
        registry,
        Arc::new(FilesystemFolder(folder.to_path_buf())),
        version::build(),
        "the written folder did not load",
        engine::core::content::LoadPurpose::Check,
    )
    .unwrap_or_else(|report| panic!("{report}"))
}

fn written_kinds(game: &Game) -> BTreeSet<String> {
    let registry = game.registry();
    game.folder()
        .files
        .into_keys()
        .filter(|path| registry.writes(path))
        .collect()
}

fn on_disk(folder: &Path) -> BTreeSet<String> {
    WalkDir::new(folder)
        .into_iter()
        .map(|entry| entry.expect("an entry"))
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            let relative = entry.path().strip_prefix(folder).expect("under the folder");
            relative.to_string_lossy().replace('\\', "/")
        })
        .collect()
}
