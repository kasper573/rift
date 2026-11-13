use bevy_app::App;
use bevy_ecs::message::Message;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

/// What a line starts with to be a command rather than something said.
pub const COMMAND_PREFIX: char = '/';

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TerminalInput {
    pub text: String,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TerminalLine {
    pub text: String,
    pub kind: LineKind,
}

/// What a line is, by which a terminal tells its lines apart without reading their text.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum LineKind {
    Said(LineSpeaker),
    Notice,
    Failed,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LineSpeaker {
    pub name: String,
    /// The app's number for whoever typed the line at a terminal, by which a terminal knows its
    /// own lines; none for a voice of the app's own.
    pub typist: Option<u64>,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct TerminalSaid {
    pub conn: Entity,
    pub text: String,
}

/// A command the connection `conn` sent: its line without the prefix, for the app to run.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct TerminalCommanded {
    pub conn: Entity,
    pub line: String,
}

/// Carries a line to the connection `conn` names: the app's transport, whatever it is.
pub type DeliverTerminalLine = fn(&mut World, Entity, TerminalLine);

pub fn register(app: &mut App, deliver: DeliverTerminalLine) {
    app.add_message::<TerminalSaid>()
        .add_message::<TerminalCommanded>();
    route(app.world_mut(), deliver);
}

/// Sends the lines of the terminal of `world` out through `deliver`.
pub fn route(world: &mut World, deliver: DeliverTerminalLine) {
    world.insert_resource(Outlet(deliver));
}

pub fn say(world: &mut World, conns: &[Entity], speaker: LineSpeaker, text: impl Into<String>) {
    let (text, kind) = (text.into(), LineKind::Said(speaker));
    for &conn in conns {
        send(world, conn, &text, &kind);
    }
}

pub fn reply(world: &mut World, conn: Entity, text: impl Into<String>) {
    send(world, conn, &text.into(), &LineKind::Notice);
}

pub fn reply_failure(world: &mut World, conn: Entity, text: impl Into<String>) {
    send(world, conn, &text.into(), &LineKind::Failed);
}

/// Takes in what the connection `conn` typed, for the app to pass on as it sees fit: a command
/// to run, or anything else as said.
pub fn hear(world: &mut World, conn: Entity, input: &TerminalInput) {
    let text = typed(&input.text);
    if text.is_empty() {
        return;
    }
    match command_line(&text) {
        Some(line) => {
            let line = line.to_owned();
            world.write_message(TerminalCommanded { conn, line });
        }
        None => {
            world.write_message(TerminalSaid { conn, text });
        }
    }
}

/// The command `text` holds, if it is one: what follows the prefix.
pub fn command_line(text: &str) -> Option<&str> {
    text.strip_prefix(COMMAND_PREFIX)
}

/// What a terminal takes of `text`: trimmed, and no longer than a line may be.
fn typed(text: &str) -> String {
    text.trim().chars().take(MAX_INPUT_LEN).collect()
}

pub const MAX_INPUT_LEN: usize = 256;

#[derive(Resource)]
struct Outlet(DeliverTerminalLine);

fn send(world: &mut World, conn: Entity, text: &str, kind: &LineKind) {
    let deliver = world.resource::<Outlet>().0;
    for line in text.lines() {
        deliver(
            world,
            conn,
            TerminalLine {
                text: line.to_owned(),
                kind: kind.clone(),
            },
        );
    }
}
