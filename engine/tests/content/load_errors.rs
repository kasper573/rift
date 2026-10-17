use engine::core::content::ErrorPlace;
use serde_json::json;

use crate::support::{TempFolder, load};

const PINNED: &str = include_str!("load_errors.txt");

#[test]
fn every_rule_names_its_file_line_pointer_and_nearest_value() {
    let folder = TempFolder::of_template();
    folder.edit("game.json", |game| {
        game["player"] = json!("adventurr");
        game["initial_spawn"]["x"] = json!(40.0);
        game["initial_spawn"]["y"] = json!(40.0);
    });
    folder.write(
        "tables/sfx/splash.json",
        &json!({ "file": "audio/combat/splash.wav", "role": "combat", "volume": 1.0, "pitch": 1.0 })
            .to_string(),
    );
    folder.remove("tables/input/interact.json");
    folder.write("tables/class/apprentice.json", "{ \"name\": 3 }");
    folder.write(
        "tables/area/cove.json",
        &serde_json::to_string_pretty(&json!({
            "name": "Cove",
            "map": { "type": "tilemap", "size": { "width": 4, "height": 4 }, "tile": 16, "ground": ["ground"] },
            "modules": [
                { "type": "scenery", "at": { "x": 9.0, "y": 0.0 }, "tileset": "beach_tileset", "tiles": [[338]] }
            ]
        }))
        .expect("json"),
    );
    folder.write(
        "tables/area/cove/palette.csv",
        "tileset,tile,flip\nbeach_tileset,338,\n",
    );
    folder.write(
        "tables/area/cove/ground.csv",
        "0,0,0,0\n0,0,7,0\n0,0,0,0\n0,0,0,0\n",
    );
    folder.write(
        "tables/area/cove/walk.csv",
        "1,1,1,1\n1,1,1,1\n1,1,1,1\n1,1,1,1\n",
    );
    folder.write(
        "tables/test_character/lost.json",
        &serde_json::to_string_pretty(&json!({
            "name": "Lost",
            "class": "adventurer",
            "known_areas": ["cove"],
            "start": { "area": "start", "x": 40.0, "y": 40.0 }
        }))
        .expect("json"),
    );

    let report = load(folder.path()).err().expect("refused");

    let text = report.to_string();
    let body = text
        .split_once("\nresult   ")
        .map(|(_, body)| body)
        .expect("a result line");
    assert_eq!(
        body.trim_end(),
        PINNED.trim_end(),
        "the report text\n{text}"
    );
}

#[test]
fn a_malformed_palette_names_the_row_and_column_of_each_bad_cell() {
    let folder = TempFolder::of_template();
    folder.write(
        "tables/area/cove.json",
        &json!({
            "name": "Cove",
            "map": { "type": "tilemap", "size": { "width": 4, "height": 4 }, "tile": 16, "ground": ["ground"] }
        })
        .to_string(),
    );
    folder.write(
        "tables/area/cove/palette.csv",
        "tileset,tile,flip\nbeach_tileset,x,\nbeach_tileset,1,sideways\nbeach_tileset,1\n",
    );
    folder.write("tables/area/cove/ground.csv", &"0,0,0,0\n".repeat(4));
    folder.write("tables/area/cove/walk.csv", &"1,1,1,1\n".repeat(4));

    let report = load(folder.path()).err().expect("refused");

    let palette: Vec<(ErrorPlace, &str)> = report
        .errors
        .iter()
        .filter(|error| error.file == "tables/area/cove/palette.csv")
        .map(|error| (error.at.clone(), error.problem.expected.as_str()))
        .collect();
    assert_eq!(
        palette,
        vec![
            (ErrorPlace::Csv { row: 2, column: 2 }, "a tile number"),
            (
                ErrorPlace::Csv { row: 3, column: 3 },
                "empty, horizontal, vertical or both"
            ),
            (
                ErrorPlace::Csv { row: 4, column: 1 },
                "three fields: tileset,tile,flip"
            ),
        ]
    );
}

#[test]
fn a_bust_listed_before_the_generic_ones_is_placed_at_its_own_line() {
    let folder = TempFolder::of_template();
    let model = folder.path().join("tables/model/adventurer.json");
    let text = std::fs::read_to_string(&model).expect("read");
    let busts = r#"{ "modules": [{ "type": "bust", "own": [{ "name": "Scar", "file": "busts/adventurer/scar.png" }], "neutral": "busts/adventurer/neutral.png", "happy": "busts/adventurer/happy.png", "sad": "busts/adventurer/sad.png", "angry": "busts/adventurer/angry.png", "surprised": "busts/adventurer/surprised.png", "thinking": "busts/adventurer/thinking.png" }],"#;
    let edited = text.replacen('{', busts, 1);
    std::fs::write(&model, edited).expect("written");

    let report = load(folder.path()).err().expect("refused");

    let own = report
        .errors
        .iter()
        .find(|error| {
            error.file == "tables/model/adventurer.json" && error.problem.found.contains("scar.png")
        })
        .expect("the own bust is refused");
    assert_eq!(own.problem.rule, "missing file");
    assert!(
        matches!(&own.at, ErrorPlace::Json { pointer, .. } if pointer == "/modules/0/own/0/file"),
        "{:?}",
        own.at
    );
}

#[test]
fn a_binding_the_game_never_reads_for_its_action_is_refused() {
    let folder = TempFolder::of_template();
    folder.edit("tables/input/carry_item.json", |row| {
        row["bindings"] = json!([{ "type": "click", "button": "Left", "times": 1 }]);
    });

    let report = load(folder.path()).err().expect("refused");

    let rules: Vec<(&str, &str)> = report
        .errors
        .iter()
        .map(|error| (error.problem.rule, error.problem.expected.as_str()))
        .collect();
    assert_eq!(
        rules,
        vec![("unread binding", "a binding the game reads for this action")]
    );
    assert_eq!(report.errors[0].file, "tables/input/carry_item.json");
}

#[test]
fn a_shader_that_does_not_compile_is_refused_at_its_line() {
    let folder = TempFolder::of_template();
    folder.write(
        "assets/shaders/tint.wgsl",
        "fn vfx(uv: vec2<f32>) -> vec4<f32> {\n    return oops;\n}\n",
    );

    let report = load(folder.path()).err().expect("refused");

    let error = report
        .errors
        .iter()
        .find(|error| error.file == "assets/shaders/tint.wgsl")
        .expect("the shader is refused");
    assert_eq!(error.problem.rule, "shader does not compile");
    assert!(
        matches!(error.at, ErrorPlace::Text { line: 2, .. }),
        "{:?}",
        error.at
    );
}

#[test]
fn a_shader_row_without_its_file_is_refused() {
    let folder = TempFolder::of_template();
    folder.remove("assets/shaders/tint.wgsl");

    let report = load(folder.path()).err().expect("refused");

    let error = report
        .errors
        .iter()
        .find(|error| error.file == "tables/vfx_shader/tint.json")
        .expect("the row is refused");
    assert_eq!(error.problem.rule, "shader file missing");
    assert_eq!(error.problem.expected, "assets/shaders/tint.wgsl");
}

#[test]
fn a_layer_filling_a_parameter_its_shader_lacks_is_refused() {
    let folder = TempFolder::of_template();
    folder.edit("tables/vfx/healed.json", |row| {
        row["layers"][1]["params"]["glow"] = json!(1.0);
    });

    let report = load(folder.path()).err().expect("refused");

    let error = report
        .errors
        .iter()
        .find(|error| error.file == "tables/vfx/healed.json")
        .expect("the layer is refused");
    assert_eq!(error.problem.rule, "unknown shader parameter");
    assert!(
        matches!(&error.at, ErrorPlace::Json { pointer, .. } if pointer == "/layers/1/params/glow"),
        "{:?}",
        error.at
    );
}
