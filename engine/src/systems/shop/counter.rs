use bevy::prelude::*;
use ui::card::CardPane;
use ui::controls::{self, RowLook};
use ui::drag;
use ui::egui;
use ui::egui::{Align, Layout, Pos2, Rect, Sense, Stroke, Ui, UiBuilder, Vec2};
use ui::face::{self, TextRole};
use ui::sheet::{self, Depth, Tab, TabSize};
use ui::{card, ink};

use super::{OfferView, SaleQuote, SaleVerdict, ShopId, ShopRequest, ShopView, WareRefusal};
use crate::core::content::Content;
use crate::systems::dialogue::stage::{self, DockedPanel, PanelChrome, PanelHead};
use crate::systems::hud::{self, Carried, Confirm};
use crate::systems::input::map::{self, InputAction};
use crate::systems::interface::InterfaceSound;
use crate::systems::item::card::{CardLook, CardNoteTone, ItemCardNote};
use crate::systems::item::widget::{
    InventoryPane, SlotAction, SlotVerdict, inventory_pane, inventory_room, slot_action,
    slot_notes, slot_verdict_source,
};
use crate::systems::item::{self, Inventory, ItemCategory, ItemId, ItemStack};
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::text::duration;

const COUNTER_WIDTH: f32 = 760.0;
const COUNTER_HEIGHT: f32 = 440.0;
const PANE_WIDTH: f32 = 326.0;
const PAGE_GAP: f32 = 10.0;
const NOTE_ROOM: f32 = 22.0;
const PURSE: f32 = 26.0;
const ICON: f32 = 24.0;
const PURSE_ICON: f32 = 18.0;
const COUNT_WIDTH: f32 = 30.0;

pub struct ShopCounterPlugin;

impl Plugin for ShopCounterPlugin {
    fn build(&self, app: &mut App) {
        slot_verdict_source(app, verdict);
        slot_action(
            app,
            SlotAction {
                action: InputAction::SellItem,
                active: selling,
                act: sell_stack,
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
    offered: Option<Offered>,
    open: Option<ShopId>,
    /// Until the shop has answered one more, nothing further is sent.
    answered_when_asked: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Offered {
    slot: u32,
    /// A sale shifts the slots after it, so the slot alone would come to name another item.
    item: ItemId,
    count: u32,
}

struct Shelf {
    shown: ShopView,
    tab: ShopTab,
    ware: usize,
    offered: Option<(Offered, ItemStack)>,
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
    verb: &'static str,
    request: ShopRequest,
}

enum SalePress {
    Count(u32),
    Sell,
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
    let offered = world.resource::<Browse>().offered.and_then(|offered| {
        let held = held_in(world, offered.slot).filter(|held| held.item == offered.item)?;
        Some(Offered {
            count: offered.count.clamp(1, held.count),
            ..offered
        })
    });
    world.resource_mut::<Browse>().offered = offered;
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
    let shown = ui.scope(|ui| counter(ui, world, &shelf)).response;
    if let Some(Carried::BagSlot(slot)) = drag::dropped::<Carried>(&shown) {
        sell_stack(world, slot);
    }
}

fn shelf(world: &World) -> Option<Shelf> {
    let shown = view(world)?.clone();
    let browse = world.resource::<Browse>();
    Some(Shelf {
        waiting: browse.answered_when_asked == Some(shown.answered),
        shown,
        tab: browse.tab,
        ware: browse.ware,
        offered: browse
            .offered
            .and_then(|offered| Some((offered, held_in(world, offered.slot)?))),
    })
}

fn held_in(world: &World, slot: u32) -> Option<ItemStack> {
    session::me(world)
        .and_then(|seen| world.get::<Inventory>(seen))
        .and_then(|inventory| inventory.slots.get(slot as usize).copied())
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
        sheet::tabs(ui, TabSize::Sub, &tabs, &mut tab);
        if tab != shelf.tab {
            let mut browse = world.resource_mut::<Browse>();
            browse.tab = tab;
            browse.ware = 0;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            sheet::label(ui, TextRole::Small, &wants_note(&wants), ink::GOLD);
        });
    });
    ui.add_space(PAGE_GAP);
    let room = ui.available_rect_before_wrap();
    let purse = Rect::from_min_max(Pos2::new(room.min.x, room.max.y - PURSE), room.max);
    let pages = Rect::from_min_max(room.min, Pos2::new(room.max.x, purse.min.y - PAGE_GAP));
    let left = Rect::from_min_max(
        pages.min,
        Pos2::new(pages.max.x - PANE_WIDTH - PAGE_GAP, pages.max.y),
    );
    let pane = CardPane {
        size: Vec2::new(PANE_WIDTH, pages.height()),
        foot: controls::CONTROL_HEIGHT,
    };
    let right = Rect::from_min_max(Pos2::new(pages.max.x - PANE_WIDTH, pages.min.y), pages.max);
    let wares = shelf.wares(&content);
    let ware = shelf.ware.min(wares.len().saturating_sub(1));
    let column = |rect| {
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min))
    };
    ui.scope_builder(column(left), |ui| match shelf.tab {
        ShopTab::Buy | ShopTab::Buyback => wares_column(ui, world, shelf, &wares, ware),
        ShopTab::Sell => inventory_column(ui, world, shelf),
    });
    ui.scope_builder(column(right), |ui| match shelf.tab {
        ShopTab::Buy | ShopTab::Buyback => ware_pane(ui, world, shelf, pane, wares.get(ware)),
        ShopTab::Sell => sale_pane(ui, world, shelf, pane),
    });
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(purse)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| purse_line(ui, world, shelf),
    );
    ui.advance_cursor_after_rect(room);
}

fn wares_column(ui: &mut Ui, world: &mut World, shelf: &Shelf, wares: &[Ware], selected: usize) {
    let kept = shelf.tab == ShopTab::Buyback;
    let rows = ui.available_height() - if kept { NOTE_ROOM } else { 0.0 };
    sheet::scroll(
        ui,
        egui::ScrollArea::vertical()
            .max_height(rows.max(0.0))
            .auto_shrink([false, false]),
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
    let def = ware.item.get(&content);
    let name = counted(&def.name, ware.count);
    let note = ware
        .unavailable()
        .map(|reason| (reason, ink::RUBRIC))
        .or_else(|| ware.stock.clone().map(|stock| (stock, ink::INK3)));
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
                    hud::icon(ui, world, &def.icon, ICON);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.add(egui::Label::new(face::body(&name)).wrap());
                        if let Some((note, color)) = &note {
                            sheet::label(ui, TextRole::Small, note, *color);
                        }
                    });
                });
            });
        });
    })
    .response;
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &name)
    });
    response
}

fn price(ui: &mut Ui, world: &mut World, have: u32, stack: ItemStack) {
    let color = if have >= stack.count {
        ink::INK
    } else {
        ink::RUBRIC
    };
    let icon = stack.item.get(world.resource::<Content>()).icon.clone();
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(face::body(stack.count.to_string()).color(color));
        hud::icon(ui, world, &icon, ICON);
    });
}

fn ware_pane(ui: &mut Ui, world: &mut World, shelf: &Shelf, pane: CardPane, ware: Option<&Ware>) {
    let content = world.resource::<Content>().clone();
    let mut pressed = false;
    pane.show(
        ui,
        |ui| {
            if let Some(ware) = ware {
                let notes = ware.notes(&content);
                item::card::show_noted(ui, world, ware.item, card_look(ui), &notes);
            }
        },
        |ui| {
            if let Some(ware) = ware {
                pressed = trade_button(ui, &content, ware, shelf.waiting).clicked();
            }
        },
    );
    if let (true, Some(ware)) = (pressed, ware)
        && send(world, ware.request)
    {
        InterfaceSound::UiPick.play(world);
    }
}

fn card_look(ui: &Ui) -> CardLook {
    CardLook {
        width: PANE_WIDTH,
        max_height: ui.available_height().min(card::MAX_HEIGHT),
        art: card::ART_SHORT,
        closable: false,
        depth: Depth::Front,
    }
}

fn trade_button(ui: &mut Ui, content: &Content, ware: &Ware, waiting: bool) -> egui::Response {
    let label = match &ware.refusal {
        Some(WareRefusal::Unaffordable(_)) => "Can't afford".to_owned(),
        Some(WareRefusal::NoRoom { .. }) => "Inventory full".to_owned(),
        Some(WareRefusal::SoldOut) => "Sold out".to_owned(),
        Some(WareRefusal::Unavailable(_)) => "Unavailable".to_owned(),
        None => priced(ware.verb, &stacks_text(content, &ware.price, 1)),
    };
    match (&ware.refusal, waiting) {
        (_, true) => controls::waiting(ui, label),
        (Some(_), false) => controls::faded(ui, label),
        (None, false) => controls::primary(ui, label),
    }
}

fn inventory_column(ui: &mut Ui, world: &mut World, shelf: &Shelf) {
    let before = shelf.offered.map(|(offered, _)| offered.slot);
    let mut slot = before;
    let inventory = ui
        .scope(|ui| {
            sheet::scroll(
                ui,
                egui::ScrollArea::vertical().auto_shrink([false, false]),
                |ui| {
                    let pane = InventoryPane {
                        width: ui.available_width(),
                        heading: None,
                        selected: &mut slot,
                    };
                    inventory_pane(ui, world, pane);
                },
            );
        })
        .response;
    // A slot let go of over the inventory it was taken from was put back, not handed to the shop:
    // taking the drop here leaves none for the counter around it.
    drag::dropped::<Carried>(&inventory);
    if slot != before
        && let Some(slot) = slot
        && let Some(held) = held_in(world, slot)
    {
        world.resource_mut::<Browse>().offered = Some(Offered {
            slot,
            item: held.item,
            count: 1,
        });
        InterfaceSound::UiPick.play(world);
    }
}

fn sale_pane(ui: &mut Ui, world: &mut World, shelf: &Shelf, pane: CardPane) {
    let content = world.resource::<Content>().clone();
    let verdict = shelf
        .offered
        .and_then(|(_, held)| quote(&shelf.shown, held.item))
        .map(|quote| &quote.verdict);
    let notes: Vec<ItemCardNote> = shelf
        .offered
        .map(|(_, held)| sale_notes(world, held, verdict))
        .unwrap_or_default();
    let mut pressed = None;
    pane.show(
        ui,
        |ui| match shelf.offered {
            Some((_, held)) => {
                item::card::show_noted(ui, world, held.item, card_look(ui), &notes);
            }
            None => {
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.add_space(card::MARGIN);
                    sheet::label(ui, TextRole::Small, "Select what you would sell", ink::INK3);
                });
            }
        },
        |ui| {
            if let Some((offered, held)) = shelf.offered {
                pressed = sale_actions(ui, &content, offered, held, verdict, shelf.waiting);
            }
        },
    );
    match (pressed, shelf.offered) {
        (Some(SalePress::Count(count)), Some(_)) => {
            if let Some(offered) = world.resource_mut::<Browse>().offered.as_mut() {
                offered.count = count;
            }
        }
        (Some(SalePress::Sell), Some((offered, held))) => {
            request_sale(
                world,
                offered.slot,
                ItemStack::new(held.item, offered.count),
            );
        }
        _ => {}
    }
}

fn sale_notes(world: &World, held: ItemStack, verdict: Option<&SaleVerdict>) -> Vec<ItemCardNote> {
    let content = world.resource::<Content>();
    let said = match verdict {
        Some(SaleVerdict::Pays(pays)) => Some(ItemCardNote {
            text: if held.count > 1 {
                format!("Sells for {} each", stacks_text(content, pays, 1))
            } else {
                format!("Sells for {}", stacks_text(content, pays, 1))
            },
            tone: CardNoteTone::Strong,
        }),
        Some(SaleVerdict::Refused(reason)) => Some(ItemCardNote {
            text: reason.clone(),
            tone: CardNoteTone::Warning,
        }),
        None => None,
    };
    said.into_iter()
        .chain(
            slot_notes(world, held)
                .into_iter()
                .map(|text| ItemCardNote {
                    text,
                    tone: CardNoteTone::Warning,
                }),
        )
        .collect()
}

fn sale_actions(
    ui: &mut Ui,
    content: &Content,
    offered: Offered,
    held: ItemStack,
    verdict: Option<&SaleVerdict>,
    waiting: bool,
) -> Option<SalePress> {
    let Some(SaleVerdict::Pays(pays)) = verdict else {
        return controls::faded(ui, "Not bought here")
            .clicked()
            .then_some(SalePress::Sell);
    };
    let label = priced(
        &format!("Sell {}", offered.count),
        &stacks_text(content, pays, offered.count),
    );
    let sell = if waiting {
        controls::waiting(ui, label)
    } else {
        controls::primary(ui, label)
    };
    let mut pressed = sell.clicked().then_some(SalePress::Sell);
    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let step = |ui: &mut Ui, sign: &str, to: Option<u32>| {
            ui.add_enabled_ui(to.is_some(), |ui| controls::button(ui, sign))
                .inner
                .clicked()
                .then_some(to)
                .flatten()
                .map(SalePress::Count)
        };
        let fewer = step(ui, "−", offered.count.checked_sub(1).filter(|&to| to >= 1));
        let (count, _) = ui.allocate_exact_size(
            Vec2::new(COUNT_WIDTH, ui.available_height()),
            Sense::hover(),
        );
        ui.put(
            count,
            egui::Label::new(face::body(offered.count.to_string())).selectable(false),
        );
        let more = step(
            ui,
            "+",
            Some(offered.count + 1).filter(|&to| to <= held.count),
        );
        ui.add_space(4.0);
        let all = controls::button(ui, format!("All {}", held.count))
            .clicked()
            .then_some(SalePress::Count(held.count));
        pressed = pressed.take().or(fewer).or(more).or(all);
    });
    pressed
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
        let def = currency.get(&content);
        hud::icon(ui, world, &def.icon, PURSE_ICON);
        sheet::label(
            ui,
            TextRole::Small,
            &format!("{held} {}", def.name),
            ink::INK2,
        );
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
                    verb: "Buy back",
                    request: ShopRequest::Buyback { sale: index },
                })
                .collect(),
            ShopTab::Sell => Vec::new(),
        }
    }
}

impl Ware {
    fn unavailable(&self) -> Option<String> {
        match &self.refusal {
            Some(WareRefusal::SoldOut) => Some(match &self.restocks_in {
                Some(restocks) => format!("Sold out · back in {restocks}"),
                None => "Sold out".to_owned(),
            }),
            Some(WareRefusal::Unavailable(reason)) => Some(reason.clone()),
            Some(WareRefusal::Unaffordable(_) | WareRefusal::NoRoom { .. }) | None => None,
        }
    }

    fn notes(&self, content: &Content) -> Vec<ItemCardNote> {
        let text = match &self.refusal {
            Some(WareRefusal::Unaffordable(_)) => format!(
                "Costs {}. You hold {}.",
                stacks_text(content, &self.price, 1),
                self.price
                    .iter()
                    .zip(&self.held)
                    .map(|(stack, held)| ItemStack::new(stack.item, *held).describe(content))
                    .collect::<Vec<_>>()
                    .join(" + ")
            ),
            Some(refusal @ WareRefusal::NoRoom { .. }) => refusal.describe(content),
            Some(WareRefusal::SoldOut | WareRefusal::Unavailable(_)) => match self.unavailable() {
                Some(out) => out,
                None => return Vec::new(),
            },
            None => return Vec::new(),
        };
        vec![ItemCardNote {
            text,
            tone: CardNoteTone::Warning,
        }]
    }
}

fn stock_note(view: &OfferView) -> Option<String> {
    let left = view.left?;
    Some(match view.restocks_in {
        Some(restocks) => format!("{left} left · restocks in {}", duration(restocks)),
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

fn counted(name: &str, count: u32) -> String {
    if count > 1 {
        format!("{name} ×{count}")
    } else {
        name.to_owned()
    }
}

fn priced(verb: &str, price: &str) -> String {
    if price.is_empty() {
        verb.to_owned()
    } else {
        format!("{verb} for {price}")
    }
}

fn verdict(world: &World, stack: ItemStack) -> Option<SlotVerdict> {
    let content = world.resource::<Content>();
    let quote = quote(view(world)?, stack.item)?;
    Some(match &quote.verdict {
        SaleVerdict::Pays(pays) => SlotVerdict::Wanted(format!(
            "sells for {}",
            stacks_text(content, pays, stack.count)
        )),
        SaleVerdict::Refused(refusal) => SlotVerdict::Refused(refusal.clone()),
    })
}

fn quote(shown: &ShopView, item: ItemId) -> Option<&SaleQuote> {
    shown.quotes.iter().find(|quote| quote.item == item)
}

fn stacks_text(content: &Content, stacks: &[ItemStack], times: u32) -> String {
    stacks
        .iter()
        .map(|stack| {
            ItemStack::new(stack.item, stack.count.saturating_mul(times)).describe(content)
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

fn selling(world: &World) -> bool {
    view(world).is_some()
}

fn sell_stack(world: &mut World, slot: u32) {
    if let Some(held) = held_in(world, slot) {
        request_sale(world, slot, held);
    }
}

fn request_sale(world: &mut World, slot: u32, stack: ItemStack) {
    let content = world.resource::<Content>().clone();
    let Some(shown) = view(world) else {
        return;
    };
    let shop = shown.shop;
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
                    stacks_text(&content, pays, stack.count)
                ),
                "You can buy it back for the same until you leave the game.".to_owned(),
            ],
            confirm: "Sell".to_owned(),
            cancel: "Keep".to_owned(),
            answer: Box::new(move |world, sold| {
                if sold {
                    sell(world, slot, stack);
                }
            }),
        },
    );
}

fn sell(world: &mut World, slot: u32, stack: ItemStack) {
    if send(world, ShopRequest::Sell { slot, stack }) {
        InterfaceSound::Coins.play(world);
    }
}

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
