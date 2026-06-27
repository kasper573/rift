pub mod character;
pub mod effects;
pub mod equipment;
pub mod inventory;
pub mod settings;
pub mod stats;

use bevy::prelude::*;
use bevy::scene::EntityScene;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ui::component;
use ui::{Geom, OnSettle, OnTap, SnapGrid, text_colored, widget};

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
                    sync_widgets,
                    sync_windows,
                    sync_snap_grid,
                )
                    .run_if(in_state(crate::Scene::Area)),
            );
    }
}

pub struct Widget {
    pub id: &'static str,
    pub fallback: Vec2,
    pub build: fn(Vec2, &'static str) -> Box<dyn Scene>,
    pub sync: fn(&mut World),
}

::inventory::collect!(Widget);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(&'static str);

pub struct Window {
    pub id: &'static str,
    pub title: &'static str,
    pub toggle: KeyCode,
    pub keybind: &'static str,
    pub icon: &'static str,
    pub order: u32,
    pub content: fn() -> Box<dyn Scene>,
    pub sync: fn(&mut World),
}

::inventory::collect!(Window);

impl WindowId {
    fn def(self) -> &'static Window {
        ::inventory::iter::<Window>()
            .find(|def| def.id == self.0)
            .expect("a registered window")
    }
}

impl Serialize for WindowId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for WindowId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        ::inventory::iter::<Window>()
            .find(|def| def.id == id)
            .map(|def| WindowId(def.id))
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

impl Settings {
    fn snapping_enabled(&self) -> bool {
        self.0.snapping_enabled()
    }

    fn toggle_snapping(&mut self) {
        self.0.toggle_snapping();
        self.0.save();
    }
}

const KEY: &str = "rift.user_settings";
const DEFAULT_SNAP: ScreenPx = ScreenPx(16.0);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
struct ScreenPx(f32);

#[derive(Serialize, Deserialize, Clone, Copy)]
struct ScreenVec {
    x: ScreenPx,
    y: ScreenPx,
}

impl ScreenVec {
    fn to_vec2(self) -> Vec2 {
        Vec2::new(self.x.0, self.y.0)
    }

    fn from_vec2(v: Vec2) -> ScreenVec {
        ScreenVec {
            x: ScreenPx(v.x),
            y: ScreenPx(v.y),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy)]
struct Placement {
    pos: ScreenVec,
    size: ScreenVec,
}

#[derive(Serialize, Deserialize, Default)]
struct UserSettings {
    #[serde(default)]
    ui: UiSettings,
}

#[derive(Serialize, Deserialize)]
struct UiSettings {
    #[serde(default = "default_snap")]
    snap: ScreenPx,
    #[serde(default)]
    widgets: Vec<(String, ScreenVec)>,
    #[serde(default)]
    windows: Vec<(String, Placement)>,
}

impl UserSettings {
    fn load() -> UserSettings {
        crate::core::platform::load(KEY)
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            crate::core::platform::save(KEY, &json);
        }
    }

    fn snap_grid(&self) -> f32 {
        self.ui.snap.0
    }

    fn widget_pos(&self, id: &str) -> Option<ScreenVec> {
        self.ui
            .widgets
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, pos)| *pos)
    }

    fn set_widget_pos(&mut self, id: &str, pos: ScreenVec) {
        match self.ui.widgets.iter_mut().find(|(key, _)| key == id) {
            Some(entry) => entry.1 = pos,
            None => self.ui.widgets.push((id.to_owned(), pos)),
        }
    }

    fn window_placement(&self, id: &str) -> Option<Placement> {
        self.ui
            .windows
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, placement)| *placement)
    }

    fn set_window_placement(&mut self, id: &str, placement: Placement) {
        match self.ui.windows.iter_mut().find(|(key, _)| key == id) {
            Some(entry) => entry.1 = placement,
            None => self.ui.windows.push((id.to_owned(), placement)),
        }
    }

    fn snapping_enabled(&self) -> bool {
        self.ui.snap.0 > 0.0
    }

    fn toggle_snapping(&mut self) {
        self.ui.snap = if self.ui.snap.0 > 0.0 {
            ScreenPx(0.0)
        } else {
            DEFAULT_SNAP
        };
    }
}

impl Default for UiSettings {
    fn default() -> UiSettings {
        UiSettings {
            snap: default_snap(),
            widgets: Vec::new(),
            windows: Vec::new(),
        }
    }
}

fn default_snap() -> ScreenPx {
    DEFAULT_SNAP
}

#[derive(Resource, Default)]
struct Open(std::collections::HashSet<WindowId>);

#[derive(Component, Default, Clone)]
struct Hud;

#[derive(Component, Clone)]
struct WindowView {
    window: WindowId,
    open: bool,
}

fn spawn_hud(
    mut commands: Commands,
    settings: Res<Settings>,
    assets: Res<AssetServer>,
    screen: Single<&bevy::window::Window>,
) {
    let screen_w = screen.resolution.width();
    let mut scenes: Vec<Box<dyn Scene>> = Vec::new();
    for def in ::inventory::iter::<Widget>() {
        let pos = widget_pos(&settings, def.id, def.fallback);
        scenes.push((def.build)(pos, def.id));
    }
    for def in ::inventory::iter::<Window>() {
        scenes.push(Box::new(launcher(
            WindowId(def.id),
            screen_w,
            &settings,
            &assets,
        )));
    }
    commands.spawn_scene(bsn! {
        Hud
        Node { width: Val::Percent(100.0), height: Val::Percent(100.0) }
        Pickable { should_block_lower: false, is_hoverable: false }
        Children [ {scenes} ]
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

fn sync_widgets(world: &mut World) {
    for def in ::inventory::iter::<Widget>() {
        (def.sync)(world);
    }
}

fn launcher(
    window: WindowId,
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

fn window_scene(window: WindowId, settings: &Settings) -> impl Scene {
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

fn close_window(world: &mut World, window: WindowId) {
    world.resource_mut::<Open>().0.remove(&window);
}

fn sync_windows(world: &mut World) {
    for def in ::inventory::iter::<Window>() {
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

fn launcher_pos(window: WindowId, screen_w: f32) -> Vec2 {
    let x = screen_w - 8.0 - WIDGET.0;
    Vec2::new(x, 8.0 + window.def().order as f32 * (WIDGET.0 + 8.0))
}

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

#[derive(Component)]
struct Keyed(u64);

pub(super) const SLOT: f32 = 36.0;
pub(super) const SLOT_BG: Color = Color::srgb(0.14, 0.14, 0.14);
pub(super) const SLOT_BORDER: Color = Color::srgb(0.24, 0.24, 0.24);

pub(super) fn slot_node() -> Node {
    Node {
        width: Val::Px(SLOT),
        height: Val::Px(SLOT),
        margin: UiRect::all(Val::Px(1.0)),
        border: UiRect::all(Val::Px(1.0)),
        ..default()
    }
}

pub(super) fn tooltip_label(text: impl Into<String>) -> impl Scene {
    bsn! {
        Node { padding: {UiRect::axes(Val::Px(6.0), Val::Px(3.0))} }
        BackgroundColor({TOOLTIP_BG})
        Pickable { should_block_lower: false, is_hoverable: false }
        Children [ {EntityScene(text_colored(text.into(), Color::WHITE))} ]
    }
}

fn open_window(world: &mut World, window: WindowId) {
    world.resource_mut::<Open>().0.insert(window);
}

fn toggle_keys(keys: Res<ButtonInput<KeyCode>>, mut open: ResMut<Open>) {
    for def in ::inventory::iter::<Window>() {
        let window = WindowId(def.id);
        if keys.just_pressed(def.toggle) && !open.0.remove(&window) {
            open.0.insert(window);
        }
    }
}
