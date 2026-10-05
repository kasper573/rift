use std::time::Duration;

use bevy_ecs::prelude::*;
use bevy_input::ButtonInput;
use bevy_input::keyboard::KeyCode;
use bevy_scene::{EntityScene, Scene, bsn, on};
use bevy_time::Time;
use bevy_ui::{FlexDirection, JustifyContent, Node, Val};

use crate::component;
use crate::components::button::{ButtonSize, button_styled, intent};
use crate::components::dialog::modal;
use crate::components::text::styled_text;
use crate::drag::OnTap;
use crate::overlay::{OVERLAY_EXIT, Open};
use crate::theme::theme;
use crate::tokens::{spacing, typography};
use crate::{Activate, overlay};

pub struct ConfirmOptions {
    pub title: String,
    pub body: Vec<String>,
    pub confirm: String,
    pub cancel: String,
    pub on_confirm: OnTap,
    pub on_cancel: OnTap,
}

#[derive(Component, Clone, Copy, Default)]
pub struct Modal {
    opened: u64,
}

#[derive(Component, Clone)]
pub struct OnDismiss(pub OnTap);

#[derive(Component, Clone, Default)]
pub(crate) struct DespawnOnClose {
    closed_at: Option<Duration>,
}

#[derive(Resource, Default)]
pub(crate) struct ModalCounter(u64);

pub fn confirm_dialog(options: ConfirmOptions) -> impl Scene {
    let ConfirmOptions {
        title,
        body,
        confirm,
        cancel,
        on_confirm,
        on_cancel,
    } = options;
    let ink = theme().surface_elevated.on;
    let lines: Vec<Box<dyn Scene>> = body
        .into_iter()
        .map(|line| -> Box<dyn Scene> { Box::new(styled_text(line, ink, typography::BODY)) })
        .collect();
    let confirm_tap = on_confirm.0.clone();
    let cancel_tap = on_cancel.0.clone();
    let content = bsn! {
        Children [
            {EntityScene(styled_text(title, ink, typography::NAME))},
            ( Node { flex_direction: FlexDirection::Column, row_gap: Val::Px({spacing::M}) } Children [ {lines} ] ),
            (
                Node { justify_content: JustifyContent::End, column_gap: Val::Px({spacing::L}) }
                Children [
                    (
                        {button_styled(intent::DANGER, ButtonSize::Md, confirm)}
                        on(move |activate: On<Activate>, mut commands: Commands| {
                            let tap = confirm_tap.clone();
                            let button = activate.entity;
                            commands.queue(move |world: &mut World| {
                                overlay::set_overlay_open(world, button, false);
                                tap(world);
                            });
                        })
                    ),
                    (
                        {button_styled(intent::PRIMARY, ButtonSize::Md, cancel)}
                        on(move |activate: On<Activate>, mut commands: Commands| {
                            let tap = cancel_tap.clone();
                            let button = activate.entity;
                            commands.queue(move |world: &mut World| {
                                overlay::set_overlay_open(world, button, false);
                                tap(world);
                            });
                        })
                    ),
                ]
            ),
        ]
    };
    bsn! {
        {modal(true, false, bsn! { Node }, content)}
        component(OnDismiss(on_cancel))
        DespawnOnClose
    }
}

pub fn modal_open(world: &mut World) -> bool {
    topmost_modal(world).is_some()
}

pub fn dismiss_topmost(world: &mut World) -> bool {
    let Some(modal) = topmost_modal(world) else {
        return false;
    };
    world.entity_mut(modal).insert(Open(false));
    if let Some(OnDismiss(tap)) = world.get::<OnDismiss>(modal).cloned() {
        (tap.0)(world);
    }
    true
}

pub(crate) fn stamp_modals(
    mut counter: ResMut<ModalCounter>,
    mut modals: Query<(&mut Modal, &Open), Changed<Open>>,
) {
    for (mut modal, open) in &mut modals {
        if open.0 {
            counter.0 += 1;
            modal.opened = counter.0;
        }
    }
}

pub(crate) fn enter_keeps_default(keys: Res<ButtonInput<KeyCode>>, mut commands: Commands) {
    if keys.just_pressed(KeyCode::Enter) {
        commands.queue(|world: &mut World| {
            let confirming = topmost_modal(world)
                .is_some_and(|modal| world.get::<DespawnOnClose>(modal).is_some());
            if confirming {
                dismiss_topmost(world);
            }
        });
    }
}

pub(crate) fn despawn_closed(
    time: Res<Time>,
    mut closing: Query<(Entity, &Open, &mut DespawnOnClose)>,
    mut commands: Commands,
) {
    let now = time.elapsed();
    for (entity, open, mut despawn) in &mut closing {
        if open.0 {
            despawn.closed_at = None;
            continue;
        }
        let since = *despawn.closed_at.get_or_insert(now);
        if now.saturating_sub(since) >= OVERLAY_EXIT {
            commands.entity(entity).despawn();
        }
    }
}

fn topmost_modal(world: &mut World) -> Option<Entity> {
    world
        .query::<(Entity, &Modal, &Open)>()
        .iter(world)
        .filter(|(_, _, open)| open.0)
        .max_by_key(|(_, modal, _)| modal.opened)
        .map(|(entity, _, _)| entity)
}
