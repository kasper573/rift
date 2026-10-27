//! A folder of part meshes made on a skeleton's rig, taken in as the skeleton's parts: the files of
//! one part cut for each build pair up by their names, and a pack's prefix comes off.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use engine::core::content::{ContentFile, Shape, content_path, parent_of, valid_name};
use engine::systems::actor::part::{BUILD_MESHES, ModelPartDef, PART_MESH};
use engine::systems::actor::skeleton::{BuildName, SkeletonDef, SkeletonId};
use walkdir::WalkDir;

use crate::app::Editor;
use crate::project::{ContentEdit, FileBytes};
use crate::rows::name_from;
use crate::tabs::{file_title, shown_path};

/// What an import made of a folder.
pub struct ImportedParts {
    /// The parts made or given new meshes, by path.
    pub parts: Vec<String>,
    /// The files cut for some of the skeleton's builds whose part has no file for the others, nor
    /// one for every build.
    pub unpaired: Vec<String>,
}

/// Takes every `.glb` under `folder` in as a part of the skeleton at `key`, as one History step.
/// Files whose names differ only by a build of the skeleton are one part, a mesh for each build;
/// a file naming no build is a part every build wears. The leading words every file's name shares
/// come off, leaving each part a word of its own. A part goes where the skeleton's parts already
/// sit when they all sit in one folder, else in a folder beside the skeleton named after the
/// dropped one; a part already there of the same name keeps what it says and takes the new meshes.
pub fn import(editor: &mut Editor, key: &str, folder: &Path) -> Result<ImportedParts, String> {
    let skeleton = editor
        .read::<SkeletonDef>(key)
        .ok_or("no folder is open")?
        .map_err(|_| format!("{} does not read: fix it first", shown_path(key)))?;
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let reader = project.reader();
    let skeleton_id = reader
        .id_of::<SkeletonDef>(key)
        .ok_or_else(|| format!("{} is not a skeleton", shown_path(key)))?;
    let files = meshes_in(folder);
    if files.is_empty() {
        return Err(format!("{} holds no .glb files", folder.display()));
    }
    let named = named_parts(&files, &skeleton.builds)?;
    let unpaired = named
        .values()
        .filter(|meshes| !meshes.contains_key(&None))
        .filter(|meshes| {
            skeleton
                .builds
                .iter()
                .any(|build| !meshes.contains_key(&Some(build.clone())))
        })
        .flat_map(|meshes| meshes.values().map(|file| file_name(file)))
        .collect();
    let destination = destination(editor, key, skeleton_id, folder);
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let mut edit = ContentEdit::new(format!(
        "Import {} parts onto {}",
        named.len(),
        file_title(key)
    ));
    let mut parts = Vec::new();
    for (name, meshes) in named {
        let path = content_path(&destination, &name, ModelPartDef::TABLE, Shape::Bundle);
        let def = match project.read::<ModelPartDef>(&path) {
            Ok(existing) => existing.as_ref().clone(),
            Err(_) => ModelPartDef {
                skeleton: skeleton_id,
                covers: Vec::new(),
                hides: Vec::new(),
                mesh: None,
                builds: BTreeMap::new(),
            },
        };
        edit.write_content(project, &path, &def)?;
        let written: BTreeMap<String, &PathBuf> = meshes
            .iter()
            .map(|(build, file)| {
                let part = match build {
                    Some(build) => BUILD_MESHES.named(build.as_str(), "glb"),
                    None => PART_MESH.named("", "glb"),
                };
                (format!("{path}/{part}"), *file)
            })
            .collect();
        for stale in project.files.files_of(&path) {
            let part = stale.strip_prefix(&format!("{path}/")).unwrap_or(&stale);
            let mesh = PART_MESH.matches(part) || BUILD_MESHES.matches(part);
            if mesh && !written.contains_key(&stale) {
                edit.remove(stale);
            }
        }
        for (target, source) in written {
            let bytes =
                std::fs::read(source).map_err(|error| format!("{}: {error}", source.display()))?;
            edit.write(target, FileBytes::from(bytes));
        }
        parts.push(path);
    }
    editor.try_apply(edit)?;
    Ok(ImportedParts { parts, unpaired })
}

fn meshes_in(folder: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkDir::new(folder)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("glb"))
        })
        .collect();
    files.sort();
    files
}

/// Each part's meshes by the build each is cut for, `None` for the one every build wears, by the
/// part's name.
fn named_parts<'a>(
    files: &'a [PathBuf],
    builds: &[BuildName],
) -> Result<BTreeMap<String, BTreeMap<Option<BuildName>, &'a PathBuf>>, String> {
    let words: Vec<Vec<&str>> = files
        .iter()
        .map(|file| {
            file.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .split('_')
                .filter(|word| !word.is_empty())
                .collect()
        })
        .collect();
    let build_of = |word: &str| {
        builds
            .iter()
            .find(|build| build.as_str().eq_ignore_ascii_case(word))
    };
    let shared = shared_prefix(&words, &build_of);
    let mut parts: BTreeMap<String, BTreeMap<Option<BuildName>, &PathBuf>> = BTreeMap::new();
    for (file, words) in files.iter().zip(&words) {
        let rest = &words[shared..];
        let build = rest.iter().find_map(|word| build_of(word)).cloned();
        let kept: Vec<&str> = rest
            .iter()
            .copied()
            .filter(|word| build_of(word).is_none())
            .collect();
        let name = part_name(&kept);
        if !valid_name(&name) {
            return Err(format!("{} makes no part name: {name}", file_name(file)));
        }
        let meshes = parts.entry(name.clone()).or_default();
        if let Some(other) = meshes.insert(build.clone(), file) {
            let cut = build.map_or("every build".to_owned(), |build| build.to_string());
            return Err(format!(
                "{} and {} are both the mesh of {name} for {cut}",
                file_name(other),
                file_name(file)
            ));
        }
    }
    Ok(parts)
}

/// How many leading words every name shares, short of leaving any name without a word that is
/// neither a build nor a number.
fn shared_prefix<'a>(
    words: &[Vec<&str>],
    build_of: &impl Fn(&str) -> Option<&'a BuildName>,
) -> usize {
    let first = words.first().map(Vec::as_slice).unwrap_or_default();
    let mut shared = first
        .iter()
        .enumerate()
        .take_while(|(index, word)| words.iter().all(|other| other.get(*index) == Some(word)))
        .count();
    let named = |rest: &[&str]| {
        rest.iter()
            .any(|word| build_of(word).is_none() && !word.bytes().all(|b| b.is_ascii_digit()))
    };
    while shared > 0
        && !words
            .iter()
            .all(|words| named(&words[shared.min(words.len())..]))
    {
        shared -= 1;
    }
    shared
}

/// The words of a file's name as a part's name: lowercase, joined and split at each capital by a
/// dash, as `ArmLowerLeft` `00` makes `arm-lower-left-00`.
fn part_name(words: &[&str]) -> String {
    let spaced: Vec<String> = words
        .iter()
        .map(|word| {
            let mut spaced = String::new();
            let mut last: Option<char> = None;
            for c in word.chars() {
                if c.is_uppercase() && last.is_some_and(|last| last.is_lowercase()) {
                    spaced.push('-');
                }
                spaced.push(c);
                last = Some(c);
            }
            spaced
        })
        .collect();
    name_from(&spaced.join("-"))
}

/// Where the parts go: the one folder the skeleton's parts sit in, else beside the skeleton in a
/// folder named after the dropped one.
fn destination(editor: &Editor, key: &str, skeleton: SkeletonId, dropped: &Path) -> String {
    let folders: Vec<String> = editor
        .loaded()
        .map(|content| {
            content
                .table::<ModelPartDef>()
                .iter()
                .filter(|(_, part)| part.skeleton == skeleton)
                .map(|(id, _)| parent_of(id.path(&content)).to_owned())
                .collect()
        })
        .unwrap_or_default();
    match folders.split_first() {
        Some((first, rest)) if rest.iter().all(|folder| folder == first) => first.clone(),
        _ => {
            let name = dropped
                .file_name()
                .map(|name| part_name(&[name.to_string_lossy().as_ref()]))
                .filter(|name| valid_name(name))
                .unwrap_or_else(|| "parts".to_owned());
            match parent_of(key) {
                "" => name,
                parent => format!("{parent}/{name}"),
            }
        }
    }
}

fn file_name(file: &Path) -> String {
    file.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}
