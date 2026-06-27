use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;
use bevy_picking::prelude::{Out, Pointer, Press, Release};
use bevy_ui::{Checkable, Checked, Display, Node, Pressed};
use bevy_ui_widgets::Activate;

pub(crate) fn ancestor_with<C: Component>(
    entity: Entity,
    parents: &Query<&ChildOf>,
    has: &Query<(), With<C>>,
) -> Option<Entity> {
    let mut current = entity;
    loop {
        if has.get(current).is_ok() {
            return Some(current);
        }
        current = parents.get(current).ok()?.parent();
    }
}

#[derive(Component, Default, Clone)]
#[require(Node)]
pub struct SelectGroup {
    pub exclusive: bool,
    pub toggleable: bool,
    pub initial: Vec<String>,
}

pub(crate) fn init_selection(
    groups: Query<(Entity, &SelectGroup), Added<SelectGroup>>,
    items: Query<(Entity, &SelectItem)>,
    parents: Query<&ChildOf>,
    is_group: Query<(), With<SelectGroup>>,
    mut commands: Commands,
) {
    for (group, policy) in &groups {
        for (entity, item) in &items {
            if policy.initial.contains(&item.value)
                && ancestor_with::<SelectGroup>(entity, &parents, &is_group) == Some(group)
            {
                commands.entity(entity).insert(Checked);
            }
        }
    }
}

#[derive(Component, Default, Clone)]
#[require(Node)]
pub struct SelectItem {
    pub value: String,
}

#[derive(Component, Default, Clone)]
#[require(Node)]
pub struct SelectTrigger;

#[derive(EntityEvent)]
pub struct SelectionChanged {
    #[event_target]
    pub group: Entity,
}

pub(crate) fn on_select_activate(
    activate: On<Activate>,
    triggers: Query<&SelectItem, With<SelectTrigger>>,
    groups: Query<&SelectGroup>,
    items: Query<(Entity, &SelectItem, Has<Checked>)>,
    parents: Query<&ChildOf>,
    is_group: Query<(), With<SelectGroup>>,
    mut commands: Commands,
) {
    let Ok(clicked) = triggers.get(activate.entity) else {
        return;
    };
    let Some(group) = ancestor_with::<SelectGroup>(activate.entity, &parents, &is_group) else {
        return;
    };
    let Ok(policy) = groups.get(group) else {
        return;
    };
    let value = clicked.value.clone();
    let members: Vec<(Entity, bool, bool)> = items
        .iter()
        .filter(|(entity, ..)| {
            ancestor_with::<SelectGroup>(*entity, &parents, &is_group) == Some(group)
        })
        .map(|(entity, item, checked)| (entity, item.value == value, checked))
        .collect();
    let already_on = members
        .iter()
        .any(|(_, matches, checked)| *matches && *checked);

    if policy.toggleable && already_on {
        for (entity, matches, _) in &members {
            if *matches {
                commands.entity(*entity).remove::<Checked>();
            }
        }
    } else if policy.exclusive {
        for (entity, matches, _) in &members {
            if *matches {
                commands.entity(*entity).insert(Checked);
            } else {
                commands.entity(*entity).remove::<Checked>();
            }
        }
    } else {
        for (entity, matches, _) in &members {
            if *matches {
                commands.entity(*entity).insert(Checked);
            }
        }
    }
    commands.trigger(SelectionChanged { group });
}

pub fn selected(
    group: Entity,
    items: &Query<(Entity, &SelectItem, Has<Checked>)>,
    parents: &Query<&ChildOf>,
    is_group: &Query<(), With<SelectGroup>>,
) -> std::collections::HashSet<String> {
    items
        .iter()
        .filter(|(entity, _, checked)| {
            *checked && ancestor_with::<SelectGroup>(*entity, parents, is_group) == Some(group)
        })
        .map(|(_, item, _)| item.value.clone())
        .collect()
}

#[derive(Component, Default, Clone)]
#[require(Node)]
pub struct StartChecked(pub bool);

pub(crate) fn apply_start_checked(
    started: Query<(Entity, &StartChecked), Added<StartChecked>>,
    mut commands: Commands,
) {
    for (entity, start) in &started {
        if start.0 {
            commands.entity(entity).insert(Checked);
        }
        commands.entity(entity).remove::<StartChecked>();
    }
}

#[derive(Component, Default, Clone)]
#[require(Node)]
pub struct InheritChecked;

pub(crate) fn inherit_checked(
    inheritors: Query<(Entity, Has<Checked>), With<InheritChecked>>,
    controllables: Query<Has<Checked>, With<Checkable>>,
    parents: Query<&ChildOf>,
    is_checkable: Query<(), With<Checkable>>,
    mut commands: Commands,
) {
    for (entity, have) in &inheritors {
        let parent = parents.get(entity).map(ChildOf::parent);
        let Ok(parent) = parent else {
            continue;
        };
        let Some(root) = ancestor_with::<Checkable>(parent, &parents, &is_checkable) else {
            continue;
        };
        let want = controllables.get(root).unwrap_or(false);
        if want && !have {
            commands.entity(entity).insert(Checked);
        } else if !want && have {
            commands.entity(entity).remove::<Checked>();
        }
    }
}

#[derive(Component, Default, Clone)]
#[require(Node)]
pub struct Gated;

pub(crate) fn apply_gating(mut gated: Query<(&mut Node, Has<Checked>), With<Gated>>) {
    for (mut node, checked) in &mut gated {
        let want = if checked {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != want {
            node.display = want;
        }
    }
}

#[derive(Component, Default, Clone)]
#[require(Node)]
pub struct Pressable;

pub(crate) fn on_pressable_press(
    press: On<Pointer<Press>>,
    pressable: Query<(), With<Pressable>>,
    mut commands: Commands,
) {
    if pressable.contains(press.entity) {
        commands.entity(press.entity).insert(Pressed);
    }
}

pub(crate) fn on_pressable_release(
    release: On<Pointer<Release>>,
    pressable: Query<(), With<Pressable>>,
    mut commands: Commands,
) {
    if pressable.contains(release.entity) {
        commands.entity(release.entity).remove::<Pressed>();
    }
}

pub(crate) fn on_pressable_out(
    out: On<Pointer<Out>>,
    pressable: Query<(), With<Pressable>>,
    mut commands: Commands,
) {
    if pressable.contains(out.entity) {
        commands.entity(out.entity).remove::<Pressed>();
    }
}
