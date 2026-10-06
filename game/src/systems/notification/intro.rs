use std::borrow::Cow;
use std::time::Duration;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::{NotificationKind, NotificationSent};
use crate::systems::actor::Rgba;
use crate::systems::scene::Scene as GameScene;
use crate::systems::text::Motion;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct IntroAppearance {
    pub tint: Option<Rgba>,
    pub motion: Option<Motion>,
    pub emblem: Option<Cow<'static, str>>,
}

impl IntroAppearance {
    pub const PLAIN: IntroAppearance = IntroAppearance {
        tint: None,
        motion: None,
        emblem: None,
    };
}

pub struct IntroPlugin;

impl Plugin for IntroPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShownIntro>()
            .add_systems(Update, show_intro.run_if(in_state(GameScene::Area)));
    }
}

#[derive(Component, Default, Clone)]
pub(super) struct IntroHost;

pub fn show(world: &mut World, notification_sent: NotificationSent) {
    if !matches!(
        notification_sent.notification.kind,
        NotificationKind::Intro { .. }
    ) {
        return;
    }
    let stays = Duration::from(notification_sent.notification.stays());
    let mut shown = world.resource_mut::<ShownIntro>();
    let until = shown.clock + stays;
    shown.showing = Some(ShowingIntro {
        notification_sent,
        until,
        drawn: false,
    });
}

pub fn shown(world: &World) -> Option<(String, String)> {
    let showing = world.resource::<ShownIntro>().showing.as_ref()?;
    let notification = &showing.notification_sent.notification;
    let NotificationKind::Intro { title, .. } = &notification.kind else {
        return None;
    };
    Some((title.to_string(), super::plain_words(&notification.text)))
}

#[derive(Resource, Default)]
struct ShownIntro {
    showing: Option<ShowingIntro>,
    clock: Duration,
}

struct ShowingIntro {
    notification_sent: NotificationSent,
    until: Duration,
    drawn: bool,
}

fn show_intro(world: &mut World) {
    let held = super::rows::held::<ui::Intro>(world);
    let delta = world.resource::<Time>().delta();
    let mut shown = world.resource_mut::<ShownIntro>();
    if !held {
        shown.clock += delta;
    }
    let now = shown.clock;
    if shown
        .showing
        .as_ref()
        .is_some_and(|showing| showing.until <= now)
    {
        shown.showing = None;
    }
    let drawing = shown
        .showing
        .as_mut()
        .filter(|showing| !showing.drawn)
        .map(|showing| {
            showing.drawn = true;
            showing.notification_sent.notification.clone()
        });
    let leaving = shown.showing.is_none();
    if !leaving && drawing.is_none() {
        return;
    }
    let drawn: Vec<Entity> = world
        .query_filtered::<Entity, (With<ui::Intro>, Without<ui::Leaving>)>()
        .iter(world)
        .collect();
    for intro in drawn {
        world.entity_mut(intro).insert(ui::Leaving);
    }
    let Some((
        NotificationKind::Intro {
            title, appearance, ..
        },
        text,
    )) = drawing.map(|drawing| (drawing.kind, drawing.text))
    else {
        return;
    };
    let Some(host) = world
        .query_filtered::<Entity, With<IntroHost>>()
        .iter(world)
        .next()
    else {
        return;
    };
    let emblem = appearance
        .emblem
        .as_ref()
        .map(|emblem| world.resource::<AssetServer>().load(emblem.to_string()));
    let intro = ui::Intro {
        title: title.to_string(),
        text: text.rich(),
        tint: appearance.tint.map(Rgba::color),
        motion: appearance.motion.map(Motion::text_motion),
        emblem,
    };
    let stacked = bsn! {
        {ui::intro(intro)}
        Node { grid_row: {GridPlacement::start(1)}, grid_column: {GridPlacement::start(1)} }
    };
    if let Ok(mut spawned) = world.spawn_scene(stacked) {
        spawned.insert(ChildOf(host));
    }
}
