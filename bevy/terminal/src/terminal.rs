use bevy_app::App;
use bevy_ecs::message::Message;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

/// What a line starts with to be a command rather than something said.
pub const COMMAND_PREFIX: char = '/';

/// One of the channels an app's terminal has: what a line is said on, and what it is typed on.
/// Which there are, and who is in each, is the app's to say.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TerminalChannel(pub u8);

/// Where a line's text holds something of the app's own rather than a letter: the character
/// Unicode keeps for an object set in text.
pub const EMBED: char = '\u{FFFC}';

/// Something of the app's own set in a line's text: which kind of thing, and which of the kind.
/// What kinds there are, and how each is shown, is the app's to say.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TerminalEmbed {
    pub kind: String,
    pub key: String,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TerminalInput {
    pub text: String,
    pub channel: TerminalChannel,
    /// What the text holds at each [`EMBED`] of it, in their order.
    pub embeds: Vec<TerminalEmbed>,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TerminalLine {
    pub text: String,
    pub kind: LineKind,
    pub channel: TerminalChannel,
    /// What the text holds at each [`EMBED`] of it, in their order.
    pub embeds: Vec<TerminalEmbed>,
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
    pub channel: TerminalChannel,
    pub text: String,
    pub embeds: Vec<TerminalEmbed>,
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

pub fn say(
    world: &mut World,
    conns: &[Entity],
    channel: TerminalChannel,
    speaker: LineSpeaker,
    text: impl Into<String>,
    embeds: &[TerminalEmbed],
) {
    let (text, kind) = (text.into(), LineKind::Said(speaker));
    for &conn in conns {
        send(world, conn, channel, &text, embeds, &kind);
    }
}

/// Tells the connection `conn` something on `channel`, in the app's own voice.
pub fn note(world: &mut World, conn: Entity, channel: TerminalChannel, text: impl Into<String>) {
    send(world, conn, channel, &text.into(), &[], &LineKind::Notice);
}

/// Answers what the connection `conn` last typed, on the channel it typed it on.
pub fn reply(world: &mut World, conn: Entity, text: impl Into<String>) {
    let channel = typed_on(world, conn);
    send(world, conn, channel, &text.into(), &[], &LineKind::Notice);
}

pub fn reply_failure(world: &mut World, conn: Entity, text: impl Into<String>) {
    let channel = typed_on(world, conn);
    send(world, conn, channel, &text.into(), &[], &LineKind::Failed);
}

/// Takes in what the connection `conn` typed, for the app to pass on as it sees fit: a command
/// to run, or anything else as said. A command holds nothing but its text, and what is said
/// holds something at every [`EMBED`] of it or is not taken at all.
pub fn hear(world: &mut World, conn: Entity, input: &TerminalInput) {
    let text = typed(&input.text);
    if text.is_empty() {
        return;
    }
    let held = text.chars().filter(|letter| *letter == EMBED).count();
    if held != input.embeds.len() {
        return;
    }
    let channel = input.channel;
    if let Ok(mut conn) = world.get_entity_mut(conn) {
        conn.insert(TypedOn(channel));
    }
    match command_line(&text) {
        Some(line) => {
            let line = line.to_owned();
            world.write_message(TerminalCommanded { conn, line });
        }
        None => {
            world.write_message(TerminalSaid {
                conn,
                channel,
                text,
                embeds: input.embeds.clone(),
            });
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

/// The channel a connection last typed on.
#[derive(Component)]
struct TypedOn(TerminalChannel);

fn typed_on(world: &World, conn: Entity) -> TerminalChannel {
    world
        .get::<TypedOn>(conn)
        .map(|typed| typed.0)
        .unwrap_or_default()
}

fn send(
    world: &mut World,
    conn: Entity,
    channel: TerminalChannel,
    text: &str,
    embeds: &[TerminalEmbed],
    kind: &LineKind,
) {
    let deliver = world.resource::<Outlet>().0;
    let mut embeds = embeds.iter();
    for line in text.lines() {
        let held = line.chars().filter(|letter| *letter == EMBED).count();
        deliver(
            world,
            conn,
            TerminalLine {
                text: line.to_owned(),
                kind: kind.clone(),
                channel,
                embeds: embeds.by_ref().take(held).cloned().collect(),
            },
        );
    }
}
