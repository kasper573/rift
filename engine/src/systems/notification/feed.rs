use std::collections::HashMap;
use std::time::Duration;
use ui::egui;

use bevy::prelude::*;
use ui::egui::{Color32, Id, Pos2, Rect, Vec2};
use ui::ink::{self, Ground};

use super::rows::{self, Clock, NoticePlace};
use super::{FeedRow, Notification, NotificationKind, NotificationSent};
use crate::core::gui::{self, Band};
use crate::core::text::{self, StyledLine, TextLook};
use crate::core::units::Seconds;
use crate::systems::actor::plate;
use crate::systems::history::RecordTally;
use crate::systems::history::widget::tallied;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use ui::motion::{Swell, Transit, TransitMove, Transits};

const SHOWN: usize = 5;
const WIDEST: f32 = 360.0;
const ICON: f32 = 22.0;
const SIZE: f32 = 20.0;
const GAP: f32 = 5.0;
const LINE_GAP: f32 = 2.0;
const ABOVE_HEAD: f32 = 10.0;
const HALO: Color32 = Color32::from_rgba_premultiplied(10, 7, 5, 210);
const FAILED: Color32 = Color32::from_rgb(0xF0, 0x8A, 0x7E);
/// How quickly a line glides to the height the lines under it leave it, per second.
const GLIDE: f32 = 14.0;
const TRANSIT: Transit = Transit {
    enter: TransitMove::new(Vec2::new(0.0, 10.0), 0.92, 0.22),
    exit: TransitMove::new(Vec2::new(0.0, -8.0), 1.0, 0.35),
};

pub struct FeedPlugin;

impl Plugin for FeedPlugin {
    fn build(&self, app: &mut App) {
        rows::place::<Feed>(app);
        app.add_systems(
            bevy_egui::EguiPrimaryContextPass,
            draw.in_set(Band::Notices)
                .run_if(in_state(ClientScreen::Area)),
        );
    }
}

pub fn show(world: &mut World, notification_sent: NotificationSent) {
    let (notification, lasts) = (notification_sent.notification, notification_sent.stays);
    let mut feed = world.resource_mut::<Feed>();
    let clock = feed.clock;
    let merge = feed.toasts.iter().enumerate().find_map(|(index, shown)| {
        merged(&shown.notification, &notification).map(|merged| (index, merged))
    });
    if let Some((index, merged)) = merge {
        let mut toast = feed.toasts.remove(index);
        toast.notification = merged;
        toast.until = clock.ends_after(toast.lasts);
        toast.swell.bump();
        feed.toasts.push(toast);
        return;
    }
    feed.keys += 1;
    let key = feed.keys;
    feed.toasts.push(FeedToast {
        key,
        until: clock.ends_after(lasts),
        lasts,
        notification,
        swell: Swell::default(),
    });
    if feed.toasts.len() > SHOWN {
        feed.toasts.remove(0);
    }
}

pub fn rows(world: &World) -> Vec<String> {
    world
        .resource::<Feed>()
        .toasts
        .iter()
        .map(|toast| label(&toast.notification))
        .collect()
}

#[derive(Resource, Default)]
struct Feed {
    toasts: Vec<FeedToast>,
    fades: Transits<FeedToast>,
    /// How high above the head each line is drawn now, gliding toward where it belongs.
    heights: HashMap<u64, f32>,
    drawn_at: Option<f64>,
    clock: Clock,
    keys: u64,
}

#[derive(Clone)]
struct FeedToast {
    key: u64,
    notification: Notification,
    until: Duration,
    lasts: Seconds,
    swell: Swell,
}

impl NoticePlace for Feed {
    const HOLDS: bool = false;

    fn drawn(&self) -> &[Rect] {
        &[]
    }

    fn advance(&mut self, delta: Duration, _: Duration) {
        self.clock.advance(delta);
        let clock = self.clock;
        self.toasts.retain(|toast| !clock.lapsed(toast.until));
    }
}

fn draw(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let Some(head) = session::me(world).and_then(|me| plate::crown(world, me)) else {
        return;
    };
    let now = rows::time(&gui);
    let mut feed = world.resource_mut::<Feed>();
    let glide = feed
        .drawn_at
        .map_or(1.0, |then| 1.0 - (-GLIDE * (now - then) as f32).exp());
    feed.drawn_at = Some(now);
    for toast in &mut feed.toasts {
        toast.swell.start_if_bumped(now);
    }
    let current: Vec<(u64, FeedToast)> = feed
        .toasts
        .iter()
        .map(|toast| (toast.key, toast.clone()))
        .collect();
    let toasts = feed.fades.sync(now, TRANSIT, current);
    let mut stacked = 0.0;
    let mut heights = HashMap::new();
    for (toast, look) in toasts.into_iter().rev() {
        let line = feed_line(&toast.notification);
        let failure = matches!(
            toast.notification.kind,
            NotificationKind::Feed(FeedRow { failure: true, .. })
        );
        let look_of_text = text_look(failure);
        let icon = icon_of(&toast.notification).and_then(|icon| gui::image_file(world, &icon));
        let lead = if icon.is_some() { ICON + GAP } else { 0.0 };
        let text_size = text::measure(&gui, world, &line, look_of_text, WIDEST - lead);
        let size = Vec2::new(lead + text_size.x, text_size.y.max(ICON));
        let feed = world.resource::<Feed>();
        let drawn_height = feed
            .heights
            .get(&toast.key)
            .copied()
            .map_or(stacked, |was| was + (stacked - was) * glide);
        heights.insert(toast.key, drawn_height);
        stacked += size.y + LINE_GAP;
        let rect = Rect::from_min_size(
            Pos2::new(
                (head.x - size.x / 2.0).round(),
                (head.y - ABOVE_HEAD - drawn_height - size.y).round(),
            ),
            size,
        );
        let shown = rows::Stacking::UnderWindows
            .area(&gui, Id::new(("feed", toast.key)))
            .interactable(false)
            .fade_in(false)
            .fixed_pos(rect.min)
            .show(&gui, |ui| {
                ui.multiply_opacity(look.opacity);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    if let Some(icon) = &icon {
                        ui.add(icon.widget(Vec2::splat(ICON)));
                    }
                    ui.allocate_ui(text_size, |ui| {
                        text::show(ui, world, &line, look_of_text, None);
                    });
                });
            });
        let bump = toast.swell.scale(now);
        let swell = egui::emath::TSTransform::new(rect.center().to_vec2() * (1.0 - bump), bump);
        gui.set_transform_layer(shown.response.layer_id, look.around(rect) * swell);
    }
    world.resource_mut::<Feed>().heights = heights;
}

fn text_look(failure: bool) -> TextLook {
    let ink = if failure { FAILED } else { ink::LIGHT };
    TextLook::new(SIZE, ink)
        .on(Ground::Dusk)
        .bold()
        .haloed(HALO)
}

fn merged(shown: &Notification, newer: &Notification) -> Option<Notification> {
    let (NotificationKind::Feed(old), NotificationKind::Feed(new)) = (&shown.kind, &newer.kind)
    else {
        return None;
    };
    let same = shown.text == newer.text && old.icon == new.icon && old.failure == new.failure;
    let tally = old.tally?.merged(new.tally?)?;
    same.then(|| Notification {
        kind: NotificationKind::Feed(FeedRow {
            topic: old.topic,
            tally: Some(tally),
            ..new.clone()
        }),
        ..newer.clone()
    })
}

fn label(notification: &Notification) -> String {
    let text = notification.text.plain_words();
    tally_of(notification).map_or_else(|| text.clone(), |tally| tally.line(&text))
}

fn tally_of(notification: &Notification) -> Option<RecordTally> {
    match notification.kind {
        NotificationKind::Feed(ref row) => row.tally,
        _ => None,
    }
}

fn icon_of(notification: &Notification) -> Option<String> {
    match &notification.kind {
        NotificationKind::Feed(row) => row.icon.clone(),
        _ => None,
    }
}

fn feed_line(notification: &Notification) -> StyledLine {
    let mut line = notification.text.styled();
    tallied(
        &mut line,
        tally_of(notification),
        icon_of(notification).is_some(),
        Ground::Dusk,
    );
    line
}
