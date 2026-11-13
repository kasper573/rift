pub mod chat;

use std::collections::VecDeque;

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use bevy_terminal::TerminalLine;

use crate::core::gui::Band;
use crate::systems::screen::ClientScreen;

pub struct TerminalPlugin;

impl Plugin for TerminalPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TerminalLog>()
            .init_resource::<chat::ChatView>()
            .add_systems(Update, (receive, chat::suggested))
            .add_systems(Update, chat::keys.run_if(in_state(ClientScreen::Area)))
            .add_systems(
                EguiPrimaryContextPass,
                chat::draw
                    .in_set(Band::Hud)
                    .run_if(in_state(ClientScreen::Area)),
            );
    }
}

#[derive(Resource, Default)]
pub struct TerminalLog {
    lines: VecDeque<TerminalLine>,
    arrived: u64,
}

impl TerminalLog {
    pub fn lines(&self) -> impl DoubleEndedIterator<Item = &TerminalLine> {
        self.lines.iter()
    }

    /// How many lines have arrived since the log began, those it no longer keeps among them.
    pub fn arrived(&self) -> u64 {
        self.arrived
    }
}

const MAX_LINES: usize = 200;

fn receive(mut incoming: MessageReader<TerminalLine>, mut log: ResMut<TerminalLog>) {
    for line in incoming.read() {
        log.lines.push_back(line.clone());
        log.arrived += 1;
        if log.lines.len() > MAX_LINES {
            log.lines.pop_front();
        }
    }
}
