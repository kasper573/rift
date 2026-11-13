//! The dial that tells the game's hour and the time of day that holds, and the mark a time of
//! day is known by wherever the interface names one.

use std::f32::consts::TAU;

use bevy::prelude::*;
use ui::egui::emath::TSTransform;
use ui::egui::epaint::Mesh;
use ui::egui::{self, Align2, Color32, Pos2, Sense, Shape, Stroke, Ui, Vec2};
use ui::face::TextRole;
use ui::timespans::{self, Timespan};
use ui::{chip, dusk, ink, slip};

use crate::core::gui;
use crate::core::units::HourOfDay;
use crate::systems::GameSettings;
use crate::systems::area::{DayCycle, TimeOfDayName};
use crate::systems::area_map::minimap;
use crate::systems::hud::{self, HudWidget};
use crate::systems::script::condition::{Wording, WordingPart};
use crate::systems::script::sky;
use crate::systems::settings::UserSettings;

pub const DAY_DIAL: &str = "day_dial";

pub struct DayDialWidget;

impl HudWidget for DayDialWidget {
    /// Under the minimap's fallback spot.
    fn fallback(&self, screen: egui::Rect) -> Pos2 {
        let above = minimap::MinimapWidget.fallback(screen);
        Pos2::new(
            above.x + (minimap::RESTING_SIZE.x - RESTING_SIZE.x) / 2.0,
            above.y + minimap::RESTING_SIZE.y + hud::MARGIN,
        )
    }

    /// In a game without a day, before the server has told its hour, and for a player who
    /// turned it off.
    fn hidden(&self, world: &World) -> bool {
        world.resource::<GameSettings>().daylight.is_none()
            || sky::now(world).hour.is_none()
            || !world.resource::<UserSettings>().day_dial_shown()
    }

    fn show(&self, ui: &mut Ui, world: &mut World) -> Option<egui::Rect> {
        let (piece, _) = ui.allocate_exact_size(RESTING_SIZE, Sense::hover());
        let center = piece.center_top() + Vec2::new(0.0, RING / 2.0);
        let whole = egui::Rect::from_center_size(center, Vec2::splat(RING));
        let cycle = world.resource::<GameSettings>().daylight.clone()?;
        let hour = sky::now(world).hour?;
        let spans = cycle.timespans(|icon| gui::image(world, icon).map(|icon| icon.texture));
        let holds = cycle.time_at(hour).map(|time| time.name.clone());
        let face = ui
            .interact(whole, ui.id().with("face"), Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::Grab);
        hud::grab_spot(world, DAY_DIAL, &face, piece);
        let said = match &holds {
            Some(time) => format!("{} o'clock. {}.", hour.0.floor(), time.0),
            None => format!("{} o'clock.", hour.0.floor()),
        };
        face.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &said));

        let painter = ui.painter();
        painter.add(slip::SHADOW.as_shape(whole, egui::CornerRadius::same((RING / 2.0) as u8)));
        painter.circle_filled(center, RING / 2.0 - BAND, ink::LIGHT);
        ring(painter, center, &spans);
        painter.circle_stroke(center, RING / 2.0, Stroke::new(1.5, ink::GOLD));
        painter.circle_stroke(center, RING / 2.0 - BAND, Stroke::new(1.0, ink::FRAME));
        for (at, span) in spans.iter().enumerate() {
            let Some(icon) = span.icon else { continue };
            let lasts = (spans[(at + 1) % spans.len()].from - span.from).rem_euclid(DAY);
            let lasts = if spans.len() == 1 { DAY } else { lasts };
            let middle = toward(HourOfDay(span.from + lasts / 2.0)) * (RING / 2.0 - BAND / 2.0);
            let at = egui::Rect::from_center_size(center + middle, Vec2::splat(ICON));
            egui::Image::new((icon, at.size())).paint_at(ui, at);
        }
        let painter = ui.painter();
        let tip = center + toward(hour) * (RING / 2.0 - BAND - 2.0);
        painter.line_segment([center, tip], Stroke::new(2.0, ink::RUBRIC));
        painter.circle_filled(center, HUB, ink::LIGHT);
        painter.circle_stroke(center, HUB, Stroke::new(1.0, ink::FRAME));
        painter.text(
            center,
            Align2::CENTER_CENTER,
            hour.0.floor().to_string(),
            TextRole::Name.font_id(),
            ink::INK,
        );
        if let Some(time) = &holds {
            lettered(ui, &time.0, center + Vec2::new(0.0, RING / 2.0));
        }
        if !spans.is_empty() {
            face.on_hover_ui(|ui| {
                for (at, span) in spans.iter().enumerate() {
                    let until = spans[(at + 1) % spans.len()].from;
                    let text = format!("{} · {} to {}", span.name, span.from, until);
                    match span.icon {
                        Some(icon) => chip::emblem(ui, text, span.color, icon),
                        None => chip::swatch(ui, text, span.color),
                    };
                }
            });
        }
        None
    }
}

/// What a condition asks for, its times of day each shown by the game's mark for it.
pub(crate) fn wording(
    ui: &mut Ui,
    world: &mut World,
    wording: &Wording,
    role: TextRole,
    color: Color32,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for part in &wording.0 {
            match part {
                WordingPart::Words { words } => {
                    ui.label(role.rich(words).color(color));
                }
                WordingPart::Time { time } => {
                    time_mark(ui, world, time, role, color);
                }
            }
        }
    });
}

/// The time of day as the game marks it: its colour, its icon and its name.
pub(crate) fn time_mark(
    ui: &mut Ui,
    world: &mut World,
    time: &TimeOfDayName,
    role: TextRole,
    color: Color32,
) -> egui::Response {
    let named = role.rich(&time.0).color(color);
    let marked = world
        .resource::<GameSettings>()
        .daylight
        .as_ref()
        .and_then(|cycle| cycle.times.iter().find(|known| known.name == *time))
        .map(|known| (known.color, known.icon.clone()));
    match marked.and_then(|(tint, icon)| Some((tint, gui::image(world, &icon)?.texture))) {
        Some((tint, icon)) => chip::emblem(ui, named, tint.into(), icon),
        None => ui.label(named),
    }
}

impl DayCycle {
    /// The day's times as spans of its 24 hours, each with the texture `icon` gives its icon.
    pub fn timespans(
        &self,
        mut icon: impl FnMut(&crate::core::content::ImageRef) -> Option<egui::TextureId>,
    ) -> Vec<Timespan> {
        self.times
            .iter()
            .map(|time| Timespan {
                name: time.name.0.clone(),
                from: time.from.0,
                eases: time.eases.0,
                color: time.color.into(),
                icon: icon(&time.icon),
            })
            .collect()
    }
}

pub const RESTING_SIZE: Vec2 = Vec2::new(RING, RING + NAME_ROOM);

const RING: f32 = 84.0;
const BAND: f32 = 16.0;
const ICON: f32 = 13.0;
const HUB: f32 = 13.0;
const NAME_ROOM: f32 = 22.0;
const DAY: f32 = 24.0;
/// How many pieces the ring is painted in: one to a quarter of an hour.
const PIECES: u32 = 96;

/// The way from the dial's middle to `hour` on its ring: noon at the top, the hours running
/// clockwise.
fn toward(hour: HourOfDay) -> Vec2 {
    let turned = (hour.0 - 12.0) / DAY * TAU;
    Vec2::new(turned.sin(), -turned.cos())
}

/// The ring of the 24 hours in the colours of the day's times, each blending into the next as
/// that one eases in; plain for a day that names none.
fn ring(painter: &egui::Painter, center: Pos2, spans: &[Timespan]) {
    if spans.is_empty() {
        painter.circle_stroke(center, (RING - BAND) / 2.0, Stroke::new(BAND, ink::FRAME));
        return;
    }
    let mut mesh = Mesh::default();
    for piece in 0..=PIECES {
        let hour = piece as f32 / PIECES as f32 * DAY;
        let color = timespans::color_at(spans, &(0.0..=DAY), hour.min(DAY - 0.001));
        let out = toward(HourOfDay(hour));
        mesh.colored_vertex(center + out * (RING / 2.0), color);
        mesh.colored_vertex(center + out * (RING / 2.0 - BAND), color);
        if piece > 0 {
            let at = (piece - 1) * 2;
            mesh.add_triangle(at, at + 1, at + 2);
            mesh.add_triangle(at + 1, at + 2, at + 3);
        }
    }
    painter.add(Shape::mesh(mesh));
}

fn lettered(ui: &mut Ui, name: &str, top: Pos2) {
    let galley =
        ui.painter()
            .layout_no_wrap(name.to_owned(), TextRole::Name.font_id(), Color32::WHITE);
    let rect = Align2::CENTER_TOP.anchor_size(top + Vec2::new(0.0, 3.0), galley.size());
    dusk::lettering(
        ui.painter(),
        rect.min,
        galley,
        dusk::TEXT_SHADOW,
        TSTransform::IDENTITY,
    );
}
