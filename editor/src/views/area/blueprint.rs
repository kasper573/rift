use std::collections::BTreeMap;
use ui::words;

use engine::core::content::locate::escape;
use engine::core::content::{Content, ContentFile, FolderReader, name_of, parent_of, rescoped};
use engine::core::math::{Pos, Rect};
use engine::core::units::Meters;
use engine::systems::area::blueprint::{BlueprintDef, Placement};
use engine::systems::area::scene::{
    self, GroundLayer, Holes, ModuleNode, Scene, SceneGrids, SceneNode,
};
use engine::systems::area::{AreaModule, MapSize, Tilemap};
use engine::systems::script::Variable;

use super::grid::GridDraft;
use super::{AreaSnapshot, nodes, placement, shapes};
use crate::app::Editor;
use crate::moves::free_path;
use crate::project::ContentEdit;
use crate::rows;
use crate::tabs::file_title;

/// Makes a blueprint of the selection at once: a new bundle beside the one being edited holds the
/// selected nodes, or what a selected folder holds, and one placement of it takes their place. The
/// blueprint is named after the folder or node, and starts with the ground layers of the scene it
/// came from, so what it paints lands in layers of the same name.
pub fn make_blueprint_from_selection(editor: &mut Editor, frame: &AreaSnapshot) {
    let bundle = frame.bundle.clone();
    let Some(own) = nodes::bundle_of(editor, &bundle) else {
        return;
    };
    let scene = own.nodes().to_vec();
    let selected = selected_nodes(editor, &bundle, &scene);
    let Some(first) = selected.first().cloned() else {
        editor.toast("Select modules or a folder to make a blueprint of");
        return;
    };
    let node = |pointer: &str| nodes::node(&scene, pointer).cloned();
    let held: Vec<SceneNode> = match selected.as_slice() {
        [only] => match node(only) {
            Some(SceneNode::Folder(folder)) => folder.children,
            only => only.into_iter().collect(),
        },
        many => many.iter().filter_map(|pointer| node(pointer)).collect(),
    };
    let reader = frame.reader.clone();
    let label = match selected.as_slice() {
        [only] => node(only)
            .map(|node| shapes::node_label(&node, reader.as_ref()))
            .unwrap_or_default(),
        _ => "blueprint".to_owned(),
    };
    let Some(bounds) = covered(&frame.content, &reader, &held) else {
        editor.toast("The selection has no place on the map");
        return;
    };
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let name = Some(rows::name_from(&label))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "blueprint".to_owned());
    let path = free_path(
        &project.files,
        parent_of(&bundle),
        &format!("{name}.{}", BlueprintDef::TABLE),
        &[],
    );
    let origin: Pos<Meters> = Pos::new(
        (bounds.min().x + 0.5).floor(),
        (bounds.min().y + 0.5).floor(),
    );
    let named = scene::holes(&held);
    let variables: BTreeMap<String, Variable> = own
        .blueprint()
        .variables
        .iter()
        .filter(|(name, _)| named.contains(name))
        .map(|(name, variable)| (name.clone(), variable.clone()))
        .collect();
    let mut shifted = held;
    for node in &mut shifted {
        shift_tree(node, -origin.x, -origin.y);
    }
    let layers = GridDraft::of(&bundle, own.blueprint())
        .layer_names()
        .into_iter()
        .map(|name| SceneNode::GroundLayer(GroundLayer::named(name)));
    let map = Tilemap {
        size: MapSize {
            width: (bounds.max().x - origin.x + 0.5).ceil().max(1.0) as u32,
            height: (bounds.max().y - origin.y + 0.5).ceil().max(1.0) as u32,
        },
        tile: own.blueprint().map.tile,
    };
    let made = BlueprintDef {
        variables,
        scene: Scene {
            nodes: layers.chain(shifted).collect(),
            grids: SceneGrids::empty(&map),
        },
        map,
    };
    let count = nodes::modules(&made.scene.nodes).len();
    let mut edit = ContentEdit::new(format!("Make blueprint {}", name_of(&path)));
    if let Err(error) = edit.write_content(project, &path, &made) {
        editor.toast(error);
        return;
    }
    let after = project.reader_after(&edit);
    let Some(written) = after.id_of::<BlueprintDef>(&path).and_then(|made| {
        let mut changed = own.blueprint().clone();
        changed.variables = rescoped(&changed.variables, reader.scope(), after.scope()).ok()?;
        changed.scene.nodes = nodes::CopiedNodes::of(&reader, &changed.scene.nodes)?
            .read(&after, &changed.variables)
            .ok()?;
        *nodes::node_mut(&mut changed.scene.nodes, &first)? = SceneNode::Module(ModuleNode {
            label: Some(label.clone()),
            holes: Holes(
                named
                    .iter()
                    .map(|name| (format!("/values/{}", escape(name)), name.clone()))
                    .collect(),
            ),
            ..ModuleNode::new(AreaModule::Blueprint(Placement {
                blueprint: made,
                at: origin,
                flip: Default::default(),
                values: BTreeMap::new(),
            }))
        });
        nodes::SceneTree::new(&mut changed.scene.nodes).remove(&selected[1..]);
        changed.scene.nodes = nodes::CopiedNodes::of(&after, &changed.scene.nodes)?
            .read(&after, &changed.variables)
            .ok()?;
        Some(changed)
    }) else {
        editor.toast("The new blueprint doesn't read where it was made");
        return;
    };
    let wrote = match own {
        nodes::SceneBundle::Area(area) => {
            let mut area = (*area).clone();
            area.blueprint = written;
            edit.write_content_in(&after, &bundle, &area).map(|_| ())
        }
        nodes::SceneBundle::Blueprint(_) => {
            edit.write_content_in(&after, &bundle, &written).map(|_| ())
        }
    };
    if let Err(error) = wrote.and_then(|()| editor.try_apply(edit)) {
        editor.toast(error);
        return;
    }
    nodes::select(editor, &bundle, vec![first]);
    editor.toast(format!(
        "Made {} holding {count} module{}; Ctrl Z takes it back",
        file_title(&path),
        words::plural(count)
    ));
}

/// The nodes of `bundle` selected, ground layers aside, in tree order and without any held by
/// another selected folder.
fn selected_nodes(editor: &Editor, bundle: &str, scene: &[SceneNode]) -> Vec<String> {
    let selected = super::selected_parts(editor, bundle);
    let mut found: Vec<String> = Vec::new();
    nodes::walk(scene, &mut |pointer, node| {
        let inside_found = found
            .iter()
            .any(|folder| pointer.starts_with(&format!("{folder}/")));
        if selected.contains(pointer) && !nodes::is_layer(node) && !inside_found {
            found.push(pointer.to_owned());
        }
    });
    found
}

/// What the modules among `nodes`, read among `reader`'s paths, cover on the map, through their
/// folders, a placement by what its blueprint holds.
pub(super) fn covered(
    content: &Content,
    reader: &FolderReader,
    nodes: &[SceneNode],
) -> Option<Rect<Meters>> {
    nodes::modules(nodes)
        .iter()
        .filter_map(|module| match module.module() {
            AreaModule::Blueprint(placement) => {
                placement::nested_bounds(content, reader, placement)
            }
            module => shapes::bounds(module, None),
        })
        .reduce(|a, b| a.union(&b))
}

pub fn shift_tree(node: &mut SceneNode, dx: f32, dy: f32) {
    match node {
        SceneNode::Folder(folder) => {
            for child in &mut folder.children {
                shift_tree(child, dx, dy);
            }
        }
        SceneNode::Module(module) => shapes::shift(&mut module.module, dx, dy),
        SceneNode::GroundLayer(_) => {}
    }
}

/// Where the modules among `nodes` start on the map: the top-left of what they cover.
pub fn selection_origin(nodes: &[SceneNode]) -> Option<Pos<Meters>> {
    super::nodes::modules(nodes)
        .iter()
        .filter_map(|module| shapes::bounds(module.module(), None))
        .reduce(|a, b| a.union(&b))
        .map(|r| r.min())
}
