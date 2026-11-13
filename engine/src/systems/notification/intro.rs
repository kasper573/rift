use std::time::Duration;
use ui::dusk;
use ui::egui;

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::emath::TSTransform;
use ui::egui::{Align, Color32, Layout, Rect, Sense, Stroke, UiBuilder, Vec2};
use ui::face::Face;

use super::rows::{self, Clock, NoticePlace};
use super::{Notification, NotificationKind, NotificationSent};
use crate::core::audio::playback::{PlaySfx, SfxPlace};
use crate::core::content::ImageRef;
use crate::core::gui;
use crate::core::math::Rgba;
use crate::core::text::{self, TextMotion};
use crate::systems::GameSettings;
use crate::systems::screen::{ClientScreen, area};
use ui::motion::{Transit, TransitMove, Transits};
use ui::paint;

const TITLE_SIZE: f32 = 44.0;
const LINE_SIZE: f32 = 19.0;
const EMBLEM: f32 = 48.0;
const RULE: f32 = 64.0;
const RULE_GAP: f32 = 16.0;
const WIDEST: f32 = 560.0;
const PADDING: Vec2 = Vec2::new(40.0, 16.0);
const ROW_GAP: f32 = 8.0;
const TRANSIT: Transit = Transit {
    enter: TransitMove::new(Vec2::new(0.0, 14.0), 0.94, 0.8),
    exit: TransitMove::new(Vec2::new(0.0, -8.0), 1.04, 0.6),
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct IntroAppearance {
    pub tint: Option<Rgba>,
    pub motion: Option<TextMotion>,
    pub emblem: Option<ImageRef>,
}

pub struct IntroPlugin;

impl Plugin for IntroPlugin {
    fn build(&self, app: &mut App) {
        rows::place::<IntroShowing>(app);
        app.add_systems(
            Update,
            reveal_when_settled
                .after(area::carry_view_across)
                .run_if(in_state(ClientScreen::Area)),
        );
    }
}

/// An intro arrives with the area it introduces, while the transition into it still plays, so it
/// waits until the view is revealed.
pub fn show(world: &mut World, notification_sent: NotificationSent) {
    world.resource_mut::<IntroShowing>().waiting = Some(notification_sent);
}

/// The intro on show: its title and its words.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ShownIntro {
    pub title: String,
    pub text: String,
}

pub fn shown(world: &World) -> Option<ShownIntro> {
    let showing = world.resource::<IntroShowing>().showing.as_ref()?;
    let notification = &showing.notification;
    let NotificationKind::Intro { title, .. } = &notification.kind else {
        return None;
    };
    Some(ShownIntro {
        title: title.to_string(),
        text: notification.text.plain_words(),
    })
}

pub(super) fn draw(world: &mut World, gui: &egui::Context, top: f32) -> f32 {
    let now = rows::time(gui);
    let mut shown = world.resource_mut::<IntroShowing>();
    let current = shown
        .showing
        .as_ref()
        .map(|showing| (showing.key, (showing.key, showing.notification.clone())));
    let intros = shown.fades.sync(now, TRANSIT, current);
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
        let tint = appearance.tint.map_or(Color32::WHITE, Color32::from);
        let title_font = Face::BookBold.id(TITLE_SIZE);
        let title_size = paint::text_size(gui, title, title_font.clone());
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
        let shift = title_motion(appearance.motion, now);
        let shown = rows::column_area(
            gui,
            &format!("intro-{key}"),
            top,
            rows::Stacking::UnderWindows,
        )
        .show(gui, |ui| {
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
                        dusk::label(
                            &mut title_ui,
                            title,
                            title_font.clone(),
                            tint,
                            dusk::TEXT_SHADOW,
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
    world.resource_mut::<IntroShowing>().drawn = drawn;
    bottom
}

#[derive(Resource, Default)]
struct IntroShowing {
    waiting: Option<NotificationSent>,
    settled_for: Duration,
    showing: Option<ShowingIntro>,
    clock: Clock,
    fades: Transits<(u64, Notification)>,
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
    let after = Duration::from(world.resource::<GameSettings>().notices.intro_after);
    let mut shown = world.resource_mut::<IntroShowing>();
    shown.settled_for = if settled {
        shown.settled_for + delta
    } else {
        Duration::ZERO
    };
    if shown.settled_for < after || shown.waiting.is_none() {
        return;
    }
    let Some(NotificationSent {
        notification,
        stays,
        ..
    }) = shown.waiting.take()
    else {
        return;
    };
    let until = shown.clock.ends_after(stays);
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

impl NoticePlace for IntroShowing {
    fn drawn(&self) -> &[Rect] {
        &self.drawn
    }

    fn advance(&mut self, delta: Duration, _: Duration) {
        self.clock.advance(delta);
        let clock = self.clock;
        if self
            .showing
            .as_ref()
            .is_some_and(|showing| clock.lapsed(showing.until))
        {
            self.showing = None;
        }
    }
}

fn title_motion(motion: Option<TextMotion>, now: f64) -> TSTransform {
    let t = now as f32;
    match motion {
        None => TSTransform::IDENTITY,
        Some(TextMotion::Pulse) => TSTransform::from_scaling(1.0 + (t * 3.0).sin() * 0.04),
        Some(TextMotion::Wave) => {
            TSTransform::from_translation(Vec2::new(0.0, (t * 4.0).sin() * 3.0))
        }
        Some(TextMotion::Shake) => {
            TSTransform::from_translation(Vec2::new((t * 53.0).sin() * 1.5, (t * 47.0).cos() * 1.5))
        }
    }
}
