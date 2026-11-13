//! How a command line being typed may go on: the commands its sender may send while the name is
//! typed, and once a command is named, what the input at hand takes.

use bevy_ecs::prelude::*;
use bevy_replication::ToClient;
use bevy_terminal::{TerminalCompletionRequest, TerminalCompletions, TerminalOption, best_matches};

use super::input::{self, TypedCommand};
use crate::core::content::Content;
use crate::systems::GameSettings;
use crate::systems::player;

/// Answers every connection that asked how the command it is typing may go on.
pub fn answer(world: &mut World) {
    for asked in crate::systems::drain_requests::<TerminalCompletionRequest>(world) {
        let message = completions(world, asked.client, &asked.message);
        world.write_message(ToClient {
            client: asked.client,
            message,
        });
    }
}

/// How what the connection `conn` asked about may go on: nothing, unless it types a command.
pub fn completions(
    world: &World,
    conn: Entity,
    asked: &TerminalCompletionRequest,
) -> TerminalCompletions {
    let none = TerminalCompletions {
        text: asked.text.clone(),
        ..TerminalCompletions::default()
    };
    let Some(line) = asked.command() else {
        return none;
    };
    let content = world.resource::<Content>();
    let most = content.built::<GameSettings>().suggestions as usize;
    let typed = TypedCommand::of(line);
    let commands = super::allowed(world, conn);
    let keeping = |typing: &str| (asked.text.chars().count() - typing.chars().count()) as u32;
    if typed.naming {
        let names = commands.iter().map(|command| command.name(content));
        let options = best_matches(typed.name, names, most)
            .into_iter()
            .map(|place| commands[place])
            .map(|command| {
                let takes_inputs = !command.get(content).inputs.is_empty();
                TerminalOption {
                    text: if takes_inputs {
                        format!("{} ", command.name(content))
                    } else {
                        command.name(content).to_owned()
                    },
                    detail: command.usage(content),
                }
            })
            .collect();
        return TerminalCompletions {
            keeps: keeping(typed.name),
            options,
            ..none
        };
    }
    let Some(command) = commands
        .into_iter()
        .find(|command| command.name(content) == typed.name)
    else {
        return none;
    };
    let inputs = input::inputs(content, command.get(content));
    let slots = input::slots(&inputs);
    let typing = typed.parts.last().copied().unwrap_or_default().trim_start();
    let usage = command.usage(content);
    let Some(slot) = slots.get(typed.parts.len().saturating_sub(1)) else {
        return TerminalCompletions { usage, ..none };
    };
    let options = player::conn_player(world, conn)
        .map(|caller| input::options(world, caller, slot.input.kind, slot.part))
        .unwrap_or_default();
    TerminalCompletions {
        keeps: keeping(typing),
        options: best_matches(
            typing,
            options.iter().map(|option| option.text.as_str()),
            most,
        )
        .into_iter()
        .map(|place| TerminalOption {
            text: options[place].text.clone(),
            detail: options[place].detail.clone(),
        })
        .collect(),
        usage: format!("{}: {usage}", slot.name()),
        ..none
    }
}
