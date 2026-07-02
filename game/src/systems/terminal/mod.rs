pub mod widget;

use std::collections::{HashMap, VecDeque};

use crate::data::terminal::Id;
use bevy::prelude::*;
use bevy_terminal::{AvailableTerminals, TerminalLine};

use crate::systems::hud::{RefreshWindows, TERMINAL_WINDOW};

pub struct TerminalPlugin;

impl Plugin for TerminalPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Terminals>()
            .add_systems(Update, receive);
    }
}

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
    mut available: MessageReader<AvailableTerminals<Id>>,
    mut lines: MessageReader<TerminalLine<Id>>,
    mut terminals: ResMut<Terminals>,
    mut refresh: ResMut<RefreshWindows>,
) {
    for message in available.read() {
        if terminals.tabs != message.terminals {
            terminals.tabs = message.terminals.clone();
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
