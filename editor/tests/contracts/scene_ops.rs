//! A scene changes as a list of changes, each answering the change that takes it back: whatever
//! is done to a scene, undoing it in reverse leaves the scene as it was and redoing it leaves it
//! as it was made, a node followed through the changes is found where they put it, and the history
//! keeps a large scene's small edits by what they changed, not by the scene's whole file.

use crate::support::{self, area, folder, object};
use editor::doc::ops::{apply_all, follow_through};
use editor::doc::{SceneEdit, node_at, path_of, pointer_of};
use editor::views::area::nodes::{self, Landing, NodeFlag, SceneTree};
use engine::core::content::{ContentScope, with_content};
use engine::core::math::Rng;
use engine::systems::area::AreaModule;
use engine::systems::area::scene::{self, ModuleNode, SceneNode, scene_file};
use serde_json::Value;

#[test]
fn whatever_is_done_to_a_scene_undoes_and_redoes_exactly_and_follows_each_node() {
    let seed = Rng::random_seed();
    let mut rng = Rng::new(seed);
    for round in 0..ROUNDS {
        let original = random_scene(&mut rng);
        let mut scene = original.clone();
        let mut steps = Vec::new();
        for _ in 0..STEPS {
            let before = scene.clone();
            let mut edit = SceneEdit::new(&mut scene);
            random_change(&mut edit, &mut rng);
            let moves = edit.moves().to_vec();
            let changes = edit.finish(&ContentScope::new());
            nodes::walk(&before, &mut |pointer, node| {
                if nodes::is_folder(node) {
                    return;
                }
                let path = path_of(pointer).expect("a node's pointer");
                if let Some(now) = follow_through(&moves, &path) {
                    let found = node_at(&scene, &now).expect("a node where it was followed to");
                    assert_eq!(
                        identity(found),
                        identity(node),
                        "seed {seed}, round {round}: {pointer} followed to {}",
                        pointer_of(&now)
                    );
                }
            });
            steps.push((changes.undo, written(&scene)));
        }
        let made = written(&scene);
        let mut redo = Vec::new();
        for (undo, after) in steps.into_iter().rev() {
            assert_eq!(written(&scene), after, "seed {seed}, round {round}");
            redo.push(apply_all(&mut scene, undo).expect("each change undoes"));
        }
        assert_eq!(
            written(&scene),
            written(&original),
            "seed {seed}, round {round}: undone"
        );
        for ops in redo.into_iter().rev() {
            apply_all(&mut scene, ops).expect("each change redoes");
        }
        assert_eq!(written(&scene), made, "seed {seed}, round {round}: redone");
    }
}

#[test]
fn a_large_scene_keeps_a_thousand_small_edits_to_undo_and_its_file_follows_them() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut editor = support::open(support::large_game(), &scratch);
    let file = scene_file(&area());
    let at_start = support::scene(&editor, &area());

    for step in 0..EDITS {
        let pointer = format!(
            "/{}/children/{}",
            step % support::LARGE_FOLDERS,
            step % support::LARGE_PER_FOLDER
        );
        assert!(nodes::change_scene(&mut editor, &area(), |scene| {
            scene.module_mut(&pointer)?.module.shift(0.5, 0.0);
            Some(format!("Move {pointer}"))
        }));
    }
    let project = support::settled(&mut editor);
    assert_eq!(project.history.steps().len(), EDITS, "every edit is kept");
    assert!(project.unsaved().contains(&file));
    let written = project.files.get(&file).expect("the scene file");
    let read: Vec<Value> = serde_json::from_slice(&written).expect("a scene file");
    let held = support::scene(&editor, &area());
    assert_eq!(
        read.len(),
        held.len(),
        "the file holds the scene as the editor does"
    );
    assert_eq!(written_among(&editor, &held), Value::Array(read));

    for _ in 0..EDITS {
        editor.undo();
    }
    let project = support::settled(&mut editor);
    assert!(
        project.unsaved().is_empty(),
        "undone to how the disk holds it, nothing is unsaved"
    );
    assert_eq!(
        written_among(&editor, &support::scene(&editor, &area())),
        written_among(&editor, &at_start)
    );
    editor.redo();
    assert!(support::settled(&mut editor).unsaved().contains(&file));
}

const ROUNDS: usize = 40;
const STEPS: usize = 6;
const EDITS: usize = 1000;

/// A scene of a few folders, some nested, of uniquely named objects.
fn random_scene(rng: &mut Rng) -> Vec<SceneNode> {
    let mut top = Vec::new();
    for f in 0..3 + rng.rand_range(0..4) {
        if rng.rand_range(0..3) == 0 {
            top.push(object(&format!("o{f}")));
            continue;
        }
        let mut held = Vec::new();
        for inner in 0..rng.rand_range(0..5) {
            let name = format!("o{f}.{inner}");
            held.push(match rng.rand_range(0..4) {
                0 => folder(
                    &format!("f{f}.{inner}"),
                    vec![object(&format!("{name}a")), object(&format!("{name}b"))],
                ),
                _ => object(&name),
            });
        }
        top.push(folder(&format!("f{f}"), held));
    }
    top
}

/// One edit of the kinds the editor makes, on nodes picked at random.
fn random_change(edit: &mut SceneEdit, rng: &mut Rng) {
    let pointers = pointers(edit);
    let pick = |rng: &mut Rng| -> Option<String> { rng.choose(&pointers).cloned() };
    let picked: Vec<String> = (0..1 + rng.rand_range(0..3))
        .filter_map(|_| pick(rng))
        .collect();
    let Some(one) = picked.first().cloned() else {
        SceneTree::new(edit).append("", vec![object("added")]);
        return;
    };
    match rng.rand_range(0..10) {
        0 => {
            SceneTree::new(edit).remove(&picked);
        }
        1 => {
            SceneTree::new(edit).duplicate(&picked, SceneNode::clone);
        }
        2 => {
            SceneTree::new(edit).group(&distinct(picked), "group");
        }
        3 => SceneTree::new(edit).ungroup(&picked),
        4 => SceneTree::new(edit).reorder(
            &distinct(picked),
            if rng.rand_range(0..2) == 0 { -1 } else { 1 },
        ),
        5 => {
            let folder = pick(rng).filter(|f| edit.node(f).is_some_and(nodes::is_folder));
            let landing = [Landing::First, Landing::End][rng.rand_range(0..2) as usize].clone();
            SceneTree::new(edit).move_nodes(
                &distinct(picked),
                &folder.unwrap_or_default(),
                &landing,
            );
        }
        6 => SceneTree::new(edit).set_flag(&picked, NodeFlag::Hidden, rng.rand_range(0..2) == 0),
        7 => {
            edit.change_head(&one, |head| head.locked = !head.locked);
        }
        8 => {
            if let Some(module) = edit.module_mut(&one) {
                module.module.shift(1.0, -0.5);
            }
        }
        _ => {
            if edit.module(&one).is_some() {
                edit.change_head(&one, |head| head.label = Some(format!("renamed {one}")));
            }
        }
    }
}

fn pointers(nodes: &[SceneNode]) -> Vec<String> {
    let mut found = Vec::new();
    nodes::walk(nodes, &mut |pointer, _| found.push(pointer.to_owned()));
    found
}

fn distinct(mut pointers: Vec<String>) -> Vec<String> {
    pointers.sort();
    pointers.dedup();
    pointers
}

/// What tells a module apart through every change but its own taking away: its object's name.
fn identity(node: &SceneNode) -> Option<String> {
    match node {
        SceneNode::Module(ModuleNode {
            module: AreaModule::Object(object),
            ..
        }) => object.name.clone(),
        _ => None,
    }
}

/// `nodes` as a scene file holds them.
fn written(nodes: &[SceneNode]) -> Value {
    with_content(ContentScope::new(), || scene::write_nodes(nodes))
        .0
        .expect("objects of no other file write anywhere")
}

/// `nodes` as the scene file of the project open in `editor` holds them.
fn written_among(editor: &editor::app::Editor, nodes: &[SceneNode]) -> Value {
    let reader = editor.reader().expect("a project");
    nodes::CopiedNodes::of(&reader, nodes)
        .expect("the scene writes")
        .into_json()
}
