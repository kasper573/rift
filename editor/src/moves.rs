use std::collections::{BTreeMap, BTreeSet, HashSet};

use engine::core::content::{FIXTURES_DIR, SETTINGS_FILE, name_of, numbered, parent_of};
use engine::systems::registry;

use crate::app::Editor;
use crate::project::files::{FolderIndex, content_of, folder_entry, is_folder};
use crate::project::{ContentEdit, FileBytes, MovedOutside, Project, ProjectFiles};
use crate::tabs::EditorTab;
use crate::usages::{ContentUsage, ContentUsages};
use crate::user::UserFolder;
use ui::words;

/// A file, bundle, media file or folder going from one path to another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathMove {
    pub from: String,
    pub to: String,
}

impl PathMove {
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> PathMove {
        PathMove {
            from: from.into(),
            to: to.into(),
        }
    }

    /// Where `path` lands when this moves: itself if it is what moves, or the same place inside
    /// it if it is inside.
    pub fn moved(&self, path: &str) -> Option<String> {
        if path == self.from {
            return Some(self.to.clone());
        }
        let rest = path.strip_prefix(&self.from)?.strip_prefix('/')?;
        Some(format!("{}/{rest}", self.to))
    }
}

/// What a move does to the folder: the edit, and how many references it rewrites in how many
/// files.
pub struct PlannedMove {
    pub edit: ContentEdit,
    pub rewritten: usize,
    pub files_rewritten: usize,
}

/// Why `path` stays where it is, if the engine finds it there: the fixtures folder, a fixture, or a
/// folder that holds one.
pub fn fixed_place(path: &str) -> Option<&'static str> {
    let registry = registry();
    if path == FIXTURES_DIR {
        return Some("the engine reads its fixtures and the game's settings from this folder");
    }
    if registry.is_fixture(path) {
        return Some("it is a fixture, a file the engine itself reads at this path");
    }
    let inside = format!("{path}/");
    registry
        .fixture_paths()
        .any(|fixture| fixture.starts_with(&inside))
        .then_some("it holds fixtures, files the engine itself reads at their paths")
}

/// Why a copy of `file` could never load, when its type's files are the engine's fixtures and no
/// others.
pub fn uncopyable(file: &str) -> Option<&'static str> {
    content_of(file)
        .filter(|(table, _)| registry().closed(table))
        .map(|_| "the engine reads only its own fixtures of this type, and a game adds none")
}

/// The path a file or folder named `file` gets in `folder`: `file` itself when no neighbour has
/// its name, ignoring case, or else its name numbered " (1)", " (2)", the first that is free.
/// `leaving` names entries that move away in the same step, whose names are free.
pub fn free_path(files: &ProjectFiles, folder: &str, file: &str, leaving: &[String]) -> String {
    free_path_among(files, folder, file, leaving, &[])
}

/// `free_path`, with the paths `arriving` in the same step taken as well.
pub fn free_path_among(
    files: &ProjectFiles,
    folder: &str,
    file: &str,
    leaving: &[String],
    arriving: &[String],
) -> String {
    let mut taken = names_in(files, folder, leaving);
    taken.extend(
        arriving
            .iter()
            .filter(|path| parent_of(path) == folder)
            .map(|path| file_name(path).to_lowercase()),
    );
    let at = |file: &str| match folder {
        "" => file.to_owned(),
        folder => format!("{folder}/{file}"),
    };
    if !taken.contains(&file.to_lowercase()) {
        return at(file);
    }
    let (stem, rest) = file
        .split_once('.')
        .map_or((file, ""), |(stem, rest)| (stem, rest));
    let base = numbered(stem).map_or(stem, |(base, _)| base);
    let dot = if rest.is_empty() { "" } else { "." };
    (1..)
        .map(|n| format!("{base} ({n}){dot}{rest}"))
        .find(|candidate| !taken.contains(&candidate.to_lowercase()))
        .map(|candidate| at(&candidate))
        .expect("a free name")
}

/// Plans each move with every reference to what moves rewritten, as one edit labelled `label`.
pub fn plan(project: &Project, moves: &[PathMove], label: String) -> Result<PlannedMove, String> {
    for step in moves {
        if let Some(reason) = fixed_place(&step.from) {
            return Err(format!("{} stays: {reason}", step.from));
        }
        if step.to.starts_with(&format!("{}/", step.from)) {
            return Err(format!("{} can't go inside itself", step.from));
        }
    }
    let files = &project.files;
    let mut edit = ContentEdit::new(label);
    let named = named_inside(&project.usages, moves);
    let files_rewritten = rewrite_named(project, &mut edit, &named)?;
    for step in moves {
        let entries: Vec<String> = files
            .entries()
            .filter(|entry| step.moved(entry.trim_end_matches('/')).is_some())
            .cloned()
            .collect();
        for entry in entries {
            let Some(bytes) = edit.pending_bytes(files, &entry) else {
                continue;
            };
            let moved = if is_folder(&entry) {
                folder_entry(&step.moved(entry.trim_end_matches('/')).expect("inside"))
            } else {
                step.moved(&entry).expect("inside")
            };
            edit.remove(entry);
            edit.write(moved, bytes);
        }
        keep_folder(files, &mut edit, parent_of(&step.from));
    }
    Ok(PlannedMove {
        edit,
        rewritten: named.len(),
        files_rewritten,
    })
}

/// Points every reference to a path that moved on disk at where its file is now, as one step.
/// Says how many references were rewritten.
pub fn repoint(editor: &mut Editor, moved: &[MovedOutside]) -> Result<usize, String> {
    let project = settled(editor)?;
    let named: Vec<(&ContentUsage, String, String)> = moved
        .iter()
        .flat_map(|step| {
            let usages = &project.usages;
            usages
                .of_content(&step.from)
                .iter()
                .chain(usages.of_media(&step.from))
                .map(|usage| (usage, step.from.clone(), step.to.clone()))
        })
        .collect();
    let label = match moved {
        [one] => format!("Point {} at {}", one.from, one.to),
        many => format!("Point {} moved paths at where they are", many.len()),
    };
    let mut edit = ContentEdit::new(label);
    rewrite_named(project, &mut edit, &named)?;
    let count = named.len();
    editor.try_apply(edit)?;
    Ok(count)
}

/// The open folder with every pending load taken in, so its usages name what the files name now
/// unless that load failed.
pub fn settled(editor: &mut Editor) -> Result<&Project, String> {
    let project = editor.project.as_mut().ok_or("no folder is open")?;
    project.settle();
    Ok(project)
}

/// What the toast says of files moved on disk away from what names them: one file, a folder of
/// them, or several files.
pub fn moved_outside_summary(moved: &[MovedOutside], index: &FolderIndex) -> String {
    let references: usize = moved.iter().map(|step| step.references).sum();
    let broken = format!(
        "{references} reference{} to {} {} broken",
        words::plural(references),
        if moved.len() == 1 { "it" } else { "them" },
        if references == 1 { "is" } else { "are" }
    );
    let folders: Vec<Option<(String, String)>> =
        moved.iter().map(|step| moved_folder(step, index)).collect();
    match (moved, folders.first()) {
        ([], _) => String::new(),
        ([_], Some(None)) => format!("A file moved on disk and {broken}"),
        (_, Some(Some((from, to)))) if folders.iter().all(|folder| *folder == folders[0]) => {
            format!("A folder moved on disk, {from} to {to}, and {broken}")
        }
        (many, _) => format!("{} files moved on disk and {broken}", many.len()),
    }
}

/// The folder that took `step`'s file along when it moved, and where it went: the shallowest
/// folder above the file that is gone, matched with where the rest of the file's path now is.
fn moved_folder(step: &MovedOutside, index: &FolderIndex) -> Option<(String, String)> {
    let from: Vec<&str> = step.from.split('/').collect();
    let to: Vec<&str> = step.to.split('/').collect();
    let same = from
        .iter()
        .rev()
        .zip(to.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let mut folder = from[..from.len() - same].join("/");
    let mut went = to[..to.len() - same].join("/");
    let join = |base: &str, part: &str| match base {
        "" => part.to_owned(),
        base => format!("{base}/{part}"),
    };
    for part in &from[from.len() - same..from.len() - 1] {
        if !folder.is_empty() && !index.folders.contains(&folder) {
            break;
        }
        folder = join(&folder, part);
        went = join(&went, part);
    }
    (!folder.is_empty() && !index.folders.contains(&folder)).then_some((folder, went))
}

/// Writes each reference's new path into the files that hold them, each file read and written
/// back by its type. Gives how many files that touched.
fn rewrite_named(
    project: &Project,
    edit: &mut ContentEdit,
    named: &[(&ContentUsage, String, String)],
) -> Result<usize, String> {
    let renames: BTreeMap<String, String> = named
        .iter()
        .map(|(_, from, to)| (from.clone(), to.clone()))
        .collect();
    rewrite_renamed(
        project,
        edit,
        named.iter().map(|(usage, ..)| usage.file.as_str()),
        &renames,
    )
}

/// Writes every path `renames` names as a key as its value in the content files and settings
/// that `files` belong to, each read as its type and written back as the game writes it, as the
/// folder stands once `edit` is applied. Gives how many files that touched.
pub fn rewrite_renamed<'f>(
    project: &Project,
    edit: &mut ContentEdit,
    files: impl IntoIterator<Item = &'f str>,
    renames: &BTreeMap<String, String>,
) -> Result<usize, String> {
    let holders: BTreeSet<&str> = files
        .into_iter()
        .filter_map(|file| match file {
            SETTINGS_FILE => Some(SETTINGS_FILE),
            file => content_of(file).map(|(_, path)| path),
        })
        .collect();
    if holders.is_empty() {
        return Ok(0);
    }
    let reader = project.reader_after(edit);
    for holder in &holders {
        if *holder == SETTINGS_FILE {
            for (file, bytes) in reader.rewritten_settings(renames)? {
                edit.write(file, FileBytes::from(bytes));
            }
            continue;
        }
        let written = reader.rewritten(holder, renames)?;
        for (file, bytes) in written.files {
            edit.write(file, FileBytes::from(bytes));
        }
        for file in written.removed {
            edit.remove(file);
        }
    }
    Ok(holders.len())
}

/// Moves and renames as one step, and points every open tab and the selection at where things
/// went. Says what was rewritten.
pub fn apply(editor: &mut Editor, moves: &[PathMove], label: String) -> Result<(), String> {
    let project = settled(editor)?;
    let planned = plan(project, moves, label.clone())?;
    let (rewritten, files) = (planned.rewritten, planned.files_rewritten);
    editor.try_apply(planned.edit)?;
    follow(editor, moves);
    let mut said = label;
    if rewritten > 0 {
        said.push_str(&format!(
            ": {rewritten} reference{} in {files} file{} rewritten",
            words::plural(rewritten),
            words::plural(files)
        ));
    }
    editor.toast(said);
    Ok(())
}

/// Points open tabs, the selection and open folders at where `moves` took what they show.
pub fn follow(editor: &mut Editor, moves: &[PathMove]) {
    let moved = |path: &str| moves.iter().find_map(|step| step.moved(path));
    editor.rename_tabs(|tab| match tab {
        EditorTab::Script(owner) => {
            moved(&owner.key).map(|to| EditorTab::Script(crate::views::script::ScriptRow::new(&to)))
        }
        EditorTab::Asset { path } => moved(path).map(|path| EditorTab::Asset { path }),
        EditorTab::Rows { table, keys } => {
            keys.iter()
                .any(|key| moved(key).is_some())
                .then(|| EditorTab::Rows {
                    table: table.clone(),
                    keys: keys
                        .iter()
                        .map(|key| moved(key).unwrap_or_else(|| key.clone()))
                        .collect(),
                })
        }
        other => {
            let (table, key) = other.row()?;
            moved(&key).map(|to| EditorTab::for_row(&table, &to))
        }
    });
    for key in &mut editor.selection.keys {
        if let Some(to) = moved(key) {
            *key = to;
        }
    }
    editor.views.area.moved(&moved);
    if let Some(project) = &editor.project {
        let folder = UserFolder::folder_key(&project.root);
        if let Some(view) = editor.settings.content_trees.get_mut(&folder) {
            view.moved(&moved);
        }
    }
}

/// The name a moved or renamed file or folder takes in `folder`, as `free_path` gives it.
pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `path` renamed to `name`, the name before its first dot, keeping its type.
pub fn renamed(path: &str, name: &str) -> String {
    let file = file_name(path);
    let rest = file.strip_prefix(name_of(path)).unwrap_or("");
    match parent_of(path) {
        "" => format!("{name}{rest}"),
        folder => format!("{folder}/{name}{rest}"),
    }
}

/// Every reference to a path that moves, with that path and where it lands.
fn named_inside<'u>(
    usages: &'u ContentUsages,
    moves: &[PathMove],
) -> Vec<(&'u ContentUsage, String, String)> {
    let mut named = Vec::new();
    for (path, uses) in usages.content().chain(usages.media()) {
        if let Some(to) = moves.iter().find_map(|step| step.moved(path)) {
            named.extend(
                uses.iter()
                    .map(|usage| (usage, path.to_owned(), to.clone())),
            );
        }
    }
    named
}

/// Keeps `folder` once what moves leaves it, as a file manager does.
pub fn keep_folder(files: &ProjectFiles, edit: &mut ContentEdit, folder: &str) {
    if folder.is_empty() {
        return;
    }
    let kept = folder_entry(folder);
    let still_holds = files
        .paths_under(&kept)
        .any(|entry| edit.pending_bytes(files, entry).is_some());
    let written = edit
        .changes
        .iter()
        .any(|change| change.bytes.is_some() && change.file.starts_with(&kept));
    if !still_holds && !written {
        edit.write(kept, Default::default());
    }
}

fn names_in(files: &ProjectFiles, folder: &str, leaving: &[String]) -> HashSet<String> {
    let prefix = match folder {
        "" => String::new(),
        folder => folder_entry(folder),
    };
    files
        .paths_under(&prefix)
        .filter(|entry| {
            !leaving
                .iter()
                .any(|gone| *entry == gone || entry.starts_with(&format!("{gone}/")))
        })
        .filter_map(|entry| entry[prefix.len()..].split('/').next())
        .filter(|name| !name.is_empty())
        .map(str::to_lowercase)
        .collect()
}
