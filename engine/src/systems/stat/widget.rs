use bevy::prelude::*;
use design_tokens::color;
use egui::{RichText, Ui, Vec2};

use crate::systems::class;
use crate::systems::hud::HudWindow;
use crate::systems::input::map::InputAction;
use crate::systems::interface::InterfaceIcon;
use crate::systems::player::session;
use crate::systems::stat;

const WINDOW_SIZE: Vec2 = Vec2::new(300.0, 260.0);
const LABEL_WIDTH: f32 = 140.0;
const ROW_SPACING: Vec2 = Vec2::new(16.0, 6.0);

pub struct StatsWindow;

impl HudWindow for StatsWindow {
    fn title(&self) -> &'static str {
        "Stats"
    }
    fn toggle(&self) -> InputAction {
        InputAction::ToggleStats
    }
    fn icon(&self) -> InterfaceIcon {
        InterfaceIcon::Stats
    }
    fn size(&self) -> Vec2 {
        WINDOW_SIZE
    }
    fn show(&self, ui: &mut Ui, world: &mut World) {
        let rows = stat_rows(world);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                egui::Grid::new("stats")
                    .num_columns(2)
                    .min_col_width(LABEL_WIDTH)
                    .spacing(ROW_SPACING)
                    .show(ui, |ui| {
                        for (label, value) in rows {
                            ui.label(RichText::new(label).color(color::MUTED));
                            ui.label(value);
                            ui.end_row();
                        }
                    });
            });
    }
}

fn stat_rows(world: &World) -> Vec<(String, String)> {
    let Some(me) = session::my_character(world) else {
        return Vec::new();
    };
    let entity = me.id();
    let stats = stat::effective_all(world, entity);
    let content = world.resource::<crate::core::content::Content>();
    let class = class::of(world, entity)
        .map(|class| class.get(content).name.clone())
        .unwrap_or_default();
    [
        ("Class".to_owned(), class),
        ("Level".to_owned(), class::level(world, entity).to_string()),
    ]
    .into_iter()
    .chain(
        stats
            .0
            .iter()
            .map(|stat| (stat.label().to_owned(), shown_value(stat.value))),
    )
    .collect()
}

fn shown_value(value: f32) -> String {
    if (value - value.round()).abs() < 0.05 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}
