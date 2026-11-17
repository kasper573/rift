use bevy::prelude::*;
use ui::card::CardPane;
use ui::controls::{self, RowLook};
use ui::egui;
use ui::egui::{Align, Layout, Pos2, Rect, Stroke, Ui, UiBuilder, Vec2};
use ui::face::{self, TextRole};
use ui::sheet::{self, Tab, TabSize};
use ui::{card, ink};

use super::{OfferView, SaleQuote, SaleVerdict, ShopId, ShopRequest, ShopView, WareRefusal};
use crate::core::content::Content;
use crate::core::text::TextLook;
use crate::systems::area::TimeOfDayName;
use crate::systems::dialogue::stage::{self, DockedPanel, PanelChrome, PanelHead};
use crate::systems::hud::day_clock;
use crate::systems::input::map::{self, InputAction};
use crate::systems::interface::InterfaceSound;
use crate::systems::item::card::CardLook;
use crate::systems::item::inline::{self, InlineItem, InlineLook};
use crate::systems::item::transfer::{self, ItemTransfer, TransferPane, item_transfer};
use crate::systems::item::tray::ItemTray;
use crate::systems::item::widget::{
    InventoryPane, ItemTile, SlotVerdict, TileClicks, inventory_pane, inventory_room, item_tile,
    slot_verdict_source,
};
use crate::systems::item::{self, Inventory, ItemId, ItemStack, SlotStack};
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::script::condition::Wording;
use crate::systems::text::{LineText, duration};

const COUNTER_WIDTH: f32 = 784.0;
const COUNTER_HEIGHT: f32 = 604.0;
const PAGE_GAP: f32 = 10.0;
const NOTE_ROOM: f32 = 22.0;
const PURSE: f32 = 26.0;
const BESIDE_SELL: f32 = 12.0;

pub struct ShopCounterPlugin;

impl Plugin for ShopCounterPlugin {
    fn build(&self, app: &mut App) {
        slot_verdict_source(app, verdict);
        item_transfer(app, TRANSFER);
        stage::dock(
            app,
            DockedPanel {
                head,
                show: show_counter,
            },
        );
        app.init_resource::<Browse>()
            .add_systems(Update, notice_counter.run_if(in_state(ClientScreen::Area)))
            .add_systems(OnExit(ClientScreen::Area), forget);
    }
}

pub fn view(world: &World) -> Option<&ShopView> {
    session::me(world).and_then(|seen| world.get::<ShopView>(seen))
}

/// What the player has set out to sell at the open shop, and what the shop pays for all of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OfferedSale {
    pub stacks: Vec<ItemStack>,
    pub pays: Vec<ItemStack>,
}

pub fn offered(world: &World) -> Option<OfferedSale> {
    let shown = view(world)?;
    let offered = from_slots(world);
    Some(OfferedSale {
        stacks: offered.iter().map(|offered| offered.stack).collect(),
        pays: payment(shown, &offered).unwrap_or_default(),
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
    offered: ItemTray,
    open: Option<ShopId>,
    /// Until the shop has answered one more, nothing further is sent.
    answered_when_asked: Option<u32>,
}

struct Shelf {
    shown: ShopView,
    tab: ShopTab,
    ware: usize,
    tray: ItemTray,
    /// What is offered, slot by slot of the bag it comes out of.
    offered: Vec<SlotStack>,
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
    /// The times of day it is on offer by.
    times: Vec<TimeOfDayName>,
    verb: &'static str,
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
    let mut offered = world.resource::<Browse>().offered.clone();
    match bag(world) {
        Some(bag) => offered.keep_within(bag),
        None => offered.clear(),
    }
    offered.drop_short();
    if world.resource::<Browse>().offered != offered {
        world.resource_mut::<Browse>().offered = offered;
    }
}

/// What is offered, slot by slot of the bag it comes out of.
fn from_slots(world: &World) -> Vec<SlotStack> {
    bag(world).map_or_else(Vec::new, |bag| {
        world.resource::<Browse>().offered.from_slots(bag)
    })
}

fn head(world: &World) -> Option<PanelHead> {
    let content = world.resource::<Content>();
    view(world).map(|shown| PanelHead {
        width: COUNTER_WIDTH,
        chrome: PanelChrome::Sheet {
            title: shown.shop.get(content).title.to_owned(),
            height: COUNTER_HEIGHT,
        },
    })
}

fn show_counter(ui: &mut Ui, world: &mut World, _width: f32, _most: f32) {
    let Some(shelf) = shelf(world) else {
        return;
    };
    let page = UiBuilder::new().max_rect(sheet::page(ui.max_rect()));
    ui.scope_builder(page, |ui| counter(ui, world, &shelf));
}

fn shelf(world: &World) -> Option<Shelf> {
    let shown = view(world)?.clone();
    let browse = world.resource::<Browse>();
    Some(Shelf {
        waiting: browse.answered_when_asked == Some(shown.answered),
        shown,
        tab: browse.tab,
        ware: browse.ware,
        tray: browse.offered.clone(),
        offered: from_slots(world),
    })
}

fn bag(world: &World) -> Option<&Inventory> {
    session::me(world).and_then(|seen| world.get::<Inventory>(seen))
}

fn counter(ui: &mut Ui, world: &mut World, shelf: &Shelf) {
    let content = world.resource::<Content>().clone();
    let wants = shelf.shown.shop.get(&content).wants(&content);
    ui.horizontal(|ui| {
        let tabs = [
            Tab::new(ShopTab::Buy, "Buy"),
            Tab {
                shut_because: wants.is_empty().then(|| "Buys nothing".to_owned()),
                ..Tab::new(ShopTab::Sell, "Sell")
            },
            Tab::new(
                ShopTab::Buyback,
                format!("Buy back · {}", shelf.shown.buyback.len()),
            ),
        ];
        let mut tab = shelf.tab;
        sheet::tabs(ui, TabSize::Sub, &tabs, &mut tab, ui.available_width());
        if tab != shelf.tab {
            let mut browse = world.resource_mut::<Browse>();
            browse.tab = tab;
            browse.ware = 0;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let look = TextLook::new(TextRole::Small.size(), ink::INK3);
            inline::show_line(ui, world, &wants_note(&wants), look, None);
        });
    });
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
    let wares = shelf.wares(&content);
    let ware = shelf.ware.min(wares.len().saturating_sub(1));
    ui.scope_builder(column(left), |ui| {
        wares_column(ui, world, shelf, &wares, ware);
    });
    ui.scope_builder(column(right), |ui| {
        ware_pane(ui, world, shelf, pane, wares.get(ware));
    });
}

/// The bag on the left and what is offered out of it on the right, over the bar that sells it.
fn sale_pages(ui: &mut Ui, world: &mut World, shelf: &Shelf, pages: Rect) {
    let bar = Rect::from_min_max(
        Pos2::new(pages.min.x, pages.max.y - controls::CONTROL_HEIGHT),
        pages.max,
    );
    let panes = Rect::from_min_max(pages.min, Pos2::new(pages.max.x, bar.min.y - PAGE_GAP));
    let half = (panes.width() - sheet::GUTTER) / 2.0;
    let ([left], right) = sheet::spaced_columns(panes, [half]);
    ui.scope_builder(column(left), |ui| bag_pane(ui, world));
    ui.scope_builder(column(right), |ui| offer_pane(ui, world, shelf));
    ui.scope_builder(UiBuilder::new().max_rect(bar), |ui| {
        sale_bar(ui, world, shelf);
    });
}

fn bag_pane(ui: &mut Ui, world: &mut World) {
    sheet::section(ui, "Inventory");
    let room = ui.available_rect_before_wrap();
    sheet::scroll_pane(
        ui,
        sheet::Pane::bare(room),
        egui::ScrollArea::vertical().id_salt("bag"),
        |ui| {
            let pane = InventoryPane {
                width: ui.available_width(),
                heading: None,
            };
            inventory_pane(ui, world, pane);
        },
    );
}

fn offer_pane(ui: &mut Ui, world: &mut World, shelf: &Shelf) {
    sheet::section(ui, OFFER);
    let room = ui.available_rect_before_wrap();
    sheet::scroll_pane(
        ui,
        sheet::Pane::bare(room),
        egui::ScrollArea::vertical().id_salt("offer"),
        |ui| {
            let pane = TransferPane {
                placed: shelf.tray.placed(),
                slots: inventory_room(world).slots,
                named: OFFER,
                transfer: Some(TRANSFER),
            };
            transfer::transfer_pane(ui, world, pane);
        },
    );
}

/// The button that sells what is offered, in the middle; what the shop pays for it to the
/// button's left, and what stands in the sale's way to its right.
fn sale_bar(ui: &mut Ui, world: &mut World, shelf: &Shelf) {
    let bar = ui.max_rect();
    let pays = payment(&shelf.shown, &shelf.offered);
    let refusal = sale_refusal(world, shelf, pays.as_deref());
    let open = !shelf.offered.is_empty() && pays.is_some() && refusal.is_none();
    let sell = ui
        .scope_builder(
            UiBuilder::new()
                .max_rect(bar)
                .layout(Layout::top_down(Align::Center)),
            |ui| match (shelf.waiting, open) {
                (true, _) => controls::waiting(ui, "Sell"),
                (false, true) => controls::primary(ui, "Sell"),
                (false, false) => controls::faded(ui, "Sell"),
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
            Some(pays) if !shelf.offered.is_empty() => {
                let offer = LineText::plain("Offer: ").then(ItemStack::summed(pays));
                let look = TextLook::new(TextRole::Body.size(), ink::INK);
                inline::show_line(ui, world, &offer, look, None);
            }
            _ => {
                sheet::label(ui, TextRole::Small, "Click what you would sell", ink::INK3);
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
    let stacks = shelf.offered.clone();
    if sell.clicked() && open && send(world, ShopRequest::Sell { stacks }) {
        world.resource_mut::<Browse>().offered.clear();
        InterfaceSound::Coins.play(world);
    }
}

/// What the shop pays for all of `offered`, the stacks of each item it pays in added up: none
/// while it won't buy one of them.
fn payment(shown: &ShopView, offered: &[SlotStack]) -> Option<Vec<ItemStack>> {
    let mut total: Vec<ItemStack> = Vec::new();
    for SlotStack { stack, .. } in offered {
        let SaleVerdict::Pays(pays) = &quote(shown, stack.item)?.verdict else {
            return None;
        };
        for pay in pays {
            let count = pay.count.saturating_mul(stack.count);
            match total.iter_mut().find(|paid| paid.item == pay.item) {
                Some(paid) => paid.count = paid.count.saturating_add(count),
                None => total.push(ItemStack::new(pay.item, count)),
            }
        }
    }
    Some(total)
}

/// Why what is offered can't be sold as it stands, in the shop's own words where it has them.
fn sale_refusal(world: &World, shelf: &Shelf, pays: Option<&[ItemStack]>) -> Option<LineText> {
    let content = world.resource::<Content>();
    let Some(pays) = pays else {
        let shop = shelf.shown.shop.get(content);
        return shelf.offered.iter().find_map(|offered| {
            let item = offered.stack.item;
            match &quote(&shelf.shown, item)?.verdict {
                SaleVerdict::Refused(reason) => Some(reason.line(content, shop, item)),
                SaleVerdict::Pays(_) => None,
            }
        });
    };
    let mut left = bag(world)?.clone();
    left.take_out(&shelf.offered);
    left.exchange(content, &[], pays)
        .err()
        .map(|refusal| refusal.line(content))
}

fn wares_column(ui: &mut Ui, world: &mut World, shelf: &Shelf, wares: &[Ware], selected: usize) {
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
        |ui| wares_list(ui, world, shelf, wares, selected),
    );
    if kept {
        ui.add_space(4.0);
        sheet::label(
            ui,
            TextRole::Small,
            "Kept until you leave the game.",
            ink::INK3,
        );
    }
}

fn wares_list(ui: &mut Ui, world: &mut World, shelf: &Shelf, wares: &[Ware], selected: usize) {
    if wares.is_empty() {
        let note = match shelf.tab {
            ShopTab::Buyback => "Nothing sold here yet",
            ShopTab::Buy | ShopTab::Sell => "Nothing for sale",
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
    let note = ware
        .unavailable()
        .map(|reason| (reason, ink::RUBRIC))
        .or_else(|| ware.stock.clone().map(|stock| (stock.into(), ink::INK3)));
    let look = RowLook {
        selected,
        faint: ware.unavailable().is_some(),
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
                        ui.add(egui::Label::new(name).selectable(false).truncate());
                        if let Some((note, color)) = &note {
                            day_clock::wording(ui, world, note, TextRole::Small, *color);
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
                pressed = trade_button(ui, ware, shelf.waiting).clicked();
                if let Some(why) = ware.wait() {
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

fn trade_button(ui: &mut Ui, ware: &Ware, waiting: bool) -> egui::Response {
    let label = match &ware.refusal {
        Some(WareRefusal::Unaffordable(_)) => "Can't afford".to_owned(),
        Some(WareRefusal::NoRoom { .. }) => "Inventory full".to_owned(),
        Some(WareRefusal::SoldOut) => "Sold out".to_owned(),
        Some(WareRefusal::Unavailable(_)) => "Unavailable".to_owned(),
        None => ware.verb.to_owned(),
    };
    match (&ware.refusal, waiting) {
        (_, true) => controls::waiting(ui, label),
        (Some(_), false) => controls::faded(ui, label),
        (None, false) => controls::primary(ui, label),
    }
}

/// What stands in the way of an action, said beside its button.
fn beside_action(ui: &mut Ui, why: &str) {
    ui.add(egui::Label::new(face::small(why).color(ink::RUBRIC)).truncate());
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
    let inventory = session::me(world).and_then(|seen| world.get::<Inventory>(seen));
    let held: Vec<(ItemId, u32)> = shelf
        .shown
        .shop
        .get(&content)
        .currencies()
        .into_iter()
        .map(|currency| {
            let held = inventory.map_or(0, |inventory| inventory.count(currency));
            (currency, held)
        })
        .collect();
    for (currency, held) in held {
        let held = InlineItem::of(currency).count(held).nameless();
        inline::show(ui, world, held, InlineLook::new(TextRole::Small, ink::INK2));
        sheet::label(ui, TextRole::Small, "·", ink::INK3);
    }
    let room = inventory_room(world);
    let (text, color) = if room.full() {
        (
            format!("Inventory {} of {} · full", room.taken, room.slots),
            ink::RUBRIC,
        )
    } else {
        (
            format!("Inventory {} of {}", room.taken, room.slots),
            ink::INK2,
        )
    };
    sheet::label(ui, TextRole::Small, &text, color);
}

impl Shelf {
    fn wares(&self, content: &Content) -> Vec<Ware> {
        match self.tab {
            ShopTab::Buy => (0..)
                .zip(self.shown.shop.get(content).sells())
                .zip(&self.shown.offers)
                .map(|((index, offer), view)| Ware {
                    item: offer.item,
                    count: offer.count,
                    price: offer.price.to_vec(),
                    held: view.held.clone(),
                    stock: stock_note(view),
                    restocks_in: view.restocks_in.map(duration),
                    refusal: view.refusal.clone(),
                    times: view.times.clone(),
                    verb: "Buy",
                    request: ShopRequest::Buy { offer: index },
                })
                .collect(),
            ShopTab::Buyback => (0..)
                .zip(&self.shown.buyback)
                .map(|(index, buyback)| Ware {
                    item: buyback.sale.item,
                    count: buyback.sale.count,
                    price: buyback.sale.paid.clone(),
                    held: buyback.held.clone(),
                    stock: None,
                    restocks_in: None,
                    refusal: buyback.refusal.clone(),
                    times: Vec::new(),
                    verb: "Buy back",
                    request: ShopRequest::Buyback { sale: index },
                })
                .collect(),
            ShopTab::Sell => Vec::new(),
        }
    }
}

impl Ware {
    fn unavailable(&self) -> Option<Wording> {
        match &self.refusal {
            Some(WareRefusal::SoldOut) => Some(match &self.restocks_in {
                Some(restocks) => format!("Sold out · back in {restocks}").into(),
                None => "Sold out".into(),
            }),
            Some(WareRefusal::Unavailable(reason)) => Some(reason.clone()),
            Some(WareRefusal::Unaffordable(_) | WareRefusal::NoRoom { .. }) | None => None,
        }
    }

    /// What a ware that can't be had right now waits for, where the shop says: the trade
    /// button itself says that it can't.
    fn wait(&self) -> Option<String> {
        match &self.refusal {
            Some(WareRefusal::SoldOut) => self
                .restocks_in
                .as_ref()
                .map(|restocks| format!("Back in {restocks}")),
            Some(WareRefusal::Unavailable(reason)) => Some(reason.to_string()),
            Some(WareRefusal::Unaffordable(_) | WareRefusal::NoRoom { .. }) | None => None,
        }
    }
}

fn stock_note(view: &OfferView) -> Option<String> {
    let left = view.left?;
    Some(match view.restocks_in {
        Some(restocks) => format!("{left} left · restocks in {}", duration(restocks)),
        None => format!("{left} left"),
    })
}

fn wants_note(wants: &[LineText]) -> LineText {
    if wants.is_empty() {
        return "Buys nothing".into();
    }
    let listed = wants
        .iter()
        .enumerate()
        .fold(LineText::default(), |listed, (place, want)| {
            listed
                .then(if place == 0 { "" } else { ", " })
                .then(want.clone())
        });
    LineText::plain("Buys ").then(listed)
}

fn verdict(world: &World, stack: ItemStack) -> Option<SlotVerdict> {
    let content = world.resource::<Content>();
    let quote = quote(view(world)?, stack.item)?;
    Some(match &quote.verdict {
        SaleVerdict::Pays(pays) => {
            let pays: Vec<ItemStack> = pays
                .iter()
                .map(|paid| ItemStack::new(paid.item, paid.count.saturating_mul(stack.count)))
                .collect();
            SlotVerdict::Wanted(LineText::plain("sells for ").then(ItemStack::summed(&pays)))
        }
        SaleVerdict::Refused(refusal) => {
            SlotVerdict::Refused(refusal.line(content, view(world)?.shop.get(content), stack.item))
        }
    })
}

fn quote(shown: &ShopView, item: ItemId) -> Option<&SaleQuote> {
    shown.quotes.iter().find(|quote| quote.item == item)
}

/// What is offered to the shop as a window items move into.
const TRANSFER: ItemTransfer = ItemTransfer {
    key: "shop sale",
    open: offering,
    verb: "offer",
    put: offer,
    take: take_back,
    aside,
};

/// How many of the stack in the bag's slot `slot` are offered to the shop.
fn aside(world: &World, slot: u32, stack: ItemStack) -> u32 {
    bag(world).map_or(0, |bag| {
        world.resource::<Browse>().offered.aside(bag, slot, stack)
    })
}

fn offering(world: &World) -> bool {
    view(world).is_some() && world.resource::<Browse>().tab == ShopTab::Sell
}

/// Offers the shop `count` of the bag's slot `slot`, where it buys such a thing.
fn offer(world: &mut World, slot: u32, count: u32) -> bool {
    let Some(bag) = bag(world).cloned() else {
        return false;
    };
    let bought = bag.slots.get(slot as usize).is_some_and(|held| {
        view(world)
            .and_then(|shown| quote(shown, held.item))
            .is_some_and(|quote| matches!(quote.verdict, SaleVerdict::Pays(_)))
    });
    let slots = bag.max as usize;
    bought
        && world
            .resource_mut::<Browse>()
            .offered
            .place(&bag, slot, count, slots)
}

fn take_back(world: &mut World, index: usize, count: u32) -> bool {
    world
        .resource_mut::<Browse>()
        .offered
        .take_back(index, count)
}

const OFFER: &str = "To sell";

/// Nothing is sent while the shop still owes an answer, so one press is one trade however fast
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
