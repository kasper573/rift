use engine::core::content::ContentScope;
use engine::systems::area::scene::SceneNode;

use super::ops::SceneOp;
use crate::project::FileStamp;

/// What a step of the history did to one scene: the changes that take it back while the step
/// stands, or that make it again once it is undone, their ids among the paths of `scope`, which
/// the files stood at as of `paths_revision`; and what the scene file held the same as before the
/// step and after it.
#[derive(Clone)]
pub struct SceneStep {
    pub bundle: String,
    pub ops: Vec<SceneOp>,
    pub scope: ContentScope,
    pub paths_revision: u64,
    pub before: FileStamp,
    pub after: FileStamp,
}

impl SceneStep {
    /// Whether `later` can join this step: its changes' ids among the same paths.
    pub fn joins(&self, later: &SceneStep) -> bool {
        self.bundle != later.bundle || self.paths_revision == later.paths_revision
    }

    /// Takes a later step of the same scene into this one, as one step: undoing it takes the
    /// later changes back first.
    pub fn fold(&mut self, later: SceneStep) {
        let mut ops = later.ops;
        ops.append(&mut self.ops);
        self.ops = ops;
        self.after = later.after;
    }

    /// About how many bytes the step holds.
    pub fn weight(&self) -> usize {
        self.ops.iter().map(op_weight).sum()
    }
}

/// About how many bytes a node takes held, by how many a scene file takes for one.
const NODE_BYTES: usize = 512;
const OP_BYTES: usize = 64;

fn op_weight(op: &SceneOp) -> usize {
    OP_BYTES
        + match op {
            SceneOp::Insert { node, .. } => nodes_in(node) * NODE_BYTES,
            SceneOp::Replace { .. } => NODE_BYTES,
            SceneOp::Remove { .. }
            | SceneOp::Lift { .. }
            | SceneOp::Land { .. }
            | SceneOp::SetHead { .. } => 0,
        }
}

fn nodes_in(node: &SceneNode) -> usize {
    match node {
        SceneNode::Folder(folder) => 1 + folder.children.iter().map(nodes_in).sum::<usize>(),
        SceneNode::Module(_) => 1,
    }
}
