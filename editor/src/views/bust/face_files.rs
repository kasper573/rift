use super::faces::{BustFace, face_file, faces, lines, speaks_with};
use crate::app::Editor;
use crate::project::{ContentEdit, RowSelection};
use crate::rows;
use crate::tabs::shown_path;
use engine::core::content::ContentFile;
use engine::core::content::files::{NAME_RULE, valid_name};
use engine::core::script::Node;
use engine::systems::dialogue::bust::{BUST_FACES, BustDef, Expression};
use engine::systems::script::ScriptDef;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub(crate) fn png_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if !bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Err(format!("{} is not a PNG", path.display()));
    }
    Ok(bytes)
}

pub(crate) fn replace_file(editor: &mut Editor, key: &str, face: &BustFace, path: &Path) {
    let bytes = match png_bytes(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            editor.toast(error);
            return;
        }
    };
    let mut edit = ContentEdit::new(format!(
        "Replace {} of {}'s bust",
        face.name,
        shown_path(key)
    ));
    edit.write(face.file.to_owned(), bytes.into());
    editor.apply(edit);
}

/// Takes the PNGs at `paths` into the bust as one step: one named after a face the bust has
/// replaces its picture, any other becomes an expression of its own.
pub(crate) fn add_faces(editor: &mut Editor, key: &str, faces: &[BustFace], paths: &[PathBuf]) {
    let mut edit = ContentEdit::new(match paths {
        [path] => format!(
            "Add {} to {}'s bust",
            path.file_name().unwrap_or_default().to_string_lossy(),
            shown_path(key)
        ),
        _ => format!("Add {} faces to {}'s bust", paths.len(), shown_path(key)),
    });
    let named = match face_writes(&mut edit, key, faces, paths) {
        Ok(named) => named,
        Err(error) => {
            editor.toast(error);
            return;
        }
    };
    if editor.apply(edit)
        && let Some(face) = named.last()
    {
        editor.views.bust.views.view_mut(key).face = Some(face.clone());
        let part = BUST_FACES.named(face, "png");
        editor.select(RowSelection::parts(BustDef::TABLE, key, vec![part]));
    }
}

/// Writes each PNG at `paths` as the face its file is named after, numbering a name `faces`
/// or an earlier path already took. Gives the faces' names in order.
pub(crate) fn face_writes(
    edit: &mut ContentEdit,
    key: &str,
    faces: &[BustFace],
    paths: &[PathBuf],
) -> Result<Vec<String>, String> {
    let mut written: Vec<String> = Vec::new();
    for path in paths {
        let bytes = png_bytes(path)?;
        let stem = path
            .file_stem()
            .map(|stem| rows::name_from(&stem.to_string_lossy().replace('-', "_")))
            .filter(|stem| valid_name(stem))
            .unwrap_or_else(|| "expression".to_owned());
        let name = match faces.iter().find(|face| face.name == stem) {
            Some(face) if !written.contains(&face.name) => face.name.clone(),
            _ => {
                let taken: HashSet<String> = faces
                    .iter()
                    .map(|face| face.name.clone())
                    .chain(written.iter().cloned())
                    .collect();
                rows::free_name(&stem, &taken)
            }
        };
        edit.write(face_file(key, &name), bytes.into());
        written.push(name);
    }
    Ok(written)
}

pub fn rename(editor: &mut Editor, key: &str, face: &BustFace, new_name: &str) -> bool {
    let Some(Ok(bust)) = editor.read::<BustDef>(key) else {
        return false;
    };
    if !face.own {
        return false;
    }
    if !valid_name(new_name) {
        editor.toast(format!("A face's name is {NAME_RULE}"));
        return false;
    }
    if faces(&bust).iter().any(|face| face.name == new_name) {
        editor.toast(format!("{new_name} is already a face of this bust"));
        return false;
    }
    let Some(bytes) = editor
        .project
        .as_ref()
        .and_then(|project| project.files.get(&face.file).cloned())
    else {
        return false;
    };
    let Some(mut edit) = retarget_face(
        editor,
        key,
        &face.name,
        new_name,
        format!(
            "Rename face {} of {} to {new_name}",
            face.name,
            shown_path(key)
        ),
    ) else {
        return false;
    };
    edit.remove(face.file.clone());
    edit.write(face_file(key, new_name), bytes);
    editor.apply(edit)
}

pub fn remove(editor: &mut Editor, key: &str, face: &BustFace, replacement: &str) -> bool {
    if !face.own {
        return false;
    }
    let Some(mut edit) = retarget_face(
        editor,
        key,
        &face.name,
        replacement,
        format!("Remove face {} of {}", face.name, shown_path(key)),
    ) else {
        return false;
    };
    edit.remove(face.file.clone());
    editor.apply(edit)
}

/// An edit giving every line that asks the bust at `key` for face `from` face `to` instead.
pub(crate) fn retarget_face(
    editor: &mut Editor,
    key: &str,
    from: &str,
    to: &str,
    label: String,
) -> Option<ContentEdit> {
    let mut scripts: Vec<String> = lines(editor, key)
        .into_iter()
        .filter(|line| line.face == from)
        .map(|line| line.script)
        .collect();
    scripts.sort();
    scripts.dedup();
    let speaks = speaks_with(editor, key)?;
    let (from, to) = (
        Expression::from(from.to_owned()),
        Expression::from(to.to_owned()),
    );
    let mut edit = ContentEdit::new(label);
    for path in scripts {
        let mut script = (*editor.read::<ScriptDef>(&path)?.ok()?).clone();
        for node in script.nodes.values_mut() {
            if let Node::Dialogue(dialogue) = node {
                for line in &mut dialogue.lines {
                    if line.face.as_ref() == Some(&from) && speaks(line) {
                        line.face = Some(to.clone());
                    }
                }
            }
        }
        if let Err(reason) = edit.write_content(editor.project.as_ref()?, &path, &script) {
            editor.toast(reason);
            return None;
        }
    }
    Some(edit)
}
