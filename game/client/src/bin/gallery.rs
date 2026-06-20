use std::collections::HashSet;

use bevy::prelude::*;
use bevy::ui::{ComputedNode, UiGlobalTransform};
use bevy_view::{View, ViewRoot, activate_click, activate_out, activate_over, image, node};
use ui::Text;
use ui::theme::{color, provide_theme};
use ui::{
    Accordion, AccordionContent, AccordionHeader, AccordionItem, AccordionTrigger, AlertDialog,
    AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription,
    AlertDialogOutlet, AlertDialogOverlay, AlertDialogTitle, AlertDialogTrigger, Avatar,
    AvatarFallback, Button, Card, Check, Checkbox, CheckboxIndicator, Collapsible,
    CollapsibleContent, CollapsibleTrigger, Dialog, DialogClose, DialogContent, DialogDescription,
    DialogOutlet, DialogOverlay, DialogTitle, DialogTrigger, PointerState, Popover, PopoverContent,
    PopoverOutlet, PopoverTrigger, Progress, ProgressIndicator, RadioGroup, RadioGroupIndicator,
    RadioGroupItem, ScrollArea, ScrollAreaScrollbar, ScrollAreaThumb, ScrollAreaViewport,
    Separator, Side, Slider, SliderRange, SliderThumb, SliderTrack, SonnerClose, SonnerPosition,
    Switch, SwitchThumb, Tabs, TabsList, TabsTrigger, Toast, Toaster, Tooltip, TooltipContent,
    TooltipOutlet, TooltipTrigger, dismiss_overlays, themes,
};

const WINDOW: Vec2 = Vec2::new(1600.0, 900.0);

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "rift ui gallery".to_owned(),
                resolution: WINDOW.as_uvec2().into(),
                decorations: false,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(
            color::surface_inset_base.resolve(&themes::light::theme()),
        ))
        .init_resource::<Stage>()
        .init_resource::<Director>()
        .init_resource::<Fps>()
        .add_plugins((bevy_view::ViewPlugin, ui::UiPlugin))
        .add_systems(Startup, setup)
        .add_systems(Update, (whiten_checkmark, update_fps))
        .add_systems(Update, (expire_toasts, direct, place_cursor).chain())
        .run();
}

// bevy tints UI images by multiplying, so the solid-black checkmark can't be tinted; recolor it white.
fn whiten_checkmark(mut images: ResMut<Assets<Image>>, stage: Res<Stage>, mut done: Local<bool>) {
    if *done {
        return;
    }
    if let Some(mut image) = images.get_mut(&stage.checkmark) {
        if let Some(data) = image.data.as_mut() {
            for pixel in data.chunks_exact_mut(4) {
                pixel[0] = 255;
                pixel[1] = 255;
                pixel[2] = 255;
            }
        }
        *done = true;
    }
}

#[derive(Resource, Default)]
struct Fps(f32);

fn update_fps(time: Res<Time>, mut fps: ResMut<Fps>) {
    let instant = 1.0 / time.delta_secs().max(1e-4);
    fps.0 = fps.0 * 0.9 + instant * 0.1;
}

#[derive(Resource)]
struct Stage {
    checkbox: Check,
    switch: bool,
    radio: Option<String>,
    tab: Option<String>,
    accordion: HashSet<String>,
    collapsible: bool,
    slider: f32,
    progress: f32,
    dialog: bool,
    alert: bool,
    tooltip: Option<usize>,
    popover: Option<usize>,
    toasts: Vec<LiveToast>,
    next_toast: u64,
    sonner_expanded: bool,
    sonner_position: SonnerPosition,
    checkmark: Handle<Image>,
}

impl Default for Stage {
    fn default() -> Stage {
        Stage {
            checkbox: Check::Off,
            switch: false,
            radio: Some("apple".to_owned()),
            tab: Some("overview".to_owned()),
            accordion: HashSet::from(["shipping".to_owned()]),
            collapsible: false,
            slider: 35.0,
            progress: 0.0,
            dialog: false,
            alert: false,
            tooltip: None,
            popover: None,
            toasts: Vec::new(),
            next_toast: 0,
            sonner_expanded: false,
            sonner_position: SonnerPosition::BottomRight,
            checkmark: Handle::default(),
        }
    }
}

struct LiveToast {
    id: u64,
    title: &'static str,
    body: &'static str,
    born: f32,
    leaving_at: Option<f32>,
}

const TOAST_TTL: f32 = 4.0;
const TOAST_EXIT: f32 = 0.4;

const TOAST_MESSAGES: &[(&str, &str)] = &[
    ("Event created", "Monday, January 6 at 9:00 AM"),
    ("Changes saved", "Your project is up to date."),
    ("Copied to clipboard", "The share link is ready."),
    ("Upload complete", "report-q3.pdf finished uploading."),
];

fn expire_toasts(time: Res<Time>, mut stage: ResMut<Stage>) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    let expanded = stage.sonner_expanded;
    for toast in &mut stage.toasts {
        if toast.leaving_at.is_some() {
            continue;
        }
        if expanded {
            toast.born += dt;
        } else if now - toast.born >= TOAST_TTL {
            toast.leaving_at = Some(now);
        }
    }
    stage
        .toasts
        .retain(|toast| toast.leaving_at.is_none_or(|at| now - at < TOAST_EXIT));
}

fn start_leaving(stage: &mut Stage, id: u64, now: f32) {
    if let Some(toast) = stage.toasts.iter_mut().find(|toast| toast.id == id) {
        toast.leaving_at.get_or_insert(now);
    }
}

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
    Spawn,
    Scroll,
    Fill,
}

#[derive(Component)]
struct CursorDot;

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
}

#[derive(Clone, Copy, PartialEq)]
enum Beat {
    Approach,
    Hover,
    Down,
    Up,
    Leave,
    Sweep,
    Emit,
    Hold,
    Filling,
    ScrollDown,
    ScrollUp,
    SetPosition,
    Expand,
    DismissMiddle,
    DismissOne,
    Collapse,
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
        Act::Scroll => &[(Approach, 0.4), (ScrollDown, 2.2), (ScrollUp, 2.2)],
    }
}

const SETTLE: f32 = 0.4;
const INTRO: f32 = 4.0;
const STATIC_HOLD: f32 = 2.6;

fn setup(
    mut commands: Commands,
    mut director: ResMut<Director>,
    mut stage: ResMut<Stage>,
    assets: Res<AssetServer>,
) {
    director.cursor = WINDOW / 2.0;
    director.settle = INTRO;
    director.autoplay = std::env::var("RIFT_AUTOPLAY").is_ok();
    if let Ok(scene) = std::env::var("RIFT_SCENE") {
        director.scene = scene.parse::<usize>().unwrap_or(0).min(SCENES.len() - 1);
        director.pinned = true;
        director.settle = SETTLE;
    }
    stage.checkmark = assets.load("icons/misc/checkmark.png");
    commands.spawn((Camera2d, IsDefaultUiCamera));
    commands.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        ViewRoot::new(gallery),
    ));
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

    if targets.is_empty() {
        dir.idle += dt;
        if dir.idle >= STATIC_HOLD {
            advance_scene(&mut dir);
        }
        world.insert_resource(dir);
        return;
    }
    dir.idle = 0.0;

    if dir.target >= targets.len() {
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
        Up if matches!(target.act, Act::Click | Act::Close | Act::Open) => {
            dir.pressed = false;
            for entity in descendants(world, target.entity) {
                activate_over(world, entity);
                activate_click(world, entity);
            }
        }
        Leave => {
            for entity in descendants(world, target.entity) {
                activate_out(world, entity);
            }
            dismiss_overlays(world, None);
        }
        Emit => spawn_toast(world),
        Expand => {
            let mut stage = world.resource_mut::<Stage>();
            stage.sonner_position = SonnerPosition::BottomRight;
            stage.sonner_expanded = true;
        }
        DismissMiddle => {
            let now = world.resource::<Time>().elapsed_secs();
            let mut stage = world.resource_mut::<Stage>();
            let count = stage.toasts.len();
            if count >= 2 {
                let id = stage.toasts[count / 2].id;
                start_leaving(&mut stage, id, now);
            }
        }
        DismissOne => {
            let now = world.resource::<Time>().elapsed_secs();
            let mut stage = world.resource_mut::<Stage>();
            if let Some(id) = stage
                .toasts
                .iter()
                .rev()
                .find(|toast| toast.leaving_at.is_none())
                .map(|toast| toast.id)
            {
                start_leaving(&mut stage, id, now);
            }
        }
        Collapse => {
            world.resource_mut::<Stage>().sonner_expanded = false;
        }
        SetPosition => {
            let mut stage = world.resource_mut::<Stage>();
            stage.sonner_expanded = false;
            stage.sonner_position = SonnerPosition::BottomRight;
            stage.toasts.clear();
        }
        _ => {}
    }
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
            world.resource_mut::<Stage>().slider = progress * 100.0;
        }

        Filling => {
            dir.cursor = target.center;
            dir.pressed = false;
            world.resource_mut::<Stage>().progress = progress * 100.0;
        }
        Emit | Hold => {
            dir.pressed = false;
        }
        ScrollDown | ScrollUp => {
            dir.cursor = target.center;
            let frac = if beat == ScrollDown {
                progress
            } else {
                1.0 - progress
            };
            set_scroll(world, target.entity, frac);
        }
        SetPosition => {
            let position = world.resource::<Stage>().sonner_position;
            dir.cursor = stack_cursor(position);
            dir.pressed = false;
        }
        Expand | Collapse => {
            let position = world.resource::<Stage>().sonner_position;
            dir.cursor = stack_cursor(position);
            dir.pressed = false;
        }
        DismissMiddle | DismissOne => {
            let position = world.resource::<Stage>().sonner_position;
            dir.cursor = stack_cursor(position);
            dir.pressed = true;
        }
    }
}

fn stack_cursor(_position: SonnerPosition) -> Vec2 {
    Vec2::new(0.87 * WINDOW.x, 0.88 * WINDOW.y)
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
        if let Some(mut state) = world.get_mut::<PointerState>(entity) {
            state.hovered = hovered;
            state.pressed = pressed;
        }
    }
}

fn clear_all_states(world: &mut World) {
    let mut query = world.query::<&mut PointerState>();
    for mut state in query.iter_mut(world) {
        state.hovered = false;
        state.pressed = false;
    }
}

fn set_scroll(world: &mut World, root: Entity, fraction: f32) {
    for entity in descendants(world, root) {
        let max = world
            .get::<ComputedNode>(entity)
            .map(|node| (node.content_size.y - node.size.y).max(0.0) * node.inverse_scale_factor);
        if let (Some(max), Some(mut scroll)) =
            (max, world.get_mut::<bevy::ui::ScrollPosition>(entity))
        {
            scroll.0.y = max * fraction;
        }
    }
}

fn spawn_toast(world: &mut World) {
    let now = world.resource::<Time>().elapsed_secs();
    let mut stage = world.resource_mut::<Stage>();
    let id = stage.next_toast;
    stage.next_toast += 1;
    let (title, body) = TOAST_MESSAGES[(id as usize) % TOAST_MESSAGES.len()];
    stage.toasts.push(LiveToast {
        id,
        title,
        body,
        born: now,
        leaving_at: None,
    });
}

fn place_cursor(
    director: Res<Director>,
    mut cursors: Query<(&mut Node, &mut BackgroundColor), With<CursorDot>>,
) {
    let Ok((mut node, mut bg)) = cursors.single_mut() else {
        return;
    };
    // The synthetic cursor only exists for the scripted clickthrough; hide it in manual mode.
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

type Build = fn(&World) -> View;
const SCENES: &[(&str, Build)] = &[
    ("Button intents", button_intents),
    ("Button sizes", button_sizes),
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
    ("Toasts (sonner)", sonner_scene),
    ("Scroll area", scroll_area_scene),
];

fn gallery(world: &World) -> View {
    let scene = world.resource::<Director>().scene.min(SCENES.len() - 1);
    let autoplay = world.resource::<Director>().autoplay;
    let (title, build) = SCENES[scene];
    let content = node().tag(0x5ce00 + scene as u64).children([build(world)]);

    let body: Vec<View> = if autoplay {
        let centered = node().attr(center_fill).children([content.into()]);
        vec![
            centered.into(),
            outlets(),
            toaster(world),
            corner_label(title),
        ]
    } else {
        let scene_area = node()
            .attr(|entity| {
                if let Some(mut node) = entity.get_mut::<Node>() {
                    node.width = Val::Percent(100.0);
                    node.flex_grow = 1.0;
                    node.align_items = AlignItems::Center;
                    node.justify_content = JustifyContent::Center;
                }
            })
            .children([content.into()]);
        vec![
            tabs_bar(world),
            scene_area.into(),
            outlets(),
            toaster(world),
        ]
    };

    let mut body = body;
    body.push(fps_overlay());

    node()
        .bind(provide_theme(themes::light::theme()))
        .attr(move |entity| {
            if let Some(mut node) = entity.get_mut::<Node>() {
                node.width = Val::Percent(100.0);
                node.height = Val::Percent(100.0);
                node.flex_direction = FlexDirection::Column;
                if autoplay {
                    node.align_items = AlignItems::Center;
                    node.justify_content = JustifyContent::Center;
                }
            }
        })
        .children(body)
        .into()
}

fn fps_overlay() -> View {
    node()
        .attr(|entity| {
            if let Some(mut node) = entity.get_mut::<Node>() {
                node.position_type = PositionType::Absolute;
                node.left = Val::Px(12.0);
                node.bottom = Val::Px(12.0);
            }
        })
        .child(
            Text::dynamic(|world| format!("{:.0} fps", world.resource::<Fps>().0))
                .color(Color::BLACK),
        )
        .into()
}

fn center_fill(entity: &mut bevy::ecs::world::EntityWorldMut) {
    if let Some(mut node) = entity.get_mut::<Node>() {
        node.position_type = PositionType::Absolute;
        node.width = Val::Percent(100.0);
        node.height = Val::Percent(100.0);
        node.align_items = AlignItems::Center;
        node.justify_content = JustifyContent::Center;
    }
}

fn tabs_bar(world: &World) -> View {
    let current = world.resource::<Director>().scene;
    let triggers: Vec<View> = SCENES
        .iter()
        .enumerate()
        .map(|(i, (name, _))| {
            TabsTrigger::default()
                .value(i.to_string())
                .child(Text::new(*name).intent("label"))
                .into()
        })
        .collect();
    let list = node()
        .attr(|entity| {
            if let Some(mut node) = entity.get_mut::<Node>() {
                node.flex_direction = FlexDirection::Row;
                node.flex_wrap = FlexWrap::Wrap;
                node.width = Val::Percent(100.0);
                node.justify_content = JustifyContent::Center;
            }
        })
        .children(triggers);
    Tabs::default()
        .value(current.to_string())
        .on_value_change(|world, value| {
            if let Some(index) = value.and_then(|value| value.parse::<usize>().ok()) {
                world.resource_mut::<Director>().scene = index.min(SCENES.len() - 1);
            }
        })
        .child(list)
        .into()
}

fn outlets() -> View {
    View::fragment([
        DialogOutlet.into(),
        AlertDialogOutlet.into(),
        TooltipOutlet.into(),
        PopoverOutlet.into(),
    ])
}

fn toaster(world: &World) -> View {
    let stage = world.resource::<Stage>();
    let toasts = stage
        .toasts
        .iter()
        .map(|toast| {
            let (title, body) = (toast.title, toast.body);
            Toast::new(toast.id, move || toast_content(title, body))
                .leaving(toast.leaving_at.is_some())
        })
        .collect();
    Toaster::default()
        .position(stage.sonner_position)
        .expanded(stage.sonner_expanded)
        .toasts(toasts)
        .on_dismiss(|world, id| {
            let now = world.resource::<Time>().elapsed_secs();
            start_leaving(&mut world.resource_mut::<Stage>(), id, now);
        })
        .on_expand_change(|world, expanded| {
            world.resource_mut::<Stage>().sonner_expanded = expanded;
        })
        .into()
}

fn toast_content(title: &'static str, body: &'static str) -> View {
    View::fragment([
        node()
            .attr(|entity| {
                if let Some(mut node) = entity.get_mut::<Node>() {
                    node.flex_direction = FlexDirection::Row;
                    node.justify_content = JustifyContent::SpaceBetween;
                    node.align_items = AlignItems::Center;
                    node.column_gap = Val::Px(12.0);
                }
            })
            .child(
                Text::new(title)
                    .intent("body_strong")
                    .color(color::surface_elevated_on),
            )
            .child(
                SonnerClose::default()
                    .child(Button::default().variant("soft").size("sm").label("close")),
            )
            .into(),
        Text::new(body)
            .intent("body_small")
            .color(color::surface_canvas_on_soft)
            .into(),
    ])
}

fn corner_label(title: &str) -> View {
    node()
        .attr(|entity| {
            if let Some(mut node) = entity.get_mut::<Node>() {
                node.position_type = PositionType::Absolute;
                node.top = Val::Px(40.0);
                node.left = Val::Px(48.0);
            }
        })
        .child(
            Text::new(title)
                .intent("headline_small")
                .color(color::surface_canvas_on),
        )
        .into()
}

fn demo(order: u32, act: Act, child: impl Into<View>) -> View {
    node().insert(Demo { order, act }).child(child).into()
}

fn demo_wide(order: u32, act: Act, child: impl Into<View>) -> View {
    node()
        .insert(Demo { order, act })
        .attr(|entity| {
            if let Some(mut node) = entity.get_mut::<Node>() {
                node.width = Val::Percent(100.0);
            }
        })
        .child(child)
        .into()
}

fn ink(content: &str) -> View {
    Text::new(content).color(color::surface_canvas_on).into()
}

fn muted(content: &str) -> View {
    Text::new(content)
        .intent("body_small")
        .color(color::surface_canvas_on_soft)
        .into()
}

fn mark(content: &str) -> View {
    Text::new(content).color(color::primary_on).into()
}

fn checkmark(world: &World) -> View {
    let handle = world.resource::<Stage>().checkmark.clone();
    let tint = color::primary_on.resolve(&themes::light::theme());
    image(ImageNode {
        color: tint,
        ..ImageNode::new(handle)
    })
    .attr(|entity| {
        if let Some(mut node) = entity.get_mut::<Node>() {
            node.width = Val::Px(15.0);
            node.height = Val::Px(15.0);
        }
    })
    .into()
}

fn row(children: impl IntoIterator<Item = View>) -> View {
    laid_out(children, FlexDirection::Row, 18.0, JustifyContent::Center)
}

fn column(children: impl IntoIterator<Item = View>) -> View {
    laid_out(
        children,
        FlexDirection::Column,
        12.0,
        JustifyContent::Center,
    )
}

fn laid_out(
    children: impl IntoIterator<Item = View>,
    direction: FlexDirection,
    gap: f32,
    justify: JustifyContent,
) -> View {
    node()
        .attr(move |entity| {
            if let Some(mut node) = entity.get_mut::<Node>() {
                node.flex_direction = direction;
                node.column_gap = Val::Px(gap);
                node.row_gap = Val::Px(gap);
                node.align_items = AlignItems::Center;
                node.justify_content = justify;
                node.flex_wrap = FlexWrap::Wrap;
                node.max_width = Val::Px(1360.0);
            }
        })
        .children(children)
        .into()
}

fn framed(width: f32, child: impl Into<View>) -> View {
    node()
        .attr(move |entity| {
            if let Some(mut node) = entity.get_mut::<Node>() {
                node.width = Val::Px(width);
                node.flex_direction = FlexDirection::Column;
                node.align_items = AlignItems::Center;
            }
        })
        .child(child)
        .into()
}

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

fn button_intents(_: &World) -> View {
    row(INTENTS.iter().enumerate().map(|(i, intent)| {
        demo(
            i as u32,
            Act::Press,
            Button::default().variant(intent).label(*intent),
        )
    }))
}

fn button_sizes(_: &World) -> View {
    row(["sm", "md", "lg"].into_iter().enumerate().map(|(i, size)| {
        demo(
            i as u32,
            Act::Press,
            Button::default().size(size).label(size),
        )
    }))
}

fn tabs_scene(world: &World) -> View {
    let active = world.resource::<Stage>().tab.clone();
    let trigger = |order: u32, value: &'static str, label: &'static str| {
        demo(
            order,
            Act::Click,
            TabsTrigger::default().value(value).child(ink(label)),
        )
    };
    framed(
        520.0,
        Tabs::default()
            .value(active)
            .on_value_change(|world, next| world.resource_mut::<Stage>().tab = next)
            .child(
                TabsList::default()
                    .child(trigger(0, "overview", "Overview"))
                    .child(trigger(1, "activity", "Activity"))
                    .child(trigger(2, "settings", "Settings")),
            ),
    )
}

fn checkbox_scene(world: &World) -> View {
    let driven = world.resource::<Stage>().checkbox;
    let still = |state: Check| {
        Checkbox::default()
            .checked(state)
            .child(CheckboxIndicator::default().child(checkmark(world)))
            .into()
    };
    row([
        demo(
            0,
            Act::Click,
            Checkbox::default()
                .checked(driven)
                .on_checked_change(|world, next| world.resource_mut::<Stage>().checkbox = next)
                .child(CheckboxIndicator::default().child(checkmark(world))),
        ),
        still(Check::On),
        still(Check::Indeterminate),
    ])
}

fn switch_scene(world: &World) -> View {
    let on = world.resource::<Stage>().switch;
    row([
        demo(
            0,
            Act::Click,
            Switch::default()
                .checked(on)
                .on_checked_change(|world, next| world.resource_mut::<Stage>().switch = next)
                .child(SwitchThumb),
        ),
        Switch::default().checked(true).child(SwitchThumb).into(),
    ])
}

fn radio_scene(world: &World) -> View {
    let selected = world.resource::<Stage>().radio.clone();
    let item = |order: u32, value: &'static str, label: &'static str| {
        demo(
            order,
            Act::Click,
            RadioGroupItem::default()
                .value(value)
                .label(ink(label))
                .child(RadioGroupIndicator::default().child(checkmark(world))),
        )
    };
    framed(
        240.0,
        RadioGroup::default()
            .value(selected)
            .on_value_change(|world, next| world.resource_mut::<Stage>().radio = next)
            .child(item(0, "apple", "Apple"))
            .child(item(1, "banana", "Banana"))
            .child(item(2, "cherry", "Cherry")),
    )
}

fn slider_scene(world: &World) -> View {
    let value = world.resource::<Stage>().slider;
    framed(
        360.0,
        demo_wide(
            0,
            Act::Drag,
            Slider::default()
                .value(value)
                .max(100.0)
                .on_value_change(|world, next| world.resource_mut::<Stage>().slider = next)
                .child(SliderTrack::default().child(SliderRange).child(SliderThumb)),
        ),
    )
}

fn progress_scene(world: &World) -> View {
    let value = world.resource::<Stage>().progress;
    framed(
        360.0,
        demo_wide(
            0,
            Act::Fill,
            Progress::default()
                .value(value)
                .max(100.0)
                .child(ProgressIndicator),
        ),
    )
}

fn avatar_scene(_: &World) -> View {
    row([
        Avatar::default()
            .child(AvatarFallback::default().child(mark("KS")))
            .into(),
        Avatar::default()
            .child(AvatarFallback::default().child(mark("AB")))
            .into(),
    ])
}

fn separator_scene(_: &World) -> View {
    framed(
        360.0,
        column([ink("Above"), Separator::default().into(), ink("Below")]),
    )
}

fn accordion_scene(world: &World) -> View {
    let open = world.resource::<Stage>().accordion.clone();
    let item = |order: u32, value: &'static str, header: &'static str, body: &'static str| {
        AccordionItem::default()
            .value(value)
            .child(AccordionHeader::default().child(demo(
                order,
                Act::Click,
                AccordionTrigger::default().child(ink(header)),
            )))
            .child(AccordionContent::default().child(muted(body)))
    };
    framed(
        440.0,
        Accordion::default()
            .value(open)
            .on_value_change(|world, next| world.resource_mut::<Stage>().accordion = next)
            .child(item(
                0,
                "shipping",
                "Is shipping free?",
                "Yes, on orders over $50.",
            ))
            .child(item(
                1,
                "returns",
                "Can I return it?",
                "Within 30 days, no questions.",
            ))
            .child(item(
                2,
                "styled",
                "Is it themed?",
                "Every color comes from the theme.",
            )),
    )
}

fn collapsible_scene(world: &World) -> View {
    let open = world.resource::<Stage>().collapsible;
    framed(
        360.0,
        Collapsible::default()
            .open(open)
            .on_open_change(|world, next| world.resource_mut::<Stage>().collapsible = next)
            .child(demo(
                0,
                Act::Click,
                CollapsibleTrigger::default().child(ink("Notification settings")),
            ))
            .child(
                CollapsibleContent::default().child(muted("Email me about replies and mentions.")),
            ),
    )
}

fn dialog_scene(world: &World) -> View {
    let open = world.resource::<Stage>().dialog;
    demo(
        0,
        Act::Open,
        Dialog::default()
            .open(open)
            .on_open_change(|world, next| world.resource_mut::<Stage>().dialog = next)
            .child(
                DialogTrigger::default()
                    .child(Button::default().variant("soft").label("Delete project")),
            )
            .child(DialogOverlay::default())
            .child(
                DialogContent::default()
                    .child(DialogTitle::default().child(ink("Delete project?")))
                    .child(
                        DialogDescription::default()
                            .child(muted("This permanently removes the project and its data.")),
                    )
                    .child(actions([
                        demo(
                            1,
                            Act::Close,
                            DialogClose::default()
                                .child(Button::default().variant("plain").label("Cancel")),
                        ),
                        Button::default().variant("danger").label("Delete").into(),
                    ])),
            ),
    )
}

fn alert_dialog_scene(world: &World) -> View {
    let open = world.resource::<Stage>().alert;
    demo(
        0,
        Act::Open,
        AlertDialog::default()
            .open(open)
            .on_open_change(|world, next| world.resource_mut::<Stage>().alert = next)
            .child(
                AlertDialogTrigger::default().child(
                    Button::default()
                        .variant("danger")
                        .label("Reset everything"),
                ),
            )
            .child(AlertDialogOverlay::default())
            .child(
                AlertDialogContent::default()
                    .child(AlertDialogTitle::default().child(ink("Are you absolutely sure?")))
                    .child(
                        AlertDialogDescription::default()
                            .child(muted("This action cannot be undone.")),
                    )
                    .child(actions([
                        demo(
                            1,
                            Act::Close,
                            AlertDialogCancel::default()
                                .child(Button::default().variant("plain").label("Cancel")),
                        ),
                        AlertDialogAction::default()
                            .child(Button::default().variant("primary").label("Continue"))
                            .into(),
                    ])),
            ),
    )
}

/// The five trigger positions for the overlay scenes: one hugging each viewport edge (so its preferred
/// side overflows and the overlay flips), plus one in the middle as the happy case.
// Positions as fractions of the viewport, so the showcase adapts to any resolution or aspect ratio.
const SLOTS: [(f32, f32, Side); 5] = [
    (0.5, 0.02, Side::Top),
    (0.94, 0.5, Side::Right),
    (0.5, 0.95, Side::Bottom),
    (0.03, 0.5, Side::Left),
    (0.5, 0.5, Side::Bottom),
];

fn edged(open: Option<usize>, build: impl Fn(usize, Side, bool) -> View) -> View {
    let mut layer = node();
    for (i, (fx, fy, side)) in SLOTS.iter().enumerate() {
        let (left, top) = ((fx - 0.5) * WINDOW.x, (fy - 0.5) * WINDOW.y);
        let slot = node()
            .attr(move |entity| {
                if let Some(mut node) = entity.get_mut::<Node>() {
                    node.position_type = PositionType::Absolute;
                    node.left = Val::Px(left);
                    node.top = Val::Px(top);
                }
            })
            .child(demo(i as u32, Act::Open, build(i, *side, open == Some(i))));
        layer = layer.child(slot);
    }
    layer.into()
}

const FLIP_NOTE: &str = "This floating panel flips to the opposite side when its preferred side would overflow the viewport.";

fn tooltip_scene(world: &World) -> View {
    let open = world.resource::<Stage>().tooltip;
    edged(open, |index, side, is_open| {
        Tooltip::default()
            .open(is_open)
            .on_open_change(move |world, opened| {
                world.resource_mut::<Stage>().tooltip = opened.then_some(index);
            })
            .child(
                TooltipTrigger::default()
                    .child(Button::default().variant("soft").label("Hover me")),
            )
            .child(
                TooltipContent::default()
                    .side(side)
                    .child(framed(220.0, ink(FLIP_NOTE))),
            )
            .into()
    })
}

fn popover_scene(world: &World) -> View {
    let open = world.resource::<Stage>().popover;
    edged(open, |index, side, is_open| {
        Popover::default()
            .open(is_open)
            .on_open_change(move |world, opened| {
                world.resource_mut::<Stage>().popover = opened.then_some(index);
            })
            .child(PopoverTrigger::default().child(Button::default().label("Open")))
            .child(
                PopoverContent::default()
                    .side(side)
                    .child(framed(220.0, column([ink("Dimensions"), muted(FLIP_NOTE)]))),
            )
            .into()
    })
}

fn card_scene(_: &World) -> View {
    use ui::theme::ColorVar;
    // bevy has no text-colour inheritance, so the content takes the card's on-colour explicitly.
    let variant = |title: &'static str, desc: &'static str, on: ColorVar, card: Card| {
        card.child(
            node()
                .attr(|entity| {
                    if let Some(mut node) = entity.get_mut::<Node>() {
                        node.flex_direction = FlexDirection::Column;
                        node.row_gap = Val::Px(4.0);
                        node.width = Val::Px(150.0);
                    }
                })
                .child(Text::new(title).intent("body_strong").color(on))
                .child(Text::new(desc).intent("body_small").color(on)),
        )
        .into()
    };
    let surface_on = color::surface_elevated_on;
    row([
        variant("Surface", "Default, bordered", surface_on, Card::default()),
        variant(
            "Floating",
            "Elevation shadow",
            surface_on,
            Card::default().floating(true),
        ),
        variant(
            "Compact",
            "Tighter padding",
            surface_on,
            Card::default().compact(true),
        ),
        variant(
            "Interactive",
            "Hover & press me",
            surface_on,
            Card::default().interactive(true),
        ),
        variant(
            "Floating + interactive",
            "Lifts higher on hover",
            surface_on,
            Card::default().floating(true).interactive(true),
        ),
        variant(
            "Success",
            "Intent palette",
            color::success_soft_on,
            Card::default().intent("success"),
        ),
        variant(
            "Error",
            "Intent palette",
            color::error_soft_on,
            Card::default().intent("error"),
        ),
        variant(
            "Info",
            "Intent palette",
            color::info_soft_on,
            Card::default().intent("info"),
        ),
        variant(
            "Utility",
            "Intent palette",
            color::neutral_on,
            Card::default().intent("muted"),
        ),
    ])
}

// `Tooltip`/`Popover` carry no appearance: bare content floats next to the trigger (text in space). The
// `+ card` scenes compose the same content inside a `Card` for a surface — the flexible split where the
// popper only handles when to show, and the app composes what.

fn tooltip_card_scene(world: &World) -> View {
    let open = world.resource::<Stage>().tooltip;
    edged(open, |index, side, is_open| {
        Tooltip::default()
            .open(is_open)
            .on_open_change(move |world, opened| {
                world.resource_mut::<Stage>().tooltip = opened.then_some(index);
            })
            .child(
                TooltipTrigger::default()
                    .child(Button::default().variant("soft").label("Hover me")),
            )
            .child(
                TooltipContent::default().side(side).child(
                    Card::default()
                        .floating(true)
                        .child(framed(220.0, ink(FLIP_NOTE))),
                ),
            )
            .into()
    })
}

fn popover_card_scene(world: &World) -> View {
    let open = world.resource::<Stage>().popover;
    edged(open, |index, side, is_open| {
        Popover::default()
            .open(is_open)
            .on_open_change(move |world, opened| {
                world.resource_mut::<Stage>().popover = opened.then_some(index);
            })
            .child(PopoverTrigger::default().child(Button::default().label("Open")))
            .child(
                PopoverContent::default().side(side).child(
                    Card::default()
                        .floating(true)
                        .child(framed(220.0, column([ink("Dimensions"), muted(FLIP_NOTE)]))),
                ),
            )
            .into()
    })
}

fn sonner_scene(_: &World) -> View {
    // The button spawns a toast on a real click; the director also spawns directly in autoplay.
    demo(
        0,
        Act::Spawn,
        node()
            .on_click_with(|world, _| spawn_toast(world))
            .child(Button::default().variant("soft").label("Show toast")),
    )
}

fn scroll_area_scene(_: &World) -> View {
    let list = node()
        .attr(|entity| {
            if let Some(mut node) = entity.get_mut::<Node>() {
                node.flex_direction = FlexDirection::Column;
                node.row_gap = Val::Px(10.0);
                node.width = Val::Percent(100.0);
                node.padding = UiRect::all(Val::Px(8.0));
            }
        })
        .children((1..=16).map(|n| muted(&format!("Item {n}"))));
    framed(
        320.0,
        demo(
            0,
            Act::Scroll,
            node()
                .attr(|entity| {
                    if let Some(mut node) = entity.get_mut::<Node>() {
                        node.width = Val::Px(300.0);
                        node.height = Val::Px(220.0);
                    }
                })
                .child(
                    ScrollArea::default()
                        .child(ScrollAreaViewport::default().child(list))
                        .child(ScrollAreaScrollbar::default().child(ScrollAreaThumb)),
                ),
        ),
    )
}

fn actions(children: impl IntoIterator<Item = View>) -> View {
    laid_out(children, FlexDirection::Row, 12.0, JustifyContent::End)
}
