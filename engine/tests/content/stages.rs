use engine::core::content::ErrorPlace;
use engine::systems::area::stages::COVER;
use serde_json::{Value, json};
use testkit::{Game, path};

use crate::support::game;

/// A game with a vfx of one `layer`, which an object's script plays `on` a `site`.
fn playing(layer: Value, site: Value, on: Option<Value>) -> Game {
    let mut show = json!({ "type": "vfx", "vfx": path("vfx", "glow"), "site": site });
    if let Some(on) = on {
        show["on"] = on;
    }
    game()
        .shader("glare")
        .row("vfx", "glow", json!({ "name": "Glow", "layers": [layer] }))
        .script(
            "shine",
            "object",
            json!({
                "entered": { "type": "on", "event": "entered", "by": "characters", "next": "show" },
                "show": show,
            }),
        )
}

fn refused_at_the_layer(game: Game, rule: &str, file: &str, case: &str) -> bool {
    match game.try_load() {
        Ok(_) => false,
        Err(report) => {
            let errors: Vec<_> = report
                .errors
                .iter()
                .filter(|error| error.problem.rule == rule)
                .collect();
            assert!(
                !errors.is_empty(),
                "{case}: refused for something else:\n{report}"
            );
            assert!(
                errors
                    .iter()
                    .all(|error| error.file == file && pointer(&error.at).starts_with("/layers/0")),
                "{report}"
            );
            true
        }
    }
}

fn run() -> Value {
    json!({ "duration": 0.5 })
}

fn mesh(shape: Value, fit: Option<&str>) -> Value {
    let mut layer = json!({
        "type": "mesh",
        "shape": shape,
        "shader": path("vfx_shader", "glare"),
        "blend": "add",
        "run": run(),
    });
    if let Some(fit) = fit {
        layer["fit"] = json!(fit);
    }
    layer
}

fn sphere() -> Value {
    json!({ "type": "sphere", "angle": 360.0, "from": -90.0, "to": 90.0 })
}

#[test]
fn a_vfx_layer_fits_the_site_it_is_played_on_and_draws_only_with_a_shader_of_its_stage() {
    let place = json!({ "type": "place" });
    let along = json!({ "type": "path" });
    let targets = Some(json!({ "type": "skill_targets" }));
    let aimed = Some(json!({ "type": "aimed_point" }));
    let surface = json!({ "type": "surface", "run": run() });
    let coat = json!({
        "type": "coat", "shader": path("vfx_shader", "glare"), "blend": "add", "run": run(),
    });
    let fitted_ground = json!({
        "type": "ground", "shader": path("vfx_shader", "glare"), "blend": "alpha",
        "size": 2.0, "fit": true, "run": run(),
    });
    let travelling_light = json!({
        "type": "light",
        "color": { "color": [255, 255, 255, 255] },
        "intensity": { "keys": [[0.0, 1.0]] },
        "radius": 2.0,
        "place": { "travel": {} },
        "run": run(),
    });
    let tube = json!({ "type": "path_tube", "radius": 0.1, "jitter": 0.1, "rate": 10.0 });
    let cases = [
        (
            "a surface on its object",
            surface.clone(),
            place.clone(),
            None,
            false,
        ),
        (
            "a surface on the actors hit",
            surface.clone(),
            place.clone(),
            targets.clone(),
            false,
        ),
        (
            "a surface on an aimed point",
            surface.clone(),
            place.clone(),
            aimed.clone(),
            true,
        ),
        ("a surface along a path", surface, along.clone(), None, true),
        (
            "a coat on its object",
            coat.clone(),
            place.clone(),
            None,
            false,
        ),
        ("a coat along a path", coat, along.clone(), None, true),
        (
            "a shape on its object",
            mesh(sphere(), None),
            place.clone(),
            None,
            false,
        ),
        (
            "a shape along a path",
            mesh(sphere(), None),
            along.clone(),
            None,
            false,
        ),
        (
            "a fitted shape on an aimed point",
            mesh(sphere(), Some("area")),
            place.clone(),
            aimed.clone(),
            false,
        ),
        (
            "a fitted shape on the actors hit",
            mesh(sphere(), Some("area")),
            place.clone(),
            targets,
            true,
        ),
        (
            "a wrapped shape on an aimed point",
            mesh(sphere(), Some("wrap")),
            place.clone(),
            aimed.clone(),
            true,
        ),
        (
            "a path tube along a path",
            mesh(tube.clone(), None),
            along.clone(),
            None,
            false,
        ),
        (
            "a path tube on its object",
            mesh(tube, None),
            place.clone(),
            None,
            true,
        ),
        (
            "fitted ground on an aimed point",
            fitted_ground.clone(),
            place.clone(),
            aimed,
            false,
        ),
        (
            "fitted ground on its object",
            fitted_ground,
            place.clone(),
            None,
            true,
        ),
        (
            "a travelling light along a path",
            travelling_light.clone(),
            along,
            None,
            false,
        ),
        (
            "a travelling light on its object",
            travelling_light,
            place,
            None,
            true,
        ),
    ];
    for (case, layer, site, on, refused) in cases {
        let game = playing(layer, site, on);
        assert_eq!(
            refused_at_the_layer(game, "layer off its site", &path("vfx", "glow"), case),
            refused,
            "{case}"
        );
    }
    let covering =
        playing(mesh(sphere(), None), json!({ "type": "place" }), None).shader_on("glare", COVER);
    assert!(refused_at_the_layer(
        covering,
        "shader of another stage",
        &path("vfx", "glow"),
        "a shape drawn with a cover shader"
    ));
    let healed_over_an_area = game().engine_rows().whole().shader("glare").row(
        "vfx",
        "healed",
        json!({ "name": "Healed", "layers": [mesh(sphere(), Some("area"))] }),
    );
    assert!(refused_at_the_layer(
        healed_over_an_area,
        "layer off its site",
        &path("vfx", "healed"),
        "a fixture's fitted shape"
    ));
}

fn pointer(at: &ErrorPlace) -> &str {
    match at {
        ErrorPlace::Json { pointer, .. } => pointer,
        _ => "",
    }
}
