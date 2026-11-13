//! A crafting prompt as its player sees it, in one sheet: the recipes the prompt was opened from,
//! the item prompt's tray with what the placed items make beside it, and the inventory the
//! materials come from. Nothing here is typed out: every recipe, result and verdict is read off
//! the recipe books the prompt names.

use bevy::prelude::*;
use ui::controls::{self, RowLook};
use ui::egui::{self, Align, Layout, Pos2, Rect, Sense, Stroke, Ui, UiBuilder, Vec2};
use ui::face::{self, TextRole};
use ui::glyph::Mark;
use ui::{ink, paint, sheet};

use super::{CraftingRequest, CraftingView, Recipe, RecipeKey, craftables, matches};
use crate::core::content::Content;
use crate::core::text::{self, StyledLine, TextLook, TextPiece};
use crate::core::time;
use crate::core::units::Percent;
use crate::systems::GameSettings;
use crate::systems::dialogue::stage::{self, DockedPanel, PanelChrome, PanelHead};
use crate::systems::hud::{self, Confirm};
use crate::systems::input::map::{self, InputAction, InputMap, input};
use crate::systems::interface::InterfaceSound;
use crate::systems::item::widget::{SLOT, SlotLook, slot};
use crate::systems::item::{self, Inventory, ItemId, ItemStack};
use crate::systems::player::session;
use crate::systems::prompt::tray;
use crate::systems::prompt::{PromptKind, PromptStep, PromptView};
use crate::systems::screen::ClientScreen;
use crate::systems::settings::UserSettings;

const SHEET: Vec2 = Vec2::new(970.0, 400.0);
const RECIPES_WIDTH: f32 = 262.0;
const TRAY_ACROSS: u32 = 4;
const ARROW: f32 = 28.0;
const BUTTON_GAP: f32 = 10.0;
const ICON: f32 = 24.0;
const CROSS: f32 = 10.0;
const BAR: f32 = 6.0;
const CHOICE_MARK: f32 = 12.0;
const MISSING_OPACITY: f32 = 0.45;

pub struct CraftingPanelPlugin;

impl Plugin for CraftingPanelPlugin {
    fn build(&self, app: &mut App) {
        stage::dock(app, DockedPanel { head, show });
        app.init_resource::<Crafting>()
            .add_systems(Update, settle.run_if(in_state(ClientScreen::Area)))
            .add_systems(OnExit(ClientScreen::Area), forget);
    }
}

/// What the player has before them of the crafting prompt beyond its tray.
#[derive(Resource, Default)]
struct Crafting {
    chosen: Option<RecipeKey>,
    crafted: u32,
}

/// What the items placed make: `times` crafts of `recipe`, giving `makes` between them where
/// every one comes off, as each does `chance` of the time.
#[derive(Clone, Debug, PartialEq)]
pub struct CraftResult {
    pub recipe: RecipeKey,
    pub makes: Vec<ItemStack>,
    pub times: u32,
    pub chance: Percent,
}

/// A recipe as the prompt lists it: which materials the bag lacks, and whether it covers them
/// all.
#[derive(Clone, Debug, PartialEq)]
pub struct ListedRecipe {
    pub recipe: Recipe,
    pub lacking: Vec<bool>,
    pub can_make: bool,
}

/// The crafting prompt before the player, if one is open.
pub fn view(world: &World) -> Option<&CraftingView> {
    tray::view(world).filter(|prompt| prompt.kind == PromptKind::Hosted)?;
    world.get::<CraftingView>(session::me(world)?)
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
    let chosen = world.resource::<Crafting>().chosen?;
    found.into_iter().find(|result| result.recipe == chosen)
}

/// Chooses the result of `recipe` among several.
pub fn choose(world: &mut World, recipe: RecipeKey) {
    world.resource_mut::<Crafting>().chosen = Some(recipe);
}

/// How far through the craft under way the prompt is, from none to all of it.
pub fn progress(world: &World) -> Option<f32> {
    let making = view(world)?.making.as_ref()?;
    let whole = making.until.since(making.since).0;
    let done = time::server_now(world)?.since(making.since).0;
    Some(if whole > 0.0 {
        (done / whole).clamp(0.0, 1.0)
    } else {
        1.0
    })
}

pub fn cancel(world: &mut World) -> bool {
    let making = view(world).is_some_and(|view| view.making.is_some());
    if making {
        world.write_message(CraftingRequest::Cancel);
        InterfaceSound::UiClose.play(world);
    }
    making
}

/// Every recipe the prompt makes, as it lists them.
pub fn listed(world: &World) -> Vec<ListedRecipe> {
    let Some(view) = view(world) else {
        return Vec::new();
    };
    let bag = session::my_character(world).and_then(|me| me.get::<Inventory>());
    let held = |item: ItemId| bag.map_or(0, |bag| bag.count(item));
    craftables(world.resource::<Content>(), &view.recipes)
        .into_iter()
        .map(|craftable| {
            let lacking: Vec<bool> = craftable
                .recipe
                .from
                .iter()
                .map(|stack| held(stack.item) < stack.count)
                .collect();
            ListedRecipe {
                recipe: craftable.recipe.clone(),
                can_make: !lacking.contains(&true),
                lacking,
            }
        })
        .collect()
}

/// What the Craft button says, and the result it makes when it can be pressed.
pub fn verdict(world: &World) -> (String, Option<CraftResult>) {
    let content = world.resource::<Content>();
    let words = &content.built::<GameSettings>().words;
    if progress(world).is_some() {
        return (words.cancel_craft.clone(), None);
    }
    let offer = tray::offer(world);
    if offer.is_empty() || tray::waiting(world) {
        return (words.craft.clone(), None);
    }
    if results(world).is_empty() {
        return (words.no_recipe.clone(), None);
    }
    let Some(result) = picked(world) else {
        return (words.choose_result.clone(), None);
    };
    let fits = session::my_character(world)
        .and_then(|me| me.get::<Inventory>())
        .cloned()
        .unwrap_or_else(Inventory::empty)
        .exchange(content, &offer, &result.makes);
    match fits {
        Ok(()) => (words.craft.clone(), Some(result)),
        Err(refusal) => (refusal.describe(content), None),
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
        let bag = session::my_character(world).and_then(|me| me.get::<Inventory>());
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

fn head(world: &World) -> Option<PanelHead> {
    view(world)?;
    let prompt = tray::view(world)?;
    Some(PanelHead {
        width: SHEET.x,
        chrome: PanelChrome::Sheet {
            title: prompt.title.clone(),
            height: SHEET.y,
        },
    })
}

fn show(ui: &mut Ui, world: &mut World, _width: f32, _most: f32) {
    let Some(prompt) = tray::view(world).cloned() else {
        return;
    };
    if view(world).is_none() {
        return;
    }
    let locked = progress(world).is_some();
    egui::Frame::new()
        .inner_margin(sheet::PAGE_MARGIN)
        .show(ui, |ui| {
            let room = ui.available_rect_before_wrap();
            let recipes = Rect::from_min_size(room.min, Vec2::new(RECIPES_WIDTH, room.height()));
            let inventory = Rect::from_min_max(
                Pos2::new(room.max.x - tray::INVENTORY_WIDTH, room.min.y),
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
    let heading = world
        .resource::<Content>()
        .built::<GameSettings>()
        .words
        .recipes
        .clone();
    sheet::section(ui, &heading);
    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        sheet::label(
            ui,
            TextRole::Small,
            "Choosing a recipe places its materials.",
            ink::INK3,
        );
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
                    lay_out(world, &row.recipe);
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
    let name = stacks_named(&content, &row.recipe.makes);
    let (status, status_ink) = if row.can_make {
        ("can make", ink::VERDIGRIS)
    } else {
        ("materials missing", ink::INK3)
    };
    let risk = row
        .recipe
        .may_fail()
        .then(|| chance_note(row.recipe.chance));
    let look = RowLook {
        selected: laid_out,
        ..RowLook::default()
    };
    let response = controls::row(ui, look, |ui| {
        if !row.can_make {
            ui.multiply_opacity(MISSING_OPACITY);
        }
        ui.horizontal(|ui| {
            if let Some(first) = row.recipe.makes.first() {
                hud::icon(ui, world, &first.item.get(&content).icon, ICON);
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                // A line of words is no control: left at a control's height, the two lines of a
                // recipe stand so far apart that a handful of recipes no longer fit the column.
                ui.spacing_mut().interact_size.y = 0.0;
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        sheet::label(ui, TextRole::Small, status, status_ink);
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.add(egui::Label::new(face::body(&name)).truncate());
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
                        let material = format!("{} {}", stack.count, stack.item.get(&content).name);
                        sheet::label(ui, TextRole::Small, &material, ink::INK2);
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
            true,
            format!("Recipe: {name}, {status}"),
        )
    });
    response
}

fn bench_column(ui: &mut Ui, world: &mut World, prompt: &PromptView) {
    tray::heading(ui, world, prompt);
    let found = results(world);
    let slots = tray::slots_width(TRAY_ACROSS);
    let height = tray::slots_height(prompt.slots, slots).max(SLOT);
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
                        |ui| sheet::label(ui, TextRole::Body, "makes either:", ink::INK2),
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
    slot(ui, world, SlotLook::EMPTY, Sense::hover()).widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "Result: nothing")
    });
    let note = if tray::offer(world).is_empty() {
        "nothing yet".to_owned()
    } else {
        world
            .resource::<Content>()
            .built::<GameSettings>()
            .words
            .no_recipe
            .clone()
    };
    beside(ui, None, Some(&note));
}

fn one_result(ui: &mut Ui, world: &mut World, result: &CraftResult) {
    let content = world.resource::<Content>().clone();
    for made in &result.makes {
        let def = made.item.get(&content);
        let look = SlotLook {
            icon: Some(&def.icon),
            count: made.count,
            ..SlotLook::EMPTY
        };
        let response = slot(ui, world, look, Sense::hover());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Label,
                true,
                format!("Result: {}", made.describe(&content)),
            )
        });
        let lines: Vec<String> = std::iter::once(item::card::overview(def))
            .chain(crafts_note(result))
            .collect();
        hud::tooltip(&response, world, &def.name, &lines, None);
    }
    let name = match result.makes.as_slice() {
        [made] => Some(made.item.get(&content).name.as_str()),
        _ => None,
    };
    beside(ui, name, crafts_note(result).as_deref());
}

fn choices(ui: &mut Ui, world: &mut World, found: &[CraftResult]) {
    let content = world.resource::<Content>().clone();
    let chosen = world.resource::<Crafting>().chosen;
    ui.spacing_mut().item_spacing.y = 0.0;
    for (index, result) in found.iter().enumerate() {
        let wanted = chosen == Some(result.recipe);
        let name = stacks_named(&content, &result.makes);
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
                                for made in &result.makes {
                                    hud::icon(ui, world, &made.item.get(&content).icon, ICON);
                                }
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 1.0;
                                    ui.add(egui::Label::new(face::body(&name)).wrap());
                                    if let Some(note) = crafts_note(result) {
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
                format!("Result: {name}"),
            )
        });
        if map::clicked(world, InputAction::Select, &response) {
            choose(world, result.recipe);
            InterfaceSound::UiMove.play(world);
        }
    }
}

fn stacks_named(content: &Content, stacks: &[ItemStack]) -> String {
    stacks
        .iter()
        .map(|stack| {
            let name = &stack.item.get(content).name;
            if stack.count > 1 {
                format!("{name} ×{}", stack.count)
            } else {
                name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// The words beside a result's slots, painted over the rest of their line: its name over a
/// smaller note, each cut short where the line ends, taking a slot's height however many there
/// are.
fn beside(ui: &mut Ui, name: Option<&str>, note: Option<&str>) {
    const BETWEEN: f32 = 1.0;
    let across = ui.available_size_before_wrap().x;
    if across <= 0.0 || (name.is_none() && note.is_none()) {
        return;
    }
    let (rect, response) = ui.allocate_exact_size(Vec2::new(across, SLOT), Sense::hover());
    let lines: Vec<_> = [
        name.map(|name| (TextRole::Body, name, ink::INK)),
        note.map(|note| (TextRole::Small, note, ink::INK3)),
    ]
    .into_iter()
    .flatten()
    .map(|(role, words, color)| {
        let mut job = role.job(words, color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(across);
        (ui.painter().layout_job(job), color)
    })
    .collect();
    let tall: f32 = lines.iter().map(|(line, _)| line.size().y).sum::<f32>()
        + BETWEEN * (lines.len() as f32 - 1.0);
    let mut top = rect.center().y - tall / 2.0;
    for (line, color) in lines {
        let high = line.size().y;
        ui.painter()
            .galley(egui::pos2(rect.left(), top), line, color);
        top += high + BETWEEN;
    }
    let said = [name, note]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &said));
}

/// What is said of a result besides what it is: how many crafts it is, and how often one of them
/// comes off where it may fail.
fn crafts_note(result: &CraftResult) -> Option<String> {
    let crafts = (result.times > 1).then(|| format!("{} crafts", result.times));
    let chance = (result.chance < Percent::WHOLE).then(|| chance_note(result.chance));
    let notes: Vec<String> = crafts.into_iter().chain(chance).collect();
    (!notes.is_empty()).then(|| notes.join(" · "))
}

/// How often a craft comes off, as the prompt says it.
pub fn chance_note(chance: Percent) -> String {
    format!("{}% chance", chance.0)
}

/// Asks the server for the craft the button stands for, in the prompt's turn `step`.
fn craft(world: &mut World, step: PromptStep, recipe: RecipeKey) {
    world.write_message(CraftingRequest::Craft {
        step,
        offer: tray::offer(world),
        recipe,
    });
    tray::sent(world, step);
    InterfaceSound::UiPick.play(world);
}

/// Asks the player whether to go through with a craft that may fail, and crafts it if so.
fn confirm_risk(world: &mut World, step: PromptStep, result: &CraftResult) {
    let content = world.resource::<Content>().clone();
    let made: Vec<&str> = result
        .makes
        .iter()
        .map(|stack| stack.item.get(&content).name.as_str())
        .collect();
    let each = if result.times > 1 {
        format!("Each of the {} crafts", result.times)
    } else {
        "The craft".to_owned()
    };
    let recipe = result.recipe;
    hud::confirm(
        world,
        Confirm {
            title: format!("Craft {}?", made.join(" + ")),
            body: vec![
                format!("{each} has a {}.", chance_note(result.chance)),
                "One that fails spends its materials and makes nothing.".to_owned(),
            ],
            confirm: "Craft anyway".to_owned(),
            cancel: "Keep materials".to_owned(),
            answer: Box::new(move |world, crafts| {
                if crafts {
                    craft(world, step, recipe);
                }
            }),
        },
    );
}

fn foot(ui: &mut Ui, world: &mut World, prompt: &PromptView, several: bool) {
    let words = world
        .resource::<Content>()
        .built::<GameSettings>()
        .words
        .clone();
    let making = progress(world).is_some();
    let room = Vec2::new(ui.available_width(), controls::CONTROL_HEIGHT);
    ui.allocate_ui_with_layout(room, Layout::right_to_left(Align::Center), |ui| {
        if making {
            let key = world.resource::<InputMap>().name(InputAction::Dismiss);
            if controls::keyed(ui, &words.cancel_craft, key.as_deref(), true).clicked() {
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
        let keeps = format!(
            "Nothing is spent until the bar is full. {} keeps everything.",
            words.cancel_craft
        );
        ui.add(egui::Label::new(face::small(keeps).color(ink::INK3)).wrap());
    } else if several {
        let hint = StyledLine::new(vec![
            input(InputAction::Select),
            TextPiece::text(" a result to choose it"),
        ]);
        text::show(ui, world, &hint, look, None);
    } else {
        let hint = StyledLine::new(vec![
            input(InputAction::PlaceItem),
            TextPiece::text(" or "),
            input(InputAction::CarryItem),
            TextPiece::text(" from your inventory to place · click a placed item to take it back"),
        ]);
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
            craft(world, prompt.step, result.recipe);
        }
    }
}

/// Clears the tray once the server has made a craft, lets go of a choice what is placed no
/// longer makes, and forgets the prompt once it is gone.
fn settle(world: &mut World) {
    let Some(crafted) = view(world).map(|view| view.crafted) else {
        let mut crafting = world.resource_mut::<Crafting>();
        if crafting.chosen.is_some() || crafting.crafted != 0 {
            *crafting = Crafting::default();
        }
        return;
    };
    if world.resource::<Crafting>().crafted != crafted {
        world.resource_mut::<Crafting>().crafted = crafted;
        tray::clear(world);
    }
    let made: Vec<RecipeKey> = results(world).iter().map(|result| result.recipe).collect();
    let mut crafting = world.resource_mut::<Crafting>();
    if crafting
        .chosen
        .is_some_and(|chosen| !made.contains(&chosen))
    {
        crafting.chosen = None;
    }
}

fn forget(mut crafting: ResMut<Crafting>) {
    *crafting = Crafting::default();
}
