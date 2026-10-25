use engine::core::content::ErrorPlace;
use engine::core::vfx::Stage;
use serde_json::{Value, json};
use testkit::{Game, path};

use crate::support::game;

/// A game with a vfx of one `stage` shader layer, which an object's script plays `on` a `site`.
fn playing(stage: Stage, site: Value, on: Option<Value>) -> Game {
    let mut show = json!({ "type": "vfx", "vfx": path("vfx", "glow"), "site": site });
    if let Some(on) = on {
        show["on"] = on;
    }
    game()
        .shader_on("glare", stage)
        .row(
            "vfx",
            "glow",
            json!({
                "name": "Glow",
                "layers": [{ "type": "shader", "shader": path("vfx_shader", "glare"), "duration": 0.5 }],
            }),
        )
        .script(
            "shine",
            "object",
            json!({
                "entered": { "type": "on", "event": "entered", "by": "characters", "next": "show" },
                "show": show,
            }),
        )
}

fn refused_at_the_layer(game: Game, case: &str) -> bool {
    match game.try_load() {
        Ok(_) => false,
        Err(report) => {
            let stage_errors: Vec<_> = report
                .errors
                .iter()
                .filter(|error| error.problem.rule == "stage off its site")
                .collect();
            assert!(
                !stage_errors.is_empty(),
                "{case}: refused for something else:\n{report}"
            );
            assert!(
                stage_errors
                    .iter()
                    .all(|error| error.file == path("vfx", "glow")
                        && pointer(&error.at) == "/layers/0"),
                "{report}"
            );
            true
        }
    }
}

#[test]
fn a_vfx_layers_stage_fits_the_site_it_is_played_on() {
    let place = json!({ "type": "place" });
    let targets = json!({ "type": "skill_targets" });
    let cases = [
        (Stage::Ground, json!({ "type": "screen" }), None, true),
        (
            Stage::Surface,
            json!({ "type": "widget", "name": "health" }),
            None,
            true,
        ),
        (Stage::Surface, json!({ "type": "path" }), None, true),
        (Stage::Surface, place.clone(), None, false),
        (Stage::Surface, place.clone(), Some(targets.clone()), false),
        (
            Stage::Billboard,
            place.clone(),
            Some(json!({ "type": "aimed_point" })),
            false,
        ),
        (
            Stage::Surface,
            place.clone(),
            Some(json!({ "type": "aimed_point" })),
            true,
        ),
        (Stage::Text, place.clone(), Some(targets), true),
        (Stage::Screen, json!({ "type": "screen" }), None, false),
        (Stage::Ground, json!({ "type": "path" }), None, false),
        (Stage::Cover, place.clone(), None, true),
        (Stage::Cover, json!({ "type": "screen" }), None, true),
    ];
    for (stage, site, on, refused) in cases {
        let case = format!("a {} layer on {site} of {on:?}", stage.label());
        assert_eq!(
            refused_at_the_layer(playing(stage, site.clone(), on.clone()), &case),
            refused,
            "{case}"
        );
    }
}

fn pointer(at: &ErrorPlace) -> &str {
    match at {
        ErrorPlace::Json { pointer, .. } => pointer,
        _ => "",
    }
}

#[test]
fn a_sprite_layer_is_a_billboard_and_the_engines_own_vfx_play_on_actors() {
    let on_screen = game().vfx("puff").script(
        "shine",
        "object",
        json!({
            "entered": { "type": "on", "event": "entered", "by": "characters", "next": "show" },
            "show": { "type": "vfx", "vfx": path("vfx", "puff"), "site": { "type": "screen" } },
        }),
    );
    let report = on_screen.try_load().err().expect("refused");
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.problem.rule == "stage off its site"
                && error.file == path("vfx", "puff")),
        "{report}"
    );

    let screen_flash_when_hurt = game()
        .engine_rows()
        .whole()
        .shader_on("glare", Stage::Screen)
        .row(
            "vfx",
            "damage_taken",
            json!({
                "name": "Damage taken",
                "layers": [{ "type": "shader", "shader": path("vfx_shader", "glare"), "duration": 0.5 }],
            }),
        );
    let report = screen_flash_when_hurt.try_load().err().expect("refused");
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.problem.rule == "stage off its site"
                && error.file == path("vfx", "damage_taken")),
        "{report}"
    );
}
