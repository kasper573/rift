use bevy::prelude::*;
use bevy::window::{CursorIcon, SystemCursorIcon};
use bevy_view::{Geom, View, ViewRoot, draggable, each, resizable, view};
use serde::{Deserialize, Serialize};
use ui::{
    Button as UiButton, Text, Tooltip, TooltipContent, TooltipOutlet, TooltipProvider,
    TooltipTrigger,
};
use world::protocol::{Inventory, Name, Vitals, Xp};
use world::session;

use crate::Screen;
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
            .add_systems(Update, toggle_keys.run_if(in_state(Screen::Playing)));
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

#[derive(Component)]
struct Hud;

fn spawn_hud(mut commands: Commands) {
    commands.spawn((
        Hud,
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        Pickable::IGNORE,
        ViewRoot::new(hud),
    ));
}

fn hud(world: &World) -> View {
    view! {
        <TooltipProvider>
            { character_widget(world) }
            { pane(world, Pane::Inventory) }
            { pane(world, Pane::Settings) }
        </TooltipProvider>
        { death_banner(world) }
        <TooltipOutlet/>
    }
}

fn character_widget(world: &World) -> View {
    let (pos, _) = resolve(world, Panel::Character, Vec2::new(8.0, 8.0), None);
    let drag = draggable()
        .initial(pos)
        .on_settle(move |w, geom| persist(w, Panel::Character, geom));
    view! {
        <node use={drag.whole()}
            position_type=PositionType::Absolute
            width=Val::Px(140.0)
            height=Val::Px(64.0)
            border=UiRect::all(Val::Px(1.0))
            padding=UiRect::all(Val::Px(6.0))
            insert={(BackgroundColor(PANEL_BG), BorderColor::all(BORDER))}
        >
            { Text::dynamic(|w: &World| character_text(w)).color(Color::WHITE) }
        </node>
    }
}

fn pane(world: &World, pane: Pane) -> View {
    if world.resource::<Open>().0.contains(&pane) {
        window(world, pane)
    } else {
        widget(world, pane)
    }
}

fn widget(world: &World, pane: Pane) -> View {
    let info = pane.info();
    let (pos, _) = resolve(world, Panel::Widget(pane), widget_fallback(pane), None);
    let icon = world.resource::<AssetServer>().load(info.icon.to_owned());
    let drag = draggable()
        .initial(pos)
        .on_tap(move |w| open_pane(w, pane))
        .on_settle(move |w, geom| persist(w, Panel::Widget(pane), geom));
    view! {
        <Tooltip>
            <TooltipTrigger>
                <node use={drag.whole()}
                    position_type=PositionType::Absolute
                    width=Val::Px(WIDGET.0)
                    height=Val::Px(WIDGET.0)
                    border=UiRect::all(Val::Px(1.0))
                    cursor={POINTER}
                    insert={(BackgroundColor(PANEL_BG), BorderColor::all(BORDER))}
                >
                    <image src={icon}
                        width=Val::Px(32.0)
                        height=Val::Px(32.0)
                        margin=UiRect::all(Val::Px(8.0))
                        insert={Pickable::IGNORE}
                    />
                    { badge(info.keybind) }
                </node>
            </TooltipTrigger>
            <TooltipContent>{ tooltip_label(info.title) }</TooltipContent>
        </Tooltip>
    }
}

fn window(world: &World, pane: Pane) -> View {
    let info = pane.info();
    let (pos, size) = resolve(
        world,
        Panel::Window(pane),
        Vec2::new(376.0, 332.0),
        Some(WINDOW_SIZE),
    );
    let drag = draggable()
        .initial(pos)
        .initial_size(size.unwrap_or(WINDOW_SIZE))
        .on_settle(move |w, geom| persist(w, Panel::Window(pane), geom));
    let resize = resizable().min(MIN_WINDOW);
    view! {
        <node use={drag.root()}
            position_type=PositionType::Absolute
            flex_direction=FlexDirection::Column
            overflow=Overflow::clip()
            border=UiRect::all(Val::Px(1.0))
            insert={(BackgroundColor(PANEL_BG), BorderColor::all(BORDER))}
        >
            <node use={drag.handle()}
                width=Val::Percent(100.0)
                height=Val::Px(TITLE_H.0)
                align_items=AlignItems::Center
                justify_content=JustifyContent::SpaceBetween
                padding=UiRect::horizontal(Val::Px(6.0))
                insert={BackgroundColor(TITLE_BG)}
            >
                { Text::new(info.title.to_owned()).color(Color::WHITE) }
                { close_button(pane) }
            </node>
            <node
                flex_grow=1.0
                flex_wrap=FlexWrap::Wrap
                align_content=AlignContent::FlexStart
                padding=UiRect::all(Val::Px(4.0))
            >
                { window_content(world, pane) }
            </node>
            <node use={resize.handle()}
                position_type=PositionType::Absolute
                right=Val::Px(0.0)
                bottom=Val::Px(0.0)
                width=Val::Px(16.0)
                height=Val::Px(16.0)
                cursor={RESIZE}
                insert={BackgroundColor(BORDER)}
            />
        </node>
    }
}

fn window_content(world: &World, pane: Pane) -> View {
    match pane {
        Pane::Inventory => inventory_grid(),
        Pane::Settings => snapping_button(world),
    }
}

fn inventory_grid() -> View {
    each(
        |world: &World| inventory_cells(world),
        // Key by item kind, not slot: keying by kind makes a cell follow its item, so using one removes its own cell, not a neighbour's.
        |cell: &Cell| cell.kind,
        |cell: &Cell| inventory_slot(cell),
    )
}

struct Cell {
    icon: Handle<Image>,
    name: String,
    kind: u64,
    slot: u32,
}

fn inventory_cells(world: &World) -> Vec<Cell> {
    let items = session::me(world)
        .and_then(|me| me.get::<Inventory>())
        .map_or_else(Vec::new, |inventory| inventory.items.clone());
    let assets = world.resource::<AssetServer>();
    items
        .iter()
        .enumerate()
        .map(|(slot, item)| {
            let def = item.get();
            Cell {
                icon: assets.load(def.icon.0.clone()),
                name: def.display_name.clone(),
                kind: item.index() as u64,
                slot: slot as u32,
            }
        })
        .collect()
}

fn inventory_slot(cell: &Cell) -> View {
    let icon = cell.icon.clone();
    let name = cell.name.clone();
    let slot = cell.slot;
    view! {
        <Tooltip>
            <TooltipTrigger>
                <node
                    width=Val::Px(SLOT.0)
                    height=Val::Px(SLOT.0)
                    margin=UiRect::all(Val::Px(1.0))
                    insert={BackgroundColor(TITLE_BG)}
                    on:click={move |w: &mut World| session::use_item(w, slot)}
                >
                    <image src={icon}
                        width=Val::Px(32.0)
                        height=Val::Px(32.0)
                        insert={Pickable::IGNORE}
                    />
                </node>
            </TooltipTrigger>
            <TooltipContent>{ tooltip_label(name) }</TooltipContent>
        </Tooltip>
    }
}

fn snapping_button(world: &World) -> View {
    let enabled = world.resource::<Settings>().0.snapping_enabled();
    let label = if enabled {
        "ui snapping enabled"
    } else {
        "ui snapping disabled"
    };
    let variant = if enabled { "primary" } else { "surface" };
    view! {
        <UiButton
            variant={variant}
            label={label}
            on:click={move |w: &mut World| toggle_snapping(w)}
        />
    }
}

fn close_button(pane: Pane) -> View {
    view! {
        <UiButton
            variant="text"
            label="×"
            insert={Node {
                width: Val::Px(16.0),
                height: Val::Px(16.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            }}
            cursor={POINTER}
            on:click={move |w: &mut World| { w.resource_mut::<Open>().0.remove(&pane); }}
        />
    }
}

fn badge(keybind: &str) -> View {
    let keybind = keybind.to_owned();
    view! {
        <node
            position_type=PositionType::Absolute
            right=Val::Px(2.0)
            bottom=Val::Px(2.0)
            insert={Pickable::IGNORE}
        >
            { Text::new(keybind).intent("label_small").color(Color::WHITE) }
        </node>
    }
}

fn tooltip_label(text: impl Into<String>) -> View {
    let text = text.into();
    view! {
        <node
            padding=UiRect::axes(Val::Px(6.0), Val::Px(3.0))
            insert={(BackgroundColor(TOOLTIP_BG), Pickable::IGNORE)}
        >
            { Text::new(text).color(Color::WHITE) }
        </node>
    }
}

fn death_banner(world: &World) -> View {
    if !session::is_dead(world) {
        return View::empty();
    }
    view! {
        <node
            position_type=PositionType::Absolute
            width=Val::Percent(100.0)
            height=Val::Percent(100.0)
            align_items=AlignItems::Center
            justify_content=JustifyContent::Center
            insert={(GlobalZIndex(50), Pickable::IGNORE)}
        >
            { Text::new("You died! Press any key to respawn").intent("headline_medium").color(Color::WHITE) }
        </node>
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

fn resolve(
    world: &World,
    panel: Panel,
    fallback_pos: Vec2,
    fallback_size: Option<Vec2>,
) -> (Vec2, Option<Vec2>) {
    let placement = world.resource::<Settings>().0.placement(panel);
    let pos = placement.map_or(fallback_pos, |p| p.pos.to_vec2());
    let size = fallback_size.map(|fallback| {
        placement
            .and_then(|p| p.size)
            .map_or(fallback, ScreenVec::to_vec2)
    });
    (pos, size)
}

fn persist(world: &mut World, panel: Panel, geom: Geom) -> Geom {
    let mut settings = world.resource_mut::<Settings>();
    let snapped = snap(&settings.0, geom, panel.resizable());
    let pos = ScreenVec::from_vec2(snapped.pos);
    let size = panel
        .resizable()
        .then_some(ScreenVec::from_vec2(snapped.size));
    settings.0.set_placement(panel, Placement { pos, size });
    settings.0.save();
    snapped
}

fn snap(settings: &UserSettings, geom: Geom, resizable: bool) -> Geom {
    let snap = |value: f32| settings.snap(ScreenPx(value)).0;
    let pos = Vec2::new(snap(geom.pos.x), snap(geom.pos.y));
    let size = if resizable {
        Vec2::new(
            snap(geom.size.x).max(MIN_WINDOW.x),
            snap(geom.size.y).max(MIN_WINDOW.y),
        )
    } else {
        geom.size
    };
    Geom { pos, size }
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
