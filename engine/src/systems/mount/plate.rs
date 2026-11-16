//! The mount plate: a piece of the HUD shown while the player rides, naming the mount, with the
//! way off it and, for a driver, a way to set down each passenger aboard. It is as small as what
//! it holds, and rests in the bottom right corner of the screen however large that makes it.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::{self, Align2, Margin, Pos2, Rect, Ui, Vec2};
use ui::{controls, dialog, face, sheet};

use super::seat::MountRequest;
use super::{MountId, MountKind, Riding, SeatIndex, Travels};
use crate::core::content::Content;
use crate::systems::actor::Name;
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
    /// The seat the player's character holds, the driver's first.
    pub seat: SeatIndex,
    /// Who sits in each seat by name, the driver's first: none in a seat that is free.
    pub seats: Vec<Option<String>>,
    /// Why they cannot step off where they are, in the game's words.
    pub step_off_shut: Option<String>,
}

pub fn shown(world: &mut World) -> Option<ShownMount> {
    let me = session::me(world)?;
    let riding = riding(world)?;
    let def = riding.mount.get(world.resource::<Content>());
    let (kind, seats) = (MountKind::from(def.travels), def.seats.len());
    let mut sat = vec![None; seats.max(1)];
    sat[0] = Some(named(world, riding.driver));
    for (seat, rider) in passengers(world, riding.driver) {
        if let Some(held) = sat.get_mut(usize::from(seat.0)) {
            *held = Some(rider);
        }
    }
    Some(ShownMount {
        mount: riding.mount,
        kind,
        seat: riding.seat,
        seats: sat,
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
        let def = riding.mount.get(&content);
        let (title, aboard) = if riding.drives() {
            (def.name.clone(), passengers(world, me))
        } else {
            let driver = named(world, riding.driver);
            (format!("{driver}'s {}", def.name), Vec::new())
        };
        let shut = shut(world, me);
        let key = world.resource::<InputMap>().name(InputAction::StepOff);
        let mut asked = None;
        sheet::note(ui, Margin::same(dialog::PADDING), |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(face::bold(title.as_str())).extend());
                let leave = controls::keyed(ui, "Step off", key.as_deref(), shut.is_none());
                let leave = match &shut {
                    Some(why) => leave.on_hover_text(why.as_str()),
                    None => leave,
                };
                if leave.clicked() {
                    asked = Some(Asked::StepOff);
                }
            });
            if !aboard.is_empty() {
                ui.horizontal(|ui| {
                    for (seat, rider) in &aboard {
                        if controls::button(ui, format!("Set down {rider}")).clicked() {
                            asked = Some(Asked::SetDown(*seat));
                        }
                    }
                });
            }
        });
        match asked {
            Some(Asked::StepOff) => step_off(world),
            Some(Asked::SetDown(seat)) => {
                world.write_message(MountRequest::SetDown { seat });
            }
            None => {}
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

enum Asked {
    StepOff,
    SetDown(SeatIndex),
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

/// Everyone `driver` carries by name, each with the seat they hold, in the order of the seats.
fn passengers(world: &mut World, driver: Entity) -> Vec<(SeatIndex, String)> {
    super::passengers(world, driver)
        .into_iter()
        .map(|(rider, seat)| (seat, named(world, rider)))
        .collect()
}

fn named(world: &World, character: Entity) -> String {
    world
        .get::<Name>(character)
        .map(|name| name.name.clone())
        .unwrap_or_default()
}
