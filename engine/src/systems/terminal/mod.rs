pub mod chat;

use std::collections::{HashMap, VecDeque};

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use bevy_terminal::{TerminalChannel, TerminalLine};

use crate::core::gui::Band;
use crate::systems::screen::ClientScreen;

pub struct TerminalPlugin;

impl Plugin for TerminalPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TerminalLog>()
            .init_resource::<chat::ChatView>()
            .add_systems(Update, (receive, chat::suggested, chat::channels).chain())
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
    lines: VecDeque<TerminalLine>,
    arrived: HashMap<TerminalChannel, u64>,
}

impl TerminalLog {
    pub fn lines(&self) -> impl DoubleEndedIterator<Item = &TerminalLine> {
        self.lines.iter()
    }

    pub fn on(&self, channel: TerminalChannel) -> impl DoubleEndedIterator<Item = &TerminalLine> {
        self.lines
            .iter()
            .filter(move |line| line.channel == channel)
    }

    /// How many lines have arrived on `channel` since it began, those the log no longer keeps
    /// among them.
    pub fn arrived(&self, channel: TerminalChannel) -> u64 {
        self.arrived.get(&channel).copied().unwrap_or(0)
    }

    /// Lets go of everything said on `channel`.
    pub fn forget(&mut self, channel: TerminalChannel) {
        self.lines.retain(|line| line.channel != channel);
        self.arrived.remove(&channel);
    }
}

/// How many lines the log keeps of each channel.
const MAX_LINES: usize = 200;

fn receive(mut incoming: MessageReader<TerminalLine>, mut log: ResMut<TerminalLog>) {
    for line in incoming.read() {
        let channel = line.channel;
        log.lines.push_back(line.clone());
        *log.arrived.entry(channel).or_default() += 1;
        if log.on(channel).count() > MAX_LINES
            && let Some(oldest) = log.lines.iter().position(|line| line.channel == channel)
        {
            log.lines.remove(oldest);
        }
    }
}
