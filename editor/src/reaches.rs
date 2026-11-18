use engine::core::script::Reach;
use engine::systems::item::ItemId;
use engine::systems::item::effects::{Drops, Gives, Takes};
use engine::systems::quest::{Accepts, Completes, Decides, Fails, Offers, QuestId, TurnsIn};

/// What a script may do to a quest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestVerb {
    Offers,
    Accepts,
    TurnsIn,
    Fails,
    Completes,
    Decides,
}

/// The quest a reach is about and what it may do to it, when it is about one.
pub fn quest_reach(reach: &Reach) -> Option<(QuestVerb, QuestId)> {
    if let Some(Offers(quest)) = reach.downcast_ref() {
        return Some((QuestVerb::Offers, *quest));
    }
    if let Some(Accepts(quest)) = reach.downcast_ref() {
        return Some((QuestVerb::Accepts, *quest));
    }
    if let Some(TurnsIn(quest)) = reach.downcast_ref() {
        return Some((QuestVerb::TurnsIn, *quest));
    }
    if let Some(Fails(quest)) = reach.downcast_ref() {
        return Some((QuestVerb::Fails, *quest));
    }
    if let Some(Completes(quest)) = reach.downcast_ref() {
        return Some((QuestVerb::Completes, *quest));
    }
    if let Some(Decides(quest)) = reach.downcast_ref() {
        return Some((QuestVerb::Decides, *quest));
    }
    None
}

/// Whether the reach is the one verb of the quest `id`.
pub fn does(reach: &Reach, verbs: &[QuestVerb], id: QuestId) -> bool {
    quest_reach(reach).is_some_and(|(verb, quest)| quest == id && verbs.contains(&verb))
}

/// What a script may do with an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemVerb {
    Takes,
    Gives,
    Drops,
}

/// The item a reach is about and what it may do with it, when it is about one.
pub fn item_reach(reach: &Reach) -> Option<(ItemVerb, ItemId)> {
    if let Some(Takes(item)) = reach.downcast_ref() {
        return Some((ItemVerb::Takes, *item));
    }
    if let Some(Gives(item)) = reach.downcast_ref() {
        return Some((ItemVerb::Gives, *item));
    }
    if let Some(Drops(item)) = reach.downcast_ref() {
        return Some((ItemVerb::Drops, *item));
    }
    None
}
