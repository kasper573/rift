use bevy_app::App;
use bevy_ecs::message::Message;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TerminalInput {
    pub text: String,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TerminalLine {
    pub text: String,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct TerminalSaid {
    pub conn: Entity,
    pub text: String,
}

/// Carries a line to the connection `conn` names: the app's transport, whatever it is.
pub type DeliverTerminalLine = fn(&mut World, Entity, TerminalLine);

pub fn register(app: &mut App, deliver: DeliverTerminalLine) {
    app.insert_resource(Outlet(deliver))
        .add_message::<TerminalSaid>();
}

pub fn broadcast(world: &mut World, conns: &[Entity], text: impl Into<String>) {
    let text = text.into();
    for &conn in conns {
        send(world, conn, &text);
    }
}

/// Takes in what the connection `conn` typed, for the app to pass on as it sees fit.
pub fn hear(world: &mut World, conn: Entity, input: &TerminalInput) {
    let text: String = input.text.trim().chars().take(MAX_INPUT_LEN).collect();
    if text.is_empty() {
        return;
    }
    world.write_message(TerminalSaid { conn, text });
}

const MAX_INPUT_LEN: usize = 256;

#[derive(Resource)]
struct Outlet(DeliverTerminalLine);

fn send(world: &mut World, conn: Entity, text: &str) {
    let deliver = world.resource::<Outlet>().0;
    for line in text.lines() {
        deliver(
            world,
            conn,
            TerminalLine {
                text: line.to_owned(),
            },
        );
    }
}
