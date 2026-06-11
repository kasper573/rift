use std::collections::HashSet;

use bevy::prelude::*;
use world::session;

use crate::Screen;
use crate::user_settings::{Placement, UserSettings};

const WIDGET: f32 = 48.0;
const SLOT: f32 = 36.0;
const TITLE_H: f32 = 22.0;
const MIN_WINDOW: Vec2 = Vec2::new(100.0, 100.0);
const WINDOW_SIZE: Vec2 = Vec2::new(400.0, 200.0);

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

/// Persisted UI preferences (snap grid, panel placements), loaded once and saved on every change.
#[derive(Resource)]
struct Settings(UserSettings);

impl Default for Settings {
    fn default() -> Settings {
        Settings(UserSettings::load())
    }
}

#[derive(Resource, Default)]
struct Open(HashSet<&'static str>);

#[derive(Component)]
struct Hud;

/// A panel that drags and persists its top-left under `id`; `window` marks it resizable too.
#[derive(Component)]
struct Movable {
    id: &'static str,
    window: bool,
}

/// Dragging this node moves the [`Movable`] it points at (a window's title bar moves its window).
#[derive(Component)]
struct DragHandle(Entity);

/// Dragging this node resizes the window it points at.
#[derive(Component)]
struct ResizeHandle(Entity);

/// A widget that toggles a window; the widget hides while its window is open.
#[derive(Component)]
struct Widget {
    window: &'static str,
}

#[derive(Component)]
struct WindowOf(&'static str);

#[derive(Component)]
struct CharacterText;

#[derive(Component)]
struct InventoryGrid;

struct WidgetSpec {
    id: &'static str,
    window: &'static str,
    keybind: &'static str,
    icon: &'static str,
}

fn spawn_widgets(mut commands: Commands, settings: Res<Settings>, assets: Res<AssetServer>) {
    let screen = Vec2::new(1152.0, 864.0);
    character_widget(&mut commands, &settings, Vec2::new(8.0, 8.0));
    icon_widget(
        &mut commands,
        &settings,
        &assets,
        WidgetSpec {
            id: "inventory",
            window: "inventory.window",
            keybind: "I",
            icon: "icons/potion/red_potion.png",
        },
        Vec2::new(screen.x - 8.0 - WIDGET, 8.0),
    );
    icon_widget(
        &mut commands,
        &settings,
        &assets,
        WidgetSpec {
            id: "settings",
            window: "settings.window",
            keybind: "O",
            icon: "icons/weapon_and_tool/iron_sword.png",
        },
        Vec2::new(screen.x - 8.0 - WIDGET, 16.0 + WIDGET),
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

fn character_widget(commands: &mut Commands, settings: &Settings, fallback: Vec2) {
    let at = placed(settings, "character", fallback);
    commands.spawn((
        Hud,
        Movable {
            id: "character",
            window: false,
        },
        panel_node(at, Vec2::new(140.0, 64.0)),
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
    spec: WidgetSpec,
    fallback: Vec2,
) {
    let at = placed(settings, spec.id, fallback);
    commands
        .spawn((
            Hud,
            Widget {
                window: spec.window,
            },
            Movable {
                id: spec.id,
                window: false,
            },
            Tooltip(title_of(spec.window).to_owned()),
            panel_node(at, Vec2::splat(WIDGET)),
            BackgroundColor(PANEL_BG),
            BorderColor::all(BORDER),
            children![
                (
                    ImageNode::new(assets.load(spec.icon.to_owned())),
                    Node {
                        width: Val::Px(32.0),
                        height: Val::Px(32.0),
                        margin: UiRect::all(Val::Px(8.0)),
                        ..default()
                    },
                    Pickable::IGNORE,
                ),
                badge(spec.keybind),
            ],
        ))
        .observe(open_window);
}

fn open_window(click: On<Pointer<Click>>, widgets: Query<&Widget>, mut open: ResMut<Open>) {
    if let Ok(widget) = widgets.get(click.entity) {
        open.0.insert(widget.window);
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
        if !open.0.contains(window.0) {
            commands.entity(entity).despawn();
        }
    }
    for &id in &open.0 {
        if !windows.iter().any(|(_, window)| window.0 == id) {
            spawn_window(&mut commands, &settings, id);
        }
    }
    for (widget, mut visibility) in &mut widgets {
        *visibility = if open.0.contains(widget.window) {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
    }
}

fn spawn_window(commands: &mut Commands, settings: &Settings, id: &'static str) {
    let placement = settings.0.placement(id);
    let at = placement.map_or(Vec2::new(376.0, 332.0), |p| Vec2::from(p.pos));
    let size = placement
        .and_then(|p| p.size)
        .map_or(WINDOW_SIZE, Vec2::from);

    let window = commands
        .spawn((
            Hud,
            WindowOf(id),
            Movable { id, window: true },
            Node {
                flex_direction: FlexDirection::Column,
                overflow: Overflow::clip(),
                ..panel_node(at, size)
            },
            BackgroundColor(PANEL_BG),
            BorderColor::all(BORDER),
        ))
        .id();

    let title = title_of(id);
    let header = commands
        .spawn((
            ChildOf(window),
            DragHandle(window),
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(TITLE_H),
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
        Text::new(title),
        TextColor(Color::WHITE),
        Pickable::IGNORE,
    ));
    commands
        .spawn((ChildOf(header), close_button()))
        .observe(close_window(id));

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
    match id {
        "inventory.window" => {
            commands.entity(content).insert(InventoryGrid);
        }
        "settings.window" => {
            commands
                .spawn((ChildOf(content), snapping_button(settings)))
                .observe(toggle_snapping);
        }
        _ => {}
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

fn close_window(id: &'static str) -> impl Fn(On<Pointer<Click>>, ResMut<Open>) {
    move |_, mut open| {
        open.0.remove(id);
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
    let delta = drag.delta;
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
            let size = movable.window.then(|| (px(node.width), px(node.height)));
            settings
                .0
                .set_placement(movable.id, Placement { pos, size });
            settings.0.save();
            return;
        }
    }
}

fn toggle_keys(keys: Res<ButtonInput<KeyCode>>, mut open: ResMut<Open>) {
    if keys.just_pressed(KeyCode::KeyI) {
        toggle(&mut open, "inventory.window");
    }
    if keys.just_pressed(KeyCode::KeyO) {
        toggle(&mut open, "settings.window");
    }
}

fn toggle(open: &mut Open, id: &'static str) {
    if !open.0.remove(id) {
        open.0.insert(id);
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
                    width: Val::Px(SLOT),
                    height: Val::Px(SLOT),
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

fn move_node(node: &mut Node, delta: Vec2, settings: &UserSettings) {
    node.left = Val::Px(settings.snap(px(node.left) + delta.x));
    node.top = Val::Px(settings.snap(px(node.top) + delta.y));
}

fn resize_node(node: &mut Node, delta: Vec2, settings: &UserSettings) {
    let width = settings.snap((px(node.width) + delta.x).max(MIN_WINDOW.x));
    let height = settings.snap((px(node.height) + delta.y).max(MIN_WINDOW.y));
    node.width = Val::Px(width.max(MIN_WINDOW.x));
    node.height = Val::Px(height.max(MIN_WINDOW.y));
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

fn placed(settings: &Settings, id: &str, default: Vec2) -> Vec2 {
    settings
        .0
        .placement(id)
        .map_or(default, |placement| Vec2::from(placement.pos))
}

fn panel_node(at: Vec2, size: Vec2) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(at.x),
        top: Val::Px(at.y),
        width: Val::Px(size.x),
        height: Val::Px(size.y),
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

fn title_of(window: &str) -> &'static str {
    match window {
        "inventory.window" => "Inventory",
        "settings.window" => "Settings",
        _ => "",
    }
}

fn px(val: Val) -> f32 {
    match val {
        Val::Px(value) => value,
        _ => 0.0,
    }
}

fn despawn<M: Component>(panels: Query<Entity, With<M>>, mut commands: Commands) {
    for entity in &panels {
        commands.entity(entity).despawn();
    }
}
