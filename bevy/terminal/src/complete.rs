use bevy_ecs::message::Message;
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use serde::{Deserialize, Serialize};

use crate::terminal::{MAX_INPUT_LEN, command_line};

/// What a connection has typed and not yet sent, asking how it may go on.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TerminalCompletionRequest {
    pub text: String,
}

impl TerminalCompletionRequest {
    /// The command line being typed, if it is one no longer than a line may be: only those are
    /// completed. What trails it is kept, as a space after a word ends the word.
    pub fn command(&self) -> Option<&str> {
        let line = command_line(self.text.trim_start())?;
        (line.chars().count() <= MAX_INPUT_LEN).then_some(line)
    }
}

/// How the text a connection asked about may go on, best first.
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct TerminalCompletions {
    /// The text asked about, by which a terminal tells an answer to what it no longer holds.
    pub text: String,
    /// How many characters of the text stand before what an option replaces.
    pub keeps: u32,
    pub options: Vec<TerminalOption>,
    /// What is being typed, in words: a command's inputs, the one at hand marked.
    pub usage: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TerminalOption {
    /// What takes the place of the text being typed.
    pub text: String,
    /// What the option is, beside its text.
    pub detail: String,
}

impl TerminalCompletions {
    /// The text asked about with `option` in place of what was being typed.
    pub fn applied(&self, option: &TerminalOption) -> String {
        let kept: String = self.text.chars().take(self.keeps as usize).collect();
        format!("{kept}{}", option.text)
    }
}

/// Which of `labels` match `typed` best, the `most` of them at the most and the best first: the
/// place of each among them. While nothing is typed, the first of them in the order given.
pub fn best_matches<'l>(
    typed: &str,
    labels: impl IntoIterator<Item = &'l str>,
    most: usize,
) -> Vec<usize> {
    let typed = typed.trim();
    if typed.is_empty() {
        return (0..labels.into_iter().count().min(most)).collect();
    }
    let pattern = Pattern::new(
        typed,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buffer = Vec::new();
    let mut scored: Vec<(u32, usize)> = labels
        .into_iter()
        .enumerate()
        .filter_map(|(place, label)| {
            let score = pattern.score(Utf32Str::new(label, &mut buffer), &mut matcher)?;
            Some((score, place))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored
        .into_iter()
        .take(most)
        .map(|(_, place)| place)
        .collect()
}
