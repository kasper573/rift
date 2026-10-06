use std::hash::{DefaultHasher, Hash, Hasher};

use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::tokens::{palette, spacing, typography};
use ui::{Carried, CarryTarget, ChipOptions, ConfirmOptions, Family, OnTap, component};

use super::{OfferView, Sale, ShopId, ShopRequest, ShopView, WareRefusal};
use crate::core::sfx::SfxId;
use crate::core::sfx::playback::{PlaySfx, SfxPlace};
use crate::data::item::Id as ItemId;
use crate::systems::dialogue::stage;
use crate::systems::hud;
use crate::systems::input::map::{ActionInput, InputAction, input};
use crate::systems::item::widget::{SlotAction, SlotVerdict, slot_action, slot_verdict_source};
use crate::systems::item::{Inventory, ItemCategory, ItemStack, card};
use crate::systems::player::session::Viewpoint;
use crate::systems::scene::Scene as GameScene;
use crate::systems::scene::mode::Mode;

const COUNTER_SIZE: Vec2 = Vec2::new(620.0, 400.0);
const ICON: f32 = 28.0;

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
        app.init_resource::<Browse>()
            .add_systems(
                Update,
                (show_counter, sync_counter)
                    .chain()
                    .run_if(in_state(GameScene::Area)),
            )
            .add_systems(OnExit(GameScene::Area), forget);
    }
}

pub fn view(world: &World) -> Option<&ShopView> {
    world
        .resource::<Viewpoint>()
        .0
        .and_then(|seen| world.get::<ShopView>(seen))
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
    synced: Option<u64>,
}

#[derive(Component, Default, Clone)]
struct CounterPanel;

#[derive(Component, Default, Clone)]
struct ShopTabs;

#[derive(Component, Default, Clone)]
struct ShopWants;

#[derive(Component, Default, Clone)]
struct ShopList;

#[derive(Component, Default, Clone)]
struct ShopDetail;

#[derive(Component, Default, Clone)]
struct ShopFooter;

fn forget(mut browse: ResMut<Browse>) {
    *browse = Browse::default();
}

fn show_counter(world: &mut World) {
    let shop = view(world).map(|shown| shown.shop);
    let panel = world
        .query_filtered::<Entity, With<CounterPanel>>()
        .iter(world)
        .next();
    match (shop, panel) {
        (Some(shop), None) => {
            *world.resource_mut::<Browse>() = Browse::default();
            let playing = *world.resource::<Mode>() == Mode::Play;
            let panel = ui::DialoguePanelOptions {
                title: shop.get().title.to_owned(),
                width: Val::Px(COUNTER_SIZE.x),
                height: Val::Px(COUNTER_SIZE.y),
                content: counter(playing),
            };
            if let Some(panel) = stage::show_panel(world, panel) {
                world.entity_mut(panel).insert(CounterPanel);
            }
            chime(world, SfxId::UiOpen);
        }
        (None, Some(panel)) => {
            world.entity_mut(panel).despawn();
            world.resource_mut::<Browse>().synced = None;
        }
        _ => {}
    }
}

fn counter(playing: bool) -> Box<dyn Scene> {
    let header = bsn! {
        Node {
            width: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::SpaceBetween,
            padding: {UiRect::axes(Val::Px(spacing::L), Val::Px(spacing::M))},
        }
        Children [
            ( ShopTabs Node { column_gap: Val::Px({spacing::M}) } ),
            ( ShopWants Node ),
        ]
    };
    let split = ui::split_view(
        Box::new(
            bsn! { ShopList Node { flex_direction: FlexDirection::Column, width: Val::Percent(100.0) } },
        ),
        Box::new(
            bsn! { ShopDetail Node { width: Val::Percent(100.0), height: Val::Percent(100.0) } },
        ),
    );
    let column = bsn! {
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
        }
        Children [
            {EntityScene(header)},
            ( Node { flex_grow: 1.0, min_height: Val::Px(0.0), width: Val::Percent(100.0) } Children [ {EntityScene(split)} ] ),
            ( ShopFooter Node { padding: {UiRect::axes(Val::Px(spacing::L), Val::Px(spacing::M))} } ),
        ]
    };
    if playing {
        Box::new(bsn! {
            {column}
            CarryTarget
            on(|carried: On<Carried>, mut commands: Commands| {
                let slot = carried.payload as u32;
                commands.queue(move |world: &mut World| request_sale(world, slot));
            })
        })
    } else {
        Box::new(column)
    }
}

fn sync_counter(world: &mut World) {
    let Some(shown) = view(world).cloned() else {
        return;
    };
    let Some(parts) = Parts::find(world) else {
        return;
    };
    let inventory = world
        .resource::<Viewpoint>()
        .0
        .and_then(|seen| world.get::<Inventory>(seen))
        .cloned()
        .unwrap_or_else(Inventory::empty);
    let playing = *world.resource::<Mode>() == Mode::Play;
    let browse = world.resource::<Browse>();
    let synced = key(&(
        &shown,
        &inventory.slots,
        browse.tab,
        browse.selected,
        playing,
    ));
    if browse.synced == Some(synced) {
        return;
    }
    let shelf = Shelf {
        shown,
        inventory,
        tab: browse.tab,
        selected: browse.selected,
        playing,
    };
    world.resource_mut::<Browse>().synced = Some(synced);
    let wares = shelf.wares();
    let selected = shelf.selected.min(wares.len().saturating_sub(1));

    let tabs = [
        (ShopTab::Wares, "Wares".to_owned()),
        (
            ShopTab::Buyback,
            format!("Buy back ({})", shelf.shown.buyback.len()),
        ),
    ];
    let tab_keys: Vec<u64> = tabs
        .iter()
        .map(|(tab, label)| key(&(tab, label, shelf.tab == *tab)))
        .collect();
    hud::reconcile_children(world, parts.tabs, &tab_keys, |_, index| {
        let (tab, label) = tabs[index].clone();
        Box::new(tab_button(tab, label, shelf.tab == tab))
    });

    let wants = shelf.shown.shop.get().wants();
    hud::reconcile_children(world, parts.wants, &[key(&wants)], |_, _| {
        Box::new(ui::styled_text(
            wants_note(&wants),
            palette::AMBER_80,
            typography::CAPTION,
        ))
    });

    let list_keys: Vec<u64> = if wares.is_empty() {
        vec![key(&shelf.tab)]
    } else {
        wares
            .iter()
            .enumerate()
            .map(|(index, ware)| {
                key(&(
                    shelf.tab,
                    index,
                    ware,
                    index == selected,
                    held(&shelf.inventory, ware),
                ))
            })
            .collect()
    };
    hud::reconcile_children(world, parts.list, &list_keys, |world, index| {
        match wares.get(index) {
            Some(ware) => row(
                world.resource::<AssetServer>(),
                &shelf,
                ware,
                index,
                index == selected,
            ),
            None => Box::new(empty_note(shelf.tab)),
        }
    });

    let detail_keys: Vec<u64> = wares
        .get(selected)
        .map(|ware| {
            key(&(
                shelf.tab,
                selected,
                ware,
                held(&shelf.inventory, ware),
                shelf.playing,
            ))
        })
        .into_iter()
        .collect();
    hud::reconcile_children(world, parts.detail, &detail_keys, |world, _| {
        Box::new(detail(world, &shelf, &wares[selected], selected))
    });

    let selling = shelf.playing && !wants.is_empty();
    hud::reconcile_children(world, parts.footer, &[key(&selling)], |_, _| {
        Box::new(sell_hint(selling))
    });
}

struct Parts {
    tabs: Entity,
    wants: Entity,
    list: Entity,
    detail: Entity,
    footer: Entity,
}

impl Parts {
    fn find(world: &mut World) -> Option<Parts> {
        Some(Parts {
            tabs: single::<ShopTabs>(world)?,
            wants: single::<ShopWants>(world)?,
            list: single::<ShopList>(world)?,
            detail: single::<ShopDetail>(world)?,
            footer: single::<ShopFooter>(world)?,
        })
    }
}

fn single<C: Component>(world: &mut World) -> Option<Entity> {
    world.query_filtered::<Entity, With<C>>().iter(world).next()
}

fn key(value: &impl std::fmt::Debug) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{value:?}").hash(&mut hasher);
    hasher.finish()
}

fn held(inventory: &Inventory, ware: &Ware) -> Vec<u32> {
    ware.price
        .iter()
        .map(|stack| inventory.count(stack.item))
        .collect()
}

struct Shelf {
    shown: ShopView,
    inventory: Inventory,
    tab: ShopTab,
    selected: usize,
    playing: bool,
}

#[derive(Debug)]
struct Ware {
    item: ItemId,
    count: u32,
    price: Vec<ItemStack>,
    note: Option<String>,
    refusal: Option<WareRefusal>,
}
impl Shelf {
    fn wares(&self) -> Vec<Ware> {
        match self.tab {
            ShopTab::Wares => self
                .shown
                .shop
                .get()
                .sells
                .iter()
                .zip(&self.shown.offers)
                .map(|(offer, view)| Ware {
                    item: offer.item,
                    count: offer.count,
                    price: offer.price.to_vec(),
                    note: stock_note(view),
                    refusal: view.refusal.clone(),
                })
                .collect(),
            ShopTab::Buyback => self
                .shown
                .buyback
                .iter()
                .map(|sale: &Sale| Ware {
                    item: sale.item,
                    count: sale.count,
                    price: sale.paid.clone(),
                    note: None,
                    refusal: missing(&self.inventory, &sale.paid),
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

fn missing(inventory: &Inventory, price: &[ItemStack]) -> Option<WareRefusal> {
    let mut trial = inventory.clone();
    trial.exchange(price, &[]).err().map(WareRefusal::from)
}

fn wants_note(wants: &[String]) -> String {
    if wants.is_empty() {
        "Buys nothing".to_owned()
    } else {
        format!("Buys {}", wants.join(", "))
    }
}

fn sell_hint(selling: bool) -> impl Scene {
    let ink = ui::theme::theme().surface_floating.on;
    let pieces = if selling {
        vec![
            input(InputAction::SellItem),
            ui::RichPiece::text(" or "),
            input(InputAction::CarryItem),
            ui::RichPiece::text(" from your bag to sell"),
        ]
    } else {
        Vec::new()
    };
    ui::rich_text(
        ui::RichText {
            pieces,
            size: typography::CAPTION.font_size,
            color: ink.with_alpha(0.6),
        },
        false,
    )
}

fn empty_note(tab: ShopTab) -> impl Scene {
    let ink = ui::theme::theme().surface_floating.on;
    let note = match tab {
        ShopTab::Wares => "Nothing for sale",
        ShopTab::Buyback => "Nothing sold here yet",
    };
    bsn! {
        Node { padding: {UiRect::all(Val::Px(spacing::XL))} }
        Children [ {EntityScene(ui::styled_text(note, ink.with_alpha(0.6), typography::CAPTION))} ]
    }
}

fn tab_button(tab: ShopTab, label: String, active: bool) -> impl Scene {
    let intent = if active {
        ui::button::intent::PRIMARY
    } else {
        ui::button::intent::SECONDARY
    };
    bsn! {
        {ui::button_styled(intent, ui::ButtonSize::Sm, label)}
        on(move |_: On<ui::Activate>, mut commands: Commands| {
            commands.queue(move |world: &mut World| select(world, tab, 0));
        })
    }
}

fn row(
    assets: &AssetServer,
    shelf: &Shelf,
    ware: &Ware,
    index: usize,
    selected: bool,
) -> Box<dyn Scene> {
    let ink = ui::theme::theme().surface_floating.on;
    let def = ware.item.get();
    let name = counted(def.display_name, ware.count);
    let note = unavailable(ware)
        .map(|reason| (reason, palette::CRIMSON_80))
        .or_else(|| ware.note.clone().map(|note| (note, ink.with_alpha(0.6))));
    let note: Vec<Box<dyn Scene>> = note
        .into_iter()
        .map(|(note, color)| -> Box<dyn Scene> {
            Box::new(ui::styled_text(note, color, typography::CAPTION))
        })
        .collect();
    let chips: Vec<Box<dyn Scene>> = ware
        .price
        .iter()
        .map(|stack| -> Box<dyn Scene> { Box::new(price_chip(assets, &shelf.inventory, *stack)) })
        .collect();
    let background = if selected {
        ui::theme::theme().surface_inset.base
    } else {
        Color::NONE
    };
    let tab = shelf.tab;
    Box::new(bsn! {
        Node {
            width: Val::Percent(100.0),
            align_items: AlignItems::Center,
            column_gap: Val::Px({spacing::L}),
            padding: {UiRect::axes(Val::Px(spacing::L), Val::Px(spacing::M))},
        }
        BackgroundColor({background})
        Pickable { should_block_lower: true, is_hoverable: true }
        ui::component(ui::CursorStyle::Pointer)
        on(move |click: On<Pointer<Click>>, input: ActionInput, mut commands: Commands| {
            if input.clicked(InputAction::Select, &click) {
                commands.queue(move |world: &mut World| select(world, tab, index));
            }
        })
        Children [
            (
                Node { width: Val::Px({ICON}), height: Val::Px({ICON}), flex_shrink: 0.0 }
                component(ImageNode::new(assets.load(def.icon.0)))
                Pickable::IGNORE
            ),
            (
                Node { flex_direction: FlexDirection::Column, flex_grow: 1.0 }
                Pickable::IGNORE
                Children [
                    {EntityScene(ui::styled_text(name, ink, typography::LABEL))},
                    {note},
                ]
            ),
            (
                Node { flex_direction: FlexDirection::Column, align_items: AlignItems::End, row_gap: Val::Px({spacing::S}) }
                Pickable::IGNORE
                Children [ {chips} ]
            ),
        ]
    })
}

fn detail(world: &World, shelf: &Shelf, ware: &Ware, index: usize) -> impl Scene + use<> {
    let assets = world.resource::<AssetServer>();
    let family = ui::theme::theme().surface_floating;
    let ink = family.on;
    let costs: Vec<Box<dyn Scene>> = ware
        .price
        .iter()
        .map(|stack| -> Box<dyn Scene> { Box::new(cost_row(assets, &shelf.inventory, *stack)) })
        .collect();
    let notes: Vec<Box<dyn Scene>> = ware
        .note
        .iter()
        .map(|note| (note.clone(), ink.with_alpha(0.7)))
        .chain(unavailable(ware).map(|reason| (reason, palette::CRIMSON_80)))
        .map(|(note, color)| -> Box<dyn Scene> {
            Box::new(ui::styled_text(note, color, typography::CAPTION))
        })
        .collect();
    let action: Vec<Box<dyn Scene>> = shelf
        .playing
        .then(|| -> Box<dyn Scene> { Box::new(trade_button(shelf.tab, ware, index)) })
        .into_iter()
        .collect();
    let price = match ware.count {
        1 => "Price".to_owned(),
        count => format!("Price for {count}"),
    };
    let sheet = card::sheet(world, ware.item);
    bsn! {
        Node { flex_direction: FlexDirection::Column, width: Val::Percent(100.0), height: Val::Percent(100.0) }
        Children [
            (
                Node { flex_grow: 1.0, min_height: Val::Px(0.0) }
                Children [ {EntityScene(ui::scrolled(Box::new(bsn! {
                    Node { padding: {UiRect::all(Val::Px(spacing::XL))}, width: Val::Percent(100.0) }
                    Children [ {EntityScene(sheet)} ]
                })))} ]
            ),
            (
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px({spacing::M}),
                    padding: {UiRect::all(Val::Px(spacing::XL))},
                    border: {UiRect::top(Val::Px(1.0))},
                }
                component(BorderColor::all(family.border))
                Children [
                    {EntityScene(ui::styled_text(price, ink.with_alpha(0.6), typography::LABEL))},
                    ( Node { flex_direction: FlexDirection::Column, row_gap: Val::Px({spacing::M}) } Children [ {costs} ] ),
                    ( Node { flex_direction: FlexDirection::Column, row_gap: Val::Px({spacing::S}) } Children [ {notes} ] ),
                    {action},
                ]
            ),
        ]
    }
}

fn trade_button(tab: ShopTab, ware: &Ware, index: usize) -> impl Scene + use<> {
    let (intent, label) = match (&ware.refusal, tab) {
        (Some(WareRefusal::Unaffordable(_)), _) => {
            (ui::button::intent::DANGER, "Can't afford".to_owned())
        }
        (Some(WareRefusal::Unavailable(_)), _) => {
            (ui::button::intent::MUTED, "Unavailable".to_owned())
        }
        (None, ShopTab::Wares) => (ui::button::intent::PRIMARY, "Buy".to_owned()),
        (None, ShopTab::Buyback) => (ui::button::intent::PRIMARY, "Buy back".to_owned()),
    };
    bsn! {
        {ui::button_styled(intent, ui::ButtonSize::Md, label)}
        on(move |_: On<ui::Activate>, mut commands: Commands| {
            commands.queue(move |world: &mut World| trade(world, tab, index));
        })
    }
}

fn trade(world: &mut World, tab: ShopTab, index: usize) {
    if *world.resource::<Mode>() != Mode::Play {
        return;
    }
    let index = index as u32;
    match tab {
        ShopTab::Wares => {
            world.write_message(ShopRequest::Buy { offer: index });
        }
        ShopTab::Buyback => {
            world.write_message(ShopRequest::Buyback { sale: index });
        }
    }
    chime(world, SfxId::UiPick);
}

fn price_chip(assets: &AssetServer, inventory: &Inventory, stack: ItemStack) -> impl Scene + use<> {
    let covered = inventory.count(stack.item) >= stack.count;
    ui::chip(ChipOptions {
        label: stack.count.to_string(),
        icon: Some(assets.load(stack.item.get().icon.0)),
        family: Family::outline(if covered {
            palette::AMBER_70
        } else {
            palette::CRIMSON_70
        }),
        inspect: None,
    })
}

fn cost_row(assets: &AssetServer, inventory: &Inventory, stack: ItemStack) -> impl Scene + use<> {
    let have = inventory.count(stack.item);
    let color = if have >= stack.count {
        ui::theme::theme().surface_floating.on
    } else {
        palette::CRIMSON_80
    };
    let label = format!(
        "{} {} · You have {have}",
        stack.count,
        stack.item.get().display_name
    );
    bsn! {
        Node { column_gap: Val::Px({spacing::L}), align_items: AlignItems::Center }
        Children [
            (
                Node { width: Val::Px(16.0), height: Val::Px(16.0) }
                component(ImageNode::new(assets.load(stack.item.get().icon.0)))
            ),
            {EntityScene(ui::styled_text(label, color, typography::BODY))},
        ]
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
    let shown = view(world)?;
    Some(
        match shown.shop.get().pays_for(stack.item, &shown.declined) {
            Ok(pays) => SlotVerdict::Wanted(format!("sells for {}", paid(pays, stack.count))),
            Err(refusal) => SlotVerdict::Refused(refusal),
        },
    )
}

fn paid(pays: &[ItemStack], count: u32) -> String {
    pays.iter()
        .map(|pay| ItemStack::new(pay.item, pay.count.saturating_mul(count)).describe())
        .collect::<Vec<_>>()
        .join(" + ")
}

fn selling(world: &World) -> bool {
    *world.resource::<Mode>() == Mode::Play && view(world).is_some()
}

fn request_sale(world: &mut World, slot: u32) {
    if *world.resource::<Mode>() != Mode::Play {
        return;
    }
    let Some((shop, declined)) = view(world).map(|shown| (shown.shop, shown.declined.clone()))
    else {
        return;
    };
    let Some(stack) = world
        .resource::<Viewpoint>()
        .0
        .and_then(|seen| world.get::<Inventory>(seen))
        .and_then(|inventory| inventory.slots.get(slot as usize).copied())
    else {
        return;
    };
    match shop.get().pays_for(stack.item, &declined) {
        Ok(pays) if stack.item.get().category() == ItemCategory::Equipment => {
            confirm_sale(world, shop, stack, pays, slot);
        }
        _ => sell(world, slot, stack),
    }
}

fn confirm_sale(world: &mut World, shop: ShopId, stack: ItemStack, pays: &[ItemStack], slot: u32) {
    let name = stack.item.get().display_name;
    let dialog = ui::confirm_dialog(ConfirmOptions {
        title: format!("Sell {name}?"),
        body: vec![
            format!("{} pays {}.", shop.get().title, paid(pays, stack.count)),
            "You can buy it back for the same until you leave the game.".to_owned(),
        ],
        confirm: "Sell".to_owned(),
        cancel: "Keep".to_owned(),
        on_confirm: OnTap::new(move |world| sell(world, slot, stack)),
        on_cancel: OnTap::new(|_| {}),
        keep: InputAction::TakeDefault.into(),
    });
    world.spawn_scene(dialog).ok();
}

fn sell(world: &mut World, slot: u32, stack: ItemStack) {
    world.write_message(ShopRequest::Sell { slot, stack });
    chime(world, SfxId::Coins);
}

fn chime(world: &mut World, id: SfxId) {
    world.write_message(PlaySfx {
        id,
        place: SfxPlace::Interface,
    });
}
