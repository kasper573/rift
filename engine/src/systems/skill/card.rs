use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use ui::card::{self, Card, CardResponse, CardTilt};
use ui::egui;
use ui::egui::{Rect, Response, Ui};
use ui::face::{self, TextRole};
use ui::float;
use ui::glyph::CardKind;
use ui::ink;

use super::{Rank, SkillDef, SkillId, SkillKind, SkillMessages, SkillStat};
use crate::core::card::CardStyleDef;
use crate::core::content::Content;
use crate::core::geometry::Shape;
use crate::core::gui::{self, Band};
use crate::core::stage::card::HungCardLamp;
use crate::core::text::{self, StyledLine, TextLook};
use crate::systems::chat::ChatEmbed;
use crate::systems::class::{self, ClassId, NodeKey};
use crate::systems::messages::{self, Direction, Messages};

use crate::core::script::condition;
use crate::core::text::LineText;
use crate::systems::equipment::Equipment;
use crate::systems::equipment::category;
use crate::systems::hud::{self, WindowKey};
use crate::systems::item::card::{self as item_card, CardLook};
use crate::systems::item::{Inventory, ItemId, ItemStack};
use crate::systems::mention::Mention;
use crate::systems::player::session;
use crate::systems::requirement;
use crate::systems::screen::ClientScreen;
use crate::systems::skill::Learned;
use crate::systems::stat::{self, Vital};
use crate::systems::terminal::chat;

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
                hud::raise_pinned::<SkillId>(WindowKey::SkillCard)
                    .run_if(in_state(ClientScreen::Area)),
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

/// How far the pinned card is turned toward the pointer.
pub fn tilt(world: &World) -> CardTilt {
    hud::pinned_tilt(world, WindowKey::SkillCard)
}

/// The lamp the pinned card is lit by, while it is on the stage.
pub fn lamp(world: &World) -> Option<HungCardLamp> {
    hud::pinned_lamp(world, WindowKey::SkillCard)
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
    hud::window_layer(WindowKey::SkillCard)
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
        text::show(
            ui,
            world,
            &line.styled(&content),
            TextLook::role(TextRole::Tooltip, ink::INK2),
            None,
        );
        if let Some(hint) = hint {
            let look = TextLook::new(TextRole::Small.size(), ink::INK3);
            text::show(ui, world, hint, look, None);
        }
    });
}

pub fn kind(def: &SkillDef) -> CardKind {
    match def.kind {
        SkillKind::Active => CardKind::Active,
        SkillKind::Passive => CardKind::Passive,
    }
}

pub fn show(ui: &mut Ui, world: &mut World, card: &SkillCard, look: CardLook) -> CardResponse {
    let content = world.resource::<Content>().clone();
    let def = card.skill.get(&content);
    let all = messages::of(&content);
    let words = &all.skill;
    let tally = rank_tally(words, def, card.rank);
    let learned = card.rank > Rank::NONE;
    let icon = gui::image(world, &def.icon).map(|image| image.texture);
    let band = match def.kind {
        SkillKind::Active => words.active_band(),
        SkillKind::Passive => words.passive_band(),
    };
    let mut shown = Card::new(kind(def), band, &def.name);
    shown.dress = CardStyleDef::of(&content, def.card_style).dress(world, None);
    shown.tally = Some(&tally);
    shown.picture = icon;
    shown.close = look.closable.then_some(all.interface.close());
    shown.actions = vec![chat::card_action(&all.terminal)];
    shown.depth = look.depth;
    let response = shown.show(ui, |ui, plate| {
        if plate.name_cut {
            card::full_name(ui, &def.name);
        }
        text::show(
            ui,
            world,
            &card_overview(&content, def, card).styled(&content),
            TextLook::role(TextRole::CardBody, ink::INK2),
            None,
        );
        card::heading(ui, plate, words.heading_ranks());
        for (index, level) in def.levels.iter().enumerate() {
            let rank = Rank(index as u8 + 1);
            let lit = rank <= card.rank;
            let color = if lit { ink::INK } else { ink::INK3 };
            ui.add(
                egui::Label::new(
                    TextRole::CardBody
                        .rich(rank_line(all, def, rank, level.cost.0))
                        .color(color),
                )
                .wrap(),
            );
        }
        if !def.masteries.is_empty() {
            card::heading(ui, plate, words.heading_masters());
            for recipe in def.masteries.iter().filter_map(|key| key.get(&content)) {
                let makes = ItemStack::summed(&recipe.makes);
                item_card::detail_line(ui, world, None, &makes);
            }
        }
        if let Some(access) = &card.node {
            let lines = access_lines(world, &content, access);
            if !lines.is_empty() {
                let heading = if learned {
                    words.heading_unlocked_by()
                } else {
                    words.heading_costs()
                };
                card::heading(ui, plate, heading);
                for (line, met) in lines {
                    item_card::detail_line(ui, world, Some(met), &line);
                }
            }
        }
        card::heading(ui, plate, words.heading_to_use());
        let needs = use_lines(world, &content, def, card.rank);
        if needs.is_empty() {
            card::fact(ui, words.nothing_needed());
        }
        for (line, met) in needs {
            item_card::detail_line(ui, world, Some(met), &line);
        }
    });
    chat::offered(
        world,
        response.actions.first(),
        ChatEmbed::Skill(card.skill),
    );
    response
}

/// What a skill's card says under its name. Its band already says its kind and its rank, and
/// its sections what it still needs.
fn card_overview(content: &Content, def: &SkillDef, card: &SkillCard) -> LineText {
    match card.granted_by {
        Some(item) => lent_line(content, item),
        None => LineText::plain(use_line(&messages::of(content).skill, def, card.rank)),
    }
}

/// What a skill's tooltip says under its name, which has no band to say any of it.
fn overview(world: &World, content: &Content, def: &SkillDef, card: &SkillCard) -> LineText {
    if let Some(item) = card.granted_by {
        return lent_line(content, item);
    }
    let words = &messages::of(content).skill;
    if card.rank == Rank::NONE {
        let wanting = card.node.as_ref().and_then(|access| {
            access_lines(world, content, access)
                .into_iter()
                .find(|(_, met)| !met)
        });
        return match wanting {
            Some((line, _)) => words.not_learned_needs(line),
            None => LineText::plain(words.not_learned()),
        };
    }
    let usage = use_line(words, def, card.rank);
    let (rank, count) = (card.rank.0, def.rank_count());
    LineText::plain(match def.kind {
        SkillKind::Active => words.tooltip_active(rank, count, usage),
        SkillKind::Passive => words.tooltip_passive(rank, count, usage),
    })
}

fn rank_tally(words: &SkillMessages, def: &SkillDef, rank: Rank) -> String {
    words.rank_tally(def.rank_count(), rank.0)
}

fn lent_line(content: &Content, item: ItemId) -> LineText {
    messages::of(content)
        .skill
        .lent_line(Mention::Item(item).span(content))
}

fn draw_card(world: &mut World) {
    let material = |world: &World, skill: &SkillId| {
        let content = world.resource::<Content>();
        CardStyleDef::of(content, skill.get(content).card_style)
            .material
            .clone()
    };
    hud::draw_pinned::<SkillId>(
        world,
        WindowKey::SkillCard,
        material,
        |ui, world, skill, depth| {
            let card = of(world, *skill);
            let look = CardLook {
                depth,
                ..CardLook::PINNED
            };
            show(ui, world, &card, look)
        },
    );
}

fn use_line(words: &SkillMessages, def: &SkillDef, rank: Rank) -> String {
    if def.kind == SkillKind::Passive {
        return words.holds_while_known().to_owned();
    }
    let rank = rank.max(Rank::FIRST);
    let mut parts = vec![def.target.named(words).to_owned()];
    if let Some(shape) = def.shape_at(rank) {
        parts.push(area_words(words, shape));
    }
    if let Some(Some(range)) = def.target.range() {
        parts.push(words.range_of(range.0 + def.stat(rank, SkillStat::Range)));
    }
    let cooldown = def.cooldown_at(rank);
    if cooldown.0 > 0.0 {
        parts.push(words.cooldown_of(cooldown.0));
    }
    parts.join(" · ")
}

fn area_words(words: &SkillMessages, shape: Shape) -> String {
    match shape {
        Shape::Circle { radius } => words.area_circle(radius.0),
        Shape::Rect { width, height } => words.area_rect(height.0, width.0),
        Shape::Cone { angle, length } => words.area_cone(angle.0, length.0),
        Shape::Line { length, width } => words.area_line(length.0, width.0),
    }
}

fn rank_line(all: &Messages, def: &SkillDef, rank: Rank, cost: u32) -> String {
    let level = &def.levels[usize::from(rank.0) - 1];
    let changes: Vec<String> = level
        .stats
        .iter()
        .map(|(stat, value)| {
            all.skill
                .stat_change(stat.key(), value.abs(), Direction::of(*value))
        })
        .collect();
    if changes.is_empty() {
        all.skill.rank_cost(rank.0, cost)
    } else {
        all.skill
            .rank_cost_changing(rank.0, cost, all.interface.listed_words(changes))
    }
}

fn access_lines(world: &World, content: &Content, access: &NodeAccess) -> Vec<(LineText, bool)> {
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
            (requirement.line(content, &class.tree), met)
        })
        .collect()
}

fn use_lines(
    world: &World,
    content: &Content,
    def: &SkillDef,
    rank: Rank,
) -> Vec<(LineText, bool)> {
    let me = session::me(world);
    let all = messages::of(content);
    let words = &all.skill;
    let mut lines: Vec<(LineText, bool)> = def
        .requires
        .iter()
        .flat_map(condition::parts)
        .map(|part| {
            let met = me.is_some_and(|me| condition::holds(world, me, part));
            let what = requirement::describe(content, all, part);
            (what, met)
        })
        .collect();
    if !def.equipment_categories.is_empty() {
        let met = me.is_some_and(|me| category::equips_any(world, me, &def.equipment_categories));
        lines.push((category::wanted(content, &def.equipment_categories), met));
    }
    let mana = def.mana_at(rank.max(Rank::FIRST));
    if mana > 0.0 {
        let held = me.map_or(0.0, |me| stat::current(world, me, Vital::Mana));
        lines.push((LineText::plain(words.spends_mana(mana)), held >= mana));
    }
    let bag = me.and_then(|me| world.get::<Inventory>(me));
    for stack in &def.cost {
        let held = bag.map_or(0, |bag| bag.count(stack.item));
        lines.push((words.spends_item(held, stack.named()), held >= stack.count));
    }
    lines
}
