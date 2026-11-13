use bevy::prelude::*;

use super::{ResetAnswerRequest, ResetPrompt};
use crate::systems::hud::{self, Confirm};
use crate::systems::player::session;

/// Where the question on the character stands: put to the player, answered, or both until the
/// server takes it away.
#[derive(Resource, Default)]
pub struct PromptState {
    asked: bool,
    answered: bool,
}

pub fn showing(world: &World) -> bool {
    prompt(world).is_some() && !world.resource::<PromptState>().answered
}

pub fn forget(mut state: ResMut<PromptState>) {
    *state = PromptState::default();
}

/// Puts the server's question to the player through the interface's confirm, once, as soon as no
/// other question is up; dismissing it keeps the skills.
pub fn ask(world: &mut World) {
    let Some(asked) = prompt(world) else {
        *world.resource_mut::<PromptState>() = PromptState::default();
        return;
    };
    if world.resource::<PromptState>().asked || hud::confirming(world) {
        return;
    }
    world.resource_mut::<PromptState>().asked = true;
    hud::confirm(
        world,
        Confirm {
            title: "Reset your skills?".to_owned(),
            body: vec![
                asked.describe(),
                "Bar slots that held them stay, dimmed, until they are learned again.".to_owned(),
            ],
            confirm: "Reset".to_owned(),
            cancel: "Keep".to_owned(),
            answer: Box::new(|world, accepted| {
                world.resource_mut::<PromptState>().answered = true;
                world.write_message(ResetAnswerRequest { accepted });
            }),
        },
    );
}

fn prompt(world: &World) -> Option<ResetPrompt> {
    session::my_character(world).and_then(|me| me.get::<ResetPrompt>().copied())
}
