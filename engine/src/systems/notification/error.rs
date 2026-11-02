use std::time::Duration;
use ui::egui;

use bevy::prelude::*;
use ui::dusk;
use ui::egui::emath::TSTransform;
use ui::egui::{Rect, Vec2};

use super::rows::{self, Clock, NoticePlace};
use super::{Notification, NotificationSent};
use crate::core::text;
use ui::motion::{Swell, Transit, TransitMove, Transits};

const WIDEST: f32 = 356.0;
const SIZE: f32 = 16.0;
const TRANSIT: Transit = Transit {
    enter: TransitMove::new(Vec2::new(0.0, -100.0), 0.9, 0.25),
    exit: TransitMove::new(Vec2::new(0.0, -30.0), 0.94, 0.2),
};

pub fn toast(world: &mut World, notification_sent: NotificationSent) {
    let (notification, stays) = (notification_sent.notification, notification_sent.stays);
    let mut errors = world.resource_mut::<Errors>();
    let clock = errors.clock;
    if let Some(shown) = &mut errors.shown
        && shown.notification == notification
    {
        shown.until = clock.ends_after(stays);
        errors.swell.bump();
        return;
    }
    errors.keys += 1;
    let key = errors.keys;
    errors.shown = Some(ErrorToast {
        key,
        until: clock.ends_after(stays),
        notification,
    });
}

pub fn shown(world: &World) -> Option<String> {
    world
        .resource::<Errors>()
        .shown
        .as_ref()
        .map(|shown| shown.notification.text.plain_words())
}

pub(super) fn plugin(app: &mut App) {
    rows::place::<Errors>(app);
}

pub(super) fn draw(world: &mut World, gui: &egui::Context, top: f32) -> f32 {
    let now = rows::time(gui);
    let mut errors = world.resource_mut::<Errors>();
    errors.swell.start_if_bumped(now);
    let bump = errors.swell.scale(now);
    let current = errors
        .shown
        .as_ref()
        .map(|shown| (shown.key, (shown.key, shown.notification.clone())));
    let toasts = errors.fades.sync(now, TRANSIT, current);
    let width = gui.content_rect().width();
    let mut drawn: Vec<Rect> = Vec::new();
    for ((key, notification), look) in toasts {
        let shown = rows::column_area(
            gui,
            &format!("error-{key}"),
            top,
            rows::Stacking::OverWindows,
        )
        .show(gui, |ui| {
            ui.set_width(width);
            ui.vertical_centered(|ui| {
                ui.multiply_opacity(look.opacity);
                dusk::frame()
                    .show(ui, |ui| {
                        ui.set_max_width(WIDEST);
                        text::show(
                            ui,
                            world,
                            &notification.text.styled(),
                            rows::look(SIZE, dusk::DANGER),
                            None,
                        );
                    })
                    .response
                    .rect
            })
            .inner
        });
        let swell = TSTransform::new(shown.inner.center().to_vec2() * (1.0 - bump), bump);
        gui.set_transform_layer(shown.response.layer_id, look.around(shown.inner) * swell);
        drawn.push(shown.inner);
    }
    let bottom = drawn.iter().map(Rect::bottom).fold(top, f32::max);
    world.resource_mut::<Errors>().drawn = drawn;
    bottom
}

#[derive(Resource, Default)]
struct Errors {
    shown: Option<ErrorToast>,
    fades: Transits<(u64, Notification)>,
    drawn: Vec<Rect>,
    swell: Swell,
    clock: Clock,
    keys: u64,
}

struct ErrorToast {
    key: u64,
    notification: Notification,
    until: Duration,
}

impl NoticePlace for Errors {
    fn drawn(&self) -> &[Rect] {
        &self.drawn
    }

    fn advance(&mut self, delta: Duration, _: Duration) {
        self.clock.advance(delta);
        let clock = self.clock;
        if self
            .shown
            .as_ref()
            .is_some_and(|shown| clock.lapsed(shown.until))
        {
            self.shown = None;
        }
    }
}
