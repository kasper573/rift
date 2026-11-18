//! Naming a file by typing part of its name and picking it from those that match, as chat names
//! a thing it shows in a line and the market names the item a buyer looks for.

use bevy_ecs::prelude::*;
use bevy_terminal::best_matches;
use ui::egui::{KeyboardShortcut, Ui};

use super::input::{self, InputKind, InputOption, SlotPart};
use crate::core::content::Content;
use crate::systems::GameSettings;
use crate::systems::input::map::{InputAction, InputMap};
use crate::systems::player::session;

/// The files of `table` whose names best match `typed`, the best first, as many as the game
/// suggests at once.
pub fn matching(world: &World, table: &str, typed: &str) -> Vec<InputOption> {
    let most = world
        .resource::<Content>()
        .built::<GameSettings>()
        .suggestions as usize;
    let caller = session::me(world).unwrap_or(Entity::PLACEHOLDER);
    let options = input::options(world, caller, InputKind::Row(table), SlotPart::Whole);
    best_matches(
        typed,
        options.iter().map(|option| option.text.as_str()),
        most,
    )
    .into_iter()
    .map(|place| options[place].clone())
    .collect()
}

/// Lights another of `options` suggestions with the keys that choose between them, and says
/// which the key that completes text picked, or the one that submits it where `submit_picks`.
/// The keys are taken from the field even while nothing is suggested.
pub fn keyed(
    ui: &mut Ui,
    keys: &InputMap,
    options: usize,
    selected: &mut usize,
    submit_picks: bool,
) -> Option<usize> {
    let pressed = |ui: &mut Ui, action: InputAction| {
        keys.shortcut(action)
            .is_some_and(|shortcut: KeyboardShortcut| {
                ui.input_mut(|input| input.consume_shortcut(&shortcut))
            })
    };
    let picked = pressed(ui, InputAction::CompleteText)
        || (submit_picks && pressed(ui, InputAction::SubmitText));
    if options == 0 {
        return None;
    }
    *selected = (*selected).min(options - 1);
    if pressed(ui, InputAction::ChoosePrevious) {
        *selected = (*selected + options - 1) % options;
    }
    if pressed(ui, InputAction::ChooseNext) {
        *selected = (*selected + 1) % options;
    }
    picked.then_some(*selected)
}
