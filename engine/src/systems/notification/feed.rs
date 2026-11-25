use std::collections::HashMap;
use std::time::Duration;
use ui::egui;

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::{Color32, Id, Pos2, Rect, Vec2};
use ui::ink::{self, Ground};

use super::rows::{self, Clock, NoticePlace};
use super::{FeedLayout, FeedRow, Notification, NotificationKind, NotificationSent};
use crate::core::content::{Content, ImageRef};
use crate::core::gui::{self, Band};
use crate::core::text::{self, StyledLine, TextLook};
use crate::core::units::Seconds;
use crate::systems::GameSettings;
use crate::systems::actor::plate;
use crate::systems::dialogue::stage;
use crate::systems::history::RecordTally;
use crate::systems::history::widget::tallied;
use crate::systems::hud;
use crate::systems::messages;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use ui::motion::{Swell, Transit, TransitMove, TransitPose, Transits};

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
    let shown = world
        .resource::<Content>()
        .built::<GameSettings>()
        .notices
        .feed_rows as usize;
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
    if feed.toasts.len() > shown {
        feed.toasts.remove(0);
    }
}

/// Where a line of the feed stacks: over the player's head, or in the bottom right corner of the
/// window.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FeedPlace {
    #[default]
    Overhead,
    Corner,
}

/// A line of the feed as shown: its words, and where they stack.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct FeedShown {
    pub text: String,
    pub place: FeedPlace,
}

pub fn shown(world: &World) -> Vec<FeedShown> {
    let feed = world.resource::<Feed>();
    let content = world.resource::<Content>();
    feed.toasts
        .iter()
        .map(|toast| FeedShown {
            text: label(&toast.notification, content),
            place: feed
                .drawn
                .get(&toast.key)
                .map_or_else(|| feed.place_of(&toast.notification), |(place, _)| *place),
        })
        .collect()
}

#[derive(Resource, Default)]
struct Feed {
    toasts: Vec<FeedToast>,
    fades: Transits<FeedToast>,
    /// Where each line is drawn now, and how high up its stack, gliding toward where it belongs.
    drawn: HashMap<u64, (FeedPlace, f32)>,
    drawn_at: Option<f64>,
    /// Where the lines the feed's own rule places stack now.
    auto: FeedPlace,
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

impl Feed {
    fn place_of(&self, notification: &Notification) -> FeedPlace {
        match layout_of(notification) {
            FeedLayout::Auto => self.auto,
            FeedLayout::Overhead => FeedPlace::Overhead,
            FeedLayout::Corner => FeedPlace::Corner,
        }
    }
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
    let content = world.resource::<Content>().clone();
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
            let line = feed_line(&toast.notification, &content);
            let failure = matches!(
                toast.notification.kind,
                NotificationKind::Feed(FeedRow { failure: true, .. })
            );
            let text_look = text_look(failure);
            let icon = icon_of(&toast.notification).and_then(|icon| gui::image(world, &icon));
            let lead = if icon.is_some() { ICON + GAP } else { 0.0 };
            let text_size = text::measure(&gui, world, &line, text_look, WIDEST - lead);
            FeedLine {
                layout: layout_of(&toast.notification),
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
    let was = world.resource::<Feed>().auto;
    let placed_by_rule: Vec<&FeedLine> = lines
        .iter()
        .filter(|line| line.layout == FeedLayout::Auto)
        .collect();
    let over_head = head.map(|head| Pos2::new(head.x, head.y - ABOVE_HEAD));
    let auto = match over_head {
        _ if placed_by_rule.is_empty() => was,
        Some(at) if room_over_head(world, &gui, at, &placed_by_rule, was) => FeedPlace::Overhead,
        _ => FeedPlace::Corner,
    };
    let place_of = |line: &FeedLine| match (line.layout, over_head) {
        (FeedLayout::Corner, _) | (_, None) => FeedPlace::Corner,
        (FeedLayout::Overhead, Some(_)) => FeedPlace::Overhead,
        (FeedLayout::Auto, Some(_)) => auto,
    };
    let in_corner: Vec<&FeedLine> = lines
        .iter()
        .filter(|line| place_of(line) == FeedPlace::Corner)
        .collect();
    let corner = corner(world, &gui, &in_corner);
    let mut stacked: HashMap<FeedPlace, f32> = HashMap::new();
    let mut drawn = HashMap::new();
    for line in &lines {
        let place = place_of(line);
        let anchor = match (place, over_head) {
            (FeedPlace::Overhead, Some(at)) => FeedAnchor::Overhead(at),
            _ => FeedAnchor::Corner(corner),
        };
        let FeedLine {
            toast,
            look,
            line,
            text_look,
            icon,
            text_size,
            size,
            ..
        } = line;
        let stack = stacked.entry(place).or_default();
        let drawn_height = world
            .resource::<Feed>()
            .drawn
            .get(&toast.key)
            .filter(|(was, _)| *was == place)
            .map_or(*stack, |(_, was)| was + (*stack - was) * glide);
        drawn.insert(toast.key, (place, drawn_height));
        *stack += size.y + LINE_GAP;
        let rect = Rect::from_min_size(anchor.corner_of(*size, drawn_height).round(), *size);
        let shown = rows::Stacking::UnderWindows
            .area(&gui, Id::new(("feed", toast.key)))
            .interactable(false)
            .fade_in(false)
            .fixed_pos(rect.min)
            .show(&gui, |ui| {
                ui.multiply_opacity(look.opacity);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    if let Some(icon) = icon {
                        ui.add(icon.widget(Vec2::splat(ICON)));
                    }
                    ui.allocate_ui(*text_size, |ui| {
                        text::show(ui, world, line, *text_look, None);
                    });
                });
            });
        let bump = toast.swell.scale(now);
        let swell = egui::emath::TSTransform::new(rect.center().to_vec2() * (1.0 - bump), bump);
        gui.set_transform_layer(shown.response.layer_id, look.around(rect) * swell);
    }
    let mut feed = world.resource_mut::<Feed>();
    feed.drawn = drawn;
    feed.auto = auto;
}

/// A line of the feed measured, ready to be drawn.
struct FeedLine {
    layout: FeedLayout,
    toast: FeedToast,
    look: TransitPose,
    line: StyledLine,
    text_look: TextLook,
    icon: Option<gui::GuiImage>,
    text_size: Vec2,
    size: Vec2,
}

/// What a stack of the feed's lines stacks up from: a point over the player's head they are
/// centred on, or the corner of the window's bottom right they are set against. What stands
/// taller they are drawn over.
#[derive(Clone, Copy)]
enum FeedAnchor {
    Overhead(Pos2),
    Corner(Pos2),
}

impl FeedAnchor {
    /// The top left corner of a line of `size` stacked `height` up from the anchor.
    fn corner_of(self, size: Vec2, height: f32) -> Pos2 {
        match self {
            FeedAnchor::Overhead(at) => Pos2::new(at.x - size.x / 2.0, at.y - height - size.y),
            FeedAnchor::Corner(at) => Pos2::new(at.x - size.x, at.y - height - size.y),
        }
    }
}

/// Whether the lines the feed's own rule places fit over the head at `over_head`: as many lines
/// as the feed ever shows, each as tall as the lowest of `lines` and as wide as a line can be,
/// between it and the top of the window clear of every other notice drawn. Lines shown in the
/// corner come back over the head only once a line more than that fits, so they never waver
/// between the two.
fn room_over_head(
    world: &World,
    gui: &egui::Context,
    over_head: Pos2,
    lines: &[&FeedLine],
    was: FeedPlace,
) -> bool {
    let screen = gui.content_rect();
    let row = row_height(lines);
    let spare = match was {
        FeedPlace::Corner => row,
        FeedPlace::Overhead => 0.0,
    };
    let needed = shown_rows(world) * row + spare;
    let stack = Rect::from_x_y_ranges(
        over_head.x - WIDEST / 2.0..=over_head.x + WIDEST / 2.0,
        over_head.y - needed..=over_head.y,
    );
    stack.top() >= screen.top() + EDGE
        && !super::drawn_besides_feed(world)
            .iter()
            .any(|rect| rect.intersects(stack))
}

/// The bottom right corner the lines shown there stack from, clear of the interface there as far
/// as that leaves them in the window's lower half.
fn corner(world: &World, gui: &egui::Context, lines: &[&FeedLine]) -> Pos2 {
    let screen = gui.content_rect();
    let widest = lines.iter().map(|line| line.size.x).fold(0.0, f32::max);
    let right = screen.right() - EDGE;
    let mut bottom = screen.bottom() - EDGE;
    let mut taken: Vec<Rect> = hud::drawn_rects(world)
        .into_iter()
        .chain(stage::dialogue_rect(world))
        .filter(|rect| rect.right() > right - widest && rect.left() < right)
        .collect();
    taken.sort_by(|a, b| b.bottom().total_cmp(&a.bottom()));
    let reach = shown_rows(world) * row_height(lines);
    for rect in taken {
        if rect.bottom() > bottom - reach && rect.top() < bottom {
            let over = rect.top() - EDGE;
            if over - reach < screen.center().y {
                break;
            }
            bottom = over;
        }
    }
    Pos2::new(right, bottom)
}

fn row_height(lines: &[&FeedLine]) -> f32 {
    lines
        .iter()
        .map(|line| line.size.y)
        .reduce(f32::min)
        .map_or(ICON, |shortest| shortest.max(ICON))
        + LINE_GAP
}

fn shown_rows(world: &World) -> f32 {
    world
        .resource::<Content>()
        .built::<GameSettings>()
        .notices
        .feed_rows as f32
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

fn label(notification: &Notification, content: &Content) -> String {
    let text = notification.text.plain_words(content);
    tally_of(notification).map_or_else(
        || text.clone(),
        |tally| tally.line(&text, &messages::of(content).history),
    )
}

fn tally_of(notification: &Notification) -> Option<RecordTally> {
    match notification.kind {
        NotificationKind::Feed(ref row) => row.tally,
        _ => None,
    }
}

fn layout_of(notification: &Notification) -> FeedLayout {
    match &notification.kind {
        NotificationKind::Feed(row) => row.layout,
        _ => FeedLayout::Auto,
    }
}

fn icon_of(notification: &Notification) -> Option<ImageRef> {
    match &notification.kind {
        NotificationKind::Feed(row) => row.icon.clone(),
        _ => None,
    }
}

fn feed_line(notification: &Notification, content: &Content) -> StyledLine {
    let mut line = notification.text.styled(content);
    tallied(
        &mut line,
        tally_of(notification),
        icon_of(notification).is_some(),
        Ground::Dusk,
        &messages::of(content).history,
    );
    line
}
