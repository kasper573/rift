use std::hash::{DefaultHasher, Hash, Hasher};

use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::tokens::{palette, spacing, typography};
use ui::{Carried, CarryTarget, ChipOptions, ConfirmOptions, Family, OnTap, component};

use super::{OfferView, Sale, ShopId, ShopRequest, ShopWindow};
use crate::core::sfx::SfxId;
use crate::core::sfx::playback::{PlaySfx, SfxPlace};
use crate::data::item::Id as ItemId;
use crate::systems::hud;
use crate::systems::item::widget::{SlotSecondaryClicked, SlotVerdict, slot_verdict_source};
use crate::systems::item::{Inventory, ItemCategory, ItemStack, card};
use crate::systems::player::session::Viewpoint;
use crate::systems::scene::Scene as GameScene;
use crate::systems::scene::mode::Mode;

const WINDOW_ID: &str = "Shop";
const WINDOW_POS: Vec2 = Vec2::new(24.0, 96.0);
const WINDOW_SIZE: Vec2 = Vec2::new(620.0, 400.0);
const ICON: f32 = 28.0;
const BIG_ICON: f32 = 48.0;

pub struct ShopWindowPlugin;

impl Plugin for ShopWindowPlugin {
    fn build(&self, app: &mut App) {
        slot_verdict_source(app, verdict);
        app.init_resource::<Browse>()
            .add_systems(
                Update,
                (show_window, sync_body)
                    .chain()
                    .run_if(in_state(GameScene::Area)),
            )
            .add_systems(
                OnExit(GameScene::Area),
                (crate::systems::scene::despawn_all::<ShopPanel>, forget),
            )
            .add_observer(sell_on_secondary_click);
    }
}

pub fn view(world: &World) -> Option<&ShopWindow> {
    world
        .resource::<Viewpoint>()
        .0
        .and_then(|seen| world.get::<ShopWindow>(seen))
}

pub fn close(world: &mut World) -> bool {
    if *world.resource::<Mode>() != Mode::Play || shown(world).is_none() {
        return false;
    }
    world.resource_mut::<Browse>().closing = true;
    world.write_message(ShopRequest::Close);
    true
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
    closing: bool,
    built: Option<u64>,
}

#[derive(Component, Default, Clone)]
struct ShopPanel;

#[derive(Component, Default, Clone)]
struct ShopBody;

fn shown(world: &World) -> Option<&ShopWindow> {
    view(world).filter(|_| !world.resource::<Browse>().closing)
}

fn forget(mut browse: ResMut<Browse>) {
    *browse = Browse::default();
}

fn show_window(world: &mut World) {
    if view(world).is_none() {
        world.resource_mut::<Browse>().closing = false;
    }
    let shop = shown(world).map(|window| window.shop);
    let panel = world
        .query_filtered::<Entity, With<ShopPanel>>()
        .iter(world)
        .next();
    match (shop, panel) {
        (Some(shop), None) => {
            *world.resource_mut::<Browse>() = Browse::default();
            let scene = hud::placed_window(
                world,
                WINDOW_ID,
                (WINDOW_POS, WINDOW_SIZE),
                OnTap::new(|world| {
                    close(world);
                }),
                hud::single_tab(
                    shop.get().title,
                    bsn! { ShopBody Node { width: Val::Percent(100.0), height: Val::Percent(100.0) } },
                ),
            );
            if let Some(panel) = hud::spawn_in_hud(world, scene) {
                world.entity_mut(panel).insert(ShopPanel);
            }
            chime(world, SfxId::UiOpen);
        }
        (None, Some(panel)) => {
            world.entity_mut(panel).despawn();
            world.resource_mut::<Browse>().built = None;
            chime(world, SfxId::UiClose);
        }
        _ => {}
    }
}

fn sync_body(world: &mut World) {
    let Some(body) = world
        .query_filtered::<Entity, With<ShopBody>>()
        .iter(world)
        .next()
    else {
        return;
    };
    let Some(window) = shown(world).cloned() else {
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
    let mut hasher = DefaultHasher::new();
    format!(
        "{window:?}{:?}{:?}{}{playing}",
        inventory.slots, browse.tab, browse.selected
    )
    .hash(&mut hasher);
    let key = hasher.finish();
    if browse.built == Some(key) {
        return;
    }
    let tab = browse.tab;
    let selected = browse.selected;
    world.resource_mut::<Browse>().built = Some(key);
    let scene = contents(
        world,
        &Shelf {
            window,
            inventory,
            tab,
            selected,
            playing,
        },
    );
    world.entity_mut(body).despawn_related::<Children>();
    if let Ok(mut spawned) = world.spawn_scene(scene) {
        spawned.insert(ChildOf(body));
    }
}

struct Shelf {
    window: ShopWindow,
    inventory: Inventory,
    tab: ShopTab,
    selected: usize,
    playing: bool,
}

struct Ware {
    item: ItemId,
    count: u32,
    price: Vec<ItemStack>,
    note: Option<String>,
    refusal: Option<String>,
}

impl Shelf {
    fn wares(&self) -> Vec<Ware> {
        match self.tab {
            ShopTab::Wares => self
                .window
                .shop
                .get()
                .sells
                .iter()
                .zip(&self.window.offers)
                .map(|(offer, view)| Ware {
                    item: offer.item,
                    count: offer.count,
                    price: offer.price.to_vec(),
                    note: stock_note(view),
                    refusal: view.refusal.clone(),
                })
                .collect(),
            ShopTab::Buyback => self
                .window
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

fn missing(inventory: &Inventory, price: &[ItemStack]) -> Option<String> {
    let mut trial = inventory.clone();
    trial
        .exchange(price, &[])
        .err()
        .map(|refusal| refusal.describe())
}

fn contents(world: &World, shelf: &Shelf) -> Box<dyn Scene> {
    let assets = world.resource::<AssetServer>();
    let ink = ui::theme::theme().surface_floating.on;
    let shop = shelf.window.shop.get();
    let wares = shelf.wares();
    let selected = shelf.selected.min(wares.len().saturating_sub(1));
    let rows: Vec<Box<dyn Scene>> = wares
        .iter()
        .enumerate()
        .map(|(index, ware)| row(assets, shelf, ware, index, index == selected))
        .collect();
    let list: Box<dyn Scene> = if rows.is_empty() {
        Box::new(bsn! {
            Node { padding: {UiRect::all(Val::Px(spacing::XL))} }
            Children [ {EntityScene(ui::styled_text(empty_note(shelf.tab), ink.with_alpha(0.6), typography::CAPTION))} ]
        })
    } else {
        Box::new(bsn! {
            Node { flex_direction: FlexDirection::Column, width: Val::Percent(100.0) }
            Children [ {rows} ]
        })
    };
    let detail: Box<dyn Scene> = match wares.get(selected) {
        Some(ware) => Box::new(detail(assets, shelf, ware, selected)),
        None => Box::new(bsn! { Node }),
    };
    let wants = shop.wants();
    let buys_note = if wants.is_empty() {
        "Buys nothing".to_owned()
    } else {
        format!("Buys {}", wants.join(", "))
    };
    let sell_hint = if wants.is_empty() || !shelf.playing {
        String::new()
    } else {
        "Right-click or drag from your bag to sell".to_owned()
    };
    let tabs: Vec<Box<dyn Scene>> = [
        (ShopTab::Wares, "Wares".to_owned()),
        (
            ShopTab::Buyback,
            format!("Buy back ({})", shelf.window.buyback.len()),
        ),
    ]
    .into_iter()
    .map(|(tab, label)| -> Box<dyn Scene> { Box::new(tab_button(tab, label, shelf.tab == tab)) })
    .collect();
    let split = ui::split_view(list, detail);
    let header = bsn! {
        Node {
            width: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::SpaceBetween,
            padding: {UiRect::axes(Val::Px(spacing::L), Val::Px(spacing::M))},
        }
        Children [
            ( Node { column_gap: Val::Px({spacing::M}) } Children [ {tabs} ] ),
            {EntityScene(ui::styled_text(buys_note, palette::AMBER_80, typography::CAPTION))},
        ]
    };
    let footer = bsn! {
        Node { padding: {UiRect::axes(Val::Px(spacing::L), Val::Px(spacing::M))} }
        Children [ {EntityScene(ui::styled_text(sell_hint, ink.with_alpha(0.6), typography::CAPTION))} ]
    };
    let column = bsn! {
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
        }
        Children [
            {EntityScene(header)},
            ( Node { flex_grow: 1.0, min_height: Val::Px(0.0), width: Val::Percent(100.0) } Children [ {EntityScene(split)} ] ),
            {EntityScene(footer)},
        ]
    };
    if shelf.playing {
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

fn empty_note(tab: ShopTab) -> &'static str {
    match tab {
        ShopTab::Wares => "Nothing for sale",
        ShopTab::Buyback => "Nothing sold here yet",
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
    let note = ware
        .refusal
        .clone()
        .map(|refusal| (refusal, palette::CRIMSON_80))
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
        on(move |_: On<Pointer<Click>>, mut commands: Commands| {
            commands.queue(move |world: &mut World| select(world, tab, index));
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
            ( Node { column_gap: Val::Px({spacing::M}) } Pickable::IGNORE Children [ {chips} ] ),
        ]
    })
}

fn detail(assets: &AssetServer, shelf: &Shelf, ware: &Ware, index: usize) -> impl Scene + use<> {
    let ink = ui::theme::theme().surface_floating.on;
    let def = ware.item.get();
    let costs: Vec<Box<dyn Scene>> = ware
        .price
        .iter()
        .map(|stack| -> Box<dyn Scene> { Box::new(cost_row(assets, &shelf.inventory, *stack)) })
        .collect();
    let notes: Vec<Box<dyn Scene>> = ware
        .note
        .iter()
        .map(|note| (note.clone(), ink.with_alpha(0.7)))
        .chain(
            ware.refusal
                .iter()
                .map(|refusal| (refusal.clone(), palette::CRIMSON_80)),
        )
        .map(|(note, color)| -> Box<dyn Scene> {
            Box::new(ui::styled_text(note, color, typography::CAPTION))
        })
        .collect();
    let action: Vec<Box<dyn Scene>> = shelf
        .playing
        .then(|| -> Box<dyn Scene> { Box::new(trade_button(shelf.tab, ware, index)) })
        .into_iter()
        .collect();
    let category = match ware.count {
        1 => def.category().label().to_owned(),
        count => format!("{} · ×{count}", def.category().label()),
    };
    let header = bsn! {
        Node { column_gap: Val::Px({spacing::L}), align_items: AlignItems::Center }
        Children [
            (
                Node { width: Val::Px({BIG_ICON}), height: Val::Px({BIG_ICON}) }
                component(ImageNode::new(assets.load(def.icon.0)))
            ),
            (
                Node { flex_direction: FlexDirection::Column }
                Children [
                    {EntityScene(ui::styled_text(def.display_name, ink, typography::NAME))},
                    {EntityScene(ui::styled_text(category, ink.with_alpha(0.6), typography::CAPTION))},
                ]
            ),
        ]
    };
    bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px({spacing::L}),
            padding: {UiRect::all(Val::Px(spacing::XL))},
            width: Val::Percent(100.0),
        }
        Children [
            {EntityScene(ui::link(card::link(ware.item), header))},
            {EntityScene(ui::styled_text("Price", ink.with_alpha(0.6), typography::LABEL))},
            ( Node { flex_direction: FlexDirection::Column, row_gap: Val::Px({spacing::M}) } Children [ {costs} ] ),
            ( Node { flex_direction: FlexDirection::Column, row_gap: Val::Px({spacing::S}) } Children [ {notes} ] ),
            {action},
        ]
    }
}

fn trade_button(tab: ShopTab, ware: &Ware, index: usize) -> impl Scene + use<> {
    let (intent, label) = match (&ware.refusal, tab) {
        (Some(refusal), _) if refusal.starts_with("Needs") && refusal.contains("more") => {
            (ui::button::intent::DANGER, "Can't afford".to_owned())
        }
        (Some(_), _) => (ui::button::intent::MUTED, "Unavailable".to_owned()),
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
        link: None,
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
        "{} {} · have {have}",
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

fn counted(name: &str, count: u32) -> String {
    if count > 1 {
        format!("{name} ×{count}")
    } else {
        name.to_owned()
    }
}

fn verdict(world: &World, stack: ItemStack) -> Option<SlotVerdict> {
    let window = shown(world)?;
    Some(
        match window.shop.get().pays_for(stack.item, &window.declined) {
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

fn sell_on_secondary_click(clicked: On<SlotSecondaryClicked>, mut commands: Commands) {
    let slot = clicked.slot;
    commands.queue(move |world: &mut World| request_sale(world, slot));
}

fn request_sale(world: &mut World, slot: u32) {
    if *world.resource::<Mode>() != Mode::Play {
        return;
    }
    let Some((shop, declined)) = shown(world).map(|window| (window.shop, window.declined.clone()))
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
