//! Finding a player by the start of their name, among every account that has signed in.

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::content::Content;
use crate::systems::GameSettings;
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

/// Whether `typed` is enough of a name to search by in the game `content` is of.
pub fn searches(content: &Content, typed: &str) -> bool {
    goes_by(typed, least_letters(content))
}

/// The fewest letters of a name a search for a player goes by.
pub fn least_letters(content: &Content) -> u32 {
    content.built::<GameSettings>().search_letters
}

/// The accounts whose names start with `starts`, whatever its case, the asker's own among them:
/// the first `most` by name, of however many there are.
pub(crate) fn find(
    society: &Society,
    starts: &str,
    most: usize,
    search_letters: u32,
) -> CharactersFound {
    let wanted = starts.trim().to_lowercase();
    let mut found: Vec<SocietyMember> = Vec::new();
    if goes_by(&wanted, search_letters) {
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

fn goes_by(typed: &str, search_letters: u32) -> bool {
    typed.trim().chars().count() >= search_letters as usize
}
