use std::time::Duration;

use bevy_camera::visibility::Visibility;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use bevy_picking::prelude::{Click, Out, Over, Pickable, Pointer, Press};
use bevy_scene::{Scene, bsn};
use bevy_time::Time;
use bevy_ui::{Display, GlobalZIndex, Node, PositionType, UiTransform, Val};

use crate::component;
use crate::motion::transition::{EMPHASIZED_ENTER, EMPHASIZED_EXIT};
use crate::motion::{Motion, Transform2d};
use crate::place::{Placed, Placement};
use crate::state::ancestor_with;
use bevy_opacity::Opacity;

pub(crate) const OVERLAY_EXIT: Duration = Duration::from_millis(240);
const OVERLAY_Z: i32 = 1000;

pub(crate) const POPPER_ENTER: Transform2d = Transform2d {
    translation: Vec2::ZERO,
    scale: Vec2::splat(0.94),
    rotation: 0.0,
};
pub(crate) const POPPER_EXIT: Transform2d = Transform2d {
    translation: Vec2::ZERO,
    scale: Vec2::splat(0.9),
    rotation: 0.0,
};

#[derive(Component, Clone, Copy, Default)]
#[require(Node)]
pub struct Open(pub bool);

#[derive(Component, Default, Clone, Copy)]
#[require(Node)]
pub struct Dismissable;

#[derive(Component, Clone, Copy)]
#[require(Node)]
pub enum OverlayAction {
    Open,
    Close,
    Toggle,
}

#[derive(Component, Default, Clone)]
#[require(Node)]
pub struct OverlayContent {
    pub enter: Transform2d,
    pub exit: Transform2d,
    pub(crate) closing_at: Option<Duration>,
    pub(crate) was_open: bool,
}

impl OverlayContent {
    pub fn animated(enter: Transform2d, exit: Transform2d) -> impl Scene {
        bsn! {
            component(OverlayContent { enter, exit, closing_at: None, was_open: false })
            Motion
            component(Opacity::new(0.0))
            UiTransform
        }
    }
}

#[derive(Component, Default, Clone, Copy)]
#[require(Node)]
pub struct Portal;

#[derive(Component)]
pub struct PortalOwner(pub Entity);

#[derive(Resource)]
pub(crate) struct OverlayHost(Entity);

pub(crate) fn spawn_overlay_host(mut commands: Commands) {
    let host = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..Node::default()
            },
            GlobalZIndex(OVERLAY_Z),
            Pickable::IGNORE,
        ))
        .id();
    commands.insert_resource(OverlayHost(host));
}

pub(crate) fn reparent_portals(
    host: Option<Res<OverlayHost>>,
    portals: Query<Entity, (With<Portal>, Without<PortalOwner>)>,
    parents: Query<&ChildOf>,
    mut commands: Commands,
) {
    let Some(host) = host else {
        return;
    };
    for entity in &portals {
        if let Ok(owner) = parents.get(entity).map(ChildOf::parent) {
            commands
                .entity(entity)
                .insert(PortalOwner(owner))
                .insert(ChildOf(host.0));
        }
    }
}

pub(crate) fn cleanup_portals(
    portals: Query<(Entity, &PortalOwner)>,
    entities: Query<Entity>,
    mut commands: Commands,
) {
    for (entity, owner) in &portals {
        if entities.get(owner.0).is_err() {
            commands.entity(entity).despawn();
        }
    }
}

fn open_holder(
    entity: Entity,
    parents: &Query<&ChildOf>,
    is_open: &Query<(), With<Open>>,
    owners: &Query<&PortalOwner>,
) -> Option<Entity> {
    let mut current = entity;
    loop {
        if is_open.get(current).is_ok() {
            return Some(current);
        }
        if let Ok(owner) = owners.get(current) {
            return Some(owner.0);
        }
        current = parents.get(current).ok()?.parent();
    }
}

pub fn set_overlay_open(world: &mut World, entity: Entity, open: bool) {
    let mut current = entity;
    loop {
        if world.get::<Open>(current).is_some() {
            world.entity_mut(current).insert(Open(open));
            return;
        }
        if let Some(&PortalOwner(owner)) = world.get::<PortalOwner>(current) {
            if world.get::<Open>(owner).is_some() {
                world.entity_mut(owner).insert(Open(open));
            }
            return;
        }
        match world.get::<ChildOf>(current) {
            Some(child_of) => current = child_of.parent(),
            None => return,
        }
    }
}

pub(crate) fn on_overlay_action(
    click: On<Pointer<Click>>,
    actions: Query<&OverlayAction>,
    parents: Query<&ChildOf>,
    has_action: Query<(), With<OverlayAction>>,
    is_open: Query<(), With<Open>>,
    owners: Query<&PortalOwner>,
    mut opens: Query<&mut Open>,
) {
    let Some(action_entity) = ancestor_with::<OverlayAction>(click.entity, &parents, &has_action)
    else {
        return;
    };
    let Ok(action) = actions.get(action_entity) else {
        return;
    };
    let Some(root) = open_holder(action_entity, &parents, &is_open, &owners) else {
        return;
    };
    if let Ok(mut open) = opens.get_mut(root) {
        open.0 = match action {
            OverlayAction::Open => true,
            OverlayAction::Close => false,
            OverlayAction::Toggle => !open.0,
        };
    }
}

pub(crate) fn dismiss_on_press(
    press: On<Pointer<Press>>,
    dismissables: Query<Entity, (With<Open>, With<Dismissable>)>,
    parents: Query<&ChildOf>,
    mut opens: Query<&mut Open>,
) {
    let target = press.entity;
    let dismissed: Vec<Entity> = dismissables
        .iter()
        .filter(|root| !contains(*root, target, &parents))
        .collect();
    for root in dismissed {
        if let Ok(mut open) = opens.get_mut(root) {
            open.0 = false;
        }
    }
}

fn contains(root: Entity, descendant: Entity, parents: &Query<&ChildOf>) -> bool {
    let mut current = descendant;
    loop {
        if current == root {
            return true;
        }
        match parents.get(current) {
            Ok(child_of) => current = child_of.parent(),
            Err(_) => return false,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn advance_overlays(
    time: Res<Time>,
    opens: Query<&Open>,
    parents: Query<&ChildOf>,
    is_open: Query<(), With<Open>>,
    owners: Query<&PortalOwner>,
    placeable: Query<(), With<Placement>>,
    placed: Query<(), With<Placed>>,
    mut contents: Query<(
        Entity,
        &mut OverlayContent,
        &mut Node,
        &mut Motion,
        &mut Visibility,
    )>,
) {
    let now = time.elapsed();
    for (entity, mut content, mut node, mut motion, mut visibility) in &mut contents {
        let open = open_holder(entity, &parents, &is_open, &owners)
            .and_then(|root| opens.get(root).ok())
            .is_some_and(|open| open.0);

        if open {
            content.closing_at = None;
            content.was_open = true;
        } else if content.was_open && content.closing_at.is_none() {
            content.closing_at = Some(now);
        }

        let closing = content
            .closing_at
            .is_some_and(|at| now.saturating_sub(at) < OVERLAY_EXIT);
        if !open && !closing {
            content.was_open = false;
            content.closing_at = None;
        }

        let display = if open || closing {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }

        let awaiting_placement = placeable.contains(entity) && !placed.contains(entity);
        let wanted = if awaiting_placement {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *visibility != wanted {
            *visibility = wanted;
        }

        if open && !awaiting_placement {
            motion.aim_opacity(0.0, 1.0, Some(EMPHASIZED_ENTER));
            motion.aim_transform(content.enter, Transform2d::IDENTITY, Some(EMPHASIZED_ENTER));
        } else if closing {
            motion.aim_opacity(0.0, 0.0, Some(EMPHASIZED_EXIT));
            motion.aim_transform(Transform2d::IDENTITY, content.exit, Some(EMPHASIZED_EXIT));
        } else {
            motion.aim_opacity(0.0, 0.0, None);
            motion.aim_transform(content.enter, content.enter, None);
        }
    }
}

#[derive(Component, Default, Clone)]
#[require(Node)]
pub struct TooltipTimer {
    pub delay: Duration,
    pub skip_delay: Duration,
    hovered_at: Option<Duration>,
}

impl TooltipTimer {
    pub fn new(delay: Duration, skip_delay: Duration) -> TooltipTimer {
        TooltipTimer {
            delay,
            skip_delay,
            hovered_at: None,
        }
    }
}

#[derive(Resource, Default)]
pub struct TooltipClock {
    last_closed: Option<Duration>,
}

pub(crate) fn tooltip_over(
    over: On<Pointer<Over>>,
    time: Res<Time>,
    clock: Res<TooltipClock>,
    parents: Query<&ChildOf>,
    has_timer: Query<(), With<TooltipTimer>>,
    mut timers: Query<&mut TooltipTimer>,
    mut opens: Query<&mut Open>,
) {
    let Some(root) = ancestor_with::<TooltipTimer>(over.entity, &parents, &has_timer) else {
        return;
    };
    let Ok(mut timer) = timers.get_mut(root) else {
        return;
    };
    let now = time.elapsed();
    let skip = clock
        .last_closed
        .is_some_and(|last| now.saturating_sub(last) < timer.skip_delay);
    if skip {
        timer.hovered_at = None;
        if let Ok(mut open) = opens.get_mut(root) {
            open.0 = true;
        }
    } else {
        timer.hovered_at = Some(now);
    }
}

pub(crate) fn tooltip_out(
    out: On<Pointer<Out>>,
    time: Res<Time>,
    mut clock: ResMut<TooltipClock>,
    parents: Query<&ChildOf>,
    has_timer: Query<(), With<TooltipTimer>>,
    mut timers: Query<&mut TooltipTimer>,
    mut opens: Query<&mut Open>,
) {
    let Some(root) = ancestor_with::<TooltipTimer>(out.entity, &parents, &has_timer) else {
        return;
    };
    let Ok(mut timer) = timers.get_mut(root) else {
        return;
    };
    timer.hovered_at = None;
    if let Ok(mut open) = opens.get_mut(root) {
        if open.0 {
            clock.last_closed = Some(time.elapsed());
        }
        open.0 = false;
    }
}

pub(crate) fn open_due_tooltips(
    time: Res<Time>,
    mut tooltips: Query<(&mut TooltipTimer, &mut Open)>,
) {
    let now = time.elapsed();
    for (mut timer, mut open) in &mut tooltips {
        if let Some(since) = timer.hovered_at
            && now.saturating_sub(since) >= timer.delay
        {
            timer.hovered_at = None;
            open.0 = true;
        }
    }
}
