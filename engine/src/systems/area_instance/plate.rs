//! The instance plate: a piece of the HUD there only inside an instance, naming the area and
//! listing its goals and its roster, with the time it has left and the way out. Leaving is for
//! good, so the plate asks first. Once the instance is cleared it says so, and a card tells the
//! player what that gave them.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::{self, Align, Layout, Pos2, Rect, Ui, Vec2};
use ui::face::TextRole;
use ui::{controls, dialog, face, ink};

use super::{
    AreaInstanceGiven, AreaInstanceGoalShown, AreaInstanceShown, LeaveInstance, RosterPlace,
    RosterRowInside,
};
use crate::core::content::Content;
use crate::core::text::{self, TextLook};
use crate::core::time::{self, UnixMillis};
use crate::core::units::Seconds;
use crate::systems::GameSettings;
use crate::systems::hud::{self, Confirm, HudWidget};
use crate::systems::item::ItemStack;
use crate::systems::mention::Mention;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::social_request::card::time_left;
use crate::systems::society::SocietyView;
use crate::systems::text::LineText;

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
        let returns = LineText::plain("You return to ")
            .then(Mention::Area(instance.returns_to).span(&content));
        let mut leaves = None;
        dialog::note(ui, WIDTH, |ui| {
            ui.add(egui::Label::new(face::bold(name)).truncate());
            match &instance.cleared {
                Some(cleared) => {
                    ui.label(face::body("Cleared").color(ink::VERDIGRIS));
                    roster(ui, &instance.roster);
                    if way_out(ui, world, &returns, Some(cleared.closes), now, "Leave now") {
                        leaves = Some(false);
                    }
                }
                None => {
                    for goal in &instance.goals {
                        goal_row(ui, goal);
                    }
                    roster(ui, &instance.roster);
                    let left = instance.ends.map_or_else(LineText::default, |ends| {
                        LineText::plain(format!("{} left", time_left(ends, now)))
                    });
                    if way_out(ui, world, &left, None, now, "Leave") {
                        leaves = Some(true);
                    }
                }
            }
        });
        match leaves {
            Some(true) => confirm_leave(world, &instance),
            Some(false) => {
                world.write_message(LeaveInstance);
            }
            None => {}
        }
        None
    }
}

/// The instance the player is inside, as the plate shows it, for a driver of the client.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownAreaInstance {
    pub area: String,
    pub goals: Vec<AreaInstanceGoalShown>,
    /// Its roster, in the order they were taken.
    pub roster: Vec<ShownRosterMember>,
    /// How long it has left uncleared, in an area whose instances only last so long.
    pub left: Option<Seconds>,
    /// Once it is cleared: what that gave the player.
    pub cleared: Option<AreaInstanceGiven>,
    /// How long until a cleared one closes and everyone is returned.
    pub returned_in: Option<Seconds>,
}

/// A member of the roster, as the plate lists them.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownRosterMember {
    pub name: String,
    pub place: RosterPlace,
    pub me: bool,
}

pub fn shown(world: &World) -> Option<ShownAreaInstance> {
    let instance = instance(world)?;
    let now = time::server_now(world)?;
    let closes = instance.cleared.as_ref().map(|cleared| cleared.closes);
    Some(ShownAreaInstance {
        area: instance.area.get(world.resource::<Content>()).name.clone(),
        goals: instance.goals.clone(),
        roster: instance
            .roster
            .iter()
            .map(|row| ShownRosterMember {
                name: row.character.name.clone(),
                place: row.place,
                me: row.me,
            })
            .collect(),
        left: instance.ends.map(|ends| ends.since(now)),
        cleared: instance.cleared.map(|cleared| cleared.given),
        returned_in: closes.map(|at| at.since(now)),
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

/// The roster under a heading, each member's place beside their name in words.
fn roster(ui: &mut Ui, rows: &[RosterRowInside]) {
    ui.add_space(4.0);
    ui.separator();
    ui.label(face::small("ROSTER").color(ink::INK3));
    for row in rows {
        let name = if row.me {
            format!("{} (you)", row.character.name)
        } else {
            row.character.name.clone()
        };
        let (says, color) = match row.place {
            RosterPlace::Inside => ("Inside", ink::VERDIGRIS),
            RosterPlace::Away => ("Away · place kept", ink::OCHRE),
            RosterPlace::Left => ("Left", ink::INK3),
        };
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(face::small(says).color(color));
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let name = face::small(name);
                    let name = match row.place {
                        RosterPlace::Left => name.color(ink::INK3).strikethrough(),
                        _ => name.color(ink::INK),
                    };
                    ui.add(egui::Label::new(name).truncate());
                });
            });
        });
    }
}

/// Asks the player whether to leave for good before they do.
fn confirm_leave(world: &mut World, instance: &AreaInstanceShown) {
    let content = world.resource::<Content>().clone();
    let warning = world
        .resource::<GameSettings>()
        .words
        .instance_leave_warning
        .clone();
    hud::confirm(
        world,
        Confirm {
            title: LineText::plain("Leave ")
                .then(Mention::Area(instance.area).span(&content))
                .then("?"),
            body: vec![
                LineText::plain(warning)
                    .then(" You return to ")
                    .then(Mention::Area(instance.returns_to).span(&content))
                    .then("."),
            ],
            confirm: "Leave for good".to_owned(),
            cancel: "Stay".to_owned(),
            answer: Box::new(|world, leaves| {
                if leaves {
                    world.write_message(LeaveInstance);
                }
            }),
        },
    );
}

/// The plate's last line: what it says, the time left until `until` beside it, and the button
/// that leaves. Whether the button was pressed.
fn way_out(
    ui: &mut Ui,
    world: &World,
    says: &LineText,
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
                    Some(until) => says.clone().then(format!(" · {}", time_left(until, now))),
                    None => says.clone(),
                };
                let look = TextLook::role(TextRole::Small, ink::INK3);
                text::show(
                    ui,
                    world,
                    &said.styled(world.resource::<Content>()),
                    look,
                    None,
                );
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
    let named = |stacks: &[ItemStack]| -> Vec<LineText> {
        stacks.iter().map(|stack| stack.named()).collect()
    };
    let given = &cleared.given;
    let mut kept = named(&given.kept);
    if given.xp > 0 {
        kept.push(format!("{} experience", given.xp).into());
    }
    let mut body = Vec::new();
    if !kept.is_empty() {
        body.push(
            LineText::plain("Yours: ")
                .then(LineText::listed(kept))
                .then("."),
        );
    }
    if !given.sent.is_empty() {
        body.push(
            LineText::plain("No room for ")
                .then(LineText::listed(named(&given.sent)))
                .then(": sent to your Messages."),
        );
    }
    body.push(
        LineText::plain("You return to ")
            .then(Mention::Area(instance.returns_to).span(&content))
            .then(" when the time on the plate is up."),
    );
    hud::confirm(
        world,
        Confirm {
            title: Mention::Area(instance.area)
                .line(&content)
                .then(" is cleared"),
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
