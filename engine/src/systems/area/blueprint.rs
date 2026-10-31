use std::collections::BTreeMap;
use std::io;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::scene::{self, Holes, ModuleNode, SCENE, SCENE_FILE, SceneNode, scene_file};
use super::{AreaId, AreaModule, ObjectShape};
use crate::core::audio::soundscape::SoundscapeShape;
use crate::core::content::id::{Leaf, in_scope, nearest, nearest_path, read_aside};
use crate::core::content::load::{ParseError, parse_value, shape_problem};
use crate::core::content::locate::{escape, unescape};
use crate::core::content::{
    Bundle, CONFIG_FILE, Content, ContentFile, ContentId, ContentProblem, ContentScope, FileWriter,
    Nearby, PartSpec, Parts, RowCheck, Shape, serial, valid_key, with_content,
};
use crate::core::geometry::Degrees;
use crate::core::ground::GroundSize;
use crate::core::math::{Pos, Rect};
use crate::core::script::{VarValue, Variable, VariableType, Vocabulary};
use crate::core::units::Meters;
use crate::systems::script::{Place, RiftScripting, ScriptModule};

pub type BlueprintId = ContentId<BlueprintDef>;

/// Placements nest at most this deep.
pub const MAX_DEPTH: usize = 8;
pub const MAX_PARTS: usize = 4096;
/// How many modules an area holds flattened, per square of its ground.
pub const MAX_AREA_PARTS: usize = 65_536;
pub const MAX_VARIABLES: usize = 32;

/// A reusable piece of an area: a stretch of ground with a scene graph of modules on it, placed as
/// one node of an area or of another blueprint. Its variables are holes in its modules that each
/// placement fills.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlueprintDef {
    pub size: GroundSize,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub variables: BTreeMap<String, Variable<RiftScripting>>,
    #[serde(skip)]
    pub scene: Vec<SceneNode>,
}

impl ContentFile for BlueprintDef {
    const TABLE: &'static str = "blueprint";
    const SHAPE: Shape = Shape::Bundle;

    fn fresh(_: &Nearby) -> Result<BlueprintDef, String> {
        Ok(BlueprintDef {
            size: GroundSize {
                width: 4,
                height: 4,
            },
            variables: BTreeMap::new(),
            scene: Vec::new(),
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        self.size.check("/size", 1, cx);
        cx.count(
            "/variables",
            self.variables.len(),
            0,
            MAX_VARIABLES,
            "variables",
        );
        for name in self.variables.keys() {
            if !(1..=40).contains(&name.len()) || !valid_key(name) {
                cx.error(
                    format!("/variables/{}", escape(name)),
                    "out of range",
                    format!("{name:?}"),
                    "a variable name of 1 to 40 key characters",
                );
            }
        }
    }
}

impl Bundle for BlueprintDef {
    const PARTS: &'static [PartSpec] = &[SCENE];
    const WRITTEN: &'static [PartSpec] = &[SCENE];

    fn read_parts(&mut self, parts: &mut Parts) {
        scene::read(&mut self.scene, &self.variables, parts);
        let used = scene::holes(&self.scene);
        for name in self.variables.keys() {
            if !used.contains(name) {
                parts.json_error(
                    CONFIG_FILE,
                    &format!("/variables/{}", escape(name)),
                    ContentProblem::new(
                        "unused variable",
                        name,
                        "a variable some hole in the scene names",
                    ),
                );
            }
        }
    }

    fn write_parts(&self, bundle: &str, writer: &mut FileWriter) -> io::Result<()> {
        scene::write(&self.scene, bundle, writer)
    }
}

/// What a hole reads as while its module is read for the content it names: a path no file has,
/// whose reference is left out.
const HOLE: &str = "\u{1}hole";

/// A hole in a module: a value left for each placement to fill, written in its place.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HoleMark {
    var: String,
}

/// Reads the module of a node as a blueprint declaring `variables` holds it: each hole is stood in
/// and kept apart by its pointer. A problem is answered with its pointer in the module.
pub(super) fn read_module(
    mut json: Value,
    variables: &BTreeMap<String, Variable<RiftScripting>>,
) -> Result<(AreaModule, Holes), Box<(String, ContentProblem)>> {
    let misread = |error: ParseError| Box::new((error.pointer.clone(), shape_problem(&error)));
    let mut found = BTreeMap::new();
    find_holes(&json, &mut String::new(), &mut found);
    if found.is_empty() {
        let module = parse_value::<AreaModule>(&json).map_err(misread)?;
        return Ok((module, Holes::default()));
    }
    let mut named = json.clone();
    for (pointer, name) in &found {
        let Some(variable) = variables.get(name) else {
            return Err(Box::new((
                format!("{pointer}/var"),
                ContentProblem::new(
                    "unknown variable",
                    format!("{name:?}"),
                    "a variable the blueprint declares",
                )
                .near(nearest(variables.keys(), name).map(|near| format!("{near:?}"))),
            )));
        };
        let Some(stand_in) = stand_in(variable) else {
            return Err(Box::new((
                pointer.clone(),
                ContentProblem::new(
                    "missing file",
                    format!("a {} variable with no file to name", variable.ty.name()),
                    "a file of the variable's type in the game",
                ),
            )));
        };
        let named_stand_in = match variable.default {
            Some(_) => stand_in.clone(),
            None => unnamed(stand_in.clone()),
        };
        put(&mut named, pointer, named_stand_in);
        put(&mut json, pointer, stand_in);
    }
    read_aside(
        || parse_value::<AreaModule>(&named),
        |leaf| leaf.value != HOLE,
    )
    .map_err(misread)?;
    let module = read_aside(|| parse_value::<AreaModule>(&json), |_| false).map_err(misread)?;
    Ok((module, Holes(found)))
}

/// The fields of a module, without its kind, as its file holds them: a hole at each of its holes.
pub(super) fn write_module(
    module: &AreaModule,
    holes: &Holes,
) -> Result<serde_json::Map<String, Value>, serde_json::Error> {
    // Through text, since a `Value` holds an f32 widened to the f64 nearest it, which writes far
    // longer than the f32 itself does.
    let text = match module {
        AreaModule::Object(fields) => serde_json::to_string(fields),
        AreaModule::SafeZone(fields) => serde_json::to_string(fields),
        AreaModule::Soundscape(fields) => serde_json::to_string(fields),
        AreaModule::Prop(fields) => serde_json::to_string(fields),
        AreaModule::Scatter(fields) => serde_json::to_string(fields),
        AreaModule::Light(fields) => serde_json::to_string(fields),
        AreaModule::Blueprint(fields) => serde_json::to_string(fields),
        AreaModule::Population(fields) => serde_json::to_string(fields),
        AreaModule::Resident(fields) => serde_json::to_string(fields),
        AreaModule::Script(fields) => serde_json::to_string(fields),
    }?;
    let mut json: Value = serde_json::from_str(&text)?;
    for (pointer, name) in &holes.0 {
        let mark = serde_json::to_value(HoleMark { var: name.clone() })?;
        put(&mut json, pointer, mark);
    }
    match json {
        Value::Object(fields) => Ok(fields),
        _ => Err(serde::ser::Error::custom(
            "a module's fields written as an object",
        )),
    }
}

/// The module of `node` with each hole filled by the value `values` gives its variable, or else
/// the variable's default; `None` when a hole is left with neither or the filled module doesn't
/// read. Ids are those of `scope`, which `values` and the module were read in.
pub fn fill_module(
    scope: &ContentScope,
    node: &ModuleNode,
    values: &BTreeMap<String, VarValue<RiftScripting>>,
    variables: &BTreeMap<String, Variable<RiftScripting>>,
) -> Option<AreaModule> {
    if node.holes.0.is_empty() {
        return Some(node.module.clone());
    }
    with_content(scope.clone(), || {
        let mut json = serde_json::to_value(&node.module).ok()?;
        for (at, name) in &node.holes.0 {
            let value = values
                .get(name)
                .or_else(|| variables.get(name)?.default.as_ref())?;
            put(&mut json, at, serde_json::to_value(value).ok()?);
        }
        serde_json::from_value::<AreaModule>(json).ok()
    })
    .0
}

/// The variables a module's holes name.
pub fn variables_of(holes: &Holes) -> impl Iterator<Item = &str> {
    holes.0.values().map(String::as_str)
}

fn find_holes(json: &Value, pointer: &mut String, found: &mut BTreeMap<String, String>) {
    let at = pointer.len();
    match json {
        Value::Object(map) => {
            // A mark is written as an object of its one field; trying every value as one would build
            // an error for each value it refuses, at a cost to every scene read.
            if map.len() == 1
                && let Ok(mark) = HoleMark::deserialize(json)
            {
                found.insert(pointer.clone(), mark.var);
                return;
            }
            for (key, value) in map {
                pointer.push('/');
                pointer.push_str(&escape(key));
                find_holes(value, pointer, found);
                pointer.truncate(at);
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                pointer.push('/');
                pointer.push_str(&index.to_string());
                find_holes(item, pointer, found);
                pointer.truncate(at);
            }
        }
        _ => {}
    }
}

/// A value of the variable's type: its default, or else any value of the type, in the scope being
/// read in.
fn stand_in(variable: &Variable<RiftScripting>) -> Option<Value> {
    if let Some(default) = &variable.default {
        return serde_json::to_value(default).ok();
    }
    match variable.ty {
        VariableType::Integer | VariableType::Number => Some(Value::from(0)),
        VariableType::Boolean => Some(Value::Bool(false)),
        VariableType::Condition => Some(Value::Bool(true)),
        VariableType::Text | VariableType::Name | VariableType::Audio => {
            Some(Value::String(String::new()))
        }
        VariableType::Place => serde_json::to_value(Place {
            area: AreaId::new(0),
            at: Pos::new(0.0, 0.0),
        })
        .ok(),
        VariableType::Row => in_scope(|scope| {
            scope
                .paths_named(variable.table.as_deref()?)
                .first()
                .cloned()
        })
        .flatten()
        .map(Value::String),
    }
}

/// `value` with every text in it a hole's, so the files a stand-in names count as no reference.
fn unnamed(value: Value) -> Value {
    match value {
        Value::String(_) => Value::String(HOLE.to_owned()),
        Value::Array(items) => Value::Array(items.into_iter().map(unnamed).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, unnamed(value)))
                .collect(),
        ),
        other => other,
    }
}

/// Puts `value` at `pointer`, adding the field it names, and the objects holding it, where they
/// are missing: an optional map left out when empty, as a placement's values, still takes a hole.
fn put(json: &mut Value, pointer: &str, value: Value) {
    if let Some(at) = json.pointer_mut(pointer) {
        *at = value;
        return;
    }
    let Some((parent, key)) = pointer.rsplit_once('/') else {
        return;
    };
    if json.pointer(parent).is_none() {
        put(json, parent, Value::Object(serde_json::Map::new()));
    }
    if let Some(Value::Object(map)) = json.pointer_mut(parent) {
        map.insert(unescape(key), value);
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub blueprint: BlueprintId,
    #[schemars(with = "serial::PosRow")]
    #[serde(serialize_with = "serial::pos", deserialize_with = "serial::pos_from")]
    pub at: Pos<Meters>,
    /// A quarter turn, so what the blueprint places keeps its rectangles square to the world.
    #[serde(default, skip_serializing_if = "Degrees::is_zero")]
    pub yaw: Degrees,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, VarValue<RiftScripting>>,
}

/// A module of an area once its scene is flattened: folders dissolved and placed blueprints
/// expanded.
#[derive(Clone)]
pub struct Flattened {
    pub module: AreaModule,
    /// The path of the node in the area's scene that is the module or brings it.
    pub origin: String,
    /// The placed blueprints it came through, each with the path of the node in that blueprint's
    /// scene that is the module or places the next one.
    pub chain: Vec<(BlueprintId, String)>,
}

impl Flattened {
    /// The file and JSON pointer of the node the module was written as.
    pub fn site(&self, content: &Content, area: AreaId) -> (String, String) {
        match self.chain.last() {
            None => (scene_file(area.path(content)), self.origin.clone()),
            Some((blueprint, pointer)) => (scene_file(blueprint.path(content)), pointer.clone()),
        }
    }

    pub fn describe(&self, content: &Content) -> String {
        match self.chain.last() {
            None => format!("node {}", self.origin),
            Some((blueprint, pointer)) => {
                let through: Vec<&str> = self.chain[..self.chain.len() - 1]
                    .iter()
                    .map(|(blueprint, _)| blueprint.name(content))
                    .collect();
                let mut text = format!("node {pointer} of {}", blueprint.name(content));
                if !through.is_empty() {
                    text.push_str(&format!(", through {}", through.join(" → ")));
                }
                text
            }
        }
    }
}

/// A blueprint placed in an area, at any depth: where its modules land, and the scripts that run
/// once for it.
#[derive(Clone)]
pub struct PlacedBlueprint {
    pub blueprint: BlueprintId,
    pub origin: String,
    pub chain: Vec<(BlueprintId, String)>,
    pub at: Pos<Meters>,
    pub yaw: Degrees,
    pub values: BTreeMap<String, Value>,
    /// Each script with the path of its node in the blueprint's scene.
    pub scripts: Vec<(String, ScriptModule)>,
}

pub struct Flattening {
    pub modules: Vec<Flattened>,
    pub placements: Vec<PlacedBlueprint>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlattenAt {
    /// A JSON pointer in the area's scene.
    Area(String),
    /// A JSON part of a blueprint and a pointer in it.
    Blueprint(BlueprintId, &'static str, String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlattenError {
    pub at: FlattenAt,
    pub problem: ContentProblem,
}

/// The modules an area's scene adds up to, in tree order: folders dissolve and each placement brings
/// its blueprint's modules, placed.
pub fn flatten(
    content: &Content,
    nodes: &[SceneNode],
    errors: &mut Vec<FlattenError>,
) -> Flattening {
    flatten_kept(content, nodes, &|_| true, errors)
}

/// `flatten`, leaving out every module node `keep` refuses. A node is named by its path: its
/// pointer in the scene, then for a node a placement brings, its pointer in each blueprint on the
/// way to it.
pub fn flatten_kept(
    content: &Content,
    nodes: &[SceneNode],
    keep: &dyn Fn(&[&str]) -> bool,
    errors: &mut Vec<FlattenError>,
) -> Flattening {
    let mut out = Vec::new();
    let mut placements = Vec::new();
    let mut found = Vec::new();
    scene::walk(nodes, &mut |pointer, node, _| {
        if let SceneNode::Module(node) = node
            && keep(&[pointer])
        {
            found.push((pointer.to_owned(), &node.module));
        }
    });
    for (pointer, module) in found {
        match module {
            AreaModule::Blueprint(placement) => {
                let mut cx = Expansion {
                    content,
                    errors: &mut *errors,
                    out: &mut out,
                    placements: &mut placements,
                    origin: pointer.clone(),
                    values_at: FlattenAt::Area(format!("{pointer}/values")),
                    chain: Vec::new(),
                    keep,
                    holes_open: false,
                };
                cx.expand(placement, placement.at, placement.yaw, &placement.values);
            }
            module => out.push(Flattened {
                module: module.clone(),
                origin: pointer,
                chain: Vec::new(),
            }),
        }
    }
    Flattening {
        modules: out,
        placements,
    }
}

/// A blueprint's scene flattened on its own canvas, leaving out every module node `keep` refuses,
/// named by its path as `flatten_kept` names it. Its holes are filled with their defaults; a module
/// with a hole that has none is left out. Origins are pointers in the blueprint's scene, as an
/// area's are in the area's.
pub fn flatten_blueprint(
    content: &Content,
    blueprint: BlueprintId,
    keep: &dyn Fn(&[&str]) -> bool,
    errors: &mut Vec<FlattenError>,
) -> Flattening {
    let mut out = Vec::new();
    let mut placements = Vec::new();
    let placement = Placement {
        blueprint,
        at: Pos::new(0.0, 0.0),
        yaw: Degrees(0.0),
        values: BTreeMap::new(),
    };
    let mut cx = Expansion {
        content,
        errors,
        out: &mut out,
        placements: &mut placements,
        origin: String::new(),
        values_at: FlattenAt::Blueprint(blueprint, CONFIG_FILE, "/variables".to_owned()),
        chain: Vec::new(),
        keep: &|path| keep(&path[1..]),
        holes_open: true,
    };
    cx.expand(&placement, placement.at, placement.yaw, &placement.values);
    let own = |chain: &mut Vec<(BlueprintId, String)>, origin: &mut String| {
        if !chain.is_empty() {
            *origin = chain.remove(0).1;
        }
    };
    for placed in &mut out {
        own(&mut placed.chain, &mut placed.origin);
    }
    let mut nested: Vec<PlacedBlueprint> = placements.into_iter().skip(1).collect();
    for placed in &mut nested {
        own(&mut placed.chain, &mut placed.origin);
    }
    Flattening {
        modules: out,
        placements: nested,
    }
}

fn misread(at: FlattenAt, error: serde_path_to_error::Error<serde_json::Error>) -> FlattenError {
    let error = ParseError::from(error);
    let at = match at {
        FlattenAt::Area(pointer) => FlattenAt::Area(format!("{pointer}{}", error.pointer)),
        FlattenAt::Blueprint(id, part, pointer) => {
            FlattenAt::Blueprint(id, part, format!("{pointer}{}", error.pointer))
        }
    };
    FlattenError {
        at,
        problem: shape_problem(&error),
    }
}

struct Expansion<'a> {
    content: &'a Content,
    errors: &'a mut Vec<FlattenError>,
    out: &'a mut Vec<Flattened>,
    placements: &'a mut Vec<PlacedBlueprint>,
    origin: String,
    /// Where the values the placement being expanded gives are written.
    values_at: FlattenAt,
    chain: Vec<(BlueprintId, String)>,
    keep: &'a dyn Fn(&[&str]) -> bool,
    /// Whether a variable with neither a value nor a default leaves its holes open instead of
    /// failing the placement.
    holes_open: bool,
}

impl Expansion<'_> {
    fn error(&mut self, at: FlattenAt, rule: &'static str, found: String, expected: String) {
        self.errors.push(FlattenError {
            at,
            problem: ContentProblem::new(rule, found, expected),
        });
    }

    /// Where the placement being expanded is written: in the area's scene at the top, or in the
    /// scene of the blueprint that places it.
    fn placed_at(&self) -> FlattenAt {
        match self.chain.last() {
            None => FlattenAt::Area(self.origin.clone()),
            Some((blueprint, pointer)) => {
                FlattenAt::Blueprint(*blueprint, SCENE_FILE, pointer.clone())
            }
        }
    }

    fn values_pointer(&self, name: &str) -> FlattenAt {
        match &self.values_at {
            FlattenAt::Area(pointer) => FlattenAt::Area(format!("{pointer}/{}", escape(name))),
            FlattenAt::Blueprint(id, part, pointer) => {
                FlattenAt::Blueprint(*id, part, format!("{pointer}/{}", escape(name)))
            }
        }
    }

    fn expand(
        &mut self,
        placement: &Placement,
        at: Pos<Meters>,
        yaw: Degrees,
        values: &BTreeMap<String, VarValue<RiftScripting>>,
    ) {
        let content = self.content.clone();
        let Some(def) = content.table::<BlueprintDef>().get(placement.blueprint) else {
            return;
        };
        if self.chain.len() + 1 > MAX_DEPTH {
            let chain = self.chain_text(Some(placement.blueprint));
            let at = self.placed_at();
            self.error(
                at,
                "nested too deep",
                format!("depth {} through {chain}", self.chain.len() + 1),
                format!("at most {MAX_DEPTH} levels of placements"),
            );
            return;
        }
        if self
            .chain
            .iter()
            .any(|(blueprint, _)| *blueprint == placement.blueprint)
        {
            let chain = self.chain_text(Some(placement.blueprint));
            self.error(
                FlattenAt::Blueprint(placement.blueprint, SCENE_FILE, String::new()),
                "blueprint holds itself",
                chain,
                "a blueprint that never contains itself, at any depth".to_owned(),
            );
            return;
        }
        let mut filled: BTreeMap<String, Value> = BTreeMap::new();
        let mut supplied: Vec<String> = Vec::new();
        let mut missing = false;
        for name in values.keys() {
            if !def.variables.contains_key(name) {
                let at = self.values_pointer(name);
                self.unknown_variable(at, placement.blueprint, def, name);
            }
        }
        for (name, variable) in &def.variables {
            let (value, own) = match values.get(name) {
                Some(value) => (value, true),
                None => match &variable.default {
                    Some(default) => (default, false),
                    None if self.holes_open => continue,
                    None => {
                        missing = true;
                        let at = self.values_at.clone();
                        self.error(
                            at,
                            "missing value",
                            format!("no value for {name}"),
                            format!(
                                "a {} value for {name}, which has no default",
                                variable.ty.name()
                            ),
                        );
                        continue;
                    }
                },
            };
            match typed_value(&content, variable, value) {
                Ok(json) => {
                    if own {
                        strings(&json, &mut supplied);
                    }
                    filled.insert(name.clone(), json);
                }
                Err(problem) => {
                    let at = if own {
                        self.values_pointer(name)
                    } else {
                        FlattenAt::Blueprint(
                            placement.blueprint,
                            CONFIG_FILE,
                            format!("/variables/{}/default", escape(name)),
                        )
                    };
                    self.errors.push(FlattenError { at, problem });
                }
            }
        }
        if missing {
            return;
        }
        let placed = self.placements.len();
        self.placements.push(PlacedBlueprint {
            blueprint: placement.blueprint,
            origin: self.origin.clone(),
            chain: self.chain.clone(),
            at,
            yaw,
            values: filled.clone(),
            scripts: Vec::new(),
        });
        let start = self.out.len();
        let mut nodes = Vec::new();
        let keep = self.keep;
        let above: Vec<&str> = std::iter::once(self.origin.as_str())
            .chain(self.chain.iter().map(|(_, pointer)| pointer.as_str()))
            .collect();
        scene::walk(&def.scene, &mut |pointer, node, _| {
            if let SceneNode::Module(node) = node {
                let path: Vec<&str> = above.iter().copied().chain([pointer]).collect();
                if keep(&path) {
                    nodes.push((pointer.to_owned(), node));
                }
            }
        });
        for (pointer, node) in nodes {
            let site = FlattenAt::Blueprint(placement.blueprint, SCENE_FILE, pointer.clone());
            let module = if node.holes.0.is_empty() {
                node.module.clone()
            } else {
                let (written, _) = with_content(ContentScope::of(&content), || {
                    serde_json::to_value(&node.module)
                });
                let Ok(mut json) = written else {
                    continue;
                };
                let mut open = false;
                for (at, name) in &node.holes.0 {
                    match filled.get(name) {
                        Some(value) => put(&mut json, at, value.clone()),
                        None => open = true,
                    }
                }
                if open {
                    continue;
                }
                let (result, scope) = with_content(ContentScope::of(&content), || {
                    serde_path_to_error::deserialize::<_, AreaModule>(json)
                });
                let from_placement = scope
                    .leaves()
                    .iter()
                    .filter(|leaf| supplied.contains(&leaf.value));
                for problem in from_placement.filter_map(Leaf::reported) {
                    self.errors.push(FlattenError {
                        at: site.clone(),
                        problem,
                    });
                }
                match result {
                    Ok(module) => module,
                    Err(error) => {
                        self.errors.push(misread(site, error));
                        continue;
                    }
                }
            };
            let mut chain = self.chain.clone();
            chain.push((placement.blueprint, pointer.clone()));
            match module {
                AreaModule::Script(script) => {
                    self.placements[placed].scripts.push((pointer, script))
                }
                AreaModule::Blueprint(nested) => {
                    let nested_at = at_of(at, yaw, nested.at);
                    let nested_yaw = Degrees(nested.yaw.0 + yaw.0);
                    let values_at = FlattenAt::Blueprint(
                        placement.blueprint,
                        SCENE_FILE,
                        format!("{pointer}/values"),
                    );
                    let mut inner = Expansion {
                        content: self.content,
                        errors: self.errors,
                        out: self.out,
                        placements: self.placements,
                        origin: self.origin.clone(),
                        values_at,
                        chain,
                        keep: self.keep,
                        holes_open: false,
                    };
                    inner.expand(&nested, nested_at, nested_yaw, &nested.values);
                }
                mut module => {
                    place(&mut module, at, yaw);
                    self.out.push(Flattened {
                        module,
                        origin: self.origin.clone(),
                        chain,
                    });
                }
            }
        }
        if self.out.len() - start > MAX_PARTS {
            self.error(
                FlattenAt::Blueprint(placement.blueprint, SCENE_FILE, String::new()),
                "too many parts",
                format!(
                    "{}: {}",
                    placement.blueprint.name(&content),
                    self.out.len() - start
                ),
                format!("at most {MAX_PARTS} flattened parts per blueprint"),
            );
        }
    }

    fn unknown_variable(
        &mut self,
        at: FlattenAt,
        blueprint: BlueprintId,
        def: &BlueprintDef,
        name: &str,
    ) {
        self.errors.push(FlattenError {
            at,
            problem: ContentProblem::new(
                "unknown variable",
                format!("{name:?}"),
                format!("a variable {} declares", blueprint.name(self.content)),
            )
            .near(nearest(def.variables.keys(), name).map(|n| format!("{n:?}"))),
        });
    }

    fn chain_text(&self, last: Option<BlueprintId>) -> String {
        let content = self.content;
        let mut names: Vec<&str> = self
            .chain
            .iter()
            .map(|(blueprint, _)| blueprint.path(content))
            .collect();
        if let Some(last) = last {
            names.push(last.path(content));
        }
        names.join(" → ")
    }
}

/// Where a point given in a placed blueprint's own coordinates lands.
fn at_of(at: Pos<Meters>, yaw: Degrees, point: Pos<Meters>) -> Pos<Meters> {
    at + yaw.rotate(point.to_vector())
}

/// Where a rectangle given in a placed blueprint's own coordinates lands: turned by the quarter
/// turns of `yaw`, it stays a rectangle.
fn rect_of(at: Pos<Meters>, yaw: Degrees, rect: Rect<Meters>) -> Rect<Meters> {
    Rect::from_points([at_of(at, yaw, rect.min()), at_of(at, yaw, rect.max())])
}

fn typed_value(
    content: &Content,
    variable: &Variable<RiftScripting>,
    value: &VarValue<RiftScripting>,
) -> Result<Value, ContentProblem> {
    let wrong = |expected: &str| {
        ContentProblem::new(
            "wrong type",
            format!("a {} value", value.kind()),
            format!("a {expected} value"),
        )
    };
    let json = |value: &VarValue<RiftScripting>| {
        with_content(ContentScope::of(content), || serde_json::to_value(value))
            .0
            .map_err(|error| ContentProblem::new("wrong type", error, "a JSON value"))
    };
    match (variable.ty, value) {
        (VariableType::Integer, VarValue::Integer(_))
        | (VariableType::Number, VarValue::Integer(_) | VarValue::Number(_))
        | (VariableType::Boolean, VarValue::Boolean(_))
        | (VariableType::Text | VariableType::Name | VariableType::Audio, VarValue::Str(_))
        | (VariableType::Place, VarValue::Place(_))
        | (VariableType::Condition, VarValue::Condition(_) | VarValue::Boolean(_)) => json(value),
        (VariableType::Row, VarValue::Str(key)) => {
            let table = variable.table.as_deref().unwrap_or("");
            match RiftScripting::row_value(content, table, key) {
                Some(_) => json(value),
                None => Err(ContentProblem::unknown_row(
                    table,
                    key,
                    content
                        .table_named(table)
                        .and_then(|files| nearest_path(files.paths(), key))
                        .as_deref(),
                )),
            }
        }
        _ => Err(wrong(variable.ty.name())),
    }
}

fn strings(json: &Value, into: &mut Vec<String>) {
    match json {
        Value::String(text) => into.push(text.clone()),
        Value::Object(map) => map.values().for_each(|value| strings(value, into)),
        Value::Array(items) => items.iter().for_each(|item| strings(item, into)),
        _ => {}
    }
}

pub fn place(module: &mut AreaModule, at: Pos<Meters>, yaw: Degrees) {
    let point = |p: Pos<Meters>| at_of(at, yaw, p);
    let rect = |r: Rect<Meters>| rect_of(at, yaw, r);
    let turned = |facing: Degrees| Degrees((facing.0 + yaw.0).rem_euclid(360.0));
    match module {
        AreaModule::Object(object) => {
            object.shape = match object.shape {
                ObjectShape::Point(p) => ObjectShape::Point(point(p)),
                ObjectShape::Rect(r) => ObjectShape::Rect(rect(r)),
            };
        }
        AreaModule::SafeZone(zone) => zone.rect = rect(zone.rect),
        AreaModule::Soundscape(soundscape) => {
            soundscape.shape = match soundscape.shape {
                SoundscapeShape::Rect(r) => SoundscapeShape::Rect(rect(r)),
                SoundscapeShape::Ellipse(r) => SoundscapeShape::Ellipse(rect(r)),
            };
        }
        AreaModule::Prop(prop) => {
            prop.at = point(prop.at);
            prop.yaw = turned(prop.yaw);
        }
        AreaModule::Scatter(scatter) => scatter.region = rect(scatter.region),
        AreaModule::Light(light) => light.at = point(light.at),
        AreaModule::Population(population) => {
            population.roams = population.roams.map(rect);
        }
        AreaModule::Resident(resident) => {
            resident.at = point(resident.at);
            resident.facing = turned(resident.facing);
        }
        AreaModule::Blueprint(_) | AreaModule::Script(_) => {}
    }
}
