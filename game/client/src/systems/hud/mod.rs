//! The in-game HUD: draggable, dockable panes (a character readout, inventory, and settings) built on
//! the `ui` widget toolkit, their layout persisted to [`settings`]. Each pane's content lives in its
//! own module ([`character`], [`inventory`]); [`death`] overlays the respawn prompt.

pub mod character;
pub mod connection;
pub mod death;
pub mod fps;
pub mod inventory;
pub mod scenes;
pub mod settings;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use ui::button::intent as button_intent;
use ui::{
    Activate, ButtonSize, DragHandle, DragRoot, Geom, OnSettle, OnTap, SnapGrid, Widget, Window,
    button_styled, text_colored, widget, window,
};

use crate::component;
use crate::systems::hud::character::CharacterText;
use crate::systems::hud::inventory::InventoryGrid;
use crate::systems::hud::settings::{Placement, ScreenPx, ScreenVec, UserSettings};

const WIDGET: ScreenPx = ScreenPx(48.0);
const SCREEN_W: f32 = 1152.0;
const WINDOW_SIZE: Vec2 = Vec2::new(400.0, 200.0);

const PANEL_BG: Color = Color::srgb(0.1, 0.1, 0.1);
const BORDER: Color = Color::srgb(0.31, 0.31, 0.31);

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Settings>()
            .init_resource::<Open>()
            .add_systems(OnEnter(crate::GameScene::Playing), spawn_hud)
            .add_systems(OnExit(crate::GameScene::Playing), despawn::<Hud>)
            .add_systems(
                Update,
                (
                    toggle_keys,
                    rebuild_panes,
                    character::sync_character,
                    inventory::sync_inventory,
                    sync_snapping,
                    sync_snap_grid,
                    death::sync_death_banner,
                )
                    .run_if(in_state(crate::GameScene::Playing)),
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
struct SnappingButton;

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
    let node = Node {
        position_type: PositionType::Absolute,
        left: Val::Px(pos.x),
        top: Val::Px(pos.y),
        width: Val::Px(140.0),
        height: Val::Px(64.0),
        border: UiRect::all(Val::Px(1.0)),
        padding: UiRect::all(Val::Px(6.0)),
        ..default()
    };
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
    bsn! {
        {widget(Widget {
            pos,
            icon: assets.load(info.icon.to_owned()),
            badge: info.keybind.to_owned(),
            tooltip: info.title.to_owned(),
            on_tap: OnTap::new(move |world| open_pane(world, pane)),
            on_settle: OnSettle::new(move |world, geom| persist(world, Panel::Widget(pane), geom)),
        })}
        component(PaneView { pane, open: false })
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
    bsn! {
        {window(Window {
            pos,
            size,
            title: info.title.to_owned(),
            on_close: OnTap::new(move |world| close_pane(world, pane)),
            on_settle: OnSettle::new(move |world, geom| persist(world, Panel::Window(pane), geom)),
            content: content(pane),
        })}
        component(PaneView { pane, open: true })
    }
}

fn content(pane: Pane) -> Box<dyn Scene> {
    match pane {
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
    }
}

fn close_pane(world: &mut World, pane: Pane) {
    world.resource_mut::<Open>().0.remove(&pane);
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
