use std::collections::{BTreeMap, HashMap, HashSet};
use ui::egui;
use ui::egui_snarl;
use ui::theme;
use ui::words;

use engine::core::script::{Exit, NodeKind};
use engine::systems::dialogue::bust::Expression;
use engine::systems::script::{Line, Node, Speaker};
use engine::systems::text::{Span, plain};
use serde::{Deserialize, Serialize};
use ui::egui::emath::TSTransform;
use ui::egui::text::{LayoutJob, TextFormat};
use ui::egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Stroke, Ui, Vec2};
use ui::egui_snarl::ui::{
    AnyPins, NodeLayout, PinInfo, PinPlacement, SnarlPin, SnarlStyle, SnarlViewer, SnarlWidget,
    WireStyle,
};
use ui::egui_snarl::{InPin, InPinId, NodeId, OutPin, OutPinId, Snarl};
use ui::glyph::Mark;
use ui::ink;

use super::ScriptDoc;
use super::nodes::{self, NodePort, NodeText, SummaryPiece, SummaryTone};
use crate::keymap::EditorKey;
use crate::shell::widgets;
use crate::spans;

const COLUMN: f32 = 240.0;
pub const CARD: f32 = 176.0;
/// Where the graph's first node goes: a node placed nowhere else starts its column from here.
pub const FIRST: Pos2 = Pos2::new(40.0, 40.0);
const FACE: f32 = 20.0;
const PAD: f32 = 10.0;
const ROUNDING: f32 = 6.0;
const MIN_ZOOM: f32 = 0.3;
const MAX_ZOOM: f32 = 2.0;
// The graph is drawn this many times its size and shown scaled down, so its text is rasterised
// sharp up to the largest zoom.
const MAGNIFIED: f32 = MAX_ZOOM;

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
                magnified(Pos2::new(at[0], at[1])),
                ScriptGraphNode { name: name.clone() },
            );
        }
        ScriptGraph {
            snarl,
            synced_generation: None,
        }
    }

    pub fn sync(&mut self, doc: &ScriptDoc, text: &NodeText, generation: u64) {
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
        for (name, at) in self.unplaced(doc, text) {
            self.place(&name, at);
        }
        self.sync_wires(doc);
    }

    pub fn place(&mut self, name: &str, at: Pos2) {
        match self.id_of(name) {
            Some(id) => {
                if let Some(info) = self.snarl.get_node_info_mut(id) {
                    info.pos = magnified(at);
                }
            }
            None => {
                self.snarl.insert_node(
                    magnified(at),
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
            .map(|(_, pos, node)| (node.name.clone(), unmagnified(pos)))
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

    fn unplaced(&self, doc: &ScriptDoc, text: &NodeText) -> Vec<(String, Pos2)> {
        let nodes = doc.nodes();
        let depths = nodes::depths(nodes);
        let present = self.positions();
        let mut next_y: BTreeMap<usize, f32> = BTreeMap::new();
        for (name, at) in &present {
            if let Some(node) = nodes.get(name) {
                let depth = depths.get(name).copied().unwrap_or(0);
                let bottom = at.y + estimated_height(node, text) + 24.0;
                let entry = next_y.entry(depth).or_insert(FIRST.y);
                *entry = entry.max(bottom);
            }
        }
        let mut placed = Vec::new();
        for (name, node) in nodes {
            if present.contains_key(name) {
                continue;
            }
            let depth = depths.get(name).copied().unwrap_or(0);
            let y = next_y.entry(depth).or_insert(FIRST.y);
            let x = FIRST.x + depth as f32 * COLUMN;
            placed.push((name.clone(), Pos2::new(x, *y)));
            *y += estimated_height(node, text) + 24.0;
        }
        placed
    }

    fn sync_wires(&mut self, doc: &ScriptDoc) {
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
            for (output, port) in nodes::ports(node).iter().enumerate() {
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
    pub link: Option<(String, Exit)>,
}

pub enum ScriptGraphAction {
    Connect {
        from: String,
        exit: Exit,
        to: String,
    },
    Disconnect {
        from: String,
        exit: Exit,
    },
    DropInputs(String),
    Delete(Vec<String>),
    Rename(String),
    Duplicate(Vec<String>),
    Palette(NodePaletteOpen),
    AddLine(String),
    AddChoice(String),
    EditLine {
        name: String,
        index: usize,
        text: Vec<Span>,
    },
}

pub struct FaceImage {
    pub texture: egui::TextureId,
    pub uv: Rect,
}

/// The bust each speaker shows in each face their lines name.
pub type SpeakerFaces = HashMap<(Speaker, Option<Expression>), FaceImage>;

pub struct ScriptGraphViewer<'a> {
    pub doc: &'a ScriptDoc,
    pub text: NodeText<'a>,
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

pub fn ports_of(doc: &ScriptDoc) -> HashMap<String, Vec<NodePort>> {
    doc.nodes()
        .iter()
        .map(|(name, node)| (name.clone(), nodes::ports(node)))
        .collect()
}

impl ScriptGraphViewer<'_> {
    fn node(&self, name: &str) -> Option<&Node> {
        self.doc.nodes().get(name)
    }

    fn port(&self, name: &str, index: usize) -> Option<&NodePort> {
        self.ports.get(name).and_then(|ports| ports.get(index))
    }

    fn open_palette(&mut self, at: Pos2, link: Option<(String, Exit)>) {
        self.actions
            .push(ScriptGraphAction::Palette(NodePaletteOpen {
                at: self.to_global * unmagnified(at),
                placed: true,
                link,
            }));
    }

    fn click_to_select(&mut self, ui: &mut Ui, rect: Rect, name: &str) {
        let response = ui.interact(rect, ui.id().with("select"), egui::Sense::click());
        if response.clicked() {
            self.clicks.push(NodeClick {
                name: name.to_owned(),
                extend: ui.input(|i| {
                    EditorKey::ExtendSelection.held(i) || EditorKey::ToggleSelection.held(i)
                }),
            });
        }
    }

    fn dialogue_body(&mut self, ui: &mut Ui, name: &str, lines: &[Line]) {
        let editing = self.selected.len() == 1 && self.selected.contains(name);
        let shown = if editing {
            lines.len()
        } else {
            lines.len().min(3)
        };
        for (index, line) in lines.iter().take(shown).enumerate() {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = big(6.0);
                self.face(ui, line);
                let width = big(CARD - FACE - 6.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(width, 0.0),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |ui| {
                        ui.set_width(width);
                        if editing {
                            let id = ui.id().with(("line", name, index));
                            let edit = spans::line(
                                ui,
                                &line.text,
                                id,
                                FontId::proportional(big(12.5)),
                                "…",
                            );
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
                            let text = words::clip(&spans::plain(&line.text), 64);
                            let job = spans::layout_job(
                                &[plain(text)],
                                FontId::proportional(big(12.5)),
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
            ui.label(theme::faint(format!("… {} more lines", lines.len() - shown)).size(big(11.5)));
        }
        if editing {
            ui.add_space(big(2.0));
            ui.label(
                theme::faint(format!(
                    "{} new line · {} new choice",
                    EditorKey::Confirm.name(),
                    EditorKey::AddChoice.name()
                ))
                .size(big(11.0))
                .monospace(),
            );
        }
    }

    fn face(&self, ui: &mut Ui, line: &Line) {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(big(FACE)), egui::Sense::hover());
        let rect = rect.translate(Vec2::new(0.0, big(1.0)));
        let image = self
            .faces
            .get(&(line.by, line.face.clone()))
            .or_else(|| self.faces.get(&(line.by, None)));
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
                    big(4.0),
                    Stroke::new(big(1.0), theme::BORDER),
                    egui::StrokeKind::Inside,
                );
            }
            None => {
                let glyph = match line.by {
                    Speaker::Npc(_) => "N",
                    Speaker::Object => "I",
                    Speaker::Narrator | Speaker::Player => "–",
                };
                face_placeholder(ui.painter(), rect, glyph);
            }
        }
    }
}

fn face_placeholder(painter: &egui::Painter, rect: Rect, glyph: &str) {
    painter.rect_filled(rect, big(4.0), theme::RAISED);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::monospace(big(11.0)),
        theme::MUTED,
    );
}

pub fn style() -> SnarlStyle {
    let mut style = SnarlStyle::new();
    style.wire_style = Some(WireStyle::Bezier5);
    style.wire_width = Some(1.6);
    style.collapsible = Some(false);
    style.pin_size = Some(8.0);
    style.pin_placement = Some(PinPlacement::Edge);
    style.node_layout = Some(NodeLayout::sandwich());
    style.header_drag_space = Some(Vec2::new(0.0, 0.0));
    style.bg_pattern = Some(egui_snarl::ui::BackgroundPattern::NoPattern);
    style.select_stoke = Some(Stroke::new(1.0, theme::ACCENT));
    style.select_fill = Some(ink::wash(theme::ACCENT, 0.08));
    style.min_scale = Some(MIN_ZOOM);
    style.max_scale = Some(MAX_ZOOM);
    style.crisp_magnified_text = Some(true);
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
        ui.set_width(big(CARD));
        let header = ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = big(6.0);
            let (mark, color) = kind_mark(&node);
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(big(14.0)), egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, big(3.0), ink::wash(color, 0.27));
            mark.paint(ui.painter(), rect.shrink(big(3.0)), color);
            let title = card_title(&name, &node);
            ui.add(egui::Label::new(theme::strong(title).monospace().size(big(12.5))).truncate());
        });
        // The header is laid out after the body, so selecting by its whole room would sit on top
        // of the body's fields and take their clicks.
        self.click_to_select(ui, header.response.rect, &name);
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
        ui.allocate_exact_size(Vec2::splat(1.0), egui::Sense::hover());
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
        let kind = self.node(&name).map(Node::kind);
        let port = self.port(&name, pin.id.output).cloned();
        let label = port
            .as_ref()
            .map(|port| port.label.clone())
            .unwrap_or_default();
        let connected = port.as_ref().is_some_and(|port| port.target.is_some());
        let dialogue_choice = port.as_ref().is_some_and(NodePort::is_choice);
        if dialogue_choice {
            choice_output(ui, &label, connected);
        } else {
            let mut job = LayoutJob::default();
            job.append(
                &label,
                0.0,
                TextFormat::simple(FontId::monospace(big(11.0)), theme::MUTED),
            );
            if !connected {
                job.append(
                    " · end",
                    0.0,
                    TextFormat::simple(FontId::monospace(big(10.5)), theme::FAINT),
                );
            }
            ui.add(egui::Label::new(job).truncate());
        }
        let fill = if !connected {
            theme::FAINT
        } else {
            match kind {
                Some(NodeKind::On) => theme::WARN,
                Some(NodeKind::Dialogue) => DIALOGUE,
                Some(NodeKind::Effect) | None => {
                    let fails = self
                        .node(&name)
                        .is_some_and(|node| self.text.catalogue.fails(node, &label));
                    if fails { REFUSED } else { theme::OK }
                }
                Some(_) => WIRE,
            }
        };
        let wire = if kind == Some(NodeKind::On) {
            theme::WARN
        } else {
            WIRE
        };
        PinInfo::circle().with_fill(fill).with_wire_color(wire)
    }

    fn has_body(&mut self, node: &ScriptGraphNode) -> bool {
        self.node(&node.name).is_some_and(|node| {
            nodes::is_dialogue(node) || !nodes::pieces(node, &self.text).is_empty()
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
        ui.set_width(big(CARD));
        self.click_to_select(ui, ui.max_rect(), &name);
        let Some(node) = self.node(&name).cloned() else {
            return;
        };
        ui.vertical(|ui| {
            ui.set_width(big(CARD));
            ui.spacing_mut().item_spacing.y = big(3.0);
            if let Node::Dialogue(dialogue) = &node {
                self.dialogue_body(ui, &name, &dialogue.lines);
                return;
            }
            for line in nodes::pieces(&node, &self.text) {
                ui.add(egui::Label::new(pieces_job(&line, big(CARD))).wrap());
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
        frame.shadow = magnified_shadow(ui::slip::SHADOW);
        frame.corner_radius = CornerRadius::same(big(ROUNDING) as u8);
        frame.inner_margin = card_margin();
        frame.stroke = if self.current == Some(name.as_str()) {
            Stroke::new(big(2.0), CURRENT)
        } else if self.lit.contains(name) {
            Stroke::new(big(1.5), LIT)
        } else if self.selected.contains(name) {
            Stroke::new(big(1.5), theme::ACCENT)
        } else {
            Stroke::new(big(1.0), theme::BORDER)
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
        frame.shadow = egui::Shadow::NONE;
        let inner = big(ROUNDING - 1.0) as u8;
        frame.corner_radius = CornerRadius {
            nw: inner,
            ne: inner,
            sw: 0,
            se: 0,
        };
        frame.inner_margin = egui::Margin {
            bottom: big(6.0) as i8,
            ..card_margin()
        };
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
            self.rects.insert(
                graph_node.name.clone(),
                TSTransform::from_scaling(MAGNIFIED.recip()) * rect,
            );
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
        let step = big(24.0);
        let x0 = (viewport.min.x / step).floor() * step;
        let y0 = (viewport.min.y / step).floor() * step;
        let mut x = x0;
        while x < viewport.max.x {
            let mut y = y0;
            while y < viewport.max.y {
                painter.circle_filled(Pos2::new(x, y), big(0.9), DOT);
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
                exit: port.exit,
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
                exit: port.exit,
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
        if ui
            .add(widgets::shortcut_button("Add node…", EditorKey::Palette))
            .clicked()
        {
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
                self.open_palette(pos, Some((name, port.exit)));
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
            if ui
                .add(widgets::shortcut_button("Add line", EditorKey::Confirm))
                .clicked()
            {
                self.actions.push(ScriptGraphAction::AddLine(name.clone()));
                ui.close();
            }
            if ui
                .add(widgets::shortcut_button("Add choice", EditorKey::AddChoice))
                .clicked()
            {
                self.actions
                    .push(ScriptGraphAction::AddChoice(name.clone()));
                ui.close();
            }
            ui.separator();
        }
        if ui
            .add(widgets::shortcut_button("Rename…", EditorKey::Rename))
            .clicked()
        {
            self.actions.push(ScriptGraphAction::Rename(name.clone()));
            ui.close();
        }
        if ui
            .add(widgets::shortcut_button("Duplicate", EditorKey::Duplicate))
            .clicked()
        {
            self.actions
                .push(ScriptGraphAction::Duplicate(vec![name.clone()]));
            ui.close();
        }
        if ui
            .add(widgets::shortcut_button("Delete", EditorKey::Delete))
            .clicked()
        {
            self.actions.push(ScriptGraphAction::Delete(vec![name]));
            ui.close();
        }
    }

    fn current_transform(
        &mut self,
        to_global: &mut TSTransform,
        _snarl: &mut Snarl<ScriptGraphNode>,
    ) {
        if let Some((graph, canvas)) = self.fit.take() {
            let zoom = ((canvas.width() - 48.0) / graph.width())
                .min((canvas.height() - 48.0) / graph.height())
                .clamp(MIN_ZOOM, 1.0);
            let translation =
                canvas.min.to_vec2() + Vec2::new(24.0, 40.0) - graph.min.to_vec2() * zoom;
            *to_global = TSTransform::new(translation, zoom / MAGNIFIED);
        }
        self.to_global = *to_global * TSTransform::from_scaling(MAGNIFIED);
    }
}

fn choice_output(ui: &mut Ui, label: &str, connected: bool) {
    ui.allocate_ui_with_layout(
        Vec2::new(big(CARD), big(16.0)),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_width(big(CARD));
            let y = ui.max_rect().min.y - big(1.5);
            ui.painter().line_segment(
                [
                    egui::pos2(ui.max_rect().min.x - big(PAD), y),
                    egui::pos2(ui.max_rect().max.x + big(PAD), y),
                ],
                Stroke::new(big(1.0), theme::BORDER),
            );
            let mut job = LayoutJob::default();
            job.append(
                label,
                0.0,
                TextFormat::simple(FontId::proportional(big(12.0)), theme::TEXT),
            );
            ui.add(egui::Label::new(job).truncate());
            if !connected {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(theme::faint("end").monospace().size(big(10.5)));
                });
            }
        },
    );
}

const CARD_FILL: Color32 = ink::LIGHT;
const DOT: Color32 = ink::INK3;
const WIRE: Color32 = ink::AZURE;
const DIALOGUE: Color32 = ink::TYRIAN;
const REFUSED: Color32 = ink::INK3;
const LIT: Color32 = ink::GOLD;
const CURRENT: Color32 = ink::GOLD2;
const RUN: Color32 = ink::RUBRIC;
const EFFECT_FILL: Color32 = Color32::from_rgb(221, 219, 194);

fn kind_fill(node: &Node) -> Color32 {
    match node.kind() {
        NodeKind::On => Color32::from_rgb(225, 211, 180),
        NodeKind::Run => Color32::from_rgb(233, 211, 188),
        NodeKind::Dialogue => Color32::from_rgb(226, 210, 199),
        NodeKind::Effect => EFFECT_FILL,
        NodeKind::If
        | NodeKind::Random
        | NodeKind::Repeat
        | NodeKind::Wait
        | NodeKind::Set
        | NodeKind::Return => Color32::from_rgb(221, 216, 202),
    }
}

pub fn kind_mark(node: &Node) -> (Mark, Color32) {
    match node.kind() {
        NodeKind::On => (Mark::Play, theme::WARN),
        NodeKind::Run => (Mark::CaretRight, RUN),
        NodeKind::Dialogue => (Mark::Dot, DIALOGUE),
        NodeKind::Effect => (Mark::Warning, theme::OK),
        NodeKind::If
        | NodeKind::Random
        | NodeKind::Repeat
        | NodeKind::Wait
        | NodeKind::Set
        | NodeKind::Return => (Mark::Edit, WIRE),
    }
}

pub fn card_title(name: &str, node: &Node) -> String {
    match node.kind() {
        NodeKind::On => format!("on {}", nodes::event_name(node).unwrap_or("?")),
        NodeKind::Dialogue => name.to_owned(),
        _ => nodes::tag_of(node).to_owned(),
    }
}

fn pieces_job(pieces: &[SummaryPiece], width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    for piece in pieces {
        let format = match piece.tone {
            SummaryTone::Muted => TextFormat::simple(FontId::proportional(big(12.0)), theme::MUTED),
            SummaryTone::Strong => {
                TextFormat::simple(FontId::new(big(12.0), theme::medium()), theme::TEXT)
            }
            SummaryTone::Var => {
                let mut format = TextFormat::simple(FontId::monospace(big(11.0)), DIALOGUE);
                format.background = ink::wash(DIALOGUE, 0.12);
                format
            }
        };
        job.append(&piece.text, 0.0, format);
    }
    job
}

fn estimated_height(node: &Node, text: &NodeText) -> f32 {
    let outputs = nodes::ports(node).len() as f32;
    if let Node::Dialogue(dialogue) = node {
        let lines = dialogue.lines.len().min(3) as f32;
        60.0 + lines * 44.0 + outputs * 20.0
    } else {
        let lines = nodes::pieces(node, text).len() as f32;
        60.0 + lines * 18.0 + outputs * 20.0
    }
}

fn big(size: f32) -> f32 {
    size * MAGNIFIED
}

fn magnified(at: Pos2) -> Pos2 {
    (at.to_vec2() * MAGNIFIED).to_pos2()
}

fn unmagnified(at: Pos2) -> Pos2 {
    (at.to_vec2() / MAGNIFIED).to_pos2()
}

fn magnified_shadow(shadow: egui::Shadow) -> egui::Shadow {
    let scaled = |value: f32| big(value).min(f32::from(u8::MAX)) as u8;
    egui::Shadow {
        offset: [
            big(f32::from(shadow.offset[0])) as i8,
            big(f32::from(shadow.offset[1])) as i8,
        ],
        blur: scaled(f32::from(shadow.blur)),
        spread: scaled(f32::from(shadow.spread)),
        color: shadow.color,
    }
}

fn card_margin() -> egui::Margin {
    egui::Margin {
        left: big(PAD) as i8,
        right: big(PAD) as i8,
        top: big(5.0) as i8,
        bottom: big(8.0) as i8,
    }
}
