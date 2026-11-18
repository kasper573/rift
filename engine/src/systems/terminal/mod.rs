pub mod chat;
pub mod draft;

use std::collections::{HashMap, VecDeque};

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use bevy_terminal::{TerminalChannel, TerminalLine};

use crate::core::content::Content;
use crate::core::gui::Band;
use crate::systems::GameSettings;
use crate::systems::actor::portrait::ActorPortraits;
use crate::systems::chat::ChatSpeaker;
use crate::systems::messages::messages;
use crate::systems::player::ClientId;
use crate::systems::screen::ClientScreen;

pub struct TerminalPlugin;

impl Plugin for TerminalPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TerminalLog>()
            .init_resource::<ChatSpeakers>()
            .init_resource::<chat::ChatView>()
            .add_systems(
                Update,
                (
                    receive.run_if(resource_exists::<Content>),
                    meet,
                    chat::suggested,
                    chat::channels,
                )
                    .chain(),
            )
            .add_systems(Update, chat::keys.run_if(in_state(ClientScreen::Area)))
            .add_systems(
                EguiPrimaryContextPass,
                chat::draw
                    .in_set(Band::Hud)
                    .run_if(in_state(ClientScreen::Area)),
            );
    }
}

/// The latest lines of every channel, in the order they arrived.
#[derive(Resource, Default)]
pub struct TerminalLog {
    lines: VecDeque<(LoggedLineId, TerminalLine)>,
    arrived: HashMap<TerminalChannel, u64>,
    /// How many lines the log has taken in since it began.
    taken: u64,
}

/// A line the log took in, told from every other it ever did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LoggedLineId(u64);

impl TerminalLog {
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

    /// Lets go of everything said on `channel`.
    pub fn forget(&mut self, channel: TerminalChannel) {
        self.lines.retain(|(_, line)| line.channel != channel);
        self.arrived.remove(&channel);
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

fn meet(
    mut incoming: MessageReader<ChatSpeaker>,
    mut speakers: ResMut<ChatSpeakers>,
    mut portraits: ResMut<ActorPortraits>,
) {
    for speaker in incoming.read() {
        portraits.ask(&speaker.appearance);
        speakers.0.insert(speaker.typist, speaker.clone());
    }
}

fn receive(
    mut incoming: MessageReader<TerminalLine>,
    mut log: ResMut<TerminalLog>,
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

messages! {
    /// What the game says in its terminal.
    pub struct TerminalMessages {
        /// What a card's offer to set what it shows in chat is called.
        add_to_chat,
        /// How a screen reader names the tab of the channel `{channel}` in chat.
        chat_tab(channel: Text),
        /// How a screen reader names the bar that moves chat.
        move_chat,
        /// How a screen reader names the corner that resizes chat.
        resize_chat,
        /// What the button that scrolls chat to its newest lines says of the `{count}` lines that arrived unseen.
        unread(count: Number),
        /// Who said a line, `{name}`, before what they said.
        said_by{name},
        /// Who said a line, `{name}`, and the channel `{channel}` they said it on, before what they
        /// said, where chat shows every channel together.
        said_on_by{channel, name},
        /// The channel `{channel}` a player types on, before the field they type in.
        typing_on(channel: Text),
        /// What a line reads where chat covers a thing it holds.
        covered,
        /// What a covered thing in a chat line says of itself, as a tooltip and to a screen reader.
        covered_hint,
        /// The hint, beside its key, that picks the thing looked for to set in the message.
        hint_pick,
        /// The hint, beside its key, that goes back from looking for a thing.
        hint_back,
        /// The hint, beside its key, that sends the line typed.
        hint_send,
        /// The hint, beside its key, that turns to the next channel's tab.
        hint_next_tab,
        /// The hint, beside its key, that stops typing a line.
        hint_cancel,
        /// The hint, beside its key, that starts typing in chat.
        hint_to_chat,
        /// What chat says where no key starts typing in it.
        click_to_chat,
        /// The heading over the things of a kind to set in the message: `{kind}` says what a thing of the kind is.
        show_kind(kind: Text),
        /// The heading over the kinds of thing to set in the message.
        show_in_message,
    }
}
