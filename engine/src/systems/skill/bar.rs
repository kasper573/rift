use std::collections::BTreeMap;
use ui::drag;
use ui::egui;
use ui::menu;

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::VariantArray;
use ui::egui::{Align2, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use ui::face::TextRole;
use ui::glyph::{self, Halo, Mark};
use ui::{controls, ink, sheet, slip};

use super::{
    AssignBasicAttackRequest, AssignHotbarRequest, BasicAttack, BasicAttackPick, Cooldowns, Hotbar,
    HotbarEntry, HotbarSlot, Rank, SkillId, Targeting, UseSkillRequest, card, casting,
};
use crate::core::content::ImageRef;
use crate::core::content::{Content, Fixture};
use crate::core::gui;
use crate::core::text::{self, StyledLine, TextLook, TextPiece};
use crate::core::time;
use crate::core::units::Seconds;
use crate::systems::equipment::{self, EquipmentPlace, UnequipRequest};
use crate::systems::hud::{self, Carried, Confirm};
use crate::systems::input::map::{
    self, InputAction, InputBinding, InputFixture, InputMap, KeyGesture, KeyModifiers, input,
};
use crate::systems::input::scope;
use crate::systems::interface::InterfaceSound;
use crate::systems::item::widget::{SLOT, SlotLook, slot};
use crate::systems::item::{Inventory, ItemId, ItemKind, UseItemRequest, card as item_card};
use crate::systems::player::session;
use crate::systems::reach::Engaged;
use crate::systems::script::condition;
use crate::systems::settings::{BarKey, BarShape, UserSettings};
use crate::systems::skill::{CastAim, aim};
use crate::systems::{GameSettings, notification, stat};
use ui::paint;
use ui::panel::{self, GripCorner};

pub const SLOT_KEYS: [InputAction; 9] = [
    InputAction::BarSlot1,
    InputAction::BarSlot2,
    InputAction::BarSlot3,
    InputAction::BarSlot4,
    InputAction::BarSlot5,
    InputAction::BarSlot6,
    InputAction::BarSlot7,
    InputAction::BarSlot8,
    InputAction::BarSlot9,
];

const GAP: f32 = 4.0;
const PITCH: f32 = SLOT + GAP;
const BASIC_ATTACK_GAP: f32 = 10.0;
const GROOVE_INSET: f32 = 4.0;
const PRESS_MARK_SECONDS: f32 = 0.6;
const NOTICE_GAP: f32 = 6.0;

pub struct BarSlots {
    shape: BarShape,
}

impl BarSlots {
    pub fn shown(world: &mut World, gui: &egui::Context) -> BarSlots {
        BarSlots {
            shape: shown_shape(world, gui),
        }
    }

    pub fn resting_size() -> Vec2 {
        size_of(BarShape::DEFAULT)
    }

    pub fn size(&self) -> Vec2 {
        size_of(self.shape)
    }

    pub fn show(&self, ui: &mut Ui, world: &mut World) {
        let slots = slots(world);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(GAP);
            basic_attack_slot(ui, world);
            let (between, _) =
                ui.allocate_exact_size(Vec2::new(BASIC_ATTACK_GAP - GAP, SLOT), Sense::hover());
            let inset = Vec2::new(0.0, GROOVE_INSET);
            sheet::groove(
                ui.painter(),
                between.center_top() + inset,
                between.center_bottom() - inset,
            );
            egui::Grid::new("bar slots")
                .spacing(Vec2::splat(GAP))
                .min_col_width(SLOT)
                .min_row_height(SLOT)
                .show(ui, |ui| {
                    for row in 0..self.shape.rows {
                        for column in 0..self.shape.columns {
                            let slot = row * self.shape.columns + column;
                            if slot < slots {
                                bar_slot(ui, world, HotbarSlot(slot));
                            }
                        }
                        ui.end_row();
                    }
                });
        });
    }
}

pub fn reshape_grip(ui: &mut Ui, world: &mut World, paper: Rect) {
    let corner = if paper.max.y + PITCH > ui.ctx().content_rect().bottom() {
        GripCorner::TopRight
    } else {
        GripCorner::BottomRight
    };
    let grip = panel::grip(
        ui,
        ui.id().with("bar grip"),
        "Reshape the bar",
        paper,
        corner,
    );
    resize(world, ui, &grip, paper, corner);
}

pub fn rebind_notice(ui: &mut Ui, world: &World, foot: Pos2) {
    let Some(slot) = world.resource::<BarState>().rebinding else {
        return;
    };
    let line = StyledLine::new(vec![
        TextPiece::text(format!("Press a key for slot {} · ", slot.number())),
        input(InputAction::Dismiss),
        TextPiece::text(" cancels"),
    ]);
    let look = TextLook::new(TextRole::Small.size(), ink::INK);
    let size = text::measure(ui.ctx(), world, &line, look, f32::INFINITY) + slip::PADDING.sum();
    let rect = Rect::from_min_size(
        Pos2::new(foot.x - size.x / 2.0, foot.y - size.y - NOTICE_GAP),
        size,
    );
    slip::paint(ui.painter(), rect);
    let inner = rect - slip::PADDING;
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
        text::show(ui, world, &line, look, None);
    });
}

#[derive(Resource, Default)]
pub struct BarState {
    rebinding: Option<HotbarSlot>,
    primed: Option<HotbarSlot>,
    resizing: Option<Resizing>,
    presses: BTreeMap<HotbarSlot, Press>,
}

/// How a press on a slot went: what it held was used, or the press was refused, by the slot's own
/// state or by what the use ran into.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotPress {
    Used,
    Denied,
}

/// What each shown slot holds and how it is drawn, for whoever probes the client.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct SlotReport {
    pub entry: HotbarEntry,
    pub key: Option<String>,
    pub cooling: bool,
    pub blocked: bool,
    pub dimmed: bool,
    /// Waiting for the next key pressed to become its key.
    pub rebinding: bool,
    /// How the last press on the slot went, if it was ever pressed.
    pub pressed: Option<SlotPress>,
    /// Its skill is armed and waits on a target.
    pub primed: bool,
}

/// Puts something on the first free slot of the bar as it is shown, or says the bar is full.
pub fn pin(world: &mut World, entry: HotbarEntry) {
    let Some(me) = session::me(world) else {
        return;
    };
    let shown = shown(world);
    let free = world
        .get::<Hotbar>(me)
        .map_or(Some(HotbarSlot(0)), |hotbar| hotbar.first_free(shown));
    match free {
        Some(slot) => {
            world.write_message(AssignHotbarRequest { slot, entry });
            InterfaceSound::UiPick.play(world);
        }
        None => notification::local_error(world, "No free slot on the bar"),
    }
}

pub fn pin_bag_slot(world: &mut World, slot: u32) {
    let Some(item) = bag_item(world, slot) else {
        return;
    };
    pin(world, HotbarEntry::Item(item));
}

/// Takes the key a rebind waits on. It runs whether or not the interface claims the keyboard:
/// the menu button that started the rebind keeps egui's focus for the frame after its click.
pub fn capture_key(world: &mut World) {
    if let Some(slot) = world.resource::<BarState>().rebinding {
        capture(world, slot);
    }
}

/// Fires the slot a key was pressed for.
pub fn keys(world: &mut World) {
    if world.resource::<BarState>().rebinding.is_some() {
        return;
    }
    if session::is_locked(world) || !session::is_alive(world) {
        return;
    }
    let Some(me) = session::me(world) else {
        return;
    };
    let pressed = (0..shown(world))
        .map(HotbarSlot)
        .find(|slot| key_pressed(world, *slot));
    let Some(slot) = pressed else {
        return;
    };
    let entry = world
        .get::<Hotbar>(me)
        .map_or(HotbarEntry::Empty, |hotbar| hotbar.entry(slot));
    press(world, slot, entry);
}

pub fn cancel_rebind(world: &mut World) -> bool {
    world.resource_mut::<BarState>().rebinding.take().is_some()
}

/// Uses what a slot holds: a skill is cast or aimed, an item is used from the bag. A slot drawn as
/// cooling or blocked counts its press as denied whatever the use comes to.
pub fn fire(world: &mut World, entry: HotbarEntry) -> SlotPress {
    let ready = matches!(SlotView::of(world, entry).fill, SlotFill::Ready);
    let used = match entry {
        HotbarEntry::Empty => false,
        HotbarEntry::Skill(skill) => use_skill(world, skill),
        HotbarEntry::Item(item) => use_item(world, item),
    };
    if used && ready {
        SlotPress::Used
    } else {
        SlotPress::Denied
    }
}

/// An entry let go of anywhere but a slot leaves the bar, once the player agrees. A slot that took
/// the drop has cleared what was carried by now.
pub fn drop_off_bar(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let Some(Carried::BarEntry { from, entry }) = drag::carried::<Carried>(&gui) else {
        return;
    };
    if !gui.input(|input| input.pointer.any_released()) {
        return;
    }
    let content = world.resource::<Content>().clone();
    let name = entry_name(&content, entry);
    hud::confirm(
        world,
        Confirm {
            title: format!("Take {name} off the bar?"),
            body: vec![
                "It stays in your tree or your inventory; only the slot empties.".to_owned(),
            ],
            confirm: "Take off".to_owned(),
            cancel: "Keep".to_owned(),
            answer: Box::new(move |world, taken| {
                if taken {
                    world.write_message(AssignHotbarRequest {
                        slot: from,
                        entry: HotbarEntry::Empty,
                    });
                }
            }),
        },
    );
}

pub fn forget(mut state: ResMut<BarState>) {
    *state = BarState::default();
}

pub fn report(world: &World) -> Vec<SlotReport> {
    let hotbar = session::my_character(world)
        .and_then(|me| me.get::<Hotbar>())
        .cloned()
        .unwrap_or_default();
    (0..shown(world))
        .map(HotbarSlot)
        .map(|slot| {
            let entry = hotbar.entry(slot);
            let view = SlotView::of(world, entry);
            let state = world.resource::<BarState>();
            SlotReport {
                entry,
                key: key_name(world, slot),
                cooling: matches!(view.fill, SlotFill::Cooling { .. }),
                blocked: matches!(view.fill, SlotFill::Blocked(_)),
                dimmed: view.dimmed,
                rebinding: state.rebinding == Some(slot),
                pressed: state.presses.get(&slot).map(|press| press.outcome),
                primed: primed(world, slot, entry),
            }
        })
        .collect()
}

pub fn entry_name(content: &Content, entry: HotbarEntry) -> String {
    match entry {
        HotbarEntry::Empty => "nothing".to_owned(),
        HotbarEntry::Skill(skill) => skill.get(content).name.clone(),
        HotbarEntry::Item(item) => item.get(content).name.clone(),
    }
}

#[derive(Clone, Copy)]
struct Press {
    outcome: SlotPress,
    since: Seconds,
}

#[derive(Clone, Copy)]
struct Resizing {
    from: BarShape,
    grab: Pos2,
    corner: GripCorner,
}

/// How a slot is drawn over its icon: free, draining while a cooldown runs, or held under a
/// hatched fill with the glyph of what blocks it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum SlotFill {
    Ready,
    Cooling { fraction: f32, left: Seconds },
    Blocked(Glyph),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Glyph {
    Lock,
    Zero,
    Cross,
}

struct SlotView {
    icon: Option<ImageRef>,
    count: u32,
    dimmed: bool,
    fill: SlotFill,
    name: String,
}

impl SlotView {
    fn empty() -> SlotView {
        SlotView {
            icon: None,
            count: 0,
            dimmed: false,
            fill: SlotFill::Ready,
            name: "empty".to_owned(),
        }
    }

    fn of(world: &World, entry: HotbarEntry) -> SlotView {
        let content = world.resource::<Content>();
        let Some(me) = session::me(world) else {
            return SlotView::empty();
        };
        match entry {
            HotbarEntry::Empty => SlotView::empty(),
            HotbarEntry::Skill(skill) => {
                let def = skill.get(content);
                let rank = super::rank_of(world, me, skill);
                SlotView {
                    icon: Some(def.icon.clone()),
                    count: 0,
                    dimmed: rank == Rank::NONE,
                    fill: skill_fill(world, me, skill, rank),
                    name: def.name.clone(),
                }
            }
            HotbarEntry::Item(item) => {
                let def = item.get(content);
                let held = world
                    .get::<Inventory>(me)
                    .map_or(0, |inventory| inventory.count(item));
                let holds = super::hotbar::holds_item(world, me, item);
                SlotView {
                    icon: Some(def.icon.clone()),
                    count: held,
                    dimmed: !holds,
                    fill: if !holds {
                        SlotFill::Blocked(Glyph::Zero)
                    } else {
                        SlotFill::Ready
                    },
                    name: def.name.clone(),
                }
            }
        }
    }
}

fn bag_item(world: &World, slot: u32) -> Option<ItemId> {
    session::my_character(world)
        .and_then(|me| me.get::<Inventory>())
        .and_then(|inventory| inventory.slots.get(slot as usize))
        .map(|stack| stack.item)
}

fn worn_item(world: &World, slot: &EquipmentPlace) -> Option<ItemId> {
    equipment::in_slot(world, session::me(world)?, slot)
}

fn skill_fill(world: &World, me: Entity, skill: SkillId, rank: Rank) -> SlotFill {
    let content = world.resource::<Content>();
    let def = skill.get(content);
    if stat::is_dead(world, me) {
        return SlotFill::Blocked(Glyph::Cross);
    }
    if let Some(now) = time::server_now(world)
        && let Some(ready) = world
            .get::<Cooldowns>(me)
            .and_then(|cooldowns| cooldowns.ready_at(skill))
        && ready > now
    {
        let left = ready.since(now);
        let whole = def
            .cooldown_at(rank.max(Rank::FIRST))
            .0
            .max(left.0)
            .max(0.001);
        return SlotFill::Cooling {
            fraction: (left.0 / whole).clamp(0.0, 1.0),
            left,
        };
    }
    if condition::first_unmet(world, me, def.requires.iter()).is_some() {
        return SlotFill::Blocked(Glyph::Lock);
    }
    let bag = world.get::<Inventory>(me);
    if def
        .cost
        .iter()
        .any(|stack| bag.map_or(0, |bag| bag.count(stack.item)) < stack.count)
    {
        return SlotFill::Blocked(Glyph::Zero);
    }
    SlotFill::Ready
}

fn bar_slot(ui: &mut Ui, world: &mut World, index: HotbarSlot) {
    let content = world.resource::<Content>().clone();
    let entry = session::my_character(world)
        .and_then(|me| me.get::<Hotbar>())
        .map_or(HotbarEntry::Empty, |hotbar| hotbar.entry(index));
    let view = SlotView::of(world, entry);
    let rebinding = world.resource::<BarState>().rebinding == Some(index);
    let carried = drag::carried::<Carried>(ui.ctx());
    let ghost = matches!(carried, Some(Carried::BarEntry { from, .. }) if from == index);
    let look = SlotLook {
        icon: view.icon.as_ref(),
        count: view.count,
        faded: view.dimmed || ghost,
        ..SlotLook::EMPTY
    };
    let response = slot(ui, world, look, Sense::click_and_drag());
    if carried.is_some() && response.contains_pointer() {
        drop_wash(ui, response.rect);
    }
    if let Some(key) = key_name(world, index) {
        slot_key_mark(ui, response.rect, &key);
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("Bar slot {}: {}", index.number(), view.name),
        )
    });
    overlay(ui, response.rect, view.fill);
    press_mark(ui, world, index, response.rect);
    if primed(world, index, entry) {
        controls::primed_ring(ui.painter(), response.rect, ui.input(|input| input.time));
        ui.ctx().request_repaint();
    }
    if rebinding {
        ui.painter()
            .rect_filled(response.rect, 3.0, ink::wash(ink::CREAM, 0.85));
        ui.painter().rect_stroke(
            response.rect,
            3.0,
            Stroke::new(2.0, ink::GOLD),
            StrokeKind::Inside,
        );
        ui.painter().text(
            response.rect.center(),
            Align2::CENTER_CENTER,
            "key?",
            TextRole::KeyHint.font_id(),
            ink::INK,
        );
    }
    take_drop(world, &response, index, entry);
    if map::clicked(world, InputAction::PinToBar, &response) {
        if entry != HotbarEntry::Empty {
            world.write_message(AssignHotbarRequest {
                slot: index,
                entry: HotbarEntry::Empty,
            });
        }
    } else if map::clicked(world, InputAction::UseSlot, &response) {
        press(world, index, entry);
    } else if map::drag_started(world, InputAction::CarryItem, &response)
        && entry != HotbarEntry::Empty
    {
        drag::carry(ui.ctx(), Carried::BarEntry { from: index, entry });
    }
    let options = map::clicked(world, InputAction::SlotOptions, &response);
    menu::at_pointer(&response, options, |ui| {
        if ui.button("Rebind key").clicked() {
            world.resource_mut::<BarState>().rebinding = Some(index);
            ui.close();
        }
        if ui
            .add_enabled(entry != HotbarEntry::Empty, egui::Button::new("Clear slot"))
            .clicked()
        {
            world.write_message(AssignHotbarRequest {
                slot: index,
                entry: HotbarEntry::Empty,
            });
            ui.close();
        }
        ui.separator();
        let mut said = vec![TextPiece::text(format!(
            "{} · ",
            entry_name(&content, entry)
        ))];
        said.extend(key_cap(world, index).map_or_else(
            || vec![TextPiece::text("no key")],
            |key| vec![TextPiece::text("key "), key],
        ));
        text::show(
            ui,
            world,
            &StyledLine::new(said),
            TextLook::new(TextRole::Small.size(), ink::INK3),
            None,
        );
    });
    slot_tooltip(world, &response, index, entry);
}

fn basic_attack_slot(ui: &mut Ui, world: &mut World) {
    let content = world.resource::<Content>().clone();
    let held = session::my_character(world)
        .and_then(|me| me.get::<BasicAttack>())
        .and_then(|slot| slot.0);
    let entry = held.map_or(HotbarEntry::Empty, HotbarEntry::Skill);
    let view = SlotView::of(world, entry);
    let carried =
        drag::carried::<Carried>(ui.ctx()).and_then(|carried| carried_pick(world, carried));
    let look = SlotLook {
        icon: view.icon.as_ref(),
        faded: view.dimmed,
        ..SlotLook::EMPTY
    };
    let response = slot(ui, world, look, Sense::click_and_drag());
    if let Some(pick) = carried
        && response.contains_pointer()
    {
        if refusal(&content, pick).is_none() {
            drop_wash(ui, response.rect);
        } else {
            glyph::hatch(ui.painter(), response.rect.shrink(1.0), ink::ink_wash(0.45));
        }
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("Basic attack slot: {}", view.name),
        )
    });
    overlay(ui, response.rect, view.fill);
    if let Some(pick) =
        drag::dropped::<Carried>(&response).and_then(|dropped| carried_pick(world, dropped))
    {
        match refusal(&content, pick) {
            None => {
                world.write_message(AssignBasicAttackRequest { pick });
                InterfaceSound::UiPick.play(world);
            }
            Some(refusal) => notification::local_error(world, refusal),
        }
    }
    if map::clicked(world, InputAction::PinToBar, &response) {
        if held.is_some() {
            world.write_message(AssignBasicAttackRequest {
                pick: BasicAttackPick::Empty,
            });
        }
    } else if map::drag_started(world, InputAction::CarryItem, &response)
        && let Some(skill) = held
    {
        drag::carry(ui.ctx(), Carried::Skill(skill));
    }
    let options = map::clicked(world, InputAction::SlotOptions, &response);
    menu::at_pointer(&response, options, |ui| {
        if ui
            .add_enabled(held.is_some(), egui::Button::new("Clear"))
            .clicked()
        {
            world.write_message(AssignBasicAttackRequest {
                pick: BasicAttackPick::Empty,
            });
            ui.close();
        }
    });
    match held {
        Some(skill) => {
            let hint = StyledLine::new(vec![
                input(InputAction::Interact),
                TextPiece::text(" on an enemy uses it · "),
                input(InputAction::CarryItem),
                TextPiece::text(" another basic attack here to swap it · "),
                input(InputAction::PinToBar),
                TextPiece::text(" or "),
                input(InputAction::SlotOptions),
                TextPiece::text(" clears it"),
            ]);
            card::tooltip(&response, world, &card::of(world, skill), Some(&hint));
        }
        None => {
            let hint = StyledLine::new(vec![
                TextPiece::text("Wear a weapon that has one, or "),
                input(InputAction::CarryItem),
                TextPiece::text(" one here"),
            ]);
            hud::tooltip(&response, world, "Basic attack · empty", &[], Some(&hint));
        }
    }
}

/// What a drag over the basic attack slot would put there: a skill carried as itself or on a
/// bar entry, or a carried item as the weapon whose attack it is.
fn carried_pick(world: &World, carried: Carried) -> Option<BasicAttackPick> {
    match carried {
        Carried::Skill(skill) => Some(BasicAttackPick::Skill(skill)),
        Carried::BagSlot(slot) => bag_item(world, slot).map(BasicAttackPick::Weapon),
        Carried::Worn(slot) => worn_item(world, &slot).map(BasicAttackPick::Weapon),
        Carried::BarEntry { entry, .. } => match entry {
            HotbarEntry::Skill(skill) => Some(BasicAttackPick::Skill(skill)),
            HotbarEntry::Item(item) => Some(BasicAttackPick::Weapon(item)),
            HotbarEntry::Empty => None,
        },
    }
}

/// Why the basic attack slot turns a pick away, judged from the content alone; whether the player
/// knows the skill or holds the weapon is the server's to say.
fn refusal(content: &Content, pick: BasicAttackPick) -> Option<String> {
    match pick {
        BasicAttackPick::Empty => None,
        BasicAttackPick::Skill(skill) => {
            let def = skill.get(content);
            (!def.basic_attack).then(|| format!("{} can't be a basic attack", def.name))
        }
        BasicAttackPick::Weapon(item) => super::first_flagged_grant(content, std::iter::once(item))
            .is_none()
            .then(|| format!("{} has no basic attack", item.get(content).name)),
    }
}

fn take_drop(world: &mut World, response: &egui::Response, index: HotbarSlot, held: HotbarEntry) {
    let Some(dropped) = drag::dropped::<Carried>(response) else {
        return;
    };
    match dropped {
        Carried::Skill(skill) => {
            world.write_message(AssignHotbarRequest {
                slot: index,
                entry: HotbarEntry::Skill(skill),
            });
        }
        Carried::BagSlot(slot) => {
            let Some(item) = bag_item(world, slot) else {
                return;
            };
            world.write_message(AssignHotbarRequest {
                slot: index,
                entry: HotbarEntry::Item(item),
            });
        }
        Carried::Worn(slot) => {
            let Some(item) = worn_item(world, &slot) else {
                return;
            };
            world.write_message(AssignHotbarRequest {
                slot: index,
                entry: HotbarEntry::Item(item),
            });
        }
        Carried::BarEntry { from, entry } => {
            if from == index {
                return;
            }
            world.write_message(AssignHotbarRequest { slot: index, entry });
            world.write_message(AssignHotbarRequest {
                slot: from,
                entry: held,
            });
        }
    }
    InterfaceSound::UiPick.play(world);
}

fn slot_tooltip(
    world: &mut World,
    response: &egui::Response,
    index: HotbarSlot,
    entry: HotbarEntry,
) {
    let key = key_cap(world, index);
    let options = match &key {
        Some(_) => vec![
            input(InputAction::SlotOptions),
            TextPiece::text(" for options"),
        ],
        None => vec![
            TextPiece::text("No key · "),
            input(InputAction::SlotOptions),
            TextPiece::text(" to bind one"),
        ],
    };
    let uses = key.map_or_else(Vec::new, |key| {
        vec![
            key,
            TextPiece::text(" or "),
            input(InputAction::UseSlot),
            TextPiece::text(" uses it · "),
        ]
    });
    let take_off = [
        TextPiece::text(" · "),
        input(InputAction::PinToBar),
        TextPiece::text(" takes it off the bar"),
    ];
    match entry {
        HotbarEntry::Empty => {
            let hint = [
                TextPiece::text("Empty · "),
                input(InputAction::CarryItem),
                TextPiece::text(" a skill or an item here · "),
            ]
            .into_iter()
            .chain(options)
            .collect();
            hud::tooltip(
                response,
                world,
                &format!("Slot {}", index.number()),
                &[],
                Some(&StyledLine::new(hint)),
            );
        }
        HotbarEntry::Skill(skill) => {
            let hint = StyledLine::new(uses.into_iter().chain(options).chain(take_off).collect());
            card::tooltip(response, world, &card::of(world, skill), Some(&hint));
        }
        HotbarEntry::Item(item) => {
            let content = world.resource::<Content>().clone();
            let def = item.get(&content);
            let hint = StyledLine::new(uses.into_iter().chain(options).chain(take_off).collect());
            let lines = [item_card::overview(def)];
            hud::tooltip(response, world, &def.name, &lines, Some(&hint));
        }
    }
}

fn overlay(ui: &Ui, rect: Rect, fill: SlotFill) {
    let painter = ui.painter();
    match fill {
        SlotFill::Ready => {}
        SlotFill::Cooling { fraction, left } => {
            let height = rect.height() * fraction;
            let wash = Rect::from_min_max(Pos2::new(rect.min.x, rect.max.y - height), rect.max);
            painter.rect_filled(wash.shrink(1.0), 2.0, ink::wash(ink::RUBRIC, 0.45));
            if fraction > 0.02 {
                glyph::haloed(
                    painter,
                    rect.center(),
                    Align2::CENTER_CENTER,
                    &cooldown_text(left),
                    TextRole::Count.font_id(),
                    ink::INK,
                    CREAM_HALO,
                );
            }
        }
        SlotFill::Blocked(blocked) => {
            painter.rect_filled(rect.shrink(1.0), 2.0, ink::wash(ink::CREAM, 0.55));
            glyph::hatch(painter, rect.shrink(1.0), ink::ink_wash(0.3));
            match blocked {
                Glyph::Lock => Mark::Lock.paint(
                    painter,
                    Rect::from_center_size(rect.center(), Vec2::splat(14.0)),
                    ink::INK2,
                ),
                Glyph::Zero => {
                    painter.text(
                        rect.right_bottom() - Vec2::new(3.0, 1.0),
                        Align2::RIGHT_BOTTOM,
                        "0",
                        TextRole::Count.font_id(),
                        ink::RUBRIC,
                    );
                }
                Glyph::Cross => Mark::Cross.paint(
                    painter,
                    Rect::from_center_size(rect.center(), Vec2::splat(14.0)),
                    ink::RUBRIC,
                ),
            }
        }
    }
}

fn press_mark(ui: &Ui, world: &World, slot: HotbarSlot, rect: Rect) {
    let Some(press) = world.resource::<BarState>().presses.get(&slot) else {
        return;
    };
    if (time::uptime(world) - press.since).0 >= PRESS_MARK_SECONDS {
        return;
    }
    let color = match press.outcome {
        SlotPress::Used => ink::GOLD,
        SlotPress::Denied => ink::RUBRIC,
    };
    ui.painter()
        .rect_stroke(rect, 3.0, Stroke::new(2.0, color), StrokeKind::Outside);
}

fn drop_wash(ui: &Ui, rect: Rect) {
    ui.painter().rect(
        rect,
        3.0,
        ink::gold_wash(0.28),
        Stroke::new(1.0, ink::OCHRE),
        StrokeKind::Inside,
    );
}

fn slot_key_mark(ui: &Ui, rect: Rect, key: &str) {
    glyph::haloed(
        ui.painter(),
        rect.left_top() + Vec2::new(3.0, 1.0),
        Align2::LEFT_TOP,
        key,
        TextRole::SlotKey.font_id(),
        ink::INK2,
        CREAM_HALO,
    );
}

const CREAM_HALO: Halo = Halo {
    color: ink::CREAM,
    width: 1.0,
};

fn press(world: &mut World, slot: HotbarSlot, entry: HotbarEntry) {
    let outcome = fire(world, entry);
    let since = time::uptime(world);
    let arms = aim::aiming(world).is_some_and(|aimed| entry == HotbarEntry::Skill(aimed.skill));
    let mut state = world.resource_mut::<BarState>();
    state.presses.insert(slot, Press { outcome, since });
    if arms {
        state.primed = Some(slot);
    }
}

/// The slot whose press armed the skill now waiting on its target.
fn primed(world: &World, slot: HotbarSlot, entry: HotbarEntry) -> bool {
    world.resource::<BarState>().primed == Some(slot)
        && aim::aiming(world).is_some_and(|aimed| entry == HotbarEntry::Skill(aimed.skill))
}

fn cooldown_text(left: Seconds) -> String {
    if left.0 >= 60.0 {
        format!("{}m", (left.0 / 60.0).ceil() as u32)
    } else {
        format!("{}", left.0.ceil().max(1.0) as u32)
    }
}

/// Whether the skill was asked for, armed or cast; not whether the server then performs it.
fn use_skill(world: &mut World, skill: SkillId) -> bool {
    let Some(me) = session::me(world) else {
        return false;
    };
    if let Some(refusal) = super::unusable(world, me, skill) {
        notification::local_error(world, refusal);
        return false;
    }
    let content = world.resource::<Content>().clone();
    let def = skill.get(&content);
    let rank = super::rank_of(world, me, skill);
    match def.target {
        Targeting::Caster => {
            world.write_message(UseSkillRequest {
                skill,
                aim: CastAim::None,
            });
            true
        }
        Targeting::SingleAlly | Targeting::SingleEnemy => {
            let engaged = world
                .get::<Engaged>(me)
                .map(|engaged| engaged.0)
                .filter(|target| casting::fits(world, me, *target, def.target));
            match engaged {
                Some(target) => {
                    world.write_message(UseSkillRequest {
                        skill,
                        aim: CastAim::Actor(target),
                    });
                    true
                }
                None => aim_or_cast(world, skill, rank),
            }
        }
        Targeting::AreaAlly | Targeting::AreaEnemy => aim_or_cast(world, skill, rank),
    }
}

fn aim_or_cast(world: &mut World, skill: SkillId, rank: Rank) -> bool {
    if world.resource::<UserSettings>().quick_cast() {
        aim::cast_at_pointer(world, skill)
    } else {
        aim::arm(world, skill, rank);
        true
    }
}

/// Asks to use what lies in the player's bag at `slot`: an item that casts a skill is aimed as
/// the skill is, and any other is used at once.
pub fn use_bag_slot(world: &mut World, slot: u32) -> bool {
    let casts =
        bag_item(world, slot).and_then(|item| super::cast_by(world.resource::<Content>(), item));
    match casts {
        Some(skill) => use_skill(world, skill),
        None => {
            world.write_message(UseItemRequest { slot });
            true
        }
    }
}

/// Whether a use of the item was asked for.
fn use_item(world: &mut World, item: ItemId) -> bool {
    let Some(me) = session::me(world) else {
        return false;
    };
    let content = world.resource::<Content>().clone();
    let def = item.get(&content);
    if matches!(def.kind, ItemKind::Resource) {
        return false;
    }
    let bag_slot = world
        .get::<Inventory>(me)
        .and_then(|inventory| inventory.slot_of(item));
    match bag_slot {
        Some(slot) => use_bag_slot(world, slot as u32),
        None => match equipment::place_of(world, me, item) {
            Some(slot) => {
                world.write_message(UnequipRequest { slot });
                true
            }
            None => {
                notification::local_error(world, format!("No {} left", def.name));
                false
            }
        },
    }
}

/// The key a slot answers to, once the setting is resolved: the game's own action for the slot,
/// or a gesture the player chose.
#[derive(Clone, Copy)]
enum SlotKey {
    Action(InputAction),
    Gesture(KeyGesture),
}

fn slot_key(settings: &UserSettings, slot: HotbarSlot) -> Option<SlotKey> {
    match settings.bar_key(slot) {
        BarKey::Default => SLOT_KEYS.get(slot.index()).copied().map(SlotKey::Action),
        BarKey::Unbound => None,
        BarKey::Custom(gesture) => Some(SlotKey::Gesture(gesture)),
    }
}

fn key_cap(world: &World, slot: HotbarSlot) -> Option<TextPiece> {
    match slot_key(world.resource::<UserSettings>(), slot)? {
        SlotKey::Action(action) => Some(input(action)),
        SlotKey::Gesture(gesture) => Some(TextPiece::key(InputBinding::Key(gesture).name())),
    }
}

fn key_name(world: &World, slot: HotbarSlot) -> Option<String> {
    match slot_key(world.resource::<UserSettings>(), slot)? {
        SlotKey::Action(action) => world.resource::<InputMap>().name(action),
        SlotKey::Gesture(gesture) => Some(InputBinding::Key(gesture).name()),
    }
}

fn key_pressed(world: &World, slot: HotbarSlot) -> bool {
    match slot_key(world.resource::<UserSettings>(), slot) {
        Some(SlotKey::Action(action)) => map::just_pressed(world, action),
        Some(SlotKey::Gesture(gesture)) => {
            scope::reads(world, SLOT_KEYS[0])
                && gesture.just_pressed(world.resource::<ButtonInput<KeyCode>>())
        }
        None => false,
    }
}

/// Takes the next key pressed as the slot's key. A key a fixed action holds is refused; one
/// another slot holds moves over.
fn capture(world: &mut World, slot: HotbarSlot) {
    let keys = world.resource::<ButtonInput<KeyCode>>();
    let Some(&key) = keys
        .get_just_pressed()
        .find(|key| !KeyModifiers::is_modifier(**key))
    else {
        return;
    };
    let modifiers = KeyModifiers::held(keys);
    world.resource_mut::<BarState>().rebinding = None;
    let gesture = KeyGesture { key, modifiers };
    let map = world.resource::<InputMap>().clone();
    if map
        .bindings(InputAction::Dismiss)
        .contains(&InputBinding::Key(gesture))
    {
        return;
    }
    let content = world.resource::<Content>().clone();
    let taken = InputAction::VARIANTS
        .iter()
        .copied()
        .filter(|action| !SLOT_KEYS.contains(action))
        .find(|action| map.bindings(*action).contains(&InputBinding::Key(gesture)));
    if let Some(action) = taken {
        let input = InputFixture::Game.get(&content);
        let label = input.actions.get(&action).map_or("", |bound| &bound.label);
        notification::local_error(
            world,
            format!(
                "{} is taken: {}",
                InputBinding::Key(gesture).name(),
                label.to_lowercase()
            ),
        );
        return;
    }
    let holder = (0..shown(world))
        .map(HotbarSlot)
        .filter(|other| *other != slot)
        .find(
            |other| match slot_key(world.resource::<UserSettings>(), *other) {
                Some(SlotKey::Action(action)) => {
                    map.bindings(action).contains(&InputBinding::Key(gesture))
                }
                Some(SlotKey::Gesture(held)) => held == gesture,
                None => false,
            },
        );
    let mut settings = world.resource_mut::<UserSettings>();
    if let Some(other) = holder {
        settings.set_bar_key(other, BarKey::Unbound);
    }
    settings.set_bar_key(slot, BarKey::Custom(gesture));
    if let Some(other) = holder {
        notification::local_error(
            world,
            format!(
                "Slot {} lost {} to slot {}",
                other.number(),
                InputBinding::Key(gesture).name(),
                slot.number()
            ),
        );
    } else {
        InterfaceSound::UiPick.play(world);
    }
}

/// The shape drawn now: the saved one, or the one the grip is being dragged toward, in whole slots.
fn shown_shape(world: &mut World, gui: &egui::Context) -> BarShape {
    let saved = shape(world);
    let Some(resizing) = world.resource::<BarState>().resizing else {
        return saved;
    };
    let (down, pointer) = gui.input(|input| (input.pointer.any_down(), input.pointer.latest_pos()));
    let shape = pointer.map_or(resizing.from, |pointer| {
        let moved = pointer - resizing.grab;
        let down = match resizing.corner {
            GripCorner::BottomRight => moved.y,
            GripCorner::TopRight => -moved.y,
        };
        BarShape {
            columns: ((resizing.from.columns as f32 * PITCH + moved.x) / PITCH)
                .round()
                .max(1.0) as u32,
            rows: ((resizing.from.rows as f32 * PITCH + down) / PITCH)
                .round()
                .max(1.0) as u32,
        }
        .within(slots(world))
    });
    if !down {
        world.resource_mut::<BarState>().resizing = None;
        if shape != saved {
            world.resource_mut::<UserSettings>().set_bar_shape(shape);
        }
    }
    shape
}

fn resize(world: &mut World, ui: &Ui, grip: &egui::Response, outer: Rect, corner: GripCorner) {
    if grip.drag_started()
        && let Some(grab) = ui.input(|input| input.pointer.press_origin())
    {
        world.resource_mut::<BarState>().resizing = Some(Resizing {
            from: shape(world),
            grab,
            corner,
        });
    }
    if world.resource::<BarState>().resizing.is_some() {
        paint::dashed_rect(ui.painter(), outer, Stroke::new(1.0, ink::OCHRE), 5.0, 3.0);
    }
}

fn size_of(shape: BarShape) -> Vec2 {
    Vec2::new(
        SLOT + BASIC_ATTACK_GAP + shape.columns as f32 * PITCH,
        shape.rows as f32 * PITCH - GAP,
    )
}

/// How many slots the game's hotbar has.
fn slots(world: &World) -> u32 {
    world
        .resource::<Content>()
        .built::<GameSettings>()
        .slots
        .hotbar
}

/// The bar as the player shaped it, as far as the game's hotbar fills it.
fn shape(world: &World) -> BarShape {
    world
        .resource::<UserSettings>()
        .bar_shape()
        .within(slots(world))
}

/// How many slots the bar shows.
fn shown(world: &World) -> u32 {
    shape(world).slots().min(slots(world))
}
