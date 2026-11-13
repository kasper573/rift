use engine::core::content::{ErrorPlace, Fixture, LoadReport, SETTINGS_FILE};
use engine::core::ground::{GroundSize, PaintMap, TileAt};
use engine::core::vfx::Stage;
use engine::systems::actor::model::MAX_CHARACTER_TRIANGLES;
use engine::systems::area::AreaDef;
use engine::systems::area::prop::MAX_PROP_TRIANGLES;
use engine::systems::input::map::InputFixture;
use serde_json::{Value, json};
use testkit::{Game, Map, glb_of, path, png};

use crate::support::game;

#[test]
fn every_rule_names_its_file_line_pointer_and_nearest_value() {
    let report = game()
        .edit_settings(|settings| settings["player"] = json!("heroo.player.json"))
        .sfx("slash")
        .row("sfx", "splash", json!({ "volume": 1.0, "pitch": 1.0 }))
        .file("apprentice.class.json", "{ \"name\": 3 }")
        .edit("class", "hero", |hero| {
            hero["icon"] = json!("icons/heroo.png")
        })
        .area("cove", &Map::open(8, 8), json!([]))
        .part("area", "cove", "paint/0_0.png", png(8, 8))
        .row(
            "test_character",
            "lost",
            json!({
                "name": "Lost",
                "class": path("class", "hero"),
                "appearance": { "model": path("model", "hero") },
                "known_areas": [path("area", "cove")],
                "start": { "area": path("area", "start"), "x": 40.0, "y": 40.0 },
            }),
        )
        .try_load()
        .err()
        .expect("refused");

    let json = |line: usize, column: usize, pointer: &str| ErrorPlace::Json {
        line,
        column,
        pointer: pointer.to_owned(),
    };
    assert_eq!(
        named(&report),
        [
            (
                SETTINGS_FILE,
                json(8, 13, "/player"),
                "missing file",
                Some("\"hero.player.json\""),
            ),
            (
                "apprentice.class.json",
                json(1, 11, "/name"),
                "wrong type",
                None
            ),
            ("cove.area", ErrorPlace::WholeFile, "missing file", None),
            (
                "cove.area/paint/0_0.png",
                ErrorPlace::WholeFile,
                "wrong size",
                None,
            ),
            (
                "hero.class.json",
                json(3, 11, "/icon"),
                "missing file",
                Some("\"icons/hero.png\""),
            ),
            (
                "lost.test_character.json",
                json(7, 18, "/known_areas"),
                "start not known",
                None,
            ),
            ("splash.sfx", ErrorPlace::WholeFile, "missing file", None),
        ],
        "{report}"
    );
    let text = report.to_string();
    for at in [
        "fixtures/game.config.json:8:13  /player",
        "cove.area/paint/0_0.png  whole file",
        "nearest   \"icons/hero.png\"",
    ] {
        assert!(text.contains(at), "the report says {at}:\n{text}");
    }

    let unwalkable = game()
        .edit_settings(|settings| {
            settings["initial_spawn"]["x"] = json!(40.0);
            settings["initial_spawn"]["y"] = json!(40.0);
        })
        .try_load()
        .err()
        .expect("refused");
    assert_eq!(
        named(&unwalkable),
        [(
            SETTINGS_FILE,
            json(3, 20, "/initial_spawn"),
            "not walkable",
            Some("9.15, 7.15"),
        )],
        "{unwalkable}"
    );
}

fn named(report: &LoadReport) -> Vec<(&str, ErrorPlace, &str, Option<&str>)> {
    report
        .errors
        .iter()
        .map(|error| {
            (
                error.file.as_str(),
                error.at.clone(),
                error.problem.rule,
                error.problem.nearest.as_deref(),
            )
        })
        .collect()
}

#[test]
fn an_areas_terrain_parts_are_held_to_its_size_and_depth() {
    struct Case {
        part: &'static str,
        png: Vec<u8>,
        rule: &'static str,
    }
    let cases = [
        Case {
            part: "height/0_0.png",
            png: png(17, 17),
            rule: "wrong format",
        },
        Case {
            part: "height/0_0.png",
            png: grey16(16, 17),
            rule: "wrong size",
        },
        Case {
            part: "paint/0_0.png",
            png: png(16, 15),
            rule: "wrong size",
        },
        Case {
            part: "paint/0_0.png",
            png: png(16, 16),
            rule: "wrong format",
        },
        Case {
            part: "paint/0_0.png",
            png: b"not a png".to_vec(),
            rule: "unreadable image",
        },
        Case {
            part: "paint/1_0.png",
            png: grey8(16, 16),
            rule: "not a tile of the ground",
        },
        Case {
            part: "height/00_0.png",
            png: grey16(17, 17),
            rule: "not a tile of the ground",
        },
    ];
    for case in cases {
        let report = game()
            .area("cove", &Map::open(8, 8), json!([]))
            .part("area", "cove", case.part, case.png)
            .part("area", "cove", "height_overview.png", grey16(3, 3))
            .part("area", "cove", "paint_overview.png", grey8(2, 2))
            .try_load()
            .err()
            .expect("refused");
        let rules = rules_in(&report, &format!("cove.area/{}", case.part));
        assert!(
            rules.iter().any(|(rule, _)| *rule == case.rule),
            "{} with {}: {report}",
            case.part,
            case.rule
        );
    }

    // The heights an area spans hold ground level and some height besides.
    for (lowest, highest, rule) in [
        (1.0, 16.0, "above ground level"),
        (-8.0, -1.0, "below ground level"),
        (0.0, 0.0, "empty range"),
    ] {
        let report = game()
            .area("cove", &Map::open(8, 8), json!([]))
            .edit("area", "cove", |cove| {
                cove["height_range"] = json!({ "lowest": lowest, "highest": highest });
            })
            .try_load()
            .err()
            .expect("refused");
        assert!(
            rules_in(&report, "cove.area/config.json")
                .iter()
                .any(|(found, _)| *found == rule),
            "{lowest} to {highest} m: {report}"
        );
    }

    // A ground with tiles keeps an overview of them, and it must say what the tiles say.
    let size = GroundSize {
        width: 8,
        height: 8,
    };
    let painted = PaintMap::from_fn(size, |column, _| u8::from(column < 8));
    let tile = painted
        .encode_tile(TileAt { column: 0, row: 0 })
        .expect("paint that isn't plain")
        .to_vec();
    let overview = painted.encode_overview().expect("paint that isn't plain");
    let without = game()
        .area("cove", &Map::open(8, 8), json!([]))
        .part("area", "cove", "paint/0_0.png", tile.clone())
        .try_load()
        .err()
        .expect("refused without an overview");
    assert!(
        rules_in(&without, "cove.area")
            .iter()
            .any(|(rule, expected)| *rule == "missing file" && expected.contains("paint_overview")),
        "{without}"
    );
    let stale = game()
        .area("cove", &Map::open(8, 8), json!([]))
        .part("area", "cove", "paint/0_0.png", tile.clone())
        .part("area", "cove", "paint_overview.png", grey8(2, 2))
        .try_load()
        .err()
        .expect("refused with a stale overview");
    assert!(
        rules_in(&stale, "cove.area/paint_overview.png")
            .iter()
            .any(|(rule, _)| *rule == "overview out of date"),
        "{stale}"
    );
    let content = game()
        .area("cove", &Map::open(8, 8), json!([]))
        .part("area", "cove", "paint/0_0.png", tile)
        .part("area", "cove", "paint_overview.png", overview)
        .load();
    assert!(content.by_path::<AreaDef>(&path("area", "cove")).is_some());
}

/// A 16-bit grey PNG of `width` × `height` black pixels.
fn grey16(width: u32, height: u32) -> Vec<u8> {
    grey(width, height, png::BitDepth::Sixteen)
}

/// An 8-bit grey PNG of `width` × `height` black pixels.
fn grey8(width: u32, height: u32) -> Vec<u8> {
    grey(width, height, png::BitDepth::Eight)
}

fn grey(width: u32, height: u32, depth: png::BitDepth) -> Vec<u8> {
    let bytes_per_pixel = match depth {
        png::BitDepth::Sixteen => 2,
        _ => 1,
    };
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(depth);
    let mut writer = encoder.write_header().expect("a png header");
    writer
        .write_image_data(&vec![0; (width * height * bytes_per_pixel) as usize])
        .expect("png pixels");
    writer.finish().expect("a png");
    bytes
}

#[test]
fn a_model_over_the_style_budget_is_refused_at_its_row() {
    let rock = format!("{}/model.glb", path("prop", "rock"));
    let report = game()
        .file(
            &testkit::part_file("hero"),
            glb_of(MAX_CHARACTER_TRIANGLES + 1, &[]),
        )
        .prop("rock", json!({ "type": "none" }))
        .file(&rock, glb_of(MAX_PROP_TRIANGLES + 1, &[]))
        .try_load()
        .err()
        .expect("refused");
    let mut refused: Vec<(&str, String)> = report
        .errors
        .iter()
        .map(|error| (error.problem.rule, error.file.clone()))
        .collect();
    refused.sort();
    assert_eq!(
        refused,
        [
            ("out of range", path("model", "hero")),
            (
                "out of range",
                format!("{}/config.json", path("prop", "rock"))
            ),
        ]
    );
}

#[test]
fn a_part_a_bundle_type_never_declares_is_refused() {
    let report = game()
        .prop("rock", json!({ "type": "none" }))
        .part("prop", "rock", "notes.txt", "a stray file")
        .try_load()
        .err()
        .expect("refused");

    let rules = rules_in(&report, &format!("{}/notes.txt", path("prop", "rock")));
    assert_eq!(rules.len(), 1, "{report}");
    assert_eq!(rules[0].0, "unknown part");
}

#[test]
fn the_input_file_binds_every_action_only_as_the_game_reads_it() {
    struct Case {
        case: &'static str,
        change: fn(&mut Value),
        refusal: (&'static str, &'static str),
    }
    let cases = [
        Case {
            case: "a click carrying an item",
            change: |input| {
                input["actions"]["carry_item"]["bindings"] =
                    json!([{ "type": "click", "button": "Left", "times": 1 }]);
            },
            refusal: ("variant the engine never reads", "drag"),
        },
        Case {
            case: "an action left out",
            change: |input| {
                input["actions"]
                    .as_object_mut()
                    .expect("the actions")
                    .remove("respawn");
            },
            refusal: ("missing input", "every action the engine reads"),
        },
    ];
    for Case {
        case,
        change,
        refusal,
    } in cases
    {
        let report = Game::new()
            .engine_rows()
            .edit("input", InputFixture::Game.name(), change)
            .whole()
            .try_load()
            .err()
            .expect(case);

        assert_eq!(
            rules_in(&report, &InputFixture::Game.path()),
            vec![refusal],
            "{case}"
        );
    }
}

#[test]
fn a_basic_attack_that_costs_or_cools_down_is_refused() {
    let report = game()
        .sword()
        .item("ore", json!({ "kind": { "type": "resource" } }))
        .edit("skill", "strike", |strike| {
            strike["cost"] = json!([{ "item": path("item", "ore"), "count": 1 }]);
            strike["cooldown"] = json!(2.0);
        })
        .try_load()
        .err()
        .expect("refused");

    assert_eq!(
        rules_in(&report, "strike.skill.json"),
        vec![
            (
                "basic attack with a cost",
                "nothing: a click on an enemy must always answer"
            ),
            (
                "basic attack with a cooldown",
                "nothing: a click on an enemy must always answer"
            ),
        ]
    );
}

#[test]
fn a_fixture_shader_without_the_parameter_the_engine_sets_is_refused() {
    let report = Game::new()
        .shader_on("skill_area", Stage::Ground)
        .whole()
        .try_load()
        .err()
        .expect("refused");

    let error = report
        .errors
        .iter()
        .find(|error| error.file == "fixtures/vfx/skill_area.vfx_shader/config.json")
        .expect("the shader row is refused");
    assert_eq!(error.problem.rule, "fixture entry missing");
    assert!(
        matches!(&error.at, ErrorPlace::Json { pointer, .. } if pointer == "/params"),
        "{:?}",
        error.at
    );
}

#[test]
fn a_shader_parameter_named_with_a_leading_underscore_is_refused() {
    let report = Game::new()
        .shader("tint")
        .edit("vfx_shader", "tint", |shader| {
            shader["params"] =
                json!([{ "name": "_color", "kind": "color", "default": [255, 255, 255, 255] }]);
        })
        .try_load()
        .err()
        .expect("refused");

    let error = report
        .errors
        .iter()
        .find(|error| error.file == "tint.vfx_shader/config.json")
        .expect("the shader is refused");
    assert_eq!(error.problem.rule, "invalid parameter name");
}

#[test]
fn a_shader_that_does_not_compile_is_refused_at_its_line() {
    let report = Game::new()
        .shader("tint")
        .part(
            "vfx_shader",
            "tint",
            "shader.wgsl",
            b"fn vfx(uv: vec2<f32>) -> vec4<f32> {\n    return oops;\n}\n".to_vec(),
        )
        .try_load()
        .err()
        .expect("refused");

    let error = report
        .errors
        .iter()
        .find(|error| error.file == "tint.vfx_shader/shader.wgsl")
        .expect("the shader is refused");
    assert_eq!(error.problem.rule, "shader does not compile");
    assert!(
        matches!(error.at, ErrorPlace::Text { line: 2, .. }),
        "{:?}",
        error.at
    );
}

#[test]
fn a_shader_without_its_source_is_refused() {
    let report = Game::new()
        .shader("tint")
        .remove("tint.vfx_shader/shader.wgsl")
        .try_load()
        .err()
        .expect("refused");

    let error = report
        .errors
        .iter()
        .find(|error| error.file == "tint.vfx_shader")
        .expect("the shader is refused");
    assert_eq!(error.problem.rule, "missing file");
    assert_eq!(error.problem.expected, "shader.wgsl in the bundle");
}

#[test]
fn a_layer_filling_a_parameter_its_shader_lacks_is_refused() {
    let report = Game::new()
        .shader("tint")
        .edit("vfx_shader", "tint", |shader| {
            shader["params"] = json!([{ "name": "amount", "kind": "number", "default": 0.6, "min": 0.0, "max": 1.0 }]);
        })
        .row(
            "vfx",
            "healed",
            json!({
                "name": "Healed",
                "layers": [{
                    "type": "shader",
                    "shader": path("vfx_shader", "tint"),
                    "duration": 0.35,
                    "params": { "amount": 0.5, "glow": 1.0 },
                }],
            }),
        )
        .try_load()
        .err()
        .expect("refused");

    let error = report
        .errors
        .iter()
        .find(|error| error.file == path("vfx", "healed"))
        .unwrap_or_else(|| panic!("the layer is refused:\n{report}"));
    assert_eq!(error.problem.rule, "unknown shader parameter");
    assert!(
        matches!(&error.at, ErrorPlace::Json { pointer, .. } if pointer == "/layers/0/params/glow"),
        "{:?}",
        error.at
    );
}

#[test]
fn a_game_file_that_does_not_parse_is_refused_at_its_field() {
    let report = game()
        .quest("errand",
            json!({ "title": "Errand", "category": "Errands", "blurb": "Errand", "xp": 10, "modules": [] }),
        )
        .edit_settings(|settings| settings["initial_spawn"] = json!(path("area", "start")))
        .try_load()
        .err()
        .expect("refused");

    let error = report
        .errors
        .iter()
        .find(|error| error.file == SETTINGS_FILE)
        .unwrap_or_else(|| panic!("no error in the settings:\n{report}"));
    assert_eq!(error.problem.rule, "wrong type");
}

#[test]
fn a_games_map_reaches_rise_from_the_closest() {
    for (reaches, refused) in [
        (json!([]), Some("no reach")),
        (json!([16.0, 0.0]), Some("out of range")),
        (json!([32.0, 16.0]), Some("out of order")),
        (json!([16.0, 32.0, 64.0]), None),
    ] {
        let loaded = game()
            .edit_settings(|settings| settings["map_reaches"] = reaches.clone())
            .try_load();
        let rules: Vec<&str> = loaded
            .as_ref()
            .err()
            .map(|report| {
                report
                    .errors
                    .iter()
                    .map(|error| error.problem.rule)
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(rules, Vec::from_iter(refused), "reaches of {reaches}");
    }
}

#[test]
fn a_days_times_begin_in_the_order_of_their_hours_each_under_its_own_name() {
    let time = |name: &str, from: f32, eases: f32| json!({ "name": name, "from": from, "eases": eases, "color": [0, 0, 0], "icon": "icons/time.png" });
    for (times, refused) in [
        (json!([]), None),
        (
            json!([
                time("Dawn", 5.0, 1.0),
                time("Day", 8.0, 1.0),
                time("Night", 21.0, 1.0)
            ]),
            None,
        ),
        (json!([time("Always", 9.0, 24.0)]), None),
        (
            json!([time("Day", 8.0, 0.0), time("Noon", 8.0, 0.0)]),
            Some("same hour"),
        ),
        (
            json!([time("Night", 21.0, 0.0), time("Day", 8.0, 0.0)]),
            Some("out of order"),
        ),
        (
            json!([time("Night", 2.0, 0.0), time("Night", 21.0, 0.0)]),
            Some("duplicate name"),
        ),
        (
            json!([time("Dawn", 5.0, 4.0), time("Day", 8.0, 0.0)]),
            Some("out of range"),
        ),
        (
            json!([
                time("Day", 8.0, 0.0),
                time("Dusk", 18.0, 4.0),
                time("Night", 21.0, 0.0)
            ]),
            Some("out of range"),
        ),
        (json!([time("Late", 24.0, 0.0)]), Some("out of range")),
    ] {
        let loaded = game()
            .asset("icons/time.png", png(1, 1))
            .edit_settings(|settings| {
                settings["daylight"] =
                    json!({ "day_length": 1200.0, "start": 10.0, "times": times.clone() });
            })
            .try_load();
        let rules: Vec<&str> = loaded
            .as_ref()
            .err()
            .map(|report| {
                report
                    .errors
                    .iter()
                    .map(|error| error.problem.rule)
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(rules, Vec::from_iter(refused), "times of {times}");
    }
}

#[test]
fn what_follows_the_sky_or_is_away_by_it_reads_the_sky_alone_by_times_of_day_the_game_names() {
    let night = json!({ "type": "time_share", "time": "Night" });
    let light = |when: Value| {
        json!([{
            "type": "light", "at": { "x": 2.0, "y": 2.0 }, "color": [255, 200, 120],
            "intensity": 4.0, "radius": 3.0, "height": 2.0, "when": when,
        }])
    };
    let sound = |when: Value| {
        json!([{
            "type": "soundscape",
            "shape": { "type": "rect", "x": -0.5, "y": -0.5, "w": 8.0, "h": 8.0 },
            "reach": 4.0,
            "channels": [
                { "channel": 1, "file": "audio/owls.wav", "when": when },
                { "channel": 1, "file": "audio/larks.wav" },
            ],
        }])
    };
    let pack = |more: Value| {
        let mut spawner = json!({
            "type": "spawner",
            "count": 2,
            "spawns": [{ "type": "npc", "npc": path("npc", "wolf"), "when": night.clone() }],
        });
        spawner
            .as_object_mut()
            .expect("the spawner")
            .extend(more.as_object().expect("fields").clone());
        json!([spawner])
    };
    for (modules, refused) in [
        (light(night.clone()), None),
        (
            sound(json!({ "type": "not", "of": { "type": "during", "time": "Night" } })),
            None,
        ),
        (pack(json!({ "respawn": 30.0 })), None),
        (
            light(json!({ "type": "at_least", "a": { "type": "level" }, "b": 3 })),
            Some("reads a player"),
        ),
        (
            sound(json!({ "type": "during", "time": "Nite" })),
            Some("unknown time of day"),
        ),
        (pack(json!({})), Some("needs a respawn")),
    ] {
        let loaded = game()
            .asset("icons/time.png", png(1, 1))
            .file("audio/owls.wav", testkit::wav())
            .file("audio/larks.wav", testkit::wav())
            .npc("wolf", 20.0, json!([]))
            .edit_settings(|settings| {
                settings["daylight"] = json!({
                    "day_length": 1200.0,
                    "start": 10.0,
                    "times": [
                        { "name": "Day", "from": 6.0, "eases": 1.0, "color": [0, 0, 0], "icon": "icons/time.png" },
                        { "name": "Night", "from": 21.0, "eases": 1.0, "color": [0, 0, 0], "icon": "icons/time.png" },
                    ],
                });
            })
            .area("meadow", &Map::open(16, 16), modules.clone())
            .try_load();
        let rules: Vec<&str> = loaded
            .as_ref()
            .err()
            .map(|report| {
                report
                    .errors
                    .iter()
                    .map(|error| error.problem.rule)
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(rules, Vec::from_iter(refused), "modules of {modules}");
    }

    let notice = |sign: Value| {
        let mut notice = json!({ "text": ["Gone home."] });
        notice
            .as_object_mut()
            .expect("the notice")
            .extend(sign.as_object().expect("fields").clone());
        notice
    };
    let own_sign = json!({ "sign": path("prop", "board") });
    let talks = json!({ "type": "script", "script": path("script", "wolf_talk") });
    for (modules, sign, refused) in [
        (json!([{ "type": "absence" }]), false, None),
        (
            json!([{ "type": "absence", "spans": [{ "time": "Night" }, { "time": "Day" }] }]),
            false,
            None,
        ),
        (
            json!([{ "type": "absence", "spans": [{ "time": "Night", "notice": notice(json!({})) }] }]),
            true,
            None,
        ),
        (
            json!([talks, { "type": "absence", "spans": [{ "time": "Night", "notice": notice(own_sign.clone()) }] }]),
            false,
            None,
        ),
        (
            json!([{ "type": "absence", "spans": [{ "time": "Nite" }] }]),
            false,
            Some("unknown time of day"),
        ),
        (
            json!([{ "type": "absence", "spans": [{ "time": "Night" }, { "time": "Night" }] }]),
            false,
            Some("listed twice"),
        ),
        (
            json!([talks, { "type": "absence", "spans": [{ "time": "Night", "notice": notice(json!({})) }] }]),
            false,
            Some("no sign"),
        ),
        (
            json!([{ "type": "absence", "spans": [{ "time": "Night", "notice": notice(own_sign.clone()) }] }]),
            false,
            Some("no reach"),
        ),
    ] {
        let loaded = game()
            .asset("icons/time.png", png(1, 1))
            .prop("board", json!({ "type": "none" }))
            .npc("wolf", 20.0, modules.clone())
            .script(
                "wolf_talk",
                "npc",
                json!({
                    "interacted": { "type": "on", "event": "interacted", "verb": "talk", "reach": 2.0, "next": "growl" },
                    "growl": testkit::dialogue("Grr.", json!([])),
                }),
            )
            .edit_settings(|settings| {
                settings["daylight"] = json!({
                    "day_length": 1200.0,
                    "start": 10.0,
                    "times": [
                        { "name": "Day", "from": 6.0, "eases": 1.0, "color": [0, 0, 0], "icon": "icons/time.png" },
                        { "name": "Night", "from": 21.0, "eases": 1.0, "color": [0, 0, 0], "icon": "icons/time.png" },
                    ],
                });
                if sign {
                    settings["absence_sign"] = json!({ "prop": path("prop", "board"), "reach": 2.5 });
                }
            })
            .try_load();
        let rules: Vec<&str> = loaded
            .as_ref()
            .err()
            .map(|report| {
                report
                    .errors
                    .iter()
                    .map(|error| error.problem.rule)
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(rules, Vec::from_iter(refused), "an absence of {modules}");
    }
}

#[test]
fn a_folder_without_a_game_file_is_refused() {
    let report = Game::new()
        .remove(SETTINGS_FILE)
        .whole()
        .try_load()
        .err()
        .expect("refused");

    let error = report
        .errors
        .iter()
        .find(|error| error.file == SETTINGS_FILE)
        .unwrap_or_else(|| panic!("no error for the settings:\n{report}"));
    assert_eq!(error.problem.rule, "missing file");
}

#[test]
fn a_fixture_without_its_row_is_refused() {
    let report = Game::new().whole().try_load().err().expect("refused");

    assert_eq!(
        rules_in(&report, "fixtures/game.input.json"),
        vec![(
            "missing fixture",
            "the input the engine requires at fixtures/game.input.json"
        )]
    );
}

#[test]
fn a_file_of_its_own_of_a_type_of_fixtures_alone_is_refused() {
    let report = Game::new()
        .row("world_map", "wave_hello", json!({}))
        .whole()
        .try_load()
        .err()
        .expect("refused");

    let error = report
        .errors
        .iter()
        .find(|error| error.file == "wave_hello.world_map.json")
        .expect("the stray file is refused");
    assert_eq!(error.problem.rule, "file the engine never reads");
}

#[test]
fn a_file_that_did_not_load_is_reported_alone() {
    let dressing = json!({
        "started": { "type": "on", "event": "started", "next": "dress" },
        "dress": { "type": "set_appearance", "appearance": { "model": path("model", "giant") } },
    });
    let keeping = json!({
        "interacted": { "type": "on", "event": "interacted", "verb": "talk", "reach": 2.0, "next": "shop" },
        "shop": { "type": "open_shop", "shop": path("shop", "wares") },
    });
    let cases: [(&str, Game, &str); 6] = [
        (
            "a class tree naming a skill",
            game()
                .row("skill", "broken", json!({ "name": "Broken", "icon": 3 }))
                .edit("class", "hero", |hero| {
                    hero["tree"] = json!({
                        "broken": { "skill": path("skill", "broken"), "at": { "x": 0.0, "y": 0.0 } },
                    });
                }),
            "broken.skill.json",
        ),
        (
            "an npc naming a script",
            game()
                .row(
                    "script",
                    "broken",
                    json!({ "runs_on": "npc", "nodes": { "on": { "type": "no_such_node" } } }),
                )
                .npc(
                    "guard",
                    10.0,
                    json!([{ "type": "script", "script": path("script", "broken") }]),
                ),
            "broken.script.json",
        ),
        (
            "an npc dressed by its script in a body on a skeleton",
            game()
                .skeleton("broken")
                .edit("skeleton", "broken", |skeleton| {
                    skeleton["base"]["clips"]["sprint"] = json!(testkit::clip("run"));
                })
                .model_on("giant", "broken", 4.0, json!([]))
                .npc(
                    "guard",
                    10.0,
                    json!([{ "type": "script", "script": path("script", "dress") }]),
                )
                .script("dress", "npc", dressing),
            "broken.skeleton",
        ),
        (
            "a character starting where the settings name an area",
            game()
                .edit_settings(|settings| settings["initial_spawn"]["area"] = json!("nowhere.area"))
                .row(
                    "test_character",
                    "lost",
                    json!({
                        "name": "Lost",
                        "class": path("class", "hero"),
                        "appearance": { "model": path("model", "hero") },
                        "known_areas": [path("area", "start")],
                        "start": "initial_spawn",
                    }),
                ),
            SETTINGS_FILE,
        ),
        (
            "the engine reading a fixture",
            game()
                .engine_rows()
                .whole()
                .file(&InputFixture::Game.path(), "{ \"actions\": 3 }"),
            "fixtures/game.input.json",
        ),
        (
            "a shop only its keeper opens",
            game()
                .engine_rows()
                .whole()
                .mark("merchant")
                .row(
                    "shop",
                    "wares",
                    json!({ "title": "Wares", "mark": path("attention", "merchant") }),
                )
                .script("keeping", "npc", keeping)
                .file("broken.npc.json", "{ \"name\": 3 }"),
            "broken.npc.json",
        ),
    ];
    for (case, game, broken) in cases {
        let report = game.try_load().err().expect("refused");

        let files: Vec<&str> = report
            .errors
            .iter()
            .map(|error| error.file.as_str())
            .collect();
        assert!(
            !files.is_empty() && files.iter().all(|file| file.starts_with(broken)),
            "{case}: only {broken} is reported\n{report}"
        );
    }
}

#[test]
fn a_game_of_only_what_the_engine_requires_loads_as_a_whole_one() {
    let loaded = game().engine_rows().whole().try_load();

    if let Err(report) = loaded {
        panic!("{report}");
    }
}

fn rules_in<'a>(report: &'a LoadReport, file: &str) -> Vec<(&'a str, &'a str)> {
    report
        .errors
        .iter()
        .filter(|error| error.file == file)
        .map(|error| (error.problem.rule, error.problem.expected.as_str()))
        .collect()
}
