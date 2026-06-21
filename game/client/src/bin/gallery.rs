use std::collections::HashSet;

use bevy::prelude::*;
use bevy::ui::{ComputedNode, UiGlobalTransform, widget::Text as TextWidget};
use bevy_picking::hover::Hovered;
use ui::theme::color;
use ui::themes;
use ui::{
    Align, CardOpts, Check, Orientation, Side, SonnerPosition, accordion, accordion_body,
    accordion_content, accordion_header, accordion_item, accordion_trigger, alert_dialog,
    alert_dialog_action, alert_dialog_cancel, alert_dialog_content, alert_dialog_scrim,
    alert_dialog_trigger, avatar, avatar_fallback, button, button_styled, card, checkbox,
    checkbox_indicator, collapsible, collapsible_content, collapsible_trigger, dialog,
    dialog_close, dialog_content, dialog_scrim, dialog_trigger, popover, popover_content,
    popover_trigger, progress, progress_indicator, radio_circle, radio_group, radio_indicator,
    radio_item, scroll_area, scroll_bar, scroll_thumb, scroll_viewport, separator, slider,
    slider_range, slider_thumb, slider_track, sonner_close, switch, switch_thumb, tabs, tabs_list,
    tabs_trigger, text, text_colored, toast, toaster, tooltip, tooltip_content,
};

const WINDOW: Vec2 = Vec2::new(1600.0, 900.0);
const SETTLE: f32 = 0.4;
const INTRO: f32 = 4.0;
const STATIC_HOLD: f32 = 2.6;

const TOAST_MESSAGES: &[(&str, &str)] = &[
    ("Event created", "Monday, January 6 at 9:00 AM"),
    ("Changes saved", "Your project is up to date."),
    ("Copied to clipboard", "The share link is ready."),
    ("Upload complete", "report-q3.pdf finished uploading."),
];

#[derive(Resource)]
struct CaptureCfg {
    dir: String,
    frames: u32,
}

fn main() {
    // Same contract as the client/server: the asset directory is explicit, never a baked default.
    let assets = std::fs::canonicalize(
        std::env::var_os("RIFT_ASSETS_DIR").expect("RIFT_ASSETS_DIR must be set"),
    )
    .expect("RIFT_ASSETS_DIR must point to an existing directory");
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "rift ui gallery".to_owned(),
                    resolution: WINDOW.as_uvec2().into(),
                    decorations: false,
                    ..default()
                }),
                ..default()
            })
            .set(bevy::asset::AssetPlugin {
                file_path: assets.to_string_lossy().into_owned(),
                ..default()
            }),
    )
    .insert_resource(ClearColor(
        color::surface_inset_base.resolve(&themes::light::theme()),
    ))
    .insert_resource(themes::light::theme())
    .init_resource::<Fps>()
    .init_resource::<Director>()
    .init_resource::<ToastCounter>()
    .add_plugins(ui::UiPlugin)
    .add_systems(Startup, setup)
    .add_systems(Update, (update_fps, update_fps_display).chain())
    // Director runs in Update before the ui reactive pass (same schedule point as the original's
    // render), so the state it sets (Open, Hovered) and the scene it rebuilds are seen the same frame.
    .add_systems(
        Update,
        (direct, rebuild_scene, place_cursor)
            .chain()
            .before(ui::UiReactive),
    );

    capture_setup(&mut app);
    app.run();
}

fn capture_setup(app: &mut App) {
    let Ok(dir) = std::env::var("RIFT_CAPTURE_DIR") else {
        return;
    };
    let frames: u32 = std::env::var("RIFT_CAPTURE_FRAMES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(600);
    let _ = std::fs::create_dir_all(&dir);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f64(1.0 / 60.0),
    ));
    app.insert_resource(CaptureCfg { dir, frames });
    app.add_systems(bevy::app::PostUpdate, capture_frame);
}

fn capture_frame(mut commands: Commands, cfg: Res<CaptureCfg>, mut n: Local<u32>) {
    if *n < cfg.frames {
        let path = format!("{}/f{:05}.png", cfg.dir, *n);
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(path));
    } else if *n > cfg.frames + 40 {
        std::process::exit(0);
    }
    *n += 1;
}

#[derive(Resource, Default)]
struct Fps(f32);

fn update_fps(time: Res<Time>, mut fps: ResMut<Fps>) {
    let instant = 1.0 / time.delta_secs().max(1e-4);
    fps.0 = fps.0 * 0.9 + instant * 0.1;
}

fn update_fps_display(fps: Res<Fps>, mut displays: Query<&mut TextWidget, With<FpsDisplay>>) {
    for mut text in &mut displays {
        text.0 = format!("{:.0} fps", fps.0);
    }
}

#[derive(Resource, Default)]
struct ToastCounter(u64);

#[derive(Component, Clone, Copy)]
struct Demo {
    order: u32,
    act: Act,
}

#[derive(Clone, Copy, PartialEq)]
enum Act {
    Press,
    Click,
    Close,
    Open,
    Drag,
    Fill,
    Spawn,
}

#[derive(Component)]
struct CursorDot;

#[derive(Component)]
struct ToasterEntity;

#[derive(Resource, Default)]
struct Director {
    scene: usize,
    autoplay: bool,
    pinned: bool,
    settle: f32,
    target: usize,
    beat: usize,
    t: f32,
    entered: bool,
    cursor: Vec2,
    anchor: Vec2,
    pressed: bool,
    idle: f32,
    // Reproduce the original reconciler's overlay timing: when a scene is entered by an overlay
    // closing (its Demo gating out), the original took one extra frame to swap the shared overlay
    // outlet before the next overlay could paint. The retained ui has no outlet to swap, so the
    // gallery re-adds that single frame here to keep the autoplay video frame-identical.
    prev_targets: usize,
    delay_open: bool,
    pending_overlay: Option<(Entity, bool)>,
    pending_advance: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Beat {
    Approach,
    Hover,
    Down,
    Up,
    Leave,
    Sweep,
    Filling,
    Hold,
    Emit,
    Expand,
    DismissMiddle,
    DismissOne,
    Collapse,
    SetPosition,
}

fn plan(act: Act) -> &'static [(Beat, f32)] {
    use Beat::*;
    match act {
        Act::Press => &[(Approach, 0.4), (Hover, 0.7), (Down, 0.6), (Up, 0.6)],
        Act::Click => &[(Approach, 0.4), (Hover, 0.5), (Down, 0.4), (Up, 1.1)],
        Act::Close => &[(Approach, 0.4), (Hover, 0.5), (Down, 0.4), (Up, 2.2)],
        Act::Open => &[
            (Approach, 0.4),
            (Hover, 0.4),
            (Down, 0.3),
            (Up, 1.7),
            (Leave, 0.7),
        ],
        Act::Drag => &[(Approach, 0.4), (Down, 0.3), (Sweep, 1.8), (Up, 0.6)],
        Act::Fill => &[(Approach, 0.4), (Filling, 3.0), (Hold, 0.6)],
        Act::Spawn => &[
            (SetPosition, 0.5),
            (Emit, 0.0),
            (Hold, 0.4),
            (Emit, 0.0),
            (Hold, 0.4),
            (Emit, 0.0),
            (Hold, 0.8),
            (Expand, 1.6),
            (DismissMiddle, 1.5),
            (DismissOne, 1.4),
            (DismissOne, 1.4),
            (Collapse, 0.8),
        ],
    }
}

fn ease(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

#[derive(Clone, Copy)]
struct Target {
    entity: Entity,
    order: u32,
    act: Act,
    center: Vec2,
}

fn scene_targets(world: &mut World) -> Vec<Target> {
    let mut query = world.query::<(Entity, &Demo, &ComputedNode, &UiGlobalTransform)>();
    query
        .iter(world)
        // Skip hidden targets (zero-size, e.g. a closed dialog's Cancel button still mounted as
        // Display::None). The original reconciler gated these out of the tree; the retained dialog
        // keeps them mounted, so the Director must ignore them to advance at the same time.
        .filter(|(_, _, computed, _)| computed.size.x > 0.0 && computed.size.y > 0.0)
        .map(|(entity, demo, computed, transform)| Target {
            entity,
            order: demo.order,
            act: demo.act,
            center: transform.translation * computed.inverse_scale_factor,
        })
        .collect()
}

fn descendants(world: &World, root: Entity) -> Vec<Entity> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        out.push(entity);
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
    }
    out
}

fn set_state(world: &mut World, root: Entity, hovered: bool, pressed: bool) {
    for entity in descendants(world, root) {
        if world.get::<Hovered>(entity).is_some() {
            world.entity_mut(entity).insert(Hovered(hovered));
        }
        if pressed {
            world.entity_mut(entity).insert(bevy::ui::Pressed);
        } else {
            world.entity_mut(entity).remove::<bevy::ui::Pressed>();
        }
    }
}

fn clear_all_states(world: &mut World) {
    let entities: Vec<_> = world.query::<Entity>().iter(world).collect();
    for entity in entities {
        if world.get::<Hovered>(entity).is_some() {
            world.entity_mut(entity).insert(Hovered(false));
        }
        world.entity_mut(entity).remove::<bevy::ui::Pressed>();
    }
}

#[derive(Component)]
struct SceneRoot;

#[derive(Component)]
struct FpsDisplay;

fn setup(mut commands: Commands, mut director: ResMut<Director>) {
    director.cursor = WINDOW / 2.0;
    director.settle = INTRO;
    director.autoplay = std::env::var("RIFT_AUTOPLAY").is_ok();
    if let Ok(scene) = std::env::var("RIFT_SCENE") {
        director.scene = scene.parse::<usize>().unwrap_or(0).min(SCENES.len() - 1);
        director.pinned = true;
        director.settle = SETTLE;
    }
    commands.spawn((Camera2d, IsDefaultUiCamera));
    spawn_scene(&mut commands, director.scene);

    if director.autoplay {
        let (scene_name, _) = SCENES[director.scene];
        commands.spawn((
            SceneLabel,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(40.0),
                left: Val::Px(48.0),
                ..default()
            },
            text(scene_name),
        ));
        commands.spawn((
            FpsDisplay,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(12.0),
                bottom: Val::Px(12.0),
                ..default()
            },
            text("0 fps"),
        ));
    }

    commands.spawn((
        CursorDot,
        Node {
            position_type: PositionType::Absolute,
            width: Val::Px(22.0),
            height: Val::Px(22.0),
            border: UiRect::all(Val::Px(2.0)),
            border_radius: BorderRadius::all(Val::Px(11.0)),
            ..default()
        },
        BorderColor::all(Color::srgba(0.1, 0.1, 0.12, 0.9)),
        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.55)),
        GlobalZIndex(10_000),
    ));
}

// Despawns and respawns the scene tree whenever the Director advances to a new scene (the original
// reconciler re-rendered on scene change; retained mode needs this explicit rebuild).
fn rebuild_scene(
    director: Res<Director>,
    mut spawned: Local<Option<usize>>,
    roots: Query<Entity, With<SceneRoot>>,
    mut labels: Query<&mut TextWidget, With<SceneLabel>>,
    mut commands: Commands,
) {
    if *spawned == Some(director.scene) {
        return;
    }
    *spawned = Some(director.scene);
    for root in &roots {
        commands.entity(root).despawn();
    }
    spawn_scene(&mut commands, director.scene);
    for mut label in &mut labels {
        label.0 = SCENES[director.scene].0.to_owned();
    }
}

#[derive(Component)]
struct SceneLabel;

fn spawn_scene(commands: &mut Commands, index: usize) {
    let (_name, builder) = SCENES[index.min(SCENES.len() - 1)];
    let mut scene_node = commands.spawn((
        SceneRoot,
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
    ));
    builder(&mut scene_node);
}

fn place_cursor(
    director: Res<Director>,
    mut cursors: Query<(&mut Node, &mut BackgroundColor), With<CursorDot>>,
) {
    let Ok((mut node, mut bg)) = cursors.single_mut() else {
        return;
    };
    if !director.autoplay {
        node.display = Display::None;
        return;
    }
    node.display = Display::Flex;
    node.left = Val::Px(director.cursor.x - 11.0);
    node.top = Val::Px(director.cursor.y - 11.0);
    *bg = BackgroundColor(if director.pressed {
        Color::srgba(0.2, 0.45, 0.95, 0.65)
    } else {
        Color::srgba(1.0, 1.0, 1.0, 0.55)
    });
}

fn direct(world: &mut World) {
    let dt = world.resource::<Time>().delta_secs().min(0.1);
    let mut dir = world.remove_resource::<Director>().unwrap();

    if !dir.autoplay {
        world.insert_resource(dir);
        return;
    }

    clear_all_states(world);

    if dir.settle > 0.0 {
        dir.settle -= dt;
        world.insert_resource(dir);
        return;
    }

    let mut targets = scene_targets(world);
    targets.sort_by_key(|t| t.order);

    // A target gating out (e.g. a closing dialog's Cancel) shrinks the set mid-scene; that's the
    // signal the next scene is entered by an overlay closing.
    let shrank = !targets.is_empty() && targets.len() < dir.prev_targets;
    dir.prev_targets = targets.len();

    if targets.is_empty() {
        dir.idle += dt;
        if dir.idle >= STATIC_HOLD {
            dir.delay_open = false;
            advance_scene(&mut dir);
        }
        world.insert_resource(dir);
        return;
    }
    dir.idle = 0.0;

    if dir.target >= targets.len() {
        // A scene entered by an overlay closing also advances out one frame later in the original
        // (the outlet swap again); hold one frame before advancing.
        if dir.delay_open && !dir.pending_advance {
            dir.pending_advance = true;
            world.insert_resource(dir);
            return;
        }
        dir.delay_open = shrank;
        advance_scene(&mut dir);
        world.insert_resource(dir);
        return;
    }

    let target = targets[dir.target];
    let beats = plan(target.act);
    if dir.beat >= beats.len() {
        dir.target += 1;
        dir.beat = 0;
        dir.t = 0.0;
        dir.entered = false;
        world.insert_resource(dir);
        return;
    }

    let (beat, duration) = beats[dir.beat];

    if !dir.entered {
        dir.entered = true;
        dir.anchor = dir.cursor;
        on_beat_enter(world, &mut dir, beat, target);
    }

    let progress = if duration > 0.0 {
        (dir.t / duration).clamp(0.0, 1.0)
    } else {
        1.0
    };
    drive_beat(world, &mut dir, beat, target, progress);

    dir.t += dt;
    if dir.t >= duration {
        dir.beat += 1;
        dir.t = 0.0;
        dir.entered = false;
    }
    world.insert_resource(dir);
}

fn on_beat_enter(world: &mut World, dir: &mut Director, beat: Beat, target: Target) {
    use Beat::*;
    match beat {
        Up => match target.act {
            // Overlays: `Pointer<Over>` opens hover-driven tooltips (via their real delay timer);
            // click-driven dialogs/popovers (which carry an `OverlayAction`) open by setting `Open`
            // directly — same one-system latency as hover, so they open on the same frame as the
            // original (a synthetic click would add a frame via the observer/command hop).
            Act::Open => {
                for entity in descendants(world, target.entity) {
                    fire_over(world, entity);
                    if world.get::<ui::OverlayAction>(entity).is_some() {
                        if dir.delay_open {
                            dir.pending_overlay = Some((entity, true));
                        } else {
                            ui::set_overlay_open(world, entity, true);
                        }
                    }
                }
            }
            Act::Close => {
                for entity in descendants(world, target.entity) {
                    if matches!(
                        world.get::<ui::OverlayAction>(entity),
                        Some(ui::OverlayAction::Close)
                    ) {
                        if dir.delay_open {
                            dir.pending_overlay = Some((entity, false));
                        } else {
                            ui::set_overlay_open(world, entity, false);
                        }
                    }
                }
            }
            // Selection/toggle widgets (tabs/radio/accordion/collapsible) react to `Activate`.
            Act::Click => {
                for entity in descendants(world, target.entity) {
                    world.trigger(ui::Activate { entity });
                }
            }
            _ => {}
        },
        // Leaving closes tooltips (`Pointer<Out>`) and dismisses toggle popovers; dialogs stay open
        // (they close via their own cancel button), matching the original.
        Leave => {
            for entity in descendants(world, target.entity) {
                fire_out(world, entity);
                if matches!(
                    world.get::<ui::OverlayAction>(entity),
                    Some(ui::OverlayAction::Toggle)
                ) {
                    ui::set_overlay_open(world, entity, false);
                }
            }
        }
        Emit => {
            let mut counter = world.resource_mut::<ToastCounter>();
            let id = counter.0;
            counter.0 += 1;
            let (title, body) = TOAST_MESSAGES[(id as usize) % TOAST_MESSAGES.len()];

            if let Ok(toaster_entity) = world
                .query_filtered::<Entity, With<ToasterEntity>>()
                .single(world)
            {
                world.entity_mut(toaster_entity).with_children(|parent| {
                    parent.spawn(toast()).with_children(|parent| {
                        parent
                            .spawn((Node {
                                flex_direction: FlexDirection::Row,
                                justify_content: JustifyContent::SpaceBetween,
                                align_items: AlignItems::Center,
                                column_gap: Val::Px(12.0),
                                ..default()
                            },))
                            .with_children(|parent| {
                                parent.spawn(text(title));
                                parent.spawn(sonner_close()).with_children(|parent| {
                                    parent.spawn(button_styled("secondary", "sm", "close"));
                                });
                            });
                        parent.spawn(text_colored(body, color::surface_canvas_on_soft));
                    });
                });
            }
        }
        SetPosition => {
            if let Some(toaster) = toaster_entity(world) {
                for t in toast_children(world, toaster) {
                    world.entity_mut(t).despawn();
                }
                set_toaster_expanded(world, false);
            }
        }
        Expand => set_toaster_expanded(world, true),
        Collapse => set_toaster_expanded(world, false),
        DismissMiddle => {
            if let Some(toaster) = toaster_entity(world) {
                let toasts = toast_children(world, toaster);
                if toasts.len() >= 2 {
                    let mid = toasts[toasts.len() / 2];
                    if let Some(mut toast) = world.get_mut::<ui::Toast>(mid) {
                        toast.leaving = true;
                    }
                }
            }
        }
        DismissOne => {
            if let Some(toaster) = toaster_entity(world) {
                let last = toast_children(world, toaster).into_iter().rev().find(|&t| {
                    world
                        .get::<ui::Toast>(t)
                        .is_some_and(|toast| !toast.leaving)
                });
                if let Some(last) = last
                    && let Some(mut toast) = world.get_mut::<ui::Toast>(last)
                {
                    toast.leaving = true;
                }
            }
        }
        _ => {}
    }
}

fn toaster_entity(world: &mut World) -> Option<Entity> {
    world
        .query_filtered::<Entity, With<ToasterEntity>>()
        .iter(world)
        .next()
}

fn toast_children(world: &World, toaster: Entity) -> Vec<Entity> {
    world
        .get::<Children>(toaster)
        .map(|children| {
            children
                .iter()
                .filter(|&e| world.get::<ui::Toast>(e).is_some())
                .collect()
        })
        .unwrap_or_default()
}

fn set_toaster_expanded(world: &mut World, expanded: bool) {
    if let Some(toaster) = toaster_entity(world)
        && let Some(mut toaster) = world.get_mut::<ui::Toaster>(toaster)
    {
        toaster.expanded = expanded;
    }
}

fn pointer_location() -> bevy_picking::pointer::Location {
    bevy_picking::pointer::Location {
        target: bevy::camera::NormalizedRenderTarget::None {
            width: WINDOW.x as u32,
            height: WINDOW.y as u32,
        },
        position: Vec2::ZERO,
    }
}

fn fire_over(world: &mut World, entity: Entity) {
    use bevy_picking::events::{Over, Pointer};
    world.trigger(Pointer::new(
        bevy_picking::pointer::PointerId::Mouse,
        pointer_location(),
        Over {
            hit: bevy_picking::backend::HitData::new(Entity::PLACEHOLDER, 0.0, None, None),
        },
        entity,
    ));
}

fn fire_out(world: &mut World, entity: Entity) {
    use bevy_picking::events::{Out, Pointer};
    world.trigger(Pointer::new(
        bevy_picking::pointer::PointerId::Mouse,
        pointer_location(),
        Out {
            hit: bevy_picking::backend::HitData::new(Entity::PLACEHOLDER, 0.0, None, None),
        },
        entity,
    ));
}

fn drive_beat(world: &mut World, dir: &mut Director, beat: Beat, target: Target, progress: f32) {
    use Beat::*;
    match beat {
        Approach => {
            dir.cursor = dir.anchor.lerp(target.center, ease(progress));
        }
        Hover | Up | Leave => {
            dir.cursor = target.center;
            dir.pressed = false;
            set_state(world, target.entity, true, false);
        }
        Down => {
            dir.cursor = target.center;
            dir.pressed = true;
            set_state(world, target.entity, true, true);
        }
        Sweep => {
            let half = 180.0;
            dir.cursor.x = target.center.x - half + 2.0 * half * progress;
            dir.cursor.y = target.center.y;
            dir.pressed = true;
            if let Some(mut state) = world.get_mut::<ui::SliderState>(target.entity) {
                state.value = state.min + progress * (state.max - state.min);
            }
        }
        Filling => {
            dir.cursor = target.center;
            dir.pressed = false;
            world
                .entity_mut(target.entity)
                .insert(ui::ProgressFraction(progress));
        }
        Hold | Emit => {
            dir.pressed = false;
        }
        SetPosition | Expand | Collapse => {
            let pos = WINDOW * Vec2::new(0.87, 0.88);
            dir.cursor = pos;
            dir.pressed = false;
        }
        DismissMiddle | DismissOne => {
            let pos = WINDOW * Vec2::new(0.87, 0.88);
            dir.cursor = pos;
            dir.pressed = true;
        }
    }

    // Drive a delayed overlay open/close one frame after the Up beat begins, when this scene was
    // entered by an overlay closing.
    if beat == Beat::Up
        && progress > 0.005
        && let Some((entity, open)) = dir.pending_overlay.take()
    {
        ui::set_overlay_open(world, entity, open);
    }
}

fn advance_scene(dir: &mut Director) {
    if !dir.pinned && dir.scene + 1 < SCENES.len() {
        dir.scene += 1;
    }
    dir.target = 0;
    dir.beat = 0;
    dir.t = 0.0;
    dir.entered = false;
    dir.idle = 0.0;
    dir.settle = SETTLE;
    dir.pressed = false;
    dir.prev_targets = 0;
    dir.pending_overlay = None;
    dir.pending_advance = false;
}

type SceneBuilder = fn(&mut EntityCommands);

const SCENES: &[(&str, SceneBuilder)] = &[
    ("Button intents", button_intents_scene),
    ("Button sizes", button_sizes_scene),
    ("Tabs", tabs_scene),
    ("Checkbox", checkbox_scene),
    ("Switch", switch_scene),
    ("Radio group", radio_scene),
    ("Slider", slider_scene),
    ("Progress", progress_scene),
    ("Avatar", avatar_scene),
    ("Separator", separator_scene),
    ("Accordion", accordion_scene),
    ("Collapsible", collapsible_scene),
    ("Dialog", dialog_scene),
    ("Alert dialog", alert_dialog_scene),
    ("Card", card_scene),
    ("Tooltip", tooltip_scene),
    ("Popover", popover_scene),
    ("Tooltip + card", tooltip_card_scene),
    ("Popover + card", popover_card_scene),
    ("Toasts (sonner)", toasts_scene),
    ("Scroll area", scroll_area_scene),
];

fn button_intents_scene(scene: &mut EntityCommands) {
    const INTENTS: &[&str] = &[
        "primary",
        "secondary",
        "tonal",
        "muted",
        "soft",
        "quiet",
        "bare",
        "danger",
        "danger_soft",
        "ghost",
        "plain",
        "accent",
    ];

    scene.with_children(|parent| {
        parent
            .spawn((Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(18.0),
                row_gap: Val::Px(18.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                max_width: Val::Px(1360.0),
                ..default()
            },))
            .with_children(|parent| {
                for (i, intent) in INTENTS.iter().enumerate() {
                    parent.spawn((
                        Demo {
                            order: i as u32,
                            act: Act::Press,
                        },
                        button_styled(intent, "md", *intent),
                    ));
                }
            });
    });
}

fn button_sizes_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(18.0),
                row_gap: Val::Px(18.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                max_width: Val::Px(1360.0),
                ..default()
            },))
            .with_children(|parent| {
                for (i, size) in ["sm", "md", "lg"].iter().enumerate() {
                    parent.spawn((
                        Demo {
                            order: i as u32,
                            act: Act::Press,
                        },
                        button_styled("primary", size, *size),
                    ));
                }
            });
    });
}

fn tabs_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                width: Val::Px(520.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },))
            .with_children(|parent| {
                parent
                    .spawn(tabs(Some("overview".to_owned())))
                    .with_children(|parent| {
                        parent.spawn(tabs_list()).with_children(|parent| {
                            parent
                                .spawn((
                                    Demo {
                                        order: 0,
                                        act: Act::Click,
                                    },
                                    tabs_trigger("overview"),
                                ))
                                .with_children(|parent| {
                                    parent.spawn(text("Overview"));
                                });
                            parent
                                .spawn((
                                    Demo {
                                        order: 1,
                                        act: Act::Click,
                                    },
                                    tabs_trigger("activity"),
                                ))
                                .with_children(|parent| {
                                    parent.spawn(text("Activity"));
                                });
                            parent
                                .spawn((
                                    Demo {
                                        order: 2,
                                        act: Act::Click,
                                    },
                                    tabs_trigger("settings"),
                                ))
                                .with_children(|parent| {
                                    parent.spawn(text("Settings"));
                                });
                        });
                    });
            });
    });
}

fn checkbox_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(18.0),
                row_gap: Val::Px(18.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                max_width: Val::Px(1360.0),
                ..default()
            },))
            .with_children(|parent| {
                parent
                    .spawn((
                        Demo {
                            order: 0,
                            act: Act::Click,
                        },
                        checkbox(Check::Off),
                    ))
                    .with_children(|parent| {
                        parent.spawn(checkbox_indicator()).with_children(|parent| {
                            parent.spawn(text("✓"));
                        });
                    });
                parent.spawn(checkbox(Check::On)).with_children(|parent| {
                    parent.spawn(checkbox_indicator()).with_children(|parent| {
                        parent.spawn(text("✓"));
                    });
                });
                parent
                    .spawn(checkbox(Check::Indeterminate))
                    .with_children(|parent| {
                        parent.spawn(checkbox_indicator()).with_children(|parent| {
                            parent.spawn(text("−"));
                        });
                    });
            });
    });
}

fn switch_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(18.0),
                row_gap: Val::Px(18.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                max_width: Val::Px(1360.0),
                ..default()
            },))
            .with_children(|parent| {
                parent
                    .spawn((
                        Demo {
                            order: 0,
                            act: Act::Click,
                        },
                        switch(false),
                    ))
                    .with_children(|parent| {
                        parent.spawn(switch_thumb());
                    });
                parent.spawn(switch(true)).with_children(|parent| {
                    parent.spawn(switch_thumb());
                });
            });
    });
}

fn radio_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                width: Val::Px(240.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },))
            .with_children(|parent| {
                parent
                    .spawn(radio_group(Some("apple".to_owned())))
                    .with_children(|parent| {
                        parent
                            .spawn((
                                Demo {
                                    order: 0,
                                    act: Act::Click,
                                },
                                radio_item("apple"),
                            ))
                            .with_children(|parent| {
                                parent.spawn(radio_circle()).with_children(|parent| {
                                    parent.spawn(radio_indicator()).with_children(|parent| {
                                        parent.spawn(text("●"));
                                    });
                                });
                                parent.spawn(text("Apple"));
                            });
                        parent
                            .spawn((
                                Demo {
                                    order: 1,
                                    act: Act::Click,
                                },
                                radio_item("banana"),
                            ))
                            .with_children(|parent| {
                                parent.spawn(radio_circle()).with_children(|parent| {
                                    parent.spawn(radio_indicator()).with_children(|parent| {
                                        parent.spawn(text("●"));
                                    });
                                });
                                parent.spawn(text("Banana"));
                            });
                        parent
                            .spawn((
                                Demo {
                                    order: 2,
                                    act: Act::Click,
                                },
                                radio_item("cherry"),
                            ))
                            .with_children(|parent| {
                                parent.spawn(radio_circle()).with_children(|parent| {
                                    parent.spawn(radio_indicator()).with_children(|parent| {
                                        parent.spawn(text("●"));
                                    });
                                });
                                parent.spawn(text("Cherry"));
                            });
                    });
            });
    });
}

fn slider_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                width: Val::Px(360.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },))
            .with_children(|parent| {
                parent
                    .spawn((
                        Demo {
                            order: 0,
                            act: Act::Drag,
                        },
                        slider(35.0, 0.0, 100.0),
                    ))
                    .with_children(|parent| {
                        parent.spawn(slider_track()).with_children(|parent| {
                            parent.spawn(slider_range());
                            parent.spawn(slider_thumb());
                        });
                    });
            });
    });
}

fn progress_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                width: Val::Px(360.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },))
            .with_children(|parent| {
                parent
                    .spawn((
                        Demo {
                            order: 0,
                            act: Act::Fill,
                        },
                        progress(0.0, 100.0),
                    ))
                    .with_children(|parent| {
                        parent.spawn(progress_indicator());
                    });
            });
    });
}

fn avatar_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(18.0),
                row_gap: Val::Px(18.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                max_width: Val::Px(1360.0),
                ..default()
            },))
            .with_children(|parent| {
                parent.spawn(avatar()).with_children(|parent| {
                    parent.spawn(avatar_fallback()).with_children(|parent| {
                        parent.spawn(text_colored("KS", color::primary_on));
                    });
                });
                parent.spawn(avatar()).with_children(|parent| {
                    parent.spawn(avatar_fallback()).with_children(|parent| {
                        parent.spawn(text_colored("AB", color::primary_on));
                    });
                });
            });
    });
}

fn separator_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                width: Val::Px(360.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },))
            .with_children(|parent| {
                parent.spawn(text("Above"));
                parent.spawn(separator(Orientation::Horizontal));
                parent.spawn(text("Below"));
            });
    });
}

fn accordion_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                width: Val::Px(440.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },))
            .with_children(|parent| {
                parent
                    .spawn(accordion(HashSet::from(["shipping".to_owned()]), false))
                    .with_children(|parent| {
                        parent.spawn(accordion_item()).with_children(|parent| {
                            parent.spawn(accordion_header()).with_children(|parent| {
                                parent
                                    .spawn((
                                        Demo {
                                            order: 0,
                                            act: Act::Click,
                                        },
                                        accordion_trigger("shipping"),
                                    ))
                                    .with_children(|parent| {
                                        parent.spawn(text("Is shipping free?"));
                                    });
                            });
                            parent
                                .spawn(accordion_content("shipping"))
                                .with_children(|parent| {
                                    parent.spawn(accordion_body()).with_children(|parent| {
                                        parent.spawn(text_colored(
                                            "Yes, on orders over $50.",
                                            color::surface_canvas_on_soft,
                                        ));
                                    });
                                });
                        });
                        parent.spawn(accordion_item()).with_children(|parent| {
                            parent.spawn(accordion_header()).with_children(|parent| {
                                parent
                                    .spawn((
                                        Demo {
                                            order: 1,
                                            act: Act::Click,
                                        },
                                        accordion_trigger("returns"),
                                    ))
                                    .with_children(|parent| {
                                        parent.spawn(text("Can I return it?"));
                                    });
                            });
                            parent
                                .spawn(accordion_content("returns"))
                                .with_children(|parent| {
                                    parent.spawn(accordion_body()).with_children(|parent| {
                                        parent.spawn(text_colored(
                                            "Within 30 days, no questions.",
                                            color::surface_canvas_on_soft,
                                        ));
                                    });
                                });
                        });
                        parent.spawn(accordion_item()).with_children(|parent| {
                            parent.spawn(accordion_header()).with_children(|parent| {
                                parent
                                    .spawn((
                                        Demo {
                                            order: 2,
                                            act: Act::Click,
                                        },
                                        accordion_trigger("styled"),
                                    ))
                                    .with_children(|parent| {
                                        parent.spawn(text("Is it themed?"));
                                    });
                            });
                            parent
                                .spawn(accordion_content("styled"))
                                .with_children(|parent| {
                                    parent.spawn(accordion_body()).with_children(|parent| {
                                        parent.spawn(text_colored(
                                            "Every color comes from the theme.",
                                            color::surface_canvas_on_soft,
                                        ));
                                    });
                                });
                        });
                    });
            });
    });
}

fn collapsible_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                width: Val::Px(360.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },))
            .with_children(|parent| {
                parent.spawn(collapsible(false)).with_children(|parent| {
                    parent
                        .spawn((
                            Demo {
                                order: 0,
                                act: Act::Click,
                            },
                            collapsible_trigger(),
                        ))
                        .with_children(|parent| {
                            parent.spawn(text("Notification settings"));
                        });
                    parent.spawn(collapsible_content()).with_children(|parent| {
                        parent.spawn(text_colored(
                            "Email me about replies and mentions.",
                            color::surface_canvas_on_soft,
                        ));
                    });
                });
            });
    });
}

fn dialog_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent.spawn(dialog(false)).with_children(|parent| {
            parent
                .spawn((
                    Demo {
                        order: 0,
                        act: Act::Open,
                    },
                    dialog_trigger(),
                ))
                .with_children(|parent| {
                    parent.spawn(button_styled("soft", "md", "Delete project"));
                });
            parent.spawn(ui::dialog_modal()).with_children(|parent| {
                parent.spawn(dialog_scrim());
                parent.spawn(dialog_content()).with_children(|parent| {
                    parent.spawn(text("Delete project?"));
                    parent.spawn(text_colored(
                        "This permanently removes the project and its data.",
                        color::surface_canvas_on_soft,
                    ));
                    parent
                        .spawn((Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(12.0),
                            row_gap: Val::Px(12.0),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::FlexEnd,
                            flex_wrap: FlexWrap::Wrap,
                            max_width: Val::Px(1360.0),
                            ..default()
                        },))
                        .with_children(|parent| {
                            parent
                                .spawn((
                                    Demo {
                                        order: 1,
                                        act: Act::Close,
                                    },
                                    dialog_close(),
                                ))
                                .with_children(|parent| {
                                    parent.spawn(button_styled("plain", "md", "Cancel"));
                                });
                            parent.spawn(button_styled("danger", "md", "Delete"));
                        });
                });
            });
        });
    });
}

fn alert_dialog_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent.spawn(alert_dialog(false)).with_children(|parent| {
            parent
                .spawn((
                    Demo {
                        order: 0,
                        act: Act::Open,
                    },
                    alert_dialog_trigger(),
                ))
                .with_children(|parent| {
                    parent.spawn(button_styled("danger", "md", "Reset everything"));
                });
            parent
                .spawn(ui::alert_dialog_modal())
                .with_children(|parent| {
                    parent.spawn(alert_dialog_scrim());
                    parent
                        .spawn(alert_dialog_content())
                        .with_children(|parent| {
                            parent.spawn(text("Are you absolutely sure?"));
                            parent.spawn(text_colored(
                                "This action cannot be undone.",
                                color::surface_canvas_on_soft,
                            ));
                            parent
                                .spawn((Node {
                                    flex_direction: FlexDirection::Row,
                                    column_gap: Val::Px(12.0),
                                    row_gap: Val::Px(12.0),
                                    align_items: AlignItems::Center,
                                    justify_content: JustifyContent::FlexEnd,
                                    flex_wrap: FlexWrap::Wrap,
                                    max_width: Val::Px(1360.0),
                                    ..default()
                                },))
                                .with_children(|parent| {
                                    parent
                                        .spawn((
                                            Demo {
                                                order: 1,
                                                act: Act::Close,
                                            },
                                            alert_dialog_cancel(),
                                        ))
                                        .with_children(|parent| {
                                            parent.spawn(button_styled("plain", "md", "Cancel"));
                                        });
                                    parent.spawn(alert_dialog_action()).with_children(|parent| {
                                        parent.spawn(button_styled("primary", "md", "Continue"));
                                    });
                                });
                        });
                });
        });
    });
}

fn card_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(18.0),
                row_gap: Val::Px(18.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                max_width: Val::Px(1360.0),
                ..default()
            },))
            .with_children(|parent| {
                let variants = vec![
                    (
                        "Surface",
                        "Default, bordered",
                        color::surface_elevated_on,
                        CardOpts::default(),
                    ),
                    (
                        "Floating",
                        "Elevation shadow",
                        color::surface_elevated_on,
                        CardOpts {
                            floating: true,
                            ..default()
                        },
                    ),
                    (
                        "Compact",
                        "Tighter padding",
                        color::surface_elevated_on,
                        CardOpts {
                            compact: true,
                            ..default()
                        },
                    ),
                    (
                        "Interactive",
                        "Hover & press me",
                        color::surface_elevated_on,
                        CardOpts {
                            interactive: true,
                            ..default()
                        },
                    ),
                    (
                        "Floating + interactive",
                        "Lifts higher on hover",
                        color::surface_elevated_on,
                        CardOpts {
                            floating: true,
                            interactive: true,
                            ..default()
                        },
                    ),
                    (
                        "Success",
                        "Intent palette",
                        color::success_soft_on,
                        CardOpts {
                            intent: "success",
                            ..default()
                        },
                    ),
                    (
                        "Error",
                        "Intent palette",
                        color::error_soft_on,
                        CardOpts {
                            intent: "error",
                            ..default()
                        },
                    ),
                    (
                        "Info",
                        "Intent palette",
                        color::info_soft_on,
                        CardOpts {
                            intent: "info",
                            ..default()
                        },
                    ),
                    (
                        "Utility",
                        "Intent palette",
                        color::neutral_on,
                        CardOpts {
                            intent: "muted",
                            ..default()
                        },
                    ),
                ];
                for (title, desc, on, opts) in variants {
                    parent.spawn(card(opts)).with_children(|parent| {
                        parent
                            .spawn((Node {
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(4.0),
                                width: Val::Px(150.0),
                                ..default()
                            },))
                            .with_children(|parent| {
                                parent.spawn(text_colored(title, on));
                                parent.spawn(text_colored(desc, on));
                            });
                    });
                }
            });
    });
}

fn tooltip_scene(scene: &mut EntityCommands) {
    const SLOTS: [(f32, f32, Side); 5] = [
        (0.5, 0.02, Side::Top),
        (0.94, 0.5, Side::Right),
        (0.5, 0.95, Side::Bottom),
        (0.03, 0.5, Side::Left),
        (0.5, 0.5, Side::Bottom),
    ];
    const FLIP_NOTE: &str = "This floating panel flips to the opposite side when its preferred side would overflow the viewport.";

    let open: Option<usize> = None;

    scene.with_children(|parent| {
        parent
            .spawn(Node {
                position_type: PositionType::Relative,
                ..default()
            })
            .with_children(|parent| {
                for (i, (fx, fy, side)) in SLOTS.iter().enumerate() {
                    let left = (fx - 0.5) * WINDOW.x;
                    let top = (fy - 0.5) * WINDOW.y;
                    parent
                        .spawn(Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(left),
                            top: Val::Px(top),
                            ..default()
                        })
                        .with_children(|parent| {
                            parent
                                .spawn((
                                    Node::default(),
                                    Demo {
                                        order: i as u32,
                                        act: Act::Open,
                                    },
                                ))
                                .with_children(|parent| {
                                    parent
                                        .spawn((Node::default(), tooltip(open == Some(i))))
                                        .with_children(|parent| {
                                            parent.spawn(button_styled("soft", "md", "Hover me"));
                                            parent
                                                .spawn(tooltip_content(*side, Align::Center, 8.0))
                                                .with_children(|parent| {
                                                    parent
                                                        .spawn((Node {
                                                            width: Val::Px(220.0),
                                                            padding: UiRect::all(Val::Px(12.0)),
                                                            ..default()
                                                        },))
                                                        .with_children(|parent| {
                                                            parent.spawn(text(FLIP_NOTE));
                                                        });
                                                });
                                        });
                                });
                        });
                }
            });
    });
}

fn popover_scene(scene: &mut EntityCommands) {
    const SLOTS: [(f32, f32, Side); 5] = [
        (0.5, 0.02, Side::Top),
        (0.94, 0.5, Side::Right),
        (0.5, 0.95, Side::Bottom),
        (0.03, 0.5, Side::Left),
        (0.5, 0.5, Side::Bottom),
    ];
    const FLIP_NOTE: &str = "This floating panel flips to the opposite side when its preferred side would overflow the viewport.";

    let open: Option<usize> = None;

    scene.with_children(|parent| {
        parent
            .spawn(Node {
                position_type: PositionType::Relative,
                ..default()
            })
            .with_children(|parent| {
                for (i, (fx, fy, side)) in SLOTS.iter().enumerate() {
                    let left = (fx - 0.5) * WINDOW.x;
                    let top = (fy - 0.5) * WINDOW.y;
                    parent
                        .spawn(Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(left),
                            top: Val::Px(top),
                            ..default()
                        })
                        .with_children(|parent| {
                            parent
                                .spawn((
                                    Node::default(),
                                    Demo {
                                        order: i as u32,
                                        act: Act::Open,
                                    },
                                ))
                                .with_children(|parent| {
                                    parent
                                        .spawn((Node::default(), popover(open == Some(i))))
                                        .with_children(|parent| {
                                            parent.spawn(popover_trigger()).with_children(
                                                |parent| {
                                                    parent.spawn(button("Open"));
                                                },
                                            );
                                            parent
                                                .spawn(popover_content(*side, Align::Center, 8.0))
                                                .with_children(|parent| {
                                                    parent
                                                        .spawn((Node {
                                                            width: Val::Px(220.0),
                                                            flex_direction: FlexDirection::Column,
                                                            row_gap: Val::Px(8.0),
                                                            padding: UiRect::all(Val::Px(12.0)),
                                                            ..default()
                                                        },))
                                                        .with_children(|parent| {
                                                            parent.spawn(text("Dimensions"));
                                                            parent.spawn(text_colored(
                                                                FLIP_NOTE,
                                                                color::surface_canvas_on_soft,
                                                            ));
                                                        });
                                                });
                                        });
                                });
                        });
                }
            });
    });
}

fn tooltip_card_scene(scene: &mut EntityCommands) {
    const SLOTS: [(f32, f32, Side); 5] = [
        (0.5, 0.02, Side::Top),
        (0.94, 0.5, Side::Right),
        (0.5, 0.95, Side::Bottom),
        (0.03, 0.5, Side::Left),
        (0.5, 0.5, Side::Bottom),
    ];
    const FLIP_NOTE: &str = "This floating panel flips to the opposite side when its preferred side would overflow the viewport.";

    let open: Option<usize> = None;

    scene.with_children(|parent| {
        parent
            .spawn(Node {
                position_type: PositionType::Relative,
                ..default()
            })
            .with_children(|parent| {
                for (i, (fx, fy, side)) in SLOTS.iter().enumerate() {
                    let left = (fx - 0.5) * WINDOW.x;
                    let top = (fy - 0.5) * WINDOW.y;
                    parent
                        .spawn(Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(left),
                            top: Val::Px(top),
                            ..default()
                        })
                        .with_children(|parent| {
                            parent
                                .spawn((
                                    Node::default(),
                                    Demo {
                                        order: i as u32,
                                        act: Act::Open,
                                    },
                                ))
                                .with_children(|parent| {
                                    parent
                                        .spawn((Node::default(), tooltip(open == Some(i))))
                                        .with_children(|parent| {
                                            parent.spawn(button_styled("soft", "md", "Hover me"));
                                            parent
                                                .spawn(tooltip_content(*side, Align::Center, 8.0))
                                                .with_children(|parent| {
                                                    parent
                                                        .spawn(card(CardOpts {
                                                            floating: true,
                                                            ..default()
                                                        }))
                                                        .with_children(|parent| {
                                                            parent
                                                                .spawn((Node {
                                                                    width: Val::Px(220.0),
                                                                    padding: UiRect::all(Val::Px(
                                                                        12.0,
                                                                    )),
                                                                    ..default()
                                                                },))
                                                                .with_children(|parent| {
                                                                    parent.spawn(text(FLIP_NOTE));
                                                                });
                                                        });
                                                });
                                        });
                                });
                        });
                }
            });
    });
}

fn popover_card_scene(scene: &mut EntityCommands) {
    const SLOTS: [(f32, f32, Side); 5] = [
        (0.5, 0.02, Side::Top),
        (0.94, 0.5, Side::Right),
        (0.5, 0.95, Side::Bottom),
        (0.03, 0.5, Side::Left),
        (0.5, 0.5, Side::Bottom),
    ];
    const FLIP_NOTE: &str = "This floating panel flips to the opposite side when its preferred side would overflow the viewport.";

    let open: Option<usize> = None;

    scene.with_children(|parent| {
        parent.spawn(Node {
            position_type: PositionType::Relative,
            ..default()
        }).with_children(|parent| {
            for (i, (fx, fy, side)) in SLOTS.iter().enumerate() {
                let left = (fx - 0.5) * WINDOW.x;
                let top = (fy - 0.5) * WINDOW.y;
                parent.spawn(Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(left),
                    top: Val::Px(top),
                    ..default()
                }).with_children(|parent| {
                    parent
                        .spawn((
                            Node::default(),
                            Demo {
                                order: i as u32,
                                act: Act::Open,
                            },
                        ))
                        .with_children(|parent| {
                            parent
                                .spawn((Node::default(), popover(open == Some(i))))
                                .with_children(|parent| {
                                    parent.spawn(popover_trigger())
                                        .with_children(|parent| {
                                            parent.spawn(button("Open"));
                                        });
                                    parent
                                        .spawn(popover_content(*side, Align::Center, 8.0))
                                        .with_children(|parent| {
                                            parent
                                                .spawn(card(CardOpts {
                                                    floating: true,
                                                    ..default()
                                                }))
                                                .with_children(|parent| {
                                                    parent
                                                        .spawn((
                                                            Node {
                                                                width: Val::Px(220.0),
                                                                flex_direction: FlexDirection::Column,
                                                                row_gap: Val::Px(8.0),
                                                                padding: UiRect::all(Val::Px(12.0)),
                                                                ..default()
                                                            },
                                                        ))
                                                        .with_children(|parent| {
                                                            parent.spawn(text("Dimensions"));
                                                            parent.spawn(text_colored(FLIP_NOTE, color::surface_canvas_on_soft));
                                                        });
                                                });
                                        });
                                });
                        });
                });
            }
        });
    });
}

fn toasts_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent.spawn((
            Demo {
                order: 0,
                act: Act::Spawn,
            },
            button("Show toast"),
        ));
        parent.spawn((ToasterEntity, toaster(SonnerPosition::BottomRight)));
    });
}

fn scroll_area_scene(scene: &mut EntityCommands) {
    scene.with_children(|parent| {
        parent
            .spawn((Node {
                width: Val::Px(320.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },))
            .with_children(|parent| {
                parent
                    .spawn((
                        Demo {
                            order: 0,
                            act: Act::Click,
                        },
                        Node {
                            width: Val::Px(300.0),
                            height: Val::Px(220.0),
                            ..default()
                        },
                    ))
                    .with_children(|parent| {
                        parent.spawn(scroll_area()).with_children(|parent| {
                            parent.spawn(scroll_viewport()).with_children(|parent| {
                                parent
                                    .spawn((Node {
                                        flex_direction: FlexDirection::Column,
                                        row_gap: Val::Px(10.0),
                                        width: Val::Percent(100.0),
                                        padding: UiRect::all(Val::Px(8.0)),
                                        ..default()
                                    },))
                                    .with_children(|parent| {
                                        for n in 1..=16 {
                                            parent.spawn(text_colored(
                                                format!("Item {n}"),
                                                color::surface_canvas_on_soft,
                                            ));
                                        }
                                    });
                            });
                            parent.spawn(scroll_bar()).with_children(|parent| {
                                parent.spawn(scroll_thumb());
                            });
                        });
                    });
            });
    });
}
