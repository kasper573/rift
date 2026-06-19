//! Controlled state for the component library. A component never owns its open/value state: the app
//! passes the current value as a prop and a change-request callback, the component renders from that
//! value, and interaction calls the callback to request the next one — the app stays the single source
//! of truth.
//!
//! A multi-part component (a root with separate trigger/content/item parts) shares its value and
//! callback from the root to its parts through [`bevy_view::context`]: the root [`provide`]s a
//! [`Controlled`] on its element and each part reads it with [`controlled`]. Content that mounts only
//! for a value uses a [`gate`](bevy_view::gate) whose host is the part's parent — the element carrying
//! the context. Overlay parts that portal out of the hierarchy can't be reached by context; they read
//! their close callback from the instance-keyed [`Overlays`](crate::Overlays) store instead.

use std::collections::HashSet;
use std::sync::Arc;

use bevy_ecs::prelude::*;
use bevy_ui::{FlexDirection, Node, Val};
use bevy_view::{Element, InstanceId, View, boundary, context, node, provide};

/// A full-width vertical container with an `l` gap between its parts — the default root for controllers
/// whose parts (a trigger and its content, a list of options/sections) read top-to-bottom rather than
/// side by side.
fn column() -> Element {
    node().attr(|entity| {
        if let Some(mut node) = entity.get_mut::<Node>() {
            node.flex_direction = FlexDirection::Column;
            node.width = Val::Percent(100.0);
            node.row_gap = Val::Px(8.0);
        }
    })
}

/// A request to change a controlled value to `next`.
pub(crate) type OnChange<T> = Arc<dyn Fn(&mut World, T) + Send + Sync>;

/// The controlled value of type `T` in scope, paired with the callback that requests its next value.
/// Shared from a component root to its parts via context.
#[derive(Clone)]
pub(crate) struct Controlled<T: Clone + Send + Sync + 'static> {
    pub(crate) value: T,
    pub(crate) on_change: OnChange<T>,
}

impl<T: Clone + Send + Sync + 'static> Controlled<T> {
    pub(crate) fn request(&self, world: &mut World, next: T) {
        (self.on_change)(world, next);
    }
}

/// The default change callback when a component is given a value but no handler: requests nothing.
pub(crate) fn noop<T: 'static>() -> OnChange<T> {
    Arc::new(|_, _| {})
}

/// The controlled `T` in scope at `entity`, if a root above it provides one.
pub(crate) fn controlled<T: Clone + Send + Sync + 'static>(
    world: &World,
    entity: Entity,
) -> Option<Controlled<T>> {
    context::<Controlled<T>>(world, entity).cloned()
}

/// A component root: gives its parts a shared instance ([`boundary`]) and provides `Controlled<T>` on
/// `root` so they can read the value and request changes. `root` is the component's own element.
pub(crate) fn controller<T: Clone + Send + Sync + 'static>(
    root: Element,
    value: T,
    on_change: OnChange<T>,
    children: Vec<View>,
) -> View {
    boundary(
        root.bind(provide(Controlled { value, on_change }))
            .children(children),
    )
}

/// A [`controller`] rooted at a full-width vertical [`column`], so a root's parts stack top-to-bottom.
pub(crate) fn node_controller<T: Clone + Send + Sync + 'static>(
    value: T,
    on_change: OnChange<T>,
    children: Vec<View>,
) -> View {
    controller(column(), value, on_change, children)
}

/// A click handler that flips the controlled `bool` in scope (open↔closed, pressed↔released).
pub(crate) fn flip(world: &mut World, entity: Entity) {
    if let Some(control) = controlled::<bool>(world, entity) {
        let next = !control.value;
        control.request(world, next);
    }
}

/// A click handler that drives the controlled `bool` in scope to `open`.
pub(crate) fn drive(open: bool) -> impl Fn(&mut World, Entity) + Send + Sync + 'static {
    move |world, entity| {
        if let Some(control) = controlled::<bool>(world, entity) {
            control.request(world, open);
        }
    }
}

/// A gate test admitting overlay content while it is open *or* still easing out — the closing window
/// keeps it mounted past `open=false` so the exit animation can play (see `overlay_root`).
pub(crate) fn when_open(world: &World, instance: InstanceId, host: Entity) -> bool {
    let open = controlled::<bool>(world, host).is_some_and(|control| control.value);
    open || crate::instance_closing(world, instance)
}

/// A click handler that selects `value` as the controlled `Option<String>` in scope.
pub(crate) fn select(value: String) -> impl Fn(&mut World, Entity) + Send + Sync + 'static {
    move |world, entity| {
        if let Some(control) = controlled::<Option<String>>(world, entity) {
            control.request(world, Some(value.clone()));
        }
    }
}

/// A gate test admitting its body while the controlled `Option<String>` in scope equals `value`.
pub(crate) fn when_selected(
    value: String,
) -> impl Fn(&World, InstanceId, Entity) -> bool + Send + Sync + 'static {
    move |world, _, host| {
        controlled::<Option<String>>(world, host)
            .and_then(|control| control.value)
            .as_deref()
            == Some(value.as_str())
    }
}

/// The value an item contributes to its group's selection, provided by the item to the content or
/// indicator it owns so a sibling [`gate`](bevy_view::gate) can compare it against the group's value.
#[derive(Clone)]
pub(crate) struct ItemValue(pub(crate) String);

/// A controlled multi-selection (accordion / toggle group): the set of selected item values, whether
/// several may be selected at once, and the callback requesting the next set. Shared from the group to
/// its items via context.
#[derive(Clone)]
pub(crate) struct MultiControlled {
    pub(crate) values: HashSet<String>,
    pub(crate) multiple: bool,
    pub(crate) on_change: OnChange<HashSet<String>>,
}

/// A group root sharing a [`MultiControlled`] with its items via context.
pub(crate) fn multi_controller(
    values: HashSet<String>,
    multiple: bool,
    on_change: OnChange<HashSet<String>>,
    children: Vec<View>,
) -> View {
    boundary(
        column()
            .bind(provide(MultiControlled {
                values,
                multiple,
                on_change,
            }))
            .children(children),
    )
}

/// Toggles `value`'s membership in the multi-selection in scope and requests the next set (clearing the
/// rest first when only one may be selected).
fn toggle_value(world: &mut World, entity: Entity, value: String) {
    if let Some(group) = context::<MultiControlled>(world, entity).cloned() {
        let mut next = group.values.clone();
        if !next.remove(&value) {
            if !group.multiple {
                next.clear();
            }
            next.insert(value);
        }
        (group.on_change)(world, next);
    }
}

/// A click handler that toggles the membership of the [`ItemValue`] in scope — for a trigger separate
/// from the item that carries the value (an accordion trigger).
pub(crate) fn toggle_scope(world: &mut World, entity: Entity) {
    if let Some(value) = context::<ItemValue>(world, entity).map(|item| item.0.clone()) {
        toggle_value(world, entity, value);
    }
}
