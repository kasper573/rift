use bevy::prelude::*;
use ui::egui::{Align, Layout, Stroke, Ui};
use ui::{face, ink};

use crate::core::content::Content;
use crate::systems::player::session;
use crate::systems::{class, stat};

const ROW_PADDING: f32 = 2.0;

pub fn pane(ui: &mut Ui, world: &World) {
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.spacing_mut().interact_size.y = 0.0;
    for (label, value) in stat_rows(world) {
        ui.add_space(ROW_PADDING);
        ui.horizontal(|ui| {
            ui.label(face::body(label));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(face::bold(value));
            });
        });
        ui.add_space(ROW_PADDING);
        let line = ui.cursor().min.y;
        ui.painter().hline(
            ui.max_rect().x_range(),
            line,
            Stroke::new(1.0, ink::ink_wash(0.12)),
        );
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
