//! The market window: Browse to search and buy, New listing to put items up for other items, and
//! the player's own listings with their time left.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::controls;
use ui::egui::{self, Align, Color32, Layout, Pos2, Rect, Response, Ui, Vec2};
use ui::face::TextRole;
use ui::input::InputField;
use ui::sheet::{self, Tab, TabSize};
use ui::{face, form, ink, panel};

use super::{
    ListingId, ListingRow, ListingSort, MarketAsk, MarketQuery, MarketRequest, MarketResults,
    MarketSettings, MarketView, SortBy,
};
use crate::core::content::Content;
use crate::core::text::LineText;
use crate::core::text::{self, TextLook};
use crate::core::time::{self, UnixMillis};
use crate::core::units::Seconds;
use crate::systems::GameSettings;
use crate::systems::hud::{self, Seat, Sizing, WindowKey};
use crate::systems::input::map::{self, InputAction};
use crate::systems::item::bag_view::{BagView, bag_view};
use crate::systems::item::card;
use crate::systems::item::grid::scrolled;
use crate::systems::item::inline::{self, InlineItem};
use crate::systems::item::transfer::{self, ItemTransfer, TransferPane, TrayWindow, tray_window};
use crate::systems::item::tray::ItemTray;
use crate::systems::item::{self, Inventory, ItemDef, ItemId, ItemStack};
use crate::systems::mention::{self, Mention};
use crate::systems::messages;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::settings::Placed;
use crate::systems::society::SocietyView;
use crate::systems::window::AnswerCount;

pub(crate) fn plugin(app: &mut App) {
    tray_window::<NewListing>(app);
    app.init_resource::<Browse>()
        .add_systems(Update, notice.run_if(in_state(ClientScreen::Area)))
        .add_systems(OnExit(ClientScreen::Area), forget);
}

/// The market: a sheet that is up while the market is open. Closing it closes the market.
pub(crate) fn draw(world: &mut World, gui: &egui::Context) {
    tell_created(world);
    let open = view(world).cloned();
    let (Some(settings), Some(now)) = (
        world.resource::<GameSettings>().market.clone(),
        time::server_now(world),
    ) else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let words = messages::of(&content);
    let screen = gui.content_rect();
    let size = sheet::titled_size(Vec2::new(WIDTH + 2.0 * sheet::PANE_PAD.x, BODY_HEIGHT));
    let seat = Seat {
        key: WindowKey::Market,
        of: Placed::Market,
        start: Rect::from_min_size(
            Pos2::new(screen.center().x - size.x / 2.0, screen.top() + TOP),
            size,
        ),
        sizing: Sizing::Resizable {
            min: sheet::titled_size(Vec2::new(
                SMALLEST_WIDTH + 2.0 * sheet::PANE_PAD.x,
                SMALLEST_BODY_HEIGHT,
            )),
        },
    };
    let title = |_: &MarketView| words.market.title().to_owned();
    let drawn = hud::viewed(world, gui, seat, open, title, |ui, world, shown| {
        let pad = sheet::PANE_PAD;
        let mine = my_listings(world);
        let tabs = [
            Tab::new(MarketTab::Browse, words.market.browse()),
            Tab::new(MarketTab::List, words.market.new_listing()),
            Tab::new(MarketTab::Mine, words.market.my_listings(mine.len())),
        ];
        let mut tab = world.resource::<Browse>().tab;
        ui.add_space(pad.y);
        let room = ui.available_width() - 2.0 * pad.x;
        hud::tabs(ui, world, TabSize::Sub, &tabs, &mut tab, room, pad.x);
        world.resource_mut::<Browse>().tab = tab;
        let padding = egui::Margin {
            left: pad.x as i8,
            right: pad.x as i8,
            top: TABS_GAP as i8,
            bottom: pad.y as i8,
        };
        egui::Frame::NONE.inner_margin(padding).show(ui, |ui| {
            let page = ui.available_size();
            ui.allocate_ui(page, |ui| {
                ui.set_min_height(page.y);
                match tab {
                    MarketTab::Browse => browse(ui, world, shown, now),
                    MarketTab::List => new_listing(ui, world, &settings),
                    MarketTab::Mine => own(ui, world, &mine, &settings, now),
                }
            });
        });
    });
    if drawn.is_some_and(|drawn| drawn.closed) {
        close(world);
    }
}

/// Tells the player the listing they created is up, once it shows among their listings.
fn tell_created(world: &mut World) {
    let Some((goods, before)) = world.resource::<Browse>().created.clone() else {
        return;
    };
    let up = my_listings(world)
        .iter()
        .any(|row| !before.contains(&row.id) && row.goods == goods);
    if !up {
        return;
    }
    world.resource_mut::<Browse>().created = None;
    let content = world.resource::<Content>().clone();
    let words = messages::of(&content);
    let title = words
        .market
        .listed(ItemStack::listed(&words.interface, &goods));
    let dismiss = words.interface.ok().to_owned();
    hud::tell(
        world,
        hud::Tell {
            title,
            body: Vec::new(),
            dismiss,
        },
    );
}

/// Closes the market, if it is open: what stepping out does while it is.
pub fn is_open(world: &World) -> bool {
    view(world).is_some()
}

pub fn close(world: &mut World) -> bool {
    if !is_open(world) {
        return false;
    }
    world.write_message(MarketRequest::Close);
    true
}

/// The market as its window shows it, for a driver of the client.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownMarket {
    pub tab: MarketTab,
    /// The item whose listings the player browses, none until one is chosen.
    pub item: Option<ItemId>,
    /// The page of listings the latest search found, none before the first.
    pub results: Option<Vec<ShownListing>>,
    pub sort: Option<ListingSort>,
    /// Every way the listings that match can be sorted.
    pub sorts: Vec<SortBy>,
    pub page: u32,
    pub pages: u32,
    /// How many listings match.
    pub found: u32,
    /// The listings the player has up.
    pub mine: Vec<ShownListing>,
    /// The goods of the new listing so far.
    pub goods: Vec<ShownMarketStack>,
    pub waiting: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownListing {
    pub id: ListingId,
    pub goods: Vec<ShownMarketStack>,
    pub price: Vec<ShownMarketStack>,
    pub left: Seconds,
    pub mine: bool,
    /// Whether someone else bought it before the player's purchase settled.
    pub gone: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq)]
pub struct ShownMarketStack {
    pub item: ItemId,
    pub count: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MarketTab {
    #[default]
    Browse,
    List,
    Mine,
}

pub fn shown(world: &World) -> Option<ShownMarket> {
    let view = view(world)?;
    let now = time::server_now(world)?;
    let browse = world.resource::<Browse>();
    let row = |row: &ListingRow| ShownListing {
        id: row.id,
        goods: shown_stacks(&row.goods),
        price: shown_stacks(&row.price),
        left: row.ends.since(now),
        mine: row.mine,
        gone: view.gone.contains(&row.id),
    };
    Some(ShownMarket {
        tab: browse.tab,
        item: browse.item,
        results: view
            .results
            .as_ref()
            .map(|results| results.rows.iter().map(row).collect()),
        page: view
            .results
            .as_ref()
            .map_or(0, |results| results.query.page),
        pages: view.results.as_ref().map_or(0, |results| results.pages),
        found: view.results.as_ref().map_or(0, |results| results.found),
        sort: view.results.as_ref().and_then(|results| results.query.sort),
        sorts: view
            .results
            .as_ref()
            .map(|results| results.sorts.clone())
            .unwrap_or_default(),
        mine: my_listings(world).iter().map(row).collect(),
        goods: shown_stacks(&browse.goods.covered()),
        waiting: browse.waiting_on == Some(view.answered),
    })
}

const WIDTH: f32 = 860.0;
const TOP: f32 = 80.0;
const SMALLEST_WIDTH: f32 = 620.0;
const SMALLEST_BODY_HEIGHT: f32 = 400.0;
const BODY_HEIGHT: f32 = 510.0;
const ITEM_WIDTH: f32 = 220.0;
/// The gap between a listing's two lines.
const LINE_GAP: f32 = 1.0;
/// The room under the listings: a gap, and a line of the buttons that turn the page.
const PAGER: f32 = PAGER_GAP + controls::CONTROL_HEIGHT;
const PAGER_GAP: f32 = 6.0;
const BAG_WIDTH: f32 = 320.0;
/// The room above a line that says the list is empty.
const NOTHING_ROOM: f32 = 24.0;
/// The room between the tabs and the page under them.
const TABS_GAP: f32 = 14.0;
/// The room on a card of the player's own listing for how long it has left and Cancel.
const CARD_SIDE: f32 = 96.0;
const AMOUNT_DIGITS: usize = u32::MAX.ilog10() as usize + 1;
const AMOUNT_WIDTH: f32 = 96.0;

/// What the player has set the window to, which only the client holds.
#[derive(Resource, Default)]
struct Browse {
    tab: MarketTab,
    /// The item whose listings show, none until one is chosen.
    item: Option<ItemId>,
    /// How the listings are sorted, as the market sorts them while none is chosen.
    sort: Option<ListingSort>,
    page: u32,
    /// How many listings a page has room for.
    rows: u32,
    /// The goods of the new listing, still in the bag: at most one stack of each item.
    goods: ItemTray,
    /// The new listing's price, as the seller is setting it.
    price: Vec<PriceRow>,
    duration: usize,
    /// While the market owes an answer, the count of answers it had given when asked.
    waiting_on: Option<AnswerCount>,
    /// The listing the player created and is yet to be told is up: its goods, and the
    /// listings they had up before it.
    created: Option<(Vec<ItemStack>, Vec<ListingId>)>,
    open: bool,
}

impl Browse {
    /// Shows the listings of `item`, or none, from their first page, sorted as the market sorts
    /// them.
    fn choose(&mut self, item: Option<ItemId>) {
        self.item = item;
        self.sort = None;
        self.page = 0;
    }

    /// The search the window shows once an item is chosen, within what the market's last answer
    /// for the item allows: a sort the listings still allow, the market's own until one is
    /// chosen, and a page there is.
    fn query(&self, shown: &MarketView, page_rows: u32) -> Option<MarketQuery> {
        let item = self.item?;
        let answered = shown
            .results
            .as_ref()
            .filter(|results| results.query.item == item);
        let (sort, page) = match answered {
            Some(results) => (
                self.sort
                    .filter(|sort| results.sorts.contains(&sort.by))
                    .or(results.query.sort),
                self.page.min(results.pages.saturating_sub(1)),
            ),
            None => (self.sort, self.page),
        };
        Some(MarketQuery {
            item,
            sort,
            page,
            rows: self.rows.clamp(1, page_rows),
        })
    }
}

fn forget(mut browse: ResMut<Browse>) {
    *browse = Browse::default();
}

/// Starts the window afresh each time the market opens, and keeps the goods of the new listing
/// within what the bag still holds.
fn notice(world: &mut World) {
    let open = view(world).is_some();
    if world.resource::<Browse>().open != open {
        let price = world
            .resource::<GameSettings>()
            .market
            .as_ref()
            .map(|settings| vec![PriceRow::of(settings.default_currency)])
            .unwrap_or_default();
        *world.resource_mut::<Browse>() = Browse {
            open,
            price,
            ..Browse::default()
        };
    }
    let Some(bag) = item::my_bag(world).cloned() else {
        return;
    };
    let mut goods = world.resource::<Browse>().goods.clone();
    goods.keep_within(&bag);
    goods.drop_short();
    if world.resource::<Browse>().goods != goods {
        world.resource_mut::<Browse>().goods = goods;
    }
}

fn view(world: &World) -> Option<&MarketView> {
    session::my_character(world)?.get::<MarketView>()
}

fn my_listings(world: &World) -> Vec<ListingRow> {
    session::my_character(world)
        .and_then(|me| me.get::<SocietyView>())
        .map(|view| view.listings.clone())
        .unwrap_or_default()
}

fn listing(world: &World) -> bool {
    view(world).is_some() && world.resource::<Browse>().tab == MarketTab::List
}

fn shown_stacks(stacks: &[ItemStack]) -> Vec<ShownMarketStack> {
    stacks
        .iter()
        .map(|stack| ShownMarketStack {
            item: stack.item,
            count: stack.count,
        })
        .collect()
}

/// Sends `ask` unless the market still owes an answer, so one press is one ask.
fn send(world: &mut World, ask: MarketAsk) {
    let Some(answered) = view(world).map(|view| view.answered) else {
        return;
    };
    let mut browse = world.resource_mut::<Browse>();
    if browse.waiting_on == Some(answered) {
        return;
    }
    browse.waiting_on = Some(answered);
    world.write_message(MarketRequest::Ask(ask));
}

fn waiting(world: &World) -> bool {
    view(world).is_some_and(|view| world.resource::<Browse>().waiting_on == Some(view.answered))
}

/// Browse: the Item field that names what to look for, how the listings are sorted, and a page of
/// the listings that match.
fn browse(ui: &mut Ui, world: &mut World, shown: &MarketView, now: UnixMillis) {
    let content = world.resource::<Content>().clone();
    let words = messages::of(&content);
    let chosen = world.resource::<Browse>().item;
    let results = shown
        .results
        .as_ref()
        .filter(|results| Some(results.query.item) == chosen);
    ui.horizontal(|ui| {
        item_field(ui, world, &content);
        if let Some(results) = results {
            ui.add_space(sheet::GUTTER);
            sort_fields(ui, world, results);
        }
    });
    ui.add_space(8.0);
    let heads = sheet::ColumnHeads::new(ui, words.market.goods(), words.market.price());
    let room = ui.available_height() - PAGER - heads.room(ui) + ui.spacing().item_spacing.y;
    let fit = chosen.map_or(1, |item| {
        (room / row_pitch(ui, world, item)).floor().max(1.0) as u32
    });
    world.resource_mut::<Browse>().rows = fit;
    match results {
        None if chosen.is_none() => {
            form::nothing_selected(ui, words.market.choose_item());
        }
        None => {}
        Some(results) if results.rows.is_empty() => {
            let said = words
                .market
                .no_listings_of(Mention::Item(results.query.item).span(&content));
            ui.add_space(NOTHING_ROOM);
            ui.vertical_centered(|ui| {
                line(ui, world, &said, TextRole::Small, ink::INK2, false);
            });
        }
        Some(results) => listings(ui, world, shown, results, heads, now),
    }
    let asked = content
        .built::<GameSettings>()
        .market
        .as_ref()
        .and_then(|market| world.resource::<Browse>().query(shown, market.page_rows));
    if let Some(asked) = asked
        && results.map(|results| results.query) != Some(asked)
        && !waiting(world)
    {
        send(world, MarketAsk::Search(asked));
    }
}

/// The Item field: a combo box of every item that can change hands, narrowed by typing, whose
/// pick shows the listings with it among their goods.
fn item_field(ui: &mut Ui, world: &mut World, content: &Content) {
    let words = &messages::of(content).market;
    let label = ui.label(face::small(words.item()).color(ink::INK3));
    let mut chosen = world.resource::<Browse>().item;
    let search = controls::SearchWords {
        hint: words.item_hint(),
        none_found: words.no_such_item(),
    };
    let tradeable = tradeable(content);
    mention::search_dropdown(
        ui,
        world,
        "market item",
        &mut chosen,
        &tradeable,
        search,
        ITEM_WIDTH,
    )
    .labelled_by(label.id);
    if chosen != world.resource::<Browse>().item {
        world.resource_mut::<Browse>().choose(chosen);
    }
}

/// What the listings are sorted by, among the items in their goods and those of their prices, and
/// which way.
fn sort_fields(ui: &mut Ui, world: &mut World, results: &MarketResults) {
    let Some(sorted) = results.query.sort else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let words = &messages::of(&content).market;
    let mut sort = sorted;
    let side = |of: fn(SortBy) -> Option<ItemId>| -> Vec<(SortBy, Mention)> {
        results
            .sorts
            .iter()
            .filter_map(|&by| Some((by, Mention::Item(of(by)?))))
            .collect()
    };
    let groups = [
        (
            words.goods(),
            side(|by| match by {
                SortBy::Goods(item) => Some(item),
                SortBy::Price(_) => None,
            }),
        ),
        (
            words.price(),
            side(|by| match by {
                SortBy::Price(item) => Some(item),
                SortBy::Goods(_) => None,
            }),
        ),
    ];
    let label = ui.label(face::small(words.sort_by()).color(ink::INK3));
    mention::grouped_dropdown(ui, world, "market sort", &mut sort.by, &groups)
        .labelled_by(label.id);
    let label = ui.label(face::small(words.order()).color(ink::INK3));
    let order = |descending: bool| {
        if descending {
            words.high_to_low()
        } else {
            words.low_to_high()
        }
    };
    controls::combo("market order")
        .selected_text(order(sort.descending))
        .show_ui(ui, |ui| {
            for descending in [false, true] {
                ui.selectable_value(&mut sort.descending, descending, order(descending));
            }
        })
        .response
        .labelled_by(label.id);
    if sort != sorted {
        let mut browse = world.resource_mut::<Browse>();
        browse.sort = Some(sort);
        browse.page = 0;
    }
}

/// A page of the listings that match under the heads of their columns, and the buttons that turn
/// the page.
fn listings(
    ui: &mut Ui,
    world: &mut World,
    shown: &MarketView,
    results: &MarketResults,
    mut heads: sheet::ColumnHeads,
    now: UnixMillis,
) {
    let content = world.resource::<Content>().clone();
    let words = messages::of(&content);
    heads.place(ui);
    let mut price_end = ui.max_rect().right();
    for row in &results.rows {
        let gone = shown.gone.contains(&row.id);
        let each = each(row, &results.query);
        price_end = listing_row(ui, world, row, each, now, gone);
    }
    heads.paint(ui, price_end);
    ui.add_space(PAGER_GAP);
    ui.horizontal(|ui| {
        let page = results.query.page;
        let back = page > 0
            && (controls::button(ui, words.interface.back()).clicked()
                || map::just_pressed(world, InputAction::PageBack));
        let said = words.market.page(results.found, page + 1, results.pages);
        ui.label(face::small(said));
        let on = page + 1 < results.pages
            && (controls::button(ui, words.interface.next()).clicked()
                || map::just_pressed(world, InputAction::PageForward));
        let mut browse = world.resource_mut::<Browse>();
        if back {
            browse.page = page - 1;
        } else if on {
            browse.page = page + 1;
        }
    });
}

/// One listing of a page: its goods and how long it has left, its price and, where `each`
/// says, its price per one in the item sorted by, and Buy. Answers where its price ends.
fn listing_row(
    ui: &mut Ui,
    world: &mut World,
    row: &ListingRow,
    each: Option<(f32, ItemId)>,
    now: UnixMillis,
    gone: bool,
) -> f32 {
    let look = controls::RowLook {
        faint: gone,
        hover: controls::RowHover::Unlit,
        ..controls::RowLook::default()
    };
    let ink = if gone { ink::INK3 } else { ink::INK };
    let content = world.resource::<Content>().clone();
    let words = messages::of(&content);
    let lines = row
        .goods
        .first()
        .map_or(0.0, |stack| row_lines(ui, world, stack.item));
    let mut price_end = 0.0;
    let response = controls::row(ui, look, |ui| {
        ui.set_height(lines);
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                buy_button(ui, world, row, gone);
                ui.add_space(sheet::GUTTER);
                ui.with_layout(Layout::top_down(Align::Max), |ui| {
                    price_end = ui.max_rect().right();
                    ui.spacing_mut().item_spacing.y = LINE_GAP;
                    line(
                        ui,
                        world,
                        &ItemStack::summed_by_icon(&row.price),
                        TextRole::Body,
                        ink,
                        gone,
                    );
                    if let Some((each, by)) = each {
                        let said = words.market.each(InlineItem::of(by).share(each).nameless());
                        line(ui, world, &said, TextRole::Small, ink::INK3, false);
                    }
                });
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = LINE_GAP;
                        let goods = ItemStack::listed(&words.interface, &row.goods);
                        line(ui, world, &goods, TextRole::Body, ink, gone);
                        let left = words.time.remaining(row.ends.since(now));
                        let note = if gone {
                            words.market.taken().to_owned()
                        } else if row.mine {
                            words.market.yours(left)
                        } else {
                            left
                        };
                        ui.label(face::small(note).color(ink::INK3));
                    });
                });
            });
        });
    })
    .response;
    let items: Vec<ItemId> = row.goods.iter().map(|stack| stack.item).collect();
    card::chat_shortcut(world, &response, &items);
    price_end
}

/// Buy, which pays `row`'s price for its goods: shut, with the reason on hover,
/// while the listing is gone, the player's own, or more than the player holds.
fn buy_button(ui: &mut Ui, world: &mut World, row: &ListingRow, gone: bool) {
    let affords = item::my_bag(world).is_some_and(|bag| {
        row.price
            .iter()
            .all(|stack| bag.count(stack.item) >= stack.count)
    });
    let content = world.resource::<Content>().clone();
    let words = &messages::of(&content).market;
    let shut = if gone {
        Some(words.bought_first())
    } else if row.mine {
        Some(words.own_listing())
    } else if !affords {
        Some(words.cannot_pay())
    } else {
        None
    };
    let buy = words.buy();
    let pressed = match (waiting(world), shut) {
        (true, _) => controls::waiting(ui, buy),
        (false, Some(why)) => controls::faded(ui, buy).on_hover_text(why),
        (false, None) => controls::primary(ui, buy),
    };
    if pressed.clicked() && shut.is_none() {
        confirm_buy(world, row);
    }
}

/// Asks the player whether to pay `row`'s price for its goods, and buys it once they confirm.
fn confirm_buy(world: &mut World, row: &ListingRow) {
    let content = world.resource::<Content>().clone();
    let words = messages::of(&content);
    let title = words.market.confirm_buy(
        ItemStack::listed(&words.interface, &row.goods),
        ItemStack::summed(&row.price),
    );
    let buy = MarketAsk::Buy {
        listing: row.id,
        goods: row.goods.clone(),
        price: row.price.clone(),
    };
    hud::confirm(
        world,
        hud::Confirm {
            title,
            body: vec![LineText::from(words.market.cannot_undo())],
            confirm: words.market.buy().to_owned(),
            cancel: words.interface.cancel().to_owned(),
            answer: Box::new(move |world, bought| {
                if bought {
                    send(world, buy);
                }
            }),
        },
    );
}

fn new_listing(ui: &mut Ui, world: &mut World, settings: &MarketSettings) {
    let content = world.resource::<Content>().clone();
    let words = messages::of(&content);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(BAG_WIDTH);
            let view = BagView {
                heading: words.interface.inventory(),
                counts: false,
                placement: Placed::Market,
            };
            bag_view(ui, world, view);
        });
        ui.add_space(sheet::GUTTER);
        ui.vertical(|ui| {
            scrolled(ui, "listing goods", |ui| {
                sheet::section(ui, words.market.your_goods());
                let tray = world.resource::<Browse>().goods.clone();
                let pane = TransferPane {
                    placed: tray.placed(),
                    slots: settings.max_goods_items,
                    named: words.market.your_goods(),
                    transfer: Some(ItemTransfer::of_tray::<NewListing>()),
                };
                transfer::transfer_pane(ui, world, pane);
                let goods = tray.covered();
                ui.add_space(8.0);
                sheet::section(ui, words.market.your_price());
                let mut price = world.resource::<Browse>().price.clone();
                price_rows(ui, world, settings, &mut price);
                let priced: Vec<ItemStack> = price.iter().map(PriceRow::stack).collect();
                world.resource_mut::<Browse>().price = price;
                ui.add_space(6.0);
                let mut duration = world
                    .resource::<Browse>()
                    .duration
                    .min(settings.durations.len() - 1);
                ui.horizontal(|ui| {
                    ui.label(words.market.lasts());
                    for (index, &length) in settings.durations.iter().enumerate() {
                        let label = words.time.exact(length);
                        if ui.selectable_label(duration == index, label).clicked() {
                            duration = index;
                        }
                    }
                });
                world.resource_mut::<Browse>().duration = duration;
                ui.add_space(6.0);
                let deposit = words.market.deposit(settings.deposit().named());
                line(ui, world, &deposit, TextRole::Small, ink::INK2, false);
                ui.add_space(6.0);
                let refusal = listing_refusal(world, settings, &goods, &priced);
                let create = words.market.create();
                let pressed = match (waiting(world), &refusal) {
                    (true, _) => controls::waiting(ui, create),
                    (false, Some(_)) => controls::faded(ui, create),
                    (false, None) => controls::primary(ui, create),
                };
                if let Some(why) = &refusal {
                    line(ui, world, why, TextRole::Small, ink::RUBRIC, false);
                }
                if pressed.clicked() && refusal.is_none() {
                    let before = my_listings(world).iter().map(|row| row.id).collect();
                    world.resource_mut::<Browse>().created = Some((goods.clone(), before));
                    let ask = MarketAsk::List {
                        goods,
                        price: priced,
                        duration: settings.durations[duration],
                    };
                    send(world, ask);
                    world.resource_mut::<Browse>().goods.clear();
                }
            })
        });
    });
}

/// What stands in the way of listing `goods` for `price`, as the market would refuse it.
fn listing_refusal(
    world: &World,
    settings: &MarketSettings,
    goods: &[ItemStack],
    price: &[ItemStack],
) -> Option<LineText> {
    let content = world.resource::<Content>();
    let bag = Inventory::of(world, session::me(world));
    let listed = my_listings(world).len() as u32;
    settings
        .refusal(content, &bag, listed, goods, price)
        .map(|refusal| refusal.describe(content))
}

/// The rows of a listing's price: an item and how many of it on each, and a way to add
/// another item while the listing has room for it.
fn price_rows(ui: &mut Ui, world: &mut World, settings: &MarketSettings, rows: &mut Vec<PriceRow>) {
    let content = world.resource::<Content>().clone();
    let words = messages::of(&content);
    let tradeable = tradeable(&content);
    let mut removed = None;
    egui::Grid::new("market price")
        .num_columns(3)
        .show(ui, |ui| {
            let item_heading = ui.label(face::small(words.market.item()).color(ink::INK3));
            let amount_heading = ui.label(face::small(words.market.amount()).color(ink::INK3));
            ui.end_row();
            for index in 0..rows.len() {
                let taken: Vec<ItemId> = rows.iter().map(|row| row.item).collect();
                let row = &mut rows[index];
                let choices: Vec<ItemId> = tradeable
                    .iter()
                    .copied()
                    .filter(|&item| item == row.item || !taken.contains(&item))
                    .collect();
                let mut picked = Some(row.item);
                let search = controls::SearchWords {
                    hint: words.market.item_hint(),
                    none_found: words.market.no_such_item(),
                };
                mention::search_dropdown(
                    ui,
                    world,
                    ("market price item", index),
                    &mut picked,
                    &choices,
                    search,
                    ITEM_WIDTH,
                )
                .labelled_by(item_heading.id);
                if let Some(picked) = picked {
                    row.item = picked;
                }
                InputField::new(&mut row.amount)
                    .char_limit(AMOUNT_DIGITS)
                    .width(AMOUNT_WIDTH)
                    .show(ui)
                    .labelled_by(amount_heading.id);
                if index > 0 && controls::button(ui, words.interface.remove()).clicked() {
                    removed = Some(index);
                }
                ui.end_row();
            }
        });
    if let Some(index) = removed {
        rows.remove(index);
    }
    let free = tradeable
        .iter()
        .copied()
        .find(|item| rows.iter().all(|row| row.item != *item));
    if (rows.len() as u32) < settings.max_price_items
        && let Some(item) = free
        && controls::button(ui, words.market.add_item()).clicked()
    {
        rows.push(PriceRow::of(item));
    }
}

/// One item of a new listing's price, as the seller is setting it: the item, and its amount as typed.
#[derive(Clone, Debug, PartialEq)]
struct PriceRow {
    item: ItemId,
    amount: String,
}

impl PriceRow {
    fn of(item: ItemId) -> PriceRow {
        PriceRow {
            item,
            amount: String::new(),
        }
    }

    /// The stack it prices: none of the item while the amount doesn't read as a number.
    fn stack(&self) -> ItemStack {
        ItemStack::new(self.item, self.amount.trim().parse().unwrap_or(0))
    }
}

fn own(
    ui: &mut Ui,
    world: &mut World,
    mine: &[ListingRow],
    settings: &MarketSettings,
    now: UnixMillis,
) {
    let content = world.resource::<Content>().clone();
    let words = &messages::of(&content).market;
    if mine.is_empty() {
        form::nothing_selected(ui, words.none_up());
        return;
    }
    let said = words.up_of(mine.len(), settings.max_listings);
    form::caption(ui, said, ink::INK3);
    ui.add_space(8.0);
    let card = (ui.available_width() - sheet::GUTTER) / 2.0;
    egui::ScrollArea::vertical()
        .id_salt("my listings")
        .show(ui, |ui| {
            egui::Grid::new("my listings")
                .num_columns(2)
                .spacing(Vec2::splat(sheet::GUTTER))
                .show(ui, |ui| {
                    for (place, row) in mine.iter().enumerate() {
                        own_card(ui, world, row, card, now);
                        if place % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
        });
}

/// One of the player's listings as a card `width` wide: its goods and price, how long it has
/// left, and Cancel.
fn own_card(ui: &mut Ui, world: &mut World, row: &ListingRow, width: f32, now: UnixMillis) {
    let content = world.resource::<Content>().clone();
    let words = messages::of(&content);
    panel::tile(false).show(ui, |ui| {
        let inner = width - panel::tile(false).total_margin().sum().x;
        ui.set_width(inner);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(inner - CARD_SIDE);
                let goods = words
                    .market
                    .holds(ItemStack::listed(&words.interface, &row.goods));
                line(ui, world, &goods, TextRole::Body, ink::INK, false);
                let price = words.market.costs(ItemStack::summed_by_icon(&row.price));
                line(ui, world, &price, TextRole::Body, ink::INK, false);
            });
            ui.with_layout(Layout::top_down(Align::Max), |ui| {
                let left = words.time.left(words.time.exact(row.ends.since(now)));
                ui.label(face::small(left).color(ink::INK3));
                ui.add_space(4.0);
                if controls::button(ui, words.interface.cancel()).clicked() {
                    send(world, MarketAsk::Cancel(row.id));
                }
            });
        });
    });
}

/// Every item that can change hands, by name.
fn tradeable(content: &Content) -> Vec<ItemId> {
    let mut tradeable: Vec<ItemId> = content
        .ids::<ItemDef>()
        .filter(|item| !item.get(content).bound())
        .collect();
    tradeable.sort_by_key(|item| item.get(content).name.clone());
    tradeable
}

/// `row`'s price per one of the item searched for, in the item its search is sorted by, where its
/// price has that item and its goods hold more than one.
fn each(row: &ListingRow, query: &MarketQuery) -> Option<(f32, ItemId)> {
    let SortBy::Price(by) = query.sort?.by else {
        return None;
    };
    let per = row
        .goods
        .iter()
        .find(|stack| stack.item == query.item)?
        .count;
    let amount = row.price.iter().find(|stack| stack.item == by)?.count;
    (per > 1).then(|| (amount as f32 / per as f32, by))
}

/// The new listing's goods as a window items move into.
struct NewListing;

impl TrayWindow for NewListing {
    fn open(world: &World) -> bool {
        listing(world)
    }

    fn tray(world: &World) -> &ItemTray {
        &world.resource::<Browse>().goods
    }

    fn tray_mut(world: &mut World) -> Option<&mut ItemTray> {
        Some(&mut world.resource_mut::<Browse>().into_inner().goods)
    }

    fn slots(world: &World) -> usize {
        world
            .resource::<GameSettings>()
            .market
            .as_ref()
            .map_or(0, |settings| settings.max_goods_items as usize)
    }
}

/// Draws `line` as every window draws a line that names things, its items as inline items. Struck
/// through when what it names is gone.
fn line(
    ui: &mut Ui,
    world: &mut World,
    line: &LineText,
    role: TextRole,
    color: Color32,
    struck: bool,
) -> Response {
    let drawn = inline::show_line(ui, world, line, TextLook::new(role.size(), color), None);
    if struck {
        ui.painter().hline(
            drawn.rect.x_range(),
            drawn.rect.center().y,
            egui::Stroke::new(1.0, color),
        );
    }
    drawn
}

/// How far down one listing's row starts from the one above it, its lines naming `item`.
fn row_pitch(ui: &Ui, world: &World, item: ItemId) -> f32 {
    row_lines(ui, world, item)
        + 2.0 * f32::from(controls::ROW_MARGIN.top)
        + ui.spacing().item_spacing.y
}

/// How tall a listing's two lines are: its goods or price, and under it how long it has
/// left or its price per one, each naming `item`.
fn row_lines(ui: &Ui, world: &World, item: ItemId) -> f32 {
    let content = world.resource::<Content>();
    let named = LineText::from(InlineItem::of(item)).styled(content);
    let tall = |role: TextRole| {
        let look = TextLook::new(role.size(), ink::INK);
        text::measure(ui.ctx(), world, &named, look, f32::INFINITY).y
    };
    tall(TextRole::Body) + LINE_GAP + tall(TextRole::Small)
}
