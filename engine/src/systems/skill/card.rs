use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use design_tokens::{TextRole, color};
use egui::{Pos2, Rect, Response, RichText, Ui, Vec2};

use super::{Rank, SkillDef, SkillId, SkillKind, SkillStat};
use crate::core::content::Content;
use crate::core::gui::{self, Band};
use crate::core::text::{self, StyledLine, TextLook};
use crate::systems::class::{self, ClassId};
use crate::systems::hud::{self, Sizing, WindowFrame};
use crate::systems::interface::InterfaceSound;
use crate::systems::item::Inventory;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::script::condition;
use crate::systems::skill::Learned;

const WINDOW_KEY: &str = "Skill";
const WINDOW_SIZE: Vec2 = Vec2::new(300.0, 380.0);
const WINDOW_TOP: f32 = 96.0;
const WINDOW_MARGIN: f32 = 72.0;
const ICON: f32 = 40.0;
const TOOLTIP_WIDTH: f32 = 280.0;

/// One skill as the player sees it: the row at the rank they hold, with the node that unlocks it
/// when it sits in their tree.
#[derive(Clone, Debug, PartialEq)]
pub struct SkillCard {
    pub skill: SkillId,
    pub rank: Rank,
    pub node: Option<NodeAccess>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeAccess {
    pub class: ClassId,
    pub node: String,
}

pub struct SkillCardPlugin;

impl Plugin for SkillCardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PinnedCard>()
            .add_systems(
                EguiPrimaryContextPass,
                draw_card
                    .in_set(Band::Hud)
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(OnExit(ClientScreen::Area), forget);
    }
}

pub fn open(world: &mut World, card: SkillCard) {
    let was_shown = world.resource_mut::<PinnedCard>().wanted.replace(card);
    if was_shown.is_none() {
        InterfaceSound::UiOpen.play(world);
    }
}

pub fn close(world: &mut World) -> bool {
    let closed = world.resource_mut::<PinnedCard>().wanted.take().is_some();
    if closed {
        InterfaceSound::UiClose.play(world);
    }
    closed
}

pub fn showing(world: &World) -> bool {
    world.resource::<PinnedCard>().wanted.is_some()
}

pub fn layer() -> egui::LayerId {
    hud::window_layer(WINDOW_KEY)
}

/// The card of a skill as the player's own character knows it.
pub fn of(world: &World, skill: SkillId) -> SkillCard {
    let content = world.resource::<Content>();
    let me = session::me(world);
    let rank = me.map_or(Rank::NONE, |me| super::rank_of(world, me, skill));
    let node = me.and_then(|me| class::of(world, me)).and_then(|class| {
        class
            .get(content)
            .tree
            .node_of(skill)
            .map(|(key, _)| NodeAccess {
                class,
                node: key.to_owned(),
            })
    });
    SkillCard { skill, rank, node }
}

pub fn tooltip(
    response: &Response,
    world: &mut World,
    card: &SkillCard,
    hint: Option<&StyledLine>,
) {
    gui::tooltip(response, |ui| {
        ui.set_max_width(TOOLTIP_WIDTH);
        sheet(ui, world, card);
        if let Some(hint) = hint {
            ui.add_space(4.0);
            let look = TextLook {
                size: TextRole::Small.size(),
                color: color::MUTED,
            };
            text::show(ui, world, hint, look, None);
        }
    });
}

pub fn sheet(ui: &mut Ui, world: &mut World, card: &SkillCard) {
    let content = world.resource::<Content>().clone();
    let def = card.skill.get(&content);
    ui.spacing_mut().item_spacing.y = 6.0;
    ui.horizontal(|ui| {
        hud::icon(ui, world, &def.icon, ICON);
        ui.vertical(|ui| {
            ui.label(RichText::new(def.name.as_str()).font(TextRole::Heading.font_id()));
            ui.label(
                RichText::new(format!("Rank {} of {}", card.rank.0, def.rank_count()))
                    .size(TextRole::Small.size())
                    .color(color::MUTED),
            );
        });
    });
    ui.label(
        RichText::new(use_line(def, card.rank))
            .size(TextRole::Small.size())
            .color(color::TEXT.gamma_multiply(0.85)),
    );
    for (index, level) in def.levels.iter().enumerate() {
        let rank = Rank(index as u8 + 1);
        let lit = rank <= card.rank;
        let ink = if lit { color::TEXT } else { color::FAINT };
        ui.label(RichText::new(rank_line(def, rank, level.cost.0)).color(ink));
    }
    if let Some(access) = &card.node {
        let lines = access_lines(world, &content, access);
        if !lines.is_empty() {
            section(ui, "Unlocked by");
            for (line, met) in lines {
                requirement(ui, &line, met);
            }
        }
    }
    section(ui, "To use");
    let needs = use_lines(world, &content, def);
    if needs.is_empty() {
        ui.label(RichText::new("Nothing needed").color(color::MUTED));
    }
    for (line, met) in needs {
        requirement(ui, &line, met);
    }
}

fn use_line(def: &SkillDef, rank: Rank) -> String {
    if def.kind == SkillKind::Passive {
        return "Passive · holds while known".to_owned();
    }
    let rank = rank.max(Rank::FIRST);
    let mut parts = vec![def.target.describe().to_owned()];
    if let Some(shape) = def.area_at(rank) {
        parts.push(shape.describe());
    }
    if let Some(range) = def.range {
        parts.push(format!(
            "{} tiles",
            range.0 + def.stat(rank, SkillStat::Range)
        ));
    }
    let cooldown = def.cooldown_at(rank);
    if cooldown.0 > 0.0 {
        parts.push(format!("{} s cooldown", trim(cooldown.0)));
    }
    parts.join(" · ")
}

fn rank_line(def: &SkillDef, rank: Rank, cost: u32) -> String {
    let level = &def.levels[usize::from(rank.0) - 1];
    let points = if cost == 1 { "point" } else { "points" };
    let mut line = format!("Rank {} · {cost} {points}", rank.0);
    let changes: Vec<String> = level
        .stats
        .iter()
        .map(|(stat, value)| {
            let unit = stat.unit().map_or(String::new(), |unit| format!(" {unit}"));
            format!("{} {}{unit}", stat.label(), signed(*value))
        })
        .collect();
    if !changes.is_empty() {
        line.push_str(": ");
        line.push_str(&changes.join(", "));
    }
    line
}

fn access_lines(world: &World, content: &Content, access: &NodeAccess) -> Vec<(String, bool)> {
    let class = access.class.get(content);
    let Some(node) = class.tree.node(&access.node) else {
        return Vec::new();
    };
    let me = session::me(world);
    let held = me
        .and_then(|me| world.get::<Learned>(me))
        .cloned()
        .unwrap_or_default();
    node.requires
        .iter()
        .map(|requirement| {
            let met = me.is_some_and(|me| requirement.met(world, me, class, &held.0));
            (requirement.describe(content, &class.tree), met)
        })
        .collect()
}

fn use_lines(world: &World, content: &Content, def: &SkillDef) -> Vec<(String, bool)> {
    let me = session::me(world);
    let mut lines: Vec<(String, bool)> = def
        .requires
        .iter()
        .flat_map(condition::parts)
        .map(|part| {
            let met = me.is_some_and(|me| condition::holds(world, me, part));
            (condition::describe(content, part), met)
        })
        .collect();
    let bag = me.and_then(|me| world.get::<Inventory>(me));
    for stack in &def.cost {
        let held = bag.map_or(0, |bag| bag.count(stack.item));
        lines.push((
            format!(
                "Spends {} {} ({held} held)",
                stack.count,
                stack.item.get(content).name
            ),
            held >= stack.count,
        ));
    }
    lines
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(2.0);
    ui.label(
        RichText::new(title)
            .size(TextRole::Small.size())
            .family(design_tokens::family::medium())
            .color(color::MUTED),
    );
}

fn requirement(ui: &mut Ui, line: &str, met: bool) {
    let (mark, ink) = if met {
        ("✓", color::OK)
    } else {
        ("✗", color::DANGER)
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(mark).color(ink));
        ui.label(RichText::new(line).color(color::TEXT));
    });
}

fn signed(value: f32) -> String {
    let text = trim(value.abs());
    if value < 0.0 {
        format!("−{text}")
    } else {
        format!("+{text}")
    }
}

fn trim(value: f32) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        format!("{rounded}")
    }
}

#[derive(Resource, Default)]
struct PinnedCard {
    wanted: Option<SkillCard>,
}

fn forget(mut card: ResMut<PinnedCard>) {
    card.wanted = None;
}

fn draw_card(world: &mut World) {
    let Some(card) = world.resource::<PinnedCard>().wanted.clone() else {
        return;
    };
    let Some(gui) = gui::context(world) else {
        return;
    };
    let screen = gui.content_rect();
    let start = Rect::from_min_size(
        Pos2::new(
            (screen.right() - WINDOW_SIZE.x - WINDOW_MARGIN).max(8.0),
            screen.top() + WINDOW_TOP,
        ),
        WINDOW_SIZE,
    );
    let mut open = true;
    let frame = WindowFrame {
        key: WINDOW_KEY,
        title: None,
        start,
        sizing: Sizing::FitsContent,
        body_limit: None,
    };
    let fresh = of(world, card.skill);
    hud::window(world, &gui, frame, &mut open, |ui, world| {
        egui::ScrollArea::vertical()
            .max_height(WINDOW_SIZE.y)
            .auto_shrink([false, true])
            .show(ui, |ui| sheet(ui, world, &fresh));
    });
    if !open {
        close(world);
    }
}

pub(crate) const BASIC_ATTACK_MARK: &str = "basic attack";
