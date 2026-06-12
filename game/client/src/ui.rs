use std::collections::HashSet;

use bevy::prelude::*;
use world::math::{Offset, Pos, Size};
use world::session;

use crate::Screen;
use crate::user_settings::{Placement, ScreenPx, UserSettings};

const WIDGET: ScreenPx = ScreenPx(48.0);
const SLOT: ScreenPx = ScreenPx(36.0);
const TITLE_H: ScreenPx = ScreenPx(22.0);
const MIN_WINDOW: Size<ScreenPx> = Size::new(100.0, 100.0);
const WINDOW_SIZE: Size<ScreenPx> = Size::new(400.0, 200.0);

const PANEL_BG: Color = Color::srgb(0.1, 0.1, 0.1);
const TITLE_BG: Color = Color::srgb(0.18, 0.18, 0.18);
const BORDER: Color = Color::srgb(0.31, 0.31, 0.31);

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Settings>()
            .init_resource::<Open>()
            .add_systems(OnEnter(Screen::Playing), spawn_widgets)
            .add_systems(OnExit(Screen::Playing), despawn::<Hud>)
            .add_observer(on_drag)
            .add_observer(on_drag_end)
            .add_observer(show_tooltip)
            .add_observer(hide_tooltip)
            .add_systems(
                Update,
                (
                    toggle_keys,
                    reconcile_windows,
                    sync_character,
                    sync_inventory,
                )
                    .run_if(in_state(Screen::Playing)),
            );
    }
}

/// A toggleable HUD window, opened by its icon widget.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Pane {
    Inventory,
    Settings,
}

impl Pane {
    const ALL: [Pane; 2] = [Pane::Inventory, Pane::Settings];

    fn title(self) -> &'static str {
        match self {
            Pane::Inventory => "Inventory",
            Pane::Settings => "Settings",
        }
    }

    fn toggle(self) -> KeyCode {
        match self {
            Pane::Inventory => KeyCode::KeyI,
            Pane::Settings => KeyCode::KeyO,
        }
    }

    fn keybind(self) -> &'static str {
        match self {
            Pane::Inventory => "I",
            Pane::Settings => "O",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Pane::Inventory => "icons/potion/red_potion.png",
            Pane::Settings => "icons/weapon_and_tool/iron_sword.png",
        }
    }
}

/// A draggable HUD element; [`Panel::key`] is the stable id it persists its placement under.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Panel {
    Character,
    Widget(Pane),
    Window(Pane),
}

impl Panel {
    fn key(self) -> &'static str {
        match self {
            Panel::Character => "character",
            Panel::Widget(Pane::Inventory) => "inventory",
            Panel::Widget(Pane::Settings) => "settings",
            Panel::Window(Pane::Inventory) => "inventory.window",
            Panel::Window(Pane::Settings) => "settings.window",
        }
    }
}

/// Persisted UI preferences (snap grid, panel placements), loaded once and saved on every change.
#[derive(Resource)]
struct Settings(UserSettings);

impl Default for Settings {
    fn default() -> Settings {
        Settings(UserSettings::load())
    }
}

#[derive(Resource, Default)]
struct Open(HashSet<Pane>);

#[derive(Component)]
struct Hud;

/// A panel that drags and persists its top-left under [`Movable::panel`]; `resizable` adds a window.
#[derive(Component)]
struct Movable {
    panel: Panel,
    resizable: bool,
}

/// Dragging this node moves the [`Movable`] it points at (a window's title bar moves its window).
#[derive(Component)]
struct DragHandle(Entity);

/// Dragging this node resizes the window it points at.
#[derive(Component)]
struct ResizeHandle(Entity);

/// An icon that toggles its [`Pane`]; the icon hides while the window is open.
#[derive(Component)]
struct Widget {
    pane: Pane,
}

#[derive(Component)]
struct WindowOf(Pane);

#[derive(Component)]
struct CharacterText;

#[derive(Component)]
struct InventoryGrid;

fn spawn_widgets(mut commands: Commands, settings: Res<Settings>, assets: Res<AssetServer>) {
    let screen = Size::<ScreenPx>::new(1152.0, 864.0);
    character_widget(&mut commands, &settings, Pos::new(8.0, 8.0));
    icon_widget(
        &mut commands,
        &settings,
        &assets,
        Pane::Inventory,
        Pos::new(screen.width - 8.0 - WIDGET.0, 8.0),
    );
    icon_widget(
        &mut commands,
        &settings,
        &assets,
        Pane::Settings,
        Pos::new(screen.width - 8.0 - WIDGET.0, 16.0 + WIDGET.0),
    );
    commands.spawn((
        Hud,
        TooltipDisplay,
        Node {
            position_type: PositionType::Absolute,
            padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
            ..default()
        },
        BackgroundColor(Color::BLACK),
        GlobalZIndex(100),
        Visibility::Hidden,
        children![(
            Text::new(String::new()),
            TextColor(Color::WHITE),
            Pickable::IGNORE
        )],
    ));
}

fn character_widget(commands: &mut Commands, settings: &Settings, fallback: Pos<ScreenPx>) {
    let at = placed(settings, Panel::Character, fallback);
    commands.spawn((
        Hud,
        Movable {
            panel: Panel::Character,
            resizable: false,
        },
        panel_node(at, Size::new(140.0, 64.0)),
        BackgroundColor(PANEL_BG),
        BorderColor::all(BORDER),
        children![(
            CharacterText,
            Text::new(String::new()),
            TextColor(Color::WHITE),
            Node {
                margin: UiRect::all(Val::Px(6.0)),
                ..default()
            },
        )],
    ));
}

fn icon_widget(
    commands: &mut Commands,
    settings: &Settings,
    assets: &AssetServer,
    pane: Pane,
    fallback: Pos<ScreenPx>,
) {
    let at = placed(settings, Panel::Widget(pane), fallback);
    commands
        .spawn((
            Hud,
            Widget { pane },
            Movable {
                panel: Panel::Widget(pane),
                resizable: false,
            },
            Tooltip(pane.title().to_owned()),
            panel_node(at, Size::splat(WIDGET.0)),
            BackgroundColor(PANEL_BG),
            BorderColor::all(BORDER),
            children![
                (
                    ImageNode::new(assets.load(pane.icon().to_owned())),
                    Node {
                        width: Val::Px(32.0),
                        height: Val::Px(32.0),
                        margin: UiRect::all(Val::Px(8.0)),
                        ..default()
                    },
                    Pickable::IGNORE,
                ),
                badge(pane.keybind()),
            ],
        ))
        .observe(open_window);
}

fn open_window(click: On<Pointer<Click>>, widgets: Query<&Widget>, mut open: ResMut<Open>) {
    if let Ok(widget) = widgets.get(click.entity) {
        open.0.insert(widget.pane);
    }
}

/// Spawns/despawns windows to match [`Open`], and hides a widget while its window is open.
fn reconcile_windows(
    open: Res<Open>,
    settings: Res<Settings>,
    windows: Query<(Entity, &WindowOf)>,
    mut widgets: Query<(&Widget, &mut Visibility)>,
    mut commands: Commands,
) {
    for (entity, window) in &windows {
        if !open.0.contains(&window.0) {
            commands.entity(entity).despawn();
        }
    }
    for &pane in &open.0 {
        if !windows.iter().any(|(_, window)| window.0 == pane) {
            spawn_window(&mut commands, &settings, pane);
        }
    }
    for (widget, mut visibility) in &mut widgets {
        *visibility = if open.0.contains(&widget.pane) {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
    }
}

fn spawn_window(commands: &mut Commands, settings: &Settings, pane: Pane) {
    let panel = Panel::Window(pane);
    let placement = settings.0.placement(panel.key());
    let at = placement.map_or(Pos::new(376.0, 332.0), |p| point(p.pos));
    let size = placement.and_then(|p| p.size).map_or(WINDOW_SIZE, extent);

    let window = commands
        .spawn((
            Hud,
            WindowOf(pane),
            Movable {
                panel,
                resizable: true,
            },
            Node {
                flex_direction: FlexDirection::Column,
                overflow: Overflow::clip(),
                ..panel_node(at, size)
            },
            BackgroundColor(PANEL_BG),
            BorderColor::all(BORDER),
        ))
        .id();

    let header = commands
        .spawn((
            ChildOf(window),
            DragHandle(window),
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(TITLE_H.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                padding: UiRect::horizontal(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(TITLE_BG),
        ))
        .id();
    commands.spawn((
        ChildOf(header),
        Text::new(pane.title()),
        TextColor(Color::WHITE),
        Pickable::IGNORE,
    ));
    commands
        .spawn((ChildOf(header), close_button()))
        .observe(close_window(pane));

    let content = commands
        .spawn((
            ChildOf(window),
            Node {
                flex_grow: 1.0,
                flex_wrap: FlexWrap::Wrap,
                align_content: AlignContent::FlexStart,
                padding: UiRect::all(Val::Px(4.0)),
                ..default()
            },
        ))
        .id();
    match pane {
        Pane::Inventory => {
            commands.entity(content).insert(InventoryGrid);
        }
        Pane::Settings => {
            commands
                .spawn((ChildOf(content), snapping_button(settings)))
                .observe(toggle_snapping);
        }
    }

    commands.spawn((
        ChildOf(window),
        ResizeHandle(window),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(0.0),
            bottom: Val::Px(0.0),
            width: Val::Px(16.0),
            height: Val::Px(16.0),
            ..default()
        },
        BackgroundColor(BORDER),
    ));
}

fn close_window(pane: Pane) -> impl Fn(On<Pointer<Click>>, ResMut<Open>) {
    move |_, mut open| {
        open.0.remove(&pane);
    }
}

/// Accumulates a drag into the dragged panel's position (or a window's size for a resize handle),
/// snapped to the grid.
fn on_drag(
    drag: On<Pointer<Drag>>,
    handles: Query<&DragHandle>,
    resizes: Query<&ResizeHandle>,
    children: Query<&ChildOf>,
    settings: Res<Settings>,
    mut nodes: Query<(&mut Node, &Movable)>,
) {
    let delta = Offset::<ScreenPx>::new(drag.delta.x, drag.delta.y);
    for entity in ancestry(drag.entity, &children) {
        if let Ok(handle) = handles.get(entity)
            && let Ok((mut node, _)) = nodes.get_mut(handle.0)
        {
            move_node(&mut node, delta, &settings.0);
            return;
        }
        if let Ok(resize) = resizes.get(entity)
            && let Ok((mut node, _)) = nodes.get_mut(resize.0)
        {
            resize_node(&mut node, delta, &settings.0);
            return;
        }
        if let Ok((mut node, _)) = nodes.get_mut(entity) {
            move_node(&mut node, delta, &settings.0);
            return;
        }
    }
}

/// Persists the dragged panel's geometry when the drag ends.
fn on_drag_end(
    drag: On<Pointer<DragEnd>>,
    handles: Query<&DragHandle>,
    resizes: Query<&ResizeHandle>,
    children: Query<&ChildOf>,
    nodes: Query<(&Node, &Movable)>,
    mut settings: ResMut<Settings>,
) {
    for entity in ancestry(drag.entity, &children) {
        let target = handles
            .get(entity)
            .map(|h| h.0)
            .or_else(|_| resizes.get(entity).map(|r| r.0))
            .unwrap_or(entity);
        if let Ok((node, movable)) = nodes.get(target) {
            let pos = (px(node.left), px(node.top));
            let size = movable.resizable.then(|| (px(node.width), px(node.height)));
            settings
                .0
                .set_placement(movable.panel.key(), Placement { pos, size });
            settings.0.save();
            return;
        }
    }
}

fn toggle_keys(keys: Res<ButtonInput<KeyCode>>, mut open: ResMut<Open>) {
    for pane in Pane::ALL {
        if keys.just_pressed(pane.toggle()) {
            toggle(&mut open, pane);
        }
    }
}

fn toggle(open: &mut Open, pane: Pane) {
    if !open.0.remove(&pane) {
        open.0.insert(pane);
    }
}

fn sync_character(world: &mut World) {
    let name = session::my_name(world).unwrap_or_default();
    let (health, max) = session::my_vitals(world).unwrap_or((0.0, 0.0));
    let xp = session::my_xp(world).unwrap_or(0);
    let content = format!("{name}\n{health:.0} / {max:.0}\nxp {xp}");
    let mut texts = world.query_filtered::<&mut Text, With<CharacterText>>();
    for mut text in texts.iter_mut(world) {
        text.0.clone_from(&content);
    }
}

fn sync_inventory(world: &mut World) {
    let Ok(grid) = world
        .query_filtered::<Entity, With<InventoryGrid>>()
        .single(world)
    else {
        return;
    };
    let items = session::my_inventory(world);
    let existing = world
        .get::<Children>(grid)
        .map_or(0, |children| children.len());
    if existing == items.len() {
        return;
    }
    world.entity_mut(grid).despawn_related::<Children>();
    let cells: Vec<(Handle<Image>, String)> = {
        let assets = world.resource::<AssetServer>();
        items
            .iter()
            .map(|item| {
                let def = world::items::item(*item);
                (assets.load(def.icon.0.clone()), def.display_name.clone())
            })
            .collect()
    };
    for (slot, (icon, name)) in cells.into_iter().enumerate() {
        let cell = world
            .spawn((
                ChildOf(grid),
                InventorySlot(slot as u32),
                Tooltip(name),
                Node {
                    width: Val::Px(SLOT.0),
                    height: Val::Px(SLOT.0),
                    margin: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(TITLE_BG),
                children![(
                    ImageNode::new(icon),
                    Node {
                        width: Val::Px(32.0),
                        height: Val::Px(32.0),
                        ..default()
                    },
                    Pickable::IGNORE,
                )],
            ))
            .id();
        world.entity_mut(cell).observe(use_slot);
    }
}

#[derive(Component)]
struct InventorySlot(u32);

fn use_slot(click: On<Pointer<Click>>, slots: Query<&InventorySlot>, mut commands: Commands) {
    if let Ok(slot) = slots.get(click.entity) {
        let slot = slot.0;
        commands.queue(move |world: &mut World| session::use_item(world, slot));
    }
}

fn toggle_snapping(_: On<Pointer<Click>>, mut commands: Commands) {
    commands.queue(|world: &mut World| {
        let mut settings = world.resource_mut::<Settings>();
        settings.0.toggle_snapping();
        settings.0.save();
    });
}

fn move_node(node: &mut Node, delta: Offset<ScreenPx>, settings: &UserSettings) {
    node.left = Val::Px(settings.snap(px(node.left) + delta.x).0);
    node.top = Val::Px(settings.snap(px(node.top) + delta.y).0);
}

fn resize_node(node: &mut Node, delta: Offset<ScreenPx>, settings: &UserSettings) {
    let width = settings
        .snap(px(node.width) + delta.x)
        .0
        .max(MIN_WINDOW.width);
    let height = settings
        .snap(px(node.height) + delta.y)
        .0
        .max(MIN_WINDOW.height);
    node.width = Val::Px(width);
    node.height = Val::Px(height);
}

/// `entity` then each of its ancestors, nearest first.
fn ancestry(entity: Entity, children: &Query<&ChildOf>) -> Vec<Entity> {
    let mut chain = vec![entity];
    let mut current = entity;
    while let Ok(parent) = children.get(current) {
        current = parent.parent();
        chain.push(current);
    }
    chain
}

fn placed(settings: &Settings, panel: Panel, default: Pos<ScreenPx>) -> Pos<ScreenPx> {
    settings
        .0
        .placement(panel.key())
        .map_or(default, |placement| point(placement.pos))
}

fn point((x, y): (ScreenPx, ScreenPx)) -> Pos<ScreenPx> {
    Pos::new(x.0, y.0)
}

fn extent((width, height): (ScreenPx, ScreenPx)) -> Size<ScreenPx> {
    Size::new(width.0, height.0)
}

fn panel_node(at: Pos<ScreenPx>, size: Size<ScreenPx>) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(at.x),
        top: Val::Px(at.y),
        width: Val::Px(size.width),
        height: Val::Px(size.height),
        border: UiRect::all(Val::Px(1.0)),
        ..default()
    }
}

fn badge(text: &str) -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(2.0),
            bottom: Val::Px(2.0),
            ..default()
        },
        children![(
            Text::new(text.to_owned()),
            TextFont::from_font_size(11.0),
            TextColor(Color::WHITE),
            Pickable::IGNORE,
        )],
        Pickable::IGNORE,
    )
}

fn close_button() -> impl Bundle {
    (
        Button,
        Node {
            width: Val::Px(16.0),
            height: Val::Px(16.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        children![(Text::new("×"), TextColor(Color::WHITE), Pickable::IGNORE)],
    )
}

fn snapping_button(settings: &Settings) -> impl Bundle {
    (
        Button,
        Node {
            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BorderColor::all(BORDER),
        BackgroundColor(TITLE_BG),
        children![(
            Text::new(snapping_label(settings)),
            TextColor(Color::WHITE),
            Pickable::IGNORE,
        )],
    )
}

fn snapping_label(settings: &Settings) -> &'static str {
    if settings.0.snapping_enabled() {
        "ui snapping enabled"
    } else {
        "ui snapping disabled"
    }
}

#[derive(Component)]
struct Tooltip(String);

#[derive(Component)]
struct TooltipDisplay;

fn show_tooltip(
    over: On<Pointer<Over>>,
    tips: Query<&Tooltip>,
    window: Single<&Window>,
    display: Single<(&mut Visibility, &mut Node, &Children), With<TooltipDisplay>>,
    mut texts: Query<&mut Text>,
) {
    let Ok(tip) = tips.get(over.entity) else {
        return;
    };
    let (mut visibility, mut node, children) = display.into_inner();
    *visibility = Visibility::Visible;
    if let Some(cursor) = window.cursor_position() {
        node.left = Val::Px(cursor.x + 12.0);
        node.top = Val::Px(cursor.y + 12.0);
    }
    if let Some(&label) = children.first()
        && let Ok(mut text) = texts.get_mut(label)
    {
        text.0.clone_from(&tip.0);
    }
}

fn hide_tooltip(_: On<Pointer<Out>>, mut display: Single<&mut Visibility, With<TooltipDisplay>>) {
    **display = Visibility::Hidden;
}

fn px(val: Val) -> ScreenPx {
    match val {
        Val::Px(value) => ScreenPx(value),
        _ => ScreenPx(0.0),
    }
}

fn despawn<M: Component>(panels: Query<Entity, With<M>>, mut commands: Commands) {
    for entity in &panels {
        commands.entity(entity).despawn();
    }
}
