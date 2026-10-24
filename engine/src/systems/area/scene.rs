use std::collections::BTreeMap;
use std::io;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::blueprint::{self, BlueprintDef};
use super::{AreaModule, AreaModuleKind};
use crate::core::content::load::{JsonPart, ParseError, parse_value, shape_problem};
use crate::core::content::{Content, ContentProblem, FileWriter, PartSpec, Parts, serial};
use crate::core::script::Variable;
use crate::systems::script::RiftScripting;

pub const SCENE_FILE: &str = "scene.json";

pub const SCENE: PartSpec = PartSpec::File {
    name: "scene",
    exts: &["json"],
};

/// Folders of a scene nest at most this deep.
pub const MAX_FOLDER_DEPTH: usize = 16;

/// One node of a scene graph. Folders only organize; every other node is a module.
#[derive(Clone)]
pub enum SceneNode {
    Folder(SceneFolder),
    Module(ModuleNode),
}

#[derive(Clone)]
pub struct SceneFolder {
    pub label: String,
    pub hidden: bool,
    pub locked: bool,
    pub children: Vec<SceneNode>,
}

/// A module of a scene, with the label and flags the editor keeps for its node; the game never
/// reads them.
#[derive(Clone)]
pub struct ModuleNode {
    pub label: Option<String>,
    pub hidden: bool,
    pub locked: bool,
    pub module: AreaModule,
    pub holes: Holes,
}

/// The holes a blueprint leaves in a module for each placement to fill, by their JSON pointer in
/// the module, each with the name of the variable that fills it. The module holds a stand-in of
/// the variable's type at each: its default, or any value of the type.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Holes(pub BTreeMap<String, String>);

impl SceneNode {
    pub fn label(&self) -> Option<&str> {
        match self {
            SceneNode::Folder(folder) => Some(&folder.label),
            SceneNode::Module(node) => node.label.as_deref(),
        }
    }

    pub fn hidden(&self) -> bool {
        match self {
            SceneNode::Folder(folder) => folder.hidden,
            SceneNode::Module(node) => node.hidden,
        }
    }

    pub fn locked(&self) -> bool {
        match self {
            SceneNode::Folder(folder) => folder.locked,
            SceneNode::Module(node) => node.locked,
        }
    }
}

impl ModuleNode {
    pub fn new(module: AreaModule) -> ModuleNode {
        ModuleNode {
            label: None,
            hidden: false,
            locked: false,
            module,
            holes: Holes::default(),
        }
    }
}

/// Every node under `nodes`, depth first in tree order, with its JSON pointer in the scene and how
/// many folders hold it.
pub fn walk<'a>(nodes: &'a [SceneNode], visit: &mut impl FnMut(&str, &'a SceneNode, usize)) {
    fn inner<'a>(
        nodes: &'a [SceneNode],
        prefix: &str,
        depth: usize,
        visit: &mut impl FnMut(&str, &'a SceneNode, usize),
    ) {
        for (index, node) in nodes.iter().enumerate() {
            let pointer = format!("{prefix}/{index}");
            visit(&pointer, node, depth);
            if let SceneNode::Folder(folder) = node {
                inner(
                    &folder.children,
                    &format!("{pointer}/children"),
                    depth + 1,
                    visit,
                );
            }
        }
    }
    inner(nodes, "", 0, visit);
}

/// Reads the scene of the bundle `parts` holds, reporting every problem with its structure at its
/// node.
pub fn read(
    scene: &mut Vec<SceneNode>,
    variables: &BTreeMap<String, Variable<RiftScripting>>,
    parts: &mut Parts,
) {
    let Some(file) = parts.required(SCENE) else {
        return;
    };
    let mut read = None;
    parts.json::<Vec<NodeFile>>(file, |files| read = Some(nodes_of(files, "", variables)));
    let Some(Read { nodes, problems }) = read else {
        return;
    };
    let read_whole = problems.is_empty();
    for (pointer, problem) in problems {
        parts.json_error(SCENE_FILE, &pointer, problem);
    }
    if !read_whole {
        return;
    }
    check(&nodes, parts);
    *scene = nodes;
}

pub fn write(scene: &[SceneNode], bundle: &str, writer: &mut FileWriter) -> io::Result<()> {
    let files = files_of(scene).map_err(io::Error::other)?;
    writer.json(&format!("{bundle}/{SCENE_FILE}"), &files)
}

/// `nodes` as a scene file holds them, in the scope being written in: how nodes travel apart from
/// their bundle, copied to be pasted elsewhere.
pub fn write_nodes(nodes: &[SceneNode]) -> Result<Value, serde_json::Error> {
    serde_json::to_value(files_of(nodes)?)
}

/// The nodes `json` holds as a scene file does, read as a blueprint declaring `variables` reads
/// its own, in the scope being read in; each problem at its pointer.
pub fn read_nodes(
    json: &Value,
    variables: &BTreeMap<String, Variable<RiftScripting>>,
) -> Result<Vec<SceneNode>, Vec<(String, ContentProblem)>> {
    let files = parse_value::<Vec<NodeFile>>(json)
        .map_err(|error| vec![(error.pointer.clone(), shape_problem(&error))])?;
    let read = nodes_of(&files, "", variables);
    if read.problems.is_empty() {
        Ok(read.nodes)
    } else {
        Err(read.problems)
    }
}

/// The scene of each bundle of `table`, read for the content its modules name.
pub fn json_part(table: &'static str) -> JsonPart {
    JsonPart {
        table,
        name: SCENE_FILE,
        read: read_part,
    }
}

fn read_part(content: &Content, bundle: &str, value: &Value) -> Result<(), ParseError> {
    let files = parse_value::<Vec<NodeFile>>(value)?;
    let variables = content
        .by_path::<BlueprintDef>(bundle)
        .map(|blueprint| blueprint.get(content).variables.clone())
        .unwrap_or_default();
    nodes_of(&files, "", &variables);
    Ok(())
}

/// A scene's nodes as its file holds them, read.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum NodeFile {
    Folder(FolderFile<NodeFile>),
    #[serde(untagged)]
    Module(ModuleFile),
}

/// A scene's nodes as its file holds them, written: a module's kind first, then its node's label
/// and flags, then its own fields.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum NodeOut {
    Folder(FolderFile<NodeOut>),
    #[serde(untagged)]
    Module(ModuleOut),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FolderFile<Node> {
    label: String,
    #[serde(default, skip_serializing_if = "serial::is_false")]
    hidden: bool,
    #[serde(default, skip_serializing_if = "serial::is_false")]
    locked: bool,
    #[serde(default = "Vec::new", skip_serializing_if = "Vec::is_empty")]
    children: Vec<Node>,
}

#[derive(Serialize)]
struct ModuleOut {
    #[serde(rename = "type")]
    kind: AreaModuleKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "serial::is_false")]
    hidden: bool,
    #[serde(skip_serializing_if = "serial::is_false")]
    locked: bool,
    #[serde(flatten)]
    fields: serde_json::Map<String, Value>,
}

#[derive(Serialize, Deserialize)]
struct ModuleFile {
    #[serde(flatten)]
    module: serde_json::Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<NonEmpty>,
    #[serde(default, skip_serializing_if = "serial::is_false")]
    hidden: bool,
    #[serde(default, skip_serializing_if = "serial::is_false")]
    locked: bool,
}

/// A label of text that isn't empty.
#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
struct NonEmpty(String);

impl TryFrom<String> for NonEmpty {
    type Error = &'static str;

    fn try_from(text: String) -> Result<NonEmpty, &'static str> {
        if text.is_empty() {
            Err("an empty label")
        } else {
            Ok(NonEmpty(text))
        }
    }
}

impl From<NonEmpty> for String {
    fn from(label: NonEmpty) -> String {
        label.0
    }
}

/// What a node that isn't a module says it is, read apart so a folder node that fails to read as
/// one is reported as what it says it is rather than as an unknown module.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StructureFile {
    Folder(FolderFile<NodeFile>),
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum StructureKind {
    Folder,
}

#[derive(Deserialize)]
struct StructureTag {
    #[serde(rename = "type")]
    _kind: StructureKind,
}

struct Read {
    nodes: Vec<SceneNode>,
    problems: Vec<(String, ContentProblem)>,
}

/// The nodes `files` hold, read in the scope being read in, so the content their modules name
/// counts as the bundle's references.
fn nodes_of(
    files: &[NodeFile],
    prefix: &str,
    variables: &BTreeMap<String, Variable<RiftScripting>>,
) -> Read {
    let mut read = Read {
        nodes: Vec::new(),
        problems: Vec::new(),
    };
    for (index, file) in files.iter().enumerate() {
        read_node(file, &format!("{prefix}/{index}"), variables, &mut read);
    }
    read
}

fn read_node(
    file: &NodeFile,
    pointer: &str,
    variables: &BTreeMap<String, Variable<RiftScripting>>,
    read: &mut Read,
) {
    match file {
        NodeFile::Folder(folder) => {
            let inner = nodes_of(&folder.children, &format!("{pointer}/children"), variables);
            read.problems.extend(inner.problems);
            read.nodes.push(SceneNode::Folder(SceneFolder {
                label: folder.label.clone(),
                hidden: folder.hidden,
                locked: folder.locked,
                children: inner.nodes,
            }));
        }
        NodeFile::Module(file) => {
            let json = Value::Object(file.module.clone());
            if parse_value::<StructureTag>(&json).is_ok() {
                let mut whole = json;
                if let Value::Object(map) = &mut whole {
                    map.extend(module_file_fields(file));
                }
                match parse_value::<StructureFile>(&whole) {
                    Ok(StructureFile::Folder(folder)) => {
                        read_node(&NodeFile::Folder(folder), pointer, variables, read)
                    }
                    Err(error) => read
                        .problems
                        .push((format!("{pointer}{}", error.pointer), shape_problem(&error))),
                }
                return;
            }
            match blueprint::read_module(json, variables) {
                Ok((module, holes)) => read.nodes.push(SceneNode::Module(ModuleNode {
                    label: file.label.as_ref().map(|label| label.0.clone()),
                    hidden: file.hidden,
                    locked: file.locked,
                    module,
                    holes,
                })),
                Err((at, problem)) => read.problems.push((format!("{pointer}{at}"), problem)),
            }
        }
    }
}

/// The node fields of a module file, as the node writes them.
fn module_file_fields(file: &ModuleFile) -> serde_json::Map<String, Value> {
    let fields = ModuleFile {
        module: serde_json::Map::new(),
        label: file.label.as_ref().map(|label| NonEmpty(label.0.clone())),
        hidden: file.hidden,
        locked: file.locked,
    };
    match serde_json::to_value(fields) {
        Ok(Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    }
}

/// The files `nodes` are written as, in the scope being written in.
fn files_of(nodes: &[SceneNode]) -> Result<Vec<NodeOut>, serde_json::Error> {
    nodes
        .iter()
        .map(|node| {
            Ok(match node {
                SceneNode::Folder(folder) => NodeOut::Folder(FolderFile {
                    label: folder.label.clone(),
                    hidden: folder.hidden,
                    locked: folder.locked,
                    children: files_of(&folder.children)?,
                }),
                SceneNode::Module(node) => NodeOut::Module(ModuleOut {
                    kind: node.module.kind(),
                    label: node.label.clone(),
                    hidden: node.hidden,
                    locked: node.locked,
                    fields: blueprint::write_module(&node.module, &node.holes)?,
                }),
            })
        })
        .collect()
}

fn check(nodes: &[SceneNode], parts: &mut Parts) {
    walk(nodes, &mut |pointer, node, depth| {
        if depth >= MAX_FOLDER_DEPTH && matches!(node, SceneNode::Folder(_)) {
            parts.json_error(
                SCENE_FILE,
                pointer,
                ContentProblem::new(
                    "nested too deep",
                    format!("a folder {} deep", depth + 1),
                    format!("folders nested at most {MAX_FOLDER_DEPTH} deep"),
                ),
            );
        }
        if let SceneNode::Module(node) = node
            && let AreaModule::Blueprint(placement) = &node.module
        {
            if node.label.is_none() {
                parts.json_error(
                    SCENE_FILE,
                    pointer,
                    ContentProblem::new(
                        "missing field",
                        "a placement without a label",
                        "a label on every placement, its name in the editor",
                    ),
                );
            }
            if placement.yaw.quarter_turns().is_none() {
                parts.json_error(
                    SCENE_FILE,
                    &format!("{pointer}/yaw"),
                    ContentProblem::new(
                        "not a quarter turn",
                        format!("{}°", placement.yaw.0),
                        "a placement turned by 0, 90, 180 or 270 degrees",
                    ),
                );
            }
        }
    });
    check_labels(nodes, "", parts);
}

/// Labels are unique among siblings ignoring case, as file names are in a folder.
fn check_labels(nodes: &[SceneNode], prefix: &str, parts: &mut Parts) {
    let mut seen: Vec<&str> = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        let pointer = format!("{prefix}/{index}");
        if let Some(label) = node.label() {
            if seen.iter().any(|known| known.eq_ignore_ascii_case(label)) {
                parts.json_error(
                    SCENE_FILE,
                    &format!("{pointer}/label"),
                    ContentProblem::new(
                        "duplicate name",
                        format!("{label:?} twice in one folder"),
                        "labels unique among their siblings",
                    ),
                );
            }
            seen.push(label);
        }
        if let SceneNode::Folder(folder) = node {
            check_labels(&folder.children, &format!("{pointer}/children"), parts);
        }
    }
}

/// The variables the holes of every module node name.
pub fn holes(nodes: &[SceneNode]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    walk(nodes, &mut |_, node, _| {
        if let SceneNode::Module(node) = node {
            for name in blueprint::variables_of(&node.holes) {
                if !found.iter().any(|known| known == name) {
                    found.push(name.to_owned());
                }
            }
        }
    });
    found
}

/// The path of a bundle's scene file.
pub fn scene_file(bundle: &str) -> String {
    format!("{bundle}/{SCENE_FILE}")
}
