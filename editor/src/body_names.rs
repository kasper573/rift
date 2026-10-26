//! The names a body's fields may hold: the moves any skeleton's sets declare, and the joints, clips
//! and groups of the skeleton a form edits.

use std::collections::BTreeMap;

use engine::core::content::{Content, ContentFile, ContentNames, NamedContent};
use engine::systems::actor::ModelDef;
use engine::systems::actor::model::Skeletons;
use engine::systems::actor::skeleton::{MoveName, Rung, SkeletonDef, SkeletonId};
use engine::systems::actor::{Motion, Pose};

/// What kind of name a field holds, by the marker its schema carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameKind {
    Move,
    Joint,
    Clip,
    Group,
}

impl NameKind {
    pub fn of_marker(marker: &str) -> Option<NameKind> {
        match marker {
            "x-move" => Some(NameKind::Move),
            "x-joint" => Some(NameKind::Joint),
            "x-clip" => Some(NameKind::Clip),
            "x-group" => Some(NameKind::Group),
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

/// The names a field of `kind` may hold in the form editing the row at `row`, or none when the
/// form knows no skeleton to take them from.
pub fn choices(
    content: &Content,
    kind: NameKind,
    row: Option<(&str, &str)>,
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
    let skeleton = skeleton_of(content, row?)?;
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
        NameKind::Move => unreachable!("moves are answered above"),
    })
}

/// The skeleton a row's names belong to: the row itself, or the skeleton a model wears.
fn skeleton_of(content: &Content, (table, key): (&str, &str)) -> Option<SkeletonId> {
    match table {
        SkeletonDef::TABLE => content.by_path::<SkeletonDef>(key),
        ModelDef::TABLE => Some(content.by_path::<ModelDef>(key)?.get(content).skeleton),
        _ => None,
    }
}
