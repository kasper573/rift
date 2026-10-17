use bevy::prelude::*;
use design_tokens::{CHIP_RADIUS, TextRole, color, widgets};
use egui::{Align, Id, Layout, RichText, Sense, Stroke, Ui, UiBuilder};

use super::{OfferView, SaleQuote, SaleVerdict, ShopId, ShopRequest, ShopView, WareRefusal};
use crate::core::content::Content;
use crate::core::text::{self, StyledLine, TextLook, TextPiece};
use crate::systems::dialogue::stage::{self, DockedPanel, PanelHead};
use crate::systems::hud::{self, Confirm};
use crate::systems::input::map::{self, InputAction, input};
use crate::systems::interface::InterfaceSound;
use crate::systems::item::widget::{
    CarriedSlot, SlotAction, SlotVerdict, slot_action, slot_verdict_source,
};
use crate::systems::item::{Inventory, ItemCategory, ItemId, ItemStack, card};
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;

const COUNTER_WIDTH: f32 = 620.0;
const COUNTER_HEIGHT: f32 = 400.0;
const LIST_WIDTH: f32 = 250.0;
const ICON: f32 = 28.0;
const COST_ICON: f32 = 16.0;
const CHIP_ICON: f32 = 14.0;

pub struct ShopCounterPlugin;

impl Plugin for ShopCounterPlugin {
    fn build(&self, app: &mut App) {
        slot_verdict_source(app, verdict);
        slot_action(
            app,
            SlotAction {
                action: InputAction::SellItem,
                active: selling,
                act: request_sale,
                verb: "sell",
            },
        );
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

pub fn select(world: &mut World, tab: ShopTab, index: usize) {
    let mut browse = world.resource_mut::<Browse>();
    browse.tab = tab;
    browse.selected = index;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ShopTab {
    #[default]
    Wares,
    Buyback,
}

#[derive(Resource, Default)]
struct Browse {
    tab: ShopTab,
    selected: usize,
    open: Option<ShopId>,
}

struct Shelf {
    shown: ShopView,
    tab: ShopTab,
    selected: usize,
}

struct Ware {
    item: ItemId,
    count: u32,
    price: Vec<ItemStack>,
    held: Vec<u32>,
    note: Option<String>,
    refusal: Option<WareRefusal>,
}

fn forget(mut browse: ResMut<Browse>) {
    *browse = Browse::default();
}

fn notice_counter(world: &mut World) {
    let shop = view(world).map(|shown| shown.shop);
    if world.resource::<Browse>().open == shop {
        return;
    }
    *world.resource_mut::<Browse>() = Browse {
        open: shop,
        ..Browse::default()
    };
    if shop.is_some() {
        InterfaceSound::UiOpen.play(world);
    }
}

fn head(world: &World) -> Option<PanelHead> {
    let content = world.resource::<Content>();
    view(world).map(|shown| PanelHead {
        title: shown.shop.get(content).title.to_owned(),
        width: COUNTER_WIDTH,
        fixed_height: Some(COUNTER_HEIGHT),
    })
}

fn show_counter(ui: &mut Ui, world: &mut World) {
    let Some(shelf) = shelf(world) else {
        return;
    };
    let shown = ui.scope(|ui| counter(ui, world, &shelf)).response;
    if let Some(dropped) = shown.dnd_release_payload::<CarriedSlot>() {
        request_sale(world, dropped.0);
    }
}

fn shelf(world: &World) -> Option<Shelf> {
    let shown = view(world)?.clone();
    let browse = world.resource::<Browse>();
    Some(Shelf {
        shown,
        tab: browse.tab,
        selected: browse.selected,
    })
}

fn counter(ui: &mut Ui, world: &mut World, shelf: &Shelf) {
    let content = world.resource::<Content>().clone();
    let wares = shelf.wares(&content);
    let selected = shelf.selected.min(wares.len().saturating_sub(1));
    let wants = shelf.shown.shop.get(&content).wants(&content);
    ui.set_min_size(ui.available_size());
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(10, 0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let tabs = [
                    (ShopTab::Wares, "Wares".to_owned()),
                    (
                        ShopTab::Buyback,
                        format!("Buy back ({})", shelf.shown.buyback.len()),
                    ),
                ];
                let mut tab = shelf.tab;
                widgets::tab_strip(ui, &tabs, &mut tab);
                if tab != shelf.tab {
                    select(world, tab, 0);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(wants_note(&wants))
                            .size(TextRole::Small.size())
                            .color(color::WARN),
                    );
                });
            });
        });
    let selling = !wants.is_empty();
    egui::Panel::bottom(Id::new("shop-footer"))
        .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 6)))
        .show_separator_line(true)
        .show(ui, |ui| sell_hint(ui, world, selling));
    egui::Panel::left(Id::new("shop-list"))
        .resizable(false)
        .exact_size(LIST_WIDTH)
        .frame(egui::Frame::new())
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| list(ui, world, shelf, &wares, selected));
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::new())
        .show(ui, |ui| {
            if let Some(ware) = wares.get(selected) {
                detail(ui, world, shelf, ware, selected);
            }
        });
}

fn list(ui: &mut Ui, world: &mut World, shelf: &Shelf, wares: &[Ware], selected: usize) {
    if wares.is_empty() {
        let note = match shelf.tab {
            ShopTab::Wares => "Nothing for sale",
            ShopTab::Buyback => "Nothing sold here yet",
        };
        ui.add_space(16.0);
        ui.label(
            RichText::new(note)
                .size(TextRole::Small.size())
                .color(color::MUTED),
        );
        return;
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    for (index, ware) in wares.iter().enumerate() {
        let response = ui
            .push_id(("ware", index), |ui| {
                row(ui, world, ware, index == selected)
            })
            .inner;
        if map::clicked(world, InputAction::Select, &response) {
            select(world, shelf.tab, index);
        }
    }
}

fn row(ui: &mut Ui, world: &mut World, ware: &Ware, selected: bool) -> egui::Response {
    let content = world.resource::<Content>().clone();
    let def = ware.item.get(&content);
    let name = counted(&def.name, ware.count);
    let note = unavailable(ware)
        .map(|reason| (reason, color::DANGER))
        .or_else(|| ware.note.clone().map(|note| (note, color::MUTED)));
    let background = ui.painter().add(egui::Shape::Noop);
    let row = ui.scope_builder(
        UiBuilder::new()
            .sense(Sense::click())
            .layout(Layout::top_down(Align::Min)),
        |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.with_layout(Layout::top_down(Align::Max), |ui| {
                                for (stack, have) in ware.price.iter().zip(&ware.held) {
                                    price_chip(ui, world, *have, *stack);
                                }
                            });
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                hud::icon(ui, world, &def.icon, ICON);
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 1.0;
                                    ui.label(
                                        RichText::new(&name)
                                            .family(design_tokens::family::medium()),
                                    );
                                    if let Some((note, ink)) = &note {
                                        ui.label(
                                            RichText::new(note)
                                                .size(TextRole::Small.size())
                                                .color(*ink),
                                        );
                                    }
                                });
                            });
                        });
                    });
                });
        },
    );
    let response = row.response;
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &name)
    });
    let visuals = ui.style().interact_selectable(&response, selected);
    if selected || response.hovered() {
        ui.painter().set(
            background,
            egui::epaint::RectShape::filled(response.rect, 4.0, visuals.weak_bg_fill),
        );
    }
    response
}

fn detail(ui: &mut Ui, world: &mut World, shelf: &Shelf, ware: &Ware, index: usize) {
    let content = world.resource::<Content>().clone();
    egui::Panel::bottom(Id::new("shop-price"))
        .frame(egui::Frame::new().inner_margin(egui::Margin::same(12)))
        .show_separator_line(true)
        .show(ui, |ui| {
            let price = match ware.count {
                1 => "Price".to_owned(),
                count => format!("Price for {count}"),
            };
            ui.label(
                RichText::new(price)
                    .size(TextRole::Small.size())
                    .color(color::MUTED)
                    .family(design_tokens::family::medium()),
            );
            for (stack, &have) in ware.price.iter().zip(&ware.held) {
                let ink = if have >= stack.count {
                    color::TEXT
                } else {
                    color::DANGER
                };
                let item = stack.item.get(&content);
                ui.horizontal(|ui| {
                    hud::icon(ui, world, &item.icon, COST_ICON);
                    ui.label(
                        RichText::new(format!("{} {} · You have {have}", stack.count, item.name))
                            .color(ink),
                    );
                });
            }
            for (note, ink) in ware
                .note
                .iter()
                .map(|note| (note.clone(), color::MUTED))
                .chain(unavailable(ware).map(|reason| (reason, color::DANGER)))
            {
                ui.label(RichText::new(note).size(TextRole::Small.size()).color(ink));
            }
            ui.add_space(4.0);
            if trade_button(ui, shelf.tab, ware).clicked() {
                trade(world, shelf.tab, index);
            }
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::new().inner_margin(egui::Margin::same(12)))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| card::sheet(ui, world, ware.item));
        });
}

fn trade_button(ui: &mut Ui, tab: ShopTab, ware: &Ware) -> egui::Response {
    match (&ware.refusal, tab) {
        (Some(WareRefusal::Unaffordable(_)), _) => widgets::danger(ui, "Can't afford"),
        (Some(WareRefusal::Unavailable(_)), _) => ui.add(egui::Button::new(
            RichText::new("Unavailable").color(color::MUTED),
        )),
        (None, ShopTab::Wares) => widgets::primary(ui, "Buy"),
        (None, ShopTab::Buyback) => widgets::primary(ui, "Buy back"),
    }
}

fn trade(world: &mut World, tab: ShopTab, index: usize) {
    let index = index as u32;
    match tab {
        ShopTab::Wares => {
            world.write_message(ShopRequest::Buy { offer: index });
        }
        ShopTab::Buyback => {
            world.write_message(ShopRequest::Buyback { sale: index });
        }
    }
    InterfaceSound::UiPick.play(world);
}

fn price_chip(ui: &mut Ui, world: &mut World, have: u32, stack: ItemStack) {
    let covered = have >= stack.count;
    let ink = if covered { color::WARN } else { color::DANGER };
    let icon = stack.item.get(world.resource::<Content>()).icon.clone();
    egui::Frame::new()
        .fill(ink.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0, ink.gamma_multiply(0.6)))
        .corner_radius(CHIP_RADIUS)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                hud::icon(ui, world, &icon, CHIP_ICON);
                ui.label(RichText::new(stack.count.to_string()).size(12.5).color(ink));
            });
        });
}

fn sell_hint(ui: &mut Ui, world: &mut World, selling: bool) {
    if !selling {
        return;
    }
    let hint = StyledLine::new(vec![
        input(InputAction::SellItem),
        TextPiece::text(" or "),
        input(InputAction::CarryItem),
        TextPiece::text(" from your bag to sell"),
    ]);
    let look = TextLook {
        size: TextRole::Small.size(),
        color: color::MUTED,
    };
    text::show(ui, world, &hint, look, None);
}

impl Shelf {
    fn wares(&self, content: &Content) -> Vec<Ware> {
        match self.tab {
            ShopTab::Wares => self
                .shown
                .shop
                .get(content)
                .sells()
                .zip(&self.shown.offers)
                .map(|(offer, view)| Ware {
                    item: offer.item,
                    count: offer.count,
                    price: offer.price.to_vec(),
                    held: view.held.clone(),
                    note: stock_note(view),
                    refusal: view.refusal.clone(),
                })
                .collect(),
            ShopTab::Buyback => self
                .shown
                .buyback
                .iter()
                .map(|buyback| Ware {
                    item: buyback.sale.item,
                    count: buyback.sale.count,
                    price: buyback.sale.paid.clone(),
                    held: buyback.held.clone(),
                    note: None,
                    refusal: buyback.refusal.clone(),
                })
                .collect(),
        }
    }
}

fn stock_note(view: &OfferView) -> Option<String> {
    let left = view.left?;
    Some(match view.restocks_in {
        Some(restocks) => format!(
            "{left} left · restocks in {} min",
            (restocks.0 / 60.0).ceil() as u32
        ),
        None => format!("{left} left"),
    })
}

fn wants_note(wants: &[String]) -> String {
    if wants.is_empty() {
        "Buys nothing".to_owned()
    } else {
        format!("Buys {}", wants.join(", "))
    }
}

fn unavailable(ware: &Ware) -> Option<String> {
    match &ware.refusal {
        Some(WareRefusal::Unavailable(reason)) => Some(reason.clone()),
        Some(WareRefusal::Unaffordable(_)) | None => None,
    }
}

fn counted(name: &str, count: u32) -> String {
    if count > 1 {
        format!("{name} ×{count}")
    } else {
        name.to_owned()
    }
}

fn verdict(world: &World, stack: ItemStack) -> Option<SlotVerdict> {
    let content = world.resource::<Content>();
    let quote = quote(view(world)?, stack.item)?;
    Some(match &quote.verdict {
        SaleVerdict::Pays(pays) => {
            SlotVerdict::Wanted(format!("sells for {}", paid(content, pays, stack.count)))
        }
        SaleVerdict::Refused(refusal) => SlotVerdict::Refused(refusal.clone()),
    })
}

fn quote(shown: &ShopView, item: ItemId) -> Option<&SaleQuote> {
    shown.quotes.iter().find(|quote| quote.item == item)
}

fn paid(content: &Content, pays: &[ItemStack], count: u32) -> String {
    pays.iter()
        .map(|pay| ItemStack::new(pay.item, pay.count.saturating_mul(count)).describe(content))
        .collect::<Vec<_>>()
        .join(" + ")
}

fn selling(world: &World) -> bool {
    view(world).is_some()
}

fn request_sale(world: &mut World, slot: u32) {
    let content = world.resource::<Content>().clone();
    let Some(shown) = view(world) else {
        return;
    };
    let shop = shown.shop;
    let Some(stack) = session::me(world)
        .and_then(|seen| world.get::<Inventory>(seen))
        .and_then(|inventory| inventory.slots.get(slot as usize).copied())
    else {
        return;
    };
    let Some(verdict) = quote(shown, stack.item).map(|quote| quote.verdict.clone()) else {
        return;
    };
    match verdict {
        SaleVerdict::Pays(pays)
            if stack.item.get(&content).category() == ItemCategory::Equipment =>
        {
            confirm_sale(world, shop, stack, &pays, slot);
        }
        SaleVerdict::Pays(_) | SaleVerdict::Refused(_) => sell(world, slot, stack),
    }
}

fn confirm_sale(world: &mut World, shop: ShopId, stack: ItemStack, pays: &[ItemStack], slot: u32) {
    let content = world.resource::<Content>().clone();
    let name = &stack.item.get(&content).name;
    hud::confirm(
        world,
        Confirm {
            title: format!("Sell {name}?"),
            body: vec![
                format!(
                    "{} pays {}.",
                    shop.get(&content).title,
                    paid(&content, pays, stack.count)
                ),
                "You can buy it back for the same until you leave the game.".to_owned(),
            ],
            confirm: "Sell".to_owned(),
            cancel: "Keep".to_owned(),
            on_confirm: Box::new(move |world| sell(world, slot, stack)),
        },
    );
}

fn sell(world: &mut World, slot: u32, stack: ItemStack) {
    world.write_message(ShopRequest::Sell { slot, stack });
    InterfaceSound::Coins.play(world);
}
