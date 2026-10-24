use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use ui::card::{self, Art, Card, CardResponse};
use ui::egui;
use ui::egui::{Rect, Response, Ui};
use ui::face::{self, TextRole};
use ui::float;
use ui::glyph::Kind;
use ui::ink;
use ui::sheet::Depth;

use super::{Rank, SkillDef, SkillId, SkillKind, SkillStat};
use crate::core::content::Content;
use crate::core::gui::{self, Band};
use crate::core::text::{self, StyledLine, TextLook};
use crate::systems::class::{self, ClassId, NodeKey};
use crate::systems::equipment::Equipment;
use crate::systems::hud;
use crate::systems::item::{Inventory, ItemId};
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::script::condition;
use crate::systems::skill::Learned;
use ui::words;
use ui::words::counted;

pub(crate) const BASIC_ATTACK_MARK: &str = "basic attack";

const WINDOW_KEY: &str = "Skill";

/// One skill as the player sees it: the row at the rank they hold, with the node that unlocks it
/// when it sits in their tree, and the worn item that grants it when that is how they know it.
#[derive(Clone, Debug, PartialEq)]
pub struct SkillCard {
    pub skill: SkillId,
    pub rank: Rank,
    pub node: Option<NodeAccess>,
    pub granted_by: Option<ItemId>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeAccess {
    pub class: ClassId,
    pub node: NodeKey,
}

pub struct SkillCardPlugin;

impl Plugin for SkillCardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<hud::Pinned<SkillId>>()
            .add_systems(
                Update,
                hud::raise_pinned::<SkillId>(WINDOW_KEY).run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(
                EguiPrimaryContextPass,
                draw_card
                    .in_set(Band::Hud)
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(OnExit(ClientScreen::Area), hud::forget_pinned::<SkillId>);
    }
}

pub fn open(world: &mut World, skill: SkillId, from: Option<Rect>) {
    hud::pin(world, skill, from);
}

pub fn shown(world: &World) -> Option<SkillId> {
    hud::pinned::<SkillId>(world)
}

pub fn close(world: &mut World) -> bool {
    hud::unpin::<SkillId>(world)
}

pub fn showing(world: &World) -> bool {
    hud::pinned::<SkillId>(world).is_some()
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
                node: key.clone(),
            })
    });
    let learned = me
        .zip(node.as_ref())
        .and_then(|(me, access)| Some(world.get::<Learned>(me)?.rank(&access.node)))
        .unwrap_or(Rank::NONE);
    let granted_by = (rank > learned)
        .then(|| {
            me.and_then(|me| world.get::<Equipment>(me))?
                .slots
                .values()
                .copied()
                .find(|item| super::grants_of(content, *item).contains(&skill))
        })
        .flatten();
    SkillCard {
        skill,
        rank,
        node,
        granted_by,
    }
}

pub fn tooltip(
    response: &Response,
    world: &mut World,
    card: &SkillCard,
    hint: Option<&StyledLine>,
) {
    float::tooltip(response, |ui| {
        let content = world.resource::<Content>().clone();
        let def = card.skill.get(&content);
        let line = overview(world, &content, def, card);
        ui.set_max_width(hud::TOOLTIP_WIDTH);
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(face::bold(def.name.as_str()).size(TextRole::Tooltip.size()));
        ui.label(TextRole::Tooltip.rich(line).color(ink::INK2));
        if let Some(hint) = hint {
            let look = TextLook::new(TextRole::Small.size(), ink::INK3);
            text::show(ui, world, hint, look, None);
        }
    });
}

pub fn kind(def: &SkillDef) -> Kind {
    match def.kind {
        SkillKind::Active => Kind::Active,
        SkillKind::Passive => Kind::Passive,
    }
}

pub fn show(
    ui: &mut Ui,
    world: &mut World,
    card: &SkillCard,
    closable: bool,
    depth: Depth,
) -> CardResponse {
    let content = world.resource::<Content>().clone();
    let def = card.skill.get(&content);
    let ranks = def.rank_count();
    let numeral = format!(
        "{} · {}",
        face::roman(u32::from(card.rank.0)),
        face::roman(u32::from(ranks))
    );
    let learned = card.rank > Rank::NONE;
    let maxed = card.granted_by.is_none() && learned && card.rank.0 >= ranks;
    let icon = gui::image(world, &def.icon).map(|image| image.texture);
    let mut shown = Card::new(kind(def), &def.name);
    shown.numeral = Some(&numeral);
    shown.art = Art {
        icon,
        height: card::ART,
    };
    shown.closable = closable;
    shown.stamp = maxed.then_some("maxed");
    shown.depth = depth;
    shown.show(ui, |ui, plate| {
        if plate.name_cut {
            card::full_name(ui, &def.name);
        }
        card::overview(ui, &overview(world, &content, def, card));
        card::heading(ui, "ranks");
        for (index, level) in def.levels.iter().enumerate() {
            let rank = Rank(index as u8 + 1);
            let lit = rank <= card.rank;
            let color = if lit { ink::INK } else { ink::INK3 };
            ui.add(
                egui::Label::new(
                    TextRole::CardBody
                        .rich(rank_line(def, rank, level.cost.0))
                        .color(color),
                )
                .wrap(),
            );
        }
        if let Some(access) = &card.node {
            let lines = access_lines(world, &content, access);
            if !lines.is_empty() {
                card::heading(ui, if learned { "unlocked by" } else { "costs" });
                for (line, met) in lines {
                    card::requirement(ui, met, &line);
                }
            }
        }
        card::heading(ui, "to use");
        let needs = use_lines(world, &content, def);
        if needs.is_empty() {
            card::fact(ui, "Nothing needed");
        }
        for (line, met) in needs {
            card::requirement(ui, met, &line);
        }
    })
}

fn overview(world: &World, content: &Content, def: &SkillDef, card: &SkillCard) -> String {
    if let Some(item) = card.granted_by {
        return format!("Known while your {} is worn", item.get(content).name);
    }
    if card.rank == Rank::NONE {
        let wanting = card.node.as_ref().and_then(|access| {
            access_lines(world, content, access)
                .into_iter()
                .find(|(_, met)| !met)
        });
        return match wanting {
            Some((line, _)) => format!("Not learned · needs {line}"),
            None => "Not learned".to_owned(),
        };
    }
    format!(
        "Rank {} of {} · {}",
        face::roman(u32::from(card.rank.0)),
        face::roman(u32::from(def.rank_count())),
        use_line(def, card.rank)
    )
}

fn draw_card(world: &mut World) {
    hud::draw_pinned::<SkillId>(world, WINDOW_KEY, |ui, world, skill, depth| {
        let card = of(world, *skill);
        show(ui, world, &card, true, depth)
    });
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
        parts.push(format!("{} m", range.0 + def.stat(rank, SkillStat::Range)));
    }
    let cooldown = def.cooldown_at(rank);
    if cooldown.0 > 0.0 {
        parts.push(format!("{} s cooldown", trim(cooldown.0)));
    }
    parts.join(" · ")
}

fn rank_line(def: &SkillDef, rank: Rank, cost: u32) -> String {
    let level = &def.levels[usize::from(rank.0) - 1];
    let mut line = format!("Rank {} · {}", rank.0, counted(cost, "point"));
    let changes: Vec<String> = level
        .stats
        .iter()
        .map(|(stat, value)| {
            let unit = stat.unit().map_or(String::new(), |unit| format!(" {unit}"));
            format!(
                "{} {}{unit}",
                stat.label(),
                words::signed(f64::from(*value))
            )
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

fn trim(value: f32) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        format!("{rounded}")
    }
}
