//! The lines a player sent from chat, which the recall keys bring back into the field the way a
//! shell brings back its commands. Kept in memory while the player is in the game, and never
//! saved.

use std::collections::VecDeque;

use super::draft::ChatDraft;

/// The lines this client sent, newest first, each as it was typed with what it held, and how
/// far back the player has gone among them.
#[derive(Debug, Default)]
pub struct SentLines {
    lines: VecDeque<ChatDraft>,
    /// How many lines back the field shows: `None` while it shows the player's own draft.
    back: Option<usize>,
    /// What the field held before the player went back, given back past the newest line.
    stashed: Option<InField>,
}

/// What the chat field shows: a line, and the letter its caret stands after.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InField {
    pub draft: ChatDraft,
    pub caret: usize,
}

impl InField {
    /// A recalled line, its caret at its end as a shell puts it.
    fn recalled(draft: &ChatDraft) -> InField {
        InField {
            caret: draft.text().chars().count(),
            draft: draft.clone(),
        }
    }
}

impl SentLines {
    /// Keeps `sent` as the newest line, unless it is the newest already, and lets go of the
    /// oldest past `kept`. A blank line is not kept. Whatever was being browsed is done with.
    pub fn keep(&mut self, sent: ChatDraft, kept: u32) {
        self.stop();
        if sent.text().trim().is_empty() || self.lines.front() == Some(&sent) {
            return;
        }
        self.lines.push_front(sent);
        self.lines.truncate(kept as usize);
    }

    /// The line one further back, stashing `shown` on the first step back. `None` at the
    /// oldest line.
    pub fn earlier(&mut self, shown: &InField) -> Option<InField> {
        let next = self.back.map_or(0, |back| back + 1);
        let line = InField::recalled(self.lines.get(next)?);
        if self.back.is_none() {
            self.stashed = Some(shown.clone());
        }
        self.back = Some(next);
        Some(line)
    }

    /// The line one nearer, or past the newest what the field held before. `None` while not
    /// going back.
    pub fn later(&mut self) -> Option<InField> {
        match self.back? {
            0 => self.give_back(),
            back => {
                self.back = Some(back - 1);
                self.lines.get(back - 1).map(InField::recalled)
            }
        }
    }

    /// Stops going back, giving back what the field held before. `None` while not going back.
    pub fn give_back(&mut self) -> Option<InField> {
        self.back.take()?;
        self.stashed.take()
    }

    /// Stops going back, keeping what the field shows: the player edited it or sent it.
    pub fn stop(&mut self) {
        self.back = None;
        self.stashed = None;
    }

    /// Whether the field shows a line brought back rather than the player's draft.
    pub fn browsing(&self) -> bool {
        self.back.is_some()
    }

    /// Whether going back would bring a line: one further back than the field shows is kept.
    pub fn has_earlier(&self) -> bool {
        self.back.map_or(0, |back| back + 1) < self.lines.len()
    }

    /// Where among the lines the field shows, as line `at` back of `of`, while going back.
    pub fn place(&self) -> Option<(u32, u32)> {
        self.back
            .map(|back| (back as u32 + 1, self.lines.len() as u32))
    }
}
