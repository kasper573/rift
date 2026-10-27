//! The names a body's fields may hold: the moves any skeleton's sets declare, and the joints, clips,
//! groups, builds, regions, slots and tints of the skeleton a form edits.

use std::collections::BTreeMap;

use engine::core::content::{Content, ContentFile, ContentNames, NamedContent};
use engine::systems::actor::model::Skeletons;
use engine::systems::actor::skeleton::{MoveName, Rung, SkeletonDef, SkeletonId};
use engine::systems::actor::{MaterialDef, ModelDef, ModelId, ModelPartDef};
use engine::systems::actor::{Motion, Pose};
use engine::systems::class::ClassDef;
use engine::systems::item::{ItemDef, Wear};
use engine::systems::npc::NpcDef;
use engine::systems::player::snapshot::CharacterSnapshot;

/// What kind of name a field holds, by the marker its schema carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameKind {
    Move,
    Joint,
    Clip,
    Group,
    Build,
    Region,
    Slot,
    Tint,
}

impl NameKind {
    /// The kind of name a field of `schema` holds, by the marker its schema carries.
    pub fn of_schema(schema: &serde_json::Value) -> Option<NameKind> {
        schema
            .as_object()?
            .keys()
            .find_map(|marker| NameKind::of_marker(marker))
    }

    /// Whether names of the kind are made up where a skeleton lists them, rather than picked.
    fn listed_by_skeleton(self) -> bool {
        matches!(
            self,
            NameKind::Build | NameKind::Region | NameKind::Slot | NameKind::Tint
        )
    }

    pub fn of_marker(marker: &str) -> Option<NameKind> {
        match marker {
            "x-move" => Some(NameKind::Move),
            "x-joint" => Some(NameKind::Joint),
            "x-clip" => Some(NameKind::Clip),
            "x-group" => Some(NameKind::Group),
            "x-build" => Some(NameKind::Build),
            "x-region" => Some(NameKind::Region),
            "x-slot" => Some(NameKind::Slot),
            "x-tint" => Some(NameKind::Tint),
            _ => None,
        }
    }
}

/// A name a field may take, with what it is.
#[derive(Clone, Debug, PartialEq)]
pub struct NameChoice {
    pub name: String,
    pub note: String,
}

/// Where a move is declared: by each skeleton, the sets that name it, base or a stance.
pub fn declarations(content: &Content) -> BTreeMap<MoveName, Vec<(String, Vec<String>)>> {
    let mut declared: BTreeMap<MoveName, Vec<(String, Vec<String>)>> = BTreeMap::new();
    for (_, skeleton) in content.table::<SkeletonDef>().iter() {
        let mut sets: BTreeMap<&MoveName, Vec<String>> = BTreeMap::new();
        for name in skeleton.base.moves.keys() {
            sets.entry(name).or_default().push("base".to_owned());
        }
        for set in &skeleton.stances {
            for name in set.set.moves.keys() {
                sets.entry(name)
                    .or_default()
                    .push(content.name_of(set.stance));
            }
        }
        for (name, sets) in sets {
            declared
                .entry(name.clone())
                .or_default()
                .push((skeleton.shown_name().to_owned(), sets));
        }
    }
    declared
}

/// What a skeleton plays for a move: in its base set and in each stance it has a set for, the clip
/// the ladder finds and whether that set names the move itself.
pub struct MoveCoverage {
    pub skeleton: String,
    pub cells: Vec<(String, String, bool)>,
}

/// What every skeleton that loaded plays for the move `name`.
pub fn coverage(content: &Content, name: &MoveName) -> Vec<MoveCoverage> {
    let motion = Motion::Move(name.clone());
    content
        .table::<SkeletonDef>()
        .iter()
        .filter(|(id, _)| Skeletons::loaded(content, *id).is_some())
        .map(|(_, skeleton)| {
            let columns =
                std::iter::once(None).chain(skeleton.stances.iter().map(|set| Some(set.stance)));
            MoveCoverage {
                skeleton: skeleton.shown_name().to_owned(),
                cells: columns
                    .map(|stance| {
                        let (clip, rung) = skeleton.named(stance, &motion, Pose::Idle);
                        let set =
                            stance.map_or("base".to_owned(), |stance| content.name_of(stance));
                        let own = match stance {
                            Some(_) => rung == Rung::Stance,
                            None => rung == Rung::Base,
                        };
                        (set, clip.clip().0.clone(), own)
                    })
                    .collect(),
            }
        })
        .collect()
}

/// The names a field of `kind` at `path` may hold in the form editing the row at `row`, or none
/// when the form knows no skeleton to take them from, or the field is where its skeleton lists them.
pub fn choices(
    content: &Content,
    kind: NameKind,
    row: Option<(&str, &str)>,
    path: &str,
) -> Option<Vec<NameChoice>> {
    if kind == NameKind::Move {
        return Some(
            declarations(content)
                .into_iter()
                .map(|(name, by)| NameChoice {
                    name: name.0,
                    note: by
                        .iter()
                        .map(|(skeleton, sets)| format!("{skeleton} ({})", sets.join(", ")))
                        .collect::<Vec<_>>()
                        .join(" · "),
                })
                .collect(),
        );
    }
    let (table, key) = row?;
    if table == SkeletonDef::TABLE && kind.listed_by_skeleton() {
        return None;
    }
    if table == ItemDef::TABLE && kind == NameKind::Slot {
        return Some(every_slot(content));
    }
    let skeleton = skeleton_of(content, (table, key), path)?;
    let def = skeleton.get(content);
    let listed = |names: Vec<(String, String)>| {
        names
            .into_iter()
            .map(|(name, note)| NameChoice { name, note })
            .collect()
    };
    match kind {
        NameKind::Region => {
            return Some(listed(
                def.regions
                    .iter()
                    .map(|region| (region.0.clone(), "a region of the skeleton".to_owned()))
                    .collect(),
            ));
        }
        NameKind::Slot => {
            return Some(listed(
                def.slots
                    .iter()
                    .map(|slot| (slot.0.clone(), "a slot of the skeleton".to_owned()))
                    .collect(),
            ));
        }
        NameKind::Tint => {
            return Some(listed(
                def.tints
                    .iter()
                    .map(|tint| {
                        let [r, g, b] = tint.colour.0;
                        (
                            tint.name.0.clone(),
                            format!("#{r:02x}{g:02x}{b:02x} unless painted"),
                        )
                    })
                    .collect(),
            ));
        }
        _ => {}
    }
    let built = Skeletons::loaded(content, skeleton)?;
    Some(match kind {
        NameKind::Joint => built
            .rig
            .joints()
            .map(|joint| NameChoice {
                name: built.rig.nodes[joint].name.clone(),
                note: format!("under {}", built.rig.parent_name(joint)),
            })
            .collect(),
        NameKind::Clip => built
            .clips
            .iter()
            .map(|(name, clip)| NameChoice {
                name: name.0.clone(),
                note: format!(
                    "clips/{}.glb · {:.2} s",
                    built.def.libraries[clip.library].key, clip.length.0
                ),
            })
            .collect(),
        NameKind::Group => built
            .def
            .groups
            .iter()
            .map(|(name, roots)| NameChoice {
                name: name.0.clone(),
                note: roots
                    .iter()
                    .map(|root| root.0.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            })
            .collect(),
        NameKind::Build => built
            .def
            .builds
            .iter()
            .map(|build| NameChoice {
                name: build.0.clone(),
                note: "a build of the skeleton".to_owned(),
            })
            .collect(),
        NameKind::Move => unreachable!("moves are answered above"),
        NameKind::Region | NameKind::Slot | NameKind::Tint => {
            unreachable!("named from the skeleton's file above")
        }
    })
}

/// Every slot any skeleton lists, with the skeletons that list it: an item names its slot by
/// name, the same on every body that has it.
fn every_slot(content: &Content) -> Vec<NameChoice> {
    let mut slots: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (_, skeleton) in content.table::<SkeletonDef>().iter() {
        for slot in &skeleton.slots {
            slots
                .entry(slot.0.clone())
                .or_default()
                .push(skeleton.shown_name().to_owned());
        }
    }
    slots
        .into_iter()
        .map(|(name, skeletons)| NameChoice {
            name,
            note: format!("on {}", skeletons.join(", ")),
        })
        .collect()
}

/// The skeleton the names of the field at `path` of a row belong to: the row itself; the skeleton
/// a model, part or material is made on; the one an npc's or test character's model is, or a
/// class's starts as; the one a worn item's look at `path` is for.
fn skeleton_of(content: &Content, (table, key): (&str, &str), path: &str) -> Option<SkeletonId> {
    let model_skeleton = |model: ModelId| model.get(content).skeleton;
    match table {
        SkeletonDef::TABLE => content.by_path::<SkeletonDef>(key),
        ModelDef::TABLE => Some(content.by_path::<ModelDef>(key)?.get(content).skeleton),
        ModelPartDef::TABLE => Some(content.by_path::<ModelPartDef>(key)?.get(content).skeleton),
        MaterialDef::TABLE => Some(content.by_path::<MaterialDef>(key)?.get(content).skeleton),
        NpcDef::TABLE => Some(model_skeleton(
            content
                .by_path::<NpcDef>(key)?
                .get(content)
                .appearance
                .model,
        )),
        CharacterSnapshot::TABLE => Some(model_skeleton(
            content
                .by_path::<CharacterSnapshot>(key)?
                .get(content)
                .appearance
                .model,
        )),
        ClassDef::TABLE => Some(model_skeleton(
            content.by_path::<ClassDef>(key)?.get(content).model,
        )),
        ItemDef::TABLE => {
            let index: usize = path
                .strip_prefix("/kind/looks/")?
                .split('/')
                .next()?
                .parse()
                .ok()?;
            match content.by_path::<ItemDef>(key)?.get(content).wear()? {
                Wear::Worn { looks, .. } => Some(looks.get(index)?.skeleton),
                Wear::Held { .. } => None,
            }
        }
        _ => None,
    }
}
