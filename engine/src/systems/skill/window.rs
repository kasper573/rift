use std::collections::BTreeMap;
use ui::drag;
use ui::egui;
use ui::float;
use ui::theme;

use bevy::prelude::*;
use ui::controls;
use ui::egui::scroll_area::{DragScroll, ScrollBarVisibility, ScrollSource};
use ui::egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2};
use ui::face::{self, TextRole};
use ui::glyph::{self, Mark};
use ui::ink;

use super::{LearnSkillsRequest, Learned, Rank, SkillId, SkillKind, card};
use crate::core::content::Content;
use crate::core::gui;
use crate::core::text::{StyledLine, TextPiece};
use crate::systems::class::{self, ClassDef, NodeKey, SkillPoints};
use crate::systems::hud::Carried;
use crate::systems::hud::menu::{MenuTab, MenuTabName};
use crate::systems::input::map::{self, InputAction, input};
use crate::systems::interface::{InterfaceIcon, InterfaceSound};
use crate::systems::item::widget::{SlotLook, slot};
use crate::systems::notification;
use crate::systems::player::session;
use crate::systems::skill::bar;
use ui::words::counted;

const NODE_RADIUS: f32 = 24.0;
const NODE_ICON: f32 = 28.0;
const DOCK_HEIGHT: f32 = 34.0;
const NAME_HALO: f32 = 1.0;
const COLUMN_GAP: f32 = 16.0;

pub struct SkillsMenuTab;

impl MenuTab for SkillsMenuTab {
    fn name(&self) -> MenuTabName {
        MenuTabName::Skills
    }
    fn icon(&self) -> InterfaceIcon {
        InterfaceIcon::Skills
    }
    fn opens(&self) -> InputAction {
        InputAction::OpenSkills
    }
    fn hints(&self) -> &'static [(InputAction, &'static str)] {
        &[
            (InputAction::RaiseRank, "raise"),
            (InputAction::LowerRank, "lower"),
            (InputAction::CarryItem, "to the bar"),
            (InputAction::InspectSkill, "pin the card"),
        ]
    }
    fn show(&self, ui: &mut Ui, world: &mut World) {
        let Some(sheet) = Sheet::of(world) else {
            ui.label(theme::faint("No class yet"));
            return;
        };
        let room = ui.available_rect_before_wrap();
        let beside_card = Rect::from_min_max(
            room.min,
            Pos2::new(room.max.x - ui::card::WIDTH - COLUMN_GAP, room.max.y),
        );
        ui.scope_builder(UiBuilder::new().max_rect(beside_card), |ui| {
            header(ui, world, &sheet);
            let room = ui.available_rect_before_wrap();
            let viewport =
                Rect::from_min_max(room.min, Pos2::new(room.max.x, room.max.y - DOCK_HEIGHT));
            let view = tree(ui, world, &sheet, viewport);
            *world.resource_mut::<TreeView>() = view;
            dock(ui, world, &sheet, room);
        });
    }
}

/// How the tree sat in its pane when last drawn: the viewport the tree pans within, the canvas
/// shown there and how far it is panned.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct TreeView {
    pub viewport: Rect,
    pub content: Vec2,
    pub offset: Vec2,
}

impl Default for TreeView {
    fn default() -> TreeView {
        TreeView {
            viewport: Rect::ZERO,
            content: Vec2::ZERO,
            offset: Vec2::ZERO,
        }
    }
}

/// The rank each node is picked up to in the tree, not yet confirmed.
#[derive(Resource, Default, Clone, Debug, PartialEq)]
pub struct SkillPlan(pub BTreeMap<NodeKey, Rank>);

/// Picks the server has since learned, or that lost their footing, leave the plan.
pub fn settle_plan(world: &mut World) {
    let Some(sheet) = Sheet::of(world) else {
        world.resource_mut::<SkillPlan>().0.clear();
        return;
    };
    let content = world.resource::<Content>().clone();
    let def = sheet.class.get(&content);
    let mut plan = world.resource::<SkillPlan>().clone();
    plan.0
        .retain(|key, rank| sheet.learned.rank(key) < *rank && def.tree.node(key).is_some());
    if plan != *world.resource::<SkillPlan>() {
        *world.resource_mut::<SkillPlan>() = plan;
    }
}

pub fn forget_plan(mut plan: ResMut<SkillPlan>) {
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

    fn planned_ranks(&self, plan: &SkillPlan) -> BTreeMap<NodeKey, Rank> {
        let mut ranks = self.learned.0.clone();
        for (key, rank) in &plan.0 {
            ranks.insert(key.clone(), *rank);
        }
        ranks
    }

    fn picked_cost(&self, content: &Content, def: &ClassDef, plan: &SkillPlan) -> SkillPoints {
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
    ui.horizontal(|ui| {
        ui.label(face::bold(format!("{} · level {}", def.name, sheet.level)));
        let granted = super::granted(world, sheet.me);
        if !granted.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for skill in granted.into_iter().rev() {
                    granted_skill(ui, world, skill);
                }
                ui.label(face::secondary("From your gear"));
            });
        }
    });
    ui.add_space(4.0);
}

fn granted_skill(ui: &mut Ui, world: &mut World, skill: SkillId) {
    let content = world.resource::<Content>().clone();
    let def = skill.get(&content);
    let look = SlotLook {
        icon: Some(&def.icon),
        ..SlotLook::EMPTY
    };
    let response = slot(ui, world, look, Sense::click_and_drag());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("{} granted by gear", def.name),
        )
    });
    let passive = def.kind == SkillKind::Passive;
    if map::drag_started(world, InputAction::CarryItem, &response) {
        if !passive {
            drag::carry(ui.ctx(), Carried::Skill(skill));
        }
    } else if map::clicked(world, InputAction::PinToHotbar, &response) && !passive {
        bar::pin(world, super::HotbarEntry::Skill(skill));
    } else if map::clicked(world, InputAction::InspectSkill, &response) {
        card::open(world, skill, Some(response.rect));
    }
    let hint = StyledLine::new(vec![
        TextPiece::text("Known while the gear is worn · "),
        input(InputAction::InspectSkill),
        TextPiece::text(" shows its card · "),
        input(InputAction::PinToHotbar),
        TextPiece::text(" puts it on the hotbar · "),
        input(InputAction::CarryItem),
        TextPiece::text(" carries it to a slot"),
    ]);
    card::tooltip(&response, world, &card::of(world, skill), Some(&hint));
}

/// The designer's canvas at its own scale, panned by dragging its background, and never past
/// its edges: the viewport is always full of canvas.
fn tree(ui: &mut Ui, world: &mut World, sheet: &Sheet, viewport: Rect) -> TreeView {
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
    ui.painter().rect_stroke(
        viewport.expand(1.0),
        0.0,
        Stroke::new(1.0, ink::GOLD2),
        StrokeKind::Outside,
    );
    TreeView {
        viewport,
        content: shown.inner.content_size,
        offset: shown.inner.state.offset,
    }
}

fn draw_canvas(ui: &mut Ui, world: &mut World, sheet: &Sheet, def: &ClassDef, place: Placement) {
    let content = world.resource::<Content>().clone();
    let tree = &def.tree;
    let plan = world.resource::<SkillPlan>().clone();
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
    let states: BTreeMap<&NodeKey, NodeState> = tree
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
            (key, state)
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
        let color = if opened {
            ink::GOLD2
        } else {
            ink::wash(ink::LIGHT, 0.45)
        };
        let (tail, head) = (place.at(from.at), place.at(to.at));
        let span = head - tail;
        if span.length() > NODE_RADIUS * 2.0 {
            let dir = span.normalized();
            glyph::arrow(
                &painter,
                tail + dir * (NODE_RADIUS + 4.0),
                head - dir * (NODE_RADIUS + 6.0),
                Stroke::new(2.0, color),
                false,
            );
        }
    }
    for (key, node) in &tree.0 {
        let state = states[key];
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
    key: &NodeKey,
    skill: SkillId,
    state: NodeState,
    response: &egui::Response,
) {
    let held = sheet.learned.rank(key);
    let passive = skill.get(world.resource::<Content>()).kind == SkillKind::Passive;
    if map::drag_started(world, InputAction::CarryItem, response) {
        if held > Rank::NONE && !passive {
            drag::carry(ui.ctx(), Carried::Skill(skill));
        }
    } else if map::clicked(world, InputAction::PinToHotbar, response) {
        if passive {
            let refusal = super::unslotted(skill.get(world.resource::<Content>()));
            notification::local_error(world, refusal);
        } else if held > Rank::NONE {
            bar::pin(world, super::HotbarEntry::Skill(skill));
        } else {
            InterfaceSound::UiRefuse.play(world);
        }
    } else if map::clicked(world, InputAction::LowerRank, response) {
        lower(world, sheet, key);
    } else if map::clicked(world, InputAction::InspectSkill, response) {
        card::open(world, skill, Some(response.rect));
    } else if map::clicked(world, InputAction::RaiseRank, response) {
        raise(world, sheet, key, state);
    }
    let hint = StyledLine::new(vec![
        input(InputAction::RaiseRank),
        TextPiece::text(" picks a rank · "),
        input(InputAction::LowerRank),
        TextPiece::text(" takes one back · "),
        input(InputAction::InspectSkill),
        TextPiece::text(" shows its card · "),
        input(InputAction::PinToHotbar),
        TextPiece::text(" puts it on the hotbar · "),
        input(InputAction::CarryItem),
        TextPiece::text(" carries it to a slot"),
    ]);
    card::tooltip(response, world, &card::of(world, skill), Some(&hint));
}

/// One more rank of the node joins the plan, while the skill has ranks left and the points
/// picked so far leave room for it.
fn raise(world: &mut World, sheet: &Sheet, key: &NodeKey, state: NodeState) {
    if matches!(state, NodeState::Locked | NodeState::Maxed) {
        InterfaceSound::UiRefuse.play(world);
        return;
    }
    let content = world.resource::<Content>().clone();
    let def = sheet.class.get(&content);
    let Some(node) = def.tree.node(key) else {
        return;
    };
    let skill = node.skill.get(&content);
    let plan = world.resource::<SkillPlan>().clone();
    let current = plan.0.get(key).copied().unwrap_or(sheet.learned.rank(key));
    let next = Rank(current.0 + 1);
    let cost = SkillPoints(skill.cost_to(next).0 - skill.cost_to(current).0);
    let picked = sheet.picked_cost(&content, def, &plan);
    if current.0 >= skill.rank_count() || picked.0 + cost.0 > sheet.to_spend.0 {
        InterfaceSound::UiRefuse.play(world);
        return;
    }
    world
        .resource_mut::<SkillPlan>()
        .0
        .insert(key.clone(), next);
    InterfaceSound::UiPick.play(world);
}

/// The last rank picked on the node leaves the plan, and with it every pick that stood on it.
fn lower(world: &mut World, sheet: &Sheet, key: &NodeKey) {
    let mut plan = world.resource::<SkillPlan>().clone();
    let Some(current) = plan.0.get(key).copied() else {
        InterfaceSound::UiRefuse.play(world);
        return;
    };
    let back = Rank(current.0 - 1);
    if back > sheet.learned.rank(key) {
        plan.0.insert(key.clone(), back);
    } else {
        plan.0.remove(key);
    }
    let content = world.resource::<Content>().clone();
    let def = sheet.class.get(&content);
    loop {
        let planned = sheet.planned_ranks(&plan);
        let orphan = plan
            .0
            .keys()
            .find(|picked| {
                def.tree.node(*picked).is_some_and(|node| {
                    !node
                        .requires
                        .iter()
                        .all(|requirement| requirement.met(world, sheet.me, def, &planned))
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
    *world.resource_mut::<SkillPlan>() = plan;
    InterfaceSound::UiMove.play(world);
}

// Drawn whether or not anything is picked, its buttons faded, so the tree above never moves.
fn dock(ui: &mut Ui, world: &mut World, sheet: &Sheet, room: Rect) {
    let plan = world.resource::<SkillPlan>().clone();
    let content = world.resource::<Content>().clone();
    let def = sheet.class.get(&content);
    let picked = sheet.picked_cost(&content, def, &plan);
    let left = SkillPoints(sheet.to_spend.0.saturating_sub(picked.0));
    let dock = Rect::from_min_max(Pos2::new(room.min.x, room.max.y - DOCK_HEIGHT), room.max);
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(dock)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            let planned = !plan.0.is_empty();
            let (learn, reset) = ui
                .add_enabled_ui(planned, |ui| {
                    let learn = controls::primary(ui, "Learn");
                    float::tooltip(&learn, |ui| {
                        ui.label("Spends the picked points for good");
                    });
                    let reset = controls::button(ui, "Reset plan");
                    float::tooltip(&reset, |ui| {
                        ui.label("Takes every pick back; nothing is spent");
                    });
                    (learn, reset)
                })
                .inner;
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let mut text = format!("{} to spend", counted(left.0, "skill point"));
                if picked.0 > 0 {
                    text.push_str(&format!(" · {} picked", counted(picked.0, "point")));
                }
                ui.add(egui::Label::new(face::body(text)).truncate());
            });
            if learn.clicked() {
                let ranks: Vec<(NodeKey, Rank)> = plan
                    .0
                    .iter()
                    .map(|(key, rank)| (key.clone(), *rank))
                    .collect();
                world.write_message(LearnSkillsRequest { ranks });
                InterfaceSound::UiChime.play(world);
            } else if reset.clicked() {
                world.resource_mut::<SkillPlan>().0.clear();
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
    let carded = card::shown(world) == Some(skill);
    let (fill, stroke) = match state {
        NodeState::Locked => (ink::wash(ink::INK, 0.8), Stroke::new(1.0, ink::INK3)),
        NodeState::Available => (ink::INK, Stroke::new(1.5, ink::GOLD)),
        NodeState::Learned | NodeState::Maxed => (ink::INK, Stroke::new(2.0, ink::LIGHT)),
        NodeState::Picked(_) => (
            ink::on(ink::INK, ink::GOLD2, 0.25),
            Stroke::new(2.5, ink::GOLD2),
        ),
    };
    if state == NodeState::Available {
        painter.circle_stroke(
            center,
            NODE_RADIUS + 4.0,
            Stroke::new(3.0, ink::wash(ink::GOLD, if hovered { 0.6 } else { 0.35 })),
        );
    }
    if carded {
        painter.circle_stroke(center, NODE_RADIUS + 4.0, Stroke::new(3.0, ink::GOLD2));
    }
    painter.circle_filled(center, NODE_RADIUS, fill);
    if state == NodeState::Locked {
        glyph::dashed_circle(painter, center, NODE_RADIUS, stroke);
    } else {
        painter.circle_stroke(center, NODE_RADIUS, stroke);
    }
    if hovered {
        painter.circle_stroke(
            center,
            NODE_RADIUS + 1.5,
            Stroke::new(1.0, ink::wash(ink::LIGHT, 0.6)),
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
    let corner = center + Vec2::splat(NODE_RADIUS - 4.0);
    match state {
        NodeState::Locked | NodeState::Available => {}
        NodeState::Learned | NodeState::Maxed => glyph::badge(
            painter,
            corner,
            Align2::CENTER_CENTER,
            &format!("{}/{}", held.0, def.rank_count()),
            ink::INK,
        ),
        NodeState::Picked(rank) => glyph::badge(
            painter,
            corner,
            Align2::CENTER_CENTER,
            &format!("{}→{}", held.0, rank.0),
            ink::RUBRIC,
        ),
    }
    let color = if state == NodeState::Locked {
        ink::wash(ink::LIGHT, 0.7)
    } else {
        ink::LIGHT
    };
    let name = glyph::haloed(
        painter,
        center + Vec2::new(0.0, NODE_RADIUS + 10.0),
        Align2::CENTER_CENTER,
        &def.name,
        TextRole::Small.font_id(),
        color,
        glyph::Halo {
            color: ink::INK,
            width: NAME_HALO,
        },
    );
    if state == NodeState::Locked {
        let mark = Rect::from_min_size(
            Pos2::new(name.max.x + 3.0, name.center().y - 5.0),
            Vec2::splat(10.0),
        );
        Mark::Lock.paint(painter, mark, color);
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
