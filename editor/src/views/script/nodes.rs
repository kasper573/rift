use std::collections::{BTreeMap, HashSet, VecDeque};
use ui::words;

use engine::core::content::{ContentFile, ContentId, ContentScope, FolderReader, with_content};
use engine::core::script::node::{MAX_CALL_DEPTH, NEXT_EXIT};
use engine::core::script::{
    Arrow, DialogueNode, EffectDef, EventDef, Exit, Expr, Fill, NodeKind, REFUSED, RunnerKindDef,
    Target, VarValue,
};
use engine::core::time::Seconds;
use engine::core::units::Meters;
use engine::systems::actor::model::Expression;
use engine::systems::interact::Verb;
use engine::systems::item::ItemStack;
use engine::systems::script::{
    Condition, EventKind, Line, Node, Place, RiftEffect, RiftEffectKind, RiftEvent, RiftQuery,
    RiftScripting, RunnerKind, Speaker, Who,
};
use engine::systems::text::plain;
use serde::Deserialize;
use serde_json::Value;

use crate::keymap::EditorKey;
use crate::project::Project;
use crate::schema::{self, SchemaVariant};
use crate::tabs::shown_path;
use crate::{condition, rows, spans};

pub type Nodes = BTreeMap<String, Node>;

/// The tag a node is written with: its flow kind, or the effect it is.
pub fn tag_of(node: &Node) -> &'static str {
    match node {
        Node::Effect(effect) => RiftEffectKind::from(effect).into(),
        other => other.kind().name(),
    }
}

pub fn is_dialogue(node: &Node) -> bool {
    matches!(node, Node::Dialogue(_))
}

pub fn is_event(node: &Node) -> bool {
    matches!(node, Node::On { .. })
}

pub fn event_of(node: &Node) -> Option<EventKind> {
    match node {
        Node::On { event, .. } => Some(event.kind()),
        _ => None,
    }
}

pub fn event_name(node: &Node) -> Option<&'static str> {
    event_of(node).map(Into::into)
}

pub fn hint(node: &Node) -> String {
    match node.kind() {
        NodeKind::Dialogue => format!(
            "{} adds a line · {} a choice · select text to style it",
            EditorKey::Confirm.name(),
            EditorKey::AddChoice.name()
        ),
        NodeKind::If => {
            "each slot takes one type · the same builder edits every condition".to_owned()
        }
        NodeKind::Repeat | NodeKind::Wait | NodeKind::Random => {
            "a loop may come back only through a node that pauses".to_owned()
        }
        NodeKind::Run => format!("calls nest {MAX_CALL_DEPTH} deep and never loop back"),
        NodeKind::On => "a flow starts here; its cause is the subject".to_owned(),
        NodeKind::Set | NodeKind::Return | NodeKind::Effect => {
            "an output without an arrow ends the flow".to_owned()
        }
    }
}

/// Who says a line, as a writer picks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeakerKind {
    Npc,
    Object,
    Narrator,
    Player,
}

impl SpeakerKind {
    pub fn of(speaker: Speaker) -> SpeakerKind {
        match speaker {
            Speaker::Npc(_) => SpeakerKind::Npc,
            Speaker::Object => SpeakerKind::Object,
            Speaker::Narrator => SpeakerKind::Narrator,
            Speaker::Player => SpeakerKind::Player,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SpeakerKind::Npc => "npc",
            SpeakerKind::Object => "object",
            SpeakerKind::Narrator => "narrator",
            SpeakerKind::Player => "player",
        }
    }

    /// Who may speak the lines of a script its owner runs.
    pub fn options(owner: RunnerKind) -> &'static [SpeakerKind] {
        match owner {
            RunnerKind::Object => &[SpeakerKind::Npc, SpeakerKind::Object, SpeakerKind::Narrator],
            _ => &[SpeakerKind::Npc, SpeakerKind::Narrator],
        }
    }
}

pub fn blank_line(by: Speaker) -> Line {
    Line {
        by,
        face: None,
        text: vec![plain("")],
    }
}

pub fn speaker_name(reader: &FolderReader, by: Speaker) -> String {
    match by {
        Speaker::Npc(npc) => shown_path(reader.path_of(npc).unwrap_or("?")).to_owned(),
        other => SpeakerKind::of(other).name().to_owned(),
    }
}

/// The bust image `by` shows in `face`, when an npc speaks.
pub fn bust_file(project: &Project, by: Speaker, face: Option<&Expression>) -> Option<String> {
    let Speaker::Npc(npc) = by else {
        return None;
    };
    let model = project.read_id(project.read_id(npc)?.model)?;
    let busts = model.busts()?;
    let file = match face {
        Some(face) => busts.face(face)?,
        None => &busts.generic.neutral,
    };
    Some(file.as_str().to_owned())
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodePort {
    pub label: String,
    pub exit: Exit,
    pub target: Option<String>,
}

impl NodePort {
    pub fn is_choice(&self) -> bool {
        matches!(self.exit, Exit::Choice(_))
    }
}

#[derive(Clone, Debug)]
pub struct NodeEntry {
    pub kind: String,
    pub system: String,
    pub blurb: String,
    pub owners: Vec<RunnerKind>,
    pub outputs: Vec<NodeOutput>,
    pub fields: Vec<(String, Option<String>)>,
    pub pauses: bool,
}

#[derive(Clone, Debug)]
pub struct NodeOutput {
    pub name: String,
    /// When the node takes the output, as its definition says.
    pub meaning: Option<String>,
    /// Whether the node takes the output when what it does fails.
    pub fails: bool,
}

impl NodeEntry {
    pub fn fits(&self, linking: bool) -> bool {
        (self.kind == NodeKind::On.name()) != linking
    }
}

pub struct OwnerRefusal {
    pub owner: RunnerKind,
    pub allowed: Vec<RunnerKind>,
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
                words::article(first.name()),
                first.name()
            ),
            None => self.text(),
        }
    }
}

/// Every kind of node as the schema of a node describes it: what it does, the outputs it leads
/// out through and the owners it is for.
#[derive(Default)]
pub struct NodeCatalogue {
    pub entries: Vec<NodeEntry>,
    pub events: Vec<(String, Vec<RunnerKind>)>,
}

impl NodeCatalogue {
    pub fn build() -> NodeCatalogue {
        let root = node_root();
        let nowhere = schema::Nearby::nowhere();
        let parse = |template: Value| unscoped::<Node>(template);
        let entries = variants()
            .iter()
            .map(|variant| {
                let (outputs, fields): (Vec<_>, Vec<_>) = schema::properties(variant.schema)
                    .into_iter()
                    .filter(|(name, _)| **name != variant.field)
                    .partition(|(_, field)| schema::leads_out(field));
                let node = parse(schema::variant_template(root, variant, nowhere));
                NodeEntry {
                    kind: variant.tag.clone(),
                    system: schema::system_of(variant.schema)
                        .unwrap_or("other")
                        .to_owned(),
                    blurb: schema::description(variant.schema)
                        .map(first_sentence)
                        .unwrap_or_default(),
                    owners: match &node {
                        Some(Node::Effect(effect)) => effect.owners().to_vec(),
                        _ => Vec::new(),
                    },
                    outputs: outputs
                        .into_iter()
                        .map(|(name, field)| NodeOutput {
                            name: name.clone(),
                            meaning: schema::description(field).map(first_sentence),
                            fails: schema::fails(field),
                        })
                        .collect(),
                    fields: fields
                        .into_iter()
                        .map(|(name, field)| {
                            (
                                name.clone(),
                                schema::table_of(schema::resolve(root, field)).map(str::to_owned),
                            )
                        })
                        .collect(),
                    pauses: node.as_ref().is_some_and(Node::pauses),
                }
            })
            .collect();
        let events = event_variants()
            .into_iter()
            .map(|variant| {
                let owners =
                    unscoped::<RiftEvent>(schema::variant_template(root, &variant, nowhere))
                        .map(|event| event.owners().to_vec())
                        .unwrap_or_default();
                (variant.tag.clone(), owners)
            })
            .collect();
        NodeCatalogue { entries, events }
    }

    pub fn entry(&self, kind: &str) -> Option<&NodeEntry> {
        self.entries.iter().find(|entry| entry.kind == kind)
    }

    pub fn entry_of(&self, node: &Node) -> Option<&NodeEntry> {
        self.entry(tag_of(node))
    }

    /// What becomes of a flow that takes the output while no arrow leaves it.
    pub fn unwired(&self, node: &Node, output: &str) -> String {
        let meaning = self
            .entry_of(node)
            .and_then(|entry| entry.outputs.iter().find(|known| known.name == output))
            .and_then(|known| known.meaning.as_deref());
        match meaning {
            Some(meaning) => format!("{meaning} · no arrow ends the flow"),
            None if output == REFUSED => {
                "no arrow: the flow ends and the player is told why".to_owned()
            }
            None => "no arrow · ends the flow".to_owned(),
        }
    }

    /// Whether the node takes `output` when what it does fails.
    pub fn fails(&self, node: &Node, output: &str) -> bool {
        self.entry_of(node)
            .and_then(|entry| entry.outputs.iter().find(|known| known.name == output))
            .is_some_and(|known| known.fails)
    }

    pub fn refusal(&self, kind: &str, owner: RunnerKind) -> Option<OwnerRefusal> {
        allowed(&self.entry(kind)?.owners, owner)
    }

    pub fn event_refusal(&self, event: &str, owner: RunnerKind) -> Option<OwnerRefusal> {
        let (_, owners) = self.events.iter().find(|(name, _)| name == event)?;
        allowed(owners, owner)
    }
}

/// `template` read as a `T` whatever the files it names, which a template of no folder names
/// none of.
fn unscoped<T: serde::de::DeserializeOwned>(template: Value) -> Option<T> {
    with_content(ContentScope::new(), || {
        serde_json::from_value(template).ok()
    })
    .0
}

fn allowed(owners: &[RunnerKind], owner: RunnerKind) -> Option<OwnerRefusal> {
    if owners.is_empty()
        || owner == RunnerKind::Any
        || owners.contains(&owner)
        || owners.contains(&RunnerKind::Any)
    {
        return None;
    }
    Some(OwnerRefusal {
        owner,
        allowed: owners.to_vec(),
    })
}

pub fn has_input(node: &Node) -> bool {
    !is_event(node)
}

/// The outputs a node's card shows, each where it leaves the node: a dialogue's choices stand in
/// for its plain next.
pub fn ports(node: &Node) -> Vec<NodePort> {
    let port = |label: String, exit: Exit, arrow: &Arrow| NodePort {
        label,
        exit,
        target: arrow.to().map(str::to_owned),
    };
    match node {
        Node::Random { weights, outputs } => outputs
            .iter()
            .enumerate()
            .map(|(index, arrow)| {
                let weight = weights
                    .get(index)
                    .map_or_else(|| (index + 1).to_string(), u32::to_string);
                port(weight, Exit::Output(index), arrow)
            })
            .collect(),
        Node::Dialogue(dialogue) => {
            let mut ports: Vec<NodePort> = dialogue
                .choices
                .iter()
                .enumerate()
                .map(|(index, choice)| {
                    let label = words::clip(&spans::plain(&choice.label), 22);
                    port(label, Exit::Choice(index), &choice.to)
                })
                .collect();
            if dialogue.choices.is_empty() {
                ports.push(port(exit_name(NEXT_EXIT), NEXT_EXIT, &dialogue.next));
            }
            let left = Exit::Named("left");
            ports.push(port(exit_name(left), left, &dialogue.left));
            ports
        }
        other => other
            .exits()
            .into_iter()
            .map(|(exit, arrow)| port(exit_name(exit), exit, arrow))
            .collect(),
    }
}

/// The exit of `node` a scene names by the label of its output.
pub fn exit_called(node: &Node, output: &str) -> Option<Exit> {
    node.exits()
        .into_iter()
        .map(|(exit, _)| exit)
        .find(|exit| exit_name(*exit) == output)
}

pub fn exit_label(exit: Exit) -> String {
    exit_name(exit)
}

fn exit_name(exit: Exit) -> String {
    match exit {
        Exit::Named(name) => name.to_owned(),
        Exit::Output(index) => format!("output {}", index + 1),
        Exit::Choice(index) => format!("choice {}", index + 1),
    }
}

pub fn set_arrow(node: &mut Node, exit: Exit, target: Option<&str>) {
    if let Some(arrow) = node.exit_mut(exit) {
        arrow.point(target);
    }
}

pub fn arrows_into(nodes: &Nodes, target: &str) -> Vec<(String, Exit)> {
    let mut found = Vec::new();
    for (name, node) in nodes {
        for (exit, arrow) in node.exits() {
            if arrow.to() == Some(target) {
                found.push((name.clone(), exit));
            }
        }
    }
    found
}

pub fn unlink(nodes: &mut Nodes, target: &str) {
    for node in nodes.values_mut() {
        for (_, arrow) in node.exits_mut() {
            if arrow.to() == Some(target) {
                arrow.point(None);
            }
        }
    }
}

pub fn port_label(nodes: &Nodes, from: &str, exit: Exit) -> String {
    nodes
        .get(from)
        .and_then(|node| ports(node).into_iter().find(|port| port.exit == exit))
        .map_or_else(|| exit_name(exit), |port| port.label)
}

pub fn reachable(nodes: &Nodes) -> HashSet<String> {
    depths(nodes).into_keys().collect()
}

pub fn depths(nodes: &Nodes) -> BTreeMap<String, usize> {
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
            for (_, arrow) in node.exits() {
                if let Some(target) = arrow.to()
                    && !depths.contains_key(target)
                {
                    queue.push_back((target.to_owned(), depth + 1));
                }
            }
        }
    }
    depths
}

pub fn rename(nodes: &mut Nodes, old: &str, new: &str) {
    let Some(node) = nodes.remove(old) else {
        return;
    };
    nodes.insert(new.to_owned(), node);
    for node in nodes.values_mut() {
        for (_, arrow) in node.exits_mut() {
            if arrow.to() == Some(old) {
                arrow.point(Some(new));
            }
        }
    }
}

pub fn free_name(nodes: &Nodes, base: &str) -> String {
    let base = base
        .trim()
        .replace(|c: char| !c.is_ascii_alphanumeric() && c != '_', "_");
    let base = if base.is_empty() { "node" } else { &base };
    rows::free_name(base, &nodes.keys().cloned().collect())
}

/// The schema of a node, as the forms of a node's fields walk it.
pub fn node_root() -> &'static Value {
    schema::of_type::<Node>()
}

pub fn variants() -> Vec<SchemaVariant<'static>> {
    let root = node_root();
    schema::tagged_variants(root, root)
}

pub fn variant_schema(kind: &str) -> Option<&'static Value> {
    variants()
        .into_iter()
        .find(|variant| variant.tag == kind)
        .map(|variant| variant.schema)
}

pub fn event_variants() -> Vec<SchemaVariant<'static>> {
    let Some(on) = variant_schema(NodeKind::On.name()) else {
        return Vec::new();
    };
    schema::tagged_variants(node_root(), on)
}

pub fn event_variant(event: &str) -> Option<&'static Value> {
    event_variants()
        .into_iter()
        .find(|variant| variant.tag == event)
        .map(|variant| variant.schema)
}

/// A node of `kind` as the editor first places it: the least it can be, with a start a writer
/// can see work. Its references are among the files `nearby`.
pub fn fresh(
    reader: &FolderReader,
    kind: &str,
    runs_on: RunnerKind,
    nearby: schema::Nearby,
    speaker: Speaker,
) -> Option<Node> {
    let none = Arrow::default;
    let flow = kind.parse::<NodeKind>().ok();
    Some(match flow {
        Some(NodeKind::On) => return fresh_event(reader, EventKind::Started.into(), nearby),
        Some(NodeKind::Dialogue) => Node::Dialogue(DialogueNode {
            lines: vec![blank_line(speaker)],
            choices: Vec::new(),
            next: none(),
            left: none(),
        }),
        Some(NodeKind::Random) => Node::Random {
            weights: vec![1, 1],
            outputs: vec![none(), none()],
        },
        Some(NodeKind::Repeat) => Node::Repeat {
            times: Expr::Int(2),
            at_most: 10,
            each: none(),
            then: none(),
        },
        Some(NodeKind::Wait) => Node::Wait {
            seconds: Expr::Num(1.0),
            at_most: Seconds(60.0),
            next: none(),
        },
        Some(NodeKind::If) => Node::If {
            test: Expr::Bool(true),
            yes: none(),
            no: none(),
        },
        Some(NodeKind::Set) => Node::Set {
            values: Vec::new(),
            next: none(),
        },
        Some(NodeKind::Return) => Node::Return { value: None },
        Some(NodeKind::Run) => Node::Run {
            script: reader.id_of(&nearby.script_run_by(runs_on)?)?,
            values: BTreeMap::new(),
            into: None,
            next: none(),
        },
        Some(NodeKind::Effect) | None => {
            let variant = variants().into_iter().find(|variant| variant.tag == kind)?;
            let template = schema::variant_template(node_root(), &variant, nearby);
            return reader.from_json(&template).ok();
        }
    })
}

/// A node starting a flow at the first event a script of `kind` answers that none in `used`
/// already starts.
pub fn free_event(
    reader: &FolderReader,
    catalogue: &NodeCatalogue,
    kind: RunnerKind,
    used: &HashSet<&str>,
    nearby: schema::Nearby,
) -> Option<Node> {
    let (event, _) = catalogue
        .events
        .iter()
        .find(|(event, owners)| owners.contains(&kind) && !used.contains(event.as_str()))?;
    fresh_event(reader, event, nearby)
}

/// An event node for `event`, its fields at a start a writer can see work.
pub fn fresh_event(reader: &FolderReader, event: &str, nearby: schema::Nearby) -> Option<Node> {
    let variant = event_variants()
        .into_iter()
        .find(|variant| variant.tag == event)?;
    let template = schema::variant_template(node_root(), &variant, nearby);
    let mut event = reader.from_json::<RiftEvent>(&template).ok()?;
    match &mut event {
        RiftEvent::Interacted { verb, reach, .. } => {
            *verb = Verb::Talk;
            *reach = Meters(1.5);
        }
        RiftEvent::CameNear { within, .. } | RiftEvent::Heard { within, .. } => {
            *within = Meters(4.0);
        }
        _ => {}
    }
    Some(Node::On {
        event,
        next: Arrow::default(),
    })
}

/// The schema of the fields a node's form shows, and those it leaves to the node's arrows.
pub fn form_fields(node: &Node) -> Option<(&'static Value, Vec<String>)> {
    let variant = variants()
        .into_iter()
        .find(|variant| variant.tag == tag_of(node))?;
    let hidden = schema::properties(variant.schema)
        .into_iter()
        .filter(|(name, field)| **name == variant.field || schema::leads_out(field))
        .map(|(name, _)| name.clone())
        .collect();
    Some((variant.schema, hidden))
}

/// What reads the names and files a node's summary shows.
pub struct NodeText<'a> {
    pub project: &'a Project,
    pub reader: &'a FolderReader,
    pub catalogue: &'a NodeCatalogue,
}

impl NodeText<'_> {
    fn path_of<Row: ContentFile>(&self, id: ContentId<Row>) -> String {
        shown_path(self.reader.path_of(id).unwrap_or("?")).to_owned()
    }
}

pub fn summary(node: &Node, text: &NodeText) -> String {
    pieces(node, text)
        .into_iter()
        .map(|line| line.into_iter().map(|piece| piece.text).collect::<String>())
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

pub fn place_text(text: &NodeText, place: &Place) -> String {
    format!(
        "{} {}, {}",
        text.path_of(place.area),
        place.at.x,
        place.at.y
    )
}

pub struct OutlineRow {
    pub name: String,
    pub depth: usize,
    pub via: Option<String>,
    pub reachable: bool,
    pub link: bool,
    pub choice: Option<usize>,
}

pub fn outline(nodes: &Nodes) -> Vec<OutlineRow> {
    let mut walker = Walker {
        nodes,
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
    nodes: &'a Nodes,
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
        if let Node::Dialogue(dialogue) = node {
            for (index, choice) in dialogue.choices.iter().enumerate() {
                self.rows.push(OutlineRow {
                    name: name.to_owned(),
                    depth: depth + 1,
                    via: None,
                    reachable: self.reachable,
                    link: false,
                    choice: Some(index),
                });
                if let Some(target) = choice.to.to() {
                    self.walk(target, depth + 2, None);
                }
            }
        }
        for port in ports(node) {
            if port.is_choice() {
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
    /// What the nodes' tests and choice requirements ask of the world, each file by its path.
    pub fn of(reader: &FolderReader, nodes: &Nodes) -> NodeReads {
        let mut asked: Vec<&RiftQuery> = Vec::new();
        for node in nodes.values() {
            match node {
                Node::If { test, .. } => queries_in(test, &mut asked),
                Node::Dialogue(dialogue) => {
                    for requires in dialogue.choices.iter().filter_map(|c| c.requires.as_ref()) {
                        queries_in(requires, &mut asked);
                    }
                }
                _ => {}
            }
        }
        let mut reads = NodeReads::default();
        for query in asked {
            reads.note(reader, query);
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
            let memories: Vec<&str> = self.memories.iter().map(|m| shown_path(m)).collect();
            parts.push(format!("the subject's memory {}", memories.join(", ")));
        }
        parts.extend(self.items.iter().cloned());
        parts.extend(self.places.iter().cloned());
        parts.extend(self.other.iter().cloned());
        parts.join(" · ")
    }

    fn note(&mut self, reader: &FolderReader, query: &RiftQuery) {
        let path = |path: Option<&str>| path.unwrap_or("?").to_owned();
        match query {
            RiftQuery::Memory { memory, .. } | RiftQuery::SetWithin { memory, .. } => {
                self.memories.push(path(reader.path_of(*memory)));
            }
            RiftQuery::Holding { item, .. } => {
                let item = path(reader.path_of(*item));
                self.items.push(format!("holding {}", shown_path(&item)));
            }
            RiftQuery::Wearing { item, .. } => {
                let item = path(reader.path_of(*item));
                self.items.push(format!("wearing {}", shown_path(&item)));
            }
            RiftQuery::Inside { object, .. } => self.places.push(format!("inside {object}")),
            RiftQuery::InArea { area, .. } => {
                let area = path(reader.path_of(*area));
                self.places.push(format!("in {}", shown_path(&area)));
            }
            RiftQuery::KnowsArea { area, .. } => {
                let area = path(reader.path_of(*area));
                self.places.push(format!("knows {}", shown_path(&area)));
            }
            RiftQuery::OnQuest { quest, .. }
            | RiftQuery::QuestDone { quest, .. }
            | RiftQuery::QuestCooling { quest, .. }
            | RiftQuery::TaskDone { quest, .. }
            | RiftQuery::QuestResetsIn { quest, .. } => {
                let quest = path(reader.path_of(*quest));
                self.other.push(shown_path(&quest).to_owned());
            }
            RiftQuery::Level { .. } => self.other.push("the level".to_owned()),
            _ => {}
        }
    }
}

fn queries_in<'a>(condition: &'a Condition, found: &mut Vec<&'a RiftQuery>) {
    if let Expr::Query(query) = condition {
        found.push(query);
    }
    for child in condition.children() {
        queries_in(child, found);
    }
}

/// A description's opening sentence, without its full stop: what a palette row says.
fn first_sentence(text: &str) -> String {
    let first = text.split_once(". ").map_or(text, |(first, _)| first);
    first.trim_end_matches('.').to_owned()
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

fn hole(name: &str) -> SummaryPiece {
    var(format!("◇ {name}"))
}

pub fn pieces(node: &Node, text: &NodeText) -> Vec<Vec<SummaryPiece>> {
    match node {
        Node::On { event, .. } => vec![event_pieces(text, event)],
        Node::If { test, .. } => vec![expression_pieces(text, test)],
        Node::Random { weights, .. } => {
            let weights: Vec<String> = weights.iter().map(u32::to_string).collect();
            vec![vec![muted("weights "), strong(weights.join(" : "))]]
        }
        Node::Repeat { times, at_most, .. } => {
            let mut line = expression_pieces(text, times);
            line.push(muted(" times"));
            line.push(muted(" · at most "));
            line.push(strong(at_most.to_string()));
            vec![line]
        }
        Node::Wait {
            seconds, at_most, ..
        } => {
            let mut line = expression_pieces(text, seconds);
            line.push(muted(" s"));
            if matches!(seconds, Expr::Num(0.0) | Expr::Int(0)) {
                line.push(muted(" · the next tick"));
            } else {
                line.push(muted(" · at most "));
                line.push(strong(at_most.0.to_string()));
            }
            vec![line]
        }
        Node::Set { values, .. } => values
            .iter()
            .map(|assignment| {
                let (Target::Local(name) | Target::State(name)) = &assignment.target;
                let mut line = vec![strong(name), muted(" = ")];
                line.extend(expression_pieces(text, &assignment.to));
                line
            })
            .collect(),
        Node::Run { script, values, .. } => {
            let mut lines = vec![vec![muted("script/"), strong(text.path_of(*script))]];
            if !values.is_empty() {
                let mut line = Vec::new();
                for (index, (name, value)) in values.iter().enumerate() {
                    if index > 0 {
                        line.push(muted(" · "));
                    }
                    line.push(muted(format!("{name} ")));
                    line.extend(var_value_pieces(text, value));
                }
                lines.push(line);
            }
            lines
        }
        Node::Return { value } => match value {
            Some(value) => vec![expression_pieces(text, value)],
            None => vec![vec![muted("back to the caller")]],
        },
        Node::Dialogue(dialogue) => vec![dialogue_pieces(text, dialogue)],
        Node::Effect(effect) => effect_pieces(text, effect),
    }
}

fn effect_pieces(text: &NodeText, effect: &RiftEffect) -> Vec<Vec<SummaryPiece>> {
    match effect {
        RiftEffect::Remember(remember) => {
            let mut line = vec![strong(text.path_of(remember.memory)), muted(" = ")];
            match &remember.to {
                Some(to) => line.extend(expression_pieces(text, to)),
                None => line.push(strong("true")),
            }
            vec![line]
        }
        RiftEffect::Forget(forget) => {
            let memories: Vec<String> = forget.memories.iter().map(|m| text.path_of(*m)).collect();
            vec![vec![strong(memories.join(", ")), muted(" back to zero")]]
        }
        RiftEffect::Items(items) => {
            let mut lines = Vec::new();
            for (stacks, word) in [(&items.take, "take "), (&items.give, "give ")] {
                if !stacks.is_empty() {
                    let mut line = vec![muted(word)];
                    line.extend(stack_pieces(text, stacks));
                    lines.push(line);
                }
            }
            if lines.is_empty() {
                lines.push(vec![muted("nothing changes hands")]);
            }
            lines
        }
        RiftEffect::Warp(warp) => {
            let mut line = match &warp.who {
                Some(who) => expression_pieces(text, who),
                None => vec![strong("the subject")],
            };
            line.push(muted(" to "));
            match &warp.to {
                Fill::Var { var } => line.push(hole(var)),
                Fill::Const(place) => line.push(strong(place_text(text, place))),
            }
            vec![line, vec![muted("sets its spawn point")]]
        }
        other => field_lines(text, other),
    }
}

/// An effect's fields as its definition lists them, read back as the types they may be: files by
/// their path, places, stacks of items, conditions, and the variables filling them.
fn field_lines(text: &NodeText, effect: &RiftEffect) -> Vec<Vec<SummaryPiece>> {
    let Ok(json) = text.reader.to_json(effect) else {
        return Vec::new();
    };
    let fields = text
        .catalogue
        .entry(RiftEffectKind::from(effect).into())
        .map(|entry| entry.fields.clone())
        .unwrap_or_default();
    let mut named: Vec<SummaryPiece> = Vec::new();
    let mut rest: Vec<SummaryPiece> = Vec::new();
    for (field, table) in &fields {
        let Some(value) = json.get(field).filter(|value| !value.is_null()) else {
            continue;
        };
        let (main, piece) = field_pieces(text, field, table.is_some(), value);
        let into = if main { &mut named } else { &mut rest };
        if !into.is_empty() {
            into.push(muted(" · "));
        }
        into.extend(piece);
    }
    [named, rest]
        .into_iter()
        .filter(|line| !line.is_empty())
        .collect()
}

/// A field of an effect, and whether it is one of what the effect is about rather than a detail.
fn field_pieces(
    text: &NodeText,
    field: &str,
    names_a_file: bool,
    value: &Value,
) -> (bool, Vec<SummaryPiece>) {
    let labelled = |pieces: Vec<SummaryPiece>| -> Vec<SummaryPiece> {
        let mut out = vec![muted(format!("{field} "))];
        out.extend(pieces);
        out
    };
    if let Some(name) = var_name(value) {
        return (true, labelled(vec![hole(&name)]));
    }
    if let (true, Value::String(path)) = (names_a_file, value) {
        return (true, vec![strong(shown_path(path))]);
    }
    if let Ok(place) = text.reader.from_json::<Place>(value) {
        return (true, labelled(vec![strong(place_text(text, &place))]));
    }
    if let Ok(stacks) = text.reader.from_json::<Vec<ItemStack>>(value)
        && !stacks.is_empty()
    {
        return (true, labelled(stack_pieces(text, &stacks)));
    }
    match value {
        Value::Number(_) | Value::Bool(_) | Value::String(_) => {
            (false, labelled(vec![strong(value_text(value))]))
        }
        _ => match text.reader.from_json::<Condition>(value) {
            Ok(condition) => (true, labelled(expression_pieces(text, &condition))),
            Err(_) => (true, labelled(vec![strong(value_text(value))])),
        },
    }
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Array(items) => items.iter().map(value_text).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

/// A value a variable takes, as text.
pub fn var_value_text(text: &NodeText, value: &VarValue<RiftScripting>) -> String {
    var_value_pieces(text, value)
        .into_iter()
        .map(|piece| piece.text)
        .collect()
}

fn var_value_pieces(text: &NodeText, value: &VarValue<RiftScripting>) -> Vec<SummaryPiece> {
    match value {
        VarValue::Integer(number) => vec![strong(number.to_string())],
        VarValue::Number(number) => vec![strong(number.to_string())],
        VarValue::Boolean(flag) => vec![strong(flag.to_string())],
        VarValue::Str(name) => vec![strong(name)],
        VarValue::Place(place) => vec![strong(place_text(text, place))],
        VarValue::Condition(condition) => expression_pieces(text, condition),
    }
}

fn dialogue_pieces(text: &NodeText, dialogue: &DialogueNode<RiftScripting>) -> Vec<SummaryPiece> {
    let Some(first) = dialogue.lines.first() else {
        return vec![muted("no lines")];
    };
    let mut line = vec![
        strong(speaker_name(text.reader, first.by)),
        muted(": "),
        strong(words::clip(&spans::plain(&first.text), 48)),
    ];
    if dialogue.lines.len() > 1 {
        line.push(muted(format!(" (+{} lines)", dialogue.lines.len() - 1)));
    }
    if !dialogue.choices.is_empty() {
        line.push(muted(format!(" · {} choices", dialogue.choices.len())));
    }
    line
}

fn stack_pieces(text: &NodeText, stacks: &[ItemStack]) -> Vec<SummaryPiece> {
    let mut line = Vec::new();
    for (index, stack) in stacks.iter().enumerate() {
        if index > 0 {
            line.push(muted(", "));
        }
        line.push(strong(format!(
            "{} {}",
            stack.count,
            text.path_of(stack.item)
        )));
    }
    line
}

fn event_pieces(text: &NodeText, event: &RiftEvent) -> Vec<SummaryPiece> {
    let who = |by| match by {
        Who::Characters => "characters",
        Who::Npcs => "npcs",
        Who::Anyone => "anyone",
    };
    match event {
        RiftEvent::Interacted { verb, reach, .. } => vec![
            muted("verb "),
            strong(verb.label().to_lowercase()),
            muted(" · reach "),
            strong(reach.0.to_string()),
            muted(" m"),
        ],
        RiftEvent::CameNear { within, by, .. } => vec![
            strong(who(*by)),
            muted(" within "),
            strong(within.0.to_string()),
        ],
        RiftEvent::Heard { words, within } => vec![
            strong(words.join(", ")),
            muted(" within "),
            strong(within.0.to_string()),
        ],
        RiftEvent::Entered { by } | RiftEvent::Left { by } | RiftEvent::Died { by } => {
            vec![strong(who(*by))]
        }
        RiftEvent::StoodOn { by, object } => {
            vec![strong(who(*by)), muted(" on "), strong(object)]
        }
        RiftEvent::Interval { seconds } => {
            vec![muted("every "), strong(seconds.0.to_string()), muted(" s")]
        }
        RiftEvent::Bought { item: Some(item) } | RiftEvent::Sold { item: Some(item) } => {
            vec![strong(text.path_of(*item))]
        }
        RiftEvent::Started => vec![muted("the owner spawned")],
        RiftEvent::Run => vec![muted("started by a run node")],
        _ => Vec::new(),
    }
}

pub fn expression_pieces(text: &NodeText, condition: &Condition) -> Vec<SummaryPiece> {
    if let Expr::Var { var } = condition {
        return vec![hole(var)];
    }
    let summary = condition::summary(text.project, condition);
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

/// The variable a value of a script is filled from, when it is filled from one.
pub fn var_name(value: &Value) -> Option<String> {
    match Fill::<Value>::deserialize(value) {
        Ok(Fill::Var { var }) => Some(var),
        _ => None,
    }
}

/// The variables the values within `value` are filled from.
pub fn vars_in(value: &Value, names: &mut Vec<String>) {
    if let Some(name) = var_name(value) {
        if !names.contains(&name) {
            names.push(name);
        }
        return;
    }
    match value {
        Value::Object(map) => map.values().for_each(|inner| vars_in(inner, names)),
        Value::Array(items) => items.iter().for_each(|inner| vars_in(inner, names)),
        _ => {}
    }
}

/// The variables `node` is filled from.
pub fn node_vars(reader: &FolderReader, node: &Node) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(json) = reader.to_json(node) {
        vars_in(&json, &mut names);
    }
    names
}
