//! What a client has heard said: the latest lines of every channel it is in, and who said them.

use std::collections::{HashMap, VecDeque};

use bevy_app::{App, Plugin, Update};
use bevy_ecs::prelude::*;
use bevy_terminal::{TerminalChannel, TerminalLine};

use super::{ChatChannel, ChatChannels, ChatSpeaker};
use crate::core::content::Content;
use crate::systems::GameSettings;
use crate::systems::player::{ClientId, session};

/// Keeps a client's [`ChatLog`] and [`ChatSpeakers`] as the server tells it of them.
pub struct ChatLogPlugin;

impl Plugin for ChatLogPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ChatLog>()
            .init_resource::<ChatSpeakers>()
            .add_systems(
                Update,
                (
                    hear.run_if(resource_exists::<Content>),
                    meet,
                    follow_channels,
                )
                    .chain()
                    .in_set(ChatLogSystems),
            );
    }
}

/// Where the log takes in what the server sent, for what reads it the same frame.
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub struct ChatLogSystems;

/// The latest lines of every channel, in the order they arrived.
#[derive(Resource, Default)]
pub struct ChatLog {
    lines: VecDeque<(LoggedLineId, TerminalLine)>,
    arrived: HashMap<TerminalChannel, u64>,
    /// How many lines the log has taken in since it began.
    taken: u64,
    /// The channels the player was last told they are in.
    channels: Vec<ChatChannel>,
}

/// A line the log took in, told from every other it ever did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LoggedLineId(u64);

impl ChatLog {
    pub fn lines(&self) -> impl DoubleEndedIterator<Item = &TerminalLine> {
        self.lines.iter().map(|(_, line)| line)
    }

    /// The lines the log keeps, each with what tells it from the others.
    pub fn numbered(&self) -> impl DoubleEndedIterator<Item = (LoggedLineId, &TerminalLine)> {
        self.lines.iter().map(|(id, line)| (*id, line))
    }

    pub fn on(&self, channel: TerminalChannel) -> impl DoubleEndedIterator<Item = &TerminalLine> {
        self.lines().filter(move |line| line.channel == channel)
    }

    /// How many lines have arrived on `channel` since it began, those the log no longer keeps
    /// among them.
    pub fn arrived(&self, channel: TerminalChannel) -> u64 {
        self.arrived.get(&channel).copied().unwrap_or(0)
    }
}

/// Everyone whose lines this client has heard, as it was last told of them.
#[derive(Resource, Default)]
pub struct ChatSpeakers(HashMap<ClientId, ChatSpeaker>);

impl ChatSpeakers {
    pub fn of(&self, typist: ClientId) -> Option<&ChatSpeaker> {
        self.0.get(&typist)
    }
}

fn meet(mut incoming: MessageReader<ChatSpeaker>, mut speakers: ResMut<ChatSpeakers>) {
    for speaker in incoming.read() {
        speakers.0.insert(speaker.typist, speaker.clone());
    }
}

fn hear(
    mut incoming: MessageReader<TerminalLine>,
    mut log: ResMut<ChatLog>,
    content: Res<Content>,
) {
    let kept = content.built::<GameSettings>().chat.kept_lines as usize;
    for line in incoming.read() {
        let channel = line.channel;
        let id = LoggedLineId(log.taken);
        log.taken += 1;
        log.lines.push_back((id, line.clone()));
        *log.arrived.entry(channel).or_default() += 1;
        let oldest = log.lines().position(|line| line.channel == channel);
        if log.on(channel).count() > kept
            && let Some(oldest) = oldest
        {
            log.lines.remove(oldest);
        }
    }
}

/// A channel the player is no longer in takes its lines with it.
fn follow_channels(world: &mut World) {
    let Some(told) = session::my_character(world)
        .and_then(|me| me.get::<ChatChannels>().cloned())
        .filter(|told| told.0 != world.resource::<ChatLog>().channels)
    else {
        return;
    };
    let mut log = world.resource_mut::<ChatLog>();
    let gone: Vec<TerminalChannel> = log
        .channels
        .iter()
        .filter(|channel| !told.0.contains(channel))
        .map(|channel| channel.terminal())
        .collect();
    log.lines.retain(|(_, line)| !gone.contains(&line.channel));
    log.arrived.retain(|channel, _| !gone.contains(channel));
    log.channels = told.0;
}
