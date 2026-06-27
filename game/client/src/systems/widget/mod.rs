//! The in-game HUD: a row of draggable launcher widgets that each open a draggable, titled [`Window`]
//! (inventory, equipment, stats, settings), their layout persisted to [`settings`], plus the [`fps`]
//! readout and the always-on character readout and active-effects row. Each window registers its
//! [`WindowDef`] from its own module; this module iterates the registrations rather than naming any.

pub mod character;
pub mod effects;
pub mod equipment;
pub mod fps;
pub mod inventory;
pub mod settings;
pub mod stats;

use bevy::prelude::*;
use bevy::scene::EntityScene;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ui::{DragHandle, DragRoot, Geom, OnSettle, OnTap, SnapGrid, text_colored, widget};

use crate::systems::widget::character::CharacterText;
use crate::systems::widget::effects::EffectsGrid;
use crate::systems::widget::settings::{Placement, ScreenPx, ScreenVec, UserSettings};
use ui::component;

const WIDGET: ScreenPx = ScreenPx(48.0);
const WINDOW_SIZE: Vec2 = Vec2::new(400.0, 200.0);

const PANEL_BG: Color = Color::srgb(0.1, 0.1, 0.1);
const BORDER: Color = Color::srgb(0.31, 0.31, 0.31);
const TOOLTIP_BG: Color = Color::BLACK;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Settings>()
            .init_resource::<Open>()
            .add_systems(OnEnter(crate::Scene::Area), spawn_hud)
            .add_systems(
                OnExit(crate::Scene::Area),
                crate::systems::despawn_all::<Hud>,
            )
            .add_systems(
                Update,
                (
                    toggle_keys,
                    rebuild_windows,
                    character::sync_character,
                    effects::sync_effects,
                    sync_windows,
                    sync_snap_grid,
                )
                    .run_if(in_state(crate::Scene::Area)),
            );
    }
}

/// A HUD window, identified by the stable id its [`WindowDef`] registers under (also its persisted
/// placement key).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Window(&'static str);

/// Everything the HUD needs to show and drive a window: `content` builds the open window's body, `sync`
/// reconciles it each frame, and `order` is its place down the docked widget column.
pub struct WindowDef {
    pub id: &'static str,
    pub title: &'static str,
    pub toggle: KeyCode,
    pub keybind: &'static str,
    pub icon: &'static str,
    pub order: u32,
    pub content: fn() -> Box<dyn Scene>,
    pub sync: fn(&mut World),
}

::inventory::collect!(WindowDef);

impl Window {
    fn def(self) -> &'static WindowDef {
        ::inventory::iter::<WindowDef>()
            .find(|def| def.id == self.0)
            .expect("a registered window")
    }
}

impl Serialize for Window {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Window {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        ::inventory::iter::<WindowDef>()
            .find(|def| def.id == id)
            .map(|def| Window(def.id))
            .ok_or_else(|| serde::de::Error::custom(format!("unknown window '{id}'")))
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
struct Open(std::collections::HashSet<Window>);

#[derive(Component, Default, Clone)]
struct Hud;

#[derive(Component, Clone)]
struct WindowView {
    window: Window,
    open: bool,
}

fn spawn_hud(
    mut commands: Commands,
    settings: Res<Settings>,
    assets: Res<AssetServer>,
    screen: Single<&bevy::window::Window>,
) {
    let screen_w = screen.resolution.width();
    let mut widgets: Vec<Box<dyn Scene>> = vec![
        Box::new(character_widget(&settings)),
        Box::new(effects_widget(&settings)),
    ];
    for def in ::inventory::iter::<WindowDef>() {
        widgets.push(Box::new(launcher(
            Window(def.id),
            screen_w,
            &settings,
            &assets,
        )));
    }
    commands.spawn_scene(bsn! {
        Hud
        Node { width: Val::Percent(100.0), height: Val::Percent(100.0) }
        Pickable { should_block_lower: false, is_hoverable: false }
        Children [ {widgets} ]
    });
}

fn rebuild_windows(
    open: Res<Open>,
    views: Query<(Entity, &WindowView, &ChildOf)>,
    settings: Res<Settings>,
    assets: Res<AssetServer>,
    screen: Single<&bevy::window::Window>,
    mut commands: Commands,
) {
    if !open.is_changed() {
        return;
    }
    let screen_w = screen.resolution.width();
    for (entity, view, child_of) in &views {
        let should_open = open.0.contains(&view.window);
        if should_open == view.open {
            continue;
        }
        let hud = child_of.parent();
        let window = view.window;
        commands.entity(entity).despawn();
        let panel: Box<dyn Scene> = if should_open {
            Box::new(window_scene(window, &settings))
        } else {
            Box::new(launcher(window, screen_w, &settings, &assets))
        };
        commands.spawn_scene(panel).insert(ChildOf(hud));
    }
}

fn character_widget(settings: &Settings) -> impl Scene {
    let pos = widget_pos(settings, "character", Vec2::new(8.0, 8.0));
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
        component(OnSettle::new(move |world, geom| persist_widget(world, "character", geom)))
        Children [ ( {text_colored(String::new(), Color::WHITE)} CharacterText ) ]
    }
}

/// Always-on row of active-effect icons (those whose effect declares a widget). `sync_effects`
/// reconciles its children; it is empty until the player has a visible effect.
fn effects_widget(settings: &Settings) -> impl Scene {
    let pos = widget_pos(settings, "effects", Vec2::new(8.0, 80.0));
    let node = Node {
        position_type: PositionType::Absolute,
        left: Val::Px(pos.x),
        top: Val::Px(pos.y),
        min_width: Val::Px(WIDGET.0),
        min_height: Val::Px(WIDGET.0 / 2.0),
        padding: UiRect::all(Val::Px(2.0)),
        ..default()
    };
    bsn! {
        template_value(node)
        BackgroundColor({PANEL_BG})
        DragRoot
        DragHandle
        EffectsGrid
        component(OnSettle::new(move |world, geom| persist_widget(world, "effects", geom)))
    }
}

fn launcher(
    window: Window,
    screen_w: f32,
    settings: &Settings,
    assets: &AssetServer,
) -> impl Scene {
    let def = window.def();
    let pos = widget_pos(settings, window.0, launcher_pos(window, screen_w));
    bsn! {
        {widget(ui::WidgetOptions {
            pos,
            icon: assets.load(def.icon.to_owned()),
            badge: def.keybind.to_owned(),
            tooltip: def.title.to_owned(),
            on_tap: OnTap::new(move |world| open_window(world, window)),
            on_settle: OnSettle::new(move |world, geom| persist_widget(world, window.0, geom)),
        })}
        component(WindowView { window, open: false })
    }
}

fn window_scene(window: Window, settings: &Settings) -> impl Scene {
    let def = window.def();
    let (pos, size) = window_geom(settings, window.0, Vec2::new(376.0, 332.0), WINDOW_SIZE);
    bsn! {
        {ui::window(ui::WindowOptions {
            pos,
            size,
            title: def.title.to_owned(),
            on_close: OnTap::new(move |world| close_window(world, window)),
            on_settle: OnSettle::new(move |world, geom| persist_window(world, window.0, geom)),
            content: (def.content)(),
        })}
        component(WindowView { window, open: true })
    }
}

fn close_window(world: &mut World, window: Window) {
    world.resource_mut::<Open>().0.remove(&window);
}

fn sync_windows(world: &mut World) {
    for def in ::inventory::iter::<WindowDef>() {
        (def.sync)(world);
    }
}

fn widget_pos(settings: &Settings, id: &str, fallback: Vec2) -> Vec2 {
    settings.0.widget_pos(id).map_or(fallback, |p| p.to_vec2())
}

fn window_geom(
    settings: &Settings,
    id: &str,
    fallback_pos: Vec2,
    fallback_size: Vec2,
) -> (Vec2, Vec2) {
    let placement = settings.0.window_placement(id);
    let pos = placement.map_or(fallback_pos, |p| p.pos.to_vec2());
    let size = placement.map_or(fallback_size, |p| p.size.to_vec2());
    (pos, size)
}

// The live drag already renders (and clamps) the snapped geometry, so settling just records it.
fn persist_widget(world: &mut World, id: &str, geom: Geom) -> Geom {
    let mut settings = world.resource_mut::<Settings>();
    settings
        .0
        .set_widget_pos(id, ScreenVec::from_vec2(geom.pos));
    settings.0.save();
    geom
}

fn persist_window(world: &mut World, id: &str, geom: Geom) -> Geom {
    let mut settings = world.resource_mut::<Settings>();
    settings.0.set_window_placement(
        id,
        Placement {
            pos: ScreenVec::from_vec2(geom.pos),
            size: ScreenVec::from_vec2(geom.size),
        },
    );
    settings.0.save();
    geom
}

fn sync_snap_grid(settings: Res<Settings>, mut grid: ResMut<SnapGrid>) {
    grid.0 = settings.0.snap_grid();
}

fn launcher_pos(window: Window, screen_w: f32) -> Vec2 {
    let x = screen_w - 8.0 - WIDGET.0;
    Vec2::new(x, 8.0 + window.def().order as f32 * (WIDGET.0 + 8.0))
}

/// Keeps `container`'s keyed children equal to `keys`: when the live keys differ (in value or order)
/// they are despawned and rebuilt from `build`, in order — so dynamic lists (inventory, equipment,
/// effects) stay correct here instead of via a hand-written diff at each call site.
pub(super) fn reconcile_children(
    world: &mut World,
    container: Entity,
    keys: &[u64],
    build: impl Fn(usize) -> Box<dyn Scene>,
) {
    let current: Vec<(Entity, u64)> = world
        .get::<Children>(container)
        .map(|children| {
            children
                .iter()
                .filter_map(|child| world.get::<Keyed>(child).map(|keyed| (child, keyed.0)))
                .collect()
        })
        .unwrap_or_default();
    if current.iter().map(|(_, key)| *key).eq(keys.iter().copied()) {
        return;
    }
    for (entity, _) in current {
        world.entity_mut(entity).despawn();
    }
    for (index, &key) in keys.iter().enumerate() {
        if let Ok(mut spawned) = world.spawn_scene(build(index)) {
            spawned.insert(Keyed(key));
            let child = spawned.id();
            world.entity_mut(container).add_child(child);
        }
    }
}

/// A reconciled child's identity within its list (see [`reconcile_children`]).
#[derive(Component)]
struct Keyed(u64);

pub(super) const SLOT: f32 = 36.0;
pub(super) const SLOT_BG: Color = Color::srgb(0.14, 0.14, 0.14);
pub(super) const SLOT_BORDER: Color = Color::srgb(0.24, 0.24, 0.24);

/// A square cell node shared by the inventory, equipment, and effects grids.
pub(super) fn slot_node() -> Node {
    Node {
        width: Val::Px(SLOT),
        height: Val::Px(SLOT),
        margin: UiRect::all(Val::Px(1.0)),
        border: UiRect::all(Val::Px(1.0)),
        ..default()
    }
}

/// The small black label shown inside a hovered slot/icon's tooltip.
pub(super) fn tooltip_label(text: impl Into<String>) -> impl Scene {
    bsn! {
        Node { padding: {UiRect::axes(Val::Px(6.0), Val::Px(3.0))} }
        BackgroundColor({TOOLTIP_BG})
        Pickable { should_block_lower: false, is_hoverable: false }
        Children [ {EntityScene(text_colored(text.into(), Color::WHITE))} ]
    }
}

fn open_window(world: &mut World, window: Window) {
    world.resource_mut::<Open>().0.insert(window);
}

fn toggle_keys(keys: Res<ButtonInput<KeyCode>>, mut open: ResMut<Open>) {
    for def in ::inventory::iter::<WindowDef>() {
        let window = Window(def.id);
        if keys.just_pressed(def.toggle) && !open.0.remove(&window) {
            open.0.insert(window);
        }
    }
}
