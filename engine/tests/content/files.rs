use engine::core::content::{FolderReader, LoadReport, SETTINGS_FILE};
use engine::systems::npc::NpcDef;
use serde_json::json;
use testkit::{Game, path, png};

use crate::support::game;

#[test]
fn content_loads_from_any_folder_and_is_named_by_its_path() {
    let content = game()
        .file(
            "world/forest/npcs/orc.npc.json",
            npc_fields("Orc", &path("model", "hero")),
        )
        .file("world/forest/notes.md", "Orcs live here.")
        .load();

    let orc = content
        .by_path::<NpcDef>("world/forest/npcs/orc.npc.json")
        .expect("the orc");
    assert_eq!(orc.name(&content), "orc");
    assert_eq!(orc.get(&content).name, "Orc");
}

#[test]
fn a_file_of_no_known_type_is_refused_naming_the_nearest_type() {
    let report = refused(game().file("orc.ncp.json", "{}"));
    let error = only(&report, "orc.ncp.json");
    assert_eq!(error.0, "unknown type");
    assert_eq!(error.1.as_deref(), Some("\"npc\""));

    let report = refused(game().file("todo.txt", "later"));
    assert_eq!(only(&report, "todo.txt").0, "unknown file");
}

#[test]
fn the_settings_anywhere_but_their_place_are_refused_saying_where_they_belong() {
    let report = refused(game().file("game.config.json", "{}"));

    let error = report
        .errors
        .iter()
        .find(|error| error.file == "game.config.json")
        .unwrap_or_else(|| panic!("{report}"));
    assert_eq!(error.problem.rule, "unknown file");
    assert!(error.problem.expected.contains(SETTINGS_FILE), "{report}");
}

#[test]
fn a_name_a_disk_or_a_url_would_change_is_refused() {
    for name in [
        "Orc.npc.json",
        "big orc.npc.json",
        "con.npc.json",
        "-orc.npc.json",
    ] {
        let report = refused(game().file(name, npc_fields("Orc", &path("model", "hero"))));

        assert_eq!(only(&report, name).0, "invalid name", "{name}");
    }

    let numbered = "orc (2).npc.json";
    let content = game()
        .file(numbered, npc_fields("Orc", &path("model", "hero")))
        .load();
    assert!(content.by_path::<NpcDef>(numbered).is_some());
}

#[test]
fn names_that_differ_only_by_case_clash() {
    let report = refused(game().file("caves/a.md", "one").file("Caves/b.md", "two"));

    let rules: Vec<&str> = report
        .errors
        .iter()
        .map(|error| error.problem.rule)
        .collect();
    assert!(rules.contains(&"case clash"), "{report}");
}

#[test]
fn a_dotted_name_must_take_the_shape_its_type_does() {
    for (game, file, rule) in [
        (
            game().file("old.stuff/a.md", "notes"),
            "old.stuff",
            "unknown type",
        ),
        (
            game().file("lamp.model_part.json", "{}"),
            "lamp.model_part.json",
            "bundle kept as one file",
        ),
        (
            game().file("orc.npc/config.json", "{}"),
            "orc.npc",
            "folder with a dot",
        ),
    ] {
        let report = refused(game);
        assert_eq!(only(&report, file).0, rule, "{file}");
    }
}

#[test]
fn hidden_files_are_left_alone() {
    game()
        .file(".git/HEAD", "ref: refs/heads/main")
        .file("world/.DS_Store", "")
        .load();
}

#[test]
fn a_bundles_part_is_never_named_from_outside_it() {
    let report = refused(game().edit("class", "hero", |hero| {
        hero["icon"] = json!(format!("{}/model.glb", path("model", "hero")));
    }));

    assert_eq!(only(&report, "hero.class.json").0, "part of a bundle");
}

#[test]
fn media_is_named_by_its_path_from_the_game_folder() {
    game()
        .asset("art/icons/axe.png", png(1, 1))
        .edit("class", "hero", |hero| {
            hero["icon"] = json!("art/icons/axe.png")
        })
        .load();
}

fn npc_fields(name: &str, model: &str) -> Vec<u8> {
    serde_json::to_vec_pretty(&json!({
        "name": name,
        "model": model,
        "stats": testkit::stats(10.0),
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
            .bust_of("orc")
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
