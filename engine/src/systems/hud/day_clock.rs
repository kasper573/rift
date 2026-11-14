//! The clock round the minimap that tells roughly what time of the game's day it is, and the
//! mark a time of day is known by wherever the interface names one.

use std::f32::consts::TAU;

use bevy::prelude::*;
use ui::egui::emath::TSTransform;
use ui::egui::{self, Align2, Color32, Pos2, Sense, Stroke, Ui, Vec2};
use ui::face::{Face, TextRole};
use ui::paint::TextShadow;
use ui::{chip, dusk, ink};

use crate::core::gui;
use crate::core::units::HourOfDay;
use crate::systems::GameSettings;
use crate::systems::area::TimeOfDayName;
use crate::systems::script::condition::{Wording, WordingPart};
use crate::systems::script::sky;
use crate::systems::settings::UserSettings;

/// How far past the disc it goes round the clock reaches.
pub(crate) const REACH: f32 = ORBIT + NUMERAL_ROOM;

/// Whether the clock is shown: in a game with a day, to a player who has not turned it off.
pub(crate) fn shown(world: &World) -> bool {
    world.resource::<GameSettings>().daylight.is_some()
        && world.resource::<UserSettings>().day_clock_shown()
}

/// The face of a twelve hour clock floating round a disc of radius `disc` at `center`: numerals
/// at twelve, three, six and nine, a tick at every hour between, and no hands. Where the hour
/// hand would point a badge goes round, twice a day, holding the mark of the time of day that
/// holds, each time's mark giving way to the next as that one comes.
pub(crate) fn face(ui: &mut Ui, world: &mut World, center: Pos2, disc: f32) {
    let orbit = disc + ORBIT;
    let painter = ui.painter();
    for hour in 1..=12_u8 {
        let out = hand(HourOfDay(f32::from(hour)));
        let at = center + out * orbit;
        if hour % 3 == 0 {
            let numeral = Face::BookBold.id(NUMERAL);
            let galley = painter.layout_no_wrap(hour.to_string(), numeral, ink::CREAM);
            let rect = Align2::CENTER_CENTER.anchor_size(at, galley.size());
            dusk::lettering(painter, rect.min, galley, OUTLINE, TSTransform::IDENTITY);
        } else {
            let reach = out * (TICK / 2.0);
            painter.line_segment(
                [at - reach - out, at + reach + out],
                Stroke::new(TICK_WIDTH + 2.0, ink::FRAME),
            );
            painter.line_segment(
                [at - reach, at + reach],
                Stroke::new(TICK_WIDTH, ink::CREAM),
            );
        }
    }
    let Some(hour) = sky::now(world).hour else {
        return;
    };
    let Some(cycle) = world.resource::<GameSettings>().daylight.clone() else {
        return;
    };
    let at = center + hand(hour) * orbit;
    ui.painter()
        .circle(at, BADGE, ink::FRAME, Stroke::new(1.5, ink::GOLD));
    let mark = egui::Rect::from_center_size(at, Vec2::splat(MARK));
    for time in &cycle.times {
        let share = cycle.share(&time.name, hour);
        if share <= 0.0 {
            continue;
        }
        if let Some(icon) = gui::image(world, &time.icon) {
            egui::Image::new((icon.texture, mark.size()))
                .tint(Color32::WHITE.gamma_multiply(share))
                .paint_at(ui, mark);
        }
    }
    let said = match cycle.time_at(hour) {
        Some(time) => format!("{} o'clock. {}.", hour.0.floor(), time.name.0),
        None => format!("{} o'clock.", hour.0.floor()),
    };
    let badge = egui::Rect::from_center_size(at, Vec2::splat(2.0 * BADGE));
    ui.interact(badge, ui.id().with("clock"), Sense::hover())
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &said));
}

/// The way from a clock's middle to where its hour hand points at `hour`: twelve at the top, the
/// hours running clockwise, twice round in a day.
pub fn hand(hour: HourOfDay) -> Vec2 {
    let turned = hour.0 / HALF_DAY * TAU;
    Vec2::new(turned.sin(), -turned.cos())
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

/// How far out from the disc's edge the marks and the badge float: clear of what sits on the
/// disc's rim.
const ORBIT: f32 = 20.0;
const NUMERAL: f32 = 13.0;
const NUMERAL_ROOM: f32 = 10.0;
const TICK: f32 = 6.0;
const TICK_WIDTH: f32 = 2.0;
const BADGE: f32 = 9.0;
const MARK: f32 = 14.0;
const HALF_DAY: f32 = 12.0;

/// The dark edge on every side of a numeral.
const OUTLINE: &[TextShadow] = &[
    edge(1.0, 0.0),
    edge(-1.0, 0.0),
    edge(0.0, 1.0),
    edge(0.0, -1.0),
    edge(1.0, 1.0),
    edge(-1.0, 1.0),
    edge(1.0, -1.0),
    edge(-1.0, -1.0),
];

const fn edge(x: f32, y: f32) -> TextShadow {
    TextShadow {
        offset: Vec2::new(x, y),
        color: ink::FRAME,
    }
}
