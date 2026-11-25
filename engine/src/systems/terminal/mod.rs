pub mod chat;
pub mod draft;
pub mod history;

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;

use crate::core::gui::Band;
use crate::systems::actor::portrait::ActorPortraits;
use crate::systems::chat::ChatSpeaker;
use crate::systems::chat::log::ChatLogSystems;
use crate::systems::messages::messages;
use crate::systems::screen::ClientScreen;

pub struct TerminalPlugin;

impl Plugin for TerminalPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<chat::ChatView>()
            .add_systems(
                Update,
                (ask_portraits, chat::suggested, chat::channels)
                    .chain()
                    .after(ChatLogSystems),
            )
            .add_systems(Update, chat::keys.run_if(in_state(ClientScreen::Area)))
            .add_systems(OnExit(ClientScreen::Area), chat::forget)
            .add_systems(
                EguiPrimaryContextPass,
                chat::draw
                    .in_set(Band::Hud)
                    .run_if(in_state(ClientScreen::Area)),
            );
    }
}

fn ask_portraits(mut incoming: MessageReader<ChatSpeaker>, mut portraits: ResMut<ActorPortraits>) {
    for speaker in incoming.read() {
        portraits.ask(&speaker.appearance);
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
        /// The hint, beside its key, that brings back the line sent before the one in the field.
        recall_earlier,
        /// The hint, beside its key, that brings back the line sent after the one in the field.
        recall_later,
        /// Where among the lines sent the field is, line `{at}` back of `{of}`.
        recalled(at: Number, of: Number),
        /// The hint, beside its key, that gives back the line the player was typing in place of
        /// one brought back.
        back_to_draft,
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
