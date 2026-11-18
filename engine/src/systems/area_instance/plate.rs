//! What an instanced area adds under its name beneath the minimap: that it is instanced, its
//! goals, its roster, the time it has left and the way out, lettered as the trackers under it
//! are. Leaving is for good, so the way out asks first. Once the instance is cleared it says so,
//! and a dialog tells the player what that gave them.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::emath::TSTransform;
use ui::egui::{Align, Align2, Color32, Id, Layout, Pos2, Rect, Ui, UiBuilder, Vec2};
use ui::face::TextRole;
use ui::tracker::{self, TrackerLine, TrackerTone};
use ui::{controls, dusk};

use super::{
    AreaInstanceGiven, AreaInstanceGoalShown, AreaInstanceShown, LeaveInstance, RosterPlace,
};
use crate::core::content::Content;
use crate::core::text::LineText;
use crate::core::time::{self, UnixMillis};
use crate::core::units::Seconds;
use crate::systems::hud::{self, Confirm};
use crate::systems::item::deliver;
use crate::systems::mention::Mention;
use crate::systems::messages;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::settings::UserSettings;
use crate::systems::society::SocietyView;

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<Lettered>()
        .add_systems(Update, announce.run_if(in_state(ClientScreen::Area)));
}

/// How tall what the instance adds under the area's name was when last lettered: the room the
/// minimap keeps for it.
pub fn height(world: &World) -> f32 {
    world.resource::<Lettered>().0
}

/// Letters what the instance the player is inside adds under the area's name, whose foot is at
/// `top`: in the trackers' column, its right edge at `right`, the line naming it instanced
/// centred under the name at `center`. Outside an instance it letters nothing.
pub fn under_title(ui: &mut Ui, world: &mut World, top: f32, right: f32, center: f32) {
    let lettered = instance(world)
        .zip(time::server_now(world))
        .map_or(0.0, |(instance, now)| {
            lettered(ui, world, &instance, now, Pos2::new(right, top), center) - top
        });
    world.resource_mut::<Lettered>().0 = lettered;
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

#[derive(Resource, Default)]
struct Lettered(f32);

fn instance(world: &World) -> Option<AreaInstanceShown> {
    session::my_character(world)?
        .get::<SocietyView>()?
        .instance
        .clone()
}

/// Letters the instance from `right_top` down, on the trackers' plate, and tells where it ends.
fn lettered(
    ui: &mut Ui,
    world: &mut World,
    instance: &AreaInstanceShown,
    now: UnixMillis,
    right_top: Pos2,
    center: f32,
) -> f32 {
    let content = world.resource::<Content>().clone();
    let says = &messages::of(&content).instance;
    let backing = ui.painter().add(ui::egui::Shape::Noop);
    let left = right_top.x - tracker::WIDTH;
    let id = ui.id().with("instance");
    let mut foot = centred(ui, says.instanced(), Pos2::new(center, right_top.y + GAP));
    let plate_top = foot + GAP;
    let line = |ui: &mut Ui, at: f32, text: &str, tone: TrackerTone, key: (&str, usize)| {
        TrackerLine {
            at: Pos2::new(left, at + tracker::ROW_GAP),
            width: tracker::WIDTH,
            mark: tone.mark().map(|mark| (mark, tone.color())),
            text,
            font: TextRole::Small.font_id(),
            color: tone.color(),
        }
        .letter(ui, id.with(key))
        .bottom()
    };
    foot = plate_top;
    match &instance.cleared {
        Some(cleared) => {
            foot = line(ui, foot, says.cleared(), TrackerTone::Done, ("cleared", 0));
            let closes =
                says.closes_in(messages::of(&content).time.rough(cleared.closes.since(now)));
            foot = line(ui, foot, &closes, TrackerTone::Timed, ("closes", 0));
        }
        None => {
            for (index, goal) in instance.goals.iter().enumerate() {
                foot = goal_row(
                    ui,
                    says,
                    goal,
                    Pos2::new(left, foot),
                    id.with(("goal", index)),
                );
            }
            if let Some(ends) = instance.ends {
                let left_of_it = messages::of(&content).time.remaining(ends.since(now));
                foot = line(ui, foot, &left_of_it, TrackerTone::Timed, ("left", 0));
            }
        }
    }
    foot += tracker::ENTRY_GAP;
    foot = TrackerLine {
        at: Pos2::new(left, foot),
        width: tracker::WIDTH,
        mark: None,
        text: says.roster(),
        font: TextRole::Section.font_id(),
        color: ui::ink::GOLD2,
    }
    .letter(ui, id.with("roster"))
    .bottom();
    for (index, row) in instance.roster.iter().enumerate() {
        let name = if row.me {
            says.you_named(&row.character.name)
        } else {
            row.character.name.clone()
        };
        let (place, tone) = match row.place {
            RosterPlace::Inside => (says.inside(), TrackerTone::Done),
            RosterPlace::Away => (says.away(), TrackerTone::Timed),
            RosterPlace::Left => (says.left(), TrackerTone::Bad),
        };
        let at = Pos2::new(left, foot + tracker::ROW_GAP);
        foot = beside(ui, at, &name, place, tone, id.with(("member", index)));
    }
    foot += tracker::ENTRY_GAP;
    let (leave, cleared) = match &instance.cleared {
        Some(_) => (says.leave_now(), true),
        None => (says.leave(), false),
    };
    let button = Rect::from_min_size(
        Pos2::new(left, foot),
        Vec2::new(tracker::WIDTH, controls::CONTROL_HEIGHT),
    );
    let mut leaves = false;
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(button)
            .layout(Layout::right_to_left(Align::Center)),
        |ui| leaves = controls::button(ui, leave).clicked(),
    );
    foot = button.bottom();
    let opacity = world.resource::<UserSettings>().tracker_backing().0 / 100.0;
    let plate = Rect::from_min_max(Pos2::new(left, plate_top), Pos2::new(right_top.x, foot));
    ui.painter().set(
        backing,
        dusk::plate_shape(plate.expand(PLATE_PADDING), opacity),
    );
    if leaves && cleared {
        world.write_message(LeaveInstance);
    } else if leaves {
        confirm_leave(world, instance);
    }
    foot + PLATE_PADDING
}

/// A goal's row: its label struck once it is met, and how far it has come beside it while that
/// is more than met or not.
fn goal_row(
    ui: &mut Ui,
    says: &super::InstanceMessages,
    goal: &AreaInstanceGoalShown,
    at: Pos2,
    id: Id,
) -> f32 {
    let met = goal.done >= goal.of;
    let tone = if met {
        TrackerTone::Done
    } else {
        TrackerTone::Plain
    };
    let at = Pos2::new(at.x, at.y + tracker::ROW_GAP);
    if goal.of > 1 || !met {
        beside(
            ui,
            at,
            &goal.label,
            &says.goal_count(goal.done, goal.of),
            tone,
            id,
        )
    } else {
        let lead = tone.mark().map_or(0.0, |mark| {
            tracker::mark(ui, at, mark, tone.color(), TextRole::Small.size())
        });
        TrackerLine {
            at: Pos2::new(at.x + lead, at.y),
            width: tracker::WIDTH - lead,
            mark: None,
            text: &goal.label,
            font: TextRole::Small.font_id(),
            color: tone.color(),
        }
        .letter(ui, id)
        .bottom()
    }
}

/// A row of two cells: `name` from `at`, behind its tone's mark, and `aside` at the column's
/// right edge in the tone's colour.
fn beside(ui: &mut Ui, at: Pos2, name: &str, aside: &str, tone: TrackerTone, id: Id) -> f32 {
    let font = TextRole::Small.font_id();
    let galley = ui
        .painter()
        .layout_no_wrap(aside.to_owned(), font.clone(), tone.color());
    let right = Pos2::new(at.x + tracker::WIDTH, at.y);
    let aside_at = Align2::RIGHT_TOP.anchor_size(right, galley.size());
    dusk::lettering(
        ui.painter(),
        aside_at.min,
        galley,
        dusk::EDGED,
        TSTransform::IDENTITY,
    );
    let lead = tone.mark().map_or(0.0, |mark| {
        tracker::mark(ui, at, mark, tone.color(), font.size)
    });
    TrackerLine {
        at: Pos2::new(at.x + lead, at.y),
        width: aside_at.left() - at.x - lead - ASIDE_GAP,
        mark: None,
        text: name,
        font,
        color: Color32::WHITE,
    }
    .letter(ui, id)
    .bottom()
    .max(aside_at.bottom())
}

/// Letters `text` centred on `top`, as the area's name above it is, and tells where it ends.
fn centred(ui: &mut Ui, text: &str, top: Pos2) -> f32 {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), TextRole::Small.font_id(), dusk::TEXT);
    let rect = Align2::CENTER_TOP.anchor_size(top, galley.size());
    ui.interact(rect, ui.id().with("instanced"), ui::egui::Sense::hover())
        .widget_info(|| ui::egui::WidgetInfo::labeled(ui::egui::WidgetType::Label, true, text));
    dusk::lettering(
        ui.painter(),
        rect.min,
        galley,
        dusk::TEXT_SHADOW,
        TSTransform::IDENTITY,
    );
    rect.bottom()
}

/// Asks the player whether to leave for good before they do.
fn confirm_leave(world: &mut World, instance: &AreaInstanceShown) {
    let content = world.resource::<Content>().clone();
    let says = &messages::of(&content).instance;
    let area = |area| Mention::Area(area).span(&content);
    hud::confirm(
        world,
        Confirm {
            title: says.leave_title(area(instance.area)),
            body: vec![says.leave_body(area(instance.returns_to))],
            confirm: says.leave_for_good().to_owned(),
            cancel: says.stay().to_owned(),
            answer: Box::new(|world, leaves| {
                if leaves {
                    world.write_message(LeaveInstance);
                }
            }),
        },
    );
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
    let all = messages::of(&content);
    let says = &all.instance;
    let given = &cleared.given;
    let mut kept: Vec<LineText> = given.kept.iter().map(|stack| stack.named()).collect();
    if given.xp > 0 {
        kept.push(says.experience(given.xp).into());
    }
    let mut body = Vec::new();
    if !kept.is_empty() {
        body.push(says.yours(all.interface.listed(kept)));
    }
    if !given.sent.is_empty() {
        body.push(deliver::overflow_line(&content, &given.sent));
    }
    body.push(says.return_when(Mention::Area(instance.returns_to).span(&content)));
    hud::confirm(
        world,
        Confirm {
            title: says.cleared_title(Mention::Area(instance.area).span(&content)),
            body,
            confirm: says.leave_now().to_owned(),
            cancel: says.stay_till().to_owned(),
            answer: Box::new(|world, leaves| {
                if leaves {
                    world.write_message(LeaveInstance);
                }
            }),
        },
    );
}

/// The room between the area's name, the line naming it instanced and the plate under that.
const GAP: f32 = 2.0;
const PLATE_PADDING: f32 = 6.0;
const ASIDE_GAP: f32 = 8.0;
