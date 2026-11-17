//! Finding a player by the start of their name, among every account that has signed in.

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::systems::society::{Society, SocietyMember};

/// A player asking for the players whose names start with this.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct FindCharacters(pub String);

/// The players whose names start with what a player asked for.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CharactersFound {
    /// What was asked for, so an answer to something typed over since is told apart.
    pub starts: String,
    /// The first of them by name, as many as the game suggests at once.
    pub characters: Vec<SocietyMember>,
    /// How many there are in all.
    pub in_all: u32,
}

/// The fewest letters a search goes by.
pub const LEAST_LETTERS: usize = 2;

/// Whether `typed` is enough of a name to search by.
pub fn searches(typed: &str) -> bool {
    typed.trim().chars().count() >= LEAST_LETTERS
}

/// The accounts whose names start with `starts`, whatever its case, the asker's own among them:
/// the first `most` by name, of however many there are.
pub(crate) fn find(society: &Society, starts: &str, most: usize) -> CharactersFound {
    let wanted = starts.trim().to_lowercase();
    let mut found: Vec<SocietyMember> = Vec::new();
    if searches(&wanted) {
        found = society
            .accounts()
            .iter()
            .filter(|account| account.name.to_lowercase().starts_with(&wanted))
            .filter_map(|account| society.member(&account.id))
            .collect();
        found.sort_by(|a, b| {
            (a.name.to_lowercase(), a.account).cmp(&(b.name.to_lowercase(), b.account))
        });
    }
    let in_all = found.len() as u32;
    found.truncate(most);
    CharactersFound {
        starts: starts.to_owned(),
        characters: found,
        in_all,
    }
}
