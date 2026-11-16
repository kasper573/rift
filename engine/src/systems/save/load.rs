//! Reading a saved character under a game whose content may have changed since it was saved.

use super::{GameChange, SavedCharacter};
use crate::core::content::{
    Content, ContentScope, MissingFile, SavedList, name_of, serial, with_content,
};
use crate::core::store::{self, RecordStoreProblem};
use crate::systems::GameSettings;
use crate::systems::actor::ActorModel;
use crate::systems::area;
use crate::systems::class::NodeKey;
use crate::systems::equipment;
use crate::systems::item::ItemStack;
use crate::systems::player::snapshot::{
    AtInitialSpawn, CharacterSnapshot, CharacterStart, SnapshotSpot, start_of,
};
use crate::systems::script::Place;
use crate::systems::skill::Learned;
use crate::systems::skill::learn::plan;

/// A saved character as the game it is read under takes it: where it starts, and what the game
/// changing since it was saved did to it that its player has not been told.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedCharacter {
    pub saved: SavedCharacter,
    pub start: Place,
    pub changes: Vec<GameChange>,
}

/// The character `body` holds, read under the game `content` holds: whatever names a file the
/// game no longer has is set aside, and what no longer fits is put right. Refused when it is not
/// a record this build reads whole, or names something no character is without.
pub fn load(content: &Content, body: &str) -> Result<LoadedCharacter, RecordStoreProblem> {
    let scope = ContentScope::of(content);
    let mut saved: SavedCharacter = store::read(body, &scope.clone().setting_aside())?;
    let character = &mut saved.character;
    let mut changes = gone(character);
    if character
        .basic_attack
        .is_some_and(|skill| !scope.holds(skill))
    {
        character.basic_attack = None;
    }
    let start = settle_start(content, &scope, character);
    let (whole, _) = with_content(scope, || serde_json::to_string(&saved.character));
    whole.map_err(|problem| {
        RecordStoreProblem(format!(
            "the character of {} names what the game no longer has: {problem}",
            saved.account.0
        ))
    })?;
    let character = &mut saved.character;
    changes.extend(refit_gear(content, character));
    changes.extend(refit_bag(content, character));
    changes.extend(refit_skills(content, character));
    let told = std::mem::replace(&mut saved.told, changes.clone());
    changes.retain(|change| !told.contains(change));
    Ok(LoadedCharacter {
        saved,
        start,
        changes,
    })
}

/// A character memory still holds as it left a world of this server, whose game is the one it
/// was saved under.
pub fn unchanged(content: &Content, saved: &SavedCharacter) -> LoadedCharacter {
    LoadedCharacter {
        saved: saved.clone(),
        start: start_of(content, &saved.character),
        changes: Vec::new(),
    }
}

/// What was set aside of everything the character holds that a player would miss.
fn gone(character: &CharacterSnapshot) -> Vec<GameChange> {
    fn missing<T>(list: &SavedList<T>) -> impl Iterator<Item = &MissingFile> {
        list.set_aside()
            .iter()
            .filter_map(|aside| aside.missing.first())
    }
    let mut changes: Vec<GameChange> = missing(&character.bag)
        .chain(missing(&character.equipped))
        .chain(missing(&character.hotbar))
        .chain(missing(&character.quests))
        .chain(missing(&character.statuses))
        .chain(missing(&character.recipe_masteries))
        .map(|file| GameChange::Gone {
            table: file.table.to_owned(),
            name: name_of(&file.path).to_owned(),
        })
        .collect();
    changes.dedup();
    changes
}

/// Where the character starts: where it was saved, the nearest place to it a character can
/// stand, or the game's start when its area is gone or has no such place. Where it rises after a
/// fall follows it when that spot is no longer one to stand on.
fn settle_start(
    content: &Content,
    scope: &ContentScope,
    character: &mut CharacterSnapshot,
) -> Place {
    let spawn = content.built::<GameSettings>().initial_spawn;
    let kept = match character.start {
        CharacterStart::At(place) if scope.holds(place.area) => area::standable(content, place),
        CharacterStart::At(_) => None,
        CharacterStart::InitialSpawn(_) => Some(spawn),
    };
    let Some(start) = kept else {
        character.start = CharacterStart::InitialSpawn(AtInitialSpawn::InitialSpawn);
        character.rises_at = None;
        return spawn;
    };
    if character.start != CharacterStart::InitialSpawn(AtInitialSpawn::InitialSpawn) {
        character.start = CharacterStart::At(start);
    }
    let rises = character.rises_at.map(|spot| Place {
        area: start.area,
        at: (spot.x, spot.y).into(),
    });
    character.rises_at = rises
        .filter(|&rises| area::check_place(content, rises).is_ok())
        .map(|rises| SnapshotSpot {
            x: rises.at.x,
            y: rises.at.y,
        });
    start
}

/// Gear the character's body no longer wears, or two pieces that now take one place, goes to the
/// bag, where the next step keeps what does not fit.
fn refit_gear(content: &Content, character: &mut CharacterSnapshot) -> Vec<GameChange> {
    let body = ActorModel::get(content, character.appearance.model);
    let put = equipment::put_on(content, body, &character.equipped);
    let worn: Vec<_> = put.slots.values().copied().collect();
    let (kept, off): (Vec<_>, Vec<_>) = character
        .equipped
        .iter()
        .copied()
        .partition(|item| worn.contains(item));
    if off.is_empty() {
        return Vec::new();
    }
    *character.equipped = kept;
    character
        .bag
        .extend(off.into_iter().map(|item| ItemStack::new(item, 1)));
    Vec::new()
}

/// Stacks past what a bag now holds are kept aside as they were saved.
fn refit_bag(content: &Content, character: &mut CharacterSnapshot) -> Vec<GameChange> {
    let slots = content.built::<GameSettings>().slots.bag;
    if character.bag.len() <= slots as usize {
        return Vec::new();
    }
    let over = character.bag.split_off(slots as usize);
    let stacks = over.len() as u32;
    let scope = ContentScope::of(content);
    for stack in over {
        let (entry, _) = with_content(scope.clone(), || serial::json_of(&stack));
        character
            .bag
            .put_aside(entry.expect("a stack of the game's items is written"));
    }
    vec![GameChange::DidNotFit { slots, stacks }]
}

/// Ranks the class's tree or the character's points no longer allow are all handed back.
fn refit_skills(content: &Content, character: &mut CharacterSnapshot) -> Vec<GameChange> {
    if character.skills.is_empty() {
        return Vec::new();
    }
    let class = character.class.get(content);
    let picks: Vec<(NodeKey, _)> = character
        .skills
        .iter()
        .map(|learned| (learned.node.clone(), learned.rank))
        .collect();
    let level = class.level_at(character.xp);
    let granted = class.granted_points(character.xp);
    let allowed = plan(content, class, &Learned::default(), &picks, level, |_| true)
        .is_ok_and(|planned| planned.cost <= granted);
    if allowed {
        return Vec::new();
    }
    character.skills.clear();
    vec![GameChange::SkillsReset {
        class: class.name.clone(),
        points: granted.0,
    }]
}
