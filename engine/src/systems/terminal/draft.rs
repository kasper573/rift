//! What a player has typed into chat and not yet sent: its words, and the things set among them.

use std::ops::Range;

use bevy_terminal::{EMBED, TerminalEmbed};

use crate::core::content::Content;
use crate::systems::chat::ChatEmbed;

/// A line being typed. A thing set in it shows as its name, which is held whole: a caret never
/// rests inside it, and erasing any letter of it takes the whole thing out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatDraft {
    text: String,
    held: Vec<HeldEmbed>,
}

#[derive(Clone, Debug, PartialEq)]
struct HeldEmbed {
    /// Which letters of the text show the thing.
    letters: Range<usize>,
    embed: ChatEmbed,
}

impl ChatDraft {
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The things the line holds, each with the letters that show it, in their order.
    pub fn held(&self) -> impl Iterator<Item = (Range<usize>, ChatEmbed)> + '_ {
        self.held
            .iter()
            .map(|held| (held.letters.clone(), held.embed))
    }

    /// Lets go of the line, for one typed afresh.
    pub fn take(&mut self) -> ChatDraft {
        std::mem::take(self)
    }

    /// Has the line read `text` and nothing else: words alone.
    pub fn write(&mut self, text: String) {
        *self = ChatDraft {
            text,
            held: Vec::new(),
        };
    }

    /// Takes in what the field the line is typed in now reads, its caret after letter `caret`
    /// of it. A thing whose name the typing reached into is taken out whole; where that changes
    /// what the field should read, the caret's new place is returned.
    pub fn retyped(&mut self, text: &str, caret: usize) -> Option<usize> {
        let old: Vec<char> = self.text.chars().collect();
        let mut new: Vec<char> = text.chars().collect();
        if old == new {
            return None;
        }
        let caret = caret.min(new.len());
        let (from, old_to) = changed(&old, &new, caret);
        let grown = new.len() as isize - old.len() as isize;
        let mut cuts: Vec<Range<usize>> = Vec::new();
        self.held.retain_mut(|held| {
            let letters = held.letters.clone();
            if letters.end <= from {
                return true;
            }
            if letters.start >= old_to {
                held.letters = shifted(&letters, grown);
                return true;
            }
            if letters.start < from {
                cuts.push(letters.start..from);
            }
            if letters.end > old_to {
                cuts.push(shifted(&(old_to..letters.end), grown));
            }
            false
        });
        let mut caret = caret;
        cuts.sort_by_key(|cut| std::cmp::Reverse(cut.start));
        for cut in &cuts {
            new.drain(cut.clone());
            for held in &mut self.held {
                if held.letters.start >= cut.end {
                    held.letters = shifted(&held.letters, -(cut.len() as isize));
                }
            }
            if caret >= cut.end {
                caret -= cut.len();
            } else if caret > cut.start {
                caret = cut.start;
            }
        }
        self.text = new.into_iter().collect();
        (!cuts.is_empty()).then_some(caret)
    }

    /// Takes the letters `letters` out of the line, the things they reach into with them.
    pub fn cut(&mut self, letters: Range<usize>) {
        let all: Vec<char> = self.text.chars().collect();
        let kept: String = all[..letters.start.min(all.len())]
            .iter()
            .chain(&all[letters.end.min(all.len())..])
            .collect();
        self.retyped(&kept, letters.start);
    }

    /// Sets `embed` in the line before letter `at`, shown as `name`, and tells where the caret
    /// stands after it.
    pub fn embed(&mut self, at: usize, embed: ChatEmbed, name: &str) -> usize {
        let at = self.kept_out(at.min(self.text.chars().count()));
        let long = name.chars().count();
        let before: String = self.text.chars().take(at).collect();
        let after: String = self.text.chars().skip(at).collect();
        self.text = format!("{before}{name}{after}");
        for held in &mut self.held {
            if held.letters.start >= at {
                held.letters = shifted(&held.letters, long as isize);
            }
        }
        let place = self
            .held
            .iter()
            .position(|held| held.letters.start > at)
            .unwrap_or(self.held.len());
        self.held.insert(
            place,
            HeldEmbed {
                letters: at..at + long,
                embed,
            },
        );
        at + long
    }

    /// Sets `embed` at the end of the line, a space apart from what stands there, and tells
    /// where the caret stands after it.
    pub fn add(&mut self, embed: ChatEmbed, name: &str) -> usize {
        if self
            .text
            .chars()
            .last()
            .is_some_and(|last| !last.is_whitespace())
        {
            self.text.push(' ');
        }
        let end = self.text.chars().count();
        self.embed(end, embed, name)
    }

    /// Where a caret after letter `caret` may rest: where it is, or past the thing whose name
    /// it would stand inside.
    pub fn kept_out(&self, caret: usize) -> usize {
        self.held
            .iter()
            .find(|held| held.letters.start < caret && caret < held.letters.end)
            .map_or(caret, |held| held.letters.end)
    }

    /// The line as a terminal sends it: a mark in place of each thing's name, and the things
    /// in their order.
    pub fn sent(&self, content: &Content) -> (String, Vec<TerminalEmbed>) {
        let mut text = String::new();
        let mut held = self.held.iter().peekable();
        for (place, letter) in self.text.chars().enumerate() {
            match held.peek() {
                Some(next) if next.letters.start == place => text.push(EMBED),
                _ => {}
            }
            match held.peek() {
                Some(next) if next.letters.contains(&place) => {
                    if place + 1 == next.letters.end {
                        held.next();
                    }
                }
                _ => text.push(letter),
            }
        }
        let embeds = self
            .held
            .iter()
            .map(|held| held.embed.sent(content))
            .collect();
        (text, embeds)
    }
}

/// Which letters of `old` the typing replaced to make `new`, given that it ended at `caret` in
/// `new`: where they begin in both, and where they end in `old`. The caret settles what a
/// repeated letter would leave open.
fn changed(old: &[char], new: &[char], caret: usize) -> (usize, usize) {
    let after = new.len() - caret;
    let kept_after = if after <= old.len() && new[caret..] == old[old.len() - after..] {
        after
    } else {
        old.iter()
            .rev()
            .zip(new.iter().rev())
            .take_while(|(old, new)| old == new)
            .count()
    };
    let most = old.len().min(new.len()) - kept_after.min(old.len().min(new.len()));
    let from = old
        .iter()
        .zip(new)
        .take(most)
        .take_while(|(old, new)| old == new)
        .count();
    (from, (old.len() - kept_after).max(from))
}

fn shifted(letters: &Range<usize>, by: isize) -> Range<usize> {
    let moved = |at: usize| (at as isize + by).max(0) as usize;
    moved(letters.start)..moved(letters.end)
}
