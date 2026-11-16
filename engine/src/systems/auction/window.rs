//! The auction house window: Browse to search and buy, Sell to list a stack, and the player's own
//! auctions with their time left.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::VariantArray;
use ui::controls::{self, Pill};
use ui::egui::{self, Align, Layout, Pos2, Rect, Ui, Vec2};
use ui::input::InputField;
use ui::sheet::{self, Tab, TabSize};
use ui::{dialog, face, form, ink};

use super::{
    AuctionAsk, AuctionHouseView, AuctionId, AuctionQuery, AuctionRequest, AuctionRow,
    AuctionSettings,
};
use crate::core::content::Content;
use crate::core::time::{self, UnixMillis};
use crate::core::units::Seconds;
use crate::systems::GameSettings;
use crate::systems::hud::{self, AmountQuestion, HudWidget};
use crate::systems::input::map::{self, InputAction};
use crate::systems::item::card::{self, CardLook};
use crate::systems::item::rarity::ItemRarityId;
use crate::systems::item::widget::{
    InventoryPane, SlotAction, SlotVerdict, inventory_pane, listed_icon, slot_action,
    slot_aside_source, slot_verdict_source,
};
use crate::systems::item::{Inventory, ItemCategory, ItemStack, SlotStack};
use crate::systems::mention::Mention;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::society::SocietyView;
use crate::systems::text::LineText;

pub const AUCTION_HOUSE: &str = "auction_house";

pub(crate) fn plugin(app: &mut App) {
    slot_verdict_source(app, verdict);
    slot_aside_source(app, aside);
    slot_action(
        app,
        SlotAction {
            action: InputAction::Select,
            active: selling,
            act: |world, slot| pick(world, slot, None),
            verb: "sell it",
        },
    );
    slot_action(
        app,
        SlotAction {
            action: InputAction::SplitStack,
            active: selling,
            act: pick_some,
            verb: "sell some of it",
        },
    );
    app.init_resource::<Browse>()
        .add_systems(Update, notice.run_if(in_state(ClientScreen::Area)))
        .add_systems(OnExit(ClientScreen::Area), forget);
}

/// The auction house, while it is open.
pub struct AuctionWindowWidget;

impl HudWidget for AuctionWindowWidget {
    fn fallback(&self, _: &World, screen: Rect) -> Pos2 {
        Pos2::new(screen.center().x - WIDTH / 2.0, screen.top() + 80.0)
    }

    fn hidden(&self, world: &World) -> bool {
        view(world).is_none()
    }

    fn show(&self, ui: &mut Ui, world: &mut World) -> Option<Rect> {
        let shown = view(world)?.clone();
        let settings = world.resource::<GameSettings>().auctions.clone()?;
        let now = time::server_now(world)?;
        dialog::note(ui, WIDTH, |ui| {
            ui.horizontal(|ui| {
                ui.label(face::bold("Auction house"));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if controls::button(ui, "Close").clicked() {
                        world.write_message(AuctionRequest::Close);
                    }
                });
            });
            let mine = my_auctions(world);
            let tabs = [
                Tab::new(AuctionTab::Browse, "Browse"),
                Tab::new(AuctionTab::Sell, "Sell"),
                Tab::new(AuctionTab::Mine, format!("My auctions · {}", mine.len())),
            ];
            let mut tab = world.resource::<Browse>().tab;
            sheet::tabs(ui, TabSize::Sub, &tabs, &mut tab, ui.available_width());
            world.resource_mut::<Browse>().tab = tab;
            ui.add_space(8.0);
            ui.allocate_ui(Vec2::new(WIDTH, PAGE_HEIGHT), |ui| {
                ui.set_min_height(PAGE_HEIGHT);
                match tab {
                    AuctionTab::Browse => browse(ui, world, &shown, &settings, now),
                    AuctionTab::Sell => sell(ui, world, &settings),
                    AuctionTab::Mine => own(ui, world, &mine, &settings, now),
                }
            });
            ui.separator();
            purse(ui, world, &settings);
        });
        None
    }
}

/// Closes the house, if it is open: what stepping out does while it is.
pub fn close(world: &mut World) -> bool {
    if view(world).is_none() {
        return false;
    }
    world.write_message(AuctionRequest::Close);
    true
}

/// The house as its window shows it, for a driver of the client.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownAuctionHouse {
    pub tab: AuctionTab,
    /// What the latest search found, none before the first.
    pub results: Option<Vec<ShownAuction>>,
    pub page: u32,
    pub pages: u32,
    /// The auctions the player has running.
    pub mine: Vec<ShownAuction>,
    /// The stack picked to sell.
    pub selling: Option<ShownAuctionStack>,
    pub waiting: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownAuction {
    pub id: AuctionId,
    pub item: crate::systems::item::ItemId,
    pub count: u32,
    pub price: u32,
    pub left: Seconds,
    pub mine: bool,
    /// Whether someone else bought it before the player's purchase settled.
    pub gone: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq)]
pub struct ShownAuctionStack {
    pub item: crate::systems::item::ItemId,
    pub count: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AuctionTab {
    #[default]
    Browse,
    Sell,
    Mine,
}

pub fn shown(world: &World) -> Option<ShownAuctionHouse> {
    let view = view(world)?;
    let now = time::server_now(world)?;
    let browse = world.resource::<Browse>();
    let row = |row: &AuctionRow| ShownAuction {
        id: row.id,
        item: row.stack.item,
        count: row.stack.count,
        price: row.price,
        left: row.ends.since(now),
        mine: row.mine,
        gone: view.gone.contains(&row.id),
    };
    Some(ShownAuctionHouse {
        tab: browse.tab,
        results: view
            .results
            .as_ref()
            .map(|results| results.rows.iter().map(row).collect()),
        page: view
            .results
            .as_ref()
            .map_or(0, |results| results.query.page),
        pages: view.results.as_ref().map_or(0, |results| results.pages),
        mine: my_auctions(world).iter().map(row).collect(),
        selling: browse.selling.map(|picked| ShownAuctionStack {
            item: picked.stack.item,
            count: picked.stack.count,
        }),
        waiting: browse.waiting_on == Some(view.answered),
    })
}

const WIDTH: f32 = 760.0;
const PAGE_HEIGHT: f32 = 430.0;
const LIST_WIDTH: f32 = 440.0;
const ICON: f32 = 24.0;

/// What the player has set the window to, which only the client holds.
#[derive(Resource, Default)]
struct Browse {
    tab: AuctionTab,
    name: String,
    category: Option<ItemCategory>,
    rarity: Option<ItemRarityId>,
    /// The row of the results picked, by its auction.
    picked: Option<AuctionId>,
    /// The stack picked to sell, still in its slot of the bag.
    selling: Option<SlotStack>,
    price: u32,
    duration: usize,
    /// While the house owes an answer, the count of answers it had given when asked.
    waiting_on: Option<u32>,
    open: bool,
}

fn forget(mut browse: ResMut<Browse>) {
    *browse = Browse::default();
}

/// Starts the window afresh each time the house opens, and lets go of a stack to sell the bag no
/// longer holds as picked.
fn notice(world: &mut World) {
    let open = view(world).is_some();
    if world.resource::<Browse>().open != open {
        *world.resource_mut::<Browse>() = Browse {
            open,
            price: 1,
            ..Browse::default()
        };
    }
    let held = world.resource::<Browse>().selling.and_then(|picked| {
        let bag = bag(world)?;
        let slot = bag.slots.get(picked.slot as usize)?;
        (slot.item == picked.stack.item && slot.count >= picked.stack.count).then_some(picked)
    });
    if world.resource::<Browse>().selling != held {
        world.resource_mut::<Browse>().selling = held;
    }
}

fn view(world: &World) -> Option<&AuctionHouseView> {
    session::my_character(world)?.get::<AuctionHouseView>()
}

fn bag(world: &World) -> Option<&Inventory> {
    session::my_character(world)?.get::<Inventory>()
}

fn my_auctions(world: &World) -> Vec<AuctionRow> {
    session::my_character(world)
        .and_then(|me| me.get::<SocietyView>())
        .map(|view| view.auctions.clone())
        .unwrap_or_default()
}

fn selling(world: &World) -> bool {
    view(world).is_some() && world.resource::<Browse>().tab == AuctionTab::Sell
}

/// Sends `ask` unless the house still owes an answer, so one press is one ask.
fn send(world: &mut World, ask: AuctionAsk) {
    let Some(answered) = view(world).map(|view| view.answered) else {
        return;
    };
    let mut browse = world.resource_mut::<Browse>();
    if browse.waiting_on == Some(answered) {
        return;
    }
    browse.waiting_on = Some(answered);
    world.write_message(AuctionRequest::Ask(ask));
}

fn waiting(world: &World) -> bool {
    view(world).is_some_and(|view| world.resource::<Browse>().waiting_on == Some(view.answered))
}

fn search(world: &mut World, page: u32) {
    let browse = world.resource::<Browse>();
    let query = AuctionQuery {
        name: browse.name.clone(),
        category: browse.category,
        rarity: browse.rarity,
        page,
    };
    world.resource_mut::<Browse>().picked = None;
    send(world, AuctionAsk::Search(query));
}

fn browse(
    ui: &mut Ui,
    world: &mut World,
    shown: &AuctionHouseView,
    settings: &AuctionSettings,
    now: UnixMillis,
) {
    let content = world.resource::<Content>().clone();
    let mut searched = false;
    ui.horizontal(|ui| {
        let mut name = world.resource::<Browse>().name.clone();
        let field = InputField::new(&mut name)
            .hint("Search by name")
            .width(220.0)
            .show(ui);
        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        world.resource_mut::<Browse>().name = name;
        let mut category = world.resource::<Browse>().category;
        egui::ComboBox::from_id_salt("auction category")
            .selected_text(category.map_or("All categories", |category| category.plural()))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut category, None, "All categories");
                for &each in ItemCategory::VARIANTS {
                    ui.selectable_value(&mut category, Some(each), each.plural());
                }
            });
        world.resource_mut::<Browse>().category = category;
        let mut rarity = world.resource::<Browse>().rarity;
        let mut rarities: Vec<ItemRarityId> = content
            .ids::<crate::systems::item::rarity::ItemRarityDef>()
            .collect();
        rarities.sort_by_key(|rarity| rarity.get(&content).rank);
        egui::ComboBox::from_id_salt("auction rarity")
            .selected_text(rarity.map_or("All rarities".to_owned(), |rarity| {
                rarity.get(&content).name.clone()
            }))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut rarity, None, "All rarities");
                for each in rarities {
                    ui.selectable_value(&mut rarity, Some(each), each.get(&content).name.clone());
                }
            });
        world.resource_mut::<Browse>().rarity = rarity;
        let pressed = if waiting(world) {
            controls::waiting(ui, "Search")
        } else {
            controls::primary(ui, "Search")
        };
        searched = pressed.clicked() || entered;
    });
    if searched {
        search(world, 0);
    }
    ui.add_space(8.0);
    let Some(results) = &shown.results else {
        form::nothing_selected(ui, "Search to see what is on sale.");
        return;
    };
    if results.rows.is_empty() {
        let mut used = vec![format!("“{}”", results.query.name.trim())];
        if let Some(category) = results.query.category {
            used.push(category.plural().to_owned());
        }
        if let Some(rarity) = results.query.rarity {
            used.push(rarity.get(&content).name.clone());
        }
        form::nothing_selected(
            ui,
            &format!("Nothing on sale matches {}.", used.join(" · ")),
        );
        return;
    }
    let picked = world.resource::<Browse>().picked;
    let chosen = results
        .rows
        .iter()
        .find(|row| Some(row.id) == picked)
        .copied();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(LIST_WIDTH);
            for row in &results.rows {
                let gone = shown.gone.contains(&row.id);
                let clicked = auction_row(
                    ui,
                    world,
                    &content,
                    settings,
                    row,
                    now,
                    gone,
                    picked == Some(row.id),
                );
                if clicked {
                    world.resource_mut::<Browse>().picked = Some(row.id);
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let page = results.query.page;
                let back = page > 0
                    && (controls::button(ui, "‹ Back").clicked()
                        || map::just_pressed(world, InputAction::PageBack));
                ui.label(face::small(format!(
                    "Page {} of {}",
                    page + 1,
                    results.pages
                )));
                let on = page + 1 < results.pages
                    && (controls::button(ui, "Next ›").clicked()
                        || map::just_pressed(world, InputAction::PageForward));
                if back {
                    search(world, page - 1);
                } else if on {
                    search(world, page + 1);
                }
            });
        });
        ui.add_space(sheet::GUTTER);
        ui.vertical(|ui| match chosen {
            Some(row) => chosen_auction(ui, world, settings, row, shown.gone.contains(&row.id)),
            None => form::nothing_selected(ui, "Pick an auction to see its item."),
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn auction_row(
    ui: &mut Ui,
    world: &mut World,
    content: &Content,
    settings: &AuctionSettings,
    row: &AuctionRow,
    now: UnixMillis,
    gone: bool,
    selected: bool,
) -> bool {
    let look = controls::RowLook {
        selected,
        faint: gone,
        ..controls::RowLook::default()
    };
    let response = controls::row(ui, look, |ui| {
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.with_layout(Layout::top_down(Align::Max), |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let total = ItemStack::new(settings.currency, row.price).describe(content);
                    let total = face::body(total);
                    ui.label(if gone { total.strikethrough() } else { total });
                    if row.stack.count > 1 {
                        let each = f64::from(row.price) / f64::from(row.stack.count);
                        ui.label(face::small(format!("{each:.1} each")).color(ink::INK3));
                    }
                });
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    listed_icon(ui, world, content, row.stack.item, ICON);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        let name = face::body(row.stack.describe(content));
                        ui.label(if gone { name.strikethrough() } else { name });
                        let note = if gone {
                            "Sold".to_owned()
                        } else if row.mine {
                            format!("Yours · {}", left(row.ends, now))
                        } else {
                            left(row.ends, now)
                        };
                        ui.label(face::small(note).color(ink::INK3));
                    });
                });
            });
        });
    })
    .response;
    card::chat_shortcut(world, &response, &[row.stack.item]);
    map::clicked(world, InputAction::Select, &response)
}

fn chosen_auction(
    ui: &mut Ui,
    world: &mut World,
    settings: &AuctionSettings,
    row: AuctionRow,
    gone: bool,
) {
    let content = world.resource::<Content>().clone();
    card::show(ui, world, row.stack.item, CardLook::DOCKED);
    ui.add_space(6.0);
    let held = bag(world).map_or(0, |bag| bag.count(settings.currency));
    let price = ItemStack::new(settings.currency, row.price).describe(&content);
    let shut = if gone {
        Some("Someone bought it first.")
    } else if row.mine {
        Some("This is your own auction.")
    } else if held < row.price {
        Some("You can't afford it.")
    } else {
        None
    };
    let label = format!("Buy for {price}");
    let pressed = match (waiting(world), shut) {
        (true, _) => controls::waiting(ui, label),
        (false, Some(_)) => controls::faded(ui, label),
        (false, None) => controls::primary(ui, label),
    };
    if let Some(why) = shut {
        ui.label(face::small(why).color(ink::RUBRIC));
    }
    if pressed.clicked() && shut.is_none() {
        send(
            world,
            AuctionAsk::Buy {
                auction: row.id,
                stack: row.stack,
                price: row.price,
            },
        );
    }
}

fn sell(ui: &mut Ui, world: &mut World, settings: &AuctionSettings) {
    let content = world.resource::<Content>().clone();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(LIST_WIDTH - 120.0);
            sheet::section(ui, "Inventory");
            let pane = InventoryPane {
                width: ui.available_width(),
                heading: None,
            };
            inventory_pane(ui, world, pane);
        });
        ui.add_space(sheet::GUTTER);
        ui.vertical(|ui| {
            sheet::section(ui, "To sell");
            let Some(picked) = world.resource::<Browse>().selling else {
                form::nothing_selected(ui, "Click a stack in your inventory to sell it.");
                return;
            };
            ui.horizontal(|ui| {
                listed_icon(ui, world, &content, picked.stack.item, ICON);
                ui.label(face::body(picked.stack.describe(&content)));
            });
            ui.add_space(6.0);
            let mut price = world.resource::<Browse>().price;
            ui.horizontal(|ui| {
                ui.label("Price");
                ui.add(egui::DragValue::new(&mut price).range(0..=u32::MAX));
                ui.label(face::body(settings.currency.get(&content).name.clone()));
                if picked.stack.count > 1 {
                    let each = f64::from(price) / f64::from(picked.stack.count);
                    ui.label(face::small(format!("{each:.1} each")).color(ink::INK3));
                }
            });
            world.resource_mut::<Browse>().price = price;
            ui.add_space(4.0);
            let mut duration = world
                .resource::<Browse>()
                .duration
                .min(settings.durations.len() - 1);
            ui.horizontal(|ui| {
                ui.label("For");
                for (index, length) in settings.durations.iter().enumerate() {
                    let hours = length.0 / 3600.0;
                    let label = if hours >= 1.0 {
                        format!("{} h", hours.round())
                    } else {
                        format!("{} min", (length.0 / 60.0).round())
                    };
                    if ui.selectable_label(duration == index, label).clicked() {
                        duration = index;
                    }
                }
            });
            world.resource_mut::<Browse>().duration = duration;
            ui.add_space(6.0);
            let money = |count: u32| ItemStack::new(settings.currency, count).describe(&content);
            let deposit = settings.deposit_on(price);
            form::caption(
                ui,
                format!("Deposit {}, given back only if it sells.", money(deposit)),
                ink::INK2,
            );
            form::caption(
                ui,
                format!(
                    "The house keeps {} if it sells. You get {}.",
                    money(settings.cut_of(price)),
                    money(price - settings.cut_of(price))
                ),
                ink::INK2,
            );
            ui.add_space(6.0);
            let refusal = listing_refusal(world, settings, picked.stack, price);
            let pressed = match (waiting(world), &refusal) {
                (true, _) => controls::waiting(ui, "Create auction"),
                (false, Some(_)) => controls::faded(ui, "Create auction"),
                (false, None) => controls::primary(ui, "Create auction"),
            };
            if let Some(why) = &refusal {
                ui.label(face::small(why.as_str()).color(ink::RUBRIC));
            }
            if pressed.clicked() && refusal.is_none() {
                let ask = AuctionAsk::List {
                    stack: picked.stack,
                    price,
                    duration: settings.durations[duration],
                };
                send(world, ask);
                world.resource_mut::<Browse>().selling = None;
            }
        });
    });
}

/// What stands in the way of listing `stack` at `price`, as the house would refuse it.
fn listing_refusal(
    world: &World,
    settings: &AuctionSettings,
    stack: ItemStack,
    price: u32,
) -> Option<String> {
    let content = world.resource::<Content>();
    let words = &world.resource::<GameSettings>().words;
    if price == 0 {
        return Some(words.auction_no_price.clone());
    }
    if stack.item.get(content).bound() {
        return Some(format!("{} is bound.", stack.item.get(content).name));
    }
    if my_auctions(world).len() as u32 >= settings.listings {
        return Some(
            words
                .auction_too_many
                .replace("{listings}", &settings.listings.to_string()),
        );
    }
    let deposit = settings.deposit_on(price);
    let held = bag(world).map_or(0, |bag| bag.count(settings.currency));
    let selling_currency = if stack.item == settings.currency {
        stack.count
    } else {
        0
    };
    (held < deposit.saturating_add(selling_currency)).then(|| {
        format!(
            "You can't pay the {} deposit.",
            ItemStack::new(settings.currency, deposit).describe(content)
        )
    })
}

fn own(
    ui: &mut Ui,
    world: &mut World,
    mine: &[AuctionRow],
    settings: &AuctionSettings,
    now: UnixMillis,
) {
    let content = world.resource::<Content>().clone();
    if mine.is_empty() {
        form::nothing_selected(ui, "You have no auctions running.");
        return;
    }
    form::caption(
        ui,
        format!(
            "{} of {} auctions. Cancelling gives the stack back, but not the deposit.",
            mine.len(),
            settings.listings
        ),
        ink::INK3,
    );
    ui.add_space(4.0);
    for row in mine {
        ui.horizontal(|ui| {
            ui.set_min_height(controls::CONTROL_HEIGHT);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if controls::button(ui, "Cancel").clicked() {
                    send(world, AuctionAsk::Cancel(row.id));
                }
                let price = ItemStack::new(settings.currency, row.price).describe(&content);
                ui.label(face::body(price));
                controls::pill(ui, &exact_left(row.ends, now), Pill::Ink);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    listed_icon(ui, world, &content, row.stack.item, ICON);
                    ui.label(face::body(row.stack.describe(&content)));
                });
            });
        });
    }
}

fn purse(ui: &mut Ui, world: &mut World, settings: &AuctionSettings) {
    let content = world.resource::<Content>().clone();
    let held = bag(world).map_or(0, |bag| bag.count(settings.currency));
    ui.horizontal(|ui| {
        let icon = settings.currency.get(&content).icon.clone();
        hud::icon(ui, world, &icon, 18.0);
        ui.label(
            face::small(ItemStack::new(settings.currency, held).describe(&content))
                .color(ink::INK2),
        );
    });
}

/// How long until `ends`, roughly: the hours, or the minutes in the last hour.
fn left(ends: UnixMillis, now: UnixMillis) -> String {
    let seconds = ends.since(now).0.max(0.0) as u64;
    match seconds {
        0..60 => "Ending".to_owned(),
        60..3600 => format!("{} min left", seconds / 60),
        _ => format!("{} h left", seconds / 3600),
    }
}

/// How long until `ends`, to the minute.
fn exact_left(ends: UnixMillis, now: UnixMillis) -> String {
    let minutes = (ends.since(now).0.max(0.0) / 60.0).ceil() as u64;
    match minutes {
        0..60 => format!("{minutes} min left"),
        _ => format!("{} h {} min left", minutes / 60, minutes % 60),
    }
}

/// Picks the stack in the bag's slot `slot` to sell: all of it, or `count` of it.
fn pick(world: &mut World, slot: u32, count: Option<u32>) {
    let Some(held) = bag(world).and_then(|bag| bag.slots.get(slot as usize).copied()) else {
        return;
    };
    let content = world.resource::<Content>();
    if held.item.get(content).bound() {
        return;
    }
    let stack = ItemStack::new(held.item, count.unwrap_or(held.count).min(held.count));
    world.resource_mut::<Browse>().selling = Some(SlotStack { slot, stack });
}

fn pick_some(world: &mut World, slot: u32) {
    let Some(held) = bag(world).and_then(|bag| bag.slots.get(slot as usize).copied()) else {
        return;
    };
    if held.count < 2 {
        return pick(world, slot, None);
    }
    let named = Mention::Item(held.item).span(world.resource::<Content>());
    hud::ask_amount(
        world,
        AmountQuestion {
            title: LineText::plain("Sell how many ").then(named),
            amounts: 1..=held.count,
            suggested: held.count / 2,
            says: Box::new(move |count| format!("{count} of {}", held.count)),
            answer: Box::new(move |world, count| {
                if let Some(count) = count {
                    pick(world, slot, Some(count));
                }
            }),
        },
    );
}

/// How many of the stack in the bag's slot `slot` are picked to sell.
fn aside(world: &World, slot: u32, _: ItemStack) -> u32 {
    world
        .resource::<Browse>()
        .selling
        .filter(|picked| picked.slot == slot && view(world).is_some())
        .map_or(0, |picked| picked.stack.count)
}

fn verdict(world: &World, stack: ItemStack) -> Option<SlotVerdict> {
    if !selling(world) {
        return None;
    }
    stack
        .item
        .get(world.resource::<Content>())
        .bound()
        .then(|| SlotVerdict::Refused(LineText::plain("Bound · can't be sold")))
}
