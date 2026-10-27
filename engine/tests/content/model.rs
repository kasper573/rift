use std::collections::BTreeMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_math::Quat;
use engine::core::content::{LoadReport, fields_file};
use engine::core::math::Rgb;
use engine::core::render::world::paint::{PaintLayer, bake};
use engine::systems::actor::appearance::look;
use engine::systems::actor::skeleton::{Colours, EquipmentSlot, TintName};
use engine::systems::actor::{ActorModel, Appearance, ModelDef};
use engine::systems::item::ItemDef;
use serde_json::{Value, json};
use testkit::{Declared, Game, path, png, rig, rig_clips, skin_glb};

use crate::support::game;

#[test]
fn a_part_that_does_not_fit_its_skeleton_is_refused_naming_the_joint() {
    struct Case {
        case: &'static str,
        change: fn(&mut engine::core::content::gltf::NodeTree),
        found: &'static str,
        expected: &'static str,
    }
    let cases = [
        Case {
            case: "a joint under another parent",
            change: |tree| tree.nodes[3].parent = Some(2),
            found: "mesh.glb: joint hand.l has parent hand.r",
            expected: "body.skeleton expects body",
        },
        Case {
            case: "a joint resting turned",
            change: |tree| tree.nodes[3].rest.rotation = Quat::from_rotation_z(10f32.to_radians()),
            found: "mesh.glb: joint hand.l rests 10.0° and 0.0 mm from body.skeleton",
            expected: "at most 2° and 2 mm, so library clips play true",
        },
        Case {
            case: "a joint resting moved",
            change: |tree| tree.nodes[2].rest.translation.x += 0.01,
            found: "mesh.glb: joint hand.r rests 0.0° and 10.0 mm from body.skeleton",
            expected: "at most 2° and 2 mm, so library clips play true",
        },
    ];
    let mesh = format!("{}/mesh.glb", path("model_part", "hero"));
    for case in cases {
        let mut tree = rig();
        (case.change)(&mut tree);
        let report = game()
            .file(&mesh, skin_glb(&tree, &[]))
            .try_load()
            .err()
            .unwrap_or_else(|| panic!("{}: refused", case.case));
        assert_eq!(
            problems_in(&report, &mesh),
            [(case.found.to_owned(), case.expected.to_owned())],
            "{}",
            case.case
        );
    }

    let mut fewer = rig();
    fewer.nodes.pop();
    let mut extra = rig();
    extra.nodes.push(engine::core::content::gltf::TreeNode {
        name: "tail".to_owned(),
        parent: Some(1),
        ..rig().nodes[2].clone()
    });
    for (case, tree) in [
        ("a part skinned to only some of the rig's joints", fewer),
        ("a part skinned to a joint the rig lacks", extra),
    ] {
        game()
            .file(&mesh, skin_glb(&tree, &[]))
            .try_load()
            .unwrap_or_else(|report| panic!("{case} loads: {report}"));
    }
}

#[test]
fn a_model_draws_each_part_as_cut_for_its_build() {
    let content = built(game()).load();
    let drawn = |name: &str| {
        let model = content.declared::<ModelDef>(name).expect("declared");
        let skins = ActorModel::get(&content, model).body().skins;
        skins.into_iter().map(|skin| skin.mesh).collect::<Vec<_>>()
    };
    let mesh = |part: &str, file: &str| format!("{}/{file}", path("model_part", part));

    assert_eq!(
        drawn("man"),
        [
            mesh("torso", "builds/male.glb"),
            mesh("cape", "mesh.glb"),
            mesh("beard", "builds/male.glb"),
        ],
        "a part cut for the build, one for every build, and one only the build wears"
    );
    assert_eq!(
        drawn("woman"),
        [mesh("torso", "builds/female.glb"), mesh("cape", "mesh.glb")],
        "a part with no mesh for the build is not drawn"
    );
}

#[test]
fn a_body_looks_as_its_model_its_own_parts_and_what_it_wears_laid_over_each_other() {
    struct Case {
        case: &'static str,
        own: &'static [&'static str],
        worn: &'static [(&'static str, &'static str)],
        drawn: &'static [&'static str],
    }
    let cases = [
        Case {
            case: "the model's own parts",
            own: &[],
            worn: &[],
            drawn: &["torso", "cape", "beard"],
        },
        Case {
            case: "an own part replaces the model's part over the same region",
            own: &["hair", "tunic"],
            worn: &[],
            drawn: &["cape", "beard", "hair", "tunic"],
        },
        Case {
            case: "a worn item's part replaces an own part over the same region",
            own: &["tunic"],
            worn: &[("chest", "mail")],
            drawn: &["cape", "beard", "mail"],
        },
        Case {
            case: "a part takes off whatever covers a region it hides",
            own: &["hair"],
            worn: &[("head", "helm")],
            drawn: &["torso", "cape", "beard", "helm"],
        },
        Case {
            case: "a slot the skeleton lists later lays its parts over an earlier slot's",
            own: &[],
            worn: &[("back", "cloak"), ("chest", "mail")],
            drawn: &["beard", "cloak"],
        },
        Case {
            case: "an item with no look on the skeleton changes nothing",
            own: &[],
            worn: &[("chest", "ring")],
            drawn: &["torso", "cape", "beard"],
        },
    ];
    let content = dressed(built(game())).load();
    let man = content.declared::<ModelDef>("man").expect("declared");
    for case in cases {
        let appearance = Appearance {
            model: man,
            parts: case
                .own
                .iter()
                .map(|part| content.declared(part).expect("declared"))
                .collect(),
            colours: Colours::default(),
        };
        let worn: BTreeMap<EquipmentSlot, _> = case
            .worn
            .iter()
            .map(|(slot, item)| {
                let item = content.declared::<ItemDef>(item).expect("declared");
                (EquipmentSlot::new(*slot), item)
            })
            .collect();
        let drawn: Vec<&str> = look(&content, &appearance, &worn)
            .parts
            .iter()
            .map(|laid| laid.part.name(&content))
            .collect();
        assert_eq!(drawn, case.drawn, "{}", case.case);
    }
}

#[test]
fn an_appearance_reaches_the_client_as_the_world_holds_it() {
    let content = dressed(built(game())).load();
    let man = content.declared::<ModelDef>("man").expect("declared");
    let red = Colours([(TintName::new("skin"), Rgb([200, 0, 0]))].into());
    for (parts, colours) in [
        (vec![], Colours::default()),
        (vec!["hair", "tunic"], Colours::default()),
        (vec!["hair"], red),
    ] {
        let sent = Appearance {
            model: man,
            parts: parts
                .iter()
                .map(|part| content.declared(part).expect("declared"))
                .collect(),
            colours,
        };
        let wire = postcard::to_allocvec(&sent).expect("an appearance serializes");
        let received: Appearance = postcard::from_bytes(&wire).expect("it deserializes");
        assert_eq!(received, sent, "{parts:?}");
    }
}

#[test]
fn a_body_is_painted_in_its_skeletons_colours_under_its_models_its_own_and_what_it_wears() {
    const SKIN: Rgb = Rgb([255, 200, 170]);
    const HAIR: Rgb = Rgb([90, 60, 30]);
    const METAL: Rgb = Rgb([140, 150, 150]);
    const TAN: Rgb = Rgb([180, 120, 80]);
    const RED: Rgb = Rgb([200, 0, 0]);
    const GOLD: Rgb = Rgb([220, 180, 40]);
    struct Case {
        case: &'static str,
        model: Value,
        own: Value,
        worn: Value,
        painted: &'static [(&'static str, [Rgb; 3])],
    }
    let cases = [
        Case {
            case: "the skeleton's colours where nothing says otherwise",
            model: json!({}),
            own: json!({}),
            worn: json!({}),
            painted: &[("cape", [METAL, SKIN, HAIR]), ("mail", [METAL, SKIN, HAIR])],
        },
        Case {
            case: "the model's colours over the skeleton's",
            model: json!({ "skin": TAN }),
            own: json!({}),
            worn: json!({}),
            painted: &[("cape", [METAL, TAN, HAIR]), ("mail", [METAL, TAN, HAIR])],
        },
        Case {
            case: "the appearance's colours over the model's",
            model: json!({ "skin": TAN, "hair": GOLD }),
            own: json!({ "hair": RED }),
            worn: json!({}),
            painted: &[("cape", [METAL, TAN, RED]), ("mail", [METAL, TAN, RED])],
        },
        Case {
            case: "a worn item's colours over the wearer's, on its own parts alone",
            model: json!({ "skin": TAN }),
            own: json!({ "hair": RED }),
            worn: json!({ "metal": GOLD, "skin": RED }),
            painted: &[("cape", [METAL, TAN, RED]), ("mail", [GOLD, RED, RED])],
        },
    ];
    for case in cases {
        let content = painted(dressed(built(game())))
            .edit("model", "man", |man| man["colours"] = case.model.clone())
            .edit("item", "mail", |mail| {
                mail["kind"]["looks"][0]["colours"] = case.worn.clone()
            })
            .load();
        let man = content.declared::<ModelDef>("man").expect("declared");
        let appearance = Appearance {
            model: man,
            parts: Vec::new(),
            colours: serde_json::from_value(case.own.clone()).expect("colours"),
        };
        let mail = content.declared::<ItemDef>("mail").expect("declared");
        let worn = BTreeMap::from([(EquipmentSlot::new("chest"), mail)]);
        let body = look(&content, &appearance, &worn);
        let files = ActorModel::get(&content, man).dressed(&content, &body);
        for (part, colours) in case.painted {
            let mesh = format!("{}/mesh.glb", path("model_part", part));
            let paint = files
                .skins
                .iter()
                .find(|skin| skin.mesh == mesh)
                .and_then(|skin| skin.paint.as_ref())
                .unwrap_or_else(|| panic!("{}: {part} is painted", case.case));
            let layers: Vec<(usize, Rgb)> = paint
                .layers
                .iter()
                .map(|layer| (layer.channel, layer.colour))
                .collect();
            assert_eq!(
                layers,
                [(2, colours[0]), (0, colours[1]), (1, colours[2])],
                "{}: {part}, painted in the order the skeleton lists its tints",
                case.case
            );
        }
        let beard = files
            .skins
            .iter()
            .find(|skin| skin.mesh.contains("beard"))
            .expect("the beard is drawn");
        assert_eq!(beard.paint, None, "{}: a part with no material", case.case);
    }
}

#[test]
fn a_painted_texture_lays_each_colour_as_thick_as_its_mask_is_bright() {
    let image = |pixels: &[[u8; 4]]| {
        Image::new(
            Extent3d {
                width: pixels.len() as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            pixels.concat(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::all(),
        )
    };
    let grey = [128, 128, 128, 255];
    let base = image(&[grey, grey, grey, grey]);
    let mask = image(&[
        [0, 0, 0, 255],
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [255, 255, 0, 255],
    ]);
    let layer = |channel: usize, colour: [u8; 3]| PaintLayer {
        mask: "mask.png".to_owned(),
        channel,
        colour: Rgb(colour),
    };
    let red = layer(0, [255, 0, 0]);
    let blue = layer(1, [0, 0, 255]);
    let painted = bake(&base, &[(&mask, &red), (&mask, &blue)]).expect("it bakes");
    assert_eq!(
        painted.data.expect("pixels").chunks(4).collect::<Vec<_>>(),
        [
            [128, 128, 128, 255],
            [255, 0, 0, 255],
            [0, 0, 255, 255],
            [0, 0, 255, 255],
        ],
        "the base where no mask marks it, a colour where one does, the later over the earlier"
    );

    let half = image(&[[128, 0, 0, 255]]);
    let black = image(&[[0, 0, 0, 255]]);
    let painted = bake(&black, &[(&half, &layer(0, [255, 255, 255]))]).expect("it bakes");
    let [r, g, b, _] = painted.data.expect("pixels")[..] else {
        panic!("one pixel");
    };
    assert!(
        r == g && g == b && (180..=190).contains(&r),
        "half a mask mixes halfway in linear light: {r}"
    );

    let wide = image(&[grey, grey]);
    assert!(
        bake(&wide, &[(&black, &red)]).is_none(),
        "a mask of another size paints nothing"
    );
}

#[test]
fn a_model_is_held_to_its_skeletons_parts_and_builds() {
    struct Case {
        case: &'static str,
        change: fn(Game) -> Game,
        file: String,
        rule: &'static str,
    }
    let man = path("model", "man");
    let cases = [
        Case {
            case: "a part made on another skeleton",
            change: |game| {
                game.edit("model", "man", |man| {
                    man["parts"][0] = json!(path("model_part", "hero"))
                })
            },
            file: man.clone(),
            rule: "part of another skeleton",
        },
        Case {
            case: "a build its skeleton lacks",
            change: |game| game.edit("model", "man", |man| man["build"] = json!("giant")),
            file: man.clone(),
            rule: "unknown build",
        },
        Case {
            case: "no build on a skeleton of builds",
            change: |game| {
                game.edit("model", "man", |man| {
                    man.as_object_mut().expect("fields").remove("build");
                })
            },
            file: man.clone(),
            rule: "missing build",
        },
        Case {
            case: "a part's mesh for a build its skeleton lacks",
            change: |game| {
                game.part(
                    "model_part",
                    "cape",
                    "builds/giant.glb",
                    skin_glb(&rig(), &[]),
                )
            },
            file: format!("{}/builds/giant.glb", path("model_part", "cape")),
            rule: "unknown build",
        },
        Case {
            case: "a part over a region its skeleton lacks",
            change: |game| {
                game.edit("model_part", "cape", |cape| {
                    cape["covers"] = json!(["wings"])
                })
            },
            file: fields_file(&path("model_part", "cape")),
            rule: "unknown region",
        },
        Case {
            case: "an npc's own part made on another skeleton",
            change: |game| {
                game.npc("hermit", 10.0, json!([]))
                    .edit("npc", "hermit", |hermit| {
                        hermit["appearance"] = json!({
                            "model": path("model", "man"),
                            "parts": [path("model_part", "hero")],
                        })
                    })
            },
            file: path("npc", "hermit"),
            rule: "part of another skeleton",
        },
        Case {
            case: "an item worn in a slot its look's skeleton lacks",
            change: |game| worn(game, "crown", "feet", json!([])),
            file: path("item", "crown"),
            rule: "no such slot",
        },
        Case {
            case: "an item's look of a part made on another skeleton",
            change: |game| worn(game, "crown", "chest", json!([path("model_part", "hero")])),
            file: path("item", "crown"),
            rule: "part of another skeleton",
        },
        Case {
            case: "a model's colour of a tint its skeleton lacks",
            change: |game| {
                game.edit("model", "man", |man| {
                    man["colours"] = json!({ "fur": [1, 2, 3] })
                })
            },
            file: man.clone(),
            rule: "unknown tint",
        },
        Case {
            case: "an npc's colour of a tint its skeleton lacks",
            change: |game| {
                game.npc("hermit", 10.0, json!([]))
                    .edit("npc", "hermit", |hermit| {
                        hermit["appearance"] = json!({
                            "model": path("model", "man"),
                            "colours": { "fur": [1, 2, 3] },
                        })
                    })
            },
            file: path("npc", "hermit"),
            rule: "unknown tint",
        },
        Case {
            case: "an item's look in a colour of a tint its skeleton lacks",
            change: |game| {
                game.edit("item", "mail", |mail| {
                    mail["kind"]["looks"][0]["colours"] = json!({ "fur": [1, 2, 3] })
                })
            },
            file: path("item", "mail"),
            rule: "unknown tint",
        },
        Case {
            case: "a part painted with a material made on another skeleton",
            change: |game| {
                game.edit("model_part", "hero", |hero| {
                    hero["material"] = json!(path("material", "cloth"))
                })
            },
            file: fields_file(&path("model_part", "hero")),
            rule: "material of another skeleton",
        },
        Case {
            case: "a material painting a tint its skeleton lacks",
            change: |game| {
                game.edit("material", "cloth", |cloth| {
                    cloth["masks"]["paint"]["a"] = json!("fur")
                })
            },
            file: fields_file(&path("material", "cloth")),
            rule: "unknown tint",
        },
        Case {
            case: "a material painting a tint twice",
            change: |game| {
                game.edit("material", "cloth", |cloth| {
                    cloth["masks"]["paint"]["a"] = json!("skin")
                })
            },
            file: fields_file(&path("material", "cloth")),
            rule: "tint twice",
        },
        Case {
            case: "a material's mask with no image",
            change: |game| {
                game.edit("material", "cloth", |cloth| {
                    cloth["masks"]["trim"] = json!({})
                })
            },
            file: fields_file(&path("material", "cloth")),
            rule: "missing file",
        },
        Case {
            case: "a material's image of a mask it names no channels for",
            change: |game| game.part("material", "cloth", "masks/trim.png", png(4, 4)),
            file: format!("{}/masks/trim.png", path("material", "cloth")),
            rule: "unused mask",
        },
        Case {
            case: "a material's mask not the size of its base",
            change: |game| game.part("material", "cloth", "masks/paint.png", png(8, 4)),
            file: format!("{}/masks/paint.png", path("material", "cloth")),
            rule: "wrong size",
        },
    ];
    for case in cases {
        let report = (case.change)(painted(dressed(built(game()))))
            .try_load()
            .err()
            .unwrap_or_else(|| panic!("{}: refused", case.case));
        let rules: Vec<&str> = report
            .errors
            .iter()
            .filter(|error| error.file == case.file)
            .map(|error| error.problem.rule)
            .collect();
        assert_eq!(rules, [case.rule], "{}: {report}", case.case);
    }
}

/// A skeleton of a male and a female build, and a man and a woman put together from a torso cut
/// for each, a cape for every build and a beard only the male build wears.
fn built(game: Game) -> Game {
    let mesh = || skin_glb(&rig(), &[]);
    let model = |build: &str| {
        let parts = ["torso", "cape", "beard"].map(|part| path("model_part", part));
        json!({
            "skeleton": path("skeleton", "frame"),
            "build": build,
            "hitbox": { "radius": 0.5, "height": 1.8 },
            "parts": parts,
        })
    };
    let part = |game: Game, name: &str| -> Game {
        game.row(
            "model_part",
            name,
            json!({ "skeleton": path("skeleton", "frame") }),
        )
    };
    let game = game.skeleton_of(
        "frame",
        &rig_clips(),
        json!({ "builds": ["male", "female"] }),
    );
    let game = part(game, "torso")
        .part("model_part", "torso", "builds/male.glb", mesh())
        .part("model_part", "torso", "builds/female.glb", mesh());
    let game = part(game, "cape").part("model_part", "cape", "mesh.glb", mesh());
    let game = part(game, "beard").part("model_part", "beard", "builds/male.glb", mesh());
    game.row("model", "man", model("male"))
        .row("model", "woman", model("female"))
}

/// Over [`built`]'s skeleton of regions and slots, parts to wear over the man's torso, cape and
/// beard, and items worn in each slot: a mail over the chest, a helm over the head hiding the
/// hair, a cloak over the back and the torso, and a ring with no look on the skeleton.
fn dressed(game: Game) -> Game {
    let parts = [
        ("torso", json!(["torso"]), json!([])),
        ("cape", json!(["back"]), json!([])),
        ("beard", json!(["face"]), json!([])),
        ("hair", json!(["hair"]), json!([])),
        ("tunic", json!(["torso"]), json!([])),
        ("mail", json!(["torso"]), json!([])),
        ("helm", json!(["head"]), json!(["hair"])),
        ("cloak", json!(["back", "torso"]), json!([])),
    ];
    let game = game.edit("skeleton", "frame", |frame| {
        frame["regions"] = json!(["torso", "back", "face", "hair", "head"]);
        frame["slots"] = json!(["chest", "head", "back"]);
    });
    let game = parts.into_iter().fold(game, |game, (name, covers, hides)| {
        let game = if ["torso", "cape", "beard"].contains(&name) {
            game
        } else {
            game.row(
                "model_part",
                name,
                json!({ "skeleton": path("skeleton", "frame") }),
            )
            .part("model_part", name, "mesh.glb", skin_glb(&rig(), &[]))
        };
        game.edit("model_part", name, |part| {
            part["covers"] = covers;
            part["hides"] = hides;
        })
    });
    let game = worn(game, "mail", "chest", json!([path("model_part", "mail")]));
    let game = worn(game, "helm", "head", json!([path("model_part", "helm")]));
    let game = worn(game, "cloak", "back", json!([path("model_part", "cloak")]));
    game.item(
        "ring",
        json!({ "kind": { "type": "equipment", "wear": "worn", "slot": "chest" } }),
    )
}

/// Over [`dressed`], tints on the skeleton listed metal, skin, hair, and a material painting them
/// from one mask's blue, red and green, worn by the cape and the mail but not the beard.
fn painted(game: Game) -> Game {
    let game = game
        .edit("skeleton", "frame", |frame| {
            frame["tints"] = json!([
                { "name": "metal", "colour": [140, 150, 150] },
                { "name": "skin", "colour": [255, 200, 170] },
                { "name": "hair", "colour": [90, 60, 30] },
            ])
        })
        .row(
            "material",
            "cloth",
            json!({
                "skeleton": path("skeleton", "frame"),
                "masks": { "paint": { "r": "skin", "g": "hair", "b": "metal" } },
            }),
        )
        .part("material", "cloth", "base.png", png(4, 4))
        .part("material", "cloth", "masks/paint.png", png(4, 4));
    ["cape", "mail"].into_iter().fold(game, |game, part| {
        game.edit("model_part", part, |part| {
            part["material"] = json!(path("material", "cloth"))
        })
    })
}

/// An item worn in `slot`, laying `parts` over a body of [`built`]'s skeleton.
fn worn(game: Game, name: &str, slot: &str, parts: Value) -> Game {
    game.item(
        name,
        json!({ "kind": {
            "type": "equipment",
            "wear": "worn",
            "slot": slot,
            "looks": [{ "skeleton": path("skeleton", "frame"), "parts": parts }],
        } }),
    )
}

fn problems_in(report: &LoadReport, file: &str) -> Vec<(String, String)> {
    report
        .errors
        .iter()
        .filter(|error| error.file == file)
        .map(|error| (error.problem.found.clone(), error.problem.expected.clone()))
        .collect()
}
