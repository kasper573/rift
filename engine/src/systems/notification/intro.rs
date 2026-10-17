use std::time::Duration;

use bevy::prelude::*;
use design_tokens::family;
use egui::emath::TSTransform;
use egui::{Align, Color32, FontId, Layout, Rect, Sense, Stroke, UiBuilder, Vec2};
use serde::{Deserialize, Serialize};

use super::rows::{self, Clock, Fades, Move, Presence};
use super::{Notification, NotificationKind, NotificationSent, window_visible};
use crate::core::assets::AssetRef;
use crate::core::audio::playback::{PlaySfx, SfxPlace};
use crate::core::gui;
use crate::core::math::Rgba;
use crate::core::text::{self, TextShadow};
use crate::systems::screen::ClientScreen;
use crate::systems::screen::area;
use crate::systems::text::Motion;

const TITLE_SIZE: f32 = 44.0;
const LINE_SIZE: f32 = 19.0;
const EMBLEM: f32 = 48.0;
const RULE: f32 = 64.0;
const RULE_GAP: f32 = 16.0;
const WIDEST: f32 = 560.0;
const PADDING: Vec2 = Vec2::new(40.0, 16.0);
const ROW_GAP: f32 = 8.0;
const SHADOW: TextShadow = TextShadow {
    offset: Vec2::new(0.0, 2.0),
    color: Color32::from_black_alpha(204),
};
/// The breathing room after an area settles before its intro lands: a deliberate pause, so the
/// title and its sound do not arrive on the same frame as the transition ends.
const BREATH_AFTER_TRANSITION: Duration = Duration::from_millis(300);
const PRESENCE: Presence = Presence {
    enter: Move::new(Vec2::new(0.0, 14.0), 0.94, 0.8),
    exit: Move::new(Vec2::new(0.0, -8.0), 1.04, 0.6),
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct IntroAppearance {
    pub tint: Option<Rgba>,
    pub motion: Option<Motion>,
    pub emblem: Option<AssetRef>,
}

pub struct IntroPlugin;

impl Plugin for IntroPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShownIntro>()
            .add_systems(
                Update,
                reveal_when_settled
                    .after(area::carry_view_across)
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(
                Update,
                age.run_if(in_state(ClientScreen::Area))
                    .run_if(window_visible),
            )
            .add_systems(OnExit(ClientScreen::Area), rows::forget::<ShownIntro>);
    }
}

/// An intro arrives with the area it introduces, while the transition into it still plays, so it
/// waits until the view is revealed.
pub fn show(world: &mut World, notification_sent: NotificationSent) {
    world.resource_mut::<ShownIntro>().waiting = Some(notification_sent.notification);
}

pub fn shown(world: &World) -> Option<(String, String)> {
    let showing = world.resource::<ShownIntro>().showing.as_ref()?;
    let notification = &showing.notification;
    let NotificationKind::Intro { title, .. } = &notification.kind else {
        return None;
    };
    Some((title.to_string(), notification.text.plain_words()))
}

pub(super) fn draw(world: &mut World, gui: &egui::Context, top: f32) -> f32 {
    let (now, still) = rows::motion_time(world, gui);
    let mut shown = world.resource_mut::<ShownIntro>();
    let current = shown
        .showing
        .as_ref()
        .map(|showing| (showing.key, (showing.key, showing.notification.clone())));
    let intros = shown.fades.sync(now, PRESENCE, still, current);
    let width = gui.content_rect().width();
    let mut drawn: Vec<Rect> = Vec::new();
    for ((key, notification), look) in intros {
        let NotificationKind::Intro {
            title, appearance, ..
        } = &notification.kind
        else {
            continue;
        };
        let line = notification.text.styled();
        let tint = appearance.tint.map_or(Color32::WHITE, color_of);
        let title_font = FontId::new(TITLE_SIZE, family::bold());
        let title_size = gui
            .fonts_mut(|fonts| fonts.layout_no_wrap(title.to_string(), title_font.clone(), tint))
            .size();
        let room = WIDEST - 2.0 * PADDING.x;
        let line_size = text::measure(
            gui,
            world,
            &line,
            rows::look(LINE_SIZE, Color32::WHITE),
            room,
        );
        let title_row = Vec2::new(title_size.x + 2.0 * (RULE + RULE_GAP), title_size.y);
        let inner = title_row.x.max(line_size.x).max(EMBLEM).min(room);
        let emblem = appearance
            .emblem
            .as_ref()
            .map(|emblem| gui::image(world, emblem));
        let shift = title_motion(appearance.motion, now, still);
        let shown = rows::column_area(gui, &format!("intro-{key}"), top).show(gui, |ui| {
            ui.set_width(width);
            ui.vertical_centered(|ui| {
                ui.multiply_opacity(look.opacity);
                egui::Frame::new()
                    .fill(Color32::from_black_alpha(89))
                    .corner_radius(16.0)
                    .inner_margin(egui::Margin::symmetric(PADDING.x as i8, PADDING.y as i8))
                    .show(ui, |ui| {
                        ui.set_width(inner);
                        ui.spacing_mut().item_spacing.y = ROW_GAP;
                        match emblem {
                            Some(Some(image)) => {
                                ui.add(image.widget(Vec2::splat(EMBLEM)));
                            }
                            Some(None) => {
                                ui.allocate_exact_size(Vec2::splat(EMBLEM), Sense::hover());
                            }
                            None => {}
                        }
                        let (row, _) = ui.allocate_exact_size(title_row, Sense::hover());
                        let rule_y = row.center().y;
                        let rule = Stroke::new(1.0, tint.gamma_multiply(0.6));
                        ui.painter()
                            .hline(row.left()..=row.left() + RULE, rule_y, rule);
                        ui.painter()
                            .hline(row.right() - RULE..=row.right(), rule_y, rule);
                        let mut title_ui = ui.new_child(
                            UiBuilder::new()
                                .max_rect(row.shrink2(Vec2::new(RULE + RULE_GAP, 0.0)))
                                .layout(Layout::left_to_right(Align::Center)),
                        );
                        text::show_shadowed(
                            &mut title_ui,
                            title,
                            title_font.clone(),
                            tint,
                            SHADOW,
                            shift,
                        );
                        let (lines, _) = ui.allocate_exact_size(line_size, Sense::hover());
                        let mut line_ui = ui.new_child(
                            UiBuilder::new()
                                .max_rect(lines)
                                .layout(Layout::top_down(Align::Min)),
                        );
                        text::show(
                            &mut line_ui,
                            world,
                            &line,
                            rows::look(LINE_SIZE, Color32::WHITE),
                            None,
                        );
                    })
                    .response
                    .rect
            })
            .inner
        });
        gui.set_transform_layer(shown.response.layer_id, look.around(shown.inner));
        drawn.push(shown.inner);
    }
    let bottom = drawn.iter().map(Rect::bottom).fold(top, f32::max);
    world.resource_mut::<ShownIntro>().drawn = drawn;
    bottom
}

#[derive(Resource, Default)]
struct ShownIntro {
    waiting: Option<Notification>,
    settled_for: Duration,
    showing: Option<ShowingIntro>,
    clock: Clock,
    fades: Fades<(u64, Notification)>,
    drawn: Vec<Rect>,
    keys: u64,
}

struct ShowingIntro {
    key: u64,
    notification: Notification,
    until: Duration,
}

fn reveal_when_settled(world: &mut World) {
    let delta = world.resource::<Time>().delta();
    let settled = area::settled(world);
    let mut shown = world.resource_mut::<ShownIntro>();
    shown.settled_for = if settled {
        shown.settled_for + delta
    } else {
        Duration::ZERO
    };
    if shown.settled_for < BREATH_AFTER_TRANSITION || shown.waiting.is_none() {
        return;
    }
    let Some(notification) = shown.waiting.take() else {
        return;
    };
    let until = shown.clock.ends_after(notification.stays());
    shown.keys += 1;
    let key = shown.keys;
    let sfx = notification.kind.sfx();
    shown.showing = Some(ShowingIntro {
        key,
        notification,
        until,
    });
    if let Some(id) = sfx {
        world.write_message(PlaySfx {
            id,
            place: SfxPlace::Interface,
        });
    }
}

fn age(world: &mut World) {
    let held = rows::hovered(world, &world.resource::<ShownIntro>().drawn);
    let delta = world.resource::<Time>().delta();
    let mut shown = world.resource_mut::<ShownIntro>();
    if !held {
        shown.clock.advance(delta);
    }
    let clock = shown.clock;
    if shown
        .showing
        .as_ref()
        .is_some_and(|showing| clock.lapsed(showing.until))
    {
        shown.showing = None;
    }
}

fn title_motion(motion: Option<Motion>, now: f64, still: bool) -> TSTransform {
    let t = now as f32;
    match motion.filter(|_| !still) {
        None => TSTransform::IDENTITY,
        Some(Motion::Pulse) => TSTransform::from_scaling(1.0 + (t * 3.0).sin() * 0.04),
        Some(Motion::Wave) => TSTransform::from_translation(Vec2::new(0.0, (t * 4.0).sin() * 3.0)),
        Some(Motion::Shake) => {
            TSTransform::from_translation(Vec2::new((t * 53.0).sin() * 1.5, (t * 47.0).cos() * 1.5))
        }
    }
}

fn color_of(rgba: Rgba) -> Color32 {
    let [r, g, b, a] = rgba.0.to_be_bytes();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}
