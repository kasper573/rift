use bevy::prelude::*;

use super::map::{self, InputAction, InputMap};
use super::scope::InputScope;
use crate::core::gui;
use crate::systems::dialogue::stage;
use crate::systems::hud::{WindowKey, menu};
use crate::systems::{
    actor, area_instance, crafting, hud, item, market, quest, resource, skill, terminal, trade,
};

pub(super) fn plugin(app: &mut App) {
    app.add_systems(Update, step_out.run_if(resource_exists::<InputMap>));
}

/// What a press of Dismiss steps out of, in the order the press tries them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dismissal {
    /// The keyboard goes back from the field or the list that has it.
    Typing,
    Confirm,
    ActorOptions,
    Rebind,
    Aim,
    /// The sheet in front closes.
    Sheet(WindowKey),
    Crafting,
    Conversation,
    Work,
    /// Nothing is there to step out of, so the menu opens.
    OpenMenu,
}

/// What a press of Dismiss would step out of right now, so a hint names the key only where the
/// press does what the hint says.
pub(crate) fn dismisses(world: &World) -> Option<Dismissal> {
    let front = || front_sheet(world).map(Dismissal::Sheet);
    let bar = || {
        if skill::bar::rebinding(world) {
            Some(Dismissal::Rebind)
        } else {
            skill::aim::aiming(world).map(|_| Dismissal::Aim)
        }
    };
    match *world.resource::<InputScope>() {
        InputScope::Typing => Some(Dismissal::Typing),
        InputScope::Modal => hud::confirming(world).then_some(Dismissal::Confirm),
        InputScope::Conversation => front()
            .or_else(|| crafting::panel::making(world).then_some(Dismissal::Crafting))
            .or_else(|| stage::leavable(world).then_some(Dismissal::Conversation)),
        InputScope::Menu => bar().or_else(front),
        InputScope::World => actor::options::is_open(world)
            .then_some(Dismissal::ActorOptions)
            .or_else(bar)
            .or_else(front)
            .or_else(|| resource::render::working(world).then_some(Dismissal::Work))
            .or(Some(Dismissal::OpenMenu)),
    }
}

fn step_out(world: &mut World) {
    if !map::just_pressed(world, InputAction::Dismiss) {
        return;
    }
    let Some(dismissal) = dismisses(world) else {
        return;
    };
    match dismissal {
        Dismissal::Typing => {
            terminal::chat::hand_back(world);
            gui::release_keyboard(world);
        }
        Dismissal::Confirm => {
            hud::dismiss_confirm(world);
        }
        Dismissal::ActorOptions => {
            actor::options::close(world);
        }
        Dismissal::Rebind => {
            skill::bar::cancel_rebind(world);
        }
        Dismissal::Aim => {
            skill::aim::disarm(world);
        }
        Dismissal::Sheet(sheet) => close(world, sheet),
        Dismissal::Crafting => {
            crafting::panel::cancel(world);
        }
        Dismissal::Conversation => {
            stage::leave(world);
        }
        Dismissal::Work => {
            resource::render::stop(world);
        }
        Dismissal::OpenMenu => menu::open(world),
    }
}

/// The sheets stand in a stack, and a press steps out of the one in front: whichever the player
/// opened or reached for last. One that is already on its way out is passed over.
fn front_sheet(world: &World) -> Option<WindowKey> {
    hud::sheets_front_first(world)
        .into_iter()
        .find(|&sheet| match sheet {
            WindowKey::Menu => menu::is_open(world),
            WindowKey::ItemCard => item::card::showing(world),
            WindowKey::SkillCard => skill::card::showing(world),
            WindowKey::QuestCard => quest::card::showing(world),
            WindowKey::Trade => trade::window::is_open(world),
            WindowKey::Market => market::window::is_open(world),
            WindowKey::Roster => area_instance::gather::leads_call(world),
            WindowKey::Confirm
            | WindowKey::Stage
            | WindowKey::Shop
            | WindowKey::Crafting
            | WindowKey::Prompt
            | WindowKey::Entrance
            | WindowKey::QuestOffer => false,
        })
}

fn close(world: &mut World, sheet: WindowKey) {
    match sheet {
        WindowKey::Menu => {
            menu::close(world);
        }
        WindowKey::ItemCard => {
            item::card::close(world);
        }
        WindowKey::SkillCard => {
            skill::card::close(world);
        }
        WindowKey::QuestCard => {
            quest::card::close(world);
        }
        WindowKey::Trade => {
            trade::window::cancel(world);
        }
        WindowKey::Market => {
            market::window::close(world);
        }
        WindowKey::Roster => {
            area_instance::gather::call_off(world);
        }
        WindowKey::Confirm
        | WindowKey::Stage
        | WindowKey::Shop
        | WindowKey::Crafting
        | WindowKey::Prompt
        | WindowKey::Entrance
        | WindowKey::QuestOffer => {}
    }
}
