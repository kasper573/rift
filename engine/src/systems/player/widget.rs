use bevy::prelude::*;
use egui::{Pos2, Rect, Ui};

use crate::systems::actor::Name;
use crate::systems::class;
use crate::systems::hud::{self, HudWidget};
use crate::systems::player::{Xp, session};
use crate::systems::stat;
use crate::systems::status;

const WIDTH: f32 = 140.0;

pub struct CharacterWidget;

impl HudWidget for CharacterWidget {
    fn fallback(&self, screen: Rect) -> Pos2 {
        screen.min + egui::vec2(8.0, 8.0)
    }
    fn show(&self, ui: &mut Ui, world: &mut World) {
        let fresh = character_sheet(world);
        let mut last = world.get_resource_or_init::<LastSheet>();
        if let Some(fresh) = fresh {
            last.0 = fresh;
        }
        let sheet = last.0.clone();
        hud::panel().show(ui, |ui| {
            ui.set_width(WIDTH);
            for line in &sheet {
                ui.label(line);
            }
        });
        status::widget::grid(ui, world);
    }
}

// Between areas there is no character for a moment; the panel keeps what it last showed.
#[derive(Resource, Default)]
struct LastSheet(Vec<String>);

fn character_sheet(world: &World) -> Option<Vec<String>> {
    let me = session::my_character(world)?;
    let entity = me.id();
    let name = me
        .get::<Name>()
        .map_or_else(String::new, |n| n.name.clone());
    let xp = me.get::<Xp>().map_or(0, |x| x.amount);
    let health = stat::current_health(world, entity);
    let max = stat::max_health(world, entity);
    let content = world.resource::<crate::core::content::Content>();
    let mut lines = vec![name];
    if let Some(class) = class::of(world, entity) {
        lines.push(format!(
            "{} · level {}",
            class.get(content).name,
            class::level(world, entity)
        ));
    }
    lines.push(format!("{health:.0} / {max:.0}"));
    lines.push(format!("xp {xp}"));
    let points = class::points_to_spend(world, entity);
    if points.0 > 0 {
        lines.push(format!(
            "{} skill point{} to spend",
            points.0,
            if points.0 == 1 { "" } else { "s" }
        ));
    }
    Some(lines)
}
