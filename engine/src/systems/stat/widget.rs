use bevy::prelude::*;
use ui::egui;
use ui::egui::{Ui, Vec2};
use ui::{face, sheet};

use crate::core::content::Content;
use crate::systems::hud::HudWindow;
use crate::systems::input::map::InputAction;
use crate::systems::interface::InterfaceIcon;
use crate::systems::player::session;
use crate::systems::{class, stat};

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
        sheet::scroll(ui, egui::ScrollArea::vertical().auto_shrink(false), |ui| {
            egui::Grid::new("stats")
                .num_columns(2)
                .min_col_width(LABEL_WIDTH)
                .spacing(ROW_SPACING)
                .show(ui, |ui| {
                    for (label, value) in rows {
                        ui.label(face::body(label));
                        ui.label(face::bold(value));
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
    let content = world.resource::<Content>();
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
            .listed()
            .map(|stat| (stat.label().to_owned(), shown(stat))),
    )
    .collect()
}

fn shown(stat: stat::Stat) -> String {
    let value = if (stat.value - stat.value.round()).abs() < 0.05 {
        format!("{:.0}", stat.value)
    } else {
        format!("{:.1}", stat.value)
    };
    match stat.kind.unit() {
        Some(unit) => format!("{value} {unit}"),
        None => value,
    }
}
