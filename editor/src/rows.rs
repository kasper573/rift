use std::collections::{BTreeMap, HashSet};
use ui::controls;
use ui::egui;
use ui::panel;
use ui::theme;
use ui::words;

use engine::core::assets::AssetKind;
use engine::core::content::files::NAME_RULE;
use engine::core::content::serial::TypeTag;
use engine::core::content::{
    ContentFile, FolderReader, MODULES, ModuleRule, NOTE_EXTENSION, PartSpec, fields_file, name_of,
    parent_of, table_of, valid_name,
};
use engine::core::script::RunnerKindDef;
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::scene::SCENE_FILE;
use engine::systems::registry;
use engine::systems::script::{
    ListedScriptModule, Node, RunnerKind, ScriptDef, ScriptId, ScriptModule,
};
use serde_json::Value;
use ui::egui::Ui;

use crate::app::{DeleteDialog, Editor, ModalDialog};
use crate::make;
use crate::moves::{self, PathMove, fixed_place, free_path, free_path_among};
use crate::project::files::{FolderIndex, content_holding, folder_entry};
use crate::project::{ContentEdit, FileBytes, Project, ProjectFiles};
use crate::scene_graph;
use crate::schema::{self, SchemaShapes};
use crate::shell::widgets;
use crate::tabs::shown_path;
use crate::tabs::{EditorTab, file_title};
use crate::usages::ContentUsage;
use crate::views::area::{self as area, placement};
use crate::views::script::nodes as script_nodes;

/// What deleted files leave behind: the uses of one kind of a deleted file, and what becomes of
/// them.
#[derive(Clone, Debug)]
pub struct UsageGroup {
    /// The type of the deleted files the group's uses name, or `MEDIA`.
    pub target: String,
    pub table: String,
    pub kind: String,
    pub usages: Vec<ContentUsage>,
    pub removable: bool,
    pub whole_modules: bool,
    pub decision: UsageDecision,
}

/// The target of uses that name media rather than a content file.
pub const MEDIA: &str = "media";

#[derive(Clone, Debug, PartialEq)]
pub enum UsageDecision {
    Undecided,
    Remove,
    Repoint(String),
    Unpack,
}

/// Why `name` can't name a file or folder beside the others in `folder`, leaving out the one at
/// `own` that is being renamed. `kind` is what follows a name in the file's name, its type and
/// extension (`.npc.json`), empty for a folder: two files of different kinds may share a name,
/// as an npc and its model often do.
pub fn name_problem(
    files: &ProjectFiles,
    folder: &str,
    name: &str,
    kind: &str,
    own: Option<&str>,
) -> Option<String> {
    let note = own.is_some_and(|own| own.ends_with(&format!(".{NOTE_EXTENSION}")));
    if note && (name.is_empty() || name.starts_with('.') || name.contains(['/', '\\'])) {
        return Some(
            "a note's name is any name without a slash, not starting with a dot".to_owned(),
        );
    }
    if !note && !valid_name(name) {
        return Some(format!("a name is {NAME_RULE}"));
    }
    let wanted = format!("{name}{kind}").to_lowercase();
    let prefix = match folder {
        "" => String::new(),
        folder => folder_entry(folder),
    };
    files
        .paths_under(&prefix)
        .filter_map(|entry| entry[prefix.len()..].split('/').next())
        .filter(|file| !file.is_empty())
        .find(|file| {
            let path = format!("{prefix}{file}");
            Some(path.as_str()) != own && file.to_lowercase() == wanted
        })
        .map(|taken| format!("{taken} already has that name here (names ignore case)"))
}

/// What follows the name in the file name of `path`: its type and extension, with the dot.
pub fn name_kind(path: &str) -> &str {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.find('.').map_or("", |dot| &file[dot..])
}

/// `name` as a name within a file, such as a node's or a variable's: `name` itself when none of
/// `taken` has it, ignoring case, or else `name_2`, `name_3`, the first that is free.
pub fn free_name(name: &str, taken: &HashSet<String>) -> String {
    let is_taken = |candidate: &str| taken.iter().any(|t| t.eq_ignore_ascii_case(candidate));
    if !is_taken(name) {
        return name.to_owned();
    }
    let base = match name.rfind('_') {
        Some(cut) if cut > 0 && name[cut + 1..].chars().all(|c| c.is_ascii_digit()) => &name[..cut],
        _ => name,
    };
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|candidate| !is_taken(candidate))
        .expect("a free name")
}

/// A file name made from what someone typed: lower case, spaces as underscores, and only what a
/// name may hold.
pub fn name_from(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .chars()
        .map(|c| if c == ' ' { '_' } else { c })
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-')
        .collect()
}

/// The path a new content file of `table` named `name` gets in `folder`, numbered when the name is
/// taken there.
pub fn new_path(files: &ProjectFiles, folder: &str, table: &str, name: &str) -> String {
    let shape = registry()
        .shape_of(table)
        .unwrap_or(engine::core::content::Shape::Json);
    let wanted = engine::core::content::content_path("", name, table, shape);
    free_path(files, folder, &wanted, &[])
}

/// Makes a content file of `table` in `folder`, named after its type and numbered when that is
/// taken, holding the type's default content, and opens it. Gives its path.
pub fn new_file(editor: &mut Editor, folder: &str, table: &str) -> Result<String, String> {
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let path = new_path(&project.files, folder, table, table);
    let mut edit = ContentEdit::new(format!("New {}", shown_path(&path)));
    make::content_file(&mut edit, project, table, &path)?;
    editor.try_apply(edit)?;
    editor.open_row(table, &path);
    Ok(path)
}

/// Makes a content file of `table` beside `file` and writes its path into `file` at `pointer`, as
/// one step.
pub fn new_file_into(
    editor: &mut Editor,
    table: &str,
    file: &str,
    pointer: &str,
) -> Result<String, String> {
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let folder = parent_of(content_holding(file).unwrap_or(file)).to_owned();
    let path = new_path(&project.files, &folder, table, table);
    let mut edit = ContentEdit::new(format!("New {}", shown_path(&path)));
    make::content_file(&mut edit, project, table, &path)?;
    edit.update_json(&project.files, file, |target| {
        *target.pointer_mut(pointer)? = Value::String(path.clone());
        Some(())
    });
    editor.try_apply(edit)?;
    editor.open_row(table, &path);
    Ok(path)
}

/// Gives the row `key` of `table` a script module naming a new script beside it, run by rows of
/// its kind, as one step: what adding a script module takes when no script runs on its kind yet.
pub fn new_script_module(editor: &mut Editor, table: &str, key: &str) -> Result<String, String> {
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let runs_on: RunnerKind = table
        .parse()
        .map_err(|_| format!("a {table} runs no scripts"))?;
    let file = fields_file(key);
    let row = project
        .files
        .json_owned(&file)
        .ok_or_else(|| format!("{} doesn't read", shown_path(key)))?;
    let listed = project.reader().modules_in(table, &row)?;
    let answered = answered_by(project, &listed);
    let mut edit = ContentEdit::new(String::new());
    let answering = answering_first(project, key, runs_on, &answered)?;
    let (path, reader, script) =
        new_script(&mut edit, project, key, name_of(key), runs_on, answering)?;
    edit.label = format!("New {} run by {}", shown_path(&path), shown_path(key));
    let module = reader
        .to_json(&ListedScriptModule::Script(ScriptModule {
            script,
            values: BTreeMap::new(),
        }))
        .map_err(|error| error.to_string())?;
    let mut modules = reader.modules_in(table, &row)?;
    modules.push(module);
    let row = reader.with_modules_in(table, &row, &modules)?;
    edit.write_json(&file, &row);
    editor.try_apply(edit)?;
    Ok(path)
}

/// The nodes of a new script run by owners of `runs_on`, made for the file `key`: one starting a
/// flow at the first event they answer that none in `answered` is, as every script a row runs
/// needs one and an owner answers each event once.
pub fn answering_first(
    project: &Project,
    key: &str,
    runs_on: RunnerKind,
    answered: &HashSet<&str>,
) -> Result<BTreeMap<String, Node>, String> {
    let nearby = schema::Nearby::of(project, Some(key));
    let event = script_nodes::free_event(
        &project.reader(),
        &project.catalogue,
        runs_on,
        answered,
        nearby,
    )
    .ok_or_else(|| {
        format!(
            "every event a {} answers already starts a script of {}",
            runs_on.name(),
            shown_path(key)
        )
    })?;
    let name = script_nodes::event_name(&event).unwrap_or("on").to_owned();
    Ok(BTreeMap::from([(name, event)]))
}

/// Writes into `edit` a script of `nodes`, run by owners of `runs_on`, named `name` beside the file
/// `beside`. Gives its path, what reads the files with it, and its id there.
pub fn new_script(
    edit: &mut ContentEdit,
    project: &Project,
    beside: &str,
    name: &str,
    runs_on: RunnerKind,
    nodes: BTreeMap<String, Node>,
) -> Result<(String, FolderReader, ScriptId), String> {
    let path = new_path(
        &project.files.with(&edit.changes),
        parent_of(beside),
        ScriptDef::TABLE,
        name,
    );
    let script = ScriptDef {
        nodes,
        ..ScriptDef::running_on(runs_on)
    };
    edit.write_content(project, &path, &script)?;
    let reader = project.reader_after(edit);
    let script = reader
        .id_of(&path)
        .ok_or_else(|| format!("{} doesn't read", shown_path(&path)))?;
    Ok((path, reader, script))
}

/// Makes a folder named "folder", numbered when that is taken, in `parent`. Gives its path.
pub fn new_folder(editor: &mut Editor, parent: &str) -> Result<String, String> {
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let path = free_path(&project.files, parent, "folder", &[]);
    let mut edit = ContentEdit::new(match parent {
        "" => "New folder".to_owned(),
        parent => format!("New folder in {}", shown_path(parent)),
    });
    edit.write(folder_entry(&path), FileBytes::from([]));
    editor.try_apply(edit)?;
    Ok(path)
}

/// Makes an empty note named "note" in `folder`. Gives its path.
pub fn new_note(editor: &mut Editor, folder: &str) -> Result<String, String> {
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let path = free_path(
        &project.files,
        folder,
        &format!("note.{NOTE_EXTENSION}"),
        &[],
    );
    let mut edit = ContentEdit::new(format!("New note {path}"));
    edit.write(path.clone(), FileBytes::from([]));
    editor.try_apply(edit)?;
    Ok(path)
}

/// Copies the file or bundle at `path` beside it as "name (1)", and opens the copy.
pub fn duplicate(editor: &mut Editor, path: &str) -> Result<String, String> {
    let copies = copy_into(
        editor,
        &[path.to_owned()],
        parent_of(path),
        format!("Duplicate {}", shown_path(path)),
    )?;
    let copy = copies.into_iter().next().ok_or("nothing was copied")?;
    if let Some(table) = table_of(&copy).filter(|_| content_holding(&copy).is_some()) {
        let table = table.to_owned();
        editor.open_row(&table, &copy);
    }
    Ok(copy)
}

/// Copies what is at each path, a file, bundle or folder with all it holds, into `folder`,
/// numbering a name taken there, as one step labelled `label`. Gives where each copy landed.
pub fn copy_into(
    editor: &mut Editor,
    paths: &[String],
    folder: &str,
    label: String,
) -> Result<Vec<String>, String> {
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let mut edit = ContentEdit::new(label);
    let mut copies: Vec<String> = Vec::new();
    for path in paths {
        if folder == path || folder.starts_with(&format!("{path}/")) {
            return Err(format!(
                "{} can't be copied inside itself",
                shown_path(path)
            ));
        }
        let copy = free_path_among(&project.files, folder, moves::file_name(path), &[], &copies);
        let held = if project.files.index().folders.contains(path) {
            project
                .files
                .paths_under(&folder_entry(path))
                .cloned()
                .collect()
        } else {
            project.files.files_of(path)
        };
        if let Some(reason) = held.iter().find_map(|file| moves::uncopyable(file)) {
            return Err(format!("{} can't be copied: {reason}", shown_path(path)));
        }
        let files: Vec<(String, FileBytes)> = held
            .into_iter()
            .filter_map(|file| Some((file.clone(), project.files.get(&file)?.clone())))
            .collect();
        write_moved(&mut edit, &files, path, &copy);
        copies.push(copy);
    }
    let copied: Vec<(String, String)> = paths.iter().cloned().zip(copies.iter().cloned()).collect();
    crate::views::world::cards_beside(project, &mut edit, &copied)?;
    editor.try_apply(edit)?;
    Ok(copies)
}

/// Copies media files from anywhere on disk into `folder`, numbering a name taken there, as one
/// step. Gives their paths.
pub fn import_media(
    editor: &mut Editor,
    folder: &str,
    sources: &[std::path::PathBuf],
) -> Result<Vec<String>, String> {
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let mut edit = ContentEdit::new(match sources {
        [one] => format!("Import {}", import_name(one)),
        many => format!("Import {} files", many.len()),
    });
    let mut made: Vec<String> = Vec::new();
    for source in sources {
        let name = import_name(source);
        if AssetKind::of(&name).is_none() {
            return Err(format!("{name} is no kind of media a game holds"));
        }
        let bytes = std::fs::read(source).map_err(|error| format!("{name}: {error}"))?;
        let path = free_path_among(&project.files, folder, &name, &[], &made);
        edit.write(path.clone(), FileBytes::from(bytes));
        made.push(path);
    }
    editor.try_apply(edit)?;
    Ok(made)
}

/// Puts the bytes of `source` in place of the media file at `path`, which keeps its name.
pub fn replace_media(
    editor: &mut Editor,
    path: &str,
    source: &std::path::Path,
) -> Result<(), String> {
    let name = import_name(source);
    let extension = |file: &str| file.rsplit_once('.').map(|(_, ext)| ext.to_lowercase());
    if extension(&name) != extension(path) {
        return Err(format!(
            "{name} isn't the same kind of file as {}",
            shown_path(path)
        ));
    }
    let bytes = std::fs::read(source).map_err(|error| format!("{name}: {error}"))?;
    let mut edit = ContentEdit::new(format!("Replace {} with {name}", shown_path(path)));
    edit.write(path.to_owned(), FileBytes::from(bytes));
    editor.try_apply(edit)
}

/// Puts the files `sources` into the part `spec` of the bundle at `bundle`: the one file of a file
/// part, in place of what it held, or each file of a keyed part under the name it comes with.
pub fn put_parts(
    editor: &mut Editor,
    bundle: &str,
    spec: PartSpec,
    sources: &[std::path::PathBuf],
) -> Result<usize, String> {
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let part = match spec {
        PartSpec::File { name, .. } if sources.len() != 1 => {
            return Err(format!("the {name} of {} is one file", shown_path(bundle)));
        }
        PartSpec::File { name, .. } => name,
        PartSpec::Keyed { folder, .. } => folder,
    };
    let exts = spec.exts();
    let title = file_title(bundle);
    let mut edit = ContentEdit::new(match (spec, sources) {
        (PartSpec::File { .. }, [one]) => {
            format!("Replace the {part} of {title} with {}", import_name(one))
        }
        (_, [one]) => format!("Import {} into the {part} of {title}", import_name(one)),
        (_, many) => format!("Import {} {part} into {title}", many.len()),
    });
    for source in sources {
        let name = import_name(source);
        let Some((stem, ext)) = name.rsplit_once('.').filter(|(_, ext)| exts.contains(ext)) else {
            return Err(format!("{name} isn't a {} file", exts.join(" or ")));
        };
        let key = name_from(stem);
        if matches!(spec, PartSpec::Keyed { .. }) && !valid_name(&key) {
            return Err(format!("{name} names no {part}: {NAME_RULE}"));
        }
        let bytes = std::fs::read(source).map_err(|error| format!("{name}: {error}"))?;
        for other in exts.iter().filter(|other| **other != ext) {
            let stale = format!("{bundle}/{}", spec.named(&key, other));
            if project.files.contains(&stale) {
                edit.remove(stale);
            }
        }
        edit.write(
            format!("{bundle}/{}", spec.named(&key, ext)),
            FileBytes::from(bytes),
        );
    }
    editor.try_apply(edit)?;
    Ok(sources.len())
}

/// Writes the default content of the fixture at `path`, which the engine looks for there.
pub fn make_fixture(editor: &mut Editor, path: &str) -> Result<(), String> {
    let table = table_of(path).ok_or("no type in its name")?;
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let mut edit = ContentEdit::new(format!("Make {}", shown_path(path)));
    make::content_file(&mut edit, project, table, path)?;
    editor.try_apply(edit)?;
    let table = table.to_owned();
    editor.open_row(&table, path);
    Ok(())
}

/// The files picked in the system's dialog, when any were.
pub fn picked_files(dialog: rfd::FileDialog) -> Option<Vec<std::path::PathBuf>> {
    // The desktop portal answers a cancelled pick with no files rather than with none.
    dialog.pick_files().filter(|files| !files.is_empty())
}

fn import_name(source: &std::path::Path) -> String {
    source
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase().replace(' ', "_"))
        .unwrap_or_default()
}

/// Renames the file, bundle or folder at `path` to `name`, keeping its type, and rewrites every
/// reference to it and to what is inside it. A content file the last step made, untouched since,
/// is made under the name instead, in that step, so what it takes from its name takes the new one.
pub fn rename(editor: &mut Editor, path: &str, name: &str) -> Result<String, String> {
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let to = moves::renamed(path, name);
    if to == path {
        return Ok(to);
    }
    if let Some(problem) = name_problem(
        &project.files,
        parent_of(path),
        name,
        name_kind(path),
        Some(path),
    ) {
        return Err(problem);
    }
    let made = format!("New {}", shown_path(path));
    let history = &project.history;
    let just_made = project.files.index().is_content(path)
        && history.applied() == history.steps().len()
        && history
            .steps()
            .last()
            .is_some_and(|step| step.label == made);
    let step = PathMove::new(path, &to);
    match table_of(path).filter(|_| just_made) {
        Some(table) => {
            let table = table.to_owned();
            make_as_named(editor, step, &table, &made)?
        }
        None => moves::apply(
            editor,
            &[step],
            format!("Rename {} to {name}", shown_path(path)),
        )?,
    }
    Ok(to)
}

fn make_as_named(
    editor: &mut Editor,
    step: PathMove,
    table: &str,
    made: &str,
) -> Result<(), String> {
    let project = moves::settled(editor)?;
    let label = format!("New {}", shown_path(&step.to));
    let mut planned = moves::plan(project, std::slice::from_ref(&step), label)?;
    let made_beside: Vec<String> = project
        .history
        .steps()
        .last()
        .into_iter()
        .flat_map(|made| &made.after)
        .map(|change| change.file.clone())
        .filter(|file| *file != step.from && !file.starts_with(&format!("{}/", step.from)))
        .collect();
    for file in made_beside {
        planned.edit.remove(file);
    }
    make::content_file(&mut planned.edit, project, table, &step.to)?;
    editor.try_apply(planned.edit.amending(made))?;
    moves::follow(editor, &[step]);
    Ok(())
}

/// Moves what is at each path into `folder`, numbering a name taken there.
pub fn move_into(editor: &mut Editor, paths: &[String], folder: &str) -> Result<(), String> {
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let mut steps: Vec<PathMove> = Vec::new();
    for path in paths {
        if parent_of(path) == folder {
            continue;
        }
        if folder == path || folder.starts_with(&format!("{path}/")) {
            return Err(format!("{} can't go inside itself", shown_path(path)));
        }
        let arriving: Vec<String> = steps.iter().map(|step| step.to.clone()).collect();
        let to = free_path_among(
            &project.files,
            folder,
            moves::file_name(path),
            paths,
            &arriving,
        );
        steps.push(PathMove::new(path, to));
    }
    if steps.is_empty() {
        return Ok(());
    }
    let into = if folder.is_empty() {
        "the game folder"
    } else {
        folder
    };
    let label = match steps.as_slice() {
        [one] => format!("Move {} into {into}", one.from),
        many => format!("Move {} files into {into}", many.len()),
    };
    moves::apply(editor, &steps, label)
}

/// The files `files` were kept in, written to where they land when what they make up moves from
/// `from` to `to`.
fn write_moved(edit: &mut ContentEdit, files: &[(String, FileBytes)], from: &str, to: &str) {
    let step = PathMove::new(from, to);
    for (file, bytes) in files {
        edit.write(
            step.moved(file).unwrap_or_else(|| file.clone()),
            bytes.clone(),
        );
    }
}

/// Asks what becomes of every use of the files at `paths`, and of everything inside the folders
/// among them, before deleting them.
pub fn delete_dialog(editor: &mut Editor, paths: Vec<String>) {
    let paths: Vec<String> = paths
        .into_iter()
        .filter(|path| match fixed_place(path) {
            Some(reason) => {
                editor
                    .output
                    .say("editor", format!("{} stays: {reason}", shown_path(path)));
                false
            }
            None => true,
        })
        .collect();
    if paths.is_empty() {
        editor.toast("Fixtures stay where the engine finds them");
        return;
    }
    let Ok(project) = moves::settled(editor) else {
        return;
    };
    let index = project.files.index();
    let gone = gone_with(&index, &paths);
    let own: Vec<String> = gone
        .iter()
        .flat_map(|path| project.files.files_of(path))
        .collect();
    let mut groups: Vec<UsageGroup> = Vec::new();
    for path in &gone {
        let target = target_of(project, path).to_owned();
        let usages = match target.as_str() {
            MEDIA => project.usages.of_media(path),
            _ => project.usages.of_content(path),
        };
        for usage in usages {
            if own.contains(&usage.file) {
                continue;
            }
            let UsageKind {
                table: other_table,
                kind,
                entry,
            } = kind_of(project, &usage.file, &usage.pointer);
            let removable = entry.is_some();
            let same_group = |group: &&mut UsageGroup| {
                group.target == target
                    && group.table == other_table
                    && group.removable == removable
                    && if removable {
                        group.kind == kind
                    } else {
                        group.usages.iter().any(|known| known.file == usage.file)
                    }
            };
            match groups.iter_mut().find(same_group) {
                Some(group) => group.usages.push(usage.clone()),
                None => groups.push(UsageGroup {
                    target: target.clone(),
                    table: other_table,
                    kind,
                    usages: vec![usage.clone()],
                    removable,
                    whole_modules: entry == Some(Entry::Module),
                    decision: if removable {
                        UsageDecision::Remove
                    } else {
                        UsageDecision::Repoint(String::new())
                    },
                }),
            }
        }
    }
    editor.dialogs.push(ModalDialog::Delete(DeleteDialog {
        paths,
        gone,
        groups,
        error: None,
    }));
}

/// Every file that goes when `paths` do: each one itself, and what the folders among them hold.
fn gone_with(index: &FolderIndex, paths: &[String]) -> Vec<String> {
    let mut gone = Vec::new();
    for path in paths {
        if index.folders.contains(path) {
            let inside = format!("{path}/");
            gone.extend(
                index
                    .content
                    .values()
                    .flatten()
                    .chain(&index.media)
                    .chain(&index.notes)
                    .filter(|file| file.starts_with(&inside))
                    .cloned(),
            );
        } else {
            gone.push(path.clone());
        }
    }
    gone.sort();
    gone.dedup();
    gone
}

/// What deleting `paths` is called: one file by its name, or how many files go.
pub fn delete_title(dialog: &DeleteDialog) -> String {
    match dialog.paths.as_slice() {
        [one] if dialog.gone.len() == 1 && dialog.gone[0] == *one => {
            format!("Delete {}", file_title(one))
        }
        [one] if dialog.gone.is_empty() => {
            format!("Delete empty folder {}", moves::file_name(one))
        }
        [one] => format!(
            "Delete folder {} and its {} file{}",
            moves::file_name(one),
            dialog.gone.len(),
            words::plural(dialog.gone.len())
        ),
        _ => format!(
            "Delete {} file{}",
            dialog.gone.len(),
            words::plural(dialog.gone.len())
        ),
    }
}

pub struct UsageSummary {
    pub table: String,
    pub field: String,
    pub keys: Vec<String>,
}

pub fn usage_groups(project: &Project, path: &str) -> Vec<UsageSummary> {
    let mut groups: Vec<UsageSummary> = Vec::new();
    for usage in project.usages.of_content(path) {
        let Some(other_key) = content_holding(&usage.file) else {
            continue;
        };
        let other_table = table_of(other_key).unwrap_or_default();
        let field = field_of(project, &usage.file, &usage.pointer);
        match groups
            .iter_mut()
            .find(|group| group.table == other_table && group.field == field)
        {
            Some(group) => {
                if !group.keys.iter().any(|k| k == other_key) {
                    group.keys.push(other_key.to_owned());
                }
            }
            None => groups.push(UsageSummary {
                table: other_table.to_owned(),
                field,
                keys: vec![other_key.to_owned()],
            }),
        }
    }
    groups
}

pub fn field_of(project: &Project, file: &str, pointer: &str) -> String {
    if is_beside(file) {
        let name = file.rsplit('/').next().unwrap_or(file);
        return name
            .split_once('.')
            .map_or(name, |(stem, _)| stem)
            .to_owned();
    }
    let segments: Vec<&str> = pointer.split('/').skip(1).collect();
    if is_scene(file)
        && let Some(row) = project.files.json(file)
        && let Some(kind) = row
            .pointer(&crate::scene_graph::node_pointer(pointer))
            .and_then(TypeTag::of_json)
    {
        return kind;
    }
    if segments.first() == Some(&MODULES)
        && let Some(index) = segments.get(1)
        && let Some(row) = project.files.json(file)
        && let Some(kind) = row
            .pointer(&format!("/{MODULES}/{index}"))
            .and_then(TypeTag::of_json)
    {
        return kind;
    }
    segments
        .first()
        .map(|s| (*s).to_owned())
        .unwrap_or_default()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Entry {
    List,
    Module,
}

struct UsageKind {
    table: String,
    kind: String,
    entry: Option<Entry>,
}

fn kind_of(project: &Project, file: &str, pointer: &str) -> UsageKind {
    let table = content_holding(file)
        .and_then(table_of)
        .unwrap_or("game")
        .to_owned();
    let kind = field_of(project, file, pointer);
    let segments: Vec<&str> = pointer.split('/').skip(1).collect();
    let slot = (!is_beside(file) && !segments.contains(&"nodes"))
        .then(|| list_slot(pointer))
        .flatten();
    let entry = match slot {
        None => None,
        Some(_) if is_scene(file) => Some(Entry::Module),
        Some((parent, index)) if parent.ends_with("/modules") => {
            let any_number = project
                .files
                .json(file)
                .and_then(|row| row.pointer(&parent)?.get(index).cloned())
                .and_then(|module| registry().module_rule(&table, &module))
                == Some(ModuleRule::AnyNumber);
            any_number.then_some(Entry::Module)
        }
        Some(_) => Some(Entry::List),
    };
    UsageKind { table, kind, entry }
}

fn list_slot(pointer: &str) -> Option<(String, usize)> {
    let segments: Vec<&str> = pointer.split('/').skip(1).collect();
    let at = segments
        .iter()
        .rposition(|segment| segment.parse::<usize>().is_ok())?;
    let parent = if at == 0 {
        String::new()
    } else {
        format!("/{}", segments[..at].join("/"))
    };
    Some((parent, segments[at].parse().ok()?))
}

fn usage_detail(project: &Project, file: &str, pointer: &str) -> String {
    let (Some(row), Some(table)) = (
        project.files.json(file),
        content_holding(file).and_then(table_of),
    ) else {
        return String::new();
    };
    let parent = pointer
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("");
    let Some(at) = row.pointer(parent) else {
        return String::new();
    };
    let Some(entry) = at.as_object() else {
        return String::new();
    };
    let root = project.shapes.shape_of(table);
    let shape = schema::at_pointer(root, root, &row, parent).unwrap_or(&Value::Null);
    let mut parts: Vec<String> = Vec::new();
    let tag = schema::tag_of(shape).map(|(field, _)| field);
    for (name, value) in entry {
        if format!("{parent}/{name}") == pointer || tag.as_ref() == Some(name) {
            continue;
        }
        let text = match value {
            Value::Number(number) => {
                let number = number.as_f64().unwrap_or(0.0);
                schema::number_shorthand(root, schema::property(shape, name), number)
                    .unwrap_or_else(|| format!("{name} {number}"))
            }
            Value::String(text) => format!("{name} {text}"),
            Value::Bool(true) => name.clone(),
            _ => continue,
        };
        parts.push(text);
    }
    parts.extend(
        schema::blank_meaning(shape, at)
            .into_iter()
            .map(str::to_owned),
    );
    parts.join(" · ")
}

/// Whether `file` is a scene graph, whose nodes nest in folders' children.
fn is_scene(file: &str) -> bool {
    file.rsplit('/').next() == Some(SCENE_FILE)
}

/// Whether `file` is a part of a bundle that names content in a format of its own, not JSON.
fn is_beside(file: &str) -> bool {
    !file.ends_with(".json")
}

/// What the uses of `path` name: its type when it is a content file, or `MEDIA`.
fn target_of<'p>(project: &Project, path: &'p str) -> &'p str {
    table_of(path)
        .filter(|_| project.files.index().is_content(path))
        .unwrap_or(MEDIA)
}

pub fn delete_body(editor: &mut Editor, ui: &mut Ui, dialog: &mut DeleteDialog) {
    let Some(project) = &editor.project else {
        return;
    };
    let index = project.files.index();
    let total: usize = dialog.groups.iter().map(|group| group.usages.len()).sum();
    let rows: usize = {
        let mut files: Vec<&String> = dialog
            .groups
            .iter()
            .flat_map(|group| group.usages.iter().map(|usage| &usage.file))
            .collect();
        files.sort();
        files.dedup();
        files.len()
    };
    if dialog.groups.is_empty() {
        ui.label(theme::muted("Nothing names it."));
    } else {
        let used = format!(
            "Used in {total} place{} in {rows} file{}. Nothing is deleted until every group has an answer.",
            words::plural(total),
            words::plural(rows)
        );
        ui.label(theme::muted(used));
    }
    let candidates = |target: &str| -> Vec<String> {
        let all: Vec<&String> = match target {
            MEDIA => index.media.iter().collect(),
            table => index.content.get(table).into_iter().flatten().collect(),
        };
        all.into_iter()
            .filter(|path| !dialog.gone.contains(path))
            .cloned()
            .collect()
    };
    let details: Vec<Vec<(String, String)>> = dialog
        .groups
        .iter()
        .map(|group| {
            let mut lines: Vec<(String, String, usize)> = Vec::new();
            for usage in &group.usages {
                let row = content_holding(&usage.file)
                    .unwrap_or(&usage.file)
                    .to_owned();
                let mut detail = usage_detail(project, &usage.file, &usage.pointer);
                let line = if !group.removable {
                    let field = field_of(project, &usage.file, &usage.pointer);
                    detail = if detail.is_empty() {
                        field
                    } else {
                        format!("{field}  {detail}")
                    };
                    (String::new(), detail)
                } else {
                    (row, detail)
                };
                match lines
                    .iter_mut()
                    .find(|(r, d, _)| *r == line.0 && *d == line.1)
                {
                    Some((_, _, n)) => *n += 1,
                    None => lines.push((line.0, line.1, 1)),
                }
            }
            lines
                .into_iter()
                .map(|(row, detail, n)| {
                    if n > 1 {
                        (
                            row,
                            if detail.is_empty() {
                                format!("× {n}")
                            } else {
                                format!("{detail} × {n}")
                            },
                        )
                    } else {
                        (row, detail)
                    }
                })
                .collect()
        })
        .collect();
    ui.add_space(6.0);
    egui::ScrollArea::vertical()
        .max_height(440.0)
        .show(ui, |ui| {
            for (index, group) in dialog.groups.iter_mut().enumerate() {
                let title = group_title(project, group);
                panel::card(false).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let candidates = candidates(&group.target);
                            decision_picker(ui, &project.shapes, index, group, &candidates);
                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    let badge = if group.kind.is_empty() {
                                        &group.table
                                    } else {
                                        &group.kind
                                    };
                                    widgets::badge(ui, badge);
                                    ui.add(egui::Label::new(theme::strong(&title)).truncate());
                                },
                            );
                        });
                    });
                    ui.add_space(2.0);
                    usage_lines(ui, index, &details[index]);
                });
                ui.add_space(4.0);
            }
        });
    let files_removed = dialog.gone.len();
    ui.add_space(4.0);
    let summary = if dialog.groups.is_empty() {
        format!(
            "{files_removed} deleted file{}, one undo step.",
            words::plural(files_removed)
        )
    } else {
        format!(
            "{total} edit{} in {rows} file{} and {files_removed} deleted file{}, one undo step.",
            words::plural(total),
            words::plural(rows),
            words::plural(files_removed)
        )
    };
    ui.label(theme::muted(summary).size(12.5));
    if let Some(error) = &dialog.error {
        ui.colored_label(theme::DANGER, error);
    }
}

fn group_title(project: &Project, group: &UsageGroup) -> String {
    let mut files: Vec<&String> = group.usages.iter().map(|usage| &usage.file).collect();
    files.sort();
    files.dedup();
    let mut kinds: Vec<String> = group
        .usages
        .iter()
        .map(|usage| field_of(project, &usage.file, &usage.pointer))
        .collect();
    kinds.sort();
    kinds.dedup();
    if files.len() != 1 {
        return format!(
            "{} · {} {}s",
            group.kind.replace('_', " "),
            files.len(),
            group.table
        );
    }
    let row = content_holding(files[0])
        .map(|path| file_title(path).to_owned())
        .unwrap_or_else(|| files[0].clone());
    match kinds.as_slice() {
        [kind] if !kind.is_empty() && *kind != group.table => {
            format!("{row} · {}", kind.replace('_', " "))
        }
        _ => row,
    }
}

fn decision_picker(
    ui: &mut Ui,
    shapes: &SchemaShapes,
    index: usize,
    group: &mut UsageGroup,
    candidates: &[String],
) {
    let table = group.target.as_str();
    let mut repoint = matches!(group.decision, UsageDecision::Repoint(_)) || !group.removable;
    if group.removable {
        let remove_label = if group.whole_modules {
            format!("Remove the {} modules", group.kind.replace('_', " "))
        } else {
            format!("Remove {}", usage_phrase(shapes, &group.table, &group.kind))
        };
        let unpackable = table == BlueprintDef::TABLE && group.whole_modules;
        let label = match group.decision {
            UsageDecision::Repoint(_) => "Replace with…".to_owned(),
            UsageDecision::Unpack => "Unpack into its parts".to_owned(),
            _ => remove_label.clone(),
        };
        controls::combo(("decision", index))
            .selected_text(theme::mono(label).size(13.0))
            .show_ui(ui, |ui| {
                if unpackable
                    && ui
                        .selectable_label(
                            group.decision == UsageDecision::Unpack,
                            "Unpack into its parts",
                        )
                        .on_hover_text(
                            "Each placement becomes its parts, placed, and looks exactly the same",
                        )
                        .clicked()
                {
                    group.decision = UsageDecision::Unpack;
                    repoint = false;
                }
                if ui
                    .selectable_label(
                        matches!(group.decision, UsageDecision::Remove),
                        &remove_label,
                    )
                    .clicked()
                {
                    group.decision = UsageDecision::Remove;
                    repoint = false;
                }
                if ui.selectable_label(repoint, "Replace with…").clicked() && !repoint {
                    group.decision = UsageDecision::Repoint(String::new());
                    repoint = true;
                }
            });
    }
    if !repoint {
        return;
    }
    let mut picked = match &group.decision {
        UsageDecision::Repoint(key) => key.clone(),
        _ => String::new(),
    };
    let selected = if picked.is_empty() {
        theme::muted("pick a replacement…").size(13.0)
    } else {
        theme::code(file_title(&picked).to_owned()).size(13.0)
    };
    ui.allocate_ui_with_layout(
        egui::vec2(170.0, ui.spacing().interact_size.y),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            controls::combo(("repoint", index))
                .width(160.0)
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for candidate in candidates {
                        if ui
                            .selectable_label(picked == *candidate, theme::mono(candidate))
                            .on_hover_text(candidate.as_str())
                            .clicked()
                        {
                            picked = candidate.clone();
                        }
                    }
                });
        },
    );
    if !picked.is_empty() && group.decision != UsageDecision::Repoint(picked.clone()) {
        group.decision = UsageDecision::Repoint(picked);
    }
    ui.label(theme::muted("Replace with").size(12.5));
}

fn usage_lines(ui: &mut Ui, index: usize, lines: &[(String, String)]) {
    egui::Grid::new(("usages", index))
        .num_columns(2)
        .spacing([14.0, 2.0])
        .show(ui, |ui| {
            for (row, detail) in lines {
                if row.is_empty() {
                    ui.label(theme::mono(detail).size(12.5).color(theme::TEXT));
                    ui.label("");
                } else {
                    ui.label(theme::mono(row).size(12.5).color(theme::TEXT));
                    ui.label(theme::muted(detail).monospace().size(12.5));
                }
                ui.end_row();
            }
        });
}

fn usage_phrase(shapes: &SchemaShapes, table: &str, field: &str) -> String {
    shapes
        .tables
        .get(table)
        .and_then(|table| schema::usage(schema::property(&table.shape, field)))
        .unwrap_or("the entries")
        .to_owned()
}

pub fn delete(editor: &mut Editor, dialog: &DeleteDialog) -> Result<(), String> {
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    if let Some(group) = dialog
        .groups
        .iter()
        .find(|group| group.decision == UsageDecision::Undecided)
    {
        return Err(format!("decide what happens to {}", group.kind));
    }
    let label = delete_title(dialog);
    let mut edit = ContentEdit::new(label.clone());
    let mut by_file: BTreeMap<String, Vec<(&ContentUsage, &UsageGroup)>> = Default::default();
    for group in &dialog.groups {
        for usage in &group.usages {
            by_file
                .entry(usage.file.clone())
                .or_default()
                .push((usage, group));
        }
    }
    let mut renames: BTreeMap<String, String> = BTreeMap::new();
    let mut repointed: Vec<&str> = Vec::new();
    for (file, entries) in by_file {
        let mut taken: Vec<TakenSlot> = Vec::new();
        let mut taken_from: Vec<&ContentUsage> = Vec::new();
        for (usage, group) in entries {
            match &group.decision {
                UsageDecision::Repoint(to) => {
                    renames.extend(
                        dialog
                            .gone
                            .iter()
                            .filter(|gone| target_of(project, gone) == group.target)
                            .map(|gone| (gone.clone(), to.clone())),
                    );
                    repointed.push(&usage.file);
                }
                UsageDecision::Remove => {
                    taken.extend(TakenSlot::of(&usage.pointer, false));
                    taken_from.push(usage);
                }
                UsageDecision::Unpack => {
                    taken.extend(TakenSlot::of(
                        &scene_graph::node_pointer(&usage.pointer),
                        true,
                    ));
                    taken_from.push(usage);
                }
                UsageDecision::Undecided => {}
            }
        }
        if taken.is_empty() {
            continue;
        }
        let Some(mut value) = project.files.json_owned(&file) else {
            continue;
        };
        let still_named = |usage: &&ContentUsage| {
            usage
                .named_in(&value)
                .is_some_and(|named| dialog.gone.contains(&named))
        };
        if !taken_from.iter().all(still_named) {
            return Err(format!(
                "{} changed since what it names was read; delete again once the folder loads",
                shown_path(&file)
            ));
        }
        taken.sort_by(|a, b| b.steps.cmp(&a.steps).then(a.unpack.cmp(&b.unpack)));
        taken.dedup_by(|later, first| later.steps == first.steps);
        for slot in taken {
            if slot.unpack {
                unpack_entry(project, &file, &mut value, &slot.parent, slot.index);
            } else {
                remove_entry(&mut value, &slot.parent, slot.index);
            }
        }
        edit.write_json(file, &value);
    }
    moves::rewrite_renamed(project, &mut edit, repointed, &renames)?;
    for path in &dialog.paths {
        let inside = folder_entry(path);
        let entries: Vec<String> = if project.files.index().folders.contains(path) {
            project.files.paths_under(&inside).cloned().collect()
        } else {
            project.files.files_of(path)
        };
        for entry in entries {
            edit.remove(entry);
        }
        moves::keep_folder(&project.files, &mut edit, parent_of(path));
    }
    project
        .check(&edit)
        .map_err(|refusal| format!("{}: {}", refusal.place(), refusal.sentence()))?;
    editor.try_apply(edit)?;
    for path in &dialog.gone {
        close_tabs(editor, path);
    }
    let gone = |key: &String| {
        dialog.gone.contains(key)
            || dialog
                .paths
                .iter()
                .any(|path| key.starts_with(&format!("{path}/")))
    };
    editor.selection.keys.retain(|key| !gone(key));
    editor.toast(label.replacen("Delete", "Deleted", 1));
    Ok(())
}

/// Puts in place of the placement at entry `index` of the list at `parent` of the scene file
/// `file` the nodes it places, as nodes of that scene.
fn unpack_entry(project: &Project, file: &str, value: &mut Value, parent: &str, index: usize) {
    let Some(bundle) = file.strip_suffix(&format!("/{SCENE_FILE}")) else {
        return;
    };
    let node = format!("{parent}/{index}");
    let reader = project.reader();
    let unpacked = area::nodes::bundle_in(project, bundle).and_then(|read| {
        let placement = area::nodes::placement_of(area::nodes::node(read.nodes(), &node)?)?;
        let place = placement::AreaPlacement::of(project, &node, placement)?;
        let nodes = placement::unpacked(&reader, &place)?;
        area::nodes::CopiedNodes::of(&reader, &nodes)
    });
    let Some(parts) = unpacked.and_then(|copied| copied.into_json().as_array().cloned()) else {
        return;
    };
    let Some(list) = value.pointer_mut(parent).and_then(Value::as_array_mut) else {
        return;
    };
    if index >= list.len() {
        return;
    }
    list.remove(index);
    for (offset, part) in parts.into_iter().enumerate() {
        list.insert(index + offset, part);
    }
}

fn remove_entry(value: &mut Value, parent: &str, index: usize) {
    if let Some(list) = value.pointer_mut(parent).and_then(Value::as_array_mut)
        && index < list.len()
    {
        list.remove(index);
    }
}

/// A list entry a delete takes out of a file: removed, or unpacked when it places a blueprint.
struct TakenSlot {
    parent: String,
    index: usize,
    unpack: bool,
    steps: Vec<PointerStep>,
}

impl TakenSlot {
    fn of(pointer: &str, unpack: bool) -> Option<TakenSlot> {
        let (parent, index) = list_slot(pointer)?;
        let steps = format!("{parent}/{index}")
            .split('/')
            .skip(1)
            .map(|step| match step.parse() {
                Ok(index) => PointerStep::Index(index),
                Err(_) => PointerStep::Key(step.to_owned()),
            })
            .collect();
        Some(TakenSlot {
            parent,
            index,
            unpack,
            steps,
        })
    }
}

/// A step of a JSON pointer, its list indices ordered as numbers, so that taking entries out in
/// descending order never shifts one still to be taken.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum PointerStep {
    Key(String),
    Index(usize),
}

pub fn close_tabs(editor: &mut Editor, path: &str) {
    let gone: Vec<EditorTab> = editor
        .dock
        .iter_all_tabs()
        .map(|(_, tab)| tab.clone())
        .filter(|tab| {
            tab.row().is_some_and(|(_, key)| key == path)
                || *tab
                    == EditorTab::Asset {
                        path: path.to_owned(),
                    }
        })
        .collect();
    for tab in gone {
        if let Some(path) = editor.dock.find_tab(&tab) {
            editor.dock.remove_tab(path);
        }
    }
}

/// The events the scripts among `modules`, a row's modules as its file lists them, answer.
fn answered_by(project: &Project, modules: &[Value]) -> HashSet<&'static str> {
    let reader = project.reader();
    modules
        .iter()
        .filter_map(|module| reader.from_json::<ListedScriptModule>(module).ok())
        .filter_map(|ListedScriptModule::Script(module)| reader.path_of(module.script))
        .filter_map(|path| project.read::<ScriptDef>(path).ok())
        .flat_map(|script| {
            script
                .nodes
                .values()
                .filter_map(script_nodes::event_name)
                .collect::<Vec<_>>()
        })
        .collect()
}
