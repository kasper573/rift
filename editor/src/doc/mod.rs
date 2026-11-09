//! The scene of an open area or blueprint as the editor reads it: its nodes as the typed bundle
//! holds them, read once, each known by a [`NodeId`] and found at once by its pointer, with the
//! folder holding it, what it holds and how many modules lie under it, and its modules listed in
//! tree order and by kind. The views read the scene through it, so a frame asks of the nodes it
//! shows rather than walking the scene, and nothing copies the scene to look at it.

mod derived;
mod edit;
pub mod history;
mod ids;
mod index;
pub mod ops;

use std::collections::HashMap;

use engine::systems::area::AreaModuleKind;
use engine::systems::area::scene::{ModuleNode, SceneNode};

use crate::views::area::nodes::SceneBundle;

pub use derived::Derived;
pub use edit::{SceneChanges, SceneEdit, node_at, path_of, pointer_of};
pub use ids::NodeId;
pub use index::NodeGrid;

pub struct SceneDoc {
    generation: u32,
    bundle: SceneBundle,
    /// Every node, folders before what they hold, so what a folder holds follows it.
    entries: Vec<Entry>,
    top: Vec<NodeId>,
    by_pointer: HashMap<String, NodeId>,
    modules: Vec<NodeId>,
    kinds: HashMap<AreaModuleKind, Vec<NodeId>>,
}

/// A module of a document, where the scene file holds it.
#[derive(Clone, Copy)]
pub struct DocModule<'a> {
    pub id: NodeId,
    pub pointer: &'a str,
    pub node: &'a ModuleNode,
}

impl SceneDoc {
    /// The document of `bundle`'s scene.
    pub fn of(bundle: SceneBundle) -> SceneDoc {
        let mut doc = SceneDoc {
            generation: ids::next_generation(),
            bundle: bundle.clone(),
            entries: Vec::new(),
            top: Vec::new(),
            by_pointer: HashMap::new(),
            modules: Vec::new(),
            kinds: HashMap::new(),
        };
        doc.top = doc.ingest(bundle.nodes(), None, "", &[]);
        doc
    }

    /// The bundle the document reads, as its files read when it was read.
    pub fn bundle(&self) -> &SceneBundle {
        &self.bundle
    }

    /// The scene's nodes as the bundle holds them.
    pub fn nodes(&self) -> &[SceneNode] {
        self.bundle.nodes()
    }

    /// The node at `pointer` in the scene file.
    pub fn id(&self, pointer: &str) -> Option<NodeId> {
        self.by_pointer.get(pointer).copied()
    }

    pub fn node(&self, id: NodeId) -> Option<&SceneNode> {
        let entry = self.entry(id)?;
        let (last, folders) = entry.path.split_last()?;
        let mut siblings = self.nodes();
        for &index in folders {
            siblings = match siblings.get(index as usize)? {
                SceneNode::Folder(folder) => &folder.children,
                SceneNode::Module(_) => return None,
            };
        }
        siblings.get(*last as usize)
    }

    /// The node at `pointer` in the scene file.
    pub fn at(&self, pointer: &str) -> Option<&SceneNode> {
        self.node(self.id(pointer)?)
    }

    pub fn module(&self, id: NodeId) -> Option<&ModuleNode> {
        match self.node(id)? {
            SceneNode::Module(module) => Some(module),
            SceneNode::Folder(_) => None,
        }
    }

    /// Where the scene file holds `id`.
    pub fn pointer(&self, id: NodeId) -> Option<&str> {
        self.entry(id).map(|entry| entry.pointer.as_str())
    }

    /// The folder holding `id`, none at the top.
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.entry(id)?.parent
    }

    /// The nodes `folder` holds, the top of the scene for none.
    pub fn children(&self, folder: Option<NodeId>) -> &[NodeId] {
        match folder {
            None => &self.top,
            Some(id) => self.entry(id).map_or(&[], |entry| &entry.children),
        }
    }

    /// How many modules lie under `id`, however deep: none under a module.
    pub fn inside(&self, id: NodeId) -> usize {
        self.entry(id).map_or(0, |entry| entry.modules_inside)
    }

    /// Every module of the scene in tree order.
    pub fn modules(&self) -> impl Iterator<Item = DocModule<'_>> + '_ {
        self.modules.iter().filter_map(|&id| self.doc_module(id))
    }

    /// Every module of the scene of `kind`, in tree order.
    pub fn of_kind(&self, kind: AreaModuleKind) -> impl Iterator<Item = DocModule<'_>> + '_ {
        self.kinds
            .get(&kind)
            .into_iter()
            .flatten()
            .filter_map(|&id| self.doc_module(id))
    }

    /// Every module under `id`, however deep, in tree order: `id` itself when it is a module.
    pub fn modules_under(&self, id: NodeId) -> impl Iterator<Item = DocModule<'_>> + '_ {
        let range = self
            .entry(id)
            .map_or(0..0, |entry| id.index..id.index + 1 + entry.descendants);
        let first = self
            .modules
            .partition_point(|module| module.index < range.start);
        self.modules[first..]
            .iter()
            .take_while(move |module| module.index < range.end)
            .filter_map(|&id| self.doc_module(id))
    }

    /// The module at `pointer`, with where it is.
    pub fn module_at(&self, pointer: &str) -> Option<DocModule<'_>> {
        self.doc_module(self.id(pointer)?)
    }

    pub fn doc_module(&self, id: NodeId) -> Option<DocModule<'_>> {
        Some(DocModule {
            id,
            pointer: self.pointer(id)?,
            node: self.module(id)?,
        })
    }

    /// How many modules the scene holds.
    pub fn module_count(&self) -> usize {
        self.modules.len()
    }

    fn entry(&self, id: NodeId) -> Option<&Entry> {
        (id.generation == self.generation)
            .then(|| self.entries.get(id.index as usize))
            .flatten()
    }

    /// Takes in `nodes`, held by `parent` at `prefix` along `path`; the ids it gave them.
    fn ingest(
        &mut self,
        nodes: &[SceneNode],
        parent: Option<NodeId>,
        prefix: &str,
        path: &[u32],
    ) -> Vec<NodeId> {
        let mut ids = Vec::with_capacity(nodes.len());
        for (index, node) in nodes.iter().enumerate() {
            let id = NodeId {
                index: self.entries.len() as u32,
                generation: self.generation,
            };
            let pointer = format!("{prefix}/{index}");
            let mut at = path.to_vec();
            at.push(index as u32);
            self.entries.push(Entry {
                pointer: pointer.clone(),
                path: at.clone().into(),
                parent,
                children: Vec::new(),
                descendants: 0,
                modules_inside: 0,
            });
            self.by_pointer.insert(pointer.clone(), id);
            match node {
                SceneNode::Module(module) => {
                    self.modules.push(id);
                    self.kinds.entry(module.module.kind()).or_default().push(id);
                }
                SceneNode::Folder(folder) => {
                    let modules_before = self.modules.len();
                    let children = self.ingest(
                        &folder.children,
                        Some(id),
                        &format!("{pointer}/children"),
                        &at,
                    );
                    let descendants = self.entries.len() as u32 - id.index - 1;
                    let entry = &mut self.entries[id.index as usize];
                    entry.children = children;
                    entry.descendants = descendants;
                    entry.modules_inside = self.modules.len() - modules_before;
                }
            }
            ids.push(id);
        }
        ids
    }
}

struct Entry {
    pointer: String,
    /// Its index among its siblings, and those of each folder above it, from the top.
    path: Box<[u32]>,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    /// How many nodes lie under it, however deep: they follow it.
    descendants: u32,
    modules_inside: usize,
}

impl DocModule<'_> {
    pub fn module(&self) -> &engine::systems::area::AreaModule {
        &self.node.module
    }
}
