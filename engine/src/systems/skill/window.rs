use std::collections::BTreeMap;

use bevy::prelude::*;
use design_tokens::{HAIRLINE, TextRole, color, widgets};
use egui::scroll_area::{DragScroll, ScrollBarVisibility, ScrollSource};
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2};

use super::card::{self, NodeAccess, SkillCard};
use super::{LearnSkillsRequest, Learned, Rank, SkillId, SkillKind};
use crate::core::content::Content;
use crate::core::gui;
use crate::core::text::{StyledLine, TextPiece};
use crate::systems::class::{self, ClassDef, SkillPoints};
use crate::systems::hud::HudWindow;
use crate::systems::input::map::{self, InputAction, input};
use crate::systems::interface::{InterfaceIcon, InterfaceSound};
use crate::systems::notification;
use crate::systems::player::session;
use crate::systems::skill::bar;

const WINDOW_SIZE: Vec2 = Vec2::new(560.0, 440.0);
const NODE_RADIUS: f32 = 24.0;
const NODE_ICON: f32 = 28.0;
const GRANTED_ICON: f32 = 28.0;
const ARROW_HEAD: f32 = 7.0;
const DOCK_HEIGHT: f32 = 34.0;

pub struct SkillsWindow;

impl HudWindow for SkillsWindow {
    fn title(&self) -> &'static str {
        "Skills"
    }
    fn toggle(&self) -> InputAction {
        InputAction::ToggleSkills
    }
    fn icon(&self) -> InterfaceIcon {
        InterfaceIcon::Skills
    }
    fn size(&self) -> Vec2 {
        WINDOW_SIZE
    }
    fn body_limit(&self, world: &World) -> Option<Vec2> {
        let sheet = Sheet::of(world)?;
        let canvas = sheet.class.get(world.resource::<Content>()).canvas()?;
        let view = world.resource::<TreeView>();
        Some(canvas.size() + view.body - view.viewport.size())
    }
    fn show(&self, ui: &mut Ui, world: &mut World) {
        let Some(sheet) = Sheet::of(world) else {
            ui.label(RichText::new("No class yet").color(color::MUTED));
            return;
        };
        header(ui, world, &sheet);
        let room = ui.available_rect_before_wrap();
        let viewport =
            Rect::from_min_max(room.min, Pos2::new(room.max.x, room.max.y - DOCK_HEIGHT));
        let panned = tree(ui, world, &sheet, viewport);
        *world.resource_mut::<TreeView>() = TreeView {
            body: ui.max_rect().size(),
            viewport,
            content: panned.content,
            offset: panned.offset,
        };
        dock(ui, world, &sheet, room);
    }
}

/// How the tree sat in its window when last drawn: the window's body, the viewport the tree
/// pans within, the canvas shown there and how far it is panned. The window may grow until
/// the viewport takes the whole canvas, and no further.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct TreeView {
    pub body: Vec2,
    pub viewport: Rect,
    pub content: Vec2,
    pub offset: Vec2,
}

impl Default for TreeView {
    fn default() -> TreeView {
        TreeView {
            body: Vec2::ZERO,
            viewport: Rect::ZERO,
            content: Vec2::ZERO,
            offset: Vec2::ZERO,
        }
    }
}

struct Panned {
    content: Vec2,
    offset: Vec2,
}

/// The ranks picked in the window and not yet confirmed, one rank per node.
#[derive(Resource, Default, Clone, Debug, PartialEq)]
pub struct Plan(pub BTreeMap<String, Rank>);

/// A skill being dragged out of the tree or an item card, toward a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CarriedSkill(pub SkillId);

/// Picks the server has since learned, or that lost their footing, leave the plan.
pub fn settle_plan(world: &mut World) {
    let Some(sheet) = Sheet::of(world) else {
        world.resource_mut::<Plan>().0.clear();
        return;
    };
    let content = world.resource::<Content>().clone();
    let def = sheet.class.get(&content);
    let mut plan = world.resource::<Plan>().clone();
    plan.0
        .retain(|key, rank| sheet.learned.rank(key) < *rank && def.tree.node(key).is_some());
    if plan != *world.resource::<Plan>() {
        *world.resource_mut::<Plan>() = plan;
    }
}

pub fn forget_plan(mut plan: ResMut<Plan>) {
    plan.0.clear();
}

struct Sheet {
    me: Entity,
    class: class::ClassId,
    learned: Learned,
    level: u32,
    to_spend: SkillPoints,
}

impl Sheet {
    fn of(world: &World) -> Option<Sheet> {
        let me = session::me(world)?;
        let class = class::of(world, me)?;
        Some(Sheet {
            me,
            class,
            learned: world.get::<Learned>(me).cloned().unwrap_or_default(),
            level: class::level(world, me),
            to_spend: class::points_to_spend(world, me),
        })
    }

    fn planned_ranks(&self, plan: &Plan) -> BTreeMap<String, Rank> {
        let mut ranks = self.learned.0.clone();
        for (key, rank) in &plan.0 {
            ranks.insert(key.clone(), *rank);
        }
        ranks
    }

    fn picked_cost(&self, content: &Content, def: &ClassDef, plan: &Plan) -> SkillPoints {
        plan.0
            .iter()
            .filter_map(|(key, rank)| {
                let skill = def.tree.node(key)?.skill.get(content);
                Some(SkillPoints(
                    skill.cost_to(*rank).0 - skill.cost_to(self.learned.rank(key)).0,
                ))
            })
            .fold(SkillPoints(0), |sum, cost| sum + cost)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NodeState {
    Locked,
    Available,
    Learned,
    Maxed,
    Picked(Rank),
}

impl NodeState {
    fn label(self) -> String {
        match self {
            NodeState::Locked => "locked".to_owned(),
            NodeState::Available => "available".to_owned(),
            NodeState::Learned => "learned".to_owned(),
            NodeState::Maxed => "maxed".to_owned(),
            NodeState::Picked(rank) => format!("picked rank {}", rank.0),
        }
    }
}

fn header(ui: &mut Ui, world: &mut World, sheet: &Sheet) {
    let content = world.resource::<Content>().clone();
    let def = sheet.class.get(&content);
    let plan = world.resource::<Plan>().clone();
    let picked = sheet.picked_cost(&content, def, &plan);
    let left = SkillPoints(sheet.to_spend.0.saturating_sub(picked.0));
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{} · level {}", def.name, sheet.level))
                .family(design_tokens::family::medium()),
        );
        let mut pill = format!("{} to spend", points(left.0));
        if picked.0 > 0 {
            pill.push_str(&format!(" · {} picked", points(picked.0)));
        }
        let color = if left.0 > 0 || picked.0 > 0 {
            color::WARN
        } else {
            color::MUTED
        };
        widgets::pill(ui, &pill, color);
        let granted = super::granted(world, sheet.me);
        if !granted.is_empty() {
            ui.separator();
            ui.label(
                RichText::new("From your gear")
                    .size(TextRole::Small.size())
                    .color(color::MUTED),
            );
            for skill in granted {
                granted_skill(ui, world, skill);
            }
        }
    });
    ui.add_space(4.0);
}

fn granted_skill(ui: &mut Ui, world: &mut World, skill: SkillId) {
    let content = world.resource::<Content>().clone();
    let def = skill.get(&content);
    let (rect, response) =
        ui.allocate_exact_size(Vec2::splat(GRANTED_ICON + 8.0), Sense::click_and_drag());
    let painter = ui.painter();
    painter.rect_stroke(rect, 4.0, HAIRLINE, StrokeKind::Inside);
    painter.rect_stroke(rect.shrink(2.0), 3.0, HAIRLINE, StrokeKind::Inside);
    if let Some(image) = gui::image(world, &def.icon) {
        ui.put(
            Rect::from_center_size(rect.center(), Vec2::splat(GRANTED_ICON)),
            image.widget(Vec2::splat(GRANTED_ICON)),
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("{} granted by gear", def.name),
        )
    });
    let passive = skill.get(world.resource::<Content>()).kind == SkillKind::Passive;
    if map::drag_started(world, InputAction::CarryItem, &response) {
        if !passive {
            egui::DragAndDrop::set_payload(ui.ctx(), CarriedSkill(skill));
        }
    } else if map::clicked(world, InputAction::PinToHotbar, &response) && !passive {
        bar::pin(world, super::HotbarEntry::Skill(skill));
    } else if response.secondary_clicked() {
        card::open(world, card::of(world, skill));
    }
    let hint = StyledLine::new(vec![
        TextPiece::text("Known while the gear is worn · "),
        input(InputAction::PinToHotbar),
        TextPiece::text(" puts it on the hotbar · drag it to a slot"),
    ]);
    card::tooltip(&response, world, &card::of(world, skill), Some(&hint));
}

/// The designer's canvas at its own scale, panned by dragging its background, and never past
/// its edges: the viewport is always full of canvas.
fn tree(ui: &mut Ui, world: &mut World, sheet: &Sheet, viewport: Rect) -> Panned {
    let content = world.resource::<Content>().clone();
    let def = sheet.class.get(&content);
    let canvas = def
        .canvas()
        .unwrap_or(Rect::from_min_size(Pos2::ZERO, viewport.size()));
    let last = *world.resource::<TreeView>();
    let shown = ui.scope_builder(UiBuilder::new().id_salt("tree").max_rect(viewport), |ui| {
        ui.set_clip_rect(viewport.intersect(ui.clip_rect()));
        let mut area = egui::ScrollArea::both()
            .id_salt("tree")
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
            .scroll_source(ScrollSource {
                scroll_bar: false,
                drag: DragScroll::Always,
                mouse_wheel: true,
            })
            .auto_shrink([false, false]);
        let furthest = (canvas.size() - viewport.size()).max(Vec2::ZERO);
        if last.viewport == Rect::ZERO {
            if let Some(nodes) = def.tree.bounds() {
                let centred = nodes.center() - canvas.min - viewport.size() / 2.0;
                area = area.scroll_offset(centred.clamp(Vec2::ZERO, furthest));
            }
        } else if last.offset != last.offset.clamp(Vec2::ZERO, furthest) {
            // The area clamps its offset only after drawing, which would bare a strip of the
            // viewport for the frame a growing window lets more canvas in.
            area = area.scroll_offset(last.offset.clamp(Vec2::ZERO, furthest));
        }
        area.show(ui, |ui| {
            let size = canvas.size().max(viewport.size());
            let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Skill tree")
            });
            let origin = rect.min + (size - canvas.size()) / 2.0 - canvas.min.to_vec2();
            draw_canvas(ui, world, sheet, def, Placement { origin });
        })
    });
    Panned {
        content: shown.inner.content_size,
        offset: shown.inner.state.offset,
    }
}

fn draw_canvas(ui: &mut Ui, world: &mut World, sheet: &Sheet, def: &ClassDef, place: Placement) {
    let content = world.resource::<Content>().clone();
    let tree = &def.tree;
    let plan = world.resource::<Plan>().clone();
    let planned = sheet.planned_ranks(&plan);
    if let Some(backdrop) = &def.backdrop
        && let Some(image) = gui::image(world, &backdrop.image)
    {
        let frame = place.rect(backdrop.frame.rect());
        let drawn = backdrop.fit.place(frame, image.size);
        ui.painter().with_clip_rect(frame).image(
            image.texture,
            drawn,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    let states: BTreeMap<&str, NodeState> = tree
        .0
        .iter()
        .map(|(key, node)| {
            let held = sheet.learned.rank(key);
            let max = Rank(node.skill.get(&content).rank_count());
            let state = match plan.0.get(key) {
                Some(rank) => NodeState::Picked(*rank),
                None if held >= max => NodeState::Maxed,
                None if held > Rank::NONE => NodeState::Learned,
                None if node
                    .requires
                    .iter()
                    .all(|requirement| requirement.met(world, sheet.me, def, &planned)) =>
                {
                    NodeState::Available
                }
                None => NodeState::Locked,
            };
            (key.as_str(), state)
        })
        .collect();
    let painter = ui.painter().clone();
    for arrow in tree.arrows() {
        let (Some(from), Some(to)) = (tree.node(arrow.from), tree.node(arrow.to)) else {
            continue;
        };
        let opened = planned
            .get(arrow.from)
            .is_some_and(|held| *held >= arrow.rank);
        let color = if opened { color::WARN } else { color::BORDER };
        draw_arrow(&painter, place.at(from.at), place.at(to.at), color);
    }
    for (key, node) in &tree.0 {
        let state = states[key.as_str()];
        let center = place.at(node.at);
        let rect = Rect::from_center_size(center, Vec2::splat(NODE_RADIUS * 2.0));
        let response = ui.interact(rect, ui.id().with(("node", key)), Sense::click_and_drag());
        let skill = node.skill.get(&content);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("{} node: {}", skill.name, state.label()),
            )
        });
        draw_node(
            ui,
            world,
            &response,
            node.skill,
            state,
            sheet.learned.rank(key),
        );
        node_input(ui, world, sheet, key, node.skill, state, &response);
    }
}

fn node_input(
    ui: &mut Ui,
    world: &mut World,
    sheet: &Sheet,
    key: &str,
    skill: SkillId,
    state: NodeState,
    response: &egui::Response,
) {
    let held = sheet.learned.rank(key);
    let passive = skill.get(world.resource::<Content>()).kind == SkillKind::Passive;
    if map::drag_started(world, InputAction::CarryItem, response) {
        if held > Rank::NONE && !passive {
            egui::DragAndDrop::set_payload(ui.ctx(), CarriedSkill(skill));
        }
    } else if map::clicked(world, InputAction::PinToHotbar, response) {
        if passive {
            notification::local_error(
                world,
                format!(
                    "{} is passive: it holds without a slot",
                    skill.get(world.resource::<Content>()).name
                ),
            );
        } else if held > Rank::NONE {
            bar::pin(world, super::HotbarEntry::Skill(skill));
        } else {
            InterfaceSound::UiRefuse.play(world);
        }
    } else if response.secondary_clicked() {
        card::open(
            world,
            SkillCard {
                skill,
                rank: held,
                node: Some(NodeAccess {
                    class: sheet.class,
                    node: key.to_owned(),
                }),
            },
        );
    } else if response.clicked() {
        pick(world, sheet, key, state);
    }
    let hint = StyledLine::new(vec![
        TextPiece::text("Click to pick a rank · right-click for the card · "),
        input(InputAction::PinToHotbar),
        TextPiece::text(" puts it on the hotbar · drag it to a slot"),
    ]);
    let card = SkillCard {
        skill,
        rank: held,
        node: Some(NodeAccess {
            class: sheet.class,
            node: key.to_owned(),
        }),
    };
    card::tooltip(response, world, &card, Some(&hint));
}

fn pick(world: &mut World, sheet: &Sheet, key: &str, state: NodeState) {
    let content = world.resource::<Content>().clone();
    let def = sheet.class.get(&content);
    match state {
        NodeState::Picked(_) => {
            let mut plan = world.resource::<Plan>().clone();
            plan.0.remove(key);
            loop {
                let planned = sheet.planned_ranks(&plan);
                let orphan =
                    plan.0
                        .keys()
                        .find(|picked| {
                            def.tree.node(picked).is_some_and(|node| {
                                !node.requires.iter().all(|requirement| {
                                    requirement.met(world, sheet.me, def, &planned)
                                })
                            })
                        })
                        .cloned();
                match orphan {
                    Some(orphan) => {
                        plan.0.remove(&orphan);
                    }
                    None => break,
                }
            }
            *world.resource_mut::<Plan>() = plan;
            InterfaceSound::UiMove.play(world);
        }
        NodeState::Available | NodeState::Learned => {
            let Some(node) = def.tree.node(key) else {
                return;
            };
            let held = sheet.learned.rank(key);
            let next = Rank(held.0 + 1);
            let skill = node.skill.get(&content);
            let cost = SkillPoints(skill.cost_to(next).0 - skill.cost_to(held).0);
            let plan = world.resource::<Plan>().clone();
            let picked = sheet.picked_cost(&content, def, &plan);
            if picked.0 + cost.0 > sheet.to_spend.0 {
                InterfaceSound::UiRefuse.play(world);
                return;
            }
            world.resource_mut::<Plan>().0.insert(key.to_owned(), next);
            InterfaceSound::UiPick.play(world);
        }
        NodeState::Locked | NodeState::Maxed => InterfaceSound::UiRefuse.play(world),
    }
}

fn dock(ui: &mut Ui, world: &mut World, sheet: &Sheet, room: Rect) {
    let plan = world.resource::<Plan>().clone();
    if plan.0.is_empty() {
        return;
    }
    let content = world.resource::<Content>().clone();
    let def = sheet.class.get(&content);
    let picked = sheet.picked_cost(&content, def, &plan);
    let dock = Rect::from_min_max(Pos2::new(room.min.x, room.max.y - DOCK_HEIGHT), room.max);
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(dock)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            let confirm = widgets::primary(ui, &format!("Confirm · {}", points(picked.0)));
            gui::tooltip(&confirm, |ui| {
                ui.label("Spends the picked points for good");
            });
            let reset = ui.button("Reset");
            gui::tooltip(&reset, |ui| {
                ui.label("Takes every pick back; nothing is spent");
            });
            if confirm.clicked() {
                let ranks: Vec<(String, Rank)> = plan
                    .0
                    .iter()
                    .map(|(key, rank)| (key.clone(), *rank))
                    .collect();
                world.write_message(LearnSkillsRequest { ranks });
                InterfaceSound::UiChime.play(world);
            } else if reset.clicked() {
                world.resource_mut::<Plan>().0.clear();
                InterfaceSound::UiMove.play(world);
            }
        },
    );
}

fn draw_node(
    ui: &mut Ui,
    world: &mut World,
    response: &egui::Response,
    skill: SkillId,
    state: NodeState,
    held: Rank,
) {
    let content = world.resource::<Content>().clone();
    let def = skill.get(&content);
    let center = response.rect.center();
    let painter = ui.painter();
    let hovered = response.hovered();
    let (fill, stroke) = match state {
        NodeState::Locked => (color::INPUT, Stroke::new(1.0, color::BORDER)),
        NodeState::Available => (color::RAISED, Stroke::new(1.5, color::WARN)),
        NodeState::Learned => (color::RAISED, Stroke::new(2.0, color::TEXT)),
        NodeState::Maxed => (gold_fill(), Stroke::new(2.0, color::WARN)),
        NodeState::Picked(_) => (color::ACCENT_SOFT, Stroke::new(2.5, color::ACCENT)),
    };
    if state == NodeState::Available {
        painter.circle_stroke(
            center,
            NODE_RADIUS + 4.0,
            Stroke::new(
                3.0,
                color::WARN.gamma_multiply(if hovered { 0.6 } else { 0.35 }),
            ),
        );
    }
    painter.circle_filled(center, NODE_RADIUS, fill);
    if state == NodeState::Locked {
        dashed_circle(painter, center, NODE_RADIUS, stroke);
    } else {
        painter.circle_stroke(center, NODE_RADIUS, stroke);
    }
    if hovered {
        painter.circle_stroke(
            center,
            NODE_RADIUS + 1.5,
            Stroke::new(1.0, color::HOVER_LINE),
        );
    }
    if let Some(image) = gui::image(world, &def.icon) {
        let tint = if state == NodeState::Locked {
            Color32::from_white_alpha(90)
        } else {
            Color32::WHITE
        };
        ui.put(
            Rect::from_center_size(center, Vec2::splat(NODE_ICON)),
            image.widget(Vec2::splat(NODE_ICON)).tint(tint),
        );
    }
    let painter = ui.painter();
    match state {
        NodeState::Locked => padlock(
            painter,
            center + Vec2::new(NODE_RADIUS - 6.0, NODE_RADIUS - 8.0),
        ),
        NodeState::Available => {}
        NodeState::Learned | NodeState::Maxed => badge(
            painter,
            center + Vec2::splat(NODE_RADIUS - 4.0),
            &format!("{}/{}", held.0, def.rank_count()),
            if state == NodeState::Maxed {
                color::WARN
            } else {
                color::TEXT
            },
        ),
        NodeState::Picked(rank) => badge(
            painter,
            center + Vec2::splat(NODE_RADIUS - 4.0),
            &format!("{}→{}", held.0, rank.0),
            color::SELECTION,
        ),
    }
    painter.text(
        center + Vec2::new(0.0, NODE_RADIUS + 10.0),
        egui::Align2::CENTER_CENTER,
        &def.name,
        egui::FontId::proportional(TextRole::Small.size()),
        if state == NodeState::Locked {
            color::MUTED
        } else {
            color::TEXT
        },
    );
}

fn draw_arrow(painter: &egui::Painter, from: Pos2, to: Pos2, color: Color32) {
    let delta = to - from;
    let length = delta.length();
    if length <= NODE_RADIUS * 2.0 {
        return;
    }
    let dir = delta / length;
    let start = from + dir * (NODE_RADIUS + 4.0);
    let end = to - dir * (NODE_RADIUS + 6.0);
    let stroke = Stroke::new(2.0, color);
    painter.line_segment([start, end], stroke);
    let side = Vec2::new(-dir.y, dir.x);
    let tip = end + dir * ARROW_HEAD;
    painter.add(egui::Shape::convex_polygon(
        vec![
            tip,
            end - dir * 2.0 + side * ARROW_HEAD * 0.6,
            end - dir * 2.0 - side * ARROW_HEAD * 0.6,
        ],
        color,
        Stroke::NONE,
    ));
}

fn dashed_circle(painter: &egui::Painter, center: Pos2, radius: f32, stroke: Stroke) {
    let points: Vec<Pos2> = (0..=48)
        .map(|step| {
            let angle = std::f32::consts::TAU * step as f32 / 48.0;
            center + Vec2::new(angle.cos(), angle.sin()) * radius
        })
        .collect();
    painter.add(egui::Shape::dashed_line(&points, stroke, 4.0, 3.0));
}

fn padlock(painter: &egui::Painter, at: Pos2) {
    let body = Rect::from_center_size(at + Vec2::new(0.0, 2.0), Vec2::new(9.0, 7.0));
    painter.rect_filled(body, 1.5, color::MUTED);
    let shackle: Vec<Pos2> = (0..=10)
        .map(|step| {
            let angle = std::f32::consts::PI * step as f32 / 10.0;
            at + Vec2::new(-angle.cos() * 3.0, -2.0 - angle.sin() * 4.0)
        })
        .collect();
    painter.add(egui::Shape::line(shackle, Stroke::new(1.5, color::MUTED)));
}

fn badge(painter: &egui::Painter, at: Pos2, text: &str, ink: Color32) {
    let font = egui::FontId::monospace(10.0);
    let galley = painter.layout_no_wrap(text.to_owned(), font, ink);
    let size = galley.size() + Vec2::new(6.0, 2.0);
    let rect = Rect::from_center_size(at, size);
    painter.rect(rect, 3.0, color::INPUT, HAIRLINE, StrokeKind::Inside);
    painter.galley(rect.center() - galley.size() / 2.0, galley, ink);
}

fn gold_fill() -> Color32 {
    Color32::from_rgba_unmultiplied(color::WARN.r(), color::WARN.g(), color::WARN.b(), 70)
}

fn points(count: u32) -> String {
    if count == 1 {
        "1 point".to_owned()
    } else {
        format!("{count} points")
    }
}

/// Where the designer's canvas origin lands on screen.
struct Placement {
    origin: Pos2,
}

impl Placement {
    fn at(&self, point: class::CanvasPoint) -> Pos2 {
        self.origin + Vec2::new(point.x, point.y)
    }

    fn rect(&self, rect: Rect) -> Rect {
        rect.translate(self.origin.to_vec2())
    }
}
