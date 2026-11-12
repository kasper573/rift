//! The wheel of emotes, on the client: held open round the player's character, with a slot to
//! click for each emote its body can do.

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use ui::egui;
use ui::wheel::{self, WheelPick};

use super::{EmoteDef, EmoteId, EmoteRequest};
use crate::core::content::Content;
use crate::core::gui::{self, Band, GuiImage};
use crate::core::render::window_point;
use crate::core::units::Meters;
use crate::systems::actor::Hitbox;
use crate::systems::input::map::{self, InputAction};
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;

pub struct EmoteWheelPlugin;

impl Plugin for EmoteWheelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            EguiPrimaryContextPass,
            draw.in_set(Band::Hud).run_if(in_state(ClientScreen::Area)),
        );
    }
}

fn draw(world: &mut World) {
    if !map::pressed(world, InputAction::Emotes) || !session::is_alive(world) {
        return;
    }
    let (Some(gui), Some(me)) = (gui::context(world), session::me(world)) else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let offered: Vec<(EmoteId, &EmoteDef)> = content
        .ids::<EmoteDef>()
        .map(|emote| (emote, emote.get(&content)))
        .filter(|(_, emote)| super::offered(world, me, emote))
        .collect();
    let stands = world
        .get::<RenderPosition>(me)
        .map(|rendered| rendered.0)
        .or_else(|| world.get::<Position>(me).map(|position| position.pos));
    let waist = world
        .get::<Hitbox>(me)
        .map(|hitbox| Meters(hitbox.0.height.0 / 2.0));
    let Some(center) = stands
        .zip(waist)
        .and_then(|(at, waist)| window_point(world, at, waist))
        .filter(|_| !offered.is_empty())
    else {
        return;
    };
    let pictures: Vec<Option<GuiImage>> = offered
        .iter()
        .map(|(_, emote)| gui::image(world, &emote.icon))
        .collect();
    let picks = offered
        .iter()
        .zip(&pictures)
        .map(|((_, emote), picture)| WheelPick {
            name: &emote.name,
            image: picture
                .as_ref()
                .map(|picture| picture.widget(egui::Vec2::splat(wheel::PICTURE))),
        })
        .collect();
    let picked = egui::Area::new(egui::Id::new("emotes"))
        .order(Band::Hud.order())
        .fixed_pos(egui::Pos2::ZERO)
        .constrain(false)
        .show(&gui, |ui| {
            wheel::wheel(ui, egui::pos2(center.x, center.y), picks)
        })
        .inner;
    if let Some(index) = picked {
        world.write_message(EmoteRequest {
            emote: offered[index].0,
        });
    }
}
