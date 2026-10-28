use engine::core::audio::playback::{SFX_CLIP, SfxDef};
use engine::core::babble::{BABBLE_LETTERS, BabbleBankDef, LETTERS};
use std::collections::BTreeMap;

use bevy::math::{Quat, Vec3};
use engine::core::assets::ModelRef;
use engine::core::content::files::{name_of, parent_of};
use engine::core::content::gltf::{NodeTree, Rest, TreeNode, placeholder_body, placeholder_rig};
use engine::core::content::{ContentFile, FolderReader, Nearby};
use engine::core::script::Arrow;
use engine::core::time::{Millis, Seconds};
use engine::core::units::{Meters, MetersPerSec};
use engine::core::vfx::VfxShaderDef;
use engine::core::vfx::shader::SHADER_SOURCE;
use engine::systems::actor::Action;
use engine::systems::actor::part::ModelPartDef;
use engine::systems::actor::skeleton::{
    Clip, ClipCell, ClipCues, ClipPace, ClipRef, Cue, SkeletonDef, TakeName,
};
use engine::systems::area::prop::{PROP_GLB, PropDef};
use engine::systems::dialogue::bust::{BUST_FACES, BustDef, GenericExpression};
use engine::systems::item::{ItemDef, ItemKind, ItemModule};
use engine::systems::script::{Node, RiftEvent, RunnerKind, ScriptDef, ScriptId, ScriptModule};
use engine::systems::skill::{SkillDef, SkillModule};
use include_dir::{Dir, include_dir};
use strum::VariantArray;

use crate::project::{ContentEdit, FileBytes, Project};
use crate::rows;
use crate::views::skeleton::rigging::{self, GltfCache};

static DEFAULTS: Dir = include_dir!("$CARGO_MANIFEST_DIR/defaults");

/// Writes a fresh content file of `table` at `path` into `edit`: what its type holds when a
/// designer makes one, written as the game writes it; for a bundle the media parts it can't load
/// without, blank: a clip of short silence, an empty image, a model of nothing visible; and beside it
/// the files it can't load without: an active skill's script answering used, a skeleton's rig and
/// clips, a part's mesh.
pub fn content_file(
    edit: &mut ContentEdit,
    project: &Project,
    table: &str,
    path: &str,
) -> Result<(), String> {
    match table {
        SkeletonDef::TABLE => return blank_skeleton(edit, project, path),
        ModelPartDef::TABLE => return blank_part(edit, project, path),
        _ => {}
    }
    let reader = project.reader();
    let files = reader.fresh(table, path)?;
    for (part, bytes) in blank_parts(table) {
        let file = format!("{path}/{part}");
        if !files.contains_key(&file) {
            edit.write(file, bytes);
        }
    }
    for (file, bytes) in files {
        edit.write(file, FileBytes::from(bytes));
    }
    if table == SkillDef::TABLE {
        used_script(edit, project, path)?;
    }
    Ok(())
}

/// A skeleton of a placeholder rig, a body under its root, whose file also holds a clip of every
/// action, each a second of the body held up, which its base set plays.
fn blank_skeleton(edit: &mut ContentEdit, project: &Project, path: &str) -> Result<(), String> {
    let joint = |name: &str, parent, height| TreeNode {
        name: name.to_owned(),
        parent,
        rest: Rest {
            translation: Vec3::Y * height,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        },
        mesh: false,
    };
    let tree = NodeTree {
        nodes: vec![joint("root", None, 0.0), joint("body", Some(0), 1.0)],
    };
    let length = Seconds(1.0);
    let clips: Vec<(&str, Seconds)> = Action::VARIANTS
        .iter()
        .map(|action| (action.as_ref(), length))
        .collect();
    let rig = ModelRef::new(
        format!("{}/{}/rig.glb", parent_of(path), name_of(path)).trim_start_matches('/'),
    );
    edit.write(
        rig.as_str().to_owned(),
        FileBytes::from(placeholder_body(&tree, &clips, 0)),
    );
    let reader = project.reader_after(edit);
    let mut row = SkeletonDef::fresh(&Nearby::new(&reader, path))?;
    row.rig.files = vec![rig.clone()];
    let take = |action: &Action| Clip::Take {
        file: rig.clone(),
        take: TakeName::new(action.as_ref()),
    };
    row.base.clips = Action::VARIANTS
        .iter()
        .map(|action| (*action, ClipCell::One(ClipRef::Whole(take(action)))))
        .collect();
    row.cues = Action::VARIANTS
        .iter()
        .filter(|action| action.takes_apex())
        .map(|action| ClipCues {
            clip: take(action),
            cues: vec![Cue {
                at: Millis(length.0 * 500.0),
                step: false,
                apex: true,
                sound: None,
            }],
        })
        .collect();
    row.paces = Action::GAITS
        .iter()
        .zip(BLANK_PACES)
        .map(|(gait, pace)| ClipPace {
            clip: take(gait),
            pace: MetersPerSec(Meters(pace)),
        })
        .collect();
    edit.write_content_in(&reader, path, &row)?;
    Ok(())
}

/// How fast a blank skeleton's walk, run and sprint carry it: a person's, until a designer paces
/// its own clips.
const BLANK_PACES: [f32; 3] = [1.4, 3.5, 6.0];

/// A part on the nearest skeleton, of a mesh beside it skinned to that skeleton's rig at rest and
/// too small to notice.
fn blank_part(edit: &mut ContentEdit, project: &Project, path: &str) -> Result<(), String> {
    let reader = project.reader();
    let mut row = ModelPartDef::fresh(&Nearby::new(&reader, path))?;
    let skeleton_path = reader
        .path_of(row.skeleton)
        .ok_or("its skeleton has no file")?
        .to_owned();
    let skeleton = project
        .read::<SkeletonDef>(&skeleton_path)
        .map_err(|_| format!("{} doesn't read", crate::tabs::shown_path(&skeleton_path)))?;
    let rig = rigging::rig_up(&skeleton, &project.files, &mut GltfCache::default())
        .rig
        .ok_or_else(|| format!("no rig of {skeleton_path} stands to skin it to"))?;
    let mesh = ModelRef::new(format!("{}.glb", path.trim_end_matches(".model_part.json")));
    edit.write(
        mesh.as_str().to_owned(),
        FileBytes::from(placeholder_body(&rig, &[], 1)),
    );
    row.mesh = Some(mesh);
    let reader = project.reader_after(edit);
    edit.write_content_in(&reader, path, &row)?;
    Ok(())
}

fn used_script(edit: &mut ContentEdit, project: &Project, skill: &str) -> Result<(), String> {
    let (reader, script) = answering_used(edit, project, skill, RunnerKind::Skill)?;
    let mut row = SkillDef::fresh(&Nearby::new(&reader, skill))?;
    row.modules.push(SkillModule::Script(ScriptModule {
        script,
        values: BTreeMap::new(),
    }));
    edit.write_content_in(&reader, skill, &row)?;
    Ok(())
}

/// Writes into `edit` a script answering used, run by owners of `runs_on`, beside `owner` and named
/// after it. Gives what reads the files with it, and its id there.
pub fn answering_used(
    edit: &mut ContentEdit,
    project: &Project,
    owner: &str,
    runs_on: RunnerKind,
) -> Result<(FolderReader, ScriptId), String> {
    let used = Node::On {
        event: RiftEvent::Used,
        next: Arrow::default(),
    };
    let (_, reader, script) = rows::new_script(
        edit,
        project,
        owner,
        &format!("{}_used", name_of(owner)),
        runs_on,
        BTreeMap::from([("used".to_owned(), used)]),
    )?;
    Ok((reader, script))
}

/// Whether `item` is usable with no script of its own answering used, which the game refuses.
pub fn usable_without_used(project: &Project, reader: &FolderReader, item: &ItemDef) -> bool {
    let answers_used = |module: &ItemModule| {
        let ItemModule::Script(module) = module else {
            return false;
        };
        reader
            .path_of(module.script)
            .and_then(|path| project.read::<ScriptDef>(path).ok())
            .is_some_and(|script| {
                script.nodes.values().any(|node| {
                    matches!(
                        node,
                        Node::On {
                            event: RiftEvent::Used,
                            ..
                        }
                    )
                })
            })
    };
    matches!(item.kind, ItemKind::Usable) && !item.modules.iter().any(answers_used)
}

fn blank_parts(table: &str) -> Vec<(String, FileBytes)> {
    let silence = || default("silence.wav");
    match table {
        SfxDef::TABLE => vec![(SFX_CLIP.named("", "wav"), silence())],
        BabbleBankDef::TABLE => LETTERS
            .chars()
            .map(|letter| (BABBLE_LETTERS.named(&letter.to_string(), "wav"), silence()))
            .collect(),
        BustDef::TABLE => GenericExpression::VARIANTS
            .iter()
            .map(|expression| {
                (
                    BUST_FACES.named(expression.as_ref(), "png"),
                    default("blank.png"),
                )
            })
            .collect(),
        PropDef::TABLE => vec![(
            PROP_GLB.named("", "glb"),
            FileBytes::from(placeholder_rig(&[])),
        )],
        VfxShaderDef::TABLE => vec![(SHADER_SOURCE.named("", "wgsl"), default("shader.wgsl"))],
        _ => Vec::new(),
    }
}

fn default(name: &str) -> FileBytes {
    let file = DEFAULTS
        .get_file(name)
        .unwrap_or_else(|| panic!("the editor ships defaults/{name}"));
    FileBytes::from(file.contents())
}
