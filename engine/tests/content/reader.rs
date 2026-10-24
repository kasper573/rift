use std::collections::BTreeMap;
use std::sync::Arc;

use engine::core::audio::playback::SfxDef;
use engine::core::content::{FolderReader, write};
use engine::systems::GameSettings;
use engine::systems::actor::model::{GenericExpression, ModelDef, ModelModule};
use engine::systems::area::AreaDef;
use engine::systems::area::tileset::SurfaceDef;
use serde_json::json;
use testkit::{Game, path};

use crate::support::game;

fn reader(game: &Game) -> FolderReader {
    FolderReader::new(game.registry(), Arc::new(game.folder()))
}

#[test]
fn a_bundle_reads_with_its_parts() {
    let game = game().model("hero", 1.0, json!([])).busts("hero");
    let model: ModelDef = reader(&game).read(&path("model", "hero")).expect("reads");

    let hero = path("model", "hero");
    assert_eq!(model.model.as_str(), format!("{hero}/model.glb"));
    let busts = model
        .modules
        .iter()
        .find_map(|module| match module {
            ModelModule::Bust(busts) => Some(busts),
            _ => None,
        })
        .expect("a bust module");
    assert_eq!(
        busts.generic.of(GenericExpression::Angry).as_str(),
        format!("{hero}/busts/angry.png")
    );
}

/// A sand surface stepping with the sound `step`: a file naming another by its path.
fn sand(game: Game, footstep: &str) -> Game {
    game.sfx("step").row(
        "surface",
        "sand",
        json!({ "name": "Sand", "footstep": footstep }),
    )
}

#[test]
fn a_reference_reads_as_the_id_of_the_path_it_names() {
    let step = path("sfx", "step");
    let game = sand(game(), &step);
    let reader = reader(&game);
    let sand: SurfaceDef = reader.read(&path("surface", "sand")).expect("reads");

    assert_eq!(reader.path_of(sand.footstep), Some(step.as_str()));
    assert_eq!(reader.id_of::<SfxDef>(&step), Some(sand.footstep));
}

#[test]
fn a_file_naming_a_path_no_file_has_does_not_read() {
    let game = sand(game(), "nowhere/gone.sfx");
    let errors = reader(&game)
        .read::<SurfaceDef>(&path("surface", "sand"))
        .err()
        .expect("refused");
    assert!(
        errors
            .iter()
            .any(|error| error.problem.rule == "missing file"),
        "{errors:?}"
    );
}

#[test]
fn a_file_read_in_another_scope_names_the_ids_of_its_paths() {
    let step = path("sfx", "step");
    let held = sand(game(), &step);
    let other = reader(&sand(game().sfx("alarm"), &step));
    let sand: SurfaceDef = reader(&held)
        .read_in(&path("surface", "sand"), other.scope())
        .expect("reads");

    assert_eq!(other.path_of(sand.footstep), Some(step.as_str()));

    let without_it = reader(&game().sfx("miss"));
    assert!(
        reader(&held)
            .read_in::<SurfaceDef>(&path("surface", "sand"), without_it.scope())
            .is_err(),
        "a file the other scope does not hold does not read in it"
    );
}

#[test]
fn a_file_of_another_type_does_not_read() {
    let game = game().sfx("hit");
    assert!(reader(&game).read::<ModelDef>(&path("sfx", "hit")).is_err());
}

#[test]
fn a_file_writes_back_exactly_as_writing_the_whole_game_does() {
    let game = || {
        game()
            .sfx("hit")
            .model("hero", 1.0, json!([]))
            .busts("hero")
    };
    let whole = tempfile::tempdir().expect("a folder");
    write(game().registry(), game().load(), whole.path()).expect("written");
    let game = game();
    let reader = reader(&game);
    let same_as_whole = |files: std::collections::BTreeMap<String, Vec<u8>>| {
        assert!(!files.is_empty());
        for (file, bytes) in files {
            let written = std::fs::read(whole.path().join(&file)).expect("written whole");
            assert_eq!(bytes, written, "{file}");
        }
    };

    let area = path("area", "start");
    let start: AreaDef = reader.read(&area).expect("reads");
    let files = reader.write(&area, &start).expect("writes").files;
    assert!(files.contains_key(&format!("{area}/scene.json")));
    assert!(files.contains_key(&format!("{area}/layers/ground.csv")));
    same_as_whole(files);
    let hero = path("model", "hero");
    let model: ModelDef = reader.read(&hero).expect("reads");
    same_as_whole(reader.write(&hero, &model).expect("writes").files);
    let settings: GameSettings = reader.settings().expect("reads");
    same_as_whole(reader.write_settings(&settings).expect("writes"));
}

#[test]
fn a_part_an_area_no_longer_writes_is_removed() {
    let game = game();
    let reader = reader(&game);
    let area = path("area", "start");
    let mut start: AreaDef = reader.read(&area).expect("reads");
    start.blueprint.scene.grids.layers.clear();

    let written = reader.write(&area, &start).expect("writes");

    let ground = format!("{area}/layers/ground.csv");
    assert!(!written.files.contains_key(&ground));
    assert_eq!(written.removed, [ground]);
}

fn renames(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(from, to)| ((*from).to_owned(), (*to).to_owned()))
        .collect()
}

fn text(files: &BTreeMap<String, Vec<u8>>, file: &str) -> String {
    String::from_utf8(files[file].clone()).expect("text")
}

#[test]
fn a_rename_writes_the_new_path_wherever_a_file_names_the_old_one() {
    let game = game();
    let area = path("area", "start");
    let ground = path("tileset", "start_ground");
    let meadow = path("tileset", "meadow");

    let written = reader(&game)
        .rewritten(&area, &renames(&[(&ground, &meadow)]))
        .expect("rewritten");

    let palette = text(&written.files, &format!("{area}/palette.csv"));
    assert_eq!(palette, format!("tileset,tile,flip\n{meadow},0,\n"));
    assert!(written.removed.is_empty());
}

#[test]
fn a_rename_of_a_path_a_file_does_not_name_writes_it_back_unchanged() {
    let game = game().sfx("hit");
    let hit = path("sfx", "hit");

    let reader = reader(&game);
    let sfx: SfxDef = reader.read(&hit).expect("reads");

    let written = reader
        .rewritten(&hit, &renames(&[("elsewhere.npc.json", "moved.npc.json")]))
        .expect("rewritten");

    assert_eq!(
        written.files,
        reader.write(&hit, &sfx).expect("writes").files
    );
}

#[test]
fn a_file_naming_a_path_that_moved_away_reads_and_names_where_it_went() {
    let gone = "lost/step.sfx";
    let step = path("sfx", "step");
    let game = sand(game(), gone);
    let sand = path("surface", "sand");
    assert!(reader(&game).read::<SurfaceDef>(&sand).is_err());
    assert!(
        reader(&game).rewritten(&sand, &BTreeMap::new()).is_err(),
        "without the move it names, the file is not rewritten"
    );

    let written = reader(&game)
        .rewritten(&sand, &renames(&[(gone, &step)]))
        .expect("rewritten");

    let fields = text(&written.files, &engine::core::content::fields_file(&sand));
    assert!(fields.contains(&format!("\"{step}\"")), "{fields}");
    assert!(!fields.contains(gone), "{fields}");
}

#[test]
fn a_rename_reaches_the_games_settings() {
    let game = game();
    let start = path("area", "start");
    let harbour = path("area", "harbour");

    let written = reader(&game)
        .rewritten_settings(&renames(&[(&start, &harbour)]))
        .expect("rewritten");

    let settings = text(&written, engine::core::content::SETTINGS_FILE);
    assert!(settings.contains(&format!("\"{harbour}\"")), "{settings}");
    assert!(!settings.contains(&format!("\"{start}\"")), "{settings}");
}
