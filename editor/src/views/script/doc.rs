use super::nodes::{NodeText, Nodes};
use crate::app::Editor;
use crate::forms::{FormAction, FormCx};
use crate::project::Project;
use crate::project::files::content_of;
use crate::scene_graph;
use engine::core::content::{ContentFile, FolderReader, fields_file};
use engine::systems::area::AreaModule;
use engine::systems::area::scene::{SCENE_FILE, SceneNode};
use engine::systems::npc::NpcDef;
use engine::systems::script::{ListedScriptModule, RunnerKind, ScriptDef, ScriptModule, Speaker};
use serde_json::Value;
use std::sync::Arc;

/// Where in its file a module that runs a script is: a module by its pointer, the place of a
/// module in a list of modules or of a node in a scene, or one of an object's scripts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ScriptPath {
    Module(String),
    ObjectScript { object: String, script: usize },
}

impl ScriptPath {
    pub fn parse(pointer: &str) -> Option<ScriptPath> {
        let module = scene_graph::module_pointer(pointer)?;
        let rest = &pointer[module.len()..];
        let Some(script) = rest
            .strip_prefix("/scripts/")
            .and_then(|rest| rest.split('/').next())
            .and_then(|index| index.parse().ok())
        else {
            return Some(ScriptPath::Module(module));
        };
        Some(ScriptPath::ObjectScript {
            object: module,
            script,
        })
    }

    pub fn pointer(&self) -> String {
        match self {
            ScriptPath::Module(module) => module.clone(),
            ScriptPath::ObjectScript { object, script } => format!("{object}/scripts/{script}"),
        }
    }

    /// The script module at this place of `file`, as the file holds it now: one of a row's
    /// modules, a script module of a scene, or one of the scripts of a scene's object.
    pub(crate) fn module_in(&self, project: &Project, file: &str) -> Option<ScriptModule> {
        let Some(bundle) = file.strip_suffix(&format!("/{SCENE_FILE}")) else {
            return match self {
                ScriptPath::Module(pointer) => project
                    .value_at::<ListedScriptModule>(file, pointer)
                    .map(|ListedScriptModule::Script(module)| module),
                ScriptPath::ObjectScript { .. } => None,
            };
        };
        let scene = crate::views::area::nodes::bundle_in(project, bundle)?;
        let pointer = match self {
            ScriptPath::Module(pointer) => pointer,
            ScriptPath::ObjectScript { object, .. } => object,
        };
        let Some(SceneNode::Module(node)) = crate::views::area::nodes::node(scene.nodes(), pointer)
        else {
            return None;
        };
        match (&node.module, self) {
            (AreaModule::Script(module), ScriptPath::Module(_)) => Some(module.clone()),
            (AreaModule::Object(object), ScriptPath::ObjectScript { script, .. }) => {
                object.scripts.get(*script).cloned()
            }
            _ => None,
        }
    }

    pub fn object(&self) -> Option<&str> {
        match self {
            ScriptPath::Module(_) => None,
            ScriptPath::ObjectScript { object, .. } => Some(object),
        }
    }

    pub fn module(&self) -> &str {
        match self {
            ScriptPath::Module(module) | ScriptPath::ObjectScript { object: module, .. } => module,
        }
    }
}

pub struct ScriptRef {
    pub module: Option<ScriptPath>,
    pub node: Option<String>,
}

impl ScriptRef {
    pub fn parse(pointer: &str) -> ScriptRef {
        ScriptRef {
            module: ScriptPath::parse(pointer),
            node: pointer
                .split_once("/nodes/")
                .and_then(|(_, rest)| rest.split('/').next())
                .map(str::to_owned),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ScriptRow {
    pub key: String,
}

impl ScriptRow {
    pub fn new(key: impl Into<String>) -> ScriptRow {
        ScriptRow { key: key.into() }
    }

    pub fn title(&self) -> String {
        crate::tabs::file_title(&self.key).to_owned()
    }

    pub fn row(&self) -> (String, String) {
        (ScriptDef::TABLE.to_owned(), self.key.clone())
    }

    pub fn file(&self) -> String {
        fields_file(&self.key)
    }

    pub fn slug(&self) -> String {
        format!("script-{}", self.key)
    }
}

#[derive(Clone)]
pub struct ScriptDoc {
    pub at: ScriptRow,
    pub file: String,
    pub script: Arc<ScriptDef>,
    /// What names the files the script's references are ids among.
    pub reader: Arc<FolderReader>,
    pub kind: RunnerKind,
    /// The one npc that names the script, who speaks its lines unless a line says otherwise.
    pub speaker: Option<String>,
}

impl ScriptDoc {
    /// The script at `at` as the game reads it, when it does.
    pub fn load(editor: &Editor, at: &ScriptRow) -> Option<ScriptDoc> {
        let project = editor.project.as_ref()?;
        let script = project.read::<ScriptDef>(&at.key).ok()?;
        let mut npcs: Vec<&str> = project
            .usages
            .of_content(&at.key)
            .iter()
            .filter_map(|usage| content_of(&usage.file))
            .filter(|(table, _)| *table == NpcDef::TABLE)
            .map(|(_, npc)| npc)
            .collect();
        npcs.dedup();
        Some(ScriptDoc {
            at: at.clone(),
            file: at.file(),
            kind: script.runs_on,
            speaker: match npcs.as_slice() {
                [npc] => Some((*npc).to_owned()),
                _ => None,
            },
            reader: project.reader(),
            script,
        })
    }

    pub fn nodes(&self) -> &Nodes {
        &self.script.nodes
    }

    /// The script as it stands, to change and write back.
    pub fn edited(&self) -> ScriptDef {
        (*self.script).clone()
    }

    pub(crate) fn default_speaker(&self) -> Speaker {
        let npc = self
            .speaker
            .as_deref()
            .and_then(|npc| self.reader.id_of::<NpcDef>(npc));
        match (self.kind, npc) {
            (RunnerKind::Npc, Some(npc)) => Speaker::Npc(npc),
            (RunnerKind::Object, _) => Speaker::Object,
            _ => Speaker::Narrator,
        }
    }

    /// What reads the names and files a node's summary shows.
    pub fn text<'a>(&'a self, project: &'a Project) -> NodeText<'a> {
        NodeText {
            project,
            reader: &self.reader,
            catalogue: &project.catalogue,
        }
    }

    pub(crate) fn form_cx<'a>(
        &self,
        editor: &'a mut Editor,
        root: &'a Value,
        actions: &'a mut Vec<FormAction>,
    ) -> FormCx<'a> {
        let Editor {
            project,
            form,
            shapes,
            ..
        } = editor;
        let project = project.as_ref().expect("a project");
        let mut cx = FormCx::new(project, root, shapes, form, actions)
            .editing(ScriptDef::TABLE, Some(&self.at.key));
        cx.kept = Some(crate::schema::KeptValues::of(&self.script));
        cx.speaker = match self.default_speaker() {
            Speaker::Npc(npc) => self.reader.path_of(npc).map(str::to_owned),
            _ => None,
        };
        cx
    }
}
