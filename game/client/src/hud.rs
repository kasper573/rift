use bevy::prelude::*;
use bevy::scene::EntityScene;
use bevy::window::{CursorIcon, SystemCursorIcon};
use serde::{Deserialize, Serialize};
use ui::button::intent as button_intent;
use ui::{
    Activate, Align, ButtonSize, Side, button_styled, text_colored, tooltip, tooltip_content,
};

use crate::component;
use world::protocol::{Inventory, Name, Vitals, Xp};
use world::session;

use crate::Screen;
use crate::drag::{
    DragHandle, DragRoot, Geom, HoverCursor, OnSettle, OnTap, ResizeHandle, SnapGrid,
};
use crate::user_settings::{Placement, ScreenPx, ScreenVec, UserSettings};

const WIDGET: ScreenPx = ScreenPx(48.0);
const SLOT: ScreenPx = ScreenPx(36.0);
const TITLE_H: ScreenPx = ScreenPx(22.0);
const SCREEN_W: f32 = 1152.0;
const MIN_WINDOW: Vec2 = Vec2::new(100.0, 100.0);
const WINDOW_SIZE: Vec2 = Vec2::new(400.0, 200.0);

const PANEL_BG: Color = Color::srgb(0.1, 0.1, 0.1);
const TITLE_BG: Color = Color::srgb(0.18, 0.18, 0.18);
const BORDER: Color = Color::srgb(0.31, 0.31, 0.31);
const TOOLTIP_BG: Color = Color::BLACK;

const POINTER: CursorIcon = CursorIcon::System(SystemCursorIcon::Pointer);
const RESIZE: CursorIcon = CursorIcon::System(SystemCursorIcon::NwseResize);

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Settings>()
            .init_resource::<Open>()
            .add_systems(OnEnter(Screen::Playing), spawn_hud)
            .add_systems(OnExit(Screen::Playing), despawn::<Hud>)
            .add_systems(
                Update,
                (
                    toggle_keys,
                    rebuild_panes,
                    sync_character,
                    sync_inventory,
                    sync_snapping,
                    sync_snap_grid,
                    sync_death_banner,
                )
                    .run_if(in_state(Screen::Playing)),
            );
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Pane {
    Inventory,
    Settings,
}

#[derive(Clone, Copy)]
struct PaneInfo {
    title: &'static str,
    toggle: KeyCode,
    keybind: &'static str,
    icon: &'static str,
}

impl Pane {
    const ALL: [Pane; 2] = [Pane::Inventory, Pane::Settings];

    fn info(self) -> PaneInfo {
        match self {
            Pane::Inventory => PaneInfo {
                title: "Inventory",
                toggle: KeyCode::KeyI,
                keybind: "I",
                icon: "icons/equipment/bag.png",
            },
            Pane::Settings => PaneInfo {
                title: "Settings",
                toggle: KeyCode::KeyO,
                keybind: "O",
                icon: "icons/misc/gear.png",
            },
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Panel {
    Character,
    Widget(Pane),
    Window(Pane),
}

impl Panel {
    fn resizable(self) -> bool {
        matches!(self, Panel::Window(_))
    }
}

#[derive(Resource)]
struct Settings(UserSettings);

impl Default for Settings {
    fn default() -> Settings {
        Settings(UserSettings::load())
    }
}

#[derive(Resource, Default)]
struct Open(std::collections::HashSet<Pane>);

#[derive(Component, Default, Clone)]
struct Hud;

#[derive(Component, Clone)]
struct PaneView {
    pane: Pane,
    open: bool,
}

#[derive(Component, Default, Clone)]
struct CharacterText;

#[derive(Component, Default, Clone)]
struct InventoryGrid;

#[derive(Component, Default, Clone)]
struct Cell {
    kind: u64,
    slot: u32,
}

#[derive(Component, Default, Clone)]
struct SnappingButton;

#[derive(Component, Default, Clone)]
struct DeathBanner;

fn spawn_hud(mut commands: Commands, settings: Res<Settings>, assets: Res<AssetServer>) {
    let mut panels: Vec<Box<dyn Scene>> = vec![Box::new(character_panel(&settings))];
    for pane in Pane::ALL {
        panels.push(Box::new(widget_panel(pane, &settings, &assets)));
    }
    commands.spawn_scene(bsn! {
        Hud
        Node { width: Val::Percent(100.0), height: Val::Percent(100.0) }
        Pickable { should_block_lower: false, is_hoverable: false }
        Children [ {panels} ]
    });
}

fn rebuild_panes(
    open: Res<Open>,
    panes: Query<(Entity, &PaneView, &ChildOf)>,
    settings: Res<Settings>,
    assets: Res<AssetServer>,
    mut commands: Commands,
) {
    if !open.is_changed() {
        return;
    }
    for (entity, view, child_of) in &panes {
        let should_open = open.0.contains(&view.pane);
        if should_open == view.open {
            continue;
        }
        let hud = child_of.parent();
        let pane = view.pane;
        commands.entity(entity).despawn();
        let panel: Box<dyn Scene> = if should_open {
            Box::new(window_panel(pane, &settings))
        } else {
            Box::new(widget_panel(pane, &settings, &assets))
        };
        commands.spawn_scene(panel).insert(ChildOf(hud));
    }
}

fn character_panel(settings: &Settings) -> impl Scene {
    let (pos, _) = resolve(settings, Panel::Character, Vec2::new(8.0, 8.0), None);
    let mut node = panel_node(pos, Vec2::new(140.0, 64.0), false);
    node.padding = UiRect::all(Val::Px(6.0));
    bsn! {
        template_value(node)
        BackgroundColor({PANEL_BG})
        component(BorderColor::all(BORDER))
        DragRoot
        DragHandle
        component(OnSettle::new(move |world, geom| persist(world, Panel::Character, geom)))
        Children [ ( {text_colored(String::new(), Color::WHITE)} CharacterText ) ]
    }
}

fn widget_panel(pane: Pane, settings: &Settings, assets: &AssetServer) -> impl Scene {
    let info = pane.info();
    let (pos, _) = resolve(settings, Panel::Widget(pane), widget_fallback(pane), None);
    let icon = assets.load(info.icon.to_owned());
    let keybind = info.keybind;
    let title = info.title;
    bsn! {
        template_value(panel_node(pos, Vec2::splat(WIDGET.0), false))
        BackgroundColor({PANEL_BG})
        component(BorderColor::all(BORDER))
        {tooltip(false)}
        DragRoot
        DragHandle
        component(OnTap::new(move |world| open_pane(world, pane)))
        component(OnSettle::new(move |world, geom| persist(world, Panel::Widget(pane), geom)))
        component(HoverCursor(POINTER))
        component(PaneView { pane, open: false })
        Children [
            (
                Node { width: Val::Px(32.0), height: Val::Px(32.0), margin: {UiRect::all(Val::Px(8.0))} }
                component(ImageNode::new(icon))
                Pickable { should_block_lower: false, is_hoverable: false }
            ),
            {EntityScene(badge(keybind))},
            (
                {tooltip_content(Side::Bottom, Align::Start, 0.0)}
                Children [ {EntityScene(tooltip_label(title))} ]
            ),
        ]
    }
}

fn window_panel(pane: Pane, settings: &Settings) -> impl Scene {
    let info = pane.info();
    let (pos, size) = resolve(
        settings,
        Panel::Window(pane),
        Vec2::new(376.0, 332.0),
        Some(WINDOW_SIZE),
    );
    let size = size.unwrap_or(WINDOW_SIZE);
    let title = info.title;
    bsn! {
        template_value(panel_node(pos, size, true))
        BackgroundColor({PANEL_BG})
        component(BorderColor::all(BORDER))
        DragRoot
        component(OnSettle::new(move |world, geom| persist(world, Panel::Window(pane), geom)))
        component(PaneView { pane, open: true })
        Children [
            {EntityScene(title_bar(title, pane))},
            {EntityScene(content_area(pane))},
            {EntityScene(resize_grip())},
        ]
    }
}

fn title_bar(title: &'static str, pane: Pane) -> impl Scene {
    bsn! {
        Node {
            width: Val::Percent(100.0),
            height: Val::Px({TITLE_H.0}),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::SpaceBetween,
            padding: {UiRect::horizontal(Val::Px(6.0))},
        }
        BackgroundColor({TITLE_BG})
        DragHandle
        Children [
            {EntityScene(text_colored(title, Color::WHITE))},
            {EntityScene(close_button(pane))},
        ]
    }
}

fn content_area(pane: Pane) -> impl Scene {
    let inner: Box<dyn Scene> = match pane {
        Pane::Inventory => Box::new(bsn! {
            Node {
                width: Val::Percent(100.0),
                flex_wrap: FlexWrap::Wrap,
                align_content: AlignContent::FlexStart,
            }
            InventoryGrid
        }),
        Pane::Settings => Box::new(bsn! {
            {button_styled(button_intent::PRIMARY, ButtonSize::Md, "ui snapping disabled")}
            SnappingButton
            on(|_: On<Activate>, mut commands: Commands| {
                commands.queue(toggle_snapping);
            })
        }),
    };
    bsn! {
        Node {
            flex_grow: 1.0,
            padding: {UiRect::all(Val::Px(4.0))},
        }
        Children [ {EntityScene(inner)} ]
    }
}

fn resize_grip() -> impl Scene {
    bsn! {
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(0.0),
            bottom: Val::Px(0.0),
            width: Val::Px(16.0),
            height: Val::Px(16.0),
        }
        BackgroundColor({BORDER})
        ResizeHandle { min: {MIN_WINDOW} }
        component(HoverCursor(RESIZE))
    }
}

fn close_button(pane: Pane) -> impl Scene {
    bsn! {
        {button_styled(button_intent::PRIMARY, ButtonSize::Icon, "×")}
        component(HoverCursor(POINTER))
        on(move |_: On<Activate>, mut open: ResMut<Open>| {
            open.0.remove(&pane);
        })
    }
}

fn badge(keybind: &'static str) -> impl Scene {
    bsn! {
        Node { position_type: PositionType::Absolute, right: Val::Px(2.0), bottom: Val::Px(2.0) }
        Pickable { should_block_lower: false, is_hoverable: false }
        Children [ {EntityScene(text_colored(keybind, Color::WHITE))} ]
    }
}

fn tooltip_label(text: impl Into<String>) -> impl Scene {
    bsn! {
        Node { padding: {UiRect::axes(Val::Px(6.0), Val::Px(3.0))} }
        BackgroundColor({TOOLTIP_BG})
        Pickable { should_block_lower: false, is_hoverable: false }
        Children [ {EntityScene(text_colored(text.into(), Color::WHITE))} ]
    }
}

fn slot(cell: &CellData) -> impl Scene {
    bsn! {
        Node {
            width: Val::Px({SLOT.0}),
            height: Val::Px({SLOT.0}),
            margin: {UiRect::all(Val::Px(1.0))},
        }
        BackgroundColor({TITLE_BG})
        {tooltip(false)}
        Cell { kind: {cell.kind}, slot: {cell.slot} }
        on(|click: On<Pointer<Click>>, cells: Query<&Cell>, mut commands: Commands| {
            if let Ok(cell) = cells.get(click.entity) {
                let slot = cell.slot;
                commands.queue(move |world: &mut World| session::use_item(world, slot));
            }
        })
        Children [
            (
                Node { width: Val::Px(32.0), height: Val::Px(32.0) }
                component(ImageNode::new(cell.icon.clone()))
                Pickable { should_block_lower: false, is_hoverable: false }
            ),
            (
                {tooltip_content(Side::Bottom, Align::Start, 0.0)}
                Children [ {EntityScene(tooltip_label(cell.name.clone()))} ]
            ),
        ]
    }
}

fn panel_node(pos: Vec2, size: Vec2, window: bool) -> Node {
    let mut node = Node {
        position_type: PositionType::Absolute,
        left: Val::Px(pos.x),
        top: Val::Px(pos.y),
        width: Val::Px(size.x),
        height: Val::Px(size.y),
        border: UiRect::all(Val::Px(1.0)),
        ..default()
    };
    if window {
        node.flex_direction = FlexDirection::Column;
        node.overflow = Overflow::clip();
    }
    node
}

struct CellData {
    icon: Handle<Image>,
    name: String,
    kind: u64,
    slot: u32,
}

fn sync_inventory(world: &mut World) {
    let cells = inventory_cells(world);
    let desired: std::collections::HashSet<u64> = cells.iter().map(|cell| cell.kind).collect();

    let mut grids = world.query_filtered::<Entity, With<InventoryGrid>>();
    let Some(grid) = grids.iter(world).next() else {
        return;
    };

    let existing: Vec<(Entity, u64)> = world
        .get::<Children>(grid)
        .map(|children| children.iter().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|child| world.get::<Cell>(child).map(|cell| (child, cell.kind)))
        .collect();

    for (entity, kind) in &existing {
        if !desired.contains(kind) {
            world.entity_mut(*entity).despawn();
        } else if let Some(cell) = cells.iter().find(|cell| cell.kind == *kind)
            && let Some(mut existing) = world.get_mut::<Cell>(*entity)
        {
            existing.slot = cell.slot;
        }
    }

    let present: std::collections::HashSet<u64> = existing.iter().map(|(_, kind)| *kind).collect();
    for cell in &cells {
        if !present.contains(&cell.kind)
            && let Ok(spawned) = world.spawn_scene(slot(cell))
        {
            let child = spawned.id();
            world.entity_mut(grid).add_child(child);
        }
    }
}

fn inventory_cells(world: &World) -> Vec<CellData> {
    let items = session::me(world)
        .and_then(|me| me.get::<Inventory>())
        .map_or_else(Vec::new, |inventory| inventory.items.clone());
    let assets = world.resource::<AssetServer>();
    items
        .iter()
        .enumerate()
        .map(|(slot, item)| {
            let def = item.get();
            CellData {
                icon: assets.load(def.icon.0.clone()),
                name: def.display_name.clone(),
                kind: item.index() as u64,
                slot: slot as u32,
            }
        })
        .collect()
}

fn sync_character(world: &mut World) {
    let text = character_text(world);
    let mut query = world.query_filtered::<&mut Text, With<CharacterText>>();
    for mut node in query.iter_mut(world) {
        node.0 = text.clone();
    }
}

fn character_text(world: &World) -> String {
    session::me(world).map_or_else(String::new, |me| {
        let (health, max) = me.get::<Vitals>().map_or((0.0, 0.0), |v| (v.health, v.max));
        let name = me
            .get::<Name>()
            .map_or_else(String::new, |n| n.name.clone());
        let xp = me.get::<Xp>().map_or(0, |x| x.amount);
        format!("{name}\n{health:.0} / {max:.0}\nxp {xp}")
    })
}

fn sync_snapping(
    settings: Res<Settings>,
    buttons: Query<&Children, With<SnappingButton>>,
    mut texts: Query<&mut Text>,
) {
    let label = if settings.0.snapping_enabled() {
        "ui snapping enabled"
    } else {
        "ui snapping disabled"
    };
    for children in &buttons {
        for child in children.iter() {
            if let Ok(mut text) = texts.get_mut(child) {
                text.0 = label.to_owned();
            }
        }
    }
}

fn sync_death_banner(world: &mut World) {
    let dead = session::is_dead(world);
    let banner = world
        .query_filtered::<Entity, With<DeathBanner>>()
        .iter(world)
        .next();
    match (dead, banner) {
        (true, None) => {
            if let Some(hud) = world
                .query_filtered::<Entity, With<Hud>>()
                .iter(world)
                .next()
                && let Ok(spawned) = world.spawn_scene(death_banner())
            {
                let banner = spawned.id();
                world.entity_mut(hud).add_child(banner);
            }
        }
        (false, Some(banner)) => world.entity_mut(banner).despawn(),
        _ => {}
    }
}

fn death_banner() -> impl Scene {
    bsn! {
        DeathBanner
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
        }
        GlobalZIndex({50})
        Pickable { should_block_lower: false, is_hoverable: false }
        Children [ {EntityScene(text_colored("You died! Press any key to respawn", Color::WHITE))} ]
    }
}

fn resolve(
    settings: &Settings,
    panel: Panel,
    fallback_pos: Vec2,
    fallback_size: Option<Vec2>,
) -> (Vec2, Option<Vec2>) {
    let placement = settings.0.placement(panel);
    let pos = placement.map_or(fallback_pos, |p| p.pos.to_vec2());
    let size = fallback_size.map(|fallback| {
        placement
            .and_then(|p| p.size)
            .map_or(fallback, ScreenVec::to_vec2)
    });
    (pos, size)
}

// The live drag already renders (and clamps) the snapped geometry, so settling just records it.
fn persist(world: &mut World, panel: Panel, geom: Geom) -> Geom {
    let mut settings = world.resource_mut::<Settings>();
    let pos = ScreenVec::from_vec2(geom.pos);
    let size = panel.resizable().then_some(ScreenVec::from_vec2(geom.size));
    settings.0.set_placement(panel, Placement { pos, size });
    settings.0.save();
    geom
}

fn sync_snap_grid(settings: Res<Settings>, mut grid: ResMut<SnapGrid>) {
    grid.0 = settings.0.snap_grid();
}

fn widget_fallback(pane: Pane) -> Vec2 {
    let x = SCREEN_W - 8.0 - WIDGET.0;
    match pane {
        Pane::Inventory => Vec2::new(x, 8.0),
        Pane::Settings => Vec2::new(x, 16.0 + WIDGET.0),
    }
}

fn open_pane(world: &mut World, pane: Pane) {
    world.resource_mut::<Open>().0.insert(pane);
}

fn toggle_snapping(world: &mut World) {
    let mut settings = world.resource_mut::<Settings>();
    settings.0.toggle_snapping();
    settings.0.save();
}

fn toggle_keys(keys: Res<ButtonInput<KeyCode>>, mut open: ResMut<Open>) {
    for pane in Pane::ALL {
        if keys.just_pressed(pane.info().toggle) && !open.0.remove(&pane) {
            open.0.insert(pane);
        }
    }
}

fn despawn<M: Component>(panels: Query<Entity, With<M>>, mut commands: Commands) {
    for entity in &panels {
        commands.entity(entity).despawn();
    }
}
