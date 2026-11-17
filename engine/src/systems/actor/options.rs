//! What a player can do with another player, offered where they click them: trading, a party,
//! friendship, a message, and keeping them away.

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::{self, Pos2};
use ui::face;

use crate::core::gui::{self, Band};
use crate::core::render;
use crate::core::units::MeterPos;
use crate::systems::actor::{self, Name};
use crate::systems::friend::AskFriendship;
use crate::systems::inbox::draft::{self, WriteToCharacter};
use crate::systems::input::map::{self, InputAction};
use crate::systems::party::page::InviteToParty;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::social_request::card;
use crate::systems::society::tab::CharacterAction;
use crate::systems::society::view::CharacterAccountNumber;
use crate::systems::society::{SocietyMember, SocietyView};
use crate::systems::trade::TradeRequest;
use crate::systems::{GameSettings, class, movement};

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<ActorOptions>()
        .add_systems(
            EguiPrimaryContextPass,
            (open, show)
                .chain()
                .in_set(Band::System)
                .run_if(in_state(ClientScreen::Area)),
        )
        .add_systems(OnExit(ClientScreen::Area), forget);
}

/// Closes the options, if they are open: what stepping out does first while they are.
pub fn close(world: &mut World) -> bool {
    let open = world.resource::<ActorOptions>().open.is_some();
    world.resource_mut::<ActorOptions>().open = None;
    open
}

/// The options the player has open, for a driver of the client.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownActorOptions {
    /// The name of the player they are for.
    pub name: String,
    pub options: Vec<String>,
}

pub fn shown(world: &World) -> Option<ShownActorOptions> {
    let open = world.resource::<ActorOptions>().open?;
    let name = world.get::<Name>(open.player)?.name.clone();
    let options = offered(world, open.player)?
        .iter()
        .map(|option| option.label(world, open.player))
        .collect();
    Some(ShownActorOptions { name, options })
}

#[derive(Resource, Default)]
struct ActorOptions {
    /// The player pressed on, and where the pointer was as the press began.
    pressed: Option<(Entity, Pos2)>,
    open: Option<OpenOptions>,
}

#[derive(Clone, Copy)]
struct OpenOptions {
    player: Entity,
    at: Pos2,
}

/// The options a player is offered of another, in the order they are listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActorOption {
    Trade,
    Invite,
    Befriend,
    Write,
    Block,
}

impl ActorOption {
    fn label(self, world: &World, player: Entity) -> String {
        match self {
            ActorOption::Trade => {
                let reach = world
                    .resource::<GameSettings>()
                    .trade
                    .map(|settings| settings.reach);
                let me = session::my_character(world).map(|me| me.id());
                let far = match (reach, me) {
                    (Some(reach), Some(me)) => {
                        match (
                            movement::position(world, me),
                            movement::position(world, player),
                        ) {
                            (Some(mine), Some(theirs)) => mine.distance(theirs) > reach,
                            _ => false,
                        }
                    }
                    _ => false,
                };
                match reach.filter(|_| far) {
                    Some(reach) => format!("Trade · walks within {} m first", reach.0.round()),
                    None => "Trade".to_owned(),
                }
            }
            ActorOption::Invite => InviteToParty.label().to_owned(),
            ActorOption::Befriend => "Add friend".to_owned(),
            ActorOption::Write => WriteToCharacter.label().to_owned(),
            ActorOption::Block => "Block".to_owned(),
        }
    }

    fn act(self, world: &mut World, player: Entity, member: SocietyMember) {
        match self {
            ActorOption::Trade => {
                world.write_message(TradeRequest::Ask { with: player });
            }
            ActorOption::Invite => InviteToParty.act(world, &member),
            ActorOption::Befriend => {
                world.write_message(AskFriendship(member.account));
            }
            ActorOption::Write => WriteToCharacter.act(world, &member),
            ActorOption::Block => card::block(world, member),
        }
    }
}

fn forget(mut options: ResMut<ActorOptions>) {
    *options = ActorOptions::default();
}

/// Opens the options of the player the pointer is on as the binding is clicked: pressed and let
/// go without the pointer moving far, so a drag that turns the camera opens nothing.
fn open(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let pointer = gui.input(|input| input.pointer.latest_pos());
    if map::just_pressed(world, InputAction::ActorOptions) && !gui::pointer_claimed(world) {
        let pressed = pointed_player(world).zip(pointer);
        world.resource_mut::<ActorOptions>().pressed = pressed;
        return;
    }
    if map::pressed(world, InputAction::ActorOptions) {
        return;
    }
    let Some((player, from)) = world.resource_mut::<ActorOptions>().pressed.take() else {
        return;
    };
    let slop = gui.options(|options| options.input_options.max_click_dist);
    let still = pointer.is_some_and(|at| at.distance(from) <= slop);
    if still && offered(world, player).is_some() {
        world.resource_mut::<ActorOptions>().open = Some(OpenOptions { player, at: from });
    }
}

fn show(world: &mut World) {
    let Some(open) = world.resource::<ActorOptions>().open else {
        return;
    };
    let (Some(gui), Some(options), Some(member)) = (
        gui::context(world),
        offered(world, open.player),
        member(world, open.player),
    ) else {
        world.resource_mut::<ActorOptions>().open = None;
        return;
    };
    let level = class::level(world, open.player);
    let mut picked = None;
    let shown = egui::Area::new(egui::Id::new("actor options"))
        .order(Band::System.order())
        .fixed_pos(open.at)
        .constrain(true)
        .show(&gui, |ui| {
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                ui.label(face::bold(format!("{} · level {level}", member.name)));
                for option in options {
                    if ui.button(option.label(world, open.player)).clicked() {
                        picked = Some(option);
                    }
                }
            });
        });
    let pressed_elsewhere = gui.input(|input| {
        input.pointer.any_pressed()
            && input
                .pointer
                .interact_pos()
                .is_some_and(|at| !shown.response.rect.contains(at))
    });
    if picked.is_some() || pressed_elsewhere {
        world.resource_mut::<ActorOptions>().open = None;
    }
    if let Some(option) = picked {
        option.act(world, open.player, member);
    }
}

/// The other player the pointer is on.
fn pointed_player(world: &mut World) -> Option<Entity> {
    let pointed = render::pointed(world)?;
    let me = session::my_character(world).map(|me| me.id());
    actor::target_at(world, &pointed, me, |world, entity| {
        world.get::<CharacterAccountNumber>(entity).is_some()
    })
}

/// The player `player` is, as the Social pages list players.
fn member(world: &World, player: Entity) -> Option<SocietyMember> {
    Some(SocietyMember {
        account: world.get::<CharacterAccountNumber>(player)?.0,
        name: world.get::<Name>(player)?.name.clone(),
    })
}

/// What the player may do with `player`, by the systems the game has on and how the two stand:
/// none while `player` is no other player.
fn offered(world: &World, player: Entity) -> Option<Vec<ActorOption>> {
    let account = world.get::<CharacterAccountNumber>(player)?.0;
    let me = session::my_character(world)?;
    if me.id() == player {
        return None;
    }
    let view = me.get::<SocietyView>()?;
    let settings = world.resource::<GameSettings>();
    let befriended = view
        .friends
        .iter()
        .any(|friend| friend.character.account == account);
    let blocked = view.blocked.iter().any(|kept| kept.account == account);
    let options = [
        (ActorOption::Trade, settings.trade.is_some()),
        (ActorOption::Invite, InviteToParty.on(world)),
        (
            ActorOption::Befriend,
            settings.friends.is_some() && !befriended,
        ),
        (
            ActorOption::Write,
            WriteToCharacter.on(world) && draft::may_write_to(view, account),
        ),
        (ActorOption::Block, !blocked),
    ];
    Some(
        options
            .into_iter()
            .filter(|(_, on)| *on)
            .map(|(option, _)| option)
            .collect(),
    )
}
