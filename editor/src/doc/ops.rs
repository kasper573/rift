//! Changes to a scene's nodes, each made by its place in the tree and each answering, as it is
//! made, the change that takes it back: undoing a step is making its changes' answers in the
//! reverse order, which answer the step again.

use std::collections::HashMap;

use engine::systems::area::scene::{ModuleNode, SceneNode};

/// Where a node stands in a scene: its index among its siblings, and those of each folder above
/// it, from the top.
pub type NodePath = Vec<usize>;

/// One change to a scene's nodes.
#[derive(Clone)]
pub enum SceneOp {
    /// Puts `node` at `at`, the nodes from there on among its siblings one further.
    Insert { at: NodePath, node: SceneNode },
    /// Takes the node at `at` away.
    Remove { at: NodePath },
    /// Takes the node at `at` into the hand as `held`, to be put down elsewhere by the same step.
    Lift { at: NodePath, held: Held },
    /// Puts the node held as `held` down at `at`.
    Land { at: NodePath, held: Held },
    /// Puts `node` in place of the module at `at`.
    Replace { at: NodePath, node: ModuleNode },
    /// Sets the label and flags of the node at `at`, keeping what a folder holds.
    SetHead { at: NodePath, head: NodeHead },
}

/// A node a step holds between lifting it and landing it elsewhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Held(pub u32);

/// What a node is called and how it is flagged, apart from what it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeHead {
    pub label: Option<String>,
    pub hidden: bool,
    pub locked: bool,
}

impl NodeHead {
    pub fn of(node: &SceneNode) -> NodeHead {
        match node {
            SceneNode::Folder(folder) => NodeHead {
                label: Some(folder.label.clone()),
                hidden: folder.hidden,
                locked: folder.locked,
            },
            SceneNode::Module(module) => NodeHead {
                label: module.label.clone(),
                hidden: module.hidden,
                locked: module.locked,
            },
        }
    }

    /// Sets `node`'s label and flags to these: a folder keeps its label when this has none.
    fn set(self, node: &mut SceneNode) {
        match node {
            SceneNode::Folder(folder) => {
                if let Some(label) = self.label {
                    folder.label = label;
                }
                folder.hidden = self.hidden;
                folder.locked = self.locked;
            }
            SceneNode::Module(module) => {
                module.label = self.label;
                module.hidden = self.hidden;
                module.locked = self.locked;
            }
        }
    }
}

/// Why a change could not be made to a scene: the place it names is not there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpMissed {
    pub at: NodePath,
}

/// The nodes a step holds between lifting and landing them.
#[derive(Default)]
pub struct Hand(HashMap<Held, SceneNode>);

impl Hand {
    pub fn get(&self, held: Held) -> Option<&SceneNode> {
        self.0.get(&held)
    }

    /// The nodes still held, which the step lifted and never landed.
    pub fn into_left(self) -> impl Iterator<Item = (Held, SceneNode)> {
        self.0.into_iter()
    }
}

impl SceneOp {
    /// Makes this change to `nodes`, holding what it lifts in `hand`; answers the change that
    /// takes it back.
    pub fn apply(self, nodes: &mut Vec<SceneNode>, hand: &mut Hand) -> Result<SceneOp, OpMissed> {
        match self {
            SceneOp::Insert { at, node } => {
                insert(nodes, &at, node)?;
                Ok(SceneOp::Remove { at })
            }
            SceneOp::Remove { at } => {
                let node = take(nodes, &at)?;
                Ok(SceneOp::Insert { at, node })
            }
            SceneOp::Lift { at, held } => {
                let node = take(nodes, &at)?;
                hand.0.insert(held, node);
                Ok(SceneOp::Land { at, held })
            }
            SceneOp::Land { at, held } => {
                let node = hand.0.remove(&held).ok_or(OpMissed { at: at.clone() })?;
                insert(nodes, &at, node)?;
                Ok(SceneOp::Lift { at, held })
            }
            SceneOp::Replace { at, node } => match node_at_mut(nodes, &at) {
                Some(SceneNode::Module(module)) => {
                    let was = std::mem::replace(module, node);
                    Ok(SceneOp::Replace { at, node: was })
                }
                _ => Err(OpMissed { at }),
            },
            SceneOp::SetHead { at, head } => {
                let node = node_at_mut(nodes, &at).ok_or(OpMissed { at: at.clone() })?;
                let was = NodeHead::of(node);
                head.set(node);
                Ok(SceneOp::SetHead { at, head: was })
            }
        }
    }
}

/// Whether `ops` change only nodes' labels and flags, which the game never reads: a scene changed
/// so builds the same game.
pub fn editor_only(ops: &[SceneOp]) -> bool {
    ops.iter().all(|op| matches!(op, SceneOp::SetHead { .. }))
}

/// Makes `ops` to `nodes` in order; answers the changes that take them back, in the order they
/// take them back. A change that can't be made stops the rest, the changes made so far taken back
/// first.
pub fn apply_all(nodes: &mut Vec<SceneNode>, ops: Vec<SceneOp>) -> Result<Vec<SceneOp>, OpMissed> {
    let mut hand = Hand::default();
    let mut undo = Vec::with_capacity(ops.len());
    for op in ops {
        match op.apply(nodes, &mut hand) {
            Ok(back) => undo.push(back),
            Err(missed) => {
                for back in undo.into_iter().rev() {
                    let _ = back.apply(nodes, &mut hand);
                }
                return Err(missed);
            }
        }
    }
    undo.reverse();
    Ok(undo)
}

/// How a change moves nodes about, apart from what it puts where: what following a node through
/// it needs.
#[derive(Clone, Debug)]
pub enum NodeMove {
    Inserted(NodePath),
    Removed(NodePath),
    Lifted(NodePath, Held),
    Landed(NodePath, Held),
}

/// Where the node standing at `path` before `moves` stands after them, none once they took it
/// away.
pub fn follow_through(moves: &[NodeMove], path: &[usize]) -> Option<NodePath> {
    let mut at = path.to_vec();
    let mut lifted: Option<(Held, NodePath)> = None;
    for change in moves {
        match (&mut lifted, change) {
            (Some((held, below)), NodeMove::Landed(landed, landing)) if held == landing => {
                at = landed.clone();
                at.append(below);
                lifted = None;
            }
            (Some(_), _) => {}
            (None, NodeMove::Inserted(put) | NodeMove::Landed(put, _)) => {
                at = shifted_by_insert(&at, put);
            }
            (None, NodeMove::Removed(taken)) => at = shifted_by_removal(&at, taken)?,
            (None, NodeMove::Lifted(taken, held)) => match shifted_by_removal(&at, taken) {
                Some(path) => at = path,
                None => lifted = Some((*held, at[taken.len()..].to_vec())),
            },
        }
    }
    lifted.is_none().then_some(at)
}

/// Where a node at `path` stands once a node is put at `at`.
fn shifted_by_insert(path: &[usize], at: &[usize]) -> NodePath {
    let mut path = path.to_vec();
    let Some(depth) = at.len().checked_sub(1) else {
        return path;
    };
    if path.len() > depth && path[..depth] == at[..depth] && path[depth] >= at[depth] {
        path[depth] += 1;
    }
    path
}

/// Where a node at `path` stands once the node at `at` is taken away: none when it is that node
/// or one it holds.
fn shifted_by_removal(path: &[usize], at: &[usize]) -> Option<NodePath> {
    if path.starts_with(at) {
        return None;
    }
    let mut path = path.to_vec();
    let Some(depth) = at.len().checked_sub(1) else {
        return Some(path);
    };
    if path.len() > depth && path[..depth] == at[..depth] && path[depth] > at[depth] {
        path[depth] -= 1;
    }
    Some(path)
}

fn insert(nodes: &mut Vec<SceneNode>, at: &[usize], node: SceneNode) -> Result<(), OpMissed> {
    let missed = || OpMissed { at: at.to_vec() };
    let (&index, holder) = at.split_last().ok_or_else(missed)?;
    let siblings = siblings_at_mut(nodes, holder).ok_or_else(missed)?;
    if index > siblings.len() {
        return Err(missed());
    }
    siblings.insert(index, node);
    Ok(())
}

fn take(nodes: &mut Vec<SceneNode>, at: &[usize]) -> Result<SceneNode, OpMissed> {
    let missed = || OpMissed { at: at.to_vec() };
    let (&index, holder) = at.split_last().ok_or_else(missed)?;
    let siblings = siblings_at_mut(nodes, holder).ok_or_else(missed)?;
    if index >= siblings.len() {
        return Err(missed());
    }
    Ok(siblings.remove(index))
}

pub(crate) fn node_at_mut<'a>(
    nodes: &'a mut [SceneNode],
    path: &[usize],
) -> Option<&'a mut SceneNode> {
    let (&first, rest) = path.split_first()?;
    let node = nodes.get_mut(first)?;
    if rest.is_empty() {
        return Some(node);
    }
    match node {
        SceneNode::Folder(folder) => node_at_mut(&mut folder.children, rest),
        SceneNode::Module(_) => None,
    }
}

pub(crate) fn siblings_at_mut<'a>(
    nodes: &'a mut Vec<SceneNode>,
    holder: &[usize],
) -> Option<&'a mut Vec<SceneNode>> {
    let Some((&first, rest)) = holder.split_first() else {
        return Some(nodes);
    };
    match nodes.get_mut(first)? {
        SceneNode::Folder(folder) => siblings_at_mut(&mut folder.children, rest),
        SceneNode::Module(_) => None,
    }
}
