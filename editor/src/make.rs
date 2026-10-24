use engine::core::audio::playback::{SFX_CLIP, SfxDef};
use engine::core::babble::{BABBLE_LETTERS, BabbleBankDef, LETTERS};
use std::collections::BTreeMap;

use engine::core::content::files::name_of;
use engine::core::content::gltf::placeholder_rig;
use engine::core::content::{ContentFile, FolderReader, Nearby};
use engine::core::script::Arrow;
use engine::core::time::Seconds;
use engine::core::vfx::VfxShaderDef;
use engine::core::vfx::shader::SHADER_SOURCE;
use engine::systems::actor::Action;
use engine::systems::actor::model::{MODEL_GLB, ModelDef};
use engine::systems::area::tileset::{TILESET_IMAGE, TilesetDef};
use engine::systems::item::{ItemDef, ItemKind, ItemModule};
use engine::systems::script::{Node, RiftEvent, RunnerKind, ScriptDef, ScriptId, ScriptModule};
use engine::systems::skill::{SkillDef, SkillModule};
use include_dir::{Dir, include_dir};
use strum::VariantArray;

use crate::project::{ContentEdit, FileBytes, Project};
use crate::rows;

static DEFAULTS: Dir = include_dir!("$CARGO_MANIFEST_DIR/defaults");

/// Writes a fresh content file of `table` at `path` into `edit`: what its type holds when a
/// designer makes one, written as the game writes it; for a bundle the media parts it can't load
/// without, blank: a clip of short silence, an empty image, a rig of nothing visible; and beside it
/// the files it can't load without: an active skill's script answering used.
pub fn content_file(
    edit: &mut ContentEdit,
    project: &Project,
    table: &str,
    path: &str,
) -> Result<(), String> {
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
        TilesetDef::TABLE => vec![(TILESET_IMAGE.named("", "png"), default("blank.png"))],
        ModelDef::TABLE => {
            let clips: Vec<(&str, Seconds)> = Action::VARIANTS
                .iter()
                .map(|action| (action.as_ref(), Seconds(1.0)))
                .collect();
            vec![(
                MODEL_GLB.named("", "glb"),
                FileBytes::from(placeholder_rig(&clips)),
            )]
        }
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
