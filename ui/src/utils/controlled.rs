//! Controlled state — app is the single source of truth. Components share value/callback from root
//! to parts via context. Multi-part content mounts conditionally with gates. Portaled parts read
//! from the [`Overlays`](crate::Overlays) store instead.

use std::collections::HashSet;
use std::sync::Arc;

use bevy_ecs::prelude::*;
use bevy_ui::{FlexDirection, Node, Val};
use bevy_view::{Element, InstanceId, View, boundary, context, node, provide};

fn column() -> Element {
    node().attr(|entity| {
        if let Some(mut node) = entity.get_mut::<Node>() {
            node.flex_direction = FlexDirection::Column;
            node.width = Val::Percent(100.0);
            node.row_gap = Val::Px(8.0);
        }
    })
}

pub(crate) type OnChange<T> = Arc<dyn Fn(&mut World, T) + Send + Sync>;

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

pub(crate) fn noop<T: 'static>() -> OnChange<T> {
    Arc::new(|_, _| {})
}

pub(crate) fn controlled<T: Clone + Send + Sync + 'static>(
    world: &World,
    entity: Entity,
) -> Option<Controlled<T>> {
    context::<Controlled<T>>(world, entity).cloned()
}

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

pub(crate) fn node_controller<T: Clone + Send + Sync + 'static>(
    value: T,
    on_change: OnChange<T>,
    children: Vec<View>,
) -> View {
    controller(column(), value, on_change, children)
}

pub(crate) fn flip(world: &mut World, entity: Entity) {
    if let Some(control) = controlled::<bool>(world, entity) {
        let next = !control.value;
        control.request(world, next);
    }
}

pub(crate) fn drive(open: bool) -> impl Fn(&mut World, Entity) + Send + Sync + 'static {
    move |world, entity| {
        if let Some(control) = controlled::<bool>(world, entity) {
            control.request(world, open);
        }
    }
}

/// Gate test: admit while open or easing out (so exit animation plays).
pub(crate) fn when_open(world: &World, instance: InstanceId, host: Entity) -> bool {
    let open = controlled::<bool>(world, host).is_some_and(|control| control.value);
    open || crate::instance_closing(world, instance)
}

pub(crate) fn select(value: String) -> impl Fn(&mut World, Entity) + Send + Sync + 'static {
    move |world, entity| {
        if let Some(control) = controlled::<Option<String>>(world, entity) {
            control.request(world, Some(value.clone()));
        }
    }
}

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

#[derive(Clone)]
pub(crate) struct ItemValue(pub(crate) String);

#[derive(Clone)]
pub(crate) struct MultiControlled {
    pub(crate) values: HashSet<String>,
    pub(crate) multiple: bool,
    pub(crate) on_change: OnChange<HashSet<String>>,
}

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

pub(crate) fn toggle_scope(world: &mut World, entity: Entity) {
    if let Some(value) = context::<ItemValue>(world, entity).map(|item| item.0.clone()) {
        toggle_value(world, entity, value);
    }
}
