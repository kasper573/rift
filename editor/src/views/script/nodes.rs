use std::collections::{BTreeMap, HashSet, VecDeque};

use engine::core::content::{Content, ContentScope, with_content};
use engine::core::script::{EffectDef, EventDef, OwnerKindDef};
use engine::systems::actor::model::Face;
use engine::systems::area::blueprint::hole_name;
use engine::systems::npc::NpcDef;
use engine::systems::script::{Node, OwnerKind};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use strum::{EnumString, IntoStaticStr, VariantArray};

use crate::project::{ProjectFiles, row_of};
use crate::schema::{self, SchemaVariant};
use crate::ui::widgets;
use crate::value::RowValue;
use crate::{condition, spans};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
pub enum FlowKind {
    On,
    If,
    Random,
    Repeat,
    Wait,
    Set,
    Run,
    Return,
    Dialogue,
}

impl FlowKind {
    pub fn of(node: &Value) -> Option<FlowKind> {
        node.kind().parse().ok()
    }

    pub fn name(self) -> &'static str {
        self.into()
    }

    pub fn arrows(self) -> &'static [&'static str] {
        match self {
            FlowKind::On | FlowKind::Wait | FlowKind::Set | FlowKind::Run => &["next"],
            FlowKind::If => &["yes", "no"],
            FlowKind::Repeat => &["each", "then"],
            FlowKind::Random => &["outputs"],
            FlowKind::Dialogue => &["choices", "next", "left"],
            FlowKind::Return => &[],
        }
    }

    pub fn pauses(self) -> bool {
        matches!(self, FlowKind::Wait | FlowKind::Dialogue)
    }

    pub fn system(self) -> &'static str {
        match self {
            FlowKind::Dialogue => "dialogue",
            _ => "flow",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            FlowKind::On => "Starts a flow at an event; its cause is the subject",
            FlowKind::If => "Takes yes when the test holds, else no",
            FlowKind::Random => "Picks an output by weight",
            FlowKind::Repeat => "Runs each a number of times, then goes on",
            FlowKind::Wait => "Pauses for seconds, or one tick at 0",
            FlowKind::Set => "Writes locals and owner state in order",
            FlowKind::Run => "Calls another script and goes on when it ends",
            FlowKind::Return => "Ends a called script, handing back a value",
            FlowKind::Dialogue => "Lines and choices spoken to the subject",
        }
    }
}

pub fn is_dialogue(node: &Value) -> bool {
    FlowKind::of(node) == Some(FlowKind::Dialogue)
}

pub fn is_event(node: &Value) -> bool {
    FlowKind::of(node) == Some(FlowKind::On)
}

pub fn hint(node: &Value) -> &'static str {
    match FlowKind::of(node) {
        Some(FlowKind::Dialogue) => {
            "Enter adds a line · Ctrl Enter a choice · select text to style it"
        }
        Some(FlowKind::If) => "each slot takes one type · the same builder edits every condition",
        Some(FlowKind::Repeat | FlowKind::Wait | FlowKind::Random) => {
            "a loop may come back only through a node that pauses"
        }
        Some(FlowKind::Run) => "calls nest 8 deep and never loop back",
        Some(FlowKind::On) => "a flow starts here; its cause is the subject",
        Some(FlowKind::Set | FlowKind::Return) | None => match node.kind() {
            "remember" | "forget" => "two arrows may lead into one node",
            _ => "an output without an arrow ends the flow",
        },
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LineSpeaker {
    Npc {
        npc: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        face: Option<String>,
    },
    Object,
    Narrator,
    Player,
}

impl LineSpeaker {
    pub fn of(by: &Value) -> Option<LineSpeaker> {
        serde_json::from_value(by.clone()).ok()
    }

    pub fn named(kind: &str, npc: &str) -> LineSpeaker {
        match kind {
            "npc" => LineSpeaker::Npc {
                npc: npc.to_owned(),
                face: None,
            },
            "object" => LineSpeaker::Object,
            "player" => LineSpeaker::Player,
            _ => LineSpeaker::Narrator,
        }
    }

    pub fn options(owner: OwnerKind) -> &'static [&'static str] {
        match owner {
            OwnerKind::Object => &["npc", "object", "narrator"],
            _ => &["npc", "narrator"],
        }
    }

    pub fn json(&self) -> Value {
        serde_json::to_value(self).expect("a speaker serialises")
    }

    pub fn blank_line(&self) -> Value {
        serde_json::json!({ "by": self.json(), "text": [""] })
    }

    pub fn kind(&self) -> &'static str {
        match self {
            LineSpeaker::Npc { .. } => "npc",
            LineSpeaker::Object => "object",
            LineSpeaker::Narrator => "narrator",
            LineSpeaker::Player => "player",
        }
    }

    pub fn npc(&self) -> Option<&str> {
        match self {
            LineSpeaker::Npc { npc, .. } => Some(npc),
            _ => None,
        }
    }

    pub fn faceless(&self) -> LineSpeaker {
        match self {
            LineSpeaker::Npc { npc, .. } => LineSpeaker::Npc {
                npc: npc.clone(),
                face: None,
            },
            other => other.clone(),
        }
    }

    pub fn name(&self) -> String {
        match self {
            LineSpeaker::Npc { npc, .. } => npc.clone(),
            other => other.kind().to_owned(),
        }
    }
}

pub fn speaker_name(by: &Value) -> String {
    LineSpeaker::of(by).map_or_else(|| "?".to_owned(), |speaker| speaker.name())
}

pub fn bust_file(content: Option<&Content>, speaker: &LineSpeaker) -> Option<String> {
    let content = content?;
    let LineSpeaker::Npc { npc, face } = speaker else {
        return None;
    };
    let busts = content
        .by_key::<NpcDef>(npc)?
        .get(content)
        .model
        .get(content)
        .busts()?;
    let face = face
        .as_deref()
        .map(|face| serde_json::from_value::<Face>(Value::String(face.to_owned())))
        .transpose()
        .ok()?;
    let file = match face {
        Some(face) => busts.face(&face)?,
        None => &busts.generic.neutral,
    };
    Some(file.as_str().to_owned())
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodePort {
    pub label: String,
    pub field: String,
    pub target: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NodeEntry {
    pub kind: String,
    pub system: &'static str,
    pub blurb: &'static str,
    pub owners: Vec<OwnerKind>,
    pub outputs: Vec<String>,
    pub fields: Vec<(String, Option<String>)>,
    pub pauses: bool,
}

impl NodeEntry {
    pub fn fits(&self, linking: bool) -> bool {
        (self.kind == FlowKind::On.name()) != linking
    }

    pub fn badge(&self) -> &'static str {
        match self.system {
            "flow" | "dialogue" => "script",
            "memory" => "quest",
            "area" => "object",
            "npc body" | "notification" | "chat" | "actor" => "npc",
            "audio" => "sfx",
            "vfx" => "skill",
            "stat" | "class" => "respawn",
            other => other,
        }
    }
}

pub struct OwnerRefusal {
    pub owner: OwnerKind,
    pub allowed: Vec<OwnerKind>,
}

impl OwnerRefusal {
    pub fn text(&self) -> String {
        let names: Vec<&str> = self.allowed.iter().map(|kind| kind.name()).collect();
        format!(
            "not for {} owners: only {}",
            self.owner.name(),
            names.join(", ")
        )
    }

    pub fn only_for(&self) -> String {
        match self.allowed.first() {
            Some(first) => format!(
                "only for {} {} owner",
                widgets::article(first.name()),
                first.name()
            ),
            None => self.text(),
        }
    }
}

#[derive(Default)]
pub struct NodeCatalogue {
    pub entries: Vec<NodeEntry>,
    pub events: Vec<(String, Vec<OwnerKind>)>,
}

impl NodeCatalogue {
    pub fn build(root: &Value, content: Option<&Content>) -> NodeCatalogue {
        let mut entries: Vec<NodeEntry> = FlowKind::VARIANTS
            .iter()
            .map(|&kind| NodeEntry {
                kind: kind.name().to_owned(),
                system: kind.system(),
                blurb: kind.blurb(),
                owners: Vec::new(),
                outputs: kind.arrows().iter().map(|&s| s.to_owned()).collect(),
                fields: Vec::new(),
                pauses: kind.pauses(),
            })
            .collect();
        let scope = content.map(ContentScope::of);
        let parse = |template: Value| -> Option<Node> {
            let scope = scope.clone()?;
            let (parsed, _) = with_content(scope, || serde_json::from_value::<Node>(template));
            parsed.ok()
        };
        for variant in variants(root) {
            if variant.tag.parse::<FlowKind>().is_ok() {
                continue;
            }
            let outputs: Vec<String> = schema::properties(variant.schema)
                .into_iter()
                .filter(|(name, field)| *name != "type" && is_arrow(field))
                .map(|(name, _)| name.clone())
                .collect();
            let fields: Vec<(String, Option<String>)> = schema::properties(variant.schema)
                .into_iter()
                .filter(|(name, field)| *name != "type" && !is_arrow(field))
                .map(|(name, field)| {
                    (
                        name.clone(),
                        schema::table_of(schema::resolve(root, field)).map(str::to_owned),
                    )
                })
                .collect();
            let (owners, pauses) = match parse(schema::variant_template(root, &variant)) {
                Some(Node::Effect(effect)) => (effect.owners().to_vec(), effect.pauses()),
                _ => (Vec::new(), false),
            };
            let (system, blurb) = EFFECT_BLURBS
                .iter()
                .find(|(kind, _, _)| *kind == variant.tag)
                .map_or(("other", ""), |(_, system, blurb)| (*system, *blurb));
            entries.push(NodeEntry {
                kind: variant.tag.clone(),
                system,
                blurb,
                owners,
                outputs,
                fields,
                pauses,
            });
        }
        let events = event_variants(root)
            .into_iter()
            .map(|variant| {
                let mut template = schema::variant_template(root, &variant);
                template["type"] = Value::String(FlowKind::On.name().to_owned());
                let owners = match parse(template) {
                    Some(Node::On { event, .. }) => event.owners().to_vec(),
                    _ => Vec::new(),
                };
                (variant.tag.clone(), owners)
            })
            .collect();
        NodeCatalogue { entries, events }
    }

    pub fn entry(&self, kind: &str) -> Option<&NodeEntry> {
        self.entries.iter().find(|entry| entry.kind == kind)
    }

    pub fn refusal(&self, kind: &str, owner: OwnerKind) -> Option<OwnerRefusal> {
        allowed(&self.entry(kind)?.owners, owner)
    }

    pub fn event_refusal(&self, event: &str, owner: OwnerKind) -> Option<OwnerRefusal> {
        let (_, owners) = self.events.iter().find(|(name, _)| name == event)?;
        allowed(owners, owner)
    }
}

fn allowed(owners: &[OwnerKind], owner: OwnerKind) -> Option<OwnerRefusal> {
    if owners.is_empty()
        || owner == OwnerKind::Any
        || owners.contains(&owner)
        || owners.contains(&OwnerKind::Any)
    {
        return None;
    }
    Some(OwnerRefusal {
        owner,
        allowed: owners.to_vec(),
    })
}

pub fn has_input(node: &Value) -> bool {
    !is_event(node)
}

pub fn ports(node: &Value, catalogue: &NodeCatalogue) -> Vec<NodePort> {
    let arrow = |field: &str| -> NodePort {
        NodePort {
            label: field.to_owned(),
            field: format!("/{field}"),
            target: node[field].as_str().map(str::to_owned),
        }
    };
    match FlowKind::of(node) {
        Some(FlowKind::Random) => {
            let weights = node["weights"].as_array().cloned().unwrap_or_default();
            let outputs = node["outputs"].as_array().cloned().unwrap_or_default();
            (0..weights.len().max(outputs.len()))
                .map(|index| NodePort {
                    label: weights
                        .get(index)
                        .and_then(Value::as_u64)
                        .map_or_else(|| format!("{}", index + 1), |w| w.to_string()),
                    field: format!("/outputs/{index}"),
                    target: outputs
                        .get(index)
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                })
                .collect()
        }
        Some(FlowKind::Dialogue) => {
            let choices = node["choices"].as_array().cloned().unwrap_or_default();
            let mut ports: Vec<NodePort> = choices
                .iter()
                .enumerate()
                .map(|(index, choice)| NodePort {
                    label: widgets::clip(&spans::plain(&choice["label"]), 22),
                    field: format!("/choices/{index}/to"),
                    target: choice["to"].as_str().map(str::to_owned),
                })
                .collect();
            if choices.is_empty() {
                ports.push(arrow("next"));
            }
            ports.push(arrow("left"));
            ports
        }
        Some(flow) => flow.arrows().iter().map(|field| arrow(field)).collect(),
        None => catalogue
            .entry(node.kind())
            .map(|entry| entry.outputs.iter().map(|name| arrow(name)).collect())
            .unwrap_or_default(),
    }
}

pub fn set_arrow(node: &mut Value, field: &str, target: Option<&str>) {
    let (parent, last) = match field.rfind('/') {
        Some(at) => (&field[..at], &field[at + 1..]),
        None => ("", field),
    };
    let Some(holder) = (if parent.is_empty() {
        Some(&mut *node)
    } else {
        node.pointer_mut(parent)
    }) else {
        return;
    };
    match holder {
        Value::Object(map) => match target {
            Some(target) => {
                map.insert(last.to_owned(), Value::String(target.to_owned()));
            }
            None => {
                map.remove(last);
            }
        },
        Value::Array(list) => {
            if let Ok(index) = last.parse::<usize>() {
                while list.len() <= index {
                    list.push(Value::Null);
                }
                list[index] = target.map_or(Value::Null, |t| Value::String(t.to_owned()));
            }
        }
        _ => {}
    }
}

pub fn arrows_into(
    nodes: &Map<String, Value>,
    target: &str,
    catalogue: &NodeCatalogue,
) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for (name, node) in nodes {
        for port in ports(node, catalogue) {
            if port.target.as_deref() == Some(target) {
                found.push((name.clone(), port.field));
            }
        }
    }
    found
}

pub fn unlink(nodes: &mut Map<String, Value>, target: &str, catalogue: &NodeCatalogue) {
    for (from, field) in arrows_into(nodes, target, catalogue) {
        if let Some(node) = nodes.get_mut(&from) {
            set_arrow(node, &field, None);
        }
    }
}

pub fn port_label(
    nodes: &Map<String, Value>,
    catalogue: &NodeCatalogue,
    from: &str,
    field: &str,
) -> String {
    nodes
        .get(from)
        .and_then(|node| {
            ports(node, catalogue)
                .into_iter()
                .find(|port| port.field == field)
        })
        .map(|port| port.label)
        .unwrap_or_else(|| field.rsplit('/').next().unwrap_or("").to_owned())
}

pub fn reachable(nodes: &Map<String, Value>, catalogue: &NodeCatalogue) -> HashSet<String> {
    depths(nodes, catalogue).into_keys().collect()
}

pub fn depths(nodes: &Map<String, Value>, catalogue: &NodeCatalogue) -> BTreeMap<String, usize> {
    let mut depths: BTreeMap<String, usize> = BTreeMap::new();
    let mut queue: VecDeque<(String, usize)> = nodes
        .iter()
        .filter(|(_, node)| is_event(node))
        .map(|(name, _)| (name.clone(), 0))
        .collect();
    while let Some((name, depth)) = queue.pop_front() {
        if depths.contains_key(&name) {
            continue;
        }
        depths.insert(name.clone(), depth);
        if let Some(node) = nodes.get(&name) {
            for port in ports(node, catalogue) {
                if let Some(target) = port.target
                    && !depths.contains_key(&target)
                {
                    queue.push_back((target, depth + 1));
                }
            }
        }
    }
    depths
}

pub fn insert_sorted(nodes: &mut Map<String, Value>, name: &str, node: Value) {
    nodes.insert(name.to_owned(), node);
    nodes.sort_keys();
}

pub fn rename(nodes: &mut Map<String, Value>, old: &str, new: &str, catalogue: &NodeCatalogue) {
    let Some(node) = nodes.remove(old) else {
        return;
    };
    insert_sorted(nodes, new, node);
    for (from, field) in arrows_into(nodes, old, catalogue) {
        if let Some(node) = nodes.get_mut(&from) {
            set_arrow(node, &field, Some(new));
        }
    }
}

pub fn free_name(nodes: &Map<String, Value>, base: &str) -> String {
    let base = base
        .trim()
        .replace(|c: char| !c.is_ascii_alphanumeric() && c != '_', "_");
    let base = if base.is_empty() {
        "node".to_owned()
    } else {
        base
    };
    if !nodes.contains_key(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|name| !nodes.contains_key(name))
        .expect("a free suffix")
}

pub fn template(root: &Value, kind: &str) -> Option<Value> {
    let flow = kind.parse::<FlowKind>().ok();
    if flow == Some(FlowKind::On) {
        return template_event(root, "started");
    }
    let variants = variants(root);
    let variant = variants.iter().find(|variant| variant.tag == kind)?;
    let mut value = schema::variant_template(root, variant);
    match flow {
        Some(FlowKind::Dialogue) => {
            value["lines"] = serde_json::json!([LineSpeaker::Narrator.blank_line()]);
        }
        Some(FlowKind::Random) => {
            value["weights"] = serde_json::json!([1, 1]);
            value["outputs"] = serde_json::json!([null, null]);
        }
        Some(FlowKind::Repeat) => {
            value["times"] = Value::from(2);
            value["at_most"] = Value::from(10);
        }
        Some(FlowKind::Wait) => {
            value["seconds"] = Value::from(1.0);
            value["at_most"] = Value::from(60.0);
        }
        Some(FlowKind::If) => value["test"] = Value::Bool(true),
        _ => {}
    }
    Some(value)
}

pub fn fill_references(root: &Value, schema: &Value, value: &mut Value, files: &ProjectFiles) {
    let schema = schema::resolve(root, schema);
    if let Some(table) = schema::table_of(schema) {
        if value.as_str().is_some_and(str::is_empty)
            && let Some(key) = files
                .paths_under(&format!("tables/{table}/"))
                .filter_map(|file| row_of(file).map(|(_, key)| key.to_owned()))
                .min()
        {
            *value = Value::String(key);
        }
        return;
    }
    match schema::kind(root, schema) {
        schema::SchemaKind::Object => {
            for (name, field) in schema::properties(schema) {
                if let Some(inner) = value.get_mut(name) {
                    fill_references(root, field, inner, files);
                }
            }
        }
        schema::SchemaKind::Choice(alternatives) => {
            if let Some(alternative) = alternatives
                .iter()
                .find(|alternative| schema::fits(root, alternative, value))
            {
                fill_references(root, alternative, value, files);
            }
        }
        schema::SchemaKind::Union(variants) => {
            if let Some(variant) = schema::variant_of(&variants, value) {
                fill_references(root, variant.schema, value, files);
            }
        }
        _ => {}
    }
}

pub fn template_event(root: &Value, event: &str) -> Option<Value> {
    let variant = event_variants(root)
        .into_iter()
        .find(|variant| variant.tag == event)?;
    let mut node = schema::variant_template(root, &variant);
    node["type"] = Value::String(FlowKind::On.name().to_owned());
    if event == "interacted" {
        node["verb"] = Value::String("talk".to_owned());
        node["reach"] = Value::from(1.5);
    }
    if event == "came_near" || event == "heard" {
        node["within"] = Value::from(4.0);
    }
    Some(node)
}

pub fn event_of(node: &Value) -> Option<&str> {
    node["event"].as_str()
}

pub fn node_schema(root: &Value) -> Option<&Value> {
    root.get("properties")?
        .get("nodes")?
        .get("additionalProperties")
}

pub fn variants(root: &Value) -> Vec<SchemaVariant<'_>> {
    let Some(schema) = node_schema(root) else {
        return Vec::new();
    };
    match schema::kind(root, schema) {
        schema::SchemaKind::Union(variants) => variants,
        _ => Vec::new(),
    }
}

pub fn variant_schema<'a>(root: &'a Value, kind: &str) -> Option<&'a Value> {
    variants(root)
        .into_iter()
        .find(|variant| variant.tag == kind)
        .map(|variant| variant.schema)
}

pub fn event_variants(root: &Value) -> Vec<SchemaVariant<'_>> {
    let Some(on) = variant_schema(root, FlowKind::On.name()) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for alternative in on
        .get("oneOf")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some((field, tag)) = schema::tag_of(alternative)
            && field == "event"
        {
            found.push(SchemaVariant {
                field,
                tag,
                schema: alternative,
            });
        }
    }
    found
}

pub fn event_variant<'a>(root: &'a Value, event: &str) -> Option<&'a Value> {
    event_variants(root)
        .into_iter()
        .find(|variant| variant.tag == event)
        .map(|variant| variant.schema)
}

pub fn form_fields<'a>(root: &'a Value, node: &Value) -> Option<(&'a Value, Vec<String>)> {
    let schema = variant_schema(root, node.kind())?;
    let mut hidden: Vec<String> = vec!["type".to_owned()];
    match FlowKind::of(node) {
        Some(flow) => {
            hidden.extend(flow.arrows().iter().map(|&s| s.to_owned()));
            match flow {
                FlowKind::On => hidden.push("event".to_owned()),
                FlowKind::Dialogue => hidden.extend(["lines".to_owned()]),
                _ => {}
            }
        }
        None => hidden.extend(
            schema::properties(schema)
                .into_iter()
                .filter(|(_, field)| is_arrow(field))
                .map(|(name, _)| name.clone()),
        ),
    }
    Some((schema, hidden))
}

fn is_arrow(field: &Value) -> bool {
    let types = schema::type_names(field);
    types.len() == 2
        && types.contains(&"string")
        && types.contains(&"null")
        && schema::marker(field).is_none()
}

pub fn summary(node: &Value, catalogue: &NodeCatalogue) -> String {
    pieces(node, catalogue)
        .into_iter()
        .map(|line| line.into_iter().map(|piece| piece.text).collect::<String>())
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

pub fn place_text(value: &Value) -> Option<String> {
    let map = value.as_object()?;
    if !map.contains_key("area") || !map.contains_key("x") {
        return None;
    }
    Some(format!(
        "{} {}, {}",
        map.text_or("area", "?"),
        map["x"].as_f64().unwrap_or(0.0),
        map["y"].as_f64().unwrap_or(0.0)
    ))
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Object(_) if hole_name(value).is_some() => {
            format!("◇{}", hole_name(value).unwrap_or(""))
        }
        Value::Array(items) => items.iter().map(value_text).collect::<Vec<_>>().join(", "),
        other => place_text(other).unwrap_or_else(|| condition::summary(other)),
    }
}

pub struct OutlineRow {
    pub name: String,
    pub depth: usize,
    pub via: Option<String>,
    pub reachable: bool,
    pub link: bool,
    pub choice: Option<usize>,
}

pub fn outline(nodes: &Map<String, Value>, catalogue: &NodeCatalogue) -> Vec<OutlineRow> {
    let mut walker = Walker {
        nodes,
        catalogue,
        seen: HashSet::new(),
        rows: Vec::new(),
        reachable: true,
    };
    for (name, node) in nodes {
        if is_event(node) {
            walker.walk(name, 0, None);
        }
    }
    walker.reachable = false;
    for name in nodes.keys() {
        if !walker.seen.contains(name) {
            walker.walk(name, 0, None);
        }
    }
    walker.rows
}

struct Walker<'a> {
    nodes: &'a Map<String, Value>,
    catalogue: &'a NodeCatalogue,
    seen: HashSet<String>,
    rows: Vec<OutlineRow>,
    reachable: bool,
}

impl Walker<'_> {
    fn walk(&mut self, name: &str, depth: usize, via: Option<String>) {
        if depth > 64 {
            return;
        }
        let link = !self.seen.insert(name.to_owned());
        self.rows.push(OutlineRow {
            name: name.to_owned(),
            depth,
            via,
            reachable: self.reachable,
            link,
            choice: None,
        });
        if link {
            return;
        }
        let Some(node) = self.nodes.get(name) else {
            return;
        };
        let choices = if is_dialogue(node) {
            node["choices"].as_array().map_or(0, Vec::len)
        } else {
            0
        };
        for (index, choice) in node["choices"]
            .as_array()
            .into_iter()
            .flatten()
            .take(choices)
            .enumerate()
        {
            self.rows.push(OutlineRow {
                name: name.to_owned(),
                depth: depth + 1,
                via: None,
                reachable: self.reachable,
                link: false,
                choice: Some(index),
            });
            if let Some(target) = choice["to"].as_str() {
                self.walk(target, depth + 2, None);
            }
        }
        for port in ports(node, self.catalogue) {
            if port.field.starts_with("/choices/") {
                continue;
            }
            if let Some(target) = port.target {
                self.walk(&target, depth + 1, Some(port.label));
            }
        }
    }
}

#[derive(Default)]
pub struct NodeReads {
    pub memories: Vec<String>,
    pub items: Vec<String>,
    pub places: Vec<String>,
    pub other: Vec<String>,
}

impl NodeReads {
    pub fn of(nodes: &Map<String, Value>) -> NodeReads {
        let mut reads = NodeReads::default();
        for node in nodes.values() {
            if let Some(test) = node.get("test") {
                reads.walk(test);
            }
            for choice in node["choices"].as_array().into_iter().flatten() {
                if let Some(requires) = choice.get("requires") {
                    reads.walk(requires);
                }
            }
        }
        for list in [
            &mut reads.memories,
            &mut reads.items,
            &mut reads.places,
            &mut reads.other,
        ] {
            list.sort();
            list.dedup();
        }
        reads
    }

    pub fn text(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.memories.is_empty() {
            parts.push(format!("the subject's memory {}", self.memories.join(", ")));
        }
        parts.extend(self.items.iter().cloned());
        parts.extend(self.places.iter().cloned());
        parts.extend(self.other.iter().cloned());
        parts.join(" · ")
    }

    fn walk(&mut self, value: &Value) {
        match value {
            Value::Object(map) => {
                match map.kind() {
                    "memory" | "set_within" => {
                        if let Some(name) = map.get("memory").and_then(Value::as_str) {
                            self.memories.push(name.to_owned());
                        }
                    }
                    "holding" | "wearing" => {
                        if let Some(item) = map.get("item").and_then(Value::as_str) {
                            self.items.push(format!("holding {item}"));
                        }
                    }
                    "inside" | "in_area" | "knows_area" => {
                        let area = map.text_or("area", "?");
                        let object = map
                            .get("object")
                            .and_then(Value::as_str)
                            .map(|o| format!(" · {o}"))
                            .unwrap_or_default();
                        self.places.push(format!("inside area/{area}{object}"));
                    }
                    "on_quest" | "quest_done" | "task_done" => {
                        if let Some(quest) = map.get("quest").and_then(Value::as_str) {
                            self.other.push(format!("quest/{quest}"));
                        }
                    }
                    "level" => self.other.push("the level".to_owned()),
                    _ => {}
                }
                map.values().for_each(|child| self.walk(child));
            }
            Value::Array(list) => list.iter().for_each(|child| self.walk(child)),
            _ => {}
        }
    }
}

const EFFECT_BLURBS: [(&str, &str, &str); 39] = [
    (
        "warp",
        "area",
        "Moves an actor to a place, setting its spawn point",
    ),
    ("heal", "stat", "Restores health up to the maximum"),
    ("damage", "stat", "Takes health, credited to an npc owner"),
    (
        "apply_status",
        "status",
        "Puts a status on actors, at a power, for a time or until removed",
    ),
    (
        "remove_status",
        "status",
        "Takes a status off actors: one effect, a category or all",
    ),
    (
        "boost",
        "status",
        "Holds a stat modifier while this status, item or passive skill is active",
    ),
    (
        "items",
        "item",
        "Takes and gives items in one exchange, all or nothing",
    ),
    ("use_item", "item", "Uses an item as the player would"),
    ("equip", "item", "Puts an item on, swapping with the slot"),
    ("unequip", "item", "Takes an item or slot off"),
    (
        "drop_items",
        "item",
        "Drops items to the ground at their feet",
    ),
    (
        "spawn_ground_items",
        "item",
        "Scatters items as loot at a place or actor",
    ),
    (
        "clear_ground_items",
        "item",
        "Removes ground items in range",
    ),
    ("move_items", "item", "Moves items from one bag to another"),
    ("grant_xp", "class", "Grants experience as kills do"),
    ("remember", "memory", "Sets a memory, timed now"),
    ("forget", "memory", "Sets memories back to zero"),
    ("offer_quest", "quest", "Shows the offer card and pauses"),
    ("accept_quest", "quest", "Takes the quest at once"),
    ("turn_in", "quest", "Shows the turn-in card and pauses"),
    ("fail_quest", "quest", "Fails an active quest"),
    ("complete_task", "quest", "Completes a task objective"),
    ("decide", "quest", "Decides a quest's path"),
    ("open_shop", "shop", "Opens the shop until it closes"),
    ("notify", "notification", "Shows a notification"),
    (
        "say",
        "chat",
        "A chat line and a speech bubble from the owner",
    ),
    ("wander", "npc body", "Walks to a random reachable spot"),
    (
        "walk_to",
        "npc body",
        "Walks to a place, an object or an actor",
    ),
    ("attack", "npc body", "Chases and swings at a target"),
    ("stop", "npc body", "Interrupts the body"),
    ("face", "actor", "Turns an actor toward something"),
    (
        "animate",
        "actor",
        "Plays a model action, waiting for its apex or end when asked",
    ),
    (
        "use_skill",
        "skill",
        "Casts a skill as a player would, aimed at an actor or a point",
    ),
    (
        "reset_skills",
        "skill",
        "Asks, then forgets every learned rank and returns the points",
    ),
    (
        "vfx",
        "vfx",
        "Plays a vfx row on each actor, a point or an area named, along a path through the actors, or on a player's screen or widget",
    ),
    (
        "set_attitude",
        "npc",
        "Hostile or friendly, until set again",
    ),
    ("spawn_npcs", "npc", "Spawns a pack of npcs nearby"),
    ("despawn", "npc", "Removes an npc that spawn_npcs made"),
    (
        "play_sound",
        "audio",
        "Plays a sound at the owner or the subject",
    ),
];

pub fn arrow_meaning(kind: &str, label: &str) -> &'static str {
    match (kind, label) {
        ("items", "refused") => "no arrow: the flow ends and the player is told why",
        ("walk_to", "arrived") => "there",
        ("walk_to", "blocked") => "no way there",
        (_, "interrupted") => "another flow took the body",
        ("wait", "next") => "ends the pass",
        ("turn_in", "refused") => "objectives unmet: the flow ends",
        ("turn_in", "cancelled") => "the card was closed: the flow ends",
        (_, "left") => "the subject walked away: the flow ends",
        _ => "no arrow · ends the flow",
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SummaryPiece {
    pub text: String,
    pub tone: SummaryTone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SummaryTone {
    Muted,
    Strong,
    Var,
}

fn muted(text: impl Into<String>) -> SummaryPiece {
    SummaryPiece {
        text: text.into(),
        tone: SummaryTone::Muted,
    }
}

fn strong(text: impl Into<String>) -> SummaryPiece {
    SummaryPiece {
        text: text.into(),
        tone: SummaryTone::Strong,
    }
}

fn var(text: impl Into<String>) -> SummaryPiece {
    SummaryPiece {
        text: text.into(),
        tone: SummaryTone::Var,
    }
}

pub fn pieces(node: &Value, catalogue: &NodeCatalogue) -> Vec<Vec<SummaryPiece>> {
    match FlowKind::of(node) {
        Some(FlowKind::On) => vec![event_pieces(node)],
        Some(FlowKind::If) => vec![expression_pieces(&node["test"])],
        Some(FlowKind::Random) => {
            let weights: Vec<String> = node["weights"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|w| w.to_string())
                .collect();
            vec![vec![muted("weights "), strong(weights.join(" : "))]]
        }
        Some(FlowKind::Repeat) => {
            let mut line = expression_pieces(&node["times"]);
            line.push(muted(" times"));
            if let Some(at_most) = node["at_most"].as_f64() {
                line.push(muted(" · at most "));
                line.push(strong(at_most.to_string()));
            }
            vec![line]
        }
        Some(FlowKind::Wait) => {
            let mut line = expression_pieces(&node["seconds"]);
            line.push(muted(" s"));
            if node["seconds"].as_f64() == Some(0.0) {
                line.push(muted(" · the next tick"));
            } else if let Some(at_most) = node["at_most"].as_f64() {
                line.push(muted(" · at most "));
                line.push(strong(at_most.to_string()));
            }
            vec![line]
        }
        Some(FlowKind::Set) => node["values"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|value| {
                let name = value["local"]
                    .as_str()
                    .or(value["state"].as_str())
                    .unwrap_or("?");
                let mut line = vec![strong(name), muted(" = ")];
                line.extend(expression_pieces(&value["to"]));
                line
            })
            .collect(),
        Some(FlowKind::Run) => {
            let mut lines = vec![vec![muted("script/"), strong(node.text_or("script", "?"))]];
            if let Some(values) = node["values"].as_object().filter(|v| !v.is_empty()) {
                let mut line = Vec::new();
                for (index, (name, value)) in values.iter().enumerate() {
                    if index > 0 {
                        line.push(muted(" · "));
                    }
                    line.push(muted(format!("{name} ")));
                    line.extend(expression_pieces(value));
                }
                lines.push(line);
            }
            lines
        }
        Some(FlowKind::Return) => match node.get("value") {
            Some(value) if !value.is_null() => vec![expression_pieces(value)],
            _ => vec![vec![muted("back to the caller")]],
        },
        Some(FlowKind::Dialogue) => vec![dialogue_pieces(node)],
        None => match node.kind() {
            "remember" => {
                let mut line = vec![strong(node.text_or("memory", "?")), muted(" = ")];
                match node.get("to") {
                    Some(to) if !to.is_null() => line.extend(expression_pieces(to)),
                    _ => line.push(strong("true")),
                }
                vec![line]
            }
            "forget" => vec![vec![
                strong(
                    node["memories"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
                muted(" back to zero"),
            ]],
            "items" => {
                let mut lines = Vec::new();
                for (field, word) in [("take", "take "), ("give", "give ")] {
                    if let Some(list) = node[field].as_array().filter(|l| !l.is_empty()) {
                        let mut line = vec![muted(word)];
                        line.extend(stack_pieces(list));
                        lines.push(line);
                    }
                }
                if lines.is_empty() {
                    lines.push(vec![muted("nothing changes hands")]);
                }
                lines
            }
            "warp" => {
                let who = node["who"]
                    .as_str()
                    .map(actor_word)
                    .unwrap_or_else(|| "the subject".to_owned());
                let mut line = vec![strong(who), muted(" to ")];
                match (hole_name(&node["to"]), place_text(&node["to"])) {
                    (Some(name), _) => line.push(var(format!("◇ {name}"))),
                    (None, Some(place)) => line.push(strong(place)),
                    (None, None) => line.extend(expression_pieces(&node["to"])),
                }
                vec![line, vec![muted("sets its spawn point")]]
            }
            other => effect_pieces(node, other, catalogue),
        },
    }
}

fn dialogue_pieces(node: &Value) -> Vec<SummaryPiece> {
    let lines = node["lines"].as_array().cloned().unwrap_or_default();
    let Some(first) = lines.first() else {
        return vec![muted("no lines")];
    };
    let mut line = vec![
        strong(speaker_name(&first["by"])),
        muted(": "),
        strong(widgets::clip(&spans::plain(&first["text"]), 48)),
    ];
    if lines.len() > 1 {
        line.push(muted(format!(" (+{} lines)", lines.len() - 1)));
    }
    let choices = node["choices"].as_array().map_or(0, Vec::len);
    if choices > 0 {
        line.push(muted(format!(" · {choices} choices")));
    }
    line
}

fn stack_pieces(list: &[Value]) -> Vec<SummaryPiece> {
    let mut line = Vec::new();
    for (index, stack) in list.iter().enumerate() {
        if index > 0 {
            line.push(muted(", "));
        }
        let count = stack["count"].as_u64().unwrap_or(1);
        line.push(strong(format!("{count} {}", stack.text_or("item", "?"))));
    }
    line
}

fn event_pieces(node: &Value) -> Vec<SummaryPiece> {
    let mut line = Vec::new();
    match event_of(node) {
        Some("interacted") => {
            line.push(muted("verb "));
            line.push(strong(node.text_or("verb", "use")));
            if let Some(reach) = node["reach"].as_f64() {
                line.push(muted(" · reach "));
                line.push(strong(reach.to_string()));
                line.push(muted(" tiles"));
            }
        }
        Some("came_near") | Some("entered") | Some("left") => {
            line.push(strong(node.text_or("by", "characters")));
            if let Some(within) = node["within"].as_f64() {
                line.push(muted(" within "));
                line.push(strong(within.to_string()));
            }
        }
        Some("stood_on") => {
            line.push(strong(node.text_or("by", "characters")));
            line.push(muted(" on "));
            line.push(strong(node.text_or("object", "?")));
        }
        Some("started") => line.push(muted("the owner spawned")),
        Some("run") => line.push(muted("started by a run node")),
        Some(_) | None => {
            for (field, value) in node.as_object().into_iter().flatten() {
                if matches!(field.as_str(), "type" | "event" | "next") {
                    continue;
                }
                if !line.is_empty() {
                    line.push(muted(" · "));
                }
                line.push(muted(format!("{field} ")));
                line.extend(expression_pieces(value));
            }
        }
    }
    line
}

fn effect_pieces(node: &Value, kind: &str, catalogue: &NodeCatalogue) -> Vec<Vec<SummaryPiece>> {
    let fields: Vec<(String, Option<String>)> = catalogue
        .entry(kind)
        .map(|entry| entry.fields.clone())
        .unwrap_or_else(|| {
            node.as_object()
                .into_iter()
                .flatten()
                .map(|(name, _)| (name.clone(), None))
                .filter(|(name, _)| name != "type")
                .collect()
        });
    let mut named: Vec<SummaryPiece> = Vec::new();
    let mut rest: Vec<SummaryPiece> = Vec::new();
    for (field, table) in &fields {
        let Some(value) = node.get(field).filter(|v| !v.is_null()) else {
            continue;
        };
        let piece = field_pieces(field, table.as_deref(), value);
        let main = table.is_some()
            || matches!(
                field.as_str(),
                "who" | "to" | "at" | "for" | "toward" | "target"
            )
            || value.is_object()
            || value.is_array();
        let into = if main { &mut named } else { &mut rest };
        if !into.is_empty() {
            into.push(muted(" · "));
        }
        into.extend(piece);
    }
    let mut lines = Vec::new();
    if !named.is_empty() {
        lines.push(named);
    }
    if !rest.is_empty() {
        lines.push(rest);
    }
    lines
}

fn field_pieces(field: &str, table: Option<&str>, value: &Value) -> Vec<SummaryPiece> {
    let is_actor_field = matches!(
        field,
        "who" | "at" | "for" | "toward" | "target" | "from" | "to"
    );
    let labelled = |pieces: Vec<SummaryPiece>| -> Vec<SummaryPiece> {
        let mut out = vec![muted(format!("{field} "))];
        out.extend(pieces);
        out
    };
    match (table, value) {
        (Some(table), Value::String(key)) => vec![muted(format!("{table}/")), strong(key)],
        _ if hole_name(value).is_some() => {
            let hole = var(format!("◇ {}", hole_name(value).unwrap_or("")));
            if field == "who" {
                vec![hole]
            } else {
                labelled(vec![hole])
            }
        }
        (_, Value::String(text)) if is_actor_field => {
            let actor = strong(actor_word(text));
            if field == "who" {
                vec![actor]
            } else {
                labelled(vec![actor])
            }
        }
        (_, Value::Array(list)) if list.iter().all(|item| item.get("item").is_some()) => {
            labelled(stack_pieces(list))
        }
        _ if place_text(value).is_some() => {
            let place = strong(place_text(value).unwrap_or_default());
            if field == "to" {
                vec![place]
            } else {
                labelled(vec![place])
            }
        }
        (_, Value::Object(map))
            if map.get("type").is_some() && matches!(map.kind(), "object" | "actor" | "place") =>
        {
            let inner = map
                .iter()
                .find(|(k, _)| *k != "type")
                .map(|(_, v)| value_text(v))
                .unwrap_or_default();
            vec![muted(format!("{} ", map.kind())), strong(inner)]
        }
        (_, Value::Number(_)) | (_, Value::Bool(_)) => labelled(vec![strong(value_text(value))]),
        (_, Value::String(text)) => vec![strong(text)],
        (_, Value::Array(list)) => labelled(vec![strong(
            list.iter().map(value_text).collect::<Vec<_>>().join(", "),
        )]),
        (_, other) => labelled(expression_pieces(other)),
    }
}

fn actor_word(text: &str) -> String {
    match text {
        "subject" => "the subject".to_owned(),
        "owner" => "the owner".to_owned(),
        other => other.replace('_', " "),
    }
}

pub fn expression_pieces(value: &Value) -> Vec<SummaryPiece> {
    if let Some(name) = hole_name(value) {
        return vec![var(format!("◇ {name}"))];
    }
    let summary = condition::summary(value);
    let mut out: Vec<SummaryPiece> = Vec::new();
    for word in summary.split(' ') {
        let bare = word.trim_matches(|c: char| c == '(' || c == ')' || c == ',');
        let is_name = !bare.is_empty()
            && (bare.contains('_')
                || bare.chars().all(|c| c.is_ascii_digit() || c == '.')
                || bare.starts_with('"')
                || bare == "true"
                || bare == "false");
        let piece = if is_name { strong(word) } else { muted(word) };
        if let Some(last) = out.last_mut()
            && last.tone == piece.tone
        {
            last.text.push(' ');
            last.text.push_str(&piece.text);
        } else {
            if !out.is_empty() {
                out.push(muted(" "));
            }
            out.push(piece);
        }
    }
    out
}
