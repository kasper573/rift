use std::collections::VecDeque;
use ui::egui;
use ui::meter::{self, Meter};

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use ui::egui::{Align2, Vec2};

use crate::core::gui::{self, Band};
use crate::core::units::{Hertz, Seconds};
use crate::systems::settings::UserSettings;

/// The game's meter of how many frames it draws a second, shown while the player's settings ask.
pub struct FrameRatePlugin;

impl Plugin for FrameRatePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FrameCountPlugin)
            .add_systems(EguiPrimaryContextPass, draw_meter.in_set(Band::System));
    }
}

/// Counts the frames the app draws into [`FrameRates`], for [`show_meter`] to show.
pub struct FrameCountPlugin;

impl Plugin for FrameCountPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FrameRates>()
            .add_systems(Update, count_frames);
    }
}

/// Draws the meter at `corner` of `gui`, `offset` in from it, on the `order` layer.
pub fn show_meter(
    gui: &egui::Context,
    rates: &FrameRates,
    order: egui::Order,
    corner: Align2,
    offset: Vec2,
) {
    let Some(latest) = rates.latest() else {
        return;
    };
    let (low, high) = rates.range();
    let readout = format!("{:.0} FPS ({:.0}-{:.0})", latest.0, low.0, high.0);
    let bars: Vec<f32> = rates.runs.iter().map(|rate| rate.0 / high.0).collect();
    meter::show(
        gui,
        "frame rate meter",
        order,
        corner,
        offset,
        &Meter {
            readout: &readout,
            bars: &bars,
            slots: runs_kept(),
        },
    );
}

/// How far back the meter looks.
const HISTORY: Seconds = Seconds(10.0);

/// Frames are counted in runs of at least this long, each one bar of the graph.
const RUN: Seconds = Seconds(0.25);

const MARGIN: f32 = 8.0;

/// The frame rate of each run of frames over the last [`HISTORY`], oldest first.
#[derive(Resource, Default)]
pub struct FrameRates {
    runs: VecDeque<Hertz>,
    frames: u32,
    spent: Seconds,
}

impl FrameRates {
    fn latest(&self) -> Option<Hertz> {
        self.runs.back().copied()
    }

    fn range(&self) -> (Hertz, Hertz) {
        self.runs
            .iter()
            .fold((Hertz(f32::INFINITY), Hertz(0.0)), |(low, high), &rate| {
                (Hertz(low.0.min(rate.0)), Hertz(high.0.max(rate.0)))
            })
    }
}

/// A run's rate is its frames over the time they took, so however frames fall into runs, a steady
/// rate reads steady.
fn count_frames(time: Res<Time<Real>>, mut rates: ResMut<FrameRates>) {
    rates.frames += 1;
    rates.spent.0 += time.delta_secs();
    if rates.spent < RUN {
        return;
    }
    let rate = Hertz(rates.frames as f32 / rates.spent.0);
    rates.runs.push_back(rate);
    while rates.runs.len() > runs_kept() {
        rates.runs.pop_front();
    }
    rates.frames = 0;
    rates.spent = Seconds(0.0);
}

fn draw_meter(world: &mut World) {
    if !world.resource::<UserSettings>().frame_rate_shown() {
        return;
    }
    let Some(gui) = gui::context(world) else {
        return;
    };
    show_meter(
        &gui,
        world.resource::<FrameRates>(),
        Band::System.order(),
        Align2::LEFT_BOTTOM,
        Vec2::new(MARGIN, -MARGIN),
    );
}

fn runs_kept() -> usize {
    (HISTORY.0 / RUN.0).round() as usize
}
