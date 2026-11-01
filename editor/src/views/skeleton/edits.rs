use super::{layers, rigging};
use crate::app::Editor;
use crate::project::ContentEdit;
use crate::tabs::shown_path;
use engine::core::units::MetersPerSec;
use engine::systems::actor::skeleton::{
    Clip, ClipCues, ClipPace, Cue, GroupName, Hand, JointName, Layer, SkeletonDef,
};
use std::collections::BTreeMap;

/// Writes the skeleton at `key` as `change` leaves it, as one History step labelled `label`.
pub fn change(
    editor: &mut Editor,
    key: &str,
    label: &str,
    change: impl FnOnce(&mut SkeletonDef) -> Result<(), String>,
) -> bool {
    let edit = ContentEdit::new(format!("{label} in {}", shown_path(key)));
    editor.change(edit, key, change)
}

/// Gives the clip `clip` the cues `cues`, in order of time: none takes its entry away.
pub fn set_cues(editor: &mut Editor, key: &str, clip: &Clip, mut cues: Vec<Cue>) -> bool {
    cues.sort_by(|a, b| a.at.0.total_cmp(&b.at.0));
    change(
        editor,
        key,
        &format!("Mark {}'s cues", clip.shown()),
        |skeleton| {
            skeleton.cues.retain(|held| held.clip != *clip);
            if !cues.is_empty() {
                skeleton.cues.push(ClipCues {
                    clip: clip.clone(),
                    cues,
                });
                skeleton.cues.sort_by(|a, b| a.clip.cmp(&b.clip));
            }
            Ok(())
        },
    )
}

/// Gives the clip `clip` the pace `pace`, merging with the pace set just before it as one drag does.
pub fn set_pace(editor: &mut Editor, key: &str, clip: &Clip, pace: MetersPerSec) -> bool {
    let edit = ContentEdit::new(format!("Pace {} in {}", clip.shown(), shown_path(key))).merging();
    editor.change(edit, key, |skeleton: &mut SkeletonDef| {
        match skeleton.paces.iter_mut().find(|held| held.clip == *clip) {
            Some(held) => held.pace = pace,
            None => {
                skeleton.paces.push(ClipPace {
                    clip: clip.clone(),
                    pace,
                });
                skeleton.paces.sort_by(|a, b| a.clip.cmp(&b.clip));
            }
        }
        Ok(())
    })
}

/// Changes the hand at `index` to `hand`, merging with the change made just before it as one drag
/// does.
pub fn set_hand(editor: &mut Editor, key: &str, index: usize, hand: Hand) -> bool {
    let edit =
        ContentEdit::new(format!("Change hand {} in {}", hand.name, shown_path(key))).merging();
    editor.change(edit, key, |skeleton: &mut SkeletonDef| {
        let held = skeleton
            .hands
            .get_mut(index)
            .ok_or_else(|| format!("it has no hand {}", index + 1))?;
        *held = hand;
        Ok(())
    })
}

/// Lists the skeleton's hands as `hands`, in order: the first main, the second off.
pub fn set_hands(editor: &mut Editor, key: &str, hands: Vec<Hand>) -> bool {
    change(editor, key, "Change the hands", |skeleton| {
        skeleton.hands = hands;
        Ok(())
    })
}

/// Sets the skeleton's groups to `groups`.
pub(crate) fn set_groups(
    editor: &mut Editor,
    key: &str,
    groups: BTreeMap<GroupName, Vec<JointName>>,
) -> bool {
    change(editor, key, "Change the groups", |skeleton| {
        skeleton.groups = groups;
        Ok(())
    })
}

/// Sets the skeleton's layers to `layers`, refused while a leg is not a chain down the rig.
pub fn set_layers(editor: &mut Editor, key: &str, layers: Vec<Layer>) -> bool {
    let rig = editor
        .read::<SkeletonDef>(key)
        .and_then(Result::ok)
        .and_then(|def| rigging(editor, key, &def))
        .and_then(|rigging| rigging.rig.clone());
    layers::set(
        editor,
        key,
        rig.as_deref(),
        layers,
        "Change the layers",
        false,
    )
}
