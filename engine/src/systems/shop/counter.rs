use bevy::prelude::*;
use ui::card::CardPane;
use ui::controls::{self, RowLook};
use ui::egui;
use ui::egui::{Align, Layout, Pos2, Rect, Stroke, Ui, UiBuilder, Vec2};
use ui::face::{self, TextRole};
use ui::sheet::{self, Tab, TabSize};
use ui::{card, ink};

use super::{
    SaleQuote, SaleVerdict, ShopId, ShopRequest, ShopView, WareRefusal, WareView, exchange_refusal,
    held, paid_for,
};
use bevy_egui::EguiPrimaryContextPass;

use crate::core::content::Content;
use crate::core::gui::{self, Band};
use crate::core::text::LineText;
use crate::core::text::TextLook;
use crate::systems::area::TimeOfDayName;
use crate::systems::dialogue::windows::{self, Opens};
use crate::systems::hud::{self, WindowKey, day_clock};
use crate::systems::input::map::{self, InputAction};
use crate::systems::interface::InterfaceSound;
use crate::systems::item::card::CardLook;
use crate::systems::item::grid::scrolled;
use crate::systems::item::inline::{self, InlineItem, InlineLook};
use crate::systems::item::transfer::{self, ItemTransfer, TransferPane, TrayWindow, tray_window};
use crate::systems::item::tray::ItemTray;
use crate::systems::item::widget::{
    InventoryPane, ItemTile, SlotVerdict, TileClicks, inventory_pane, item_tile,
};
use crate::systems::item::{self, Inventory, ItemId, ItemStack, SlotStack};
use crate::systems::messages::{self, Messages};
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::settings::Placed;
use crate::systems::shop::ShopMessages;
use crate::systems::window::AnswerCount;

const COUNTER: Vec2 = Vec2::new(784.0, 604.0);
const COUNTER_LEAST: Vec2 = Vec2::new(700.0, 560.0);
const PAGE_GAP: f32 = 10.0;
const NOTE_ROOM: f32 = 22.0;
const PURSE: f32 = 26.0;
const BESIDE_SELL: f32 = 12.0;

pub struct ShopCounterPlugin;

impl Plugin for ShopCounterPlugin {
    fn build(&self, app: &mut App) {
        tray_window::<SaleTray>(app);
        app.init_resource::<Browse>()
            .add_systems(Update, notice_counter.run_if(in_state(ClientScreen::Area)))
            .add_systems(
                EguiPrimaryContextPass,
                draw_counter
                    .in_set(Band::Hud)
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(OnExit(ClientScreen::Area), forget);
    }
}

pub fn view(world: &World) -> Option<&ShopView> {
    session::me(world).and_then(|seen| world.get::<ShopView>(seen))
}

/// What the player has set out to sell at the open shop, and what the shop pays for all of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlacedSale {
    pub stacks: Vec<ItemStack>,
    pub pays: Vec<ItemStack>,
}

pub fn placed_sale(world: &World) -> Option<PlacedSale> {
    let shown = view(world)?;
    let to_sell = from_slots(world);
    Some(PlacedSale {
        stacks: to_sell.iter().map(|placed| placed.stack).collect(),
        pays: payment(shown, &to_sell).unwrap_or_default(),
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ShopTab {
    #[default]
    Buy,
    Sell,
    Buyback,
}

#[derive(Resource, Default)]
struct Browse {
    tab: ShopTab,
    ware: usize,
    /// What the player would sell, still in the bag: nothing leaves it before the sale.
    to_sell: ItemTray,
    open: Option<ShopId>,
    /// Until the shop has answered one more, nothing further is sent.
    answered_when_asked: Option<AnswerCount>,
}

struct Shelf {
    shown: ShopView,
    tab: ShopTab,
    ware: usize,
    tray: ItemTray,
    /// What is placed to sell, slot by slot of the bag it comes out of.
    to_sell: Vec<SlotStack>,
    waiting: bool,
}

struct Ware {
    item: ItemId,
    count: u32,
    price: Vec<ItemStack>,
    held: Vec<u32>,
    stock: Option<String>,
    restocks_in: Option<String>,
    refusal: Option<WareRefusal>,
    /// The times of day it is for sale by.
    times: Vec<TimeOfDayName>,
    verb: String,
    request: ShopRequest,
}

fn forget(mut browse: ResMut<Browse>) {
    *browse = Browse::default();
}

fn notice_counter(world: &mut World) {
    let shop = view(world).map(|shown| shown.shop);
    if world.resource::<Browse>().open != shop {
        *world.resource_mut::<Browse>() = Browse {
            open: shop,
            ..Browse::default()
        };
        if shop.is_some() {
            InterfaceSound::UiOpen.play(world);
        }
    }
    let mut to_sell = world.resource::<Browse>().to_sell.clone();
    match item::my_bag(world) {
        Some(bag) => to_sell.keep_within(bag),
        None => to_sell.clear(),
    }
    to_sell.drop_short();
    if world.resource::<Browse>().to_sell != to_sell {
        world.resource_mut::<Browse>().to_sell = to_sell;
    }
}

/// What is placed to sell, slot by slot of the bag it comes out of.
fn from_slots(world: &World) -> Vec<SlotStack> {
    item::my_bag(world).map_or_else(Vec::new, |bag| {
        world.resource::<Browse>().to_sell.from_slots(bag)
    })
}

fn draw_counter(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let opens = Opens {
        key: WindowKey::Shop,
        of: Placed::Shop,
        size: COUNTER,
        min: COUNTER_LEAST,
    };
    let title = |shown: &ShopView| shown.shop.get(&content).title.to_owned();
    windows::window(
        world,
        &gui,
        opens,
        view(world).cloned(),
        title,
        |ui, world, _| show_counter(ui, world),
    );
}

fn show_counter(ui: &mut Ui, world: &mut World) {
    let Some(shelf) = shelf(world) else {
        return;
    };
    let page = UiBuilder::new().max_rect(sheet::page(ui.max_rect()));
    ui.scope_builder(page, |ui| counter(ui, world, &shelf));
}

fn shelf(world: &World) -> Option<Shelf> {
    let shown = view(world)?.clone();
    let browse = world.resource::<Browse>();
    let content = world.resource::<Content>();
    let buys = !shown.shop.get(content).wants(content).is_empty();
    Some(Shelf {
        waiting: browse.answered_when_asked == Some(shown.answered),
        shown,
        tab: if buys { browse.tab } else { ShopTab::Buy },
        ware: browse.ware,
        tray: browse.to_sell.clone(),
        to_sell: from_slots(world),
    })
}

fn counter(ui: &mut Ui, world: &mut World, shelf: &Shelf) {
    let content = world.resource::<Content>().clone();
    let wants = shelf.shown.shop.get(&content).wants(&content);
    let messages = messages::of(&content);
    let buys = !wants.is_empty();
    if buys {
        ui.horizontal(|ui| {
            let tabs = [
                Tab::new(ShopTab::Buy, messages.shop.buy()),
                Tab::new(ShopTab::Sell, messages.shop.sell()),
                Tab::new(
                    ShopTab::Buyback,
                    messages.shop.buy_back_tab(shelf.shown.buyback.len()),
                ),
            ];
            let mut tab = shelf.tab;
            let room = ui.available_width();
            hud::tabs(ui, world, TabSize::Sub, &tabs, &mut tab, room, 0.0);
            ui.add_space(sheet::TABS_GAP);
            if tab != shelf.tab {
                let mut browse = world.resource_mut::<Browse>();
                browse.tab = tab;
                browse.ware = 0;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let look = TextLook::new(TextRole::Small.size(), ink::INK3);
                inline::show_line(ui, world, &wants_note(&content, &wants), look, None);
            });
        });
    }
    ui.add_space(PAGE_GAP);
    let room = ui.available_rect_before_wrap();
    let purse = Rect::from_min_max(Pos2::new(room.min.x, room.max.y - PURSE), room.max);
    let pages = Rect::from_min_max(room.min, Pos2::new(room.max.x, purse.min.y - PAGE_GAP));
    match shelf.tab {
        ShopTab::Buy | ShopTab::Buyback => wares_pages(ui, world, shelf, pages),
        ShopTab::Sell => sale_pages(ui, world, shelf, pages),
    }
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(purse)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| purse_line(ui, world, shelf),
    );
    ui.advance_cursor_after_rect(room);
}

fn column(rect: Rect) -> UiBuilder {
    UiBuilder::new()
        .max_rect(rect)
        .layout(Layout::top_down(Align::Min))
}

/// The wares on the left, and the card of the one chosen on the right.
fn wares_pages(ui: &mut Ui, world: &mut World, shelf: &Shelf, pages: Rect) {
    let content = world.resource::<Content>().clone();
    let beside = pages.width() - sheet::GUTTER - card::WIDTH;
    let ([left], right) = sheet::spaced_columns(pages, [beside]);
    let pane = CardPane {
        size: Vec2::new(card::WIDTH, pages.height()),
        foot: controls::CONTROL_HEIGHT,
    };
    let bag = Inventory::of(world, session::me(world));
    let wares = shelf.wares(&content, &bag);
    let ware = shelf.ware.min(wares.len().saturating_sub(1));
    ui.scope_builder(column(left), |ui| {
        wares_column(ui, world, shelf, &wares, ware);
    });
    ui.scope_builder(column(right), |ui| {
        ware_pane(ui, world, shelf, pane, wares.get(ware));
    });
}

/// The bag on the left and what is placed to sell out of it on the right, over the bar that sells it.
fn sale_pages(ui: &mut Ui, world: &mut World, shelf: &Shelf, pages: Rect) {
    let bar = Rect::from_min_max(
        Pos2::new(pages.min.x, pages.max.y - controls::CONTROL_HEIGHT),
        pages.max,
    );
    let panes = Rect::from_min_max(pages.min, Pos2::new(pages.max.x, bar.min.y - PAGE_GAP));
    let half = (panes.width() - sheet::GUTTER) / 2.0;
    let ([left], right) = sheet::spaced_columns(panes, [half]);
    ui.scope_builder(column(left), |ui| bag_pane(ui, world));
    ui.scope_builder(column(right), |ui| sale_pane(ui, world, shelf));
    ui.scope_builder(UiBuilder::new().max_rect(bar), |ui| {
        sale_bar(ui, world, shelf);
    });
}

fn bag_pane(ui: &mut Ui, world: &mut World) {
    let content = world.resource::<Content>().clone();
    sheet::section(ui, messages::of(&content).interface.inventory());
    scrolled(ui, "bag", |ui| {
        let pane = InventoryPane {
            width: ui.available_width(),
            heading: None,
        };
        inventory_pane(ui, world, pane);
    });
}

fn sale_pane(ui: &mut Ui, world: &mut World, shelf: &Shelf) {
    let content = world.resource::<Content>().clone();
    let to_sell = messages::of(&content).shop.to_sell();
    sheet::section(ui, to_sell);
    scrolled(ui, "sale", |ui| {
        let pane = TransferPane {
            placed: shelf.tray.placed(),
            slots: item::my_bag(world).map_or(0, |bag| bag.max),
            named: to_sell,
            transfer: Some(ItemTransfer::of_tray::<SaleTray>()),
        };
        transfer::transfer_pane(ui, world, pane);
    });
}

/// The button that sells what is placed, in the middle; what the shop pays for it to the
/// button's left, and what stands in the sale's way to its right.
fn sale_bar(ui: &mut Ui, world: &mut World, shelf: &Shelf) {
    let content = world.resource::<Content>().clone();
    let words = &messages::of(&content).shop;
    let bar = ui.max_rect();
    let pays = payment(&shelf.shown, &shelf.to_sell);
    let refusal = sale_refusal(world, shelf, pays.as_deref());
    let open = !shelf.to_sell.is_empty() && pays.is_some() && refusal.is_none();
    let sell = ui
        .scope_builder(
            UiBuilder::new()
                .max_rect(bar)
                .layout(Layout::top_down(Align::Center)),
            |ui| match (shelf.waiting, open) {
                (true, _) => controls::waiting(ui, words.sell()),
                (false, true) => controls::primary(ui, words.sell()),
                (false, false) => controls::faded(ui, words.sell()),
            },
        )
        .inner;
    let before = Rect::from_min_max(bar.min, Pos2::new(sell.rect.min.x - BESIDE_SELL, bar.max.y));
    let after = Rect::from_min_max(Pos2::new(sell.rect.max.x + BESIDE_SELL, bar.min.y), bar.max);
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(before)
            .layout(Layout::right_to_left(Align::Center)),
        |ui| match pays.as_deref() {
            Some(pays) if !shelf.to_sell.is_empty() => {
                let total = words.sale_total(ItemStack::summed(pays));
                let look = TextLook::new(TextRole::Body.size(), ink::INK);
                inline::show_line(ui, world, &total, look, None);
            }
            _ => {
                sheet::label(ui, TextRole::Small, words.click_to_sell(), ink::INK3);
            }
        },
    );
    if let Some(refusal) = &refusal {
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(after)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| beside_sale(ui, world, refusal),
        );
    }
    let stacks = shelf.to_sell.clone();
    if sell.clicked() && open && send(world, ShopRequest::Sell { stacks }) {
        world.resource_mut::<Browse>().to_sell.clear();
        InterfaceSound::Coins.play(world);
    }
}

/// What the shop pays for all that is placed, the stacks of each item it pays in added up: none
/// while it won't buy one of them.
fn payment(shown: &ShopView, to_sell: &[SlotStack]) -> Option<Vec<ItemStack>> {
    let mut total: Vec<ItemStack> = Vec::new();
    for SlotStack { stack, .. } in to_sell {
        let SaleVerdict::Pays(pays) = &quote(shown, stack.item)?.verdict else {
            return None;
        };
        total.extend(paid_for(pays, stack.count));
    }
    Some(item::merged(&total))
}

/// Why what is placed can't be sold as it stands, in the shop's own words where it has them.
fn sale_refusal(world: &World, shelf: &Shelf, pays: Option<&[ItemStack]>) -> Option<LineText> {
    let content = world.resource::<Content>();
    let Some(pays) = pays else {
        let shop = shelf.shown.shop.get(content);
        return shelf.to_sell.iter().find_map(|placed| {
            let item = placed.stack.item;
            match &quote(&shelf.shown, item)?.verdict {
                SaleVerdict::Refused(reason) => Some(reason.line(content, shop, item)),
                SaleVerdict::Pays(_) => None,
            }
        });
    };
    let mut left = item::my_bag(world)?.clone();
    left.take_out(&shelf.to_sell);
    left.exchange(content, &[], pays)
        .err()
        .map(|refusal| refusal.line(content))
}

fn wares_column(ui: &mut Ui, world: &mut World, shelf: &Shelf, wares: &[Ware], selected: usize) {
    let content = world.resource::<Content>().clone();
    let words = &messages::of(&content).shop;
    let kept = shelf.tab == ShopTab::Buyback;
    let rows = ui.available_height() - if kept { NOTE_ROOM } else { 0.0 };
    let pane = Rect::from_min_size(
        ui.cursor().min,
        Vec2::new(ui.available_width(), rows.max(0.0)),
    );
    sheet::scroll_pane(
        ui,
        sheet::Pane::bare(pane),
        egui::ScrollArea::vertical(),
        |ui| wares_list(ui, world, shelf, wares, selected, words),
    );
    if kept {
        ui.add_space(4.0);
        sheet::label(ui, TextRole::Small, words.kept_until_leaving(), ink::INK3);
    }
}

fn wares_list(
    ui: &mut Ui,
    world: &mut World,
    shelf: &Shelf,
    wares: &[Ware],
    selected: usize,
    words: &ShopMessages,
) {
    if wares.is_empty() {
        let note = match shelf.tab {
            ShopTab::Buyback => words.nothing_sold_yet(),
            ShopTab::Buy | ShopTab::Sell => words.nothing_for_sale(),
        };
        ui.add_space(16.0);
        sheet::label(ui, TextRole::Small, note, ink::INK3);
        return;
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    for (index, ware) in wares.iter().enumerate() {
        let response = ui
            .push_id(("ware", index), |ui| {
                ware_row(ui, world, ware, index == selected)
            })
            .inner;
        if map::clicked(world, InputAction::Select, &response) {
            world.resource_mut::<Browse>().ware = index;
        }
    }
}

fn ware_row(ui: &mut Ui, world: &mut World, ware: &Ware, selected: bool) -> egui::Response {
    let content = world.resource::<Content>().clone();
    let stack = ItemStack::new(ware.item, ware.count);
    let name = ware.item.get(&content).name.clone();
    let plain = match ware.count {
        1 => name.clone(),
        _ => stack.describe(&content),
    };
    let unavailable = ware.unavailable(&content);
    let note = unavailable
        .clone()
        .map(|reason| (reason, ink::RUBRIC))
        .or_else(|| {
            ware.stock
                .clone()
                .map(|stock| (LineText::from(stock), ink::INK3))
        });
    let look = RowLook {
        selected,
        faint: unavailable.is_some(),
        ..RowLook::default()
    };
    let response = controls::row(ui, look, |ui| {
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.with_layout(Layout::top_down(Align::Max), |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (stack, have) in ware.price.iter().zip(&ware.held) {
                        price(ui, world, *have, *stack);
                    }
                });
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let tile = ItemTile {
                        clicks: TileClicks::Row,
                        ..ItemTile::of(stack)
                    };
                    item_tile(ui, world, tile);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        let name = TextRole::Body.rich(name.as_str()).color(ink::INK);
                        ui.add(
                            egui::Label::new(name)
                                .selectable(false)
                                .truncate()
                                .show_tooltip_when_elided(false),
                        );
                        if let Some((note, color)) = &note {
                            let look = TextLook::new(TextRole::Small.size(), *color);
                            inline::show_plain_line(ui, world, note, look);
                        } else {
                            ui.horizontal(|ui| {
                                for time in &ware.times {
                                    day_clock::time_mark(
                                        ui,
                                        world,
                                        time,
                                        TextRole::Small,
                                        ink::INK3,
                                    );
                                }
                            });
                        }
                    });
                });
            });
        });
    })
    .response;
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &plain)
    });
    item::card::chat_shortcut(world, &response, &[ware.item]);
    response
}

fn price(ui: &mut Ui, world: &mut World, have: u32, stack: ItemStack) {
    let color = if have >= stack.count {
        ink::INK
    } else {
        ink::RUBRIC
    };
    let price = InlineItem::from(stack).nameless();
    inline::show_with_tooltip(ui, world, price, InlineLook::new(TextRole::Body, color));
}

fn ware_pane(ui: &mut Ui, world: &mut World, shelf: &Shelf, pane: CardPane, ware: Option<&Ware>) {
    let content = world.resource::<Content>().clone();
    let words = &messages::of(&content).shop;
    let mut pressed = false;
    pane.show(
        ui,
        |ui| {
            if let Some(ware) = ware {
                item::card::show(ui, world, ware.item, CardLook::DOCKED);
            }
        },
        |ui| {
            if let Some(ware) = ware {
                pressed = ware_button(ui, ware, shelf.waiting, words).clicked();
                if let Some(why) = ware.wait(&content) {
                    beside_action(ui, &why);
                }
            }
        },
    );
    if let (true, Some(ware)) = (pressed, ware)
        && send(world, ware.request.clone())
    {
        InterfaceSound::UiPick.play(world);
    }
}

fn ware_button(ui: &mut Ui, ware: &Ware, waiting: bool, words: &ShopMessages) -> egui::Response {
    let label = match &ware.refusal {
        Some(WareRefusal::Unaffordable(_)) => words.cant_afford().to_owned(),
        Some(WareRefusal::NoRoom { .. }) => words.inventory_full().to_owned(),
        Some(WareRefusal::SoldOut) => words.sold_out().to_owned(),
        Some(WareRefusal::Unavailable(_)) => words.unavailable().to_owned(),
        None => ware.verb.clone(),
    };
    match (&ware.refusal, waiting) {
        (_, true) => controls::waiting(ui, label),
        (Some(_), false) => controls::faded(ui, label),
        (None, false) => controls::primary(ui, label),
    }
}

/// What stands in the way of an action, said beside its button.
fn beside_action(ui: &mut Ui, why: &str) {
    ui.add(
        egui::Label::new(face::small(why).color(ink::RUBRIC))
            .truncate()
            .show_tooltip_when_elided(false),
    );
}

fn beside_sale(ui: &mut Ui, world: &mut World, why: &LineText) {
    let look = TextLook::new(TextRole::Small.size(), ink::RUBRIC);
    inline::show_line(ui, world, why, look, None);
}

fn purse_line(ui: &mut Ui, world: &mut World, shelf: &Shelf) {
    let content = world.resource::<Content>().clone();
    let line = ui.max_rect();
    ui.painter().hline(
        line.x_range(),
        line.top(),
        Stroke::new(1.0, ink::ink_wash(0.18)),
    );
    ui.spacing_mut().item_spacing.x = 6.0;
    let bag = Inventory::of(world, session::me(world));
    let held: Vec<(ItemId, u32)> = shelf
        .shown
        .shop
        .get(&content)
        .currencies()
        .into_iter()
        .map(|currency| (currency, bag.count(currency)))
        .collect();
    for (currency, held) in held {
        let held = InlineItem::of(currency).count(held).nameless();
        inline::show(ui, world, held, InlineLook::new(TextRole::Small, ink::INK2));
        sheet::label(ui, TextRole::Small, "·", ink::INK3);
    }
    let words = &messages::of(&content).shop;
    let (text, color) = if bag.is_full() {
        (words.bag_room_full(bag.max, bag.taken_slots()), ink::RUBRIC)
    } else {
        (words.bag_room(bag.max, bag.taken_slots()), ink::INK2)
    };
    sheet::label(ui, TextRole::Small, &text, color);
}

impl Shelf {
    /// The wares, what the player can afford and has room for judged by the bag they carry now,
    /// which the shop's view of it can lag behind.
    fn wares(&self, content: &Content, bag: &Inventory) -> Vec<Ware> {
        let messages = messages::of(content);
        match self.tab {
            ShopTab::Buy => (0..)
                .zip(self.shown.shop.get(content).sells())
                .zip(&self.shown.wares)
                .map(|((index, ware), view)| Ware {
                    item: ware.item,
                    count: ware.count,
                    price: ware.price.to_vec(),
                    held: held(bag, &ware.price),
                    stock: stock_note(view, messages),
                    restocks_in: view.restocks_in.map(|time| messages.time.rough(time)),
                    refusal: match &view.refusal {
                        Some(refusal @ (WareRefusal::SoldOut | WareRefusal::Unavailable(_))) => {
                            Some(refusal.clone())
                        }
                        _ => exchange_refusal(
                            content,
                            bag,
                            &ware.price,
                            &[ItemStack::new(ware.item, ware.count)],
                        ),
                    },
                    times: view.times.clone(),
                    verb: messages.shop.buy().to_owned(),
                    request: ShopRequest::Buy { ware: index },
                })
                .collect(),
            ShopTab::Buyback => (0..)
                .zip(&self.shown.buyback)
                .map(|(index, buyback)| Ware {
                    item: buyback.sale.item,
                    count: buyback.sale.count,
                    price: buyback.sale.paid.clone(),
                    held: held(bag, &buyback.sale.paid),
                    stock: None,
                    restocks_in: None,
                    refusal: exchange_refusal(
                        content,
                        bag,
                        &buyback.sale.paid,
                        &[ItemStack::new(buyback.sale.item, buyback.sale.count)],
                    ),
                    times: Vec::new(),
                    verb: messages.shop.buy_back().to_owned(),
                    request: ShopRequest::Buyback { sale: index },
                })
                .collect(),
            ShopTab::Sell => Vec::new(),
        }
    }
}

impl Ware {
    fn unavailable(&self, content: &Content) -> Option<LineText> {
        let words = &messages::of(content).shop;
        match &self.refusal {
            Some(WareRefusal::SoldOut) => Some(match &self.restocks_in {
                Some(restocks) => words.sold_out_back_in(restocks).into(),
                None => words.sold_out().into(),
            }),
            Some(WareRefusal::Unavailable(requirement)) => Some(words.needs(requirement.clone())),
            Some(WareRefusal::Unaffordable(_) | WareRefusal::NoRoom { .. }) | None => None,
        }
    }

    /// What a ware that can't be had right now waits for, where the shop says: the buy
    /// button itself says that it can't.
    fn wait(&self, content: &Content) -> Option<String> {
        let words = &messages::of(content).shop;
        match &self.refusal {
            Some(WareRefusal::SoldOut) => self
                .restocks_in
                .as_ref()
                .map(|restocks| words.back_in(restocks)),
            Some(WareRefusal::Unavailable(requirement)) => {
                Some(words.needs(requirement.clone()).plain_words(content))
            }
            Some(WareRefusal::Unaffordable(_) | WareRefusal::NoRoom { .. }) | None => None,
        }
    }
}

fn stock_note(view: &WareView, messages: &Messages) -> Option<String> {
    let left = view.left?;
    Some(match view.restocks_in {
        Some(restocks) => messages
            .shop
            .left_restocks(left, messages.time.rough(restocks)),
        None => messages.shop.left(left),
    })
}

fn wants_note(content: &Content, wants: &[LineText]) -> LineText {
    let messages = messages::of(content);
    messages
        .shop
        .buys(messages.interface.listed(wants.to_vec()))
}

fn sale_verdict(world: &World, stack: ItemStack) -> Option<SlotVerdict> {
    let content = world.resource::<Content>();
    let quote = quote(view(world)?, stack.item)?;
    Some(match &quote.verdict {
        SaleVerdict::Pays(pays) => SlotVerdict::Wanted(
            messages::of(content)
                .shop
                .sells_for(ItemStack::summed(&paid_for(pays, stack.count))),
        ),
        SaleVerdict::Refused(refusal) => {
            SlotVerdict::Refused(refusal.line(content, view(world)?.shop.get(content), stack.item))
        }
    })
}

fn quote(shown: &ShopView, item: ItemId) -> Option<&SaleQuote> {
    shown.quotes.iter().find(|quote| quote.item == item)
}

/// What is placed to sell to the shop, as a window items move into: only what the shop buys.
struct SaleTray;

impl TrayWindow for SaleTray {
    fn open(world: &World) -> bool {
        view(world).is_some() && world.resource::<Browse>().tab == ShopTab::Sell
    }

    fn tray(world: &World) -> &ItemTray {
        &world.resource::<Browse>().to_sell
    }

    fn tray_mut(world: &mut World) -> Option<&mut ItemTray> {
        Some(&mut world.resource_mut::<Browse>().into_inner().to_sell)
    }

    fn slots(world: &World) -> usize {
        item::my_bag(world).map_or(0, |bag| bag.max as usize)
    }

    fn takes(world: &World, held: ItemStack) -> bool {
        view(world)
            .and_then(|shown| quote(shown, held.item))
            .is_some_and(|quote| matches!(quote.verdict, SaleVerdict::Pays(_)))
    }

    fn verdict(world: &World, stack: ItemStack) -> Option<SlotVerdict> {
        sale_verdict(world, stack)
    }
}

/// Nothing is sent while the shop still owes an answer, so one press is one sale however fast
/// the next press follows.
fn send(world: &mut World, request: ShopRequest) -> bool {
    let Some(answered) = view(world).map(|shown| shown.answered) else {
        return false;
    };
    let mut browse = world.resource_mut::<Browse>();
    if browse.answered_when_asked == Some(answered) {
        return false;
    }
    browse.answered_when_asked = Some(answered);
    world.write_message(request);
    true
}
