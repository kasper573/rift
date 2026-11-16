//! The mount plate: a piece of the HUD shown while the player rides, naming the mount, with the
//! way off it. It is as small as what it holds, and rests in the bottom right corner of the
//! screen.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::{self, Align2, Margin, Pos2, Rect, Ui, Vec2};
use ui::{controls, dialog, face, sheet};

use super::seat::MountRequest;
use super::{MountId, MountKind, Riding, Travels};
use crate::core::content::Content;
use crate::systems::hud::{self, HudWidget};
use crate::systems::input::map::{self, InputAction, InputMap};
use crate::systems::notification;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::skill::aim;

pub const MOUNT_PLATE: &str = "mount";

pub fn plugin(app: &mut App) {
    app.add_systems(Update, keys.run_if(in_state(ClientScreen::Area)));
}

pub struct MountPlateWidget;

/// The mount the player's character rides, as the plate shows it.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownMount {
    pub mount: MountId,
    pub kind: MountKind,
    /// Why they cannot step off where they are, in the game's words.
    pub step_off_shut: Option<String>,
}

pub fn shown(world: &mut World) -> Option<ShownMount> {
    let me = session::me(world)?;
    let riding = riding(world)?;
    let def = riding.mount.get(world.resource::<Content>());
    Some(ShownMount {
        mount: riding.mount,
        kind: MountKind::from(def.travels),
        step_off_shut: shut(world, me),
    })
}

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
        let title = riding.mount.get(&content).name.clone();
        let shut = shut(world, me);
        let key = world.resource::<InputMap>().name(InputAction::StepOff);
        let mut leaves = false;
        sheet::note(ui, Margin::same(dialog::PADDING), |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(face::bold(title.as_str())).extend());
                let leave = controls::keyed(ui, "Step off", key.as_deref(), shut.is_none());
                let leave = match &shut {
                    Some(why) => leave.on_hover_text(why.as_str()),
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

fn riding(world: &World) -> Option<Riding> {
    session::my_character(world)?.get::<Riding>().copied()
}

/// Whether `rider` floats where no walker could stand, and so picks the ground they leave for.
fn afloat(world: &World, rider: Entity) -> bool {
    let content = world.resource::<Content>();
    matches!(
        super::travels(content, world, rider),
        Some(Travels::Floating { .. })
    ) && !super::over_ground(world, rider)
}

/// Why `rider` cannot step off where they are, in the game's words: nothing afloat, where the
/// ground is theirs to pick.
fn shut(world: &World, rider: Entity) -> Option<String> {
    if afloat(world, rider) {
        return None;
    }
    super::landing(world, rider, None).err()
}
