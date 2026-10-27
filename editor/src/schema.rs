use std::any::TypeId;
use std::collections::{BTreeMap, HashMap};
use std::sync::{LazyLock, Mutex, OnceLock, PoisonError};

use engine::core::assets::{AssetKind, AudioRef};
use engine::core::content::fixture::FixtureLock;
use engine::core::content::load::{ContentShapes, TableShape};
use engine::core::content::{
    Content, ContentFile, ContentScope, fields_file, locate, with_content,
};
use engine::core::math::Pos;
use engine::core::script::VariableType;
use engine::core::script::{Assignment, Target, ValueType};
use engine::systems::area::{self, AreaDef, AreaModule};
use engine::systems::registry;
use engine::systems::script::{Condition, Place, RiftQuery, RunnerKind, ScriptDef};
use serde_json::{Map, Value, json};

use crate::project::{Project, ProjectFiles};

pub struct SchemaShapes {
    pub tables: BTreeMap<String, TableShape>,
    pub order: Vec<String>,
    pub settings: Value,
    /// The shape of one module of an area's or a blueprint's scene.
    pub scene_module: Value,
}

pub fn shapes() -> ContentShapes {
    registry().shapes(&schema_settings())
}

fn schema_settings() -> schemars::generate::SchemaSettings {
    schemars::generate::SchemaSettings::draft2020_12()
        .with(|settings| settings.inline_subschemas = true)
}

impl SchemaShapes {
    pub fn load() -> SchemaShapes {
        let shapes = shapes();
        SchemaShapes {
            order: shapes
                .tables
                .iter()
                .map(|table| table.table.clone())
                .collect(),
            tables: shapes
                .tables
                .into_iter()
                .map(|table| (table.table.clone(), table))
                .collect(),
            settings: shapes.settings.unwrap_or_default(),
            scene_module: schema_settings()
                .into_generator()
                .into_root_schema_for::<AreaModule>()
                .to_value(),
        }
    }

    /// The kinds of module a scene can hold, each with its shape within `scene_module`.
    pub fn scene_variants(&self) -> Vec<SchemaVariant<'_>> {
        match kind(&self.scene_module, &self.scene_module) {
            SchemaKind::Union(variants) => variants,
            _ => Vec::new(),
        }
    }

    pub fn table(&self, name: &str) -> Option<&TableShape> {
        self.tables.get(name)
    }

    pub fn shape_of(&self, name: &str) -> &Value {
        static NULL: Value = Value::Null;
        self.tables.get(name).map_or(&NULL, |table| &table.shape)
    }

    pub fn is_fixture(&self, table: &str, key: &str) -> bool {
        self.tables
            .get(table)
            .is_some_and(|shape| shape.fixtures.iter().any(|fixture| fixture == key))
    }

    pub fn is_closed(&self, table: &str) -> bool {
        self.tables.get(table).is_some_and(|shape| shape.closed)
    }

    pub fn locks(&self, table: &str, key: &str) -> &[FixtureLock] {
        self.tables
            .get(table)
            .and_then(|shape| shape.locks.get(key))
            .map_or(&[], Vec::as_slice)
    }

    pub fn modules_of(&self, table: &str) -> Option<&Value> {
        modules_in(&self.tables.get(table)?.shape)
    }

    pub fn module_variants(&self, table: &str) -> Vec<SchemaVariant<'_>> {
        let Some(shape) = self.tables.get(table).map(|table| &table.shape) else {
            return Vec::new();
        };
        match self.modules_of(table).map(|items| kind(shape, items)) {
            Some(SchemaKind::Union(variants)) => variants,
            _ => Vec::new(),
        }
    }
}

pub enum SchemaKind<'a> {
    Object,
    Array(&'a Value),
    Map(&'a Value),
    Union(Vec<SchemaVariant<'a>>),
    Choice(Vec<&'a Value>),
    Enum(Vec<String>),
    Const(&'a Value),
    Text,
    Integer,
    Number,
    Boolean,
    Null,
    Any,
}

pub struct SchemaVariant<'a> {
    pub field: String,
    pub tag: String,
    pub schema: &'a Value,
}

pub fn resolve<'a>(root: &'a Value, schema: &'a Value) -> &'a Value {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str)
        && let Some(pointer) = reference.strip_prefix('#')
        && let Some(at) = root.pointer(pointer)
    {
        return at;
    }
    schema
}

pub fn at_pointer<'a>(
    root: &'a Value,
    schema: &'a Value,
    value: &Value,
    pointer: &str,
) -> Option<&'a Value> {
    let mut schema = settle(root, schema, value);
    let mut value = value;
    for segment in pointer.split('/').skip(1) {
        let segment = locate::unescape(segment);
        let field = match kind(root, schema) {
            SchemaKind::Object => schema.get("properties")?.get(&segment)?,
            SchemaKind::Map(values) => values,
            SchemaKind::Array(items) => items,
            _ => return None,
        };
        value = match value {
            Value::Object(fields) => fields.get(&segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
        schema = settle(root, field, value);
    }
    Some(schema)
}

pub fn settle<'a>(root: &'a Value, schema: &'a Value, value: &Value) -> &'a Value {
    let schema = resolve(root, schema);
    match kind(root, schema) {
        SchemaKind::Union(variants) => variants
            .into_iter()
            .find(|variant| {
                value.get(&variant.field).and_then(Value::as_str) == Some(variant.tag.as_str())
            })
            .map_or(schema, |variant| variant.schema),
        SchemaKind::Choice(alternatives) => alternatives
            .into_iter()
            .find(|alternative| fits(root, alternative, value))
            .map_or(schema, |alternative| resolve(root, alternative)),
        _ => schema,
    }
}

/// What an optional schema holds when it holds something: the one alternative beside null, its
/// own nesting left whole.
pub fn non_null<'a>(root: &'a Value, schema: &'a Value) -> &'a Value {
    let schema = resolve(root, schema);
    let Some(alternatives) = schema
        .get("anyOf")
        .or_else(|| schema.get("oneOf"))
        .and_then(Value::as_array)
    else {
        return schema;
    };
    let mut taken = alternatives
        .iter()
        .filter(|alternative| type_names(resolve(root, alternative)) != ["null"]);
    match (taken.next(), taken.next()) {
        (Some(only), None) if alternatives.len() > 1 => resolve(root, only),
        _ => schema,
    }
}

pub fn number_shorthand(root: &Value, field: &Value, number: f64) -> Option<String> {
    if non_null(root, field)["x-unit"] == "%" {
        Some(format!("{number}%"))
    } else if field.get("x-count").is_some() {
        Some(format!("× {number}"))
    } else {
        None
    }
}

/// What the entries of the field `schema` describes are, as its type says in a few words.
pub fn usage(schema: &Value) -> Option<&str> {
    schema.get("x-usage").and_then(Value::as_str)
}

/// Whether the objects `schema` describes hold no field but their tag.
pub fn is_fieldless(schema: &Value) -> bool {
    let tag = tag_of(schema).map(|(field, _)| field);
    properties(schema)
        .iter()
        .all(|(name, _)| tag.as_deref() == Some(name.as_str()))
}

/// The schema of each module of the rows `shape` describes, if they hold modules.
pub fn modules_in(shape: &Value) -> Option<&Value> {
    shape.get("properties")?.get("modules")?.get("items")
}

/// The schema of an object with `properties`, of which those `required` must be present.
pub fn object_of(properties: Map<String, Value>, required: Vec<String>) -> Value {
    json!({ "type": "object", "properties": properties, "required": required })
}

/// The schema of the field `name` of the object `schema` describes.
pub fn property<'a>(schema: &'a Value, name: &str) -> &'a Value {
    static NONE: Value = Value::Null;
    schema
        .get("properties")
        .and_then(|properties| properties.get(name))
        .unwrap_or(&NONE)
}

/// A short line of each field of `value`, the object `shape` describes within `root`: what a
/// folded card or a comparison shows of it.
pub fn lines_of(root: &Value, shape: &Value, value: &Value) -> String {
    let mut lines: Vec<String> = Vec::new();
    for (name, field) in value.as_object().into_iter().flatten() {
        if tag_of(shape).is_some_and(|(tag, _)| tag == *name) {
            continue;
        }
        let field_shape = property(shape, name);
        match field {
            Value::Array(items) => {
                let item_shape = match kind(root, non_null(root, field_shape)) {
                    SchemaKind::Array(items) => resolve(root, items),
                    _ => &Value::Null,
                };
                for item in items {
                    match item.as_object() {
                        Some(_) => lines.push(entry_cells(root, item_shape, item).join("  ")),
                        None => lines.push(item.to_string()),
                    }
                }
            }
            Value::Object(_) => lines.push(format!("{name} …")),
            other => lines.push(format!("{name} {}", cell(root, field_shape, other))),
        }
    }
    lines.join("\n")
}

fn entry_cells(root: &Value, shape: &Value, entry: &Value) -> Vec<String> {
    let fields = properties(settle(root, shape, entry));
    if fields.is_empty() {
        return entry
            .as_object()
            .into_iter()
            .flatten()
            .map(|(_, value)| cell(root, &Value::Null, value))
            .collect();
    }
    fields
        .into_iter()
        .filter_map(|(name, field)| match entry.get(name.as_str()) {
            Some(value) if !value.is_null() => Some(cell(root, field, value)),
            _ if field.get("x-count").is_some() => field
                .get("default")
                .map(|default| cell(root, field, default)),
            _ => field
                .get("x-blank")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
        .collect()
}

fn cell(root: &Value, field: &Value, value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => {
            let number = number.as_f64().unwrap_or(0.0);
            if non_null(root, field)["x-unit"] == "%" {
                format!("{number}%")
            } else {
                number.to_string()
            }
        }
        other => other.to_string(),
    }
}

/// A few words of the fields of `value`, the object `shape` describes within `root`: each text
/// as it is, each number in its unit, each list by its length, and each object `object` can say.
pub fn summary_of(
    root: &Value,
    shape: &Value,
    value: &Value,
    object: impl Fn(&Value) -> Option<String>,
) -> String {
    let mut parts = Vec::new();
    for (name, field) in value.as_object().into_iter().flatten() {
        if tag_of(shape).is_some_and(|(tag, _)| tag == *name) {
            continue;
        }
        match field {
            Value::String(text) => parts.push(text.clone()),
            Value::Number(number) => {
                let number = number.as_f64().unwrap_or(0.0);
                parts.push(
                    number_shorthand(root, property(shape, name), number)
                        .unwrap_or_else(|| format!("{name} {number}")),
                );
            }
            Value::Array(items) => parts.push(format!("{} {name}", items.len())),
            Value::Object(_) => parts.extend(object(field)),
            _ => {}
        }
        if parts.len() >= 3 {
            break;
        }
    }
    parts.join(" · ")
}

pub fn blank_meaning<'a>(schema: &'a Value, value: &Value) -> Vec<&'a str> {
    properties(schema)
        .into_iter()
        .filter(|(name, _)| value.get(name.as_str()).is_none_or(Value::is_null))
        .filter_map(|(_, field)| field.get("x-blank").and_then(Value::as_str))
        .collect()
}

pub fn kind<'a>(root: &'a Value, schema: &'a Value) -> SchemaKind<'a> {
    let schema = resolve(root, schema);
    if let Some(value) = schema.get("const") {
        return SchemaKind::Const(value);
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return SchemaKind::Enum(
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect(),
        );
    }
    let alternatives = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(Value::as_array);
    if let Some(alternatives) = alternatives {
        let constants: Option<Vec<String>> = alternatives
            .iter()
            .map(|alternative| {
                let alternative = resolve(root, alternative);
                Some(alternative.get("const")?.as_str()?.to_owned())
            })
            .collect();
        if let Some(values) = constants {
            return SchemaKind::Enum(values);
        }
        let mut variants = Vec::new();
        let mut plain = Vec::new();
        for alternative in alternatives {
            collect_variants(root, alternative, &mut variants, &mut plain);
        }
        if !variants.is_empty() && plain.is_empty() {
            return SchemaKind::Union(variants);
        }
        if variants.is_empty() {
            return SchemaKind::Choice(plain);
        }
        let mut all = plain;
        all.extend(variants.into_iter().map(|variant| variant.schema));
        return SchemaKind::Choice(all);
    }
    let types = type_names(schema);
    let base = types.iter().find(|name| **name != "null").copied();
    match base {
        Some("object") => {
            if schema.get("properties").is_some() {
                SchemaKind::Object
            } else if let Some(values) = schema.get("additionalProperties")
                && values.is_object()
            {
                SchemaKind::Map(values)
            } else if let Some(patterns) =
                schema.get("patternProperties").and_then(Value::as_object)
                && let Some((_, values)) = patterns.iter().next()
            {
                SchemaKind::Map(values)
            } else {
                SchemaKind::Object
            }
        }
        Some("array") => SchemaKind::Array(schema.get("items").unwrap_or(&Value::Null)),
        Some("string") => SchemaKind::Text,
        Some("integer") => SchemaKind::Integer,
        Some("number") => SchemaKind::Number,
        Some("boolean") => SchemaKind::Boolean,
        Some("null") => SchemaKind::Null,
        _ => SchemaKind::Any,
    }
}

fn collect_variants<'a>(
    root: &'a Value,
    schema: &'a Value,
    variants: &mut Vec<SchemaVariant<'a>>,
    plain: &mut Vec<&'a Value>,
) {
    let schema = resolve(root, schema);
    if let Some(nested) = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(Value::as_array)
        && schema.get("properties").is_none()
        && marker(schema).is_none()
    {
        for alternative in nested {
            collect_variants(root, alternative, variants, plain);
        }
        return;
    }
    if let Some((field, tag)) = tag_of(schema) {
        variants.push(SchemaVariant { field, tag, schema });
    } else {
        plain.push(schema);
    }
}

/// The variants of a union flattened into an object of `schema`, which share its value: each
/// alternative fixes the same field, the tag, to a word of its own.
pub fn flattened_variants<'a>(root: &'a Value, schema: &'a Value) -> Vec<SchemaVariant<'a>> {
    schema.get("properties").map_or(Vec::new(), |_| {
        let Some(alternatives) = schema.get("oneOf").and_then(Value::as_array) else {
            return Vec::new();
        };
        let alternatives: Vec<&Value> = alternatives
            .iter()
            .map(|alternative| resolve(root, alternative))
            .collect();
        let tag_in = |alternative: &Value, field: &str| {
            Some(
                alternative
                    .get("properties")?
                    .get(field)?
                    .get("const")?
                    .as_str()?
                    .to_owned(),
            )
        };
        let Some(field) = alternatives.first().and_then(|first| {
            properties(first).into_iter().find_map(|(field, _)| {
                alternatives
                    .iter()
                    .all(|alternative| tag_in(alternative, field).is_some())
                    .then(|| field.clone())
            })
        }) else {
            return Vec::new();
        };
        alternatives
            .into_iter()
            .filter_map(|schema| {
                Some(SchemaVariant {
                    tag: tag_in(schema, &field)?,
                    field: field.clone(),
                    schema,
                })
            })
            .collect()
    })
}

/// The tagged variants among the alternatives `schema` is, within `root`.
pub fn tagged_variants<'a>(root: &'a Value, schema: &'a Value) -> Vec<SchemaVariant<'a>> {
    match kind(root, schema) {
        SchemaKind::Union(variants) => variants,
        SchemaKind::Choice(alternatives) => alternatives
            .into_iter()
            .filter_map(|schema| {
                let (field, tag) = tag_of(schema)?;
                Some(SchemaVariant { field, tag, schema })
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub fn tag_of(schema: &Value) -> Option<(String, String)> {
    let properties = schema.get("properties")?.as_object()?;
    for field in ["type", "event"] {
        if let Some(tag) = properties.get(field).and_then(|p| p.get("const")?.as_str()) {
            return Some((field.to_owned(), tag.to_owned()));
        }
    }
    None
}

pub fn is_optional(schema: &Value) -> bool {
    type_names(schema).contains(&"null")
        || schema
            .get("anyOf")
            .and_then(Value::as_array)
            .is_some_and(|alternatives| {
                alternatives
                    .iter()
                    .any(|alternative| type_names(alternative) == vec!["null"])
            })
}

pub fn fits(root: &Value, schema: &Value, value: &Value) -> bool {
    let schema = resolve(root, schema);
    if let Some(constant) = schema.get("const") {
        return constant == value;
    }
    if let Some(options) = schema.get("enum").and_then(Value::as_array) {
        return options.contains(value);
    }
    if let Some(alternatives) = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(Value::as_array)
    {
        return alternatives
            .iter()
            .any(|alternative| fits(root, alternative, value));
    }
    let types = type_names(schema);
    if types.is_empty() {
        return true;
    }
    types.iter().any(|name| match *name {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "string" => value.is_string(),
        "array" => value.is_array(),
        "object" => {
            value.is_object()
                && required(schema)
                    .iter()
                    .all(|name| value.get(name).is_some())
                && schema
                    .get("properties")
                    .and_then(Value::as_object)
                    .is_none_or(|properties| {
                        properties
                            .iter()
                            .filter(|(name, _)| value.get(*name).is_some())
                            .all(|(name, field)| fits(root, field, &value[name]))
                    })
        }
        _ => true,
    })
}

pub fn type_names(schema: &Value) -> Vec<&str> {
    match schema.get("type") {
        Some(Value::String(name)) => vec![name.as_str()],
        Some(Value::Array(names)) => names.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

pub fn properties(schema: &Value) -> Vec<(&String, &Value)> {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| properties.iter().collect())
        .unwrap_or_default()
}

pub fn number_paths(root: &Value) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    number_paths_in(root, root, &mut Vec::new(), &mut out, 0);
    out.sort();
    out.dedup();
    out
}

pub fn required(schema: &Value) -> Vec<&str> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// Whether `schema`, a map, keys rows the engine fixes: a sheet lists them, and none is added,
/// removed or renamed.
pub fn fixed_rows(schema: &Value) -> bool {
    schema.get("x-fixed-rows").and_then(Value::as_bool) == Some(true)
}

pub fn description(schema: &Value) -> Option<&str> {
    schema.get("description")?.as_str()
}

/// The game system a node belongs to, as its definition declares it.
pub fn system_of(schema: &Value) -> Option<&str> {
    schema.get("x-system")?.as_str()
}

/// Whether the output a field holds is the one taken when what its node does fails.
pub fn fails(schema: &Value) -> bool {
    schema.get("x-fails").and_then(Value::as_bool) == Some(true)
}

/// Whether the field holds an arrow to another node, or a list of them.
pub fn leads_out(field: &Value) -> bool {
    let is_arrow = |schema: &Value| marker(schema) == Some("x-arrow");
    is_arrow(field) || field.get("items").is_some_and(is_arrow)
}

pub fn marker(schema: &Value) -> Option<&str> {
    schema
        .as_object()?
        .keys()
        .find(|key| key.starts_with("x-"))
        .map(String::as_str)
}

pub fn is_var_fill(schema: &Value) -> bool {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|properties| properties.len() == 1 && properties.contains_key("var"))
}

pub fn table_of(schema: &Value) -> Option<&str> {
    schema.get("x-table").and_then(Value::as_str)
}

/// The table whose files a map's keys name, when they name files.
pub fn key_table(map: &Value) -> Option<&str> {
    map.get("propertyNames").and_then(table_of)
}

/// The kind of name a map's keys are, when they are names a skeleton lists.
pub fn key_names(map: &Value) -> Option<crate::body_names::NameKind> {
    crate::body_names::NameKind::of_schema(map.get("propertyNames")?)
}

pub fn minimum(schema: &Value) -> Option<f64> {
    schema.get("minimum").and_then(Value::as_f64)
}

pub fn maximum(schema: &Value) -> Option<f64> {
    schema.get("maximum").and_then(Value::as_f64)
}

/// Where a fresh value is written, so a template names content and media that is there: the file
/// it goes in, and the files of the game around it.
#[derive(Clone, Copy)]
pub struct Nearby<'a> {
    pub files: &'a ProjectFiles,
    /// The file the value is written into, when it is known yet.
    pub from: Option<&'a str>,
    /// The game as it last loaded, whose maps say where a fresh place may stand.
    pub loaded: Option<&'a Content>,
    /// What the script written into keeps by name, when the value is written into a script.
    pub kept: Option<&'a KeptValues>,
}

/// The values a script names, each with its type: the locals that last one flow, the state its
/// runner shares across flows, and the variables each runner gives it.
#[derive(Clone, Default)]
pub struct KeptValues {
    pub locals: Vec<(String, ValueType)>,
    pub state: Vec<(String, ValueType)>,
    pub variables: Vec<(String, VariableType)>,
}

impl KeptValues {
    pub fn of(script: &ScriptDef) -> KeptValues {
        let listed = |kept: &BTreeMap<String, ValueType>| {
            kept.iter().map(|(name, ty)| (name.clone(), *ty)).collect()
        };
        KeptValues {
            locals: listed(&script.locals),
            state: listed(&script.state),
            variables: script
                .variables
                .iter()
                .map(|(name, variable)| (name.clone(), variable.ty))
                .collect(),
        }
    }

    /// The first kept value, a local before state, as a target an assignment writes.
    pub fn first(&self) -> Option<(Target, ValueType)> {
        let local = self
            .locals
            .first()
            .map(|(name, ty)| (Target::Local(name.clone()), *ty));
        local.or_else(|| {
            self.state
                .first()
                .map(|(name, ty)| (Target::State(name.clone()), *ty))
        })
    }
}

impl<'a> Nearby<'a> {
    /// Values written into `from` in `project`.
    pub fn of(project: &'a Project, from: Option<&'a str>) -> Nearby<'a> {
        Nearby {
            files: &project.files,
            from,
            loaded: project.content.as_ref(),
            kept: None,
        }
    }

    /// Values made for no game, naming nothing: what a catalogue of every kind of value reads.
    pub fn nowhere() -> Nearby<'static> {
        static NONE: LazyLock<ProjectFiles> = LazyLock::new(ProjectFiles::default);
        Nearby {
            files: &NONE,
            from: None,
            loaded: None,
            kept: None,
        }
    }

    /// A place on the walkable cell nearest the corner of the area written into, or else of the
    /// area nearest the file written into.
    fn place(&self) -> Option<Value> {
        let content = self.loaded?;
        let area = self
            .from
            .and_then(|from| content.by_path::<AreaDef>(from))
            .or_else(|| content.by_path::<AreaDef>(&self.content(AreaDef::TABLE)?))?;
        let at = area::load(content, area).mesh.nearest(Pos::new(0.0, 0.0))?;
        let place = Place { area, at };
        with_content(ContentScope::of(content), || serde_json::to_value(place))
            .0
            .ok()
    }

    /// `place` moved to the walkable cell of its area nearest where it stands, as a place whose
    /// area changed starts.
    pub fn walkable(&self, place: &Value) -> Option<Value> {
        let content = self.loaded?;
        let scope = || ContentScope::of(content);
        let mut place: Place = with_content(scope(), || serde_json::from_value(place.clone()))
            .0
            .ok()?;
        place.at = area::load(content, place.area).mesh.nearest(place.at)?;
        with_content(scope(), || serde_json::to_value(place)).0.ok()
    }

    /// The file of `table` nearest the file written into, by how far up their paths meet.
    pub fn content(&self, table: &str) -> Option<String> {
        let from = self.from.map(fields_file);
        self.nearest(
            self.files
                .index()
                .paths_of(table)
                .filter(|path| Some(fields_file(path)) != from),
        )
    }

    /// An assignment of the first value the script keeps, to a value of its type: what a set
    /// node writes first.
    fn assignment(&self) -> Option<Value> {
        let (target, ty) = self.kept?.first()?;
        let to = match ty {
            ValueType::Boolean => Condition::Bool(false),
            ValueType::Integer => Condition::Int(0),
            ValueType::Number => Condition::Num(0.0),
            ValueType::Text => Condition::Text(String::new()),
            ValueType::Actor => Condition::Query(RiftQuery::Subject),
            ValueType::Actors | ValueType::Point | ValueType::Area => match &target {
                Target::Local(name) => Condition::Local {
                    local: name.clone(),
                },
                Target::State(name) => Condition::State {
                    state: name.clone(),
                },
            },
        };
        serde_json::to_value(Assignment { target, to }).ok()
    }

    /// The script nearest the file written into that a script run by `kind` may run: one for that
    /// kind, or for any.
    pub fn script_run_by(&self, kind: RunnerKind) -> Option<String> {
        let content = self.loaded?;
        let scripts = content.table::<ScriptDef>();
        let from = self.from.map(fields_file);
        let index = self.files.index();
        self.nearest(index.paths_of(ScriptDef::TABLE).filter(|path| {
            Some(fields_file(path)) != from
                && content.by_path::<ScriptDef>(path).is_some_and(|id| {
                    matches!(scripts.row(id).runs_on, RunnerKind::Any)
                        || scripts.row(id).runs_on == kind
                })
        }))
    }

    /// The media file of `kind` nearest the file written into.
    pub fn media(&self, kind: AssetKind) -> Option<String> {
        let index = self.files.index();
        let of_kind = index
            .media
            .iter()
            .map(String::as_str)
            .filter(|path| AssetKind::of(path) == Some(kind));
        self.nearest(of_kind)
    }

    fn nearest<'p>(&self, paths: impl Iterator<Item = &'p str>) -> Option<String> {
        paths
            .min_by_key(|path| (crate::forms::seen_from(self.from, path).0, *path))
            .map(str::to_owned)
    }
}

/// A fresh value of `schema`: its default, or the least value of its shape, naming the content and
/// media nearest where it is written wherever it names some.
pub fn template(root: &Value, schema: &Value, nearby: Nearby) -> Value {
    if let Some(default) = schema.get("default") {
        return default.clone();
    }
    let fresh = match marker(resolve(root, schema)) {
        Some("x-place") => nearby.place(),
        Some("x-assignment") => nearby.assignment(),
        _ => None,
    };
    if let Some(fresh) = fresh {
        return fresh;
    }
    let least = minimum(schema);
    let schema = resolve(root, schema);
    let least = least.or_else(|| minimum(schema)).unwrap_or(0.0);
    match kind(root, schema) {
        SchemaKind::Const(value) => value.clone(),
        SchemaKind::Enum(values) => Value::String(values.first().cloned().unwrap_or_default()),
        SchemaKind::Union(variants) => variants
            .first()
            .map(|variant| variant_template(root, variant, nearby))
            .unwrap_or(Value::Null),
        SchemaKind::Choice(alternatives) => alternatives
            .iter()
            .find(|alternative| {
                !type_names(alternative).contains(&"null") && !is_var_fill(alternative)
            })
            .or_else(|| {
                alternatives
                    .iter()
                    .find(|alternative| !type_names(alternative).contains(&"null"))
            })
            .map(|alternative| template(root, alternative, nearby))
            .unwrap_or(Value::Null),
        SchemaKind::Object => {
            let required = required(schema);
            let mut object = Map::new();
            for (name, field) in properties(schema) {
                if required.contains(&name.as_str()) {
                    object.insert(name.clone(), template(root, field, nearby));
                }
            }
            Value::Object(object)
        }
        SchemaKind::Array(items) => {
            let least = schema["minItems"].as_u64().unwrap_or(0);
            Value::Array((0..least).map(|_| template(root, items, nearby)).collect())
        }
        SchemaKind::Map(_) => Value::Object(Map::new()),
        SchemaKind::Text => Value::String(named(schema, nearby).unwrap_or_default()),
        SchemaKind::Integer => Value::from(least as i64),
        SchemaKind::Number => Value::from(least),
        SchemaKind::Boolean => Value::Bool(false),
        SchemaKind::Null | SchemaKind::Any => Value::Null,
    }
}

pub fn variant_template(root: &Value, variant: &SchemaVariant, nearby: Nearby) -> Value {
    let mut value = template(root, variant.schema, nearby);
    if let Some(object) = value.as_object_mut() {
        object.insert(variant.field.clone(), Value::String(variant.tag.clone()));
    }
    value
}

/// The content or media file nearest where a text of `schema` is written, when it names one.
fn named(schema: &Value, nearby: Nearby) -> Option<String> {
    if let Some(table) = table_of(schema) {
        return nearby.content(table);
    }
    let kind = schema.get("x-asset").and_then(Value::as_str)?;
    nearby.media(kind.parse().ok()?)
}

pub fn variant_of<'a>(
    variants: &'a [SchemaVariant<'a>],
    value: &Value,
) -> Option<&'a SchemaVariant<'a>> {
    variants.iter().find(|variant| {
        value
            .get(&variant.field)
            .and_then(Value::as_str)
            .is_some_and(|tag| tag == variant.tag)
    })
}

pub fn variable_schema(ty: VariableType, table: Option<&str>) -> Value {
    match ty {
        VariableType::Integer => json!({ "type": "integer" }),
        VariableType::Number => json!({ "type": "number" }),
        VariableType::Boolean => json!({ "type": "boolean" }),
        VariableType::Text | VariableType::Name => json!({ "type": "string" }),
        VariableType::Row => json!({ "type": "string", "x-table": table.unwrap_or_default() }),
        VariableType::Audio => schema_of::<AudioRef>(),
        VariableType::Place => schema_of::<Place>(),
        VariableType::Condition => json!({ "x-condition": true }),
    }
}

pub fn title(name: &str) -> String {
    let mut out = String::new();
    for (index, part) in name.split('_').enumerate() {
        if index > 0 {
            out.push(' ');
        }
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            if index == 0 {
                out.extend(first.to_uppercase());
            } else {
                out.push(first);
            }
            out.push_str(chars.as_str());
        }
    }
    out
}

const NUMBER_DEPTH: usize = 5;

fn number_paths_in(
    root: &Value,
    schema: &Value,
    path: &mut Vec<String>,
    out: &mut Vec<Vec<String>>,
    depth: usize,
) {
    if depth > NUMBER_DEPTH {
        return;
    }
    let schema = resolve(root, schema);
    if type_names(schema)
        .iter()
        .any(|name| matches!(*name, "number" | "integer"))
    {
        out.push(path.clone());
        return;
    }
    if let Some(items) = schema.get("items") {
        number_paths_in(root, items, path, out, depth + 1);
    }
    for (name, field) in properties(schema) {
        if name == "modules" || name == "type" {
            continue;
        }
        path.push(name.clone());
        number_paths_in(root, field, path, out, depth + 1);
        path.pop();
    }
    for variant in ["oneOf", "anyOf"]
        .iter()
        .filter_map(|key| schema.get(*key))
        .filter_map(Value::as_array)
        .flatten()
    {
        let variant = resolve(root, variant);
        match tag_of(variant) {
            Some((_, tag)) => {
                path.push(tag);
                number_paths_in(root, variant, path, out, depth + 1);
                path.pop();
            }
            None => number_paths_in(root, variant, path, out, depth + 1),
        }
    }
    if let Some(modules) = schema
        .get("properties")
        .and_then(|properties| properties.get("modules"))
    {
        number_paths_in(root, modules, path, out, depth + 1);
    }
}

/// The schema of `T` as the game reads it, made once per type.
pub fn of_type<T: schemars::JsonSchema + 'static>() -> &'static Value {
    static MADE: OnceLock<Mutex<HashMap<TypeId, &'static Value>>> = OnceLock::new();
    let mut made = MADE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    made.entry(TypeId::of::<T>())
        .or_insert_with(|| Box::leak(Box::new(schema_of::<T>())))
}

fn schema_of<T: schemars::JsonSchema>() -> Value {
    let schema = schemars::generate::SchemaSettings::draft2020_12()
        .with(|settings| settings.inline_subschemas = true)
        .into_generator()
        .into_root_schema_for::<T>();
    serde_json::to_value(schema).expect("a schema is json")
}

/// A field of a value a blueprint can leave to a variable: where it is in the value, the type of
/// variable that fills it, and the table it names when it names a file.
pub struct HoleableField {
    pub pointer: String,
    pub kind: VariableType,
    pub table: Option<String>,
}

/// Every field of `value`, a value of the type whose schema is `root`, a variable could fill.
pub fn holeable_fields(root: &Value, value: &Value) -> Vec<HoleableField> {
    let mut out = Vec::new();
    collect_holeable(root, root, value, "", &mut out);
    out
}

/// A field of a script node that holds its value or a variable of the script, as `Fill` does.
pub struct VarField {
    pub field: HoleableField,
    /// The schema of a value it holds in place of a variable.
    pub fixed: Value,
}

/// Every field of `value`, a value of the type whose schema is `schema` within `root`, that holds
/// a value or the name of one of the script's variables.
pub fn var_fields(root: &Value, schema: &Value, value: &Value) -> Vec<VarField> {
    let mut out = Vec::new();
    collect_var_fields(root, schema, value, "", &mut out);
    out
}

fn collect_var_fields(
    root: &Value,
    schema: &Value,
    value: &Value,
    pointer: &str,
    out: &mut Vec<VarField>,
) {
    let resolved = resolve(root, schema);
    let schema = match kind(root, resolved) {
        SchemaKind::Choice(alternatives)
            if alternatives
                .iter()
                .any(|alternative| is_var_fill(alternative)) =>
        {
            resolved
        }
        _ => settle(root, schema, value),
    };
    match kind(root, schema) {
        SchemaKind::Choice(alternatives)
            if alternatives
                .iter()
                .any(|alternative| is_var_fill(alternative)) =>
        {
            let Some(fixed) = alternatives.iter().find(|alternative| {
                !is_var_fill(alternative) && type_names(alternative) != ["null"]
            }) else {
                return;
            };
            let fixed = resolve(root, fixed);
            let mut kinds = Vec::new();
            collect_holeable(root, fixed, &Value::Null, pointer, &mut kinds);
            if let Some(field) = kinds.pop() {
                out.push(VarField {
                    field,
                    fixed: fixed.clone(),
                });
            }
        }
        SchemaKind::Object => {
            for (name, field) in properties(schema) {
                if let Some(child) = value.get(name) {
                    let at = format!("{pointer}/{}", locate::escape(name));
                    collect_var_fields(root, field, child, &at, out);
                }
            }
        }
        SchemaKind::Array(items) => {
            for (index, child) in value.as_array().into_iter().flatten().enumerate() {
                collect_var_fields(root, items, child, &format!("{pointer}/{index}"), out);
            }
        }
        _ => {}
    }
}

fn collect_holeable(
    root: &Value,
    schema: &Value,
    value: &Value,
    pointer: &str,
    out: &mut Vec<HoleableField>,
) {
    let schema = settle(root, schema, value);
    let kind = match marker(schema) {
        Some("x-name") => Some(VariableType::Name),
        Some("x-condition") => Some(VariableType::Condition),
        Some("x-place") => Some(VariableType::Place),
        Some("x-table") => Some(VariableType::Row),
        Some("x-asset") => (schema["x-asset"] == "audio").then_some(VariableType::Audio),
        Some(_) => return,
        None => match kind(root, schema) {
            SchemaKind::Text => Some(VariableType::Text),
            SchemaKind::Integer => Some(VariableType::Integer),
            SchemaKind::Number => Some(VariableType::Number),
            SchemaKind::Boolean => Some(VariableType::Boolean),
            SchemaKind::Choice(_) | SchemaKind::Any => Some(value_kind(value)),
            SchemaKind::Object => {
                for (name, field) in properties(schema) {
                    if let Some(child) = value.get(name) {
                        let at = format!("{pointer}/{}", locate::escape(name));
                        collect_holeable(root, field, child, &at, out);
                    }
                }
                return;
            }
            SchemaKind::Array(items) => {
                for (index, child) in value.as_array().into_iter().flatten().enumerate() {
                    collect_holeable(root, items, child, &format!("{pointer}/{index}"), out);
                }
                return;
            }
            SchemaKind::Map(values) => {
                for (name, child) in value.as_object().into_iter().flatten() {
                    let at = format!("{pointer}/{}", locate::escape(name));
                    collect_holeable(root, values, child, &at, out);
                }
                return;
            }
            _ => None,
        },
    };
    if let Some(kind) = kind
        && !pointer.is_empty()
    {
        out.push(HoleableField {
            pointer: pointer.to_owned(),
            kind,
            table: table_of(schema).map(str::to_owned),
        });
    }
}

/// The type of variable a value of no one schema reads as, by what it holds now.
fn value_kind(value: &Value) -> VariableType {
    match value {
        Value::Number(n) if n.is_i64() => VariableType::Integer,
        Value::Number(_) => VariableType::Number,
        Value::Bool(_) => VariableType::Boolean,
        Value::Object(o) if o.contains_key("area") => VariableType::Place,
        Value::Object(_) => VariableType::Condition,
        _ => VariableType::Text,
    }
}
