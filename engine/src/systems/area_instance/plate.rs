//! The instance plate: a piece of the HUD there only inside an instance, naming the area and
//! listing its goals, with how many are inside, the time it has left and the way out. Once the
//! instance is cleared it says so, and a card tells the player what that gave them.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::{self, Align, Layout, Pos2, Rect, Ui, Vec2};
use ui::{controls, dialog, face, ink};

use super::{AreaInstanceGiven, AreaInstanceGoalShown, AreaInstanceShown, LeaveInstance};
use crate::core::content::Content;
use crate::core::time::{self, UnixMillis};
use crate::core::units::Seconds;
use crate::systems::GameSettings;
use crate::systems::hud::{self, Confirm, HudWidget};
use crate::systems::item::ItemStack;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::social_request::card::time_left;
use crate::systems::society::SocietyView;

pub const INSTANCE_PLATE: &str = "instance";

pub(crate) fn plugin(app: &mut App) {
    app.add_systems(Update, announce.run_if(in_state(ClientScreen::Area)));
}

pub struct InstancePlateWidget;

impl HudWidget for InstancePlateWidget {
    fn fallback(&self, _: &World, screen: Rect) -> Pos2 {
        screen.right_top() + Vec2::new(-(WIDTH + hud::MARGIN), FROM_TOP)
    }

    fn hidden(&self, world: &World) -> bool {
        instance(world).is_none()
    }

    fn show(&self, ui: &mut Ui, world: &mut World) -> Option<Rect> {
        let instance = instance(world)?;
        let now = time::server_now(world)?;
        let content = world.resource::<Content>().clone();
        let name = instance.area.get(&content).name.clone();
        let returns = format!("You return to {}", instance.returns_to.get(&content).name);
        let not_theirs = world
            .resource::<GameSettings>()
            .words
            .instance_not_yours
            .clone();
        let mut leaves = false;
        dialog::note(ui, WIDTH, |ui| {
            ui.add(egui::Label::new(face::bold(name)).truncate());
            match (instance.sent_out, &instance.cleared) {
                (Some(out_at), _) => {
                    ui.label(face::body(not_theirs).color(ink::VERMILION));
                    leaves = way_out(ui, &returns, Some(out_at), now, "Leave now");
                }
                (None, Some(cleared)) => {
                    ui.label(face::body("Cleared").color(ink::VERDIGRIS));
                    leaves = way_out(ui, &returns, Some(cleared.closes), now, "Leave now");
                }
                (None, None) => {
                    for goal in &instance.goals {
                        goal_row(ui, goal);
                    }
                    let inside = format!("{} of {} inside", instance.inside, instance.holds);
                    leaves = way_out(ui, &inside, instance.ends, now, "Leave");
                }
            }
        });
        if leaves {
            world.write_message(LeaveInstance);
        }
        None
    }
}

/// The instance the player is inside, as the plate shows it, for a driver of the client.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownAreaInstance {
    pub area: String,
    pub goals: Vec<AreaInstanceGoalShown>,
    pub inside: u32,
    pub holds: u32,
    /// How long it has left uncleared, in an area whose instances only last so long.
    pub left: Option<Seconds>,
    /// Once it is cleared: what that gave the player.
    pub cleared: Option<AreaInstanceGiven>,
    /// How long until the player is returned: one cleared closing, or one no longer theirs.
    pub returned_in: Option<Seconds>,
    /// Whether the player is being sent out of one that is no longer theirs to be in.
    pub sent_out: bool,
}

pub fn shown(world: &World) -> Option<ShownAreaInstance> {
    let instance = instance(world)?;
    let now = time::server_now(world)?;
    let closes = instance.cleared.as_ref().map(|cleared| cleared.closes);
    Some(ShownAreaInstance {
        area: instance.area.get(world.resource::<Content>()).name.clone(),
        goals: instance.goals.clone(),
        inside: instance.inside,
        holds: instance.holds,
        left: instance.ends.map(|ends| ends.since(now)),
        cleared: instance.cleared.map(|cleared| cleared.given),
        returned_in: instance.sent_out.or(closes).map(|at| at.since(now)),
        sent_out: instance.sent_out.is_some(),
    })
}

fn instance(world: &World) -> Option<AreaInstanceShown> {
    session::my_character(world)?
        .get::<SocietyView>()?
        .instance
        .clone()
}

fn goal_row(ui: &mut Ui, goal: &AreaInstanceGoalShown) {
    let met = goal.done >= goal.of;
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if goal.of > 1 || !met {
                let count = format!("{} of {}", goal.done, goal.of);
                ui.label(face::bold(count).color(if met { ink::INK3 } else { ink::INK }));
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let label = face::body(goal.label.as_str());
                let label = if met {
                    label.color(ink::INK3).strikethrough()
                } else {
                    label.color(ink::INK)
                };
                ui.add(egui::Label::new(label).truncate());
            });
        });
    });
}

/// The plate's last line: what it says, the time left until `until` beside it, and the button
/// that leaves. Whether the button was pressed.
fn way_out(
    ui: &mut Ui,
    says: &str,
    until: Option<UnixMillis>,
    now: UnixMillis,
    leave: &str,
) -> bool {
    let mut leaves = false;
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            leaves = controls::button(ui, leave).clicked();
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let said = match until {
                    Some(until) => format!("{says} · {}", time_left(until, now)),
                    None => says.to_owned(),
                };
                ui.add(egui::Label::new(face::small_faint(said)).truncate());
            });
        });
    });
    leaves
}

/// Tells the player what clearing the instance they are in gave them, once, as it is cleared.
fn announce(world: &mut World, mut told: Local<Option<UnixMillis>>) {
    let cleared = instance(world).and_then(|instance| Some((instance.clone(), instance.cleared?)));
    let Some((instance, cleared)) = cleared else {
        *told = None;
        return;
    };
    if *told == Some(cleared.closes) {
        return;
    }
    *told = Some(cleared.closes);
    let content = world.resource::<Content>().clone();
    let listed = |stacks: &[ItemStack]| {
        let named: Vec<String> = stacks
            .iter()
            .map(|stack| stack.describe(&content))
            .collect();
        ui::words::join_and(&named)
    };
    let given = &cleared.given;
    let mut kept: Vec<String> = given
        .kept
        .iter()
        .map(|stack| stack.describe(&content))
        .collect();
    if given.xp > 0 {
        kept.push(format!("{} experience", given.xp));
    }
    let mut body = Vec::new();
    if !kept.is_empty() {
        body.push(format!("Yours: {}.", ui::words::join_and(&kept)));
    }
    if !given.sent.is_empty() {
        body.push(format!(
            "No room for {}: sent to your Messages.",
            listed(&given.sent)
        ));
    }
    body.push(format!(
        "You return to {} when the time on the plate is up.",
        instance.returns_to.get(&content).name
    ));
    hud::confirm(
        world,
        Confirm {
            title: format!("{} is cleared", instance.area.get(&content).name),
            body,
            confirm: "Leave now".to_owned(),
            cancel: "Stay till then".to_owned(),
            answer: Box::new(|world, leaves| {
                if leaves {
                    world.write_message(LeaveInstance);
                }
            }),
        },
    );
}

const WIDTH: f32 = 280.0;
const FROM_TOP: f32 = 190.0;
