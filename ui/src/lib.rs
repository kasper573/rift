//! A small Radix-inspired UI component library over [`bevy_view`].
//!
//! Every component is **controlled**: it never owns its open/value state. The app passes the current
//! value as a prop and a change-request callback; the component renders from that value and calls the
//! callback on interaction. [`controlled`] shows how a component shares its value and callback from its
//! root to its separate parts. Overlays additionally float their content into an app-placed outlet —
//! draw order comes from where the outlet sits in the tree, not a z-index — and position it
//! collision-aware against its trigger; the math lives in [`place`] and is unit-tested apart from
//! layout.

/// Generates the `child` builder method shared by every component part — the seam the `view!` macro's
/// `<Comp>…children…</Comp>` lowering targets.
macro_rules! children_builder {
    ($ty:ty) => {
        impl $ty {
            pub fn child(mut self, child: impl Into<::bevy_view::View>) -> $ty {
                self.children.push(child.into());
                self
            }
        }
    };
}

/// Generates a variant-selection builder method per named dimension, each pushing its choice onto the
/// component's `variants: Vec<(&'static str, &'static str)>` for its [`recipe`] to resolve. The method
/// name is the dimension key, so `<Button size="lg"/>` lowers to `.size("lg")` → `("size", "lg")`.
macro_rules! variant_props {
    ($ty:ty { $($dimension:ident),+ $(,)? }) => {
        impl $ty {
            $(
                pub fn $dimension(mut self, option: &'static str) -> $ty {
                    self.variants.push((stringify!($dimension), option));
                    self
                }
            )+
        }
    };
}

mod components;
mod utils;

pub mod themes;
pub mod tokens;

use std::collections::HashMap;
use std::time::Duration;

use bevy_app::{App, Plugin, PostUpdate, PreUpdate, Startup};
use bevy_asset::{AssetServer, Handle};
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::message::MessageReader;
use bevy_ecs::prelude::*;
use bevy_input::mouse::{MouseScrollUnit, MouseWheel};
use bevy_math::Vec2;
use bevy_picking::hover::HoverMap;
use bevy_picking::prelude::{Pickable, Pointer, Press};
use bevy_text::Font;
use bevy_ui::{
    ComputedNode, Node, PositionType, ScrollPosition, UiGlobalTransform, UiSystems, Val,
};
use bevy_window::Window;

use bevy_view::{Element, Instance, InstanceId, PortalKind, View, instance_of, node, outlet};

use utils::controlled::{OnChange, controller};
use utils::interaction::{on_out, on_over, on_press, on_release};
use utils::motion::MotionPlugin;

pub use components::*;
pub use utils::interaction::PointerState;
pub use utils::motion::{Easing, Timing, Transform2d, transition};
pub use utils::recipe::{Paint, Recipe, Style, Styled};
pub use utils::theme;

// Internal module paths the components still reach for as `crate::recipe` / `crate::controlled`.
pub(crate) use utils::{collapse, controlled, interaction, motion, popper, recipe};

/// Drives overlay positioning, outside-press dismissal, and tooltip open timing, and registers the
/// [`Overlays`] store overlays coordinate through. Add it alongside [`bevy_view::ViewPlugin`].
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MotionPlugin)
            .init_resource::<Overlays>()
            .add_message::<MouseWheel>()
            .add_systems(Startup, load_fonts)
            .add_systems(PreUpdate, scroll_hovered)
            .add_systems(
                PostUpdate,
                (
                    position_overlays.after(UiSystems::Layout),
                    sync_scrollbars.after(UiSystems::Layout),
                    collapse::advance_collapse.after(UiSystems::Layout),
                    open_due_tooltips,
                    advance_overlay_close,
                ),
            )
            .add_observer(dismiss_on_press)
            .add_observer(on_over)
            .add_observer(on_out)
            .add_observer(on_press)
            .add_observer(on_release);
    }
}

/// Keeps the library's fonts loaded so [`Text`] can resolve them by family.
#[derive(Resource)]
struct DesignFonts(#[allow(dead_code)] Vec<Handle<Font>>);

/// Loads the library's fonts (which [`Text`] matches by family) from the conventional `fonts/`
/// asset directory and keeps them alive — so any app on the library gets text without wiring fonts up.
fn load_fonts(assets: Option<Res<AssetServer>>, mut commands: Commands) {
    let Some(assets) = assets else {
        return;
    };
    let handles = [
        "fonts/circular-400-normal.ttf",
        "fonts/circular-500-normal.ttf",
        "fonts/circular-700-normal.ttf",
        "fonts/lato-400-normal.ttf",
        "fonts/lato-700-normal.ttf",
    ]
    .iter()
    .map(|path| assets.load(*path))
    .collect();
    commands.insert_resource(DesignFonts(handles));
}

/// Which side of the trigger an overlay's content prefers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Side {
    Top,
    Right,
    #[default]
    Bottom,
    Left,
}

/// How content aligns along the trigger's cross axis.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
}

/// The axis a component lays out or divides along.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Orientation {
    #[default]
    Horizontal,
    Vertical,
}

/// Per-instance overlay coordination, keyed by the instance id its parts share. The open state itself
/// lives with the app (the component is controlled); this only holds what the parts can't pass to each
/// other directly — the positioning anchor, the close callback portaled content needs, and tooltip
/// hover timing.
#[derive(Default)]
pub(crate) struct Overlay {
    pub(crate) anchor: Option<Entity>,
    pub(crate) on_open_change: Option<OnChange<bool>>,
    pub(crate) hover_at: Option<Duration>,
    pub(crate) delay: Duration,
    /// Whether the overlay was open on the previous render — so the root can spot an open→closed edge,
    /// whatever path triggered it (a close button, a press outside, leaving a hover trigger).
    pub(crate) was_open: bool,
    /// When set, the overlay is playing its exit: the content stays mounted and eases out (even though
    /// `open` is already `false`), until [`advance_overlay_close`] drops it once the exit has played.
    pub(crate) closing_at: Option<Duration>,
}

/// How long an overlay's content lingers, easing out, after a close is requested.
const OVERLAY_EXIT: Duration = Duration::from_millis(240);

#[derive(Resource, Default)]
pub(crate) struct Overlays {
    pub(crate) states: HashMap<InstanceId, Overlay>,
    pub(crate) last_tooltip_closed: Option<Duration>,
}

/// Marks floating content a press outside should dismiss (anchored popovers and menus, not tooltips).
#[derive(Component, Clone, Copy)]
pub(crate) struct Dismissable;

/// How a piece of content is placed against its anchor; read by [`position_overlays`].
#[derive(Component, Clone, Copy)]
pub(crate) struct Placement {
    pub(crate) side: Side,
    pub(crate) align: Align,
    pub(crate) offset: f32,
}

/// Builds an overlay root: a [`controller`] sharing the controlled `bool` (open) with its in-tree
/// trigger and content gate via context, plus the open-change callback registered in [`Overlays`] so
/// portaled content (which leaves the hierarchy) can still close it. Forgets the instance on unmount.
pub(crate) fn overlay_root(
    open: bool,
    on_open_change: OnChange<bool>,
    children: Vec<View>,
) -> View {
    let registered = on_open_change.clone();
    controller(
        node()
            .on_mount_with(move |world, entity| {
                let cb = registered.clone();
                set_overlay(world, entity, move |overlay| {
                    overlay.on_open_change = Some(cb)
                });
            })
            .on_cleanup_with(forget)
            .attr(move |entity| {
                // Track the open edge every render: opening cancels any pending exit; closing (after
                // having been open) starts one, so whatever closed it, the content lingers to ease out.
                let id = entity.id();
                entity.world_scope(|world| {
                    let now = overlay_now(world);
                    set_overlay(world, id, move |overlay| {
                        if open {
                            overlay.closing_at = None;
                            overlay.was_open = true;
                        } else if overlay.was_open && overlay.closing_at.is_none() {
                            overlay.closing_at = Some(now);
                        }
                    });
                });
            }),
        open,
        on_open_change,
        children,
    )
}

/// Applies `change` to the overlay state of the instance `entity` belongs to, creating it if absent.
pub(crate) fn set_overlay(world: &mut World, entity: Entity, change: impl FnOnce(&mut Overlay)) {
    if let Some(instance) = instance_of(world, entity) {
        change(
            world
                .resource_mut::<Overlays>()
                .states
                .entry(instance)
                .or_default(),
        );
    }
}

pub(crate) fn overlay(world: &World, instance: InstanceId) -> Option<&Overlay> {
    world
        .get_resource::<Overlays>()
        .and_then(|overlays| overlays.states.get(&instance))
}

/// Records `entity` as its overlay instance's positioning anchor (a trigger's `on_mount`/`on_over`).
pub(crate) fn register_anchor(world: &mut World, entity: Entity) {
    set_overlay(world, entity, |overlay| overlay.anchor = Some(entity));
}

/// Requests the overlay containing `entity` to close — for a portaled close button or menu item, which
/// can't reach the in-tree controlled `bool` by context. Sets `open=false`; the root then notices the
/// edge and holds the content mounted to ease it out.
pub(crate) fn close_overlay(world: &mut World, entity: Entity) {
    let close = instance_of(world, entity)
        .and_then(|instance| overlay(world, instance).and_then(|o| o.on_open_change.clone()));
    if let Some(close) = close {
        close(world, false);
    }
}

/// True while the entity's overlay is in its closing (exit-animating) window.
pub(crate) fn overlay_closing(world: &World, entity: Entity) -> bool {
    instance_of(world, entity)
        .and_then(|instance| overlay(world, instance))
        .is_some_and(|overlay| overlay.closing_at.is_some())
}

/// True while the instance is in its closing (exit-animating) window — keyed by instance, for the gate.
pub(crate) fn instance_closing(world: &World, instance: InstanceId) -> bool {
    overlay(world, instance).is_some_and(|overlay| overlay.closing_at.is_some())
}

/// Once a closing overlay's exit has had time to play, drops the closing mark (its `open` is already
/// `false`, so the content then unmounts). Runs every frame; available for scripted activation in tests.
pub fn advance_overlay_close(world: &mut World) {
    let now = overlay_now(world);
    let due: Vec<InstanceId> = world
        .resource::<Overlays>()
        .states
        .iter()
        .filter_map(|(id, overlay)| {
            let at = overlay.closing_at?;
            (now.saturating_sub(at) >= OVERLAY_EXIT).then_some(*id)
        })
        .collect();
    let mut overlays = world.resource_mut::<Overlays>();
    for id in due {
        if let Some(overlay) = overlays.states.get_mut(&id) {
            overlay.closing_at = None;
            overlay.was_open = false;
        }
    }
}

fn overlay_now(world: &World) -> Duration {
    world
        .get_resource::<bevy_time::Time>()
        .map(|time| time.elapsed())
        .unwrap_or_default()
}

/// Gives overlay content a show/hide animation: it fades its whole subtree (via [`Opacity`](motion)) in
/// from `enter` and, while its overlay is closing, out to `exit`. It stays mounted through the exit
/// because the overlay root defers the unmount. Pass `IDENTITY`/`IDENTITY` for a fade with no scale
/// (e.g. a full-screen backdrop, which must keep covering the viewport).
pub(crate) fn exit_on_close(
    element: bevy_view::Element,
    enter: motion::Transform2d,
    exit: motion::Transform2d,
) -> bevy_view::Element {
    use motion::transition::{EMPHASIZED_ENTER, EMPHASIZED_EXIT};
    // The content's opacity and transform are driven *only* here (the recipe deliberately doesn't touch
    // them) so one consistent target reaches the motion each frame and the tween actually runs: toward
    // the resting look while open, toward zero opacity + `exit` while closing.
    element.attr(move |entity| {
        let id = entity.id();
        let closing = entity.world_scope(|world| overlay_closing(world, id));
        if entity.get::<motion::Motion>().is_none() {
            entity.insert(motion::Motion::default());
        }
        if entity.get::<motion::Opacity>().is_none() {
            entity.insert(motion::Opacity(0.0));
        }
        if entity.get::<bevy_ui::UiTransform>().is_none() {
            entity.insert(bevy_ui::UiTransform::default());
        }
        let mut motion = entity.get_mut::<motion::Motion>().expect("just inserted");
        if closing {
            motion.aim_opacity(0.0, 0.0, Some(EMPHASIZED_EXIT));
            motion.aim_transform(motion::Transform2d::IDENTITY, exit, Some(EMPHASIZED_EXIT));
        } else {
            motion.aim_opacity(0.0, 1.0, Some(EMPHASIZED_ENTER));
            motion.aim_transform(enter, motion::Transform2d::IDENTITY, Some(EMPHASIZED_ENTER));
        }
    })
}

/// The scale a floating panel grows in from when it opens.
pub(crate) const POPPER_ENTER: motion::Transform2d = motion::Transform2d {
    translation: Vec2::ZERO,
    scale: Vec2::splat(0.94),
    rotation: 0.0,
};
/// The scale a floating panel shrinks out to when it closes.
pub(crate) const POPPER_EXIT: motion::Transform2d = motion::Transform2d {
    translation: Vec2::ZERO,
    scale: Vec2::splat(0.9),
    rotation: 0.0,
};

/// Scrolls whatever scrollable node is under the pointer by the mouse wheel — bevy_ui tracks the offset
/// in [`ScrollPosition`] but doesn't drive it from input, so this walks up from the hovered entity to the
/// nearest scrollable and moves it.
fn scroll_hovered(
    mut wheel: MessageReader<MouseWheel>,
    hover_map: Option<Res<HoverMap>>,
    parents: Query<&ChildOf>,
    mut scrollables: Query<&mut ScrollPosition>,
) {
    let Some(hover_map) = hover_map else {
        return;
    };
    let delta: f32 = wheel
        .read()
        .map(|event| match event.unit {
            MouseScrollUnit::Line => event.y * 20.0,
            MouseScrollUnit::Pixel => event.y,
        })
        .sum();
    if delta == 0.0 {
        return;
    }
    for hits in hover_map.values() {
        for &hovered in hits.keys() {
            let mut entity = hovered;
            loop {
                if let Ok(mut scroll) = scrollables.get_mut(entity) {
                    scroll.0.y = (scroll.0.y - delta).max(0.0);
                    break;
                }
                match parents.get(entity) {
                    Ok(child_of) => entity = child_of.parent(),
                    Err(_) => break,
                }
            }
        }
    }
}

/// Drops an instance's overlay state — wired to a root's cleanup so a removed overlay leaves nothing.
pub(crate) fn forget(world: &mut World, entity: Entity) {
    if let Some(instance) = instance_of(world, entity) {
        world.resource_mut::<Overlays>().states.remove(&instance);
    }
}

/// Wraps `children` in an absolutely positioned floating node carrying its placement. Dismissable
/// content is closed by a press outside it; tooltip content additionally ignores picking.
pub(crate) fn floating(
    placement: Placement,
    ignore_picking: bool,
    dismissable: bool,
    children: Vec<View>,
) -> Element {
    let mut element = node()
        .attr(|entity| {
            if let Some(mut node) = entity.get_mut::<Node>() {
                node.position_type = PositionType::Absolute;
            }
        })
        .insert(placement)
        .children(children);
    if dismissable {
        element = element.insert(Dismissable);
    }
    if ignore_picking {
        element = element.insert(Pickable::IGNORE);
    }
    element
}

/// A full-screen, click-through portal sink for anchored overlays. [`position_overlays`] writes content
/// `left`/`top` in viewport coordinates, so the outlet has to span the viewport from the origin for an
/// absolutely-positioned child to land where intended; an unsized outlet in a centered layout would
/// push it off-screen. `Pickable::IGNORE` lets presses fall through the empty area to the scene below.
pub(crate) fn overlay_outlet(kind: PortalKind) -> Element {
    outlet(kind)
        .attr(|entity| {
            if let Some(mut node) = entity.get_mut::<Node>() {
                node.position_type = PositionType::Absolute;
                node.top = Val::Px(0.0);
                node.left = Val::Px(0.0);
                node.width = Val::Percent(100.0);
                node.height = Val::Percent(100.0);
            }
        })
        .insert(Pickable::IGNORE)
}

/// Closes every dismissable overlay the press landed outside of — exposed for scripted activation; the
/// global press observer calls it with the hit entity. An overlay is open exactly while its content is
/// mounted (carrying [`Dismissable`] and the instance). A press carrying an open overlay's own instance
/// (its trigger or content) leaves it open; `None` (empty space) closes them all.
pub fn dismiss_overlays(world: &mut World, pressed: Option<Entity>) {
    let inside = pressed.and_then(|entity| instance_of(world, entity));
    let open: Vec<InstanceId> = world
        .query_filtered::<&Instance, With<Dismissable>>()
        .iter(world)
        .map(|instance| instance.id())
        .filter(|id| Some(*id) != inside)
        .collect();
    for id in open {
        let close = overlay(world, id).and_then(|overlay| overlay.on_open_change.clone());
        if let Some(close) = close {
            close(world, false);
        }
    }
}

/// A press anywhere dismisses the overlays it landed outside of.
pub(crate) fn dismiss_on_press(press: On<Pointer<Press>>, mut commands: Commands) {
    let target = press.entity;
    commands.queue(move |world: &mut World| dismiss_overlays(world, Some(target)));
}

/// Positions every mounted overlay content against its anchor, collision-aware. Runs after layout, so
/// it reads measured rects; the written `Node.left/top` settle on the next frame.
fn position_overlays(
    contents: Query<(Entity, &Placement, &Instance)>,
    measured: Query<(&ComputedNode, &UiGlobalTransform)>,
    overlays: Res<Overlays>,
    windows: Query<&Window>,
    mut nodes: Query<&mut Node>,
) {
    let Some(window) = windows.iter().next() else {
        return;
    };
    let viewport = window.size();
    for (entity, placement, instance) in &contents {
        let Some(anchor) = overlays.states.get(&instance.id()).and_then(|o| o.anchor) else {
            continue;
        };
        let (Ok((anchor_node, anchor_transform)), Ok((content_node, _))) =
            (measured.get(anchor), measured.get(entity))
        else {
            continue;
        };
        let anchor_size = anchor_node.size * anchor_node.inverse_scale_factor;
        let anchor_center = anchor_transform.translation * anchor_node.inverse_scale_factor;
        let anchor_pos = anchor_center - anchor_size / 2.0;
        let content_size = content_node.size * content_node.inverse_scale_factor;
        let pos = place(
            anchor_pos,
            anchor_size,
            content_size,
            viewport,
            placement.side,
            placement.align,
            placement.offset,
        );
        if let Ok(mut node) = nodes.get_mut(entity) {
            node.left = Val::Px(pos.x);
            node.top = Val::Px(pos.y);
        }
    }
}

// --- Positioning math (pure, unit-tested) -----------------------------------------------------

/// Places content of size `content` against the `anchor` rectangle within `viewport`, preferring
/// `side`/`align`/`offset` but flipping to the opposite side when the preferred one overflows and
/// clamping into the viewport. All values are logical pixels; returns the content's top-left.
pub fn place(
    anchor_pos: Vec2,
    anchor_size: Vec2,
    content: Vec2,
    viewport: Vec2,
    side: Side,
    align: Align,
    offset: f32,
) -> Vec2 {
    let preferred = anchored(side, anchor_pos, anchor_size, content, align, offset);
    let pos = if overflows(side, preferred, content, viewport) {
        let opposite = opposite(side);
        let alternate = anchored(opposite, anchor_pos, anchor_size, content, align, offset);
        if overflows(opposite, alternate, content, viewport) {
            preferred
        } else {
            alternate
        }
    } else {
        preferred
    };
    Vec2::new(
        pos.x.clamp(0.0, (viewport.x - content.x).max(0.0)),
        pos.y.clamp(0.0, (viewport.y - content.y).max(0.0)),
    )
}

fn anchored(side: Side, pos: Vec2, size: Vec2, content: Vec2, align: Align, offset: f32) -> Vec2 {
    let cross = |start: f32, extent: f32, content: f32| match align {
        Align::Start => start,
        Align::Center => start + (extent - content) / 2.0,
        Align::End => start + extent - content,
    };
    match side {
        Side::Bottom => Vec2::new(cross(pos.x, size.x, content.x), pos.y + size.y + offset),
        Side::Top => Vec2::new(cross(pos.x, size.x, content.x), pos.y - content.y - offset),
        Side::Right => Vec2::new(pos.x + size.x + offset, cross(pos.y, size.y, content.y)),
        Side::Left => Vec2::new(pos.x - content.x - offset, cross(pos.y, size.y, content.y)),
    }
}

fn overflows(side: Side, pos: Vec2, content: Vec2, viewport: Vec2) -> bool {
    match side {
        Side::Bottom => pos.y + content.y > viewport.y,
        Side::Top => pos.y < 0.0,
        Side::Right => pos.x + content.x > viewport.x,
        Side::Left => pos.x < 0.0,
    }
}

fn opposite(side: Side) -> Side {
    match side {
        Side::Top => Side::Bottom,
        Side::Bottom => Side::Top,
        Side::Left => Side::Right,
        Side::Right => Side::Left,
    }
}
