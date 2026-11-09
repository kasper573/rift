use engine::core::content::{ContentScope, with_content};
use engine::systems::area::AreaModule;
use engine::systems::area::scene::{ModuleNode, SceneNode};

use super::ops::{Hand, Held, NodeHead, NodeMove, NodePath, SceneOp, follow_through, node_at_mut};

/// A scene's nodes being changed, every change kept as the change that takes it back, so the
/// whole edit is undone by making those in reverse.
pub struct SceneEdit<'a> {
    nodes: &'a mut Vec<SceneNode>,
    undo: Vec<SceneOp>,
    moves: Vec<NodeMove>,
    /// Each node the edit made or changed, where it was then, and how many moves were made by
    /// then.
    touched: Vec<(usize, NodePath)>,
    hand: Hand,
    next_held: u32,
}

/// What an edit to a scene did: the changes that take it back, in the order they do, how it
/// moved nodes about, and where the nodes it made or changed stand after it.
#[derive(Clone, Default)]
pub struct SceneChanges {
    pub undo: Vec<SceneOp>,
    pub moves: Vec<NodeMove>,
    pub touched: Vec<NodePath>,
}

impl SceneChanges {
    pub fn is_empty(&self) -> bool {
        self.undo.is_empty()
    }

    /// Whether the edit changed only what the editor reads of the scene: labels and flags.
    pub fn editor_only(&self) -> bool {
        super::ops::editor_only(&self.undo)
    }
}

/// The nodes as the edit has left them so far: changed only through the edit, so every change is
/// kept.
impl std::ops::Deref for SceneEdit<'_> {
    type Target = [SceneNode];

    fn deref(&self) -> &[SceneNode] {
        self.nodes
    }
}

impl<'a> SceneEdit<'a> {
    pub fn new(nodes: &'a mut Vec<SceneNode>) -> SceneEdit<'a> {
        SceneEdit {
            nodes,
            undo: Vec::new(),
            moves: Vec::new(),
            touched: Vec::new(),
            hand: Hand::default(),
            next_held: 0,
        }
    }

    pub fn nodes(&self) -> &[SceneNode] {
        self.nodes
    }

    /// The node at `pointer`.
    pub fn node(&self, pointer: &str) -> Option<&SceneNode> {
        node_at(self.nodes, &path_of(pointer)?)
    }

    /// The module at `pointer`.
    pub fn module(&self, pointer: &str) -> Option<&ModuleNode> {
        match self.node(pointer)? {
            SceneNode::Module(module) => Some(module),
            SceneNode::Folder(_) => None,
        }
    }

    /// The module at `pointer`, to change as the edit likes.
    pub fn module_mut(&mut self, pointer: &str) -> Option<&mut ModuleNode> {
        let at = path_of(pointer)?;
        let was = match node_at(self.nodes, &at)? {
            SceneNode::Module(module) => module.clone(),
            SceneNode::Folder(_) => return None,
        };
        self.undo.push(SceneOp::Replace {
            at: at.clone(),
            node: was,
        });
        self.touched.push((self.moves.len(), at.clone()));
        match node_at_mut(self.nodes, &at)? {
            SceneNode::Module(module) => Some(module),
            SceneNode::Folder(_) => None,
        }
    }

    /// Puts `node` in place of the node at `pointer`, whatever either is.
    pub fn replace(&mut self, pointer: &str, node: SceneNode) -> Option<()> {
        let at = path_of(pointer)?;
        self.remove(&at)?;
        self.insert(&at, node)
    }

    /// Changes the label and flags of the node at `pointer` as `change` does.
    pub fn change_head(&mut self, pointer: &str, change: impl FnOnce(&mut NodeHead)) -> Option<()> {
        self.change_head_at(&path_of(pointer)?, change)
    }

    pub(crate) fn change_head_at(
        &mut self,
        at: &[usize],
        change: impl FnOnce(&mut NodeHead),
    ) -> Option<()> {
        let node = node_at(self.nodes, at)?;
        let mut head = NodeHead::of(node);
        change(&mut head);
        if head == NodeHead::of(node) {
            return Some(());
        }
        let back = SceneOp::SetHead {
            at: at.to_vec(),
            head,
        }
        .apply(self.nodes, &mut self.hand)
        .ok()?;
        self.undo.push(back);
        Some(())
    }

    /// Puts `node` at `at`.
    pub(crate) fn insert(&mut self, at: &[usize], node: SceneNode) -> Option<()> {
        self.made(SceneOp::Insert {
            at: at.to_vec(),
            node,
        })?;
        self.moves.push(NodeMove::Inserted(at.to_vec()));
        self.touched.push((self.moves.len(), at.to_vec()));
        Some(())
    }

    /// Takes the node at `at` away.
    pub(crate) fn remove(&mut self, at: &[usize]) -> Option<()> {
        self.made(SceneOp::Remove { at: at.to_vec() })?;
        self.moves.push(NodeMove::Removed(at.to_vec()));
        Some(())
    }

    /// Takes the node at `at` into the hand, to be landed elsewhere.
    pub(crate) fn lift(&mut self, at: &[usize]) -> Option<Held> {
        let held = Held(self.next_held);
        self.made(SceneOp::Lift {
            at: at.to_vec(),
            held,
        })?;
        self.next_held += 1;
        self.moves.push(NodeMove::Lifted(at.to_vec(), held));
        Some(held)
    }

    /// The node the hand holds as `held`.
    pub(crate) fn held(&self, held: Held) -> Option<&SceneNode> {
        self.hand.get(held)
    }

    /// Puts the node held as `held` down at `at`.
    pub(crate) fn land(&mut self, at: &[usize], held: Held) -> Option<()> {
        self.made(SceneOp::Land {
            at: at.to_vec(),
            held,
        })?;
        self.moves.push(NodeMove::Landed(at.to_vec(), held));
        Some(())
    }

    /// How the edit moved nodes about so far.
    pub fn moves(&self) -> &[NodeMove] {
        &self.moves
    }

    /// What the edit did, its ids among the paths of `scope`. A node lifted and never landed is
    /// taken away; a module asked for to change and left as it was made no change.
    pub fn finish(self, scope: &ContentScope) -> SceneChanges {
        let SceneEdit {
            nodes,
            mut undo,
            mut moves,
            touched,
            hand,
            ..
        } = self;
        loop {
            let unchanged = match undo.last() {
                Some(SceneOp::Replace { at, node: was }) => matches!(
                    node_at(nodes, at),
                    Some(SceneNode::Module(now)) if same_module(was, now, scope)
                ),
                _ => false,
            };
            if !unchanged {
                break;
            }
            undo.pop();
        }
        for (dropped, node) in hand.into_left() {
            for back in undo.iter_mut() {
                if let SceneOp::Land { at, held } = back
                    && *held == dropped
                {
                    let at = std::mem::take(at);
                    *back = SceneOp::Insert { at, node };
                    break;
                }
            }
            for change in moves.iter_mut() {
                if let NodeMove::Lifted(at, held) = change
                    && *held == dropped
                {
                    *change = NodeMove::Removed(std::mem::take(at));
                    break;
                }
            }
        }
        undo.reverse();
        let mut touched: Vec<NodePath> = touched
            .into_iter()
            .filter_map(|(after, at)| follow_through(&moves[after..], &at))
            .collect();
        touched.sort();
        touched.dedup();
        SceneChanges {
            undo,
            moves,
            touched,
        }
    }

    /// Makes `op`, keeping the change that takes it back.
    fn made(&mut self, op: SceneOp) -> Option<()> {
        let back = op.apply(self.nodes, &mut self.hand).ok()?;
        self.undo.push(back);
        Some(())
    }
}

/// Whether `a` and `b` are the same module, labelled and flagged alike: as they write, ids among
/// the paths of `scope`.
fn same_module(a: &ModuleNode, b: &ModuleNode, scope: &ContentScope) -> bool {
    let written =
        |module: &AreaModule| with_content(scope.clone(), || serde_json::to_value(module).ok()).0;
    a.label == b.label
        && a.hidden == b.hidden
        && a.locked == b.locked
        && a.holes == b.holes
        && written(&a.module).is_some_and(|a| Some(a) == written(&b.module))
}

/// The indices a pointer to a node of a scene steps through, folder by folder.
pub fn path_of(pointer: &str) -> Option<NodePath> {
    let mut path = Vec::new();
    let mut segments = pointer.strip_prefix('/')?.split('/');
    loop {
        path.push(segments.next()?.parse().ok()?);
        match segments.next() {
            None => return Some(path),
            Some("children") => {}
            Some(_) => return None,
        }
    }
}

/// The pointer to the node at `path` in a scene file.
pub fn pointer_of(path: &[usize]) -> String {
    path.iter()
        .map(|index| format!("/{index}"))
        .collect::<Vec<_>>()
        .join("/children")
}

/// The node at `path` of `nodes`.
pub fn node_at<'a>(nodes: &'a [SceneNode], path: &[usize]) -> Option<&'a SceneNode> {
    let (&last, holder) = path.split_last()?;
    let mut siblings = nodes;
    for &index in holder {
        siblings = match siblings.get(index)? {
            SceneNode::Folder(folder) => &folder.children,
            SceneNode::Module(_) => return None,
        };
    }
    siblings.get(last)
}
