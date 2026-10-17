use std::collections::{BTreeMap, HashMap, HashSet};

use egui::emath::TSTransform;
use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Stroke, Ui, Vec2};
use egui_snarl::ui::{
    AnyPins, NodeLayout, PinInfo, SnarlPin, SnarlStyle, SnarlViewer, SnarlWidget, WireStyle,
};
use egui_snarl::{InPin, InPinId, NodeId, OutPin, OutPinId, Snarl};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ScriptDoc;
use super::nodes::{
    self, FlowKind, LineSpeaker, NodeCatalogue, NodePort, SummaryPiece, SummaryTone,
};
use crate::spans;
use crate::ui::{theme, widgets};
use crate::value::RowValue;

const COLUMN: f32 = 240.0;
pub const CARD: f32 = 176.0;
const FACE: f32 = 20.0;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ScriptGraphLayout {
    pub nodes: BTreeMap<String, [f32; 2]>,
    pub outline: bool,
}

impl ScriptGraphLayout {
    pub fn of(graph: &ScriptGraph, outline: bool) -> ScriptGraphLayout {
        ScriptGraphLayout {
            nodes: graph
                .positions()
                .into_iter()
                .map(|(name, at)| (name, [at.x, at.y]))
                .collect(),
            outline,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ScriptGraphNode {
    pub name: String,
}

pub struct ScriptGraph {
    pub snarl: Snarl<ScriptGraphNode>,
    synced_generation: Option<u64>,
}

impl Default for ScriptGraph {
    fn default() -> Self {
        ScriptGraph::new(&ScriptGraphLayout::default())
    }
}

impl ScriptGraph {
    pub fn new(layout: &ScriptGraphLayout) -> ScriptGraph {
        let mut snarl = Snarl::new();
        for (name, at) in &layout.nodes {
            snarl.insert_node(
                Pos2::new(at[0], at[1]),
                ScriptGraphNode { name: name.clone() },
            );
        }
        ScriptGraph {
            snarl,
            synced_generation: None,
        }
    }

    pub fn sync(&mut self, doc: &ScriptDoc, catalogue: &NodeCatalogue, generation: u64) {
        if self.synced_generation == Some(generation) {
            return;
        }
        self.synced_generation = Some(generation);
        let names = doc.nodes();
        let stale: Vec<NodeId> = self
            .snarl
            .nodes_ids_data()
            .filter(|(_, node)| !names.contains_key(&node.value.name))
            .map(|(id, _)| id)
            .collect();
        for id in stale {
            self.snarl.remove_node(id);
        }
        for (name, at) in self.unplaced(doc, catalogue) {
            self.place(&name, at);
        }
        self.sync_wires(doc, catalogue);
    }

    pub fn place(&mut self, name: &str, at: Pos2) {
        match self.id_of(name) {
            Some(id) => {
                if let Some(info) = self.snarl.get_node_info_mut(id) {
                    info.pos = at;
                }
            }
            None => {
                self.snarl.insert_node(
                    at,
                    ScriptGraphNode {
                        name: name.to_owned(),
                    },
                );
            }
        }
    }

    pub fn rename(&mut self, old: &str, new: &str) {
        if let Some(node) = self.id_of(old).and_then(|id| self.snarl.get_node_mut(id)) {
            node.name = new.to_owned();
        }
    }

    pub fn positions(&self) -> BTreeMap<String, Pos2> {
        self.snarl
            .nodes_pos_ids()
            .map(|(_, pos, node)| (node.name.clone(), pos))
            .collect()
    }

    pub fn node_spots(&self) -> Vec<Pos2> {
        self.snarl.nodes_pos().map(|(pos, _)| pos).collect()
    }

    fn id_of(&self, name: &str) -> Option<NodeId> {
        self.snarl
            .nodes_ids_data()
            .find(|(_, node)| node.value.name == name)
            .map(|(id, _)| id)
    }

    fn unplaced(&self, doc: &ScriptDoc, catalogue: &NodeCatalogue) -> Vec<(String, Pos2)> {
        let nodes = doc.nodes();
        let depths = nodes::depths(nodes, catalogue);
        let present = self.positions();
        let mut next_y: BTreeMap<usize, f32> = BTreeMap::new();
        for (name, at) in &present {
            if let Some(node) = nodes.get(name) {
                let depth = depths.get(name).copied().unwrap_or(0);
                let bottom = at.y + estimated_height(node, catalogue) + 24.0;
                let entry = next_y.entry(depth).or_insert(40.0);
                *entry = entry.max(bottom);
            }
        }
        let mut placed = Vec::new();
        for (name, node) in nodes {
            if present.contains_key(name) {
                continue;
            }
            let depth = depths.get(name).copied().unwrap_or(0);
            let y = next_y.entry(depth).or_insert(40.0);
            let x = 40.0 + depth as f32 * COLUMN;
            placed.push((name.clone(), Pos2::new(x, *y)));
            *y += estimated_height(node, catalogue) + 24.0;
        }
        placed
    }

    fn sync_wires(&mut self, doc: &ScriptDoc, catalogue: &NodeCatalogue) {
        let ids: HashMap<String, NodeId> = self
            .snarl
            .nodes_ids_data()
            .map(|(id, node)| (node.value.name.clone(), id))
            .collect();
        let mut wanted = HashSet::new();
        for (name, node) in doc.nodes() {
            let Some(&from) = ids.get(name) else {
                continue;
            };
            for (output, port) in nodes::ports(node, catalogue).iter().enumerate() {
                let Some(&to) = port.target.as_ref().and_then(|target| ids.get(target)) else {
                    continue;
                };
                wanted.insert((
                    OutPinId { node: from, output },
                    InPinId { node: to, input: 0 },
                ));
            }
        }
        let present: HashSet<(OutPinId, InPinId)> = self.snarl.wires().collect();
        for (out, pin) in present.difference(&wanted) {
            self.snarl.disconnect(*out, *pin);
        }
        for (out, pin) in wanted.difference(&present) {
            self.snarl.connect(*out, *pin);
        }
    }
}

#[derive(Clone, Debug)]
pub struct NodePaletteOpen {
    pub at: Pos2,
    pub placed: bool,
    pub link: Option<(String, String)>,
}

pub enum ScriptGraphAction {
    Connect {
        from: String,
        field: String,
        to: String,
    },
    Disconnect {
        from: String,
        field: String,
    },
    DropInputs(String),
    Delete(String),
    Rename(String),
    Duplicate(String),
    Palette(NodePaletteOpen),
    AddLine(String),
    AddChoice(String),
    EditLine {
        name: String,
        index: usize,
        text: Value,
    },
}

pub struct FaceImage {
    pub texture: egui::TextureId,
    pub uv: Rect,
}

pub type SpeakerFaces = HashMap<LineSpeaker, FaceImage>;

pub struct ScriptGraphViewer<'a> {
    pub doc: &'a ScriptDoc,
    pub catalogue: &'a NodeCatalogue,
    pub ports: HashMap<String, Vec<NodePort>>,
    pub lit: &'a HashSet<String>,
    pub current: Option<&'a str>,
    pub selected: &'a HashSet<String>,
    pub faces: &'a SpeakerFaces,
    pub actions: Vec<ScriptGraphAction>,
    pub clicks: Vec<NodeClick>,
    pub to_global: TSTransform,
    pub rects: HashMap<String, Rect>,
    pub fit: Option<(Rect, Rect)>,
}

pub struct NodeClick {
    pub name: String,
    pub extend: bool,
}

pub fn ports_of(doc: &ScriptDoc, catalogue: &NodeCatalogue) -> HashMap<String, Vec<NodePort>> {
    doc.nodes()
        .iter()
        .map(|(name, node)| (name.clone(), nodes::ports(node, catalogue)))
        .collect()
}

impl ScriptGraphViewer<'_> {
    fn node(&self, name: &str) -> Option<&Value> {
        self.doc.nodes().get(name)
    }

    fn port(&self, name: &str, index: usize) -> Option<&NodePort> {
        self.ports.get(name).and_then(|ports| ports.get(index))
    }

    fn open_palette(&mut self, at: Pos2, link: Option<(String, String)>) {
        self.actions
            .push(ScriptGraphAction::Palette(NodePaletteOpen {
                at: self.to_global * at,
                placed: true,
                link,
            }));
    }

    fn click_to_select(&mut self, ui: &mut Ui, name: &str) {
        let response = ui.interact(ui.max_rect(), ui.id().with("select"), egui::Sense::click());
        if response.clicked() {
            self.clicks.push(NodeClick {
                name: name.to_owned(),
                extend: ui.input(|i| i.modifiers.shift || i.modifiers.command),
            });
        }
    }

    fn dialogue_body(&mut self, ui: &mut Ui, name: &str, node: &Value) {
        let editing = self.selected.len() == 1 && self.selected.contains(name);
        let lines = node["lines"].as_array().cloned().unwrap_or_default();
        let shown = if editing {
            lines.len()
        } else {
            lines.len().min(3)
        };
        for (index, line) in lines.iter().take(shown).enumerate() {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                self.face(ui, &line["by"]);
                let width = CARD - FACE - 6.0;
                ui.allocate_ui_with_layout(
                    Vec2::new(width, 0.0),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |ui| {
                        ui.set_width(width);
                        if editing {
                            let id = ui.id().with(("line", name, index));
                            let edit =
                                spans::line(ui, &line["text"], id, FontId::proportional(12.5), "…");
                            if let Some(text) = edit.spans {
                                self.actions.push(ScriptGraphAction::EditLine {
                                    name: name.to_owned(),
                                    index,
                                    text,
                                });
                            }
                            if edit.entered {
                                self.actions
                                    .push(ScriptGraphAction::AddLine(name.to_owned()));
                            }
                        } else {
                            let text = widgets::clip(&spans::plain(&line["text"]), 64);
                            let job = spans::layout_job(
                                &Value::Array(vec![Value::String(text)]),
                                FontId::proportional(12.5),
                                theme::TEXT,
                                width,
                            );
                            ui.add(egui::Label::new(job));
                        }
                    },
                );
            });
        }
        if lines.len() > shown {
            ui.label(theme::faint(format!("… {} more lines", lines.len() - shown)).size(11.5));
        }
        if editing {
            ui.add_space(2.0);
            ui.label(
                theme::faint("Enter new line · Ctrl Enter new choice")
                    .size(11.0)
                    .monospace(),
            );
        }
    }

    fn face(&self, ui: &mut Ui, by: &Value) {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(FACE), egui::Sense::hover());
        let rect = rect.translate(Vec2::new(0.0, 1.0));
        let speaker = LineSpeaker::of(by);
        let image = speaker.as_ref().and_then(|speaker| {
            self.faces
                .get(speaker)
                .or_else(|| self.faces.get(&speaker.faceless()))
        });
        match image {
            Some(image) => {
                ui.painter().add(egui::Shape::image(
                    image.texture,
                    rect,
                    image.uv,
                    Color32::WHITE,
                ));
                ui.painter().rect_stroke(
                    rect,
                    4.0,
                    Stroke::new(1.0, theme::BORDER),
                    egui::StrokeKind::Inside,
                );
            }
            None => {
                let glyph = match speaker {
                    Some(LineSpeaker::Npc { .. }) => "N",
                    Some(LineSpeaker::Object) => "I",
                    _ => "–",
                };
                face_placeholder(ui.painter(), rect, glyph);
            }
        }
    }
}

pub fn face_placeholder(painter: &egui::Painter, rect: Rect, glyph: &str) {
    painter.rect_filled(rect, 4.0, theme::RAISED);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::monospace(11.0),
        theme::MUTED,
    );
}

pub fn style() -> SnarlStyle {
    let mut style = SnarlStyle::new();
    style.wire_style = Some(WireStyle::Bezier5);
    style.wire_width = Some(1.6);
    style.collapsible = Some(false);
    style.pin_size = Some(8.0);
    style.node_layout = Some(NodeLayout::sandwich());
    style.header_drag_space = Some(Vec2::new(0.0, 0.0));
    style.bg_pattern = Some(egui_snarl::ui::BackgroundPattern::NoPattern);
    style.select_stoke = Some(Stroke::new(1.0, theme::ACCENT));
    style.select_fill = Some(theme::alpha(theme::ACCENT, 20));
    style.min_scale = Some(0.3);
    style.max_scale = Some(2.5);
    style
}

pub fn widget(id: egui::Id) -> SnarlWidget {
    SnarlWidget::new().id(id).style(style())
}

impl SnarlViewer<ScriptGraphNode> for ScriptGraphViewer<'_> {
    fn title(&mut self, node: &ScriptGraphNode) -> String {
        node.name.clone()
    }

    fn show_header(
        &mut self,
        id: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<ScriptGraphNode>,
    ) {
        let name = snarl[id].name.clone();
        let Some(node) = self.node(&name).cloned() else {
            return;
        };
        ui.set_width(CARD);
        self.click_to_select(ui, &name);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (glyph, color) = kind_glyph(&node);
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 3.0, theme::alpha(color, 70));
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                glyph,
                FontId::proportional(10.0),
                color,
            );
            let title = card_title(&name, &node);
            ui.add(egui::Label::new(theme::strong(title).monospace().size(12.5)).truncate());
        });
    }

    fn inputs(&mut self, node: &ScriptGraphNode) -> usize {
        usize::from(self.node(&node.name).is_some_and(nodes::has_input))
    }

    fn outputs(&mut self, node: &ScriptGraphNode) -> usize {
        self.ports.get(&node.name).map_or(0, Vec::len)
    }

    fn show_input(
        &mut self,
        pin: &InPin,
        ui: &mut Ui,
        snarl: &mut Snarl<ScriptGraphNode>,
    ) -> impl SnarlPin + 'static {
        let name = &snarl[pin.id.node].name;
        let lit = self.lit.contains(name);
        ui.allocate_exact_size(Vec2::new(1.0, 1.0), egui::Sense::hover());
        let fill = if lit {
            LIT
        } else if pin.remotes.is_empty() {
            theme::FAINT
        } else {
            WIRE
        };
        PinInfo::circle().with_fill(fill).with_wire_color(WIRE)
    }

    fn show_output(
        &mut self,
        pin: &OutPin,
        ui: &mut Ui,
        snarl: &mut Snarl<ScriptGraphNode>,
    ) -> impl SnarlPin + 'static {
        let name = snarl[pin.id.node].name.clone();
        let flow = self.node(&name).and_then(FlowKind::of);
        let port = self.port(&name, pin.id.output).cloned();
        let label = port
            .as_ref()
            .map(|port| port.label.clone())
            .unwrap_or_default();
        let connected = port.as_ref().is_some_and(|port| port.target.is_some());
        let dialogue_choice = flow == Some(FlowKind::Dialogue)
            && port
                .as_ref()
                .is_some_and(|port| port.field.starts_with("/choices/"));
        if dialogue_choice {
            choice_output(ui, &label, connected);
        } else {
            let mut job = LayoutJob::default();
            job.append(
                &label,
                0.0,
                TextFormat::simple(FontId::monospace(11.0), theme::MUTED),
            );
            if !connected {
                job.append(
                    " · end",
                    0.0,
                    TextFormat::simple(FontId::monospace(10.5), theme::FAINT),
                );
            }
            ui.add(egui::Label::new(job).truncate());
        }
        let fill = if !connected {
            theme::FAINT
        } else {
            match flow {
                Some(FlowKind::On) => theme::WARN,
                Some(FlowKind::Dialogue) => DIALOGUE,
                Some(_) => WIRE,
                None => match label.as_str() {
                    "refused" | "blocked" | "lost" | "declined" | "cancelled" | "left"
                    | "interrupted" => REFUSED,
                    _ => theme::OK,
                },
            }
        };
        let wire = if flow == Some(FlowKind::On) {
            theme::WARN
        } else {
            WIRE
        };
        PinInfo::circle().with_fill(fill).with_wire_color(wire)
    }

    fn has_body(&mut self, node: &ScriptGraphNode) -> bool {
        self.node(&node.name).is_some_and(|node| {
            nodes::is_dialogue(node) || !nodes::pieces(node, self.catalogue).is_empty()
        })
    }

    fn show_body(
        &mut self,
        id: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<ScriptGraphNode>,
    ) {
        let name = snarl[id].name.clone();
        ui.set_width(CARD);
        self.click_to_select(ui, &name);
        let Some(node) = self.node(&name).cloned() else {
            return;
        };
        ui.vertical(|ui| {
            ui.set_width(CARD);
            ui.spacing_mut().item_spacing.y = 3.0;
            if nodes::is_dialogue(&node) {
                self.dialogue_body(ui, &name, &node);
                return;
            }
            for line in nodes::pieces(&node, self.catalogue) {
                ui.add(egui::Label::new(pieces_job(&line, CARD)).wrap());
            }
        });
    }

    fn node_frame(
        &mut self,
        default: egui::Frame,
        id: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        snarl: &Snarl<ScriptGraphNode>,
    ) -> egui::Frame {
        let name = &snarl[id].name;
        let mut frame = default;
        frame.fill = CARD_FILL;
        frame.corner_radius = CornerRadius::same(6);
        frame.inner_margin = egui::Margin::symmetric(0, 0);
        frame.stroke = if self.current == Some(name.as_str()) {
            Stroke::new(2.0, CURRENT)
        } else if self.lit.contains(name) {
            Stroke::new(1.5, LIT)
        } else if self.selected.contains(name) {
            Stroke::new(1.5, theme::ACCENT)
        } else {
            Stroke::new(1.0, theme::BORDER)
        };
        frame
    }

    fn header_frame(
        &mut self,
        default: egui::Frame,
        id: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        snarl: &Snarl<ScriptGraphNode>,
    ) -> egui::Frame {
        let name = &snarl[id].name;
        let mut frame = default;
        frame.fill = self.node(name).map_or(EFFECT_FILL, kind_fill);
        frame.stroke = Stroke::NONE;
        frame.corner_radius = CornerRadius {
            nw: 6,
            ne: 6,
            sw: 0,
            se: 0,
        };
        frame.inner_margin = egui::Margin::symmetric(8, 5);
        frame
    }

    fn final_node_rect(
        &mut self,
        node: NodeId,
        rect: Rect,
        _ui: &mut Ui,
        snarl: &mut Snarl<ScriptGraphNode>,
    ) {
        if let Some(graph_node) = snarl.get_node(node) {
            self.rects.insert(graph_node.name.clone(), rect);
        }
    }

    fn draw_background(
        &mut self,
        _background: Option<&egui_snarl::ui::BackgroundPattern>,
        viewport: &Rect,
        _snarl_style: &SnarlStyle,
        _style: &egui::Style,
        painter: &egui::Painter,
        _snarl: &Snarl<ScriptGraphNode>,
    ) {
        let step = 24.0;
        let x0 = (viewport.min.x / step).floor() * step;
        let y0 = (viewport.min.y / step).floor() * step;
        let mut x = x0;
        while x < viewport.max.x {
            let mut y = y0;
            while y < viewport.max.y {
                painter.circle_filled(Pos2::new(x, y), 0.9, DOT);
                y += step;
            }
            x += step;
        }
    }

    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<ScriptGraphNode>) {
        let from_name = snarl[from.id.node].name.clone();
        let to_name = snarl[to.id.node].name.clone();
        if let Some(port) = self.port(&from_name, from.id.output) {
            self.actions.push(ScriptGraphAction::Connect {
                from: from_name,
                field: port.field.clone(),
                to: to_name,
            });
        }
    }

    fn disconnect(&mut self, from: &OutPin, _to: &InPin, snarl: &mut Snarl<ScriptGraphNode>) {
        self.drop_outputs(from, snarl);
    }

    fn drop_outputs(&mut self, pin: &OutPin, snarl: &mut Snarl<ScriptGraphNode>) {
        let from_name = snarl[pin.id.node].name.clone();
        if let Some(port) = self.port(&from_name, pin.id.output) {
            self.actions.push(ScriptGraphAction::Disconnect {
                from: from_name,
                field: port.field.clone(),
            });
        }
    }

    fn drop_inputs(&mut self, pin: &InPin, snarl: &mut Snarl<ScriptGraphNode>) {
        let name = snarl[pin.id.node].name.clone();
        self.actions.push(ScriptGraphAction::DropInputs(name));
    }

    fn has_graph_menu(&mut self, _pos: Pos2, _snarl: &mut Snarl<ScriptGraphNode>) -> bool {
        true
    }

    fn show_graph_menu(&mut self, pos: Pos2, ui: &mut Ui, _snarl: &mut Snarl<ScriptGraphNode>) {
        if ui.button("Add node…  (Tab)").clicked() {
            self.open_palette(pos, None);
            ui.close();
        }
    }

    fn has_dropped_wire_menu(
        &mut self,
        _src_pins: AnyPins,
        _snarl: &mut Snarl<ScriptGraphNode>,
    ) -> bool {
        true
    }

    fn show_dropped_wire_menu(
        &mut self,
        pos: Pos2,
        ui: &mut Ui,
        src_pins: AnyPins,
        snarl: &mut Snarl<ScriptGraphNode>,
    ) {
        if let AnyPins::Out(pins) = src_pins
            && let Some(pin) = pins.first()
        {
            let name = snarl[pin.node].name.clone();
            if let Some(port) = self.port(&name, pin.output).cloned() {
                self.open_palette(pos, Some((name, port.field)));
            }
        }
        ui.close();
    }

    fn has_node_menu(&mut self, _node: &ScriptGraphNode) -> bool {
        true
    }

    fn show_node_menu(
        &mut self,
        id: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<ScriptGraphNode>,
    ) {
        let name = snarl[id].name.clone();
        if self.node(&name).is_some_and(nodes::is_dialogue) {
            if ui.button("Add line  (Enter)").clicked() {
                self.actions.push(ScriptGraphAction::AddLine(name.clone()));
                ui.close();
            }
            if ui.button("Add choice  (Ctrl Enter)").clicked() {
                self.actions
                    .push(ScriptGraphAction::AddChoice(name.clone()));
                ui.close();
            }
            ui.separator();
        }
        if ui.button("Rename…  (F2)").clicked() {
            self.actions.push(ScriptGraphAction::Rename(name.clone()));
            ui.close();
        }
        if ui.button("Duplicate  (Ctrl D)").clicked() {
            self.actions
                .push(ScriptGraphAction::Duplicate(name.clone()));
            ui.close();
        }
        if ui.button("Delete  (Del)").clicked() {
            self.actions.push(ScriptGraphAction::Delete(name));
            ui.close();
        }
    }

    fn current_transform(
        &mut self,
        to_global: &mut TSTransform,
        _snarl: &mut Snarl<ScriptGraphNode>,
    ) {
        if let Some((graph, canvas)) = self.fit.take() {
            let scale = ((canvas.width() - 48.0) / graph.width())
                .min((canvas.height() - 48.0) / graph.height())
                .clamp(0.3, 1.0);
            let translation =
                canvas.min.to_vec2() + Vec2::new(24.0, 40.0) - graph.min.to_vec2() * scale;
            *to_global = TSTransform::new(translation, scale);
        }
        self.to_global = *to_global;
    }
}

fn choice_output(ui: &mut Ui, label: &str, connected: bool) {
    ui.allocate_ui_with_layout(
        Vec2::new(CARD - 14.0, 16.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_width(CARD - 14.0);
            let y = ui.max_rect().min.y - 1.5;
            ui.painter().line_segment(
                [
                    egui::pos2(ui.max_rect().min.x, y),
                    egui::pos2(ui.max_rect().max.x + 10.0, y),
                ],
                Stroke::new(1.0, theme::BORDER),
            );
            let mut job = LayoutJob::default();
            job.append(
                label,
                0.0,
                TextFormat::simple(FontId::proportional(12.0), theme::TEXT),
            );
            ui.add(egui::Label::new(job).truncate());
            if !connected {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(theme::faint("end").monospace().size(10.5));
                });
            }
        },
    );
}

const CARD_FILL: Color32 = Color32::from_rgb(30, 31, 38);
const DOT: Color32 = Color32::from_rgb(44, 45, 54);
const WIRE: Color32 = Color32::from_rgb(120, 160, 240);
const DIALOGUE: Color32 = Color32::from_rgb(176, 140, 230);
const REFUSED: Color32 = Color32::from_rgb(150, 150, 164);
const LIT: Color32 = Color32::from_rgb(250, 200, 80);
const CURRENT: Color32 = Color32::from_rgb(255, 240, 140);
const RUN: Color32 = Color32::from_rgb(240, 150, 70);
const EFFECT_FILL: Color32 = Color32::from_rgb(30, 54, 42);

fn kind_fill(node: &Value) -> Color32 {
    match FlowKind::of(node) {
        Some(FlowKind::On) => Color32::from_rgb(58, 50, 26),
        Some(FlowKind::Run) => Color32::from_rgb(64, 44, 30),
        Some(FlowKind::Dialogue) => Color32::from_rgb(54, 42, 74),
        Some(_) => Color32::from_rgb(38, 44, 62),
        None => EFFECT_FILL,
    }
}

pub fn kind_glyph(node: &Value) -> (&'static str, Color32) {
    match FlowKind::of(node) {
        Some(FlowKind::On) => ("▶", theme::WARN),
        Some(FlowKind::Run) => ("▸", RUN),
        Some(FlowKind::Dialogue) => ("●", DIALOGUE),
        Some(_) => ("◆", WIRE),
        None => ("!", theme::OK),
    }
}

pub fn card_title(name: &str, node: &Value) -> String {
    match FlowKind::of(node) {
        Some(FlowKind::On) => format!("on {}", nodes::event_of(node).unwrap_or("?")),
        Some(FlowKind::Dialogue) => name.to_owned(),
        _ => node.kind().to_owned(),
    }
}

pub fn pieces_job(pieces: &[SummaryPiece], width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    for piece in pieces {
        let format = match piece.tone {
            SummaryTone::Muted => TextFormat::simple(FontId::proportional(12.0), theme::MUTED),
            SummaryTone::Strong => {
                TextFormat::simple(FontId::new(12.0, theme::medium()), theme::TEXT)
            }
            SummaryTone::Var => {
                let mut format = TextFormat::simple(FontId::monospace(11.0), DIALOGUE);
                format.background = theme::alpha(DIALOGUE, 30);
                format
            }
        };
        job.append(&piece.text, 0.0, format);
    }
    job
}

fn estimated_height(node: &Value, catalogue: &NodeCatalogue) -> f32 {
    let outputs = nodes::ports(node, catalogue).len() as f32;
    if nodes::is_dialogue(node) {
        let lines = node["lines"].as_array().map_or(0, Vec::len).min(3) as f32;
        44.0 + lines * 30.0 + outputs * 17.0
    } else {
        let lines = nodes::pieces(node, catalogue).len() as f32;
        44.0 + lines * 16.0 + outputs * 15.0
    }
}
