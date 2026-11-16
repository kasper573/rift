mod complete;
mod terminal;

pub use complete::{TerminalCompletionRequest, TerminalCompletions, TerminalOption, best_matches};
pub use terminal::{
    COMMAND_PREFIX, DeliverTerminalLine, EMBED, LineKind, LineSpeaker, MAX_INPUT_LEN,
    TerminalChannel, TerminalCommanded, TerminalEmbed, TerminalInput, TerminalLine, TerminalSaid,
    command_line, hear, note, register, reply, reply_failure, route, say,
};
