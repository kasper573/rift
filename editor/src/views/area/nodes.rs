use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use engine::core::content::{
    ContentFile, ContentProblem, FolderReader, Unreadable, name_of, numbered, table_of,
    with_content,
};
use engine::systems::area::blueprint::{BlueprintDef, Placement};
use engine::systems::area::scene::{self, ModuleNode, SceneFolder, SceneNode};
use engine::systems::area::terrain::TerrainSize;
use engine::systems::area::{AreaDef, AreaModule};
use engine::systems::script::Variable;

use crate::app::Editor;
use crate::project::{ContentEdit, Project, RowSelection};

/// An area or a blueprint as the files hold it now, read as the game reads it: a scene graph on a
/// stretch of ground.
#[derive(Clone)]
pub enum SceneBundle {
    Area(Arc<AreaDef>),
    Blueprint(Arc<BlueprintDef>),
}

/// A bundle's scene as an edit changes it: its nodes, and the variables a blueprint's holes name.
/// An area has no variables; an edit that gives it some is refused.
#[derive(Clone, Default)]
pub struct SceneDraft {
    pub nodes: Vec<SceneNode>,
    pub variables: BTreeMap<String, Variable>,
}

impl SceneBundle {
    pub fn nodes(&self) -> &[SceneNode] {
        match self {
            SceneBundle::Area(area) => &area.scene,
            SceneBundle::Blueprint(blueprint) => &blueprint.scene,
        }
    }

    pub fn variables(&self) -> &BTreeMap<String, Variable> {
        static NONE: BTreeMap<String, Variable> = BTreeMap::new();
        match self {
            SceneBundle::Area(_) => &NONE,
            SceneBundle::Blueprint(blueprint) => &blueprint.variables,
        }
    }

    /// The ground the scene stands on, in whole metres.
    pub fn size(&self) -> TerrainSize {
        match self {
            SceneBundle::Area(area) => area.terrain.size,
            SceneBundle::Blueprint(blueprint) => blueprint.size,
        }
    }

    pub fn draft(&self) -> SceneDraft {
        SceneDraft {
            nodes: self.nodes().to_vec(),
            variables: self.variables().clone(),
        }
    }

    /// Writes this bundle at `path` with its scene changed by `change`, as part of `edit`.
    pub fn write(
        &self,
        edit: &mut ContentEdit,
        project: &Project,
        path: &str,
        change: impl FnOnce(&mut SceneDraft),
    ) -> Result<(), String> {
        self.write_in(edit, &project.reader(), path, change)
    }

    /// Writes the bundle changed by `change`, its ids those of `reader`'s paths, so it may name
    /// files the same edit makes.
    pub fn write_in(
        &self,
        edit: &mut ContentEdit,
        reader: &FolderReader,
        path: &str,
        change: impl FnOnce(&mut SceneDraft),
    ) -> Result<(), String> {
        let mut draft = self.draft();
        change(&mut draft);
        match self {
            SceneBundle::Area(area) => {
                if !draft.variables.is_empty() {
                    return Err("an area has no variables to fill holes with".to_owned());
                }
                let mut area = (**area).clone();
                area.scene = draft.nodes;
                edit.write_content_in(reader, path, &area)?;
            }
            SceneBundle::Blueprint(blueprint) => {
                let mut blueprint = (**blueprint).clone();
                blueprint.scene = draft.nodes;
                blueprint.variables = draft.variables;
                edit.write_content_in(reader, path, &blueprint)?;
            }
        }
        Ok(())
    }
}

/// The area or blueprint at `bundle` as its files read now; `None` when there is no project or no
/// such bundle.
pub fn read_bundle(editor: &Editor, bundle: &str) -> Option<Result<SceneBundle, Arc<Unreadable>>> {
    read_bundle_in(editor.project.as_ref()?, bundle)
}

/// The area or blueprint at `bundle` of `project` as its files read now; `None` when it names no
/// area or blueprint.
pub fn read_bundle_in(
    project: &Project,
    bundle: &str,
) -> Option<Result<SceneBundle, Arc<Unreadable>>> {
    Some(match table_of(bundle)? {
        AreaDef::TABLE => project.read::<AreaDef>(bundle).map(SceneBundle::Area),
        BlueprintDef::TABLE => project
            .read::<BlueprintDef>(bundle)
            .map(SceneBundle::Blueprint),
        _ => return None,
    })
}

/// The area or blueprint at `bundle`, when it reads.
pub fn bundle_of(editor: &Editor, bundle: &str) -> Option<SceneBundle> {
    read_bundle(editor, bundle)?.ok()
}

/// The area or blueprint at `bundle` of `project`, when it reads.
pub fn bundle_in(project: &Project, bundle: &str) -> Option<SceneBundle> {
    read_bundle_in(project, bundle)?.ok()
}

/// The scene graph of the area or blueprint at `bundle` as the files hold it now: empty when it
/// doesn't read.
pub fn scene_of(editor: &Editor, bundle: &str) -> Vec<SceneNode> {
    bundle_of(editor, bundle).map_or_else(Vec::new, |bundle| bundle.nodes().to_vec())
}

/// Nodes copied apart from their scene, kept as a scene file holds them, so they read again among
/// whatever files there are when they are pasted.
#[derive(Clone)]
pub struct CopiedNodes(serde_json::Value);

impl CopiedNodes {
    pub fn of(reader: &FolderReader, nodes: &[SceneNode]) -> Option<CopiedNodes> {
        with_content(reader.scope().clone(), || scene::write_nodes(nodes))
            .0
            .ok()
            .map(CopiedNodes)
    }

    /// The nodes among `reader`'s paths, read as a scene declaring `variables` reads them.
    pub fn read(
        &self,
        reader: &FolderReader,
        variables: &BTreeMap<String, Variable>,
    ) -> Result<Vec<SceneNode>, Vec<(String, ContentProblem)>> {
        with_content(reader.scope().clone(), || {
            scene::read_nodes(&self.0, variables)
        })
        .0
    }

    /// The nodes as a scene file holds them.
    pub fn into_json(self) -> serde_json::Value {
        self.0
    }

    /// The nodes as the text a scene file holds them in.
    pub fn text(&self) -> String {
        serde_json::to_string_pretty(&self.0).unwrap_or_default()
    }
}

/// A module node of a scene and its JSON pointer in the scene file.
#[derive(Clone)]
pub struct SceneModule {
    pub pointer: String,
    pub node: ModuleNode,
}

impl SceneModule {
    pub fn module(&self) -> &AreaModule {
        &self.node.module
    }
}

/// The tag a node is badged and filtered by: its module's kind, or a folder's.
pub fn kind_tag(node: &SceneNode) -> &'static str {
    match node {
        SceneNode::Folder(_) => FOLDER,
        SceneNode::Module(module) => module.module.kind().tag(),
    }
}

/// The placement a node is, when it is one.
pub fn placement_of(node: &SceneNode) -> Option<&Placement> {
    match node {
        SceneNode::Module(ModuleNode {
            module: AreaModule::Blueprint(placement),
            ..
        }) => Some(placement),
        _ => None,
    }
}

/// The nodes a folder holds.
pub fn children_of(node: &SceneNode) -> &[SceneNode] {
    match node {
        SceneNode::Folder(folder) => &folder.children,
        _ => &[],
    }
}

pub const FOLDER: &str = "folder";

/// Every module node of `nodes` in tree order, through its folders.
/// A bundle's own modules in tree order, each also found at once by its pointer.
pub struct SceneModules {
    listed: Vec<SceneModule>,
    at: HashMap<String, usize>,
}

impl SceneModules {
    pub fn of(nodes: &[SceneNode]) -> SceneModules {
        let listed = modules(nodes);
        let at = listed
            .iter()
            .enumerate()
            .map(|(index, module)| (module.pointer.clone(), index))
            .collect();
        SceneModules { listed, at }
    }

    pub fn iter(&self) -> std::slice::Iter<'_, SceneModule> {
        self.listed.iter()
    }

    pub fn get(&self, pointer: &str) -> Option<&SceneModule> {
        self.at.get(pointer).map(|index| &self.listed[*index])
    }
}

pub fn modules(nodes: &[SceneNode]) -> Vec<SceneModule> {
    let mut found = Vec::new();
    walk(nodes, &mut |pointer, node| {
        if let SceneNode::Module(node) = node {
            found.push(SceneModule {
                pointer: pointer.to_owned(),
                node: node.clone(),
            });
        }
    });
    found
}

/// Every node of `nodes` in tree order with its pointer, folders before what they hold.
pub fn walk<'a>(nodes: &'a [SceneNode], visit: &mut impl FnMut(&str, &'a SceneNode)) {
    engine::systems::area::scene::walk(nodes, &mut |pointer, node, _| visit(pointer, node));
}

pub fn is_folder(node: &SceneNode) -> bool {
    matches!(node, SceneNode::Folder(_))
}

/// The node of `nodes` at `pointer`.
pub fn node<'a>(nodes: &'a [SceneNode], pointer: &str) -> Option<&'a SceneNode> {
    let path = path_of(pointer)?;
    let (last, folders) = path.split_last()?;
    let mut siblings = nodes;
    for &index in folders {
        siblings = match siblings.get(index)? {
            SceneNode::Folder(folder) => &folder.children,
            _ => return None,
        };
    }
    siblings.get(*last)
}

pub fn node_mut<'a>(nodes: &'a mut [SceneNode], pointer: &str) -> Option<&'a mut SceneNode> {
    let path = path_of(pointer)?;
    node_at_mut(nodes, &path)
}

/// The module node of `nodes` at `pointer`.
pub fn module_node<'a>(nodes: &'a [SceneNode], pointer: &str) -> Option<&'a ModuleNode> {
    match node(nodes, pointer)? {
        SceneNode::Module(node) => Some(node),
        _ => None,
    }
}

pub fn module_node_mut<'a>(
    nodes: &'a mut [SceneNode],
    pointer: &str,
) -> Option<&'a mut ModuleNode> {
    match node_mut(nodes, pointer)? {
        SceneNode::Module(node) => Some(node),
        _ => None,
    }
}

/// The folder holding the node at `pointer`, `""` for the top level, and the node's index in it.
pub fn place_of(pointer: &str) -> Option<(&str, usize)> {
    let cut = pointer.rfind('/')?;
    let index = pointer[cut + 1..].parse().ok()?;
    let folder = pointer[..cut].strip_suffix("/children").unwrap_or("");
    Some((folder, index))
}

/// The nodes of `folder`, `""` for the top level.
pub fn children_mut<'a>(
    nodes: &'a mut Vec<SceneNode>,
    folder: &str,
) -> Option<&'a mut Vec<SceneNode>> {
    if folder.is_empty() {
        return Some(nodes);
    }
    match node_mut(nodes, folder)? {
        SceneNode::Folder(folder) => Some(&mut folder.children),
        _ => None,
    }
}

pub fn label_of(node: &SceneNode) -> Option<&str> {
    node.label()
}

pub fn set_label(node: &mut SceneNode, label: String) {
    match node {
        SceneNode::Folder(folder) => folder.label = label,
        SceneNode::Module(module) => module.label = Some(label),
    }
}

/// A label for a new node among `siblings`: `base`, or `base (1)`, `base (2)`, the next free
/// number, when a sibling has it already.
pub fn free_label(base: &str, siblings: &[SceneNode]) -> String {
    let taken = |label: &str| {
        siblings
            .iter()
            .filter_map(SceneNode::label)
            .any(|known| known.eq_ignore_ascii_case(label))
    };
    let base = numbered(base).map_or(base, |(base, _)| base);
    if !taken(base) {
        return base.to_owned();
    }
    (1..)
        .map(|n| format!("{base} ({n})"))
        .find(|label| !taken(label))
        .expect("a free number")
}

/// A folder labelled `label`, holding nothing.
pub fn folder(label: String) -> SceneNode {
    SceneNode::Folder(SceneFolder {
        label,
        hidden: false,
        locked: false,
        children: Vec::new(),
    })
}

/// Labels each placement among `new` that has none with its blueprint's name, numbered past what
/// `folder` of `nodes` holds, as a new placement is labelled.
pub fn label_placements(
    nodes: &[SceneNode],
    folder: &str,
    new: &mut [SceneNode],
    reader: &FolderReader,
) {
    let mut siblings: Vec<SceneNode> = match folder {
        "" => nodes.to_vec(),
        folder => match node(nodes, folder) {
            Some(SceneNode::Folder(folder)) => folder.children.clone(),
            _ => Vec::new(),
        },
    };
    for node in new {
        let SceneNode::Module(module) = node else {
            continue;
        };
        let AreaModule::Blueprint(placement) = &module.module else {
            continue;
        };
        if module.label.is_some() {
            continue;
        }
        let name = name_of(reader.path_of(placement.blueprint).unwrap_or_default()).to_owned();
        module.label = Some(free_label(&name, &siblings));
        siblings.push(node.clone());
    }
}

/// Where new nodes of `bundle` go: the selected folder, or the folder holding the selected node,
/// and otherwise the top level.
pub fn insertion_folder(editor: &Editor, bundle: &str) -> String {
    let selection = &editor.selection;
    if selection.keys.first().map(String::as_str) != Some(bundle) || selection.parts.len() != 1 {
        return String::new();
    }
    let pointer = &selection.parts[0];
    let nodes = scene_of(editor, bundle);
    match node(&nodes, pointer) {
        Some(node) if is_folder(node) => pointer.clone(),
        Some(_) => place_of(pointer).map_or_else(String::new, |(folder, _)| folder.to_owned()),
        None => String::new(),
    }
}

/// An edit of `bundle`'s scene: `change` changes its nodes and names the step, or answers `None` to
/// make no step.
pub fn scene_edit(
    editor: &Editor,
    bundle: &str,
    change: impl FnOnce(&mut Vec<SceneNode>) -> Option<String>,
) -> Option<ContentEdit> {
    draft_edit(editor, bundle, |draft| change(&mut draft.nodes))
}

/// An edit of the scene of `bundle` and the variables its holes name: `change` changes them and
/// names the step, or answers `None` to make no step.
pub fn draft_edit(
    editor: &Editor,
    bundle: &str,
    change: impl FnOnce(&mut SceneDraft) -> Option<String>,
) -> Option<ContentEdit> {
    let project = editor.project.as_ref()?;
    let read = bundle_of(editor, bundle)?;
    let mut label = None;
    let mut edit = ContentEdit::new(String::new());
    read.write(&mut edit, project, bundle, |draft| {
        label = change(draft);
    })
    .ok()?;
    edit.label = label?;
    Some(edit)
}

pub fn change_scene(
    editor: &mut Editor,
    bundle: &str,
    change: impl FnOnce(&mut Vec<SceneNode>) -> Option<String>,
) -> bool {
    scene_edit(editor, bundle, change).is_some_and(|edit| editor.apply(edit))
}

pub fn change_draft(
    editor: &mut Editor,
    bundle: &str,
    change: impl FnOnce(&mut SceneDraft) -> Option<String>,
) -> bool {
    draft_edit(editor, bundle, change).is_some_and(|edit| editor.apply(edit))
}

/// Adds `new` to the insertion folder of `bundle` as one step, and selects what it added.
pub fn add(editor: &mut Editor, bundle: &str, new: Vec<SceneNode>, label: &str) -> bool {
    let Some(reader) = editor.reader() else {
        return false;
    };
    let folder = insertion_folder(editor, bundle);
    let mut added = Vec::new();
    let applied = change_scene(editor, bundle, |nodes| {
        let mut new = new;
        label_placements(nodes, &folder, &mut new, &reader);
        added = SceneTree::new(nodes).append(&folder, new);
        Some(label.to_owned())
    });
    if applied {
        select(editor, bundle, added);
    }
    applied
}

pub fn select(editor: &mut Editor, bundle: &str, pointers: Vec<String>) {
    editor.selection =
        RowSelection::parts(table_of(bundle).unwrap_or(AreaDef::TABLE), bundle, pointers);
}

/// Where moved nodes land among what a folder holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Landing {
    First,
    End,
    Before(String),
    After(String),
}

/// Where each node of a scene went in a change to it, by its pointer before and after.
pub type Remap = HashMap<String, String>;

/// Where `pointer` names a node after a change, following the nearest node above it that moved.
pub fn remapped(remap: &Remap, pointer: &str) -> Option<String> {
    if let Some(now) = remap.get(pointer) {
        return Some(now.clone());
    }
    let (folder, index) = place_of(pointer)?;
    if folder.is_empty() {
        return None;
    }
    let folder = remapped(remap, folder)?;
    Some(format!("{folder}/children/{index}"))
}

/// Which flag of a node a change sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeFlag {
    Hidden,
    Locked,
}

impl NodeFlag {
    pub fn of(self, node: &SceneNode) -> bool {
        match self {
            NodeFlag::Hidden => node.hidden(),
            NodeFlag::Locked => node.locked(),
        }
    }

    fn set(self, node: &mut SceneNode, on: bool) {
        let flag = match (node, self) {
            (SceneNode::Folder(folder), NodeFlag::Hidden) => &mut folder.hidden,
            (SceneNode::Folder(folder), NodeFlag::Locked) => &mut folder.locked,
            (SceneNode::Module(module), NodeFlag::Hidden) => &mut module.hidden,
            (SceneNode::Module(module), NodeFlag::Locked) => &mut module.locked,
        };
        *flag = on;
    }
}

/// A scene's nodes being changed, following where each node it held, and each node added, goes.
pub struct SceneTree<'a> {
    nodes: &'a mut Vec<SceneNode>,
    followed: Vec<Option<Vec<usize>>>,
    /// Each followed node's pointer before the change; `None` for one the change added.
    was: Vec<Option<String>>,
}

/// A node a `SceneTree` follows.
#[derive(Clone, Copy)]
struct Followed(usize);

/// A node taken out to be put back elsewhere, with what follows it and where below it.
struct Lifted {
    node: SceneNode,
    followers: Vec<(usize, Vec<usize>)>,
}

impl<'a> SceneTree<'a> {
    pub fn new(nodes: &'a mut Vec<SceneNode>) -> SceneTree<'a> {
        let mut was = Vec::new();
        walk(nodes, &mut |pointer, _| was.push(Some(pointer.to_owned())));
        let followed = was
            .iter()
            .flatten()
            .map(|pointer| path_of(pointer))
            .collect();
        SceneTree {
            nodes,
            followed,
            was,
        }
    }

    pub fn nodes(&self) -> &[SceneNode] {
        self.nodes
    }

    /// Where every node the scene held before the change is now, by its pointer before.
    pub fn remap(&self) -> Remap {
        self.was
            .iter()
            .zip(&self.followed)
            .filter_map(|(was, now)| Some((was.clone()?, pointer_of(now.as_ref()?))))
            .collect()
    }

    /// Takes the nodes at `pointers` out, in the order given; those missing are skipped.
    pub fn remove(&mut self, pointers: &[String]) -> Vec<SceneNode> {
        let marks: Vec<Option<Followed>> = pointers.iter().map(|p| self.follow(p)).collect();
        marks
            .into_iter()
            .filter_map(|mark| {
                let path = self.path(mark?)?;
                self.take(&path)
            })
            .collect()
    }

    /// Puts a copy of each node at `pointers` right after it, made by `copy` and labelled apart
    /// from its siblings. Answers where the copies landed, in tree order.
    pub fn duplicate(
        &mut self,
        pointers: &[String],
        mut copy: impl FnMut(&SceneNode) -> SceneNode,
    ) -> Vec<String> {
        let marks: Vec<Followed> = pointers.iter().filter_map(|p| self.follow(p)).collect();
        let mut made = Vec::new();
        for mark in marks {
            let Some(path) = self.path(mark) else {
                continue;
            };
            let Some((&index, folder)) = path.split_last() else {
                continue;
            };
            let Some(original) = node_at(self.nodes, &path) else {
                continue;
            };
            let mut node = copy(original);
            if let Some(label) = original.label() {
                let siblings = siblings_at(self.nodes, folder).unwrap_or_default();
                set_label(&mut node, free_label(label, siblings));
            }
            if let Some(copy) = self.insert(folder, index + 1, node) {
                made.push(copy);
            }
        }
        self.pointers_in_order(made)
    }

    /// Adds `new` at the end of `folder`, answering where each landed.
    pub fn append(&mut self, folder: &str, new: Vec<SceneNode>) -> Vec<String> {
        let Some(folder) = self.folder_path(folder) else {
            return Vec::new();
        };
        let mut made = Vec::new();
        for node in new {
            let end = siblings_at(self.nodes, &folder).map_or(0, <[SceneNode]>::len);
            made.extend(self.insert(&folder, end, node));
        }
        made.iter().filter_map(|&mark| self.pointer(mark)).collect()
    }

    /// Moves the nodes at `pointers` into `folder`, "" for the top level, landing as `at` says, in
    /// tree order; a label a sibling there has already gets the next free number. Answers where
    /// they landed, or `None` when the folder or the node it lands beside is among those moved.
    pub fn move_nodes(
        &mut self,
        pointers: &[String],
        folder: &str,
        at: &Landing,
    ) -> Option<Vec<String>> {
        let inside =
            |pointer: &str, of: &str| pointer == of || pointer.starts_with(&format!("{of}/"));
        if pointers.iter().any(|moved| inside(folder, moved)) {
            return None;
        }
        let anchor = match at {
            Landing::Before(pointer) | Landing::After(pointer) => Some(pointer.as_str()),
            _ => None,
        };
        if anchor.is_some_and(|anchor| pointers.iter().any(|moved| moved == anchor)) {
            return None;
        }
        let target = match folder {
            "" => None,
            folder => Some(self.follow(folder)?),
        };
        let anchor = match anchor {
            Some(anchor) => Some(self.follow(anchor)?),
            None => None,
        };
        let mut ordered: Vec<String> = pointers.to_vec();
        ordered.sort_by_key(|pointer| indices(pointer));
        ordered.dedup();
        ordered.retain(|pointer| {
            !pointers
                .iter()
                .any(|other| other != pointer && pointer.starts_with(&format!("{other}/")))
        });
        let marks: Vec<Followed> = ordered.iter().filter_map(|p| self.follow(p)).collect();
        let lifted: Vec<Lifted> = marks
            .iter()
            .filter_map(|&mark| self.lift(&self.path(mark)?))
            .collect();
        let folder = match target {
            Some(mark) => self.path(mark)?,
            None => Vec::new(),
        };
        let siblings = siblings_at(self.nodes, &folder)?.len();
        let anchor_at = anchor.and_then(|mark| Some(*self.path(mark)?.last()?));
        let start = match (at, anchor_at) {
            (Landing::First, _) => 0,
            (Landing::Before(_), Some(index)) => index,
            (Landing::After(_), Some(index)) => index + 1,
            _ => siblings,
        };
        for (index, mut lifted) in (start..).zip(lifted) {
            if let Some(label) = lifted.node.label() {
                let label = free_label(label, siblings_at(self.nodes, &folder)?);
                set_label(&mut lifted.node, label);
            }
            self.land(&folder, index, lifted);
        }
        Some(
            marks
                .iter()
                .filter_map(|&mark| self.pointer(mark))
                .collect(),
        )
    }

    /// Puts the nodes at `pointers` into a new folder labelled `label`, in the deepest folder
    /// holding them all, where the first of them is. Answers the new folder's pointer.
    pub fn group(&mut self, pointers: &[String], label: &str) -> Option<String> {
        let mut ordered: Vec<String> = pointers.to_vec();
        ordered.sort_by_key(|pointer| indices(pointer));
        let first = ordered.first()?.clone();
        let holder = holding_all(&ordered)?;
        let anchor = child_holding(&holder, &first)?;
        let (_, index) = place_of(&anchor)?;
        let moved: Vec<Followed> = ordered.iter().filter_map(|p| self.follow(p)).collect();
        let holder_path = self.folder_path(&holder)?;
        let made = folder(free_label(label, siblings_at(self.nodes, &holder_path)?));
        let made = self.insert(&holder_path, index, made)?;
        let new_folder = self.pointer(made)?;
        let moved: Vec<String> = moved
            .iter()
            .filter_map(|&mark| self.pointer(mark))
            .collect();
        self.move_nodes(&moved, &new_folder, &Landing::End)?;
        self.pointer(made)
    }

    /// Takes away the folders at `pointers`, what each held taking its place in order.
    pub fn ungroup(&mut self, pointers: &[String]) {
        let folders: Vec<&String> = pointers
            .iter()
            .filter(|pointer| node(self.nodes, pointer).is_some_and(is_folder))
            .collect();
        let marks: Vec<Followed> = folders
            .into_iter()
            .filter_map(|pointer| self.follow(pointer))
            .collect();
        for mark in marks {
            let Some(path) = self.path(mark) else {
                continue;
            };
            let Some((&index, holder)) = path.split_last() else {
                continue;
            };
            let holder = holder.to_vec();
            let held: Vec<Followed> = match node_at(self.nodes, &path) {
                Some(SceneNode::Folder(folder)) => (0..folder.children.len())
                    .filter_map(|child| {
                        let mut at = path.clone();
                        at.push(child);
                        self.follow_path(at)
                    })
                    .collect(),
                _ => continue,
            };
            for (offset, child) in held.into_iter().enumerate() {
                let Some(mut lifted) = self.path(child).and_then(|at| self.lift(&at)) else {
                    continue;
                };
                if let Some(label) = lifted.node.label() {
                    let siblings = siblings_at(self.nodes, &holder).unwrap_or_default();
                    let label = free_label(label, siblings);
                    set_label(&mut lifted.node, label);
                }
                self.land(&holder, index + offset, lifted);
            }
            if let Some(path) = self.path(mark) {
                self.take(&path);
            }
        }
    }

    /// Moves each node at `pointers` one place up (`-1`) or down (`1`) among its siblings,
    /// keeping them apart from each other's way.
    pub fn reorder(&mut self, pointers: &[String], step: i32) {
        let mut ordered: Vec<String> = pointers.to_vec();
        ordered.sort_by_key(|pointer| indices(pointer));
        if step > 0 {
            ordered.reverse();
        }
        let marks: Vec<Followed> = ordered.iter().filter_map(|p| self.follow(p)).collect();
        let mut taken: Vec<(Vec<usize>, usize)> = Vec::new();
        for mark in marks {
            let Some(path) = self.path(mark) else {
                continue;
            };
            let Some((&index, holder)) = path.split_last() else {
                continue;
            };
            let holder = holder.to_vec();
            let count = siblings_at(self.nodes, &holder).map_or(0, <[SceneNode]>::len);
            let to = index.checked_add_signed(step as isize);
            let blocked = |to: usize| {
                taken
                    .iter()
                    .any(|(other, at)| *other == holder && *at == to)
            };
            match to {
                Some(to) if to < count && !blocked(to) => {
                    if let Some(lifted) = self.lift(&path) {
                        self.land(&holder, to, lifted);
                    }
                    taken.push((holder, to));
                }
                _ => taken.push((holder, index)),
            }
        }
    }

    /// Sets `flag` of every node at `pointers`.
    pub fn set_flag(&mut self, pointers: &[String], flag: NodeFlag, on: bool) {
        for pointer in pointers {
            if let Some(node) = node_mut(self.nodes, pointer) {
                flag.set(node, on);
            }
        }
    }

    /// Clears `flag` on every node; answers how many had it.
    pub fn clear_flag(&mut self, flag: NodeFlag) -> usize {
        let mut set = Vec::new();
        walk(self.nodes, &mut |pointer, node| {
            if flag.of(node) {
                set.push(pointer.to_owned());
            }
        });
        self.set_flag(&set, flag, false);
        set.len()
    }

    fn follow(&mut self, pointer: &str) -> Option<Followed> {
        let path = path_of(pointer)?;
        node_at(self.nodes, &path)?;
        self.follow_path(path)
    }

    fn follow_path(&mut self, path: Vec<usize>) -> Option<Followed> {
        if let Some(known) = self
            .followed
            .iter()
            .position(|known| known.as_ref() == Some(&path))
        {
            return Some(Followed(known));
        }
        self.followed.push(Some(path));
        self.was.push(None);
        Some(Followed(self.followed.len() - 1))
    }

    fn path(&self, mark: Followed) -> Option<Vec<usize>> {
        self.followed.get(mark.0)?.clone()
    }

    fn pointer(&self, mark: Followed) -> Option<String> {
        self.path(mark).map(|path| pointer_of(&path))
    }

    fn folder_path(&self, folder: &str) -> Option<Vec<usize>> {
        match folder {
            "" => Some(Vec::new()),
            folder => {
                let path = path_of(folder)?;
                matches!(node_at(self.nodes, &path)?, SceneNode::Folder(_)).then_some(path)
            }
        }
    }

    fn pointers_in_order(&self, marks: Vec<Followed>) -> Vec<String> {
        let mut paths: Vec<Vec<usize>> = marks.iter().filter_map(|&mark| self.path(mark)).collect();
        paths.sort();
        paths.iter().map(|path| pointer_of(path)).collect()
    }

    /// Puts `node` at `index` of the folder at `holder`, following it.
    fn insert(&mut self, holder: &[usize], index: usize, node: SceneNode) -> Option<Followed> {
        let siblings = siblings_at_mut(self.nodes, holder)?;
        let index = index.min(siblings.len());
        siblings.insert(index, node);
        self.shifted(holder, index, 1);
        let mut path = holder.to_vec();
        path.push(index);
        self.followed.push(Some(path));
        self.was.push(None);
        Some(Followed(self.followed.len() - 1))
    }

    /// Takes the node at `path` out; what it held is followed no further.
    fn take(&mut self, path: &[usize]) -> Option<SceneNode> {
        let (&index, holder) = path.split_last()?;
        let siblings = siblings_at_mut(self.nodes, holder)?;
        if index >= siblings.len() {
            return None;
        }
        let node = siblings.remove(index);
        for followed in &mut self.followed {
            if followed
                .as_ref()
                .is_some_and(|known| known.starts_with(path))
            {
                *followed = None;
            }
        }
        self.shifted(holder, index + 1, -1);
        Some(node)
    }

    /// Takes the node at `path` out to put it back elsewhere, with every node it and what it holds
    /// are followed by.
    fn lift(&mut self, path: &[usize]) -> Option<Lifted> {
        let followers: Vec<(usize, Vec<usize>)> = self
            .followed
            .iter()
            .enumerate()
            .filter_map(|(at, known)| {
                let known = known.as_ref()?;
                Some((at, known.strip_prefix(path)?.to_vec()))
            })
            .collect();
        let node = self.take(path)?;
        Some(Lifted { node, followers })
    }

    /// Puts a lifted node at `index` of the folder at `holder`, its followers with it.
    fn land(&mut self, holder: &[usize], index: usize, lifted: Lifted) {
        let Some(siblings) = siblings_at_mut(self.nodes, holder) else {
            return;
        };
        let index = index.min(siblings.len());
        siblings.insert(index, lifted.node);
        self.shifted(holder, index, 1);
        for (at, below) in lifted.followers {
            let mut path = holder.to_vec();
            path.push(index);
            path.extend(below);
            self.followed[at] = Some(path);
        }
    }

    /// Shifts by `by` every followed node at or after `index` in the folder at `holder`.
    fn shifted(&mut self, holder: &[usize], index: usize, by: isize) {
        let depth = holder.len();
        for path in self.followed.iter_mut().flatten() {
            if path.len() > depth && path.starts_with(holder) && path[depth] >= index {
                path[depth] = path[depth].saturating_add_signed(by);
            }
        }
    }
}

/// Makes `change` to `nodes` through a `SceneTree` and answers where every node went, by pointer,
/// so whatever names nodes by their pointers can follow them.
pub fn restructure<R>(
    nodes: &mut Vec<SceneNode>,
    change: impl FnOnce(&mut SceneTree) -> R,
) -> (R, Remap) {
    let mut tree = SceneTree::new(nodes);
    let out = change(&mut tree);
    (out, tree.remap())
}

/// How many folders hold the node at `pointer`.
/// Sorts `pointers` into the order their nodes stand in the scene, depth first.
pub fn sort_in_scene_order(pointers: &mut [String]) {
    pointers.sort_by_key(|pointer| indices(pointer));
}

pub fn depth(pointer: &str) -> usize {
    pointer.matches("/children/").count()
}

/// How deep the folders inside `node` go, 0 when it holds none.
pub fn folders_inside(node: &SceneNode) -> usize {
    match node {
        SceneNode::Folder(folder) => folder
            .children
            .iter()
            .filter(|child| is_folder(child))
            .map(|child| 1 + folders_inside(child))
            .max()
            .unwrap_or(0),
        _ => 0,
    }
}

/// The deepest folder holding every node at `pointers`, "" for the top level.
fn holding_all(pointers: &[String]) -> Option<String> {
    let (mut folder, _) = place_of(pointers.first()?)?;
    while !pointers
        .iter()
        .all(|pointer| folder.is_empty() || pointer.starts_with(&format!("{folder}/")))
    {
        folder = place_of(folder)?.0;
    }
    Some(folder.to_owned())
}

/// The node directly in `folder` that is `pointer` or holds it.
fn child_holding(folder: &str, pointer: &str) -> Option<String> {
    let prefix = match folder {
        "" => "/".to_owned(),
        folder => format!("{folder}/children/"),
    };
    let rest = pointer.strip_prefix(&prefix)?;
    let index = rest.split('/').next()?;
    Some(format!("{prefix}{index}"))
}

fn indices(pointer: &str) -> Vec<usize> {
    pointer
        .split('/')
        .filter_map(|segment| segment.parse().ok())
        .collect()
}

/// The indices a pointer to a node of a scene steps through, folder by folder.
fn path_of(pointer: &str) -> Option<Vec<usize>> {
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

fn pointer_of(path: &[usize]) -> String {
    path.iter()
        .map(|index| format!("/{index}"))
        .collect::<Vec<_>>()
        .join("/children")
}

fn node_at<'a>(nodes: &'a [SceneNode], path: &[usize]) -> Option<&'a SceneNode> {
    let (&last, holder) = path.split_last()?;
    siblings_at(nodes, holder)?.get(last)
}

fn node_at_mut<'a>(nodes: &'a mut [SceneNode], path: &[usize]) -> Option<&'a mut SceneNode> {
    let (&first, rest) = path.split_first()?;
    let node = nodes.get_mut(first)?;
    if rest.is_empty() {
        return Some(node);
    }
    match node {
        SceneNode::Folder(folder) => node_at_mut(&mut folder.children, rest),
        _ => None,
    }
}

fn siblings_at<'a>(nodes: &'a [SceneNode], holder: &[usize]) -> Option<&'a [SceneNode]> {
    let mut siblings = nodes;
    for &index in holder {
        siblings = match siblings.get(index)? {
            SceneNode::Folder(folder) => &folder.children,
            _ => return None,
        };
    }
    Some(siblings)
}

fn siblings_at_mut<'a>(
    nodes: &'a mut Vec<SceneNode>,
    holder: &[usize],
) -> Option<&'a mut Vec<SceneNode>> {
    let Some((&first, rest)) = holder.split_first() else {
        return Some(nodes);
    };
    match nodes.get_mut(first)? {
        SceneNode::Folder(folder) => siblings_at_mut(&mut folder.children, rest),
        _ => None,
    }
}
