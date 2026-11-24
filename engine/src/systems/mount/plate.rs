//! The mount plate: a piece of the HUD shown while the player rides, naming the mount, with the
//! way off it. It is as small as what it holds, and rests in the bottom right corner of the
//! screen.

use bevy::prelude::*;
use ui::egui::{self, Align2, Margin, Pos2, Rect, Ui, Vec2};
use ui::{controls, dialog, face, sheet};

use super::seat::MountRequest;
use super::{afloat, riding, step_off_shut};
use crate::core::content::Content;
use crate::systems::hud::{self, HudWidget};
use crate::systems::input::map::{self, InputAction, InputMap};
use crate::systems::input::scope;
use crate::systems::messages;
use crate::systems::notification;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::skill::aim;

pub const MOUNT_PLATE: &str = "mount";

pub fn plugin(app: &mut App) {
    app.add_systems(Update, keys.run_if(in_state(ClientScreen::Area)));
}

pub struct MountPlateWidget;

impl HudWidget for MountPlateWidget {
    fn fallback(&self, _: &World, screen: Rect) -> Pos2 {
        screen.right_bottom() - Vec2::splat(hud::MARGIN)
    }

    fn rests_by(&self) -> Align2 {
        Align2::RIGHT_BOTTOM
    }

    fn hidden(&self, world: &World) -> bool {
        riding(world).is_none()
    }

    fn show(&self, ui: &mut Ui, world: &mut World) -> Option<Rect> {
        let me = session::me(world)?;
        let riding = riding(world)?;
        let content = world.resource::<Content>().clone();
        let words = &messages::of(&content).mount;
        let title = riding.mount.get(&content).name.clone();
        let shut = step_off_shut(world, me);
        let key = (shut.is_none() && scope::reads(world, InputAction::StepOff))
            .then(|| world.resource::<InputMap>().cap(InputAction::StepOff))
            .flatten();
        let mut leaves = false;
        sheet::note(ui, Margin::same(dialog::PADDING), |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(face::bold(title.as_str())).extend());
                let leave = controls::keyed(ui, words.step_off(), key.as_ref(), shut.is_none());
                let leave = match &shut {
                    Some(why) => leave.on_hover_text(
                        why.words(&content, &world.resource::<InputMap>().key_names()),
                    ),
                    None => leave,
                };
                leaves = leave.clicked();
            });
        });
        if leaves {
            step_off(world);
        }
        None
    }
}

/// Leaves the seat the player's character holds: at once where there is ground under what they
/// ride or it flies, and afloat by arming the aim for the ground to step onto.
pub fn step_off(world: &mut World) {
    let Some(me) = session::me(world) else {
        return;
    };
    if afloat(world, me) {
        aim::arm_landing(world);
        return;
    }
    match super::landing(world, me, None) {
        Ok(_) => {
            world.write_message(MountRequest::StepOff { at: None });
        }
        Err(refusal) => notification::local_error(world, refusal),
    }
}

fn keys(world: &mut World) {
    if riding(world).is_some() && map::just_pressed(world, InputAction::StepOff) {
        step_off(world);
    }
}
