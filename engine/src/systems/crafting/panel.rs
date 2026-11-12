//! A crafting prompt as its player sees it: the item prompt's tray with what the placed items
//! make beside it, one button that crafts it, a bar and a way out while the craft takes its time,
//! and a window listing the recipes the prompt was opened from. Nothing here is typed out: every
//! recipe, result and verdict is read off the recipe books the prompt names.

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use bevy_egui::input::egui_wants_any_keyboard_input;
use ui::controls::{self, RowLook};
use ui::egui::{self, Align, Layout, Rect, Sense, Ui, Vec2};
use ui::face::{self, TextRole};
use ui::glyph::Mark;
use ui::{ink, paint, sheet};

use super::{CraftingRequest, CraftingView, Recipe, RecipeKey, craftables, matches};
use crate::core::content::Content;
use crate::core::gui::{self, Band};
use crate::core::text::{self, StyledLine, TextLook, TextPiece};
use crate::core::time;
use crate::core::units::Percent;
use crate::systems::GameSettings;
use crate::systems::dialogue::stage::{self, DockedPanel, PanelChrome, PanelHead};
use crate::systems::hud::{self, Confirm, WindowFrame};
use crate::systems::input::map::{self, InputAction, input};
use crate::systems::interface::InterfaceSound;
use crate::systems::item::widget::{SLOT, SlotLook, slot};
use crate::systems::item::{self, Inventory, ItemId, ItemStack};
use crate::systems::player::session;
use crate::systems::prompt::tray;
use crate::systems::prompt::{PromptKind, PromptStep, PromptView};
use crate::systems::screen::ClientScreen;
use crate::systems::settings::UserSettings;

pub const RECIPES: &str = "recipes";
const PANEL_WIDTH: f32 = 580.0;
const PADDING: f32 = 12.0;
const RESULT_WIDTH: f32 = 210.0;
const ARROW: f32 = 28.0;
const BUTTON_GAP: f32 = 10.0;
const LINE: f32 = 22.0;
const RECIPES_SIZE: Vec2 = Vec2::new(310.0, 440.0);
const RECIPES_AT: Vec2 = Vec2::new(12.0, 148.0);
const ICON: f32 = 24.0;
const CROSS: f32 = 10.0;
const BAR: f32 = 6.0;
const MISSING_OPACITY: f32 = 0.45;

pub struct CraftingPanelPlugin;

impl Plugin for CraftingPanelPlugin {
    fn build(&self, app: &mut App) {
        stage::dock(app, DockedPanel { head, show });
        app.init_resource::<Crafting>()
            .add_systems(
                Update,
                (
                    settle,
                    recipes_key.run_if(not(egui_wants_any_keyboard_input)),
                )
                    .chain()
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(OnExit(ClientScreen::Area), forget)
            .add_systems(
                EguiPrimaryContextPass,
                draw_recipes
                    .in_set(Band::Hud)
                    .run_if(in_state(ClientScreen::Area)),
            );
    }
}

/// What the player has before them of the crafting prompt beyond its tray.
#[derive(Resource, Default)]
struct Crafting {
    listing: bool,
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

/// A recipe as the Recipes window lists it: which ingredients the bag lacks, and whether it
/// covers them all.
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

/// Whether the Recipes window is open.
pub fn listing(world: &World) -> bool {
    world.resource::<Crafting>().listing && view(world).is_some()
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

/// Closes the Recipes window, when it is open.
pub fn close_recipes(world: &mut World) -> bool {
    let was = listing(world);
    if was {
        world.resource_mut::<Crafting>().listing = false;
        InterfaceSound::UiClose.play(world);
    }
    was
}

/// Every recipe the prompt makes, as its Recipes window lists them.
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

/// Places the ingredients of `recipe` in the tray in place of what is there; once they are
/// placed, another craft's worth while the bag covers it.
pub fn lay_out(world: &mut World, recipe: &Recipe) {
    if tray::waiting(world) {
        InterfaceSound::UiRefuse.play(world);
        return;
    }
    let covers = |times: u32| {
        let bag = session::my_character(world).and_then(|me| me.get::<Inventory>());
        recipe
            .ingredients(times)
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
    for stack in recipe.ingredients(times) {
        tray::put(world, stack);
    }
    InterfaceSound::UiMove.play(world);
}

fn head(world: &World) -> Option<PanelHead> {
    view(world)?;
    let prompt = tray::view(world)?;
    let chrome = sheet::chrome(true);
    let text = if prompt.text.is_some() {
        LINE + 6.0
    } else {
        0.0
    };
    let guessed = chrome.y
        + 2.0 * PADDING
        + text
        + tray::slots_height(prompt.slots, tray_width(PANEL_WIDTH - chrome.x)).max(SLOT)
        + BUTTON_GAP
        + controls::CONTROL_HEIGHT;
    Some(PanelHead {
        width: PANEL_WIDTH,
        chrome: PanelChrome::Sheet {
            title: prompt.title.clone(),
            height: tray::fitted(world, guessed),
        },
    })
}

fn tray_width(body: f32) -> f32 {
    body - 2.0 * PADDING - ARROW - RESULT_WIDTH
}

fn show(ui: &mut Ui, world: &mut World, _width: f32, _most: f32) {
    let Some(prompt) = tray::view(world).cloned() else {
        return;
    };
    if view(world).is_none() {
        return;
    }
    let drawn = egui::Frame::new()
        .inner_margin(PADDING)
        .show(ui, |ui| {
            tray::words(ui, world, &prompt);
            let across = ui.available_width();
            let slots = across - ARROW - RESULT_WIDTH;
            let height = tray::slots_height(prompt.slots, slots).max(SLOT);
            ui.allocate_ui_with_layout(
                Vec2::new(across, height),
                Layout::left_to_right(Align::Center),
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
                    let (arrow, _) = ui.allocate_exact_size(Vec2::splat(ARROW), Sense::hover());
                    Mark::CaretRight.paint(ui.painter(), arrow.shrink(7.0), ink::INK3);
                    ui.spacing_mut().item_spacing.x = 6.0;
                    result(ui, world);
                },
            );
            ui.add_space(BUTTON_GAP);
            foot(ui, world, &prompt);
        })
        .response
        .rect;
    tray::fit(ui, world, drawn.height());
}

/// What the items placed make, a slot high whatever it holds so the panel stands as tall empty as
/// filled: it grows only once more results than a line holds wrap below.
fn result(ui: &mut Ui, world: &mut World) {
    let content = world.resource::<Content>().clone();
    let found = results(world);
    let several = found.len() > 1;
    let chosen = world.resource::<Crafting>().chosen;
    let line = Layout::left_to_right(Align::Center).with_main_wrap(true);
    let room = Vec2::new(ui.available_width(), SLOT);
    ui.allocate_ui_with_layout(room, line, |ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        if found.is_empty() {
            slot(ui, world, SlotLook::EMPTY, Sense::hover()).widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "Result: nothing")
            });
            let note = if tray::offer(world).is_empty() {
                "nothing yet".to_owned()
            } else {
                content.built::<GameSettings>().words.no_recipe.clone()
            };
            beside(ui, None, Some(&note));
            return;
        }
        for (index, result) in found.iter().enumerate() {
            if index > 0 {
                sheet::label(ui, TextRole::Small, "or", ink::INK3);
            }
            let wanted = several && chosen == Some(result.recipe);
            for made in &result.makes {
                if made_slot(ui, world, &content, result, *made, several, wanted) {
                    choose(world, result.recipe);
                    InterfaceSound::UiMove.play(world);
                }
            }
        }
        if let [only] = found.as_slice() {
            let name = match only.makes.as_slice() {
                [made] => Some(made.item.get(&content).name.as_str()),
                _ => None,
            };
            beside(ui, name, crafts_note(only).as_deref());
        }
    });
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

/// One stack of what `result` makes, as a slot; whether it was clicked to choose its result.
fn made_slot(
    ui: &mut Ui,
    world: &mut World,
    content: &Content,
    result: &CraftResult,
    made: ItemStack,
    several: bool,
    wanted: bool,
) -> bool {
    let def = made.item.get(content);
    let look = SlotLook {
        icon: Some(&def.icon),
        count: made.count,
        wanted,
        ..SlotLook::EMPTY
    };
    let sense = if several {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = slot(ui, world, look, sense);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("Result: {} {}", made.count, def.name),
        )
    });
    let lines: Vec<String> = std::iter::once(item::card::overview(def))
        .chain(crafts_note(result))
        .collect();
    hud::tooltip(&response, world, &def.name, &lines, None);
    several && response.clicked()
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
                "One that fails spends its ingredients and makes nothing.".to_owned(),
            ],
            confirm: "Craft anyway".to_owned(),
            cancel: "Keep ingredients".to_owned(),
            answer: Box::new(move |world, crafts| {
                if crafts {
                    craft(world, step, recipe);
                }
            }),
        },
    );
}

/// The Recipes button, how the tray is filled, and the Craft button at the far end. While a craft
/// is under way its bar stands where the hint was and a way out beside the button, so nothing
/// else moves.
fn foot(ui: &mut Ui, world: &mut World, prompt: &PromptView) {
    let words = world
        .resource::<Content>()
        .built::<GameSettings>()
        .words
        .clone();
    let making = progress(world);
    let room = Vec2::new(ui.available_width(), controls::CONTROL_HEIGHT);
    ui.allocate_ui_with_layout(room, Layout::right_to_left(Align::Center), |ui| {
        let (word, ready) = verdict(world);
        match ready {
            None => {
                controls::faded(ui, word);
            }
            Some(result) => {
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
        }
        if making.is_some() && controls::button(ui, words.cancel_craft).clicked() {
            world.write_message(CraftingRequest::Cancel);
            InterfaceSound::UiClose.play(world);
        }
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            if controls::button(ui, words.recipes).clicked() {
                toggle_recipes(world);
            }
            if let Some(share) = making {
                let bar = Vec2::new(ui.available_width(), BAR);
                paint::progress_line(ui, bar, share, ink::GOLD);
                ui.ctx().request_repaint();
            } else if results(world).len() > 1 {
                let hint = StyledLine::new(vec![
                    input(InputAction::Select),
                    TextPiece::text(" a result to choose it"),
                ]);
                let look = TextLook::new(TextRole::Small.size(), ink::INK3);
                text::show(ui, world, &hint, look, None);
            } else {
                tray::place_hint(ui, world);
            }
        });
    });
}

fn toggle_recipes(world: &mut World) {
    let mut crafting = world.resource_mut::<Crafting>();
    crafting.listing = !crafting.listing;
    let sound = if crafting.listing {
        InterfaceSound::UiOpen
    } else {
        InterfaceSound::UiClose
    };
    sound.play(world);
}

fn recipes_key(world: &mut World) {
    if view(world).is_some() && map::just_pressed(world, InputAction::ToggleRecipes) {
        toggle_recipes(world);
    }
}

fn draw_recipes(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let open = listing(world);
    let Some(pose) = hud::sheet_pose(world, &gui, RECIPES, open) else {
        return;
    };
    let title = world
        .resource::<Content>()
        .built::<GameSettings>()
        .words
        .recipes
        .clone();
    let frame = WindowFrame {
        key: RECIPES,
        title: Some(&title),
        start: Rect::from_min_size(gui.content_rect().min + RECIPES_AT, RECIPES_SIZE),
        body_limit: None,
    };
    let mut still = open;
    hud::window(world, &gui, frame, pose, &mut still, |ui, world| {
        if !open {
            ui.disable();
        }
        list(ui, world);
    });
    if open && !still {
        close_recipes(world);
    }
}

fn list(ui: &mut Ui, world: &mut World) {
    let rows = listed(world);
    sheet::scroll(ui, egui::ScrollArea::vertical().auto_shrink(false), |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (index, row) in rows.iter().enumerate() {
            let response = ui
                .push_id(("recipe", index), |ui| recipe_row(ui, world, row))
                .inner;
            if map::clicked(world, InputAction::Select, &response) {
                lay_out(world, &row.recipe);
            }
        }
        ui.add_space(6.0);
        let hint = StyledLine::new(vec![
            input(InputAction::Select),
            TextPiece::text(" a recipe to place its ingredients"),
        ]);
        let look = TextLook::new(TextRole::Small.size(), ink::INK3);
        text::show(ui, world, &hint, look, None);
    });
}

fn recipe_row(ui: &mut Ui, world: &mut World, row: &ListedRecipe) -> egui::Response {
    let content = world.resource::<Content>().clone();
    let made: Vec<String> = row
        .recipe
        .makes
        .iter()
        .map(|stack| {
            let name = &stack.item.get(&content).name;
            if stack.count > 1 {
                format!("{name} ×{}", stack.count)
            } else {
                name.clone()
            }
        })
        .collect();
    let name = made.join(" + ");
    let (status, status_ink) = if row.can_make {
        ("can make", ink::VERDIGRIS)
    } else {
        ("materials missing", ink::INK3)
    };
    let risk = row
        .recipe
        .may_fail()
        .then(|| chance_note(row.recipe.chance));
    let response = controls::row(ui, RowLook::default(), |ui| {
        if !row.can_make {
            ui.multiply_opacity(MISSING_OPACITY);
        }
        ui.horizontal(|ui| {
            if let Some(first) = row.recipe.makes.first() {
                hud::icon(ui, world, &first.item.get(&content).icon, ICON);
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(face::body(&name)).wrap());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        sheet::label(ui, TextRole::Small, status, status_ink);
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
                        let ingredient =
                            format!("{} {}", stack.count, stack.item.get(&content).name);
                        sheet::label(ui, TextRole::Small, &ingredient, ink::INK2);
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

/// Clears the tray once the server has made a craft, lets go of a choice what is placed no
/// longer makes, and forgets the prompt once it is gone.
fn settle(world: &mut World) {
    let Some(crafted) = view(world).map(|view| view.crafted) else {
        let mut crafting = world.resource_mut::<Crafting>();
        if crafting.listing || crafting.chosen.is_some() || crafting.crafted != 0 {
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
