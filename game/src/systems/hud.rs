use bevy::prelude::*;
use bevy::scene::EntityScene;
use serde::{Deserialize, Serialize};
use ui::component;
use ui::{Geom, OnSettle, OnTap, SnapGrid, text_colored, widget};

use crate::core::platform::{ClientPlatform, Platform};
use crate::systems::input::map::{ActionInput, InputAction};
use crate::systems::scene::mode::Mode;
use crate::systems::{effect, equipment, item, player, quest, settings, spectate, stat, terminal};

pub(crate) const WIDGET: ScreenPx = ScreenPx(48.0);
const WINDOW_SIZE: Vec2 = Vec2::new(400.0, 200.0);

pub(crate) const PANEL_BG: Color = Color::srgb(0.1, 0.1, 0.1);
pub(crate) const BORDER: Color = Color::srgb(0.31, 0.31, 0.31);
const TOOLTIP_BG: Color = Color::BLACK;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Settings>()
            .init_resource::<Open>()
            .init_resource::<RefreshWindows>()
            .add_systems(Update, (persist_settings, sync_preferences))
            .add_systems(OnEnter(crate::systems::scene::Scene::Area), spawn_hud)
            .add_systems(
                OnExit(crate::systems::scene::Scene::Area),
                crate::systems::scene::despawn_all::<Hud>,
            )
            .add_systems(
                Update,
                (
                    toggle_keys.run_if(not(ui::typing)),
                    rebuild_windows,
                    sync_widgets,
                    sync_windows,
                )
                    .run_if(in_state(crate::systems::scene::Scene::Area)),
            );
    }
}

/// Open windows to rebuild because their contents derive from state that changed (not from `Open`).
#[derive(Resource, Default)]
pub(crate) struct RefreshWindows(pub std::collections::HashSet<&'static str>);

pub(crate) const TERMINAL_WINDOW: &str = "Terminal";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HudAudience {
    Players,
    Spectators,
    Everyone,
}

impl HudAudience {
    fn includes(self, mode: Mode) -> bool {
        match self {
            HudAudience::Players => mode == Mode::Play,
            HudAudience::Spectators => mode == Mode::Spectate,
            HudAudience::Everyone => true,
        }
    }
}

pub trait Widget: Send + Sync {
    fn audience(&self) -> HudAudience;
    fn fallback(&self, screen_w: f32) -> Vec2;
    fn build(&self, pos: Vec2, id: &'static str) -> Box<dyn Scene>;
    fn sync(&self, world: &mut World);
}

pub trait Window: Send + Sync {
    fn audience(&self) -> HudAudience;
    fn title(&self) -> &'static str;
    fn toggle(&self) -> InputAction;
    fn icon(&self) -> &'static str;
    fn order(&self) -> u32;
    fn size(&self) -> Vec2 {
        WINDOW_SIZE
    }
    fn contents(&self, world: &World) -> Vec<ui::WindowContent>;
    fn sync(&self, world: &mut World);
}

pub(crate) fn single_tab(title: &str, scene: impl Scene + 'static) -> Vec<ui::WindowContent> {
    vec![ui::WindowContent {
        title: title.into(),
        scene: Box::new(scene),
    }]
}

pub(crate) const SLOT: f32 = 36.0;
pub(crate) const SLOT_BG: Color = Color::srgb(0.14, 0.14, 0.14);
pub(crate) const SLOT_BORDER: Color = Color::srgb(0.24, 0.24, 0.24);

pub(crate) fn slot_node() -> Node {
    Node {
        width: Val::Px(SLOT),
        height: Val::Px(SLOT),
        margin: UiRect::all(Val::Px(1.0)),
        border: UiRect::all(Val::Px(1.0)),
        ..default()
    }
}

pub(crate) fn tooltip_label(text: impl Into<String>) -> impl Scene {
    bsn! {
        Node { padding: {UiRect::axes(Val::Px(6.0), Val::Px(3.0))} }
        BackgroundColor({TOOLTIP_BG})
        Pickable { should_block_lower: false, is_hoverable: false }
        Children [ {EntityScene(text_colored(text.into(), Color::WHITE))} ]
    }
}

pub(crate) fn reconcile_children(
    world: &mut World,
    container: Entity,
    keys: &[u64],
    build: impl Fn(&World, usize) -> Box<dyn Scene>,
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
    for (index, &key) in keys.iter().enumerate() {
        let have = current.get(index);
        if have.is_some_and(|&(_, have)| have == key) {
            continue;
        }
        if let Some(&(stale, _)) = have {
            world.entity_mut(stale).despawn();
        }
        let scene = build(world, index);
        if let Ok(mut spawned) = world.spawn_scene(scene) {
            spawned.insert(Keyed(key));
            let child = spawned.id();
            world.entity_mut(container).insert_children(index, &[child]);
        }
    }
    for &(extra, _) in current.iter().skip(keys.len()) {
        world.entity_mut(extra).despawn();
    }
}

#[derive(Component)]
struct Keyed(u64);

static WIDGETS: &[(&str, &dyn Widget)] = &[
    ("character", &player::widget::CharacterWidget),
    ("effects", &effect::widget::EffectsWidget),
    ("quests", &quest::tracker::QuestTrackerWidget),
    ("spectator", &spectate::widget::SpectatorWidget),
];

static WINDOWS: &[(&str, &dyn Window)] = &[
    ("Inventory", &item::widget::InventoryWindow),
    ("Equipment", &equipment::widget::EquipmentWindow),
    ("Stats", &stat::widget::StatsWindow),
    ("Quests", &quest::log::QuestLogWindow),
    ("Settings", &settings::SettingsWindow),
    (TERMINAL_WINDOW, &terminal::widget::TerminalWindow),
];

fn widgets(mode: Mode) -> impl Iterator<Item = (&'static str, &'static dyn Widget)> {
    WIDGETS
        .iter()
        .copied()
        .filter(move |(_, widget)| widget.audience().includes(mode))
}

fn windows(mode: Mode) -> impl Iterator<Item = (&'static str, &'static dyn Window)> {
    WINDOWS
        .iter()
        .copied()
        .filter(move |(_, window)| window.audience().includes(mode))
}

fn window_def(id: &str) -> &'static dyn Window {
    WINDOWS
        .iter()
        .find(|(name, _)| *name == id)
        .map(|(_, window)| *window)
        .expect("a registered window")
}

#[derive(Resource)]
pub(crate) struct Settings(UserSettings);

impl FromWorld for Settings {
    fn from_world(world: &mut World) -> Settings {
        Settings(UserSettings::load(&*world.resource::<ClientPlatform>().0))
    }
}

impl Settings {
    pub(crate) fn snapping_enabled(&self) -> bool {
        self.0.snapping_enabled()
    }

    pub(crate) fn toggle_snapping(&mut self) {
        self.0.toggle_snapping();
    }

    pub(crate) fn text_speed(&self) -> TextSpeed {
        self.0.text.speed
    }

    pub(crate) fn cycle_text_speed(&mut self) {
        self.0.text.speed = self.0.text.speed.next();
    }

    pub(crate) fn reduced_motion(&self) -> bool {
        self.0.text.reduced_motion
    }

    pub(crate) fn toggle_reduced_motion(&mut self) {
        self.0.text.reduced_motion = !self.0.text.reduced_motion;
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TextSpeed {
    Slow,
    #[default]
    Normal,
    Fast,
    Instant,
}

impl TextSpeed {
    pub(crate) fn label(self) -> &'static str {
        match self {
            TextSpeed::Slow => "slow",
            TextSpeed::Normal => "normal",
            TextSpeed::Fast => "fast",
            TextSpeed::Instant => "instant",
        }
    }

    fn next(self) -> TextSpeed {
        match self {
            TextSpeed::Slow => TextSpeed::Normal,
            TextSpeed::Normal => TextSpeed::Fast,
            TextSpeed::Fast => TextSpeed::Instant,
            TextSpeed::Instant => TextSpeed::Slow,
        }
    }

    fn typewriter(self) -> ui::TypewriterSpeed {
        ui::TypewriterSpeed(match self {
            TextSpeed::Slow => Some(25.0),
            TextSpeed::Normal => Some(45.0),
            TextSpeed::Fast => Some(90.0),
            TextSpeed::Instant => None,
        })
    }
}

/// Persists settings on change instead of at each mutation site. The initial load also registers as
/// a change, harmlessly rewriting what was just read.
fn persist_settings(settings: Res<Settings>, platform: Res<ClientPlatform>) {
    if settings.is_changed() {
        settings.0.save(&*platform.0);
    }
}

const KEY: &str = "rift.user_settings";
const DEFAULT_SNAP: ScreenPx = ScreenPx(16.0);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub(crate) struct ScreenPx(pub(crate) f32);

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
    #[serde(default)]
    text: TextSettings,
}

#[derive(Serialize, Deserialize, Default)]
struct TextSettings {
    #[serde(default)]
    speed: TextSpeed,
    #[serde(default)]
    reduced_motion: bool,
}

#[derive(Serialize, Deserialize)]
struct UiSettings {
    #[serde(default = "default_snap")]
    snap: Option<ScreenPx>,
    #[serde(default)]
    widgets: Vec<(String, ScreenVec)>,
    #[serde(default)]
    windows: Vec<(String, Placement)>,
}

impl UserSettings {
    fn load(platform: &dyn Platform) -> UserSettings {
        let mut settings: UserSettings = platform
            .load(KEY)
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        settings.ui.snap = settings.ui.snap.filter(|snap| snap.0 > 0.0);
        settings
    }

    fn save(&self, platform: &dyn Platform) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            platform.save(KEY, &json);
        }
    }

    fn snap_grid(&self) -> Option<f32> {
        self.ui.snap.map(|snap| snap.0)
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

    pub(crate) fn snapping_enabled(&self) -> bool {
        self.ui.snap.is_some()
    }

    pub(crate) fn toggle_snapping(&mut self) {
        self.ui.snap = match self.ui.snap {
            Some(_) => None,
            None => Some(DEFAULT_SNAP),
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

fn default_snap() -> Option<ScreenPx> {
    Some(DEFAULT_SNAP)
}

#[derive(Resource, Default)]
struct Open(std::collections::HashSet<&'static str>);

#[derive(Component, Default, Clone)]
struct Hud;

#[derive(Component, Clone)]
struct WindowView {
    window: &'static str,
    open: bool,
}

fn spawn_hud(
    mut commands: Commands,
    mode: Res<Mode>,
    settings: Res<Settings>,
    assets: Res<AssetServer>,
    screen: Single<&bevy::window::Window>,
) {
    let screen_w = screen.resolution.width();
    let mut scenes: Vec<Box<dyn Scene>> = Vec::new();
    for (name, widget) in widgets(*mode) {
        let pos = widget_pos(&settings, name, widget.fallback(screen_w));
        scenes.push(widget.build(pos, name));
    }
    for (window, _) in windows(*mode) {
        scenes.push(Box::new(launcher(
            window, *mode, screen_w, &settings, &assets,
        )));
    }
    commands.spawn_scene(bsn! {
        Hud
        Node { width: Val::Percent(100.0), height: Val::Percent(100.0) }
        Pickable { should_block_lower: false, is_hoverable: false }
        Children [ {scenes} ]
    });
}

fn rebuild_windows(world: &mut World) {
    let refresh = match world.resource::<RefreshWindows>().0.is_empty() {
        true => std::collections::HashSet::new(),
        false => std::mem::take(&mut world.resource_mut::<RefreshWindows>().0),
    };
    if !world.is_resource_changed::<Open>() && refresh.is_empty() {
        return;
    }
    let Ok(screen) = world
        .query::<&bevy::window::Window>()
        .single(world)
        .map(|screen| screen.resolution.size())
    else {
        return;
    };
    let views: Vec<(Entity, WindowView, Entity)> = world
        .query::<(Entity, &WindowView, &ChildOf)>()
        .iter(world)
        .map(|(entity, view, child_of)| (entity, view.clone(), child_of.parent()))
        .collect();
    for (entity, view, hud) in views {
        let should_open = world.resource::<Open>().0.contains(&view.window);
        if should_open == view.open && !(view.open && refresh.contains(&view.window)) {
            continue;
        }
        let window = view.window;
        let panel: Box<dyn Scene> = if should_open {
            Box::new(window_scene(world, window, screen))
        } else {
            let mode = *world.resource::<Mode>();
            let settings = world.resource::<Settings>();
            let assets = world.resource::<AssetServer>();
            Box::new(launcher(window, mode, screen.x, settings, assets))
        };
        world.entity_mut(entity).despawn();
        if let Ok(mut spawned) = world.spawn_scene(panel) {
            spawned.insert(ChildOf(hud));
        }
    }
}

fn sync_widgets(world: &mut World) {
    for (_, widget) in widgets(*world.resource::<Mode>()) {
        widget.sync(world);
    }
}

fn launcher(
    window: &'static str,
    mode: Mode,
    screen_w: f32,
    settings: &Settings,
    assets: &AssetServer,
) -> impl Scene {
    let def = window_def(window);
    let pos = widget_pos(settings, window, launcher_pos(window, mode, screen_w));
    bsn! {
        {widget(ui::WidgetOptions {
            pos,
            icon: assets.load(def.icon().to_owned()),
            badge: Some(def.toggle().into()),
            tooltip: def.title().to_owned(),
            on_tap: OnTap::new(move |world| open_window(world, window)),
            on_settle: OnSettle::new(move |world, geom| persist_widget(world, window, geom)),
        })}
        component(WindowView { window, open: false })
    }
}

fn window_scene(world: &World, window: &'static str, screen: Vec2) -> impl Scene {
    let def = window_def(window);
    let centered = ((screen - def.size()) / 2.0).max(Vec2::splat(8.0));
    bsn! {
        {placed_window(
            world,
            window,
            (centered, def.size()),
            OnTap::new(move |world| close_window(world, window)),
            def.contents(world),
        )}
        component(WindowView { window, open: true })
    }
}

pub(crate) fn spawn_in_hud(world: &mut World, scene: impl Scene) -> Option<Entity> {
    let hud = world
        .query_filtered::<Entity, With<Hud>>()
        .iter(world)
        .next()?;
    let mut spawned = world.spawn_scene(scene).ok()?;
    spawned.insert(ChildOf(hud));
    Some(spawned.id())
}

pub(crate) fn placed_window(
    world: &World,
    id: &'static str,
    (fallback_pos, fallback_size): (Vec2, Vec2),
    on_close: OnTap,
    content: Vec<ui::WindowContent>,
) -> impl Scene + use<> {
    let settings = world.resource::<Settings>();
    let (pos, size) = window_geom(settings, id, fallback_pos, fallback_size);
    ui::window(ui::WindowOptions {
        frame: ui::WindowFrame::Floating {
            pos,
            size,
            on_close,
            on_settle: OnSettle::new(move |world, geom| persist_window(world, id, geom)),
        },
        content,
    })
}

fn close_window(world: &mut World, window: &'static str) {
    world.resource_mut::<Open>().0.remove(&window);
}

pub(crate) fn close_topmost_window(world: &mut World) -> bool {
    let open: Vec<Entity> = world
        .query::<(Entity, &WindowView)>()
        .iter(world)
        .filter(|(_, view)| view.open)
        .map(|(entity, _)| entity)
        .collect();
    let Some(topmost) = ui::topmost(world, open) else {
        return false;
    };
    let window = world.get::<WindowView>(topmost).map(|view| view.window);
    if let Some(window) = window {
        close_window(world, window);
    }
    window.is_some()
}

fn sync_windows(world: &mut World) {
    for (_, window) in windows(*world.resource::<Mode>()) {
        window.sync(world);
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

pub(crate) fn persist_widget(world: &mut World, id: &str, geom: Geom) -> Geom {
    let mut settings = world.resource_mut::<Settings>();
    settings
        .0
        .set_widget_pos(id, ScreenVec::from_vec2(geom.pos));
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
    geom
}

fn sync_preferences(
    settings: Res<Settings>,
    mut grid: ResMut<SnapGrid>,
    mut speed: ResMut<ui::TypewriterSpeed>,
    mut motion: ResMut<ui::MotionPreference>,
) {
    if !settings.is_changed() {
        return;
    }
    grid.0 = settings.0.snap_grid();
    *speed = settings.0.text.speed.typewriter();
    motion.reduced = settings.0.text.reduced_motion;
}

fn launcher_pos(window: &'static str, mode: Mode, screen_w: f32) -> Vec2 {
    let order = window_def(window).order();
    let slot = windows(mode)
        .filter(|(_, other)| other.order() < order)
        .count();
    Vec2::new(
        screen_w - 8.0 - WIDGET.0,
        8.0 + slot as f32 * (WIDGET.0 + 8.0),
    )
}

fn open_window(world: &mut World, window: &'static str) {
    world.resource_mut::<Open>().0.insert(window);
}

fn toggle_keys(input: ActionInput, mode: Res<Mode>, mut open: ResMut<Open>) {
    for (window, def) in windows(*mode) {
        if input.just_pressed(def.toggle()) && !open.0.remove(&window) {
            open.0.insert(window);
        }
    }
}
