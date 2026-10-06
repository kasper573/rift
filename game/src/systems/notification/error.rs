use std::time::Duration;

use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::tokens::{palette, typography};

use super::{Notification, NotificationKind, NotificationSent};

#[derive(Component, Default, Clone)]
pub(super) struct ErrorToaster;

pub fn toast(world: &mut World, notification_sent: NotificationSent) {
    let notification = notification_sent.notification;
    if !matches!(notification.kind, NotificationKind::Error { .. }) {
        return;
    }
    let Some(toaster) = world
        .query_filtered::<Entity, With<ErrorToaster>>()
        .iter(world)
        .next()
    else {
        return;
    };
    let shown = world
        .query::<(Entity, &ErrorRow, &ui::Toast)>()
        .iter(world)
        .find(|(_, _, toast)| !toast.leaving)
        .map(|(entity, row, _)| (entity, row.0 == notification));
    match shown {
        Some((again, true)) => {
            ui::bump_toast(world, again);
            return;
        }
        Some((replaced, false)) => {
            if let Some(mut toast) = world.get_mut::<ui::Toast>(replaced) {
                toast.leaving = true;
            }
        }
        None => {}
    }
    let lasts = Duration::from(notification.stays());
    let row = bsn! {
        {ui::toast(lasts)}
        ui::component(ErrorRow(notification.clone()))
        Children [ {EntityScene(content(&notification))} ]
    };
    if let Ok(mut spawned) = world.spawn_scene(row) {
        spawned.insert(ChildOf(toaster));
    }
}

pub fn shown(world: &mut World) -> Option<String> {
    world
        .query::<(&ErrorRow, &ui::Toast)>()
        .iter(world)
        .find(|(_, toast)| !toast.leaving)
        .map(|(row, _)| super::plain_words(&row.0.text))
}

#[derive(Component, Clone)]
struct ErrorRow(Notification);

fn content(notification: &Notification) -> impl Scene + use<> {
    let text = notification.text.rich();
    let line = ui::rich_text(
        ui::RichText {
            size: typography::BODY.font_size,
            color: palette::CRIMSON_80,
            ..text
        },
        false,
    );
    bsn! {
        Node { width: Val::Percent(100.0), height: Val::Percent(100.0), align_items: AlignItems::Center }
        Pickable::IGNORE
        Children [ {EntityScene(line)} ]
    }
}
