use std::collections::HashMap;
use std::time::Duration;
use ui::egui;

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::{Color32, Id, Pos2, Rect, Vec2};
use ui::ink::{self, Ground};

use super::rows::{self, Clock, NoticePlace};
use super::{FeedRow, Notification, NotificationKind, NotificationSent};
use crate::core::gui::{self, Band};
use crate::core::text::{self, StyledLine, TextLook};
use crate::core::units::Seconds;
use crate::systems::actor::plate;
use crate::systems::dialogue::stage;
use crate::systems::history::RecordTally;
use crate::systems::history::widget::tallied;
use crate::systems::hud;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use ui::motion::{Swell, Transit, TransitMove, TransitPose, Transits};

const SHOWN: usize = 5;
const WIDEST: f32 = 360.0;
const ICON: f32 = 22.0;
const SIZE: f32 = 20.0;
const GAP: f32 = 5.0;
const LINE_GAP: f32 = 2.0;
const ABOVE_HEAD: f32 = 10.0;
const EDGE: f32 = 8.0;
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

/// Where the feed's lines stack: over the player's head while there is room there for as many as
/// it ever shows at once, else in the bottom right corner of the window.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeedPlace {
    #[default]
    OverHead,
    Corner,
}

pub fn place(world: &World) -> FeedPlace {
    world.resource::<Feed>().place
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
    place: FeedPlace,
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
    let head = session::me(world).and_then(|me| plate::crown(world, me));
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
    let lines: Vec<FeedLine> = toasts
        .into_iter()
        .rev()
        .map(|(toast, look)| {
            let line = feed_line(&toast.notification);
            let failure = matches!(
                toast.notification.kind,
                NotificationKind::Feed(FeedRow { failure: true, .. })
            );
            let text_look = text_look(failure);
            let icon = icon_of(&toast.notification).and_then(|icon| gui::image_file(world, &icon));
            let lead = if icon.is_some() { ICON + GAP } else { 0.0 };
            let text_size = text::measure(&gui, world, &line, text_look, WIDEST - lead);
            FeedLine {
                toast,
                look,
                line,
                text_look,
                icon,
                text_size,
                size: Vec2::new(lead + text_size.x, text_size.y.max(ICON)),
            }
        })
        .collect();
    let was = world.resource::<Feed>().place;
    let anchor = FeedAnchor::of(world, &gui, head, &lines, was);
    let place = anchor.place();
    let mut stacked = 0.0;
    let mut heights = HashMap::new();
    for FeedLine {
        toast,
        look,
        line,
        text_look,
        icon,
        text_size,
        size,
    } in lines
    {
        let drawn_height = world
            .resource::<Feed>()
            .heights
            .get(&toast.key)
            .copied()
            .filter(|_| place == was)
            .map_or(stacked, |was| was + (stacked - was) * glide);
        heights.insert(toast.key, drawn_height);
        stacked += size.y + LINE_GAP;
        let rect = Rect::from_min_size(anchor.corner_of(size, drawn_height).round(), size);
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
                        text::show(ui, world, &line, text_look, None);
                    });
                });
            });
        let bump = toast.swell.scale(now);
        let swell = egui::emath::TSTransform::new(rect.center().to_vec2() * (1.0 - bump), bump);
        gui.set_transform_layer(shown.response.layer_id, look.around(rect) * swell);
    }
    let mut feed = world.resource_mut::<Feed>();
    feed.heights = heights;
    feed.place = place;
}

/// A line of the feed measured, ready to be drawn.
struct FeedLine {
    toast: FeedToast,
    look: TransitPose,
    line: StyledLine,
    text_look: TextLook,
    icon: Option<gui::GuiImage>,
    text_size: Vec2,
    size: Vec2,
}

/// What the feed's lines stack up from: a point over the player's head they are centred on, or
/// the corner of the window's bottom right they are set against, clear of the interface there.
#[derive(Clone, Copy)]
enum FeedAnchor {
    OverHead(Pos2),
    Corner(Pos2),
}

impl FeedAnchor {
    /// Over `head` while as many lines as the feed ever shows fit between it and the top of the
    /// window, each as tall as the lowest of `lines`; else the corner. A feed showing lines in
    /// the corner comes back over the head only once a line more than that fits, so it never
    /// wavers between the two.
    fn of(
        world: &World,
        gui: &egui::Context,
        head: Option<bevy::math::Vec2>,
        lines: &[FeedLine],
        was: FeedPlace,
    ) -> FeedAnchor {
        let screen = gui.content_rect();
        let row = lines
            .iter()
            .map(|line| line.size.y)
            .reduce(f32::min)
            .map_or(ICON, |shortest| shortest.max(ICON))
            + LINE_GAP;
        let spare = match was {
            FeedPlace::Corner if !lines.is_empty() => row,
            FeedPlace::OverHead | FeedPlace::Corner => 0.0,
        };
        let needed = ABOVE_HEAD + SHOWN as f32 * row + spare;
        if let Some(head) = head.filter(|head| head.y - needed >= screen.top() + EDGE) {
            return FeedAnchor::OverHead(Pos2::new(head.x, head.y - ABOVE_HEAD));
        }
        let widest = lines.iter().map(|line| line.size.x).fold(0.0, f32::max);
        let right = screen.right() - EDGE;
        let mut bottom = screen.bottom() - EDGE;
        let mut taken: Vec<Rect> = hud::drawn_rects(world)
            .into_iter()
            .chain(stage::rects(world))
            .filter(|rect| rect.right() > right - widest && rect.left() < right)
            .collect();
        taken.sort_by(|a, b| b.bottom().total_cmp(&a.bottom()));
        for rect in taken {
            if rect.bottom() > bottom - SHOWN as f32 * row && rect.top() < bottom {
                bottom = rect.top() - EDGE;
            }
        }
        FeedAnchor::Corner(Pos2::new(right, bottom))
    }

    fn place(self) -> FeedPlace {
        match self {
            FeedAnchor::OverHead(_) => FeedPlace::OverHead,
            FeedAnchor::Corner(_) => FeedPlace::Corner,
        }
    }

    /// The top left corner of a line of `size` stacked `height` up from the anchor.
    fn corner_of(self, size: Vec2, height: f32) -> Pos2 {
        match self {
            FeedAnchor::OverHead(at) => Pos2::new(at.x - size.x / 2.0, at.y - height - size.y),
            FeedAnchor::Corner(at) => Pos2::new(at.x - size.x, at.y - height - size.y),
        }
    }
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
