use editor::app::Editor;
use editor::project::RowSelection;
use editor::schema::{self, SchemaShapes};
use editor::views::area::nodes::SceneTree;
use editor::views::area::{AreaAction, AreaSnapshot, modules, nodes, placement, run_action};
use engine::core::content::{Content, ContentFile};
use engine::core::math::{Pos, Rect, Size};
use engine::core::script::VariableType;
use engine::systems::area::blueprint::{self, BlueprintDef, Placement};
use engine::systems::area::scene::{ModuleNode, SceneNode};
use engine::systems::area::{self, AreaDef, AreaModule, AreaModuleKind};
use serde_json::json;
use strum::VariantArray;
use testkit::{Game, path};

use crate::support::{self, area, folder, names, object};

#[test]
fn every_kind_of_scene_module_has_a_shape_to_add_it_from() {
    let shapes = SchemaShapes::load();
    let offered: Vec<String> = shapes
        .scene_variants()
        .iter()
        .map(|variant| variant.tag.clone())
        .collect();

    for kind in AreaModuleKind::VARIANTS {
        assert!(
            offered.iter().any(|tag| tag == kind.tag()),
            "{}",
            kind.tag()
        );
    }
}

#[test]
fn a_module_placed_on_the_map_naming_no_file_lands_in_the_area_as_the_game_takes_it() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let game = support::game().file("audio/wind.wav", testkit::wav());
    let mut editor = support::open(game, &scratch);
    let unnamed = [
        AreaModuleKind::Object,
        AreaModuleKind::SafeZone,
        AreaModuleKind::Soundscape,
    ];

    for kind in unnamed {
        let reader = editor.reader().expect("a reader");
        let module =
            modules::module_template(&reader, &area(), kind, Some(Pos::new(2.0, 2.0)), None)
                .unwrap_or_else(|why| panic!("{}: {why}", kind.tag()));

        let added = nodes::add(
            &mut editor,
            &area(),
            vec![SceneNode::Module(ModuleNode::new(module))],
            kind.tag(),
        );
        support::files(&mut editor);

        let refused = editor
            .project
            .as_mut()
            .and_then(|project| project.take_refused());
        assert!(
            added && refused.is_none(),
            "{}: {:?}",
            kind.tag(),
            refused.map(|taken| taken.refusal.sentence())
        );
    }
}

#[test]
fn removing_nodes_takes_each_named_one_whatever_order_they_are_named_in() {
    let mut scene = vec![
        object("a"),
        folder("Hall", vec![object("b"), object("c")]),
        object("d"),
    ];

    let taken = SceneTree::new(&mut scene).remove(&[
        "/1/children/0".to_owned(),
        "/2".to_owned(),
        "/0".to_owned(),
    ]);

    assert_eq!(names(&taken), ["b", "d", "a"]);
    assert_eq!(names(&scene), ["Hall", "c"]);
}

#[test]
fn a_duplicate_lands_right_after_its_original_labelled_apart_from_its_siblings() {
    let mut scene = vec![
        folder(
            "Pews",
            vec![folder("Pew", Vec::new()), folder("Pew (1)", Vec::new())],
        ),
        object("door"),
    ];

    let made = SceneTree::new(&mut scene).duplicate(
        &["/0/children/0".to_owned(), "/1".to_owned()],
        SceneNode::clone,
    );

    assert_eq!(made, ["/0/children/1", "/2"]);
    assert_eq!(
        names(nodes::children_of(&scene[0])),
        ["Pew", "Pew (2)", "Pew (1)"]
    );
    assert_eq!(names(&scene[2..]), ["door"]);
}

#[test]
fn new_nodes_land_in_the_selected_folder_or_beside_the_selected_node() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = open(chapel(), &scratch);
    let door = object("door");

    nodes::select(&mut editor, &area(), vec!["/0".to_owned()]);
    assert_eq!(nodes::insertion_folder(&editor, &area()), "/0");
    nodes::select(&mut editor, &area(), vec!["/0/children/0".to_owned()]);
    assert_eq!(nodes::insertion_folder(&editor, &area()), "/0");
    nodes::select(&mut editor, &area(), Vec::new());
    assert_eq!(nodes::insertion_folder(&editor, &area()), "");

    nodes::select(&mut editor, &area(), vec!["/0/children/1".to_owned()]);
    assert!(nodes::add(&mut editor, &area(), vec![door], "Add a door"));

    let scene = nodes::scene_of(&editor, &area());
    let added = nodes::node(&scene, "/0/children/2").expect("the door");
    assert_eq!(names(std::slice::from_ref(added)), ["door"]);
    assert_eq!(editor.selection.parts, ["/0/children/2"]);
}

#[test]
fn a_change_follows_each_node_the_scene_held_to_where_it_now_sits() {
    let mut scene = vec![object("a"), folder("f", vec![object("b")])];
    let mut tree = SceneTree::new(&mut scene);
    tree.duplicate(&["/0".to_owned()], Clone::clone);
    let remap = tree.remap();

    assert_eq!(nodes::remapped(&remap, "/0").as_deref(), Some("/0"));
    assert_eq!(nodes::remapped(&remap, "/1").as_deref(), Some("/2"));
    assert_eq!(
        nodes::remapped(&remap, "/1/children/0").as_deref(),
        Some("/2/children/0")
    );
    assert_eq!(nodes::remapped(&remap, "/5"), None);
}

#[test]
fn a_new_placement_takes_its_blueprints_name_numbered_past_its_siblings() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = open(chapel(), &scratch);
    let reader = editor.reader().expect("a reader");
    let pew = SceneNode::Module(ModuleNode::new(AreaModule::Blueprint(Placement {
        blueprint: reader
            .id_of(&path("blueprint", "pew"))
            .expect("the pew blueprint"),
        at: Pos::new(5.0, 5.0),
        yaw: Default::default(),
        values: Default::default(),
    })));

    nodes::select(&mut editor, &area(), vec!["/0".to_owned()]);
    nodes::add(&mut editor, &area(), vec![pew.clone(), pew], "Add pews");

    let scene = nodes::scene_of(&editor, &area());
    let label = |pointer: &str| nodes::node(&scene, pointer).and_then(SceneNode::label);
    assert_eq!(label("/0/children/2"), Some("pew (2)"));
    assert_eq!(label("/0/children/3"), Some("pew (3)"));
}

#[test]
fn making_a_blueprint_of_a_folder_leaves_one_placement_and_the_area_as_it_was() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = open(chapel(), &scratch);
    let before = shown(support::content(&mut editor));

    nodes::select(&mut editor, &area(), vec!["/0".to_owned()]);
    let snapshot = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");
    run_action(&mut editor, &snapshot, AreaAction::MakeBlueprint);

    let made = path("blueprint", "pews");
    let scene = nodes::scene_of(&editor, &area());
    let reader = editor.reader().expect("a reader");
    let placed = nodes::placement_of(&scene[0]).expect("a placement");
    assert_eq!(scene[0].label(), Some("Pews"));
    assert_eq!(reader.path_of(placed.blueprint), Some(made.as_str()));
    assert_eq!(editor.selection.parts, ["/0"]);
    let content = support::content(&mut editor);
    let blueprint = content
        .by_path::<BlueprintDef>(&made)
        .expect("the new blueprint loads");
    let kinds: Vec<&str> = nodes::modules(&blueprint.get(content).scene)
        .iter()
        .map(|module| module.module().kind().tag())
        .collect();
    assert_eq!(kinds, ["blueprint", "blueprint"]);
    assert_eq!(shown(content), before);

    editor.undo();
    let content = support::content(&mut editor);
    assert!(content.by_path::<BlueprintDef>(&made).is_none());
    assert!(nodes::is_folder(&nodes::scene_of(&editor, &area())[0]));
}

#[test]
fn unpacking_a_placement_leaves_a_folder_of_what_it_placed_and_the_area_as_it_was() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = open(chapel(), &scratch);
    let before = shown(support::content(&mut editor));

    let snapshot = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");
    run_action(
        &mut editor,
        &snapshot,
        AreaAction::Unpack("/0/children/1".to_owned()),
    );

    let scene = nodes::scene_of(&editor, &area());
    let unpacked = nodes::node(&scene, "/0/children/1").expect("the folder");
    assert!(nodes::is_folder(unpacked));
    assert_eq!(unpacked.label(), Some("pew (1)"));
    assert!(matches!(
        nodes::children_of(unpacked),
        [
            SceneNode::Module(ModuleNode {
                module: AreaModule::Prop(_),
                ..
            }),
            SceneNode::Module(ModuleNode {
                module: AreaModule::Prop(_),
                ..
            })
        ]
    ));
    assert_eq!(shown(support::content(&mut editor)), before);
}

#[test]
fn a_placement_covers_on_the_map_what_it_places() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = open(chapel(), &scratch);

    let snapshot = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");

    assert_eq!(
        snapshot.module_bounds("/0/children/1"),
        Some(Rect::new(Pos::new(2.0, 4.0), Size::new(2.0, 1.0)))
    );
    assert_eq!(
        snapshot.hit(Pos::new(3.5, 4.5)).as_deref(),
        Some("/0/children/1")
    );
}

#[test]
fn a_variable_left_unset_takes_its_default_even_when_the_default_names_content() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let editor = open(signposted(), &scratch);
    let reader = editor.reader().expect("a reader");
    let signpost = editor
        .read::<BlueprintDef>(&path("blueprint", "signpost"))
        .and_then(Result::ok)
        .expect("the signpost reads");
    let variable = &signpost.variables["spot"];

    assert_eq!(placement::variable_default(&reader, variable), Some(spot()));
}

#[test]
fn a_blueprint_made_of_modules_with_holes_takes_their_variables_and_its_placement_fills_them() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = open(signposted(), &scratch);
    let signpost = path("blueprint", "signpost");
    let door = "/0".to_owned();
    editor.selection = RowSelection::parts(BlueprintDef::TABLE, &signpost, vec![door.clone()]);
    let snapshot = AreaSnapshot::for_bundle(&mut editor, &signpost).expect("the signpost is open");
    run_action(&mut editor, &snapshot, AreaAction::MakeBlueprint);

    let content = support::content(&mut editor);
    let made = content
        .by_path::<BlueprintDef>(&path("blueprint", "door"))
        .expect("the new blueprint loads");
    assert!(made.get(content).variables.contains_key("spot"));
    let scene = nodes::scene_of(&editor, &signpost);
    let Some(SceneNode::Module(placed)) = nodes::node(&scene, &door) else {
        panic!("a placement where the door was");
    };
    assert!(matches!(placed.module, AreaModule::Blueprint(_)));
    assert_eq!(
        blueprint::variables_of(&placed.holes).collect::<Vec<_>>(),
        ["spot"],
        "the placement fills the new blueprint's variable from the signpost's"
    );
}

fn spot() -> serde_json::Value {
    json!({ "area": area(), "x": 3.0, "y": 4.0 })
}

/// The chapel with a signpost blueprint, whose door warps to the place its `spot` variable holds.
fn signposted() -> Game {
    let warp = json!({
        "runs_on": "object",
        "variables": { "to": { "type": "place" } },
        "nodes": {
            "entered": { "type": "on", "event": "entered", "by": "characters", "next": "warp" },
            "warp": { "type": "warp", "to": { "var": "to" } }
        }
    });
    let door = json!([{
        "type": "object",
        "name": "door",
        "shape": { "type": "rect", "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0 },
        "scripts": [{ "script": path("script", "warp"), "values": { "to": { "var": "spot" } } }]
    }]);
    chapel().row("script", "warp", warp).blueprint(
        "signpost",
        (1, 1),
        json!({ "spot": { "type": "place", "default": spot() } }),
        door,
    )
}

/// A chapel: a folder of two pews placed from a blueprint of two planks side by side.
fn chapel() -> Game {
    let pew = json!([
        { "type": "prop", "prop": path("prop", "plank"), "at": { "x": 0.5, "y": 0.5 } },
        { "type": "prop", "prop": path("prop", "plank"), "at": { "x": 1.5, "y": 0.5 } }
    ]);
    support::game()
        .prop("plank", json!({ "type": "box", "width": 1.0, "depth": 1.0 }))
        .blueprint("pew", (2, 1), json!({}), pew)
        .edit_scene("area", support::AREA, |scene| {
            let pews = json!({ "type": "folder", "label": "Pews", "children": [
                { "type": "blueprint", "label": "pew", "blueprint": path("blueprint", "pew"), "at": { "x": 2.0, "y": 2.0 } },
                { "type": "blueprint", "label": "pew (1)", "blueprint": path("blueprint", "pew"), "at": { "x": 2.0, "y": 4.0 } }
            ] });
            scene.as_array_mut().expect("a scene").push(pews);
        })
}

/// What an area shows of its props, wherever in its scene each piece comes from.
fn shown(content: &Content) -> Vec<String> {
    let id = content.by_path::<AreaDef>(&area()).expect("the area");
    let mut shown: Vec<String> = area::load(content, id)
        .props
        .iter()
        .map(|prop| {
            format!(
                "{} at {:?} turned {:?}",
                prop.prop.path(content),
                prop.at,
                prop.yaw
            )
        })
        .collect();
    shown.sort();
    shown
}

fn open(game: Game, scratch: &tempfile::TempDir) -> Editor {
    let mut editor = support::open(game, scratch);
    editor.selection = RowSelection::row(AreaDef::TABLE, &area());
    editor
}

#[test]
fn every_field_of_a_module_a_variable_could_fill_is_offered_as_a_hole() {
    let door = json!({
        "type": "object",
        "name": "door",
        "title": "Chapel door",
        "shape": { "type": "rect", "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0 },
        "scripts": [{ "script": path("script", "warp") }],
    });

    let offered: Vec<(String, VariableType)> =
        schema::holeable_fields(schema::of_type::<AreaModule>(), &door)
            .into_iter()
            .map(|field| (field.pointer, field.kind))
            .collect();

    for wanted in [
        ("/name", VariableType::Name),
        ("/title", VariableType::Text),
        ("/scripts/0/script", VariableType::Row),
    ] {
        assert!(
            offered
                .iter()
                .any(|(pointer, kind)| (pointer.as_str(), *kind) == wanted),
            "{wanted:?} in {offered:?}"
        );
    }
    assert!(!offered.iter().any(|(pointer, _)| pointer == "/type"));
}
