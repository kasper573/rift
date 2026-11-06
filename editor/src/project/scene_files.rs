use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use engine::core::content::check::RowError;
use engine::core::content::{ContentScope, FileWriter, ReadRow, RowCheck, with_content};
use engine::systems::area::AreaDef;
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::scene::{self, SCENE_FILE, SceneNode};
use engine::systems::script::Variable;

use super::files::{FileBytes, FileStamp, ProjectFiles};
use crate::doc::ops::SceneOp;
use crate::doc::{SceneChanges, node_at, pointer_of};
use crate::views::area::nodes::SceneBundle;

/// A change an edit makes to the scene of a bundle: the bundle as it reads after it, its ids
/// among the paths of `scope`, and the changes that take it back.
#[derive(Clone)]
pub struct SceneChange {
    pub bundle: String,
    pub scene: SceneBundle,
    pub changes: SceneChanges,
    pub scope: ContentScope,
}

impl fmt::Debug for SceneChange {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("SceneChange")
            .field("bundle", &self.bundle)
            .field("changes", &self.changes.undo.len())
            .finish()
    }
}

/// The file the scene of the bundle at `bundle` is kept in.
pub(crate) fn scene_file(bundle: &str) -> String {
    format!("{bundle}/{SCENE_FILE}")
}

/// Puts `scene`, the bundle at `bundle` with its ids among `scope`'s paths, into `files` as its
/// scene file, written only once something reads it, the same as `same_as` when it is a scene
/// made again; answers what the file is the same as.
pub(crate) fn put_scene(
    files: &mut ProjectFiles,
    bundle: &str,
    scene: &SceneBundle,
    scope: &ContentScope,
    same_as: Option<FileStamp>,
) -> FileStamp {
    let (scene, scope, at) = (scene.clone(), scope.clone(), bundle.to_owned());
    let file = scene_file(bundle);
    let written = file.clone();
    files.project(
        file,
        same_as,
        Box::new(move || {
            let mut writer = FileWriter::in_memory();
            // The ids were read among these very paths, so each names a file the scope holds.
            with_content(scope.clone(), || {
                scene::write(scene.nodes(), &at, &mut writer)
            })
            .0
            .expect("a scene writes among the paths its ids were read among");
            let mut files = writer.into_files();
            FileBytes::from(files.remove(&written).unwrap_or_default())
        }),
    )
}

/// `scene` as the read of the bundle at `bundle`, as the editor keeps reads.
pub(crate) fn scene_read(bundle: &str, scene: &SceneBundle) -> ReadRow {
    let (row_type, row): (TypeId, Arc<dyn Any + Send + Sync>) = match scene {
        SceneBundle::Area(area) => (TypeId::of::<AreaDef>(), area.clone()),
        SceneBundle::Blueprint(blueprint) => (TypeId::of::<BlueprintDef>(), blueprint.clone()),
    };
    ReadRow {
        path: bundle.to_owned(),
        row_type,
        row,
    }
}

/// `ops`, their ids among the paths of `from`, with their ids among those of `to`: read as a scene
/// declaring `variables` reads them. None when a node names what `to` no longer holds.
pub(crate) fn rescoped_ops(
    ops: &[SceneOp],
    from: &ContentScope,
    to: &ContentScope,
    variables: &BTreeMap<String, Variable>,
) -> Option<Vec<SceneOp>> {
    let node = |node: &SceneNode| -> Option<SceneNode> {
        let json = with_content(from.clone(), || {
            scene::write_nodes(std::slice::from_ref(node))
        })
        .0
        .ok()?;
        with_content(to.clone(), || scene::read_nodes(&json, variables))
            .0
            .ok()?
            .into_iter()
            .next()
    };
    ops.iter()
        .map(|op| {
            Some(match op {
                SceneOp::Insert { at, node: held } => SceneOp::Insert {
                    at: at.clone(),
                    node: node(held)?,
                },
                SceneOp::Replace { at, node: held } => {
                    match node(&SceneNode::Module(held.clone()))? {
                        SceneNode::Module(module) => SceneOp::Replace {
                            at: at.clone(),
                            node: module,
                        },
                        SceneNode::Folder(_) => return None,
                    }
                }
                other => other.clone(),
            })
        })
        .collect()
}

/// What breaks the rules a module keeps alone among the nodes `change` made or changed, each at
/// its pointer in the scene file.
pub(crate) fn problems(change: &SceneChange) -> Vec<RowError> {
    let mut check = RowCheck::default();
    for path in &change.changes.touched {
        if let Some(node) = node_at(change.scene.nodes(), path) {
            check_under(node, pointer_of(path), &mut check);
        }
    }
    check.into_errors()
}

fn check_under(node: &SceneNode, pointer: String, check: &mut RowCheck) {
    match node {
        SceneNode::Module(module) => module.module.check(&pointer, check),
        SceneNode::Folder(folder) => {
            for (index, child) in folder.children.iter().enumerate() {
                check_under(child, format!("{pointer}/children/{index}"), check);
            }
        }
    }
}
