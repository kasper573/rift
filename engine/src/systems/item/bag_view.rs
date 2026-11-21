//! The bag as every window shows it: one heading row whose controls narrow and sort the bag, over
//! the grid of its slots, in which the player arranges stacks by carrying them from slot to slot.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use strum::VariantArray;
use ui::drag;
use ui::egui::{self, Color32, Rect, Response, Stroke, StrokeKind, Ui, Vec2};
use ui::face::TextRole;
use ui::glyph::Mark;
use ui::{controls, ink, input, menu, sheet, theme};

use crate::core::content::Content;
use crate::core::gui;
use crate::systems::equipment::UnequipRequest;
use crate::systems::hud::{self, Carried};
use crate::systems::item::grid::{self, SLOT_GAP, item_grid};
use crate::systems::item::widget::{self, SLOT};
use crate::systems::item::{
    BagSort, Inventory, ItemCategory, ItemDef, ItemMessages, ItemStack, MoveStackRequest,
    SortBagRequest,
};
use crate::systems::messages;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::settings::Placed;

/// The bag as a window shows it.
pub struct BagView<'a> {
    pub heading: &'a str,
    /// The window the bag shows in, under which what narrows it is kept.
    pub placement: Placed,
}

/// What one window narrows the bag to. It lasts while the window shows the bag, and starts empty.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BagFilter {
    pub words: String,
    /// `None` is every type.
    pub category: Option<ItemCategory>,
}

impl BagFilter {
    /// Whether a stack of `def` is shown: every word typed is in its name, whatever their case,
    /// and it is of the type picked.
    pub fn shows(&self, def: &ItemDef) -> bool {
        let name = def.name.to_lowercase();
        self.category
            .is_none_or(|category| def.category() == category)
            && self
                .words
                .split_whitespace()
                .all(|word| name.contains(&word.to_lowercase()))
    }

    /// Whether the bag can be arranged: only with nothing narrowing it, as only then does every
    /// slot show where it is.
    pub fn is_open(&self) -> bool {
        self.words.trim().is_empty() && self.category.is_none()
    }
}

/// The types the game's items are of, in `ItemCategory`'s order: those the type filter offers
/// after every type.
pub fn categories_used(content: &Content) -> Vec<ItemCategory> {
    ItemCategory::VARIANTS
        .iter()
        .copied()
        .filter(|&category| {
            content
                .ids::<ItemDef>()
                .any(|item| item.get(content).category() == category)
        })
        .collect()
}

pub struct BagViewPlugin;

impl Plugin for BagViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BagFilters>()
            .init_resource::<Arranging>()
            .init_resource::<BagsShown>()
            .add_systems(Last, forget_unshown.run_if(in_state(ClientScreen::Area)))
            .add_systems(OnExit(ClientScreen::Area), forget_all);
    }
}

/// Draws the bag as `view` places it.
pub fn bag_view(ui: &mut Ui, world: &mut World, view: BagView) {
    let content = world.resource::<Content>().clone();
    let words = &messages::of(&content).item;
    let bag = Inventory::of(world, session::me(world));
    let now = ui.input(|input| input.time);
    world.resource_mut::<Arranging>().settle(&bag, now);
    carry_lifted(ui, world);
    let mut filters = world.resource_mut::<BagFilters>();
    filters.shown.insert(view.placement);
    let mut filter = filters
        .kept
        .get(&view.placement)
        .cloned()
        .unwrap_or_default();
    let cells = widget::bag_cells(world);
    let shown: Vec<&widget::BagCell> = cells
        .iter()
        .filter(|cell| {
            filter.is_open()
                || cell
                    .stack()
                    .is_some_and(|stack| filter.shows(stack.item.get(&content)))
        })
        .collect();
    let sorting = world.resource::<Arranging>().sorting.is_some();
    let heading_width = ui
        .painter()
        .layout_job(TextRole::Section.job(view.heading, ink::RUBRIC))
        .size()
        .x;
    let sort = sheet::section_with(ui, view.heading, |tools| {
        controls_row(
            tools,
            &content,
            &bag,
            &mut filter,
            &view,
            heading_width,
            sorting,
        )
    });
    if let Some(by) = sort {
        world.write_message(SortBagRequest { by });
        world.resource_mut::<Arranging>().sorting = Some(Asked {
            before: bag.clone(),
            at: now,
        });
    }
    if shown.is_empty() && !filter.is_open() {
        nothing_found(ui, words, &mut filter);
    } else {
        let arranges = filter.is_open();
        let mut hinted = None;
        let grid = grid::scrolled(ui, ("bag", view.placement), |ui| {
            scroll_while_carrying(ui);
            item_grid(ui, shown.len() as u32, |ui, place| {
                let cell = shown[place as usize];
                let settling = world.resource::<Arranging>().settling.clone();
                let faded = settling
                    .as_ref()
                    .is_some_and(|settling| settling.from == cell.slot);
                let response = widget::bag_slot(ui, world, cell, faded, view.placement);
                if let Some(settling) = settling.filter(|settling| settling.to == cell.slot) {
                    ghost(ui, world, response.rect, settling.stack);
                }
                if let Some(does) = dropping(ui, world, &bag, &response, cell.slot, arranges) {
                    hinted = Some((Some(cell.slot), does));
                }
            })
        });
        if hinted.is_none() && is_lifted(ui) {
            let does = words.place_lifted().to_owned();
            hud::drop_hint(world, does.clone());
            hinted = Some((None, Some(does)));
        }
        if let Some(carried) = drag::carried::<Carried>(ui.ctx()) {
            let (over, does) = hinted.unzip();
            world.resource_mut::<BagsShown>().drawing.carried = Some(ShownCarry {
                from: match carried {
                    Carried::BagSlot(from) => Some(from),
                    _ => None,
                },
                over: over.flatten(),
                does: does.flatten(),
            });
        }
        if let Some(carried) = drag::dropped::<Carried>(&grid) {
            widget::dropped_on_bag(world, carried);
        }
    }
    let shown_slots = shown.iter().map(|cell| cell.slot).collect();
    world.resource_mut::<BagsShown>().drawing.views.insert(
        view.placement,
        ShownBag {
            filter: filter.clone(),
            slots: shown_slots,
        },
    );
    world
        .resource_mut::<BagFilters>()
        .kept
        .insert(view.placement, filter);
}

/// What the bag views showed in the last frame drawn, for a driver of the client.
#[derive(Resource, Default)]
pub struct BagsShown {
    pub views: BTreeMap<Placed, ShownBag>,
    /// What is carried over a bag, while something is.
    pub carried: Option<ShownCarry>,
    drawing: BagsDrawn,
}

#[derive(Default)]
struct BagsDrawn {
    views: BTreeMap<Placed, ShownBag>,
    carried: Option<ShownCarry>,
}

/// One window's bag: what narrows it, and the slots it shows, in the order shown.
#[derive(Clone, Debug)]
pub struct ShownBag {
    pub filter: BagFilter,
    pub slots: Vec<u32>,
}

/// Something carried over the bag: the slot it was taken from when it is a stack of the bag, the
/// slot under the pointer, and what letting go there does, as the hint says it.
#[derive(Clone, Debug)]
pub struct ShownCarry {
    pub from: Option<u32>,
    pub over: Option<u32>,
    pub does: Option<String>,
}

/// Lifts the stack in the bag's slot `slot` as if carried, without the pointer held down: the
/// next click on a slot places it, and a click anywhere else or Esc puts it back.
pub fn lift(world: &mut World, slot: u32) {
    world.resource_mut::<Arranging>().lifting = Some(slot);
}

/// Whether the player has narrowed the bag `placement` shows.
pub fn is_narrowed(world: &World, placement: Placed) -> bool {
    world
        .get_resource::<BagFilters>()
        .and_then(|filters| filters.kept.get(&placement))
        .is_some_and(|filter| !filter.is_open())
}

/// What narrows the bag each window shows, kept while the window shows it.
#[derive(Resource, Default)]
struct BagFilters {
    kept: BTreeMap<Placed, BagFilter>,
    /// The windows that showed their bag since the last frame ended.
    shown: BTreeSet<Placed>,
}

/// What the player asked of the bag that the server has yet to answer.
#[derive(Resource, Default)]
struct Arranging {
    /// A stack the card lifted, carried from the first frame that doesn't let go of the pointer.
    lifting: Option<u32>,
    settling: Option<Settling>,
    sorting: Option<Asked>,
}

/// A stack moved from slot `from` to slot `to` of the bag as it was `before`, at `at`.
#[derive(Clone, Debug)]
struct Settling {
    from: u32,
    to: u32,
    stack: ItemStack,
    asked: Asked,
}

#[derive(Clone, Debug)]
struct Asked {
    before: Inventory,
    at: f64,
}

impl Arranging {
    /// Lets go of what the bag `now` answers, or what it never will: a refused move leaves the
    /// bag as it was, and is given up once a reply would long have come.
    fn settle(&mut self, bag: &Inventory, now: f64) {
        let answered = |asked: &Asked| asked.before != *bag || now - asked.at > GIVEN_UP_AFTER;
        if self
            .settling
            .as_ref()
            .is_some_and(|settling| answered(&settling.asked))
        {
            self.settling = None;
        }
        if self.sorting.as_ref().is_some_and(answered) {
            self.sorting = None;
        }
    }
}

/// How wide a pane must be for the bag shown in `placement` to set its heading, search, type
/// picker and sort button on one row, as it last measured them; nothing before it first shows.
pub fn least_bag_width(gui: &egui::Context, placement: Placed) -> f32 {
    gui.data(|data| data.get_temp::<f32>(least_id(placement)))
        .unwrap_or(0.0)
}

fn least_id(placement: Placed) -> egui::Id {
    egui::Id::new(("bag view least width", placement))
}

/// How long the bag waits for the server to answer a move or a sort, in seconds.
const GIVEN_UP_AFTER: f64 = 2.0;
/// How narrow the search grows before the controls crowd the heading: two slots wide.
const SEARCH_LEAST: f32 = 2.0 * SLOT + SLOT_GAP;
const SEARCH_MOST: f32 = 150.0;
/// The least of the heading's rule left between the heading and its controls.
const RULE_LEAST: f32 = 16.0;
/// How near the edge of the bag's pane carrying a stack scrolls it, and how fast.
const EDGE: f32 = 24.0;
const EDGE_SPEED: f32 = 360.0;
const GHOST: f32 = 0.45;

/// Carries the stack the card lifted, once the click that lifted it is over: what is carried is
/// let go of on the frame the pointer is.
fn carry_lifted(ui: &Ui, world: &mut World) {
    let released = ui.input(|input| input.pointer.any_released());
    if released {
        return;
    }
    if let Some(slot) = world.resource_mut::<Arranging>().lifting.take() {
        drag::carry(ui.ctx(), Carried::BagSlot(slot));
    }
}

/// Whether a stack is carried with no button held: lifted from its card.
fn is_lifted(ui: &Ui) -> bool {
    drag::carried::<Carried>(ui.ctx()).is_some_and(|carried| matches!(carried, Carried::BagSlot(_)))
        && !ui.input(|input| input.pointer.any_down())
}

fn forget_unshown(mut filters: ResMut<BagFilters>, mut bags: ResMut<BagsShown>) {
    let shown = std::mem::take(&mut filters.shown);
    filters.kept.retain(|placed, _| shown.contains(placed));
    let drawn = std::mem::take(&mut bags.drawing);
    bags.views = drawn.views;
    bags.carried = drawn.carried;
}

fn forget_all(
    mut filters: ResMut<BagFilters>,
    mut arranging: ResMut<Arranging>,
    mut bags: ResMut<BagsShown>,
) {
    *filters = BagFilters::default();
    *arranging = Arranging::default();
    *bags = BagsShown::default();
}

/// The search, the type picker and the sort button, right to left at the end of the heading
/// row. The rule gives way first, then the search, down to two slots wide.
fn controls_row(
    ui: &mut Ui,
    content: &Content,
    bag: &Inventory,
    filter: &mut BagFilter,
    view: &BagView,
    heading_width: f32,
    sorting: bool,
) -> Option<BagSort> {
    let words = &messages::of(content).item;
    ui.spacing_mut().item_spacing.x = 4.0;
    let before = ui.available_width();
    let sort = sort_button(ui, words, sorting);
    type_picker(ui, content, bag, filter, view.placement);
    let tools = before - ui.available_width() + ui.spacing().item_spacing.x;
    let least = (heading_width + RULE_LEAST + tools + SEARCH_LEAST).ceil();
    let kept = least_id(view.placement);
    if !sorting && ui.data(|data| data.get_temp::<f32>(kept)) != Some(least) {
        ui.data_mut(|data| data.insert_temp(kept, least));
        ui.ctx()
            .request_discard("the bag's heading row changed its least width");
    }
    let room = ui.available_width() - heading_width - RULE_LEAST;
    let width = room.clamp(SEARCH_LEAST, SEARCH_MOST);
    ui.allocate_ui(Vec2::new(width, controls::CONTROL_HEIGHT), |ui| {
        ui.set_width(width);
        let field =
            input::clearable_search(ui, &mut filter.words, words.search_bag(), words.clear());
        let escaped = ui.input(|input| input.key_pressed(egui::Key::Escape));
        if field.lost_focus() && escaped && !filter.words.is_empty() {
            filter.words.clear();
            field.request_focus();
        }
    });
    sort
}

fn sort_button(ui: &mut Ui, words: &ItemMessages, sorting: bool) -> Option<BagSort> {
    if sorting {
        let waiting = ui.add_enabled(false, egui::Button::new(Mark::Hourglass.text(14.0)));
        waiting.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, false, words.sorting())
        });
        return None;
    }
    let shown = menu::button(ui, Mark::Sort.text(14.0), |ui| {
        let picked = BagSort::VARIANTS
            .iter()
            .copied()
            .find(|&by| menu::entry(ui, sort_name(by, words)).clicked());
        ui.label(theme::muted(words.sort_says()));
        picked
    });
    let label = words.sort();
    shown
        .response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    ui::float::tooltip(&shown.response, |ui| {
        ui.label(TextRole::Tooltip.rich(label));
    });
    shown.inner.flatten()
}

fn sort_name(by: BagSort, words: &ItemMessages) -> &str {
    match by {
        BagSort::Kind => words.sort_by_kind(),
        BagSort::Rarity => words.sort_by_rarity(),
        BagSort::Name => words.sort_by_name(),
    }
}

/// Every type the game's items are of, each with how many stacks of it the bag holds: a type the
/// bag holds none of is still listed, dimmed, so the list keeps its shape as the bag changes.
fn type_picker(
    ui: &mut Ui,
    content: &Content,
    bag: &Inventory,
    filter: &mut BagFilter,
    placement: Placed,
) {
    let words = &messages::of(content).item;
    let picked = filter.category.map_or_else(
        || words.all_types().to_owned(),
        |category| category.named(words),
    );
    let name = words.item_type();
    let entries = std::iter::once(words.all_types().to_owned()).chain(
        categories_used(content)
            .into_iter()
            .map(|category| category.named(words)),
    );
    let shown = controls::combo(("bag type", placement))
        .selected_text(picked)
        .fitting(entries)
        .named(name)
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(filter.category.is_none(), words.all_types())
                .clicked()
            {
                filter.category = None;
            }
            for category in categories_used(content) {
                let held = bag
                    .stacks()
                    .filter(|(_, stack)| stack.item.get(content).category() == category)
                    .count() as u32;
                let text = words.category_count(category.named(words), held);
                let text = if held == 0 {
                    theme::muted(text)
                } else {
                    egui::RichText::new(text)
                };
                if ui
                    .selectable_label(filter.category == Some(category), text)
                    .clicked()
                {
                    filter.category = Some(category);
                }
            }
        });
    ui::float::tooltip(&shown.response, |ui| {
        ui.label(TextRole::Tooltip.rich(name));
    });
}

/// The line that stands in for the grid when nothing in the bag is what the player looks for,
/// with the button that shows everything again.
fn nothing_found(ui: &mut Ui, words: &ItemMessages, filter: &mut BagFilter) {
    let looked_for = filter.words.trim().to_owned();
    let said = match filter.category {
        Some(category) if looked_for.is_empty() => words.nothing_of_type(category.named(words)),
        Some(category) => words.nothing_matches_of_type(&looked_for, category.named(words)),
        None => words.nothing_matches(&looked_for),
    };
    ui.horizontal(|ui| {
        ui.label(theme::muted(said));
        if controls::button(ui, words.clear()).clicked() {
            *filter = BagFilter::default();
        }
    });
}

/// Answers what is carried over the bag's slot `to`: marks the slot when letting go there does
/// something, says what where it isn't plain to see, and does it once it is let go of. Whether
/// the slot takes what is carried, and what it said.
fn dropping(
    ui: &Ui,
    world: &mut World,
    bag: &Inventory,
    response: &Response,
    to: u32,
    arranges: bool,
) -> Option<Option<String>> {
    let carried = drag::carried::<Carried>(ui.ctx())?;
    if !response.contains_pointer() {
        return None;
    }
    let content = world.resource::<Content>().clone();
    let words = &messages::of(&content).item;
    let hint = match &carried {
        Carried::BagSlot(from) if *from == to => None,
        Carried::BagSlot(from) if arranges => bag
            .at(*from as usize)
            .map(|moved| arrange_hint(words, bag.at(to as usize), moved, &content)),
        Carried::BagSlot(_) => Some(Some(words.clear_to_arrange().to_owned())),
        Carried::Worn(_) if arranges && bag.at(to as usize).is_none() => {
            Some(Some(words.move_here().to_owned()))
        }
        _ => None,
    };
    let hinted = hint.clone();
    if let Some(said) = hint {
        ui.painter().rect_stroke(
            response.rect,
            3.0,
            Stroke::new(2.0, ink::OCHRE),
            StrokeKind::Inside,
        );
        if let Some(said) = said {
            hud::drop_hint(world, said);
        }
    }
    let Some(dropped) = drag::dropped::<Carried>(response) else {
        return hinted;
    };
    match dropped {
        Carried::BagSlot(from) if arranges && from != to => {
            if let Some(stack) = bag.at(from as usize) {
                world.write_message(MoveStackRequest {
                    from,
                    to,
                    item: stack.item,
                });
                let at = ui.input(|input| input.time);
                world.resource_mut::<Arranging>().settling = Some(Settling {
                    from,
                    to,
                    stack,
                    asked: Asked {
                        before: bag.clone(),
                        at,
                    },
                });
            }
        }
        Carried::BagSlot(_) => {}
        Carried::Worn(slot) => {
            world.write_message(UnequipRequest {
                slot,
                into: arranges.then_some(to),
            });
        }
        carried => widget::dropped_on_bag(world, carried),
    }
    hinted
}

/// What letting `moved` go over a slot holding `there` does, as the hint names it.
/// What letting `moved` go over a slot holding `there` says: nothing for a swap, which the
/// stacks trading places shows plainly.
fn arrange_hint(
    words: &ItemMessages,
    there: Option<ItemStack>,
    moved: ItemStack,
    content: &Content,
) -> Option<String> {
    match there {
        None => Some(words.move_here().to_owned()),
        Some(there) if there.item != moved.item => None,
        Some(there) => {
            let room = moved
                .item
                .get(content)
                .stack_max()
                .saturating_sub(there.count);
            (room > 0).then(|| words.join_some(moved.count.min(room)))
        }
    }
}

/// The faded picture of a stack moved into the slot at `rect`, until the bag says where it is.
fn ghost(ui: &Ui, world: &mut World, rect: Rect, stack: ItemStack) {
    let content = world.resource::<Content>().clone();
    let Some(icon) = gui::image(world, &stack.item.get(&content).icon) else {
        return;
    };
    let whole = Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
    let at = Rect::from_center_size(rect.center(), Vec2::splat(controls::SLOT_ICON));
    let tint = Color32::from_white_alpha((GHOST * 255.0) as u8);
    ui.painter().image(icon.texture, at, whole, tint);
}

/// Scrolls the pane the bag is in while something carried is held near its top or bottom edge,
/// so a slot out of sight can still be reached.
fn scroll_while_carrying(ui: &mut Ui) {
    if drag::carried::<Carried>(ui.ctx()).is_none() {
        return;
    }
    let Some(pointer) = ui.input(|input| input.pointer.hover_pos()) else {
        return;
    };
    let pane = ui.clip_rect();
    if !pane.x_range().contains(pointer.x) {
        return;
    }
    let step = ui.input(|input| input.stable_dt).min(0.1) * EDGE_SPEED;
    let toward = if (pane.top()..pane.top() + EDGE).contains(&pointer.y) {
        step
    } else if (pane.bottom() - EDGE..pane.bottom()).contains(&pointer.y) {
        -step
    } else {
        return;
    };
    ui.scroll_with_delta(Vec2::new(0.0, toward));
    ui.ctx().request_repaint();
}
