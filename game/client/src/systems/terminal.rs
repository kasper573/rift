use std::collections::{HashMap, VecDeque};

use bevy::prelude::*;
use world::data::terminal::Id;
use world::systems::terminal::{TerminalLine, TerminalTabs};

use crate::systems::widget::{RefreshWindows, TERMINAL_WINDOW};

pub struct TerminalPlugin;

impl Plugin for TerminalPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Terminals>()
            .add_systems(Update, receive);
    }
}

/// Everything the server has told this client about its terminals. Lines live in memory only —
/// restarting the game clears them — and survive area transfers (which re-issue the tab set).
#[derive(Resource, Default)]
pub struct Terminals {
    pub tabs: Vec<Id>,
    lines: HashMap<Id, VecDeque<(u64, String)>>,
    next_seq: u64,
}

impl Terminals {
    pub fn lines(&self, terminal: Id) -> impl Iterator<Item = &(u64, String)> {
        self.lines.get(&terminal).into_iter().flatten()
    }
}

const MAX_LINES: usize = 200;

fn receive(
    mut tabs: MessageReader<TerminalTabs>,
    mut lines: MessageReader<TerminalLine>,
    mut terminals: ResMut<Terminals>,
    mut refresh: ResMut<RefreshWindows>,
) {
    for message in tabs.read() {
        if terminals.tabs != message.tabs {
            terminals.tabs = message.tabs.clone();
            refresh.0.insert(TERMINAL_WINDOW);
        }
    }
    for line in lines.read() {
        let seq = terminals.next_seq;
        terminals.next_seq += 1;
        let log = terminals.lines.entry(line.terminal).or_default();
        log.push_back((seq, line.text.clone()));
        if log.len() > MAX_LINES {
            log.pop_front();
        }
    }
}
