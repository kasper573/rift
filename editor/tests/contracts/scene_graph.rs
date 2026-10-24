use std::collections::HashSet;

use editor::app::Editor;
use editor::project::RowSelection;
use editor::shell::tree::{DropAt, Toggle, TreeDrop, TreeSource};
use editor::user::TreeView;
use editor::views::area::flags::{SceneFlags, SceneLevel};
use editor::views::area::nodes::{self, Landing, NodeFlag, SceneTree};
use editor::views::area::scene_tree::{self, SceneNarrowing, SceneSource};
use editor::views::area::{AreaSnapshot, drop_gone, placement};
use engine::core::content::{ContentFile, write};
use engine::systems::area::AreaDef;
use engine::systems::area::scene::{self, SceneNode, scene_file};
use engine::systems::registry;
use serde_json::{Value, json};
use testkit::{Game, path};

use crate::support::{self, area, folder, names, object};

fn object_json(name: &str) -> Value {
    json!({ "type": "object", "name": name, "shape": { "type": "point", "x": 1.5, "y": 1.5 } })
}

fn folder_json(label: &str, children: Vec<Value>) -> Value {
    json!({ "type": "folder", "label": label, "children": children })
}

fn flagged(node: SceneNode, flag: NodeFlag) -> SceneNode {
    let mut nodes = vec![node];
    SceneTree::new(&mut nodes).set_flag(&["/0".to_owned()], flag, true);
    nodes.remove(0)
}

#[test]
fn grouping_puts_the_rows_in_order_into_a_new_folder_where_the_first_was() {
    let mut scene = vec![object("a"), object("b"), object("c"), object("d")];

    let made = SceneTree::new(&mut scene).group(&["/2".to_owned(), "/1".to_owned()], "Storage");

    assert_eq!(made.as_deref(), Some("/1"));
    assert_eq!(names(&scene), ["a", "Storage", "b", "c", "d"]);
    assert_eq!(nodes::children_of(&scene[1]).len(), 2);

    let mut scene = vec![
        folder("Hall", vec![object("a"), object("b")]),
        folder("Altar", vec![object("c")]),
        object("d"),
    ];
    let made = SceneTree::new(&mut scene).group(
        &["/0/children/1".to_owned(), "/1/children/0".to_owned()],
        "Picked",
    );
    assert_eq!(
        made.as_deref(),
        Some("/0"),
        "rows of several folders group in the one holding them all"
    );
    assert_eq!(scene[0].label(), Some("Picked"));
    assert_eq!(names(nodes::children_of(&scene[0])), ["b", "c"]);
    assert_eq!(names(nodes::children_of(&scene[1])), ["a"]);
}

#[test]
fn ungrouping_puts_what_a_folder_held_in_its_place_in_order() {
    let mut scene = vec![
        object("a"),
        folder("Hall", vec![object("b"), object("c")]),
        object("d"),
    ];

    SceneTree::new(&mut scene).ungroup(&["/1".to_owned()]);

    assert_eq!(names(&scene), ["a", "b", "c", "d"]);
}

#[test]
fn moving_rows_into_a_folder_lands_them_where_asked() {
    let mut scene = vec![object("a"), object("b"), folder("Hall", vec![object("c")])];

    let landed = SceneTree::new(&mut scene)
        .move_nodes(&["/0".to_owned()], "/2", &Landing::First)
        .expect("the move");

    assert_eq!(landed, ["/1/children/0"]);
    assert_eq!(names(&scene), ["b", "Hall", "a", "c"]);

    SceneTree::new(&mut scene).reorder(&["/1".to_owned()], -1);
    assert_eq!(names(&scene), ["Hall", "a", "c", "b"]);
}

#[test]
fn a_folder_never_moves_into_itself() {
    let mut scene = vec![folder("Hall", vec![folder("Nave", vec![])])];
    let before = scene::write_nodes(&scene).expect("written");

    let moved =
        SceneTree::new(&mut scene).move_nodes(&["/0".to_owned()], "/0/children/0", &Landing::End);

    assert!(moved.is_none());
    assert_eq!(scene::write_nodes(&scene).expect("written"), before);
}

#[test]
fn a_folders_hide_and_lock_reach_what_it_holds_and_each_row_keeps_its_own() {
    let mut scene = vec![
        flagged(
            folder(
                "Pews",
                vec![flagged(object("a"), NodeFlag::Hidden), object("b")],
            ),
            NodeFlag::Hidden,
        ),
        flagged(folder("Guards", vec![object("c")]), NodeFlag::Locked),
        object("d"),
    ];
    let flags_of = |scene: &[SceneNode]| {
        SceneFlags::of(
            &[SceneLevel {
                under: String::new(),
                scene,
            }],
            None,
        )
    };

    let flags = flags_of(&scene);
    assert!(flags.hidden("/0/children/0") && flags.hidden("/0/children/1"));
    assert!(flags.locked("/1/children/0"));
    assert!(!flags.hidden("/2") && !flags.locked("/2"));
    assert!(!flags.keep(&["/0/children/1"]));
    assert!(flags.keep(&["/2"]));

    SceneTree::new(&mut scene).set_flag(&["/0".to_owned()], NodeFlag::Hidden, false);
    let flags = flags_of(&scene);
    assert!(
        flags.hidden("/0/children/0"),
        "a row hidden of its own stays hidden"
    );
    assert!(!flags.hidden("/0/children/1"));
}

#[test]
fn showing_only_a_row_hides_everything_else_for_the_moment() {
    let scene = vec![folder("Pews", vec![object("a")]), object("b")];

    let flags = SceneFlags::of(
        &[SceneLevel {
            under: String::new(),
            scene: &scene,
        }],
        Some("/0"),
    );

    assert!(!flags.hidden("/0"));
    assert!(!flags.hidden("/0/children/0"));
    assert!(flags.hidden("/1"));
}

#[test]
fn hiding_a_row_shows_in_the_open_view_before_the_folder_loads_again() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = open(painted_game(), &scratch);
    support::settled(&mut editor);
    editor.views.area.ensure_view(&area());
    let shown = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");
    let rug = nodes::modules(shown.def.nodes())
        .into_iter()
        .find(|module| module.node.label.as_deref() == Some("rug"))
        .expect("the rug's placement")
        .pointer;

    scene_tree::toggle(
        &mut editor,
        &area(),
        std::slice::from_ref(&rug),
        Toggle::Hide,
    );

    let hidden = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");
    assert!(hidden.flags.hidden(&rug));
    assert_ne!(hidden.picture.area.props, shown.picture.area.props);
    support::settled(&mut editor);
    let loaded = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");
    assert_eq!(loaded.picture.area.props, hidden.picture.area.props);
}

#[test]
fn grouping_and_naming_the_folder_is_one_undo_step() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = open(objects_game(), &scratch);
    let snapshot = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");
    let rows = ["/0".to_owned(), "/1".to_owned()];

    let (id, step) = scene_tree::group(&mut editor, &snapshot, &rows).expect("grouped");
    let snapshot = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");
    scene_tree::rename_grouped(&mut editor, &snapshot, &id, "Storage", &step);

    let scene = nodes::scene_of(&editor, &area());
    assert_eq!(scene[0].label(), Some("Storage"));
    assert_eq!(names(nodes::children_of(&scene[0])), ["a", "b"]);
    editor.undo();
    assert_eq!(names(&nodes::scene_of(&editor, &area())), ["a", "b", "c"]);
}

#[test]
fn a_scene_the_editor_saves_reads_as_the_engine_writes_it() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = open(objects_game(), &scratch);
    nodes::change_scene(&mut editor, &area(), |scene| {
        let mut tree = SceneTree::new(scene);
        let made = tree.group(&["/0".to_owned(), "/1".to_owned()], "Guards")?;
        tree.set_flag(&[made], NodeFlag::Locked, true);
        Some("Lock the guards".to_owned())
    });
    assert!(editor.save());

    let content = support::content(&mut editor).clone();
    let written = scratch.path().join("written");
    write(registry(), content, &written).expect("the engine writes the game");
    let file = scene_file(&area());
    let read = |root: &std::path::Path| std::fs::read_to_string(root.join(&file)).expect("a scene");
    assert_eq!(read(&scratch.path().join("game")), read(&written));
}

#[test]
fn a_new_folder_lands_in_the_insertion_folder_and_never_shares_a_label() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = open(objects_game(), &scratch);
    let snapshot = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");

    let first = scene_tree::new_folder(&mut editor, &snapshot, "").expect("a folder");
    let snapshot = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");
    let inside = scene_tree::new_folder(&mut editor, &snapshot, &first).expect("a folder");
    let snapshot = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");
    let beside = scene_tree::new_folder(&mut editor, &snapshot, "").expect("a folder");

    assert!(
        inside.starts_with(&format!("{first}/children/")),
        "{inside}"
    );
    let scene = nodes::scene_of(&editor, &area());
    let label = |id: &str| nodes::node(&scene, id).and_then(SceneNode::label);
    assert_ne!(label(&first), label(&beside));
}

#[test]
fn deleting_a_folder_takes_all_it_holds_in_one_step() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = open(objects_game(), &scratch);
    let snapshot = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");
    let (id, _) = scene_tree::group(&mut editor, &snapshot, &["/0".to_owned(), "/1".to_owned()])
        .expect("grouped");
    let snapshot = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");

    scene_tree::delete_rows(&mut editor, &snapshot, &[id]);

    assert_eq!(names(&nodes::scene_of(&editor, &area())), ["c"]);
    editor.undo();
    assert_eq!(
        names(&nodes::scene_of(&editor, &area())),
        ["New folder", "a", "b", "c"]
    );
}

#[test]
fn a_selected_folder_moves_what_it_holds_that_is_shown_and_unlocked() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let game = support::game().edit_scene("area", support::AREA, |scene| {
        let mut locked = object_json("b");
        locked["locked"] = Value::Bool(true);
        scene
            .as_array_mut()
            .expect("a scene")
            .push(folder_json("Pews", vec![object_json("a"), locked]));
    });
    let mut editor = open(game, &scratch);
    let snapshot = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");

    let moved = snapshot.selected_modules(&HashSet::from(["/0".to_owned()]));

    assert_eq!(moved, ["/0/children/0"]);
    assert!(
        snapshot
            .folder_box(&HashSet::from(["/0".to_owned()]))
            .is_some_and(|(_, label)| label == "Pews")
    );
}

#[test]
fn rows_never_drop_onto_a_placement_or_into_themselves() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let game = support::game()
        .blueprint("pew", (1, 1), json!({}), json!([]))
        .edit_scene("area", support::AREA, |scene| {
            let rows = scene.as_array_mut().expect("a scene");
            rows.push(folder_json("Hall", vec![folder_json("Nave", vec![])]));
            rows.push(json!({
                "type": "blueprint",
                "label": "pew",
                "blueprint": path("blueprint", "pew"),
                "at": { "x": 1.0, "y": 1.0 }
            }));
        });
    let mut editor = open(game, &scratch);
    let snapshot = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area is open");
    let project = editor.project.as_ref().expect("a project");
    let source = SceneSource::new(
        project,
        &snapshot,
        &editor.selection,
        SceneNarrowing {
            filter: "",
            view: &TreeView::default(),
            kept: &mut None,
        },
        false,
    );
    let into = |folder: &str| TreeDrop {
        folder: folder.to_owned(),
        at: DropAt::End,
        onto: None,
    };

    assert!(source.drop_tip(&["/1".to_owned()], &into("/0")).is_ok());
    assert!(
        source
            .drop_tip(&["/0".to_owned()], &into("/0/children/0"))
            .is_err()
    );
    let onto_a_placement = TreeDrop {
        onto: Some("/1".to_owned()),
        ..into("")
    };
    assert!(
        source
            .drop_tip(&["/0".to_owned()], &onto_a_placement)
            .is_err()
    );
}

#[test]
fn a_blueprint_that_would_hold_itself_is_refused_with_the_chain() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let game = painted_game().blueprint(
        "dais",
        (2, 2),
        json!({}),
        json!([{ "type": "blueprint", "label": "rug", "blueprint": path("blueprint", "rug"), "at": { "x": 0.0, "y": 0.0 } }]),
    );
    let editor = open(game, &scratch);
    let files = editor.project.as_ref().expect("a project");
    let rug = path("blueprint", "rug");
    let dais = path("blueprint", "dais");

    assert!(placement::holds_itself(files, &rug, &rug).is_some());
    let reason = placement::holds_itself(files, &rug, &dais).expect("refused");
    assert!(reason.contains("dais"), "{reason}");
    assert!(placement::holds_itself(files, &dais, &rug).is_none());
    assert!(placement::holds_itself(files, &area(), &dais).is_none());
}

#[test]
fn a_change_on_disk_drops_only_the_selected_nodes_it_took_away() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = open(objects_game(), &scratch);
    editor.selection = RowSelection::parts(
        AreaDef::TABLE,
        &area(),
        vec!["/1".to_owned(), "/9".to_owned()],
    );

    let said = drop_gone(&mut editor).expect("one was dropped");

    assert_eq!(editor.selection.parts, ["/1"]);
    assert!(said.contains("changed on disk"), "{said}");
    assert!(drop_gone(&mut editor).is_none());
}

/// An area of three objects.
fn objects_game() -> Game {
    support::game().edit_scene("area", support::AREA, |scene| {
        let rows = scene.as_array_mut().expect("a scene");
        rows.extend([object_json("a"), object_json("b"), object_json("c")]);
    })
}

/// An area with a rug, a blueprint of one mat prop, placed in it.
fn painted_game() -> Game {
    support::game()
        .prop("mat", json!({ "type": "box", "width": 1.0, "depth": 1.0 }))
        .blueprint(
            "rug",
            (1, 1),
            json!({}),
            json!([{ "type": "prop", "prop": path("prop", "mat"), "at": { "x": 0.0, "y": 0.0 } }]),
        )
        .edit_scene("area", support::AREA, |scene| {
            scene.as_array_mut().expect("a scene").push(json!({
                "type": "blueprint",
                "label": "rug",
                "blueprint": path("blueprint", "rug"),
                "at": { "x": 3.0, "y": 3.0 }
            }));
        })
}

fn open(game: Game, scratch: &tempfile::TempDir) -> Editor {
    let mut editor = support::open(game, scratch);
    editor.selection = RowSelection::row(AreaDef::TABLE, &area());
    editor
}
