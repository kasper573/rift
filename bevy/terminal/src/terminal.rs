use bevy_app::App;
use bevy_ecs::message::Message;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::command::{self, TerminalCommand, TerminalCommands};

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TerminalInput {
    pub text: String,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TerminalLine {
    pub text: String,
    pub tone: LineTone,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineTone {
    Said,
    Failed,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct TerminalSaid {
    pub conn: Entity,
    pub text: String,
}

/// Carries a line to the connection `conn` names: the app's transport, whatever it is.
pub type DeliverTerminalLine = fn(&mut World, Entity, TerminalLine);

pub fn register(app: &mut App, commands: &[TerminalCommand], deliver: DeliverTerminalLine) {
    app.insert_resource(TerminalCommands::new(commands))
        .insert_resource(Outlet(deliver))
        .add_message::<TerminalSaid>();
}

pub fn broadcast(world: &mut World, conns: &[Entity], text: impl Into<String>) {
    let text = text.into();
    for &conn in conns {
        send(world, conn, &text, LineTone::Said);
    }
}

pub fn reply(world: &mut World, conn: Entity, text: impl Into<String>) {
    send(world, conn, &text.into(), LineTone::Said);
}

pub fn reply_failure(world: &mut World, conn: Entity, text: impl Into<String>) {
    send(world, conn, &text.into(), LineTone::Failed);
}

/// Takes in what the connection `conn` typed: a command runs and replies to it, and anything
/// else is said, for the app to pass on as it sees fit.
pub fn hear(world: &mut World, conn: Entity, input: &TerminalInput) {
    let text: String = input.text.trim().chars().take(MAX_INPUT_LEN).collect();
    if text.is_empty() {
        return;
    }
    if let Some(line) = text.strip_prefix('/') {
        command::run(world, conn, line);
    } else {
        world.write_message(TerminalSaid { conn, text });
    }
}

const MAX_INPUT_LEN: usize = 256;

#[derive(Resource)]
struct Outlet(DeliverTerminalLine);

fn send(world: &mut World, conn: Entity, text: &str, tone: LineTone) {
    let deliver = world.resource::<Outlet>().0;
    for line in text.lines() {
        deliver(
            world,
            conn,
            TerminalLine {
                text: line.to_owned(),
                tone,
            },
        );
    }
}
