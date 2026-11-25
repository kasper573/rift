//! A crafting prompt as its player sees it, in one sheet: the recipes the prompt was opened from,
//! the item prompt's tray with what the placed items make beside it, and the inventory the
//! materials come from. Nothing here is typed out: every recipe, result and verdict is read off
//! the recipe books the prompt names.

use super::{ListedRecipe, lacks_mastery, listed, progress, view};
use crate::systems::prompt;
use bevy::prelude::*;
use ui::controls::{self, RowHover, RowLook};
use ui::egui::{self, Align, Layout, Pos2, Rect, Sense, Stroke, Ui, UiBuilder, Vec2};
use ui::face::{self, TextRole};
use ui::glyph::Mark;
use ui::{ink, paint, sheet};

use super::{CraftingMessages, CraftingRequest, Recipe, RecipeKey, craftables, matches};
use bevy_egui::EguiPrimaryContextPass;

use crate::core::content::Content;
use crate::core::gui::{self, Band};
use crate::core::text::LineText;
use crate::core::text::{self, TextLook};
use crate::core::units::Percent;
use crate::systems::dialogue::windows::{self, Opens};
use crate::systems::hud::{self, Confirm, WindowKey};
use crate::systems::input::escape::{self, Dismissal};
use crate::systems::input::map::{self, InputAction, InputMap};
use crate::systems::interface::InterfaceSound;
use crate::systems::item::grid::{grid_height, item_grid, slots_span};
use crate::systems::item::inline;
use crate::systems::item::widget::{ItemSlotLook, ItemTile, SLOT, TileClicks, item_tile, slot};
use crate::systems::item::{self, Inventory, ItemId, ItemStack};
use crate::systems::messages;
use crate::systems::player::session;
use crate::systems::prompt::PromptView;
use crate::systems::prompt::tray;
use crate::systems::screen::ClientScreen;
use crate::systems::settings::{Placed, UserSettings};
use crate::systems::window::{AnswerCount, WindowStep};

const SHEET: Vec2 = Vec2::new(970.0, 400.0);
const SHEET_LEAST: Vec2 = Vec2::new(860.0, 340.0);
const RECIPES_WIDTH: f32 = 262.0;
const TRAY_ACROSS: u32 = 4;
const ARROW: f32 = 28.0;
const BUTTON_GAP: f32 = 10.0;
const CROSS: f32 = 10.0;
const BAR: f32 = 6.0;
const CHOICE_MARK: f32 = 12.0;

pub struct CraftingPanelPlugin;

impl Plugin for CraftingPanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Crafting>()
            .add_systems(Update, settle.run_if(in_state(ClientScreen::Area)))
            .add_systems(
                EguiPrimaryContextPass,
                draw.in_set(Band::Hud).run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(OnExit(ClientScreen::Area), forget);
    }
}

/// What the player has before them of the crafting prompt beyond its tray.
#[derive(Resource, Default)]
struct Crafting {
    chosen: Option<RecipeKey>,
    answered: AnswerCount,
}

/// What the items placed make: `times` crafts of `recipe`, giving `makes` between them where
/// every one comes off, as each does `chance` of the time. `locked` where it requires a mastery
/// the player lacks.
#[derive(Clone, Debug, PartialEq)]
pub struct CraftResult {
    pub recipe: RecipeKey,
    pub makes: Vec<ItemStack>,
    pub times: u32,
    pub chance: Percent,
    pub locked: bool,
}

/// What the items placed make, by every recipe of the prompt they are a whole number of times;
/// one result for each, and one for recipes that make the same.
pub fn results(world: &World) -> Vec<CraftResult> {
    let Some(view) = view(world) else {
        return Vec::new();
    };
    let known = craftables(world.resource::<Content>(), &view.recipes);
    let mut found: Vec<CraftResult> = Vec::new();
    for (craftable, times) in matches(&known, &tray::offer(world)) {
        let makes = craftable.recipe.products(times);
        if found.iter().all(|result| result.makes != makes) {
            found.push(CraftResult {
                locked: lacks_mastery(world, &craftable),
                recipe: craftable.key,
                makes,
                times,
                chance: craftable.recipe.chance,
            });
        }
    }
    found
}

/// The result Craft would make: the only one, or of several the one chosen.
pub fn picked(world: &World) -> Option<CraftResult> {
    let mut found = results(world);
    if found.len() == 1 {
        return found.pop();
    }
    let chosen = world.resource::<Crafting>().chosen.as_ref()?;
    found.into_iter().find(|result| result.recipe == *chosen)
}

/// Chooses the result of `recipe` among several.
pub fn choose(world: &mut World, recipe: RecipeKey) {
    world.resource_mut::<Crafting>().chosen = Some(recipe);
}

pub fn making(world: &World) -> bool {
    view(world).is_some_and(|view| view.making.is_some())
}

pub fn cancel(world: &mut World) -> bool {
    let making = making(world);
    if making {
        world.write_message(CraftingRequest::Cancel);
        InterfaceSound::UiClose.play(world);
    }
    making
}

/// What the Craft button says, and the result it makes when it can be pressed.
pub fn verdict(world: &World) -> (String, Option<CraftResult>) {
    let content = world.resource::<Content>();
    let messages = &messages::of(content).crafting;
    if progress(world).is_some() {
        return (messages.cancel().to_owned(), None);
    }
    let placed = tray::offer(world);
    if placed.is_empty() || tray::waiting(world) {
        return (messages.craft().to_owned(), None);
    }
    if results(world).is_empty() {
        return (messages.no_recipe().to_owned(), None);
    }
    let Some(result) = picked(world) else {
        return (messages.choose_result().to_owned(), None);
    };
    if result.locked {
        return (messages.requires_mastery().to_owned(), None);
    }
    let fits = Inventory::of(world, session::me(world)).exchange(content, &placed, &result.makes);
    match fits {
        Ok(()) => (messages.craft().to_owned(), Some(result)),
        Err(refusal) => (refusal.line(content).plain_words(content), None),
    }
}

/// Places the materials of `recipe` in the tray in place of what is there; once they are
/// placed, another craft's worth while the bag covers it.
pub fn lay_out(world: &mut World, recipe: &Recipe) {
    if tray::waiting(world) {
        InterfaceSound::UiRefuse.play(world);
        return;
    }
    let covers = |times: u32| {
        let bag = item::my_bag(world);
        recipe
            .materials(times)
            .iter()
            .all(|stack| bag.is_some_and(|bag| bag.count(stack.item) >= stack.count))
    };
    let times = recipe
        .times(&tray::offer(world))
        .map_or(1, |placed| placed + 1);
    if !covers(times) {
        InterfaceSound::UiRefuse.play(world);
        return;
    }
    tray::clear(world);
    for stack in recipe.materials(times) {
        tray::put(world, stack);
    }
    InterfaceSound::UiMove.play(world);
}

fn draw(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let opens = Opens {
        key: WindowKey::Crafting,
        of: Placed::Crafting,
        size: tray::making_room(&gui, SHEET),
        min: tray::making_room(&gui, SHEET_LEAST),
    };
    let open = view(world).and(prompt::view(world)).cloned();
    windows::window(
        world,
        &gui,
        opens,
        open,
        |prompt| prompt.title.clone(),
        |ui, world, _| show(ui, world),
    );
}

fn show(ui: &mut Ui, world: &mut World) {
    let Some(prompt) = prompt::view(world).cloned() else {
        return;
    };
    if view(world).is_none() {
        return;
    }
    let locked = progress(world).is_some();
    egui::Frame::new()
        .inner_margin(sheet::PAGE_MARGIN)
        .show(ui, |ui| {
            tray::words(ui, world, &prompt);
            let room = ui.available_rect_before_wrap();
            let recipes = Rect::from_min_size(room.min, Vec2::new(RECIPES_WIDTH, room.height()));
            let inventory = Rect::from_min_max(
                Pos2::new(room.max.x - tray::inventory_width(ui.ctx()), room.min.y),
                room.max,
            );
            let bench = Rect::from_min_max(
                Pos2::new(recipes.max.x + sheet::GUTTER, room.min.y),
                Pos2::new(inventory.min.x - sheet::GUTTER, room.max.y),
            );
            let column = |rect| {
                UiBuilder::new()
                    .max_rect(rect)
                    .layout(Layout::top_down(Align::Min))
            };
            ui.scope_builder(column(recipes), |ui| {
                tray::lockable(ui, locked, |ui| recipes_column(ui, world));
            });
            ui.scope_builder(column(bench), |ui| bench_column(ui, world, &prompt));
            ui.scope_builder(column(inventory), |ui| tray::inventory(ui, world, locked));
            ui.advance_cursor_after_rect(room);
        });
}

fn recipes_column(ui: &mut Ui, world: &mut World) {
    let messages = messages::of(world.resource::<Content>()).crafting.clone();
    sheet::section(ui, messages.recipes());
    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        sheet::label(ui, TextRole::Small, messages.choosing_places(), ink::INK3);
        ui.add_space(6.0);
        ui.with_layout(Layout::top_down(Align::Min), |ui| list(ui, world));
    });
}

fn list(ui: &mut Ui, world: &mut World) {
    let rows = listed(world);
    let placed = tray::offer(world);
    let room = ui.available_rect_before_wrap();
    sheet::scroll_pane(
        ui,
        sheet::Pane::bare(room),
        egui::ScrollArea::vertical(),
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, row) in rows.iter().enumerate() {
                let laid_out = row.recipe.times(&placed).is_some();
                let response = ui
                    .push_id(("recipe", index), |ui| recipe_row(ui, world, row, laid_out))
                    .inner;
                if map::clicked(world, InputAction::Select, &response) {
                    if row.locked {
                        InterfaceSound::UiRefuse.play(world);
                    } else {
                        lay_out(world, &row.recipe);
                    }
                }
            }
        },
    );
}

fn recipe_row(
    ui: &mut Ui,
    world: &mut World,
    row: &ListedRecipe,
    laid_out: bool,
) -> egui::Response {
    let content = world.resource::<Content>().clone();
    let named = ItemStack::summed(&row.recipe.makes);
    let name = named.plain_words(&content);
    let messages = &messages::of(&content).crafting;
    let (state, status, status_ink) = if row.locked {
        ("locked", messages.requires_mastery(), ink::RUBRIC)
    } else if row.can_make {
        ("can_make", messages.can_make(), ink::VERDIGRIS)
    } else {
        ("missing", messages.materials_missing(), ink::RUBRIC)
    };
    let risk = row
        .recipe
        .may_fail()
        .then(|| messages.chance(row.recipe.chance.0));
    let look = RowLook {
        selected: laid_out,
        hover: if row.locked {
            RowHover::Unlit
        } else {
            RowHover::Lit
        },
        ..RowLook::default()
    };
    let response = controls::row(ui, look, |ui| {
        ui.horizontal(|ui| {
            made_tiles(ui, world, &row.recipe.makes);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                // A line of words is no control: left at a control's height, the two lines of a
                // recipe stand so far apart that a handful of recipes no longer fit the column.
                ui.spacing_mut().interact_size.y = 0.0;
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        sheet::label(ui, TextRole::Small, status, status_ink);
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            let names = made_names(&content, &row.recipe.makes);
                            let names = TextRole::Body.rich(names.as_str()).color(ink::INK);
                            ui.add(
                                egui::Label::new(names)
                                    .selectable(false)
                                    .truncate()
                                    .show_tooltip_when_elided(false),
                            );
                        });
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if let Some(risk) = &risk {
                        sheet::label(ui, TextRole::Small, risk, ink::RUBRIC);
                        ui.add_space(4.0);
                    }
                    for (stack, lacking) in row.recipe.from.iter().zip(&row.lacking) {
                        if *lacking {
                            let (cross, _) =
                                ui.allocate_exact_size(Vec2::splat(CROSS), Sense::hover());
                            Mark::Cross.paint(ui.painter(), cross, ink::RUBRIC);
                        }
                        let look = TextLook::new(TextRole::Small.size(), ink::INK2);
                        inline::show_plain_line(ui, world, &stack.named(), look);
                        ui.add_space(4.0);
                    }
                });
            });
        });
    })
    .response;
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            !row.locked,
            messages.recipe_named(&name, state),
        )
    });
    item::card::chat_shortcut(world, &response, &made_items(&row.recipe.makes));
    response
}

fn bench_column(ui: &mut Ui, world: &mut World, prompt: &PromptView) {
    tray::heading(ui, world);
    let found = results(world);
    let content = world.resource::<Content>().clone();
    let messages = &messages::of(&content).crafting;
    let slots = slots_span(TRAY_ACROSS);
    let height = grid_height(prompt.slots, slots);
    let across = ui.available_width();
    ui.allocate_ui_with_layout(
        Vec2::new(across, height),
        Layout::left_to_right(Align::Min),
        |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.allocate_ui_with_layout(
                Vec2::new(slots, height),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.set_width(slots);
                    tray::slots(ui, world, prompt.slots);
                },
            );
            let (arrow, _) = ui.allocate_exact_size(Vec2::new(ARROW, SLOT), Sense::hover());
            let caret = Rect::from_center_size(arrow.center(), Vec2::splat(ARROW - 14.0));
            Mark::CaretRight.paint(ui.painter(), caret, ink::INK3);
            ui.spacing_mut().item_spacing.x = 6.0;
            match found.as_slice() {
                [] => no_result(ui, world),
                [only] => one_result(ui, world, only),
                _ => {
                    ui.allocate_ui_with_layout(
                        Vec2::new(ui.available_width(), SLOT),
                        Layout::left_to_right(Align::Center),
                        |ui| sheet::label(ui, TextRole::Body, messages.makes_either(), ink::INK2),
                    );
                }
            }
        },
    );
    // The bar's room is kept whether or not a craft runs, so nothing under it moves as one starts.
    let bar = Vec2::new(across, BAR);
    ui.add_space(BAR);
    match progress(world) {
        Some(share) => {
            paint::progress_line(ui, bar, share, ink::GOLD);
            ui.ctx().request_repaint();
        }
        None => {
            ui.allocate_exact_size(bar, Sense::hover());
        }
    }
    ui.add_space(BAR);
    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        foot(ui, world, prompt, found.len() > 1);
        ui.add_space(BUTTON_GAP);
        if found.len() > 1 {
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                choices(ui, world, &found);
            });
        }
    });
}

fn no_result(ui: &mut Ui, world: &mut World) {
    let content = world.resource::<Content>().clone();
    let messages = &messages::of(&content).crafting;
    slot(ui, world, ItemSlotLook::EMPTY, Sense::hover()).widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, messages.result_nothing())
    });
    let note = if tray::offer(world).is_empty() {
        messages.nothing_yet()
    } else {
        messages.no_recipe()
    };
    beside(ui, messages, None, Some(note));
}

fn one_result(ui: &mut Ui, world: &mut World, result: &CraftResult) {
    let content = world.resource::<Content>().clone();
    let messages = &messages::of(&content).crafting;
    for &made in &result.makes {
        let tile = ItemTile {
            notes: crafts_note(messages, result)
                .map(LineText::from)
                .into_iter()
                .collect(),
            ..ItemTile::of(made)
        };
        item_tile(ui, world, tile).widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Label,
                true,
                messages.result_named(made.describe(&content)),
            )
        });
    }
    let name = match result.makes.as_slice() {
        [made] => Some(made.item.get(&content).name.as_str()),
        _ => None,
    };
    beside(ui, messages, name, crafts_note(messages, result).as_deref());
}

fn choices(ui: &mut Ui, world: &mut World, found: &[CraftResult]) {
    let content = world.resource::<Content>().clone();
    let messages = &messages::of(&content).crafting;
    let chosen = world.resource::<Crafting>().chosen.clone();
    ui.spacing_mut().item_spacing.y = 0.0;
    for (index, result) in found.iter().enumerate() {
        let wanted = chosen.as_ref() == Some(&result.recipe);
        let named = ItemStack::summed(&result.makes);
        let name = named.plain_words(&content);
        let look = RowLook {
            selected: wanted,
            ..RowLook::default()
        };
        let response = ui
            .push_id(("result", index), |ui| {
                controls::row(ui, look, |ui| {
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let (mark, _) =
                                ui.allocate_exact_size(Vec2::splat(CHOICE_MARK), Sense::hover());
                            let radius = CHOICE_MARK / 2.0 - 1.0;
                            ui.painter().circle_stroke(
                                mark.center(),
                                radius,
                                Stroke::new(1.5, ink::INK2),
                            );
                            if wanted {
                                ui.painter()
                                    .circle_filled(mark.center(), radius - 2.5, ink::INK);
                            }
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                made_tiles(ui, world, &result.makes);
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 1.0;
                                    let names = made_names(&content, &result.makes);
                                    sheet::label(ui, TextRole::Body, &names, ink::INK);
                                    if let Some(note) = crafts_note(messages, result) {
                                        sheet::label(ui, TextRole::Small, &note, ink::INK3);
                                    }
                                });
                            });
                        });
                    });
                })
                .response
            })
            .inner;
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::RadioButton,
                true,
                wanted,
                messages.result_named(&name),
            )
        });
        if map::clicked(world, InputAction::Select, &response) {
            choose(world, result.recipe.clone());
            InterfaceSound::UiMove.play(world);
        }
        item::card::chat_shortcut(world, &response, &made_items(&result.makes));
    }
}

/// What a recipe makes, leading its row: a tile for each item, the row taking their clicks.
fn made_tiles(ui: &mut Ui, world: &mut World, makes: &[ItemStack]) {
    item_grid(ui, makes.len() as u32, |ui, place| {
        let tile = ItemTile {
            clicks: TileClicks::Row,
            ..ItemTile::of(makes[place as usize])
        };
        item_tile(ui, world, tile);
    });
}

/// The names of what a recipe makes, beside their tiles, which show how many of each.
fn made_names(content: &Content, makes: &[ItemStack]) -> String {
    makes
        .iter()
        .map(|made| made.item.get(content).name.as_str())
        .collect::<Vec<_>>()
        .join(" + ")
}

fn made_items(makes: &[ItemStack]) -> Vec<ItemId> {
    makes.iter().map(|made| made.item).collect()
}

/// The words beside a result's tiles, painted over the rest of their line: the name of what they
/// make over a smaller note, each cut short where the line ends, taking a tile's height however
/// many there are.
fn beside(ui: &mut Ui, messages: &CraftingMessages, name: Option<&str>, note: Option<&str>) {
    const BETWEEN: f32 = 1.0;
    let across = ui.available_size_before_wrap().x;
    if across <= 0.0 || (name.is_none() && note.is_none()) {
        return;
    }
    let (rect, response) = ui.allocate_exact_size(Vec2::new(across, SLOT), Sense::hover());
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = BETWEEN;
                if let Some(name) = name {
                    let name = TextRole::Body.rich(name).color(ink::INK);
                    ui.add(
                        egui::Label::new(name)
                            .selectable(false)
                            .truncate()
                            .show_tooltip_when_elided(false),
                    );
                }
                if let Some(note) = note {
                    ui.add(
                        egui::Label::new(face::small(note).color(ink::INK3))
                            .truncate()
                            .show_tooltip_when_elided(false),
                    );
                }
            });
        },
    );
    let said = match (name, note) {
        (Some(name), Some(note)) => messages.result_described(name, note),
        (Some(only), None) | (None, Some(only)) => only.to_owned(),
        (None, None) => return,
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &said));
}

/// What is said of a result besides what it is: how many crafts it is, and how often one of them
/// comes off where it may fail.
fn crafts_note(messages: &CraftingMessages, result: &CraftResult) -> Option<String> {
    let crafts = (result.times > 1).then(|| messages.crafts(result.times));
    let chance = (result.chance < Percent::WHOLE).then(|| messages.chance(result.chance.0));
    let notes: Vec<String> = crafts.into_iter().chain(chance).collect();
    (!notes.is_empty()).then(|| notes.join(" · "))
}

/// Asks the server for the craft the button stands for, in the prompt's turn `step`.
fn craft(world: &mut World, step: WindowStep, recipe: RecipeKey) {
    world.write_message(CraftingRequest::Craft {
        step,
        placed: tray::offer(world),
        recipe,
    });
    tray::sent(world, step);
    InterfaceSound::UiPick.play(world);
}

/// Asks the player whether to go through with a craft that may fail, and crafts it if so.
fn confirm_risk(world: &mut World, step: WindowStep, result: &CraftResult) {
    let made = ItemStack::summed(&result.makes);
    let messages = messages::of(world.resource::<Content>()).crafting.clone();
    let risk = messages.risk(result.times, result.chance.0);
    let recipe = result.recipe.clone();
    hud::confirm(
        world,
        Confirm {
            title: messages.confirm_risk(made),
            body: vec![risk.into(), messages.failing_spends().into()],
            confirm: messages.craft_anyway().to_owned(),
            cancel: messages.keep_materials().to_owned(),
            answer: Box::new(move |world, crafts| {
                if crafts {
                    craft(world, step, recipe);
                }
            }),
        },
    );
}

fn foot(ui: &mut Ui, world: &mut World, prompt: &PromptView, several: bool) {
    let messages = messages::of(world.resource::<Content>()).crafting.clone();
    let making = progress(world).is_some();
    let room = Vec2::new(ui.available_width(), controls::CONTROL_HEIGHT);
    ui.allocate_ui_with_layout(room, Layout::right_to_left(Align::Center), |ui| {
        if making {
            let key = (escape::dismisses(world) == Some(Dismissal::Crafting))
                .then(|| world.resource::<InputMap>().cap(InputAction::Dismiss))
                .flatten();
            if controls::keyed(ui, messages.cancel(), key.as_ref(), true).clicked() {
                cancel(world);
            }
            return;
        }
        craft_button(ui, world, prompt);
        tray::leave_button(ui, world);
    });
    ui.add_space(BUTTON_GAP);
    let look = TextLook::new(TextRole::Small.size(), ink::INK3);
    if making {
        let keeps = messages.nothing_spent(messages.cancel());
        ui.add(egui::Label::new(face::small(keeps).color(ink::INK3)).wrap());
    } else if several {
        let content = world.resource::<Content>().clone();
        let hint = messages.choose_hint(InputAction::Select).styled(&content);
        text::show(ui, world, &hint, look, None);
    } else {
        let hint = tray::how_to(world.resource::<Content>());
        text::show(ui, world, &hint, look, None);
    }
}

fn craft_button(ui: &mut Ui, world: &mut World, prompt: &PromptView) {
    let (word, ready) = verdict(world);
    if tray::waiting(world) {
        controls::waiting(ui, word);
        return;
    }
    let Some(result) = ready else {
        controls::faded(ui, word);
        return;
    };
    if controls::primary(ui, word).clicked() {
        let asks = result.chance < Percent::WHOLE
            && world.resource::<UserSettings>().confirms_risky_crafts();
        if asks {
            confirm_risk(world, prompt.step, &result);
        } else {
            craft(world, prompt.step, result.recipe.clone());
        }
    }
}

/// Clears the tray once the server has made a craft, lets go of a choice what is placed no
/// longer makes, and forgets the prompt once it is gone.
fn settle(world: &mut World) {
    let Some(answered) = view(world).map(|view| view.answered) else {
        let mut crafting = world.resource_mut::<Crafting>();
        if crafting.chosen.is_some() || crafting.answered != AnswerCount::default() {
            *crafting = Crafting::default();
        }
        return;
    };
    if world.resource::<Crafting>().answered != answered {
        world.resource_mut::<Crafting>().answered = answered;
        tray::clear(world);
    }
    let made: Vec<RecipeKey> = results(world)
        .into_iter()
        .map(|result| result.recipe)
        .collect();
    let mut crafting = world.resource_mut::<Crafting>();
    if crafting
        .chosen
        .as_ref()
        .is_some_and(|chosen| !made.contains(chosen))
    {
        crafting.chosen = None;
    }
}

fn forget(mut crafting: ResMut<Crafting>) {
    *crafting = Crafting::default();
}
