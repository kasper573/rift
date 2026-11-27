use engine::core::content::{FolderReader, LoadReport, SETTINGS_FILE};
use engine::systems::npc::NpcDef;
use serde_json::json;
use testkit::{Game, path, png};

use crate::support::game;

#[test]
fn a_game_folder_holds_anything_beside_its_content_each_file_named_by_its_path() {
    let mut broken_model = testkit::glb(&[]);
    broken_model.truncate(broken_model.len() - 20);
    let content = game()
        .file(
            "world/forest/npcs/orc.npc.json",
            npc_fields("Orc", &path("model", "hero")),
        )
        .file("world/forest/notes.md", "Orcs live here.")
        .file("todo.txt", "later")
        .file("downloads/Human Basic Motions 2.5.pdf", "a manual")
        .file(
            "downloads/Human Basic Motions 2.5/HumanM@Walk01 - Forward.glb",
            "not read",
        )
        .file("downloads/old.npc/config.json", "{ not content }")
        .file("downloads/archive.zip", "packed")
        .file("downloads/Broken Model.glb", broken_model)
        .file("downloads/Noise.wav", b"not a sound".to_vec())
        .file(".git/HEAD", "ref: refs/heads/main")
        .file("world/.DS_Store", "")
        .asset("art/icons/axe.png", png(1, 1))
        .edit("class", "hero", |hero| {
            hero["icon"] = json!("art/icons/axe.png")
        })
        .load();

    let orc = content
        .by_path::<NpcDef>("world/forest/npcs/orc.npc.json")
        .expect("the orc");
    assert_eq!(orc.name(&content), "orc");
    assert_eq!(orc.get(&content).name, "Orc");
}

#[test]
fn a_file_the_loader_cannot_place_by_its_name_or_where_it_lies_is_refused() {
    let npc = || npc_fields("Orc", &path("model", "hero"));
    let mut cases: Vec<(String, Game, &str)> = vec![
        (
            "orc.ncp.json".to_owned(),
            game().file("orc.ncp.json", "{}"),
            "unknown type",
        ),
        (
            "lamp.prop.json".to_owned(),
            game().file("lamp.prop.json", "{}"),
            "bundle kept as one file",
        ),
        (
            "hero.class.json".to_owned(),
            game()
                .prop("rock", json!({ "type": "none" }))
                .edit("class", "hero", |hero| {
                    hero["icon"] = json!(format!("{}/model.glb", path("prop", "rock")))
                }),
            "part of a bundle",
        ),
        (
            format!("{}/notes.txt", path("prop", "rock")),
            game().prop("rock", json!({ "type": "none" })).part(
                "prop",
                "rock",
                "notes.txt",
                "a stray file",
            ),
            "unknown part",
        ),
    ];
    for name in [
        "Orc.npc.json",
        "big orc.npc.json",
        "con.npc.json",
        "-orc.npc.json",
    ] {
        cases.push((name.to_owned(), game().file(name, npc()), "invalid name"));
    }
    for (file, game, rule) in cases {
        let report = refused(game);
        assert_eq!(only(&report, &file).0, rule, "{file}");
    }
    assert_eq!(
        only(&refused(game().file("orc.ncp.json", "{}")), "orc.ncp.json")
            .1
            .as_deref(),
        Some("\"npc\""),
        "an unknown type names the nearest"
    );

    let numbered = "orc (2).npc.json";
    let content = game().file(numbered, npc()).load();
    assert!(content.by_path::<NpcDef>(numbered).is_some());

    let report = refused(game().file("caves/a.md", "one").file("Caves/b.md", "two"));
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.problem.rule == "case clash"),
        "names that differ only by case clash: {report}"
    );

    let report = refused(game().file("game.config.json", "{}"));
    let error = report
        .errors
        .iter()
        .find(|error| error.file == "game.config.json")
        .unwrap_or_else(|| panic!("{report}"));
    assert_eq!(error.problem.rule, "unknown file");
    assert!(
        error.problem.expected.contains(SETTINGS_FILE),
        "the settings anywhere but their place say where they belong: {report}"
    );
}

fn npc_fields(name: &str, model: &str) -> Vec<u8> {
    serde_json::to_vec_pretty(&json!({
        "name": name,
        "appearance": { "model": model },
        "stats": testkit::stats(10.0),
        "leash": 8.0,
        "attitude": "friendly",
    }))
    .expect("json")
}

fn refused(game: Game) -> LoadReport {
    game.try_load().err().expect("refused")
}

/// The rule and nearest value of the one problem with `file`.
fn only<'a>(report: &'a LoadReport, file: &str) -> (&'a str, Option<String>) {
    let errors: Vec<_> = report
        .errors
        .iter()
        .filter(|error| error.file == file)
        .collect();
    assert_eq!(errors.len(), 1, "{report}");
    (errors[0].problem.rule, errors[0].problem.nearest.clone())
}

#[test]
fn a_folder_held_in_memory_lists_what_each_folder_holds_directly() {
    use engine::core::content::{ContentFolder, MemoryFolder};
    let folder = MemoryFolder {
        name: "game".to_owned(),
        files: ["a-b", "a.b", "a/x", "a/y/z", "a/y0", "a0", "b/c/d"]
            .into_iter()
            .map(|path| (path.to_owned(), std::sync::Arc::from(&b""[..])))
            .collect(),
    };
    let listed = |dir: &str| -> std::collections::BTreeSet<(String, bool)> {
        folder
            .entries(dir)
            .into_iter()
            .map(|entry| (entry.name, entry.dir))
            .collect()
    };
    let named = |entries: &[(&str, bool)]| -> std::collections::BTreeSet<(String, bool)> {
        entries
            .iter()
            .map(|(name, dir)| ((*name).to_owned(), *dir))
            .collect()
    };

    assert_eq!(
        listed(""),
        named(&[
            ("a", true),
            ("a-b", false),
            ("a.b", false),
            ("a0", false),
            ("b", true)
        ])
    );
    assert_eq!(
        listed("a"),
        named(&[("x", false), ("y", true), ("y0", false)])
    );
    assert_eq!(listed("a/y"), named(&[("z", false)]));
    assert_eq!(listed("b"), named(&[("c", true)]));
    assert!(listed("c").is_empty());
}

#[test]
fn checking_any_files_of_a_game_that_loads_finds_nothing() {
    let orcs = || {
        game()
            .npc("orc", 10.0, json!([]))
            .sfx("hit")
            .file("icons/cap.png", png(8, 8))
            .file("notes/orcs.md", "Orcs live here.")
    };
    orcs().load();
    let game = orcs();
    let registry = game.registry();
    let folder = game.folder();
    let files: Vec<String> = folder.files.keys().cloned().collect();
    let reader = FolderReader::new(registry, std::sync::Arc::new(folder));
    let errors = reader.check(&files).errors;
    assert!(errors.is_empty(), "the whole folder at once: {errors:?}");
    for file in &files {
        let errors = reader.check(std::slice::from_ref(file)).errors;
        assert!(errors.is_empty(), "{file}: {errors:?}");
    }
}
