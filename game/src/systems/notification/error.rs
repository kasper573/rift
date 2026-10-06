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
    let repeated = world
        .query::<(Entity, &ErrorRow, &ui::Toast)>()
        .iter(world)
        .find(|(_, row, toast)| !toast.leaving && row.notification == notification)
        .map(|(entity, row, _)| (entity, row.repeats + 1));
    if let Some((row, repeats)) = repeated {
        world.entity_mut(row).despawn_related::<Children>();
        world.entity_mut(row).insert(ErrorRow {
            notification: notification.clone(),
            repeats,
        });
        if let Ok(mut content) = world.spawn_scene(content(&notification, repeats)) {
            content.insert(ChildOf(row));
        }
        ui::bump_toast(world, row);
        return;
    }
    let lasts = Duration::from(notification.stays());
    let row = bsn! {
        {ui::toast(lasts)}
        ui::component(ErrorRow { notification: notification.clone(), repeats: 1 })
        Children [ {EntityScene(content(&notification, 1))} ]
    };
    if let Ok(mut spawned) = world.spawn_scene(row) {
        spawned.insert(ChildOf(toaster));
    }
}

pub fn shown(world: &mut World) -> Vec<(String, u32)> {
    world
        .query::<(&ErrorRow, &ui::Toast)>()
        .iter(world)
        .filter(|(_, toast)| !toast.leaving)
        .map(|(row, _)| (super::plain_words(&row.notification.text), row.repeats))
        .collect()
}

#[derive(Component, Clone)]
struct ErrorRow {
    notification: Notification,
    repeats: u32,
}

fn content(notification: &Notification, repeats: u32) -> impl Scene + use<> {
    let mut text = notification.text.rich();
    if repeats > 1 {
        text.pieces
            .push(ui::RichSpan::plain(format!(" \u{d7}{repeats}")).into());
    }
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
