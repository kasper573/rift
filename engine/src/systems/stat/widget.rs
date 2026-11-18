use bevy::prelude::*;
use ui::egui::{Align, Layout, Stroke, Ui};
use ui::{face, ink};

use crate::core::content::Content;
use crate::systems::actor::Name;
use crate::systems::player::session;
use crate::systems::stat::StatMessages;
use crate::systems::{class, messages, stat};

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
    let name = me
        .get::<Name>()
        .map(|name| name.name.clone())
        .unwrap_or_default();
    let words = &messages::of(content).stat;
    [
        (words.name().to_owned(), name),
        (words.class().to_owned(), class),
        (
            words.level().to_owned(),
            words.figure(class::level(world, entity)),
        ),
    ]
    .into_iter()
    .chain(
        stats
            .listed()
            .map(|stat| (stat.kind.named(words).to_owned(), shown(words, stat))),
    )
    .collect()
}

fn shown(words: &StatMessages, stat: stat::Stat) -> String {
    match stat.kind.unit() {
        Some(unit) => words.figure_with_unit(unit, stat.value),
        None => words.figure(stat.value),
    }
}
