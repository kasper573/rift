use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use ui::egui;
use ui::input;
use ui::menu;
use ui::theme;
use ui::words;

use engine::core::content::ContentFile;
use engine::core::content::{SETTINGS_FILE, name_of, parent_of, table_of};
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::registry;
use ui::egui::{Color32, Ui};
use ui::glyph::Mark;

use super::browser::{self, BrowserAction, RowMenu};

use super::tree::{self, Narrowing, TreeDrop, TreeRow, TreeSource, TreeState};
use super::widgets::{self, KindMark};
use crate::app::{BottomTab, Editor, ModalDialog, PickRowDialog, PickRowThen};
use crate::icons::{self, IconTone, Preview};
use crate::import;
use crate::keymap::EditorKey;
use crate::moves::{file_name, fixed_place};
use crate::picker::Dragged;
use crate::project::Project;
use crate::project::files::{FolderIndex, content_holding};
use crate::rows;
use crate::tabs::EditorTab;
use crate::tabs::shown_path;
use crate::thumbs::Thumbnails;
use crate::user::{TreeView, UserFolder};

/// The content panel's tree of the game folder, and what it was asked to do.
#[derive(Default)]
pub struct ContentTree {
    pub search: String,
    pub focus_search: bool,
    pub only_unsaved: bool,
    pub only_unused: bool,
    pub tree: TreeState,
    /// The type New made last, which Ctrl N makes again.
    pub last_new: Option<String>,
    /// What Ctrl C copied, which Paste copies into a folder.
    pub copied: Vec<String>,
    /// The file of the tab last revealed, so a tab is revealed once each time it becomes active.
    revealed: Option<String>,
    /// What the tree lists, kept until what it was found from changes: walking every file each
    /// frame costs more than drawing the tree.
    listing: Arc<Mutex<Option<Listing>>>,
}

struct Listing {
    from: ListedFrom,
    /// While anything narrows the tree: every row shown, and folders above them.
    shown: Option<Arc<HashSet<String>>>,
    /// The rows of each folder listed so far, in order.
    children: HashMap<String, Vec<String>>,
}

/// Everything what the tree lists depends on.
#[derive(PartialEq)]
struct ListedFrom {
    files: u64,
    generation: u64,
    unsaved: Arc<[String]>,
    needle: String,
    only_unsaved: bool,
    only_unused: bool,
    hidden_kinds: BTreeSet<String>,
    hide_empty_folders: bool,
}

/// The folder new things land in: the folder selected in the tree, or the one holding what is
/// selected, or the top of the game folder.
pub fn current_folder(editor: &Editor) -> String {
    let Some(project) = &editor.project else {
        return String::new();
    };
    match editor.content_tree.tree.selected().first() {
        Some(path) if project.files.index().folders.contains(path) => path.clone(),
        Some(path) => parent_of(path).to_owned(),
        None => String::new(),
    }
}

pub fn focus_search(editor: &mut Editor) {
    editor.content_tree.focus_search = true;
}

/// Opens the folders above `path`, selects it and scrolls it into view.
pub fn reveal(editor: &mut Editor, path: &str) {
    editor.content_tree.tree.reveal(path);
}

/// Reveals `path` with its name in an edit box, ready to type over.
pub fn begin_rename(editor: &mut Editor, path: &str) {
    editor.content_tree.tree.begin_rename(path, name_of(path));
}

/// Asks which folder the files at `paths` go into.
pub fn move_to_dialog(editor: &mut Editor, paths: Vec<String>) {
    let Some(project) = &editor.project else {
        return;
    };
    if let Some((path, reason)) = paths
        .iter()
        .find_map(|path| fixed_place(path).map(|reason| (path, reason)))
    {
        editor.toast(format!("{} stays: {reason}", shown_path(path)));
        return;
    }
    let options: Vec<String> = std::iter::once(String::new())
        .chain(project.files.index().folders.iter().cloned())
        .filter(|folder| {
            !paths
                .iter()
                .any(|path| folder == path || folder.starts_with(&format!("{path}/")))
        })
        .collect();
    let title = match paths.as_slice() {
        [one] => format!("Move {} to…", file_name(one)),
        many => format!("Move {} files to…", many.len()),
    };
    editor.dialogs.push(ModalDialog::PickRow(PickRowDialog {
        title,
        table: String::new(),
        search: String::new(),
        picked: None,
        options,
        then: PickRowThen::MoveInto { paths },
    }));
}

/// Asks which type of content file to make in `folder`, among every type a game may add.
pub fn new_of_type_dialog(editor: &mut Editor, folder: String) {
    let registry = registry();
    let options: Vec<String> = editor
        .shapes
        .order
        .iter()
        .filter(|table| !registry.closed(table))
        .cloned()
        .collect();
    editor.dialogs.push(ModalDialog::PickRow(PickRowDialog {
        title: "New content file".to_owned(),
        table: String::new(),
        search: String::new(),
        picked: None,
        options,
        then: PickRowThen::NewFile { folder },
    }));
}

/// Makes a content file of `table` in `folder`, as New ▸ does: no question asked, its name ready
/// to type over.
pub fn new_file(editor: &mut Editor, folder: &str, table: &str) {
    match rows::new_file(editor, folder, table) {
        Ok(path) => {
            editor.content_tree.last_new = Some(table.to_owned());
            editor.toast(format!(
                "Made {path} with the default {table}; Ctrl Z takes it back"
            ));
            begin_rename(editor, &path);
        }
        Err(error) => editor.toast(error),
    }
}

/// The rows the tree lists inside `folder` ("" for the top) as it would show them now, with the
/// search and the user's Show ▾ applied.
pub fn listed(editor: &Editor, folder: &str) -> Vec<(String, TreeRow)> {
    let Some(project) = &editor.project else {
        return Vec::new();
    };
    let view = editor
        .settings
        .content_trees
        .get(&UserFolder::folder_key(&project.root))
        .cloned()
        .unwrap_or_default();
    let source = GameFolder::new(project, &editor.content_tree, &view);
    source
        .children(folder)
        .into_iter()
        .map(|path| {
            let row = source.row(&path);
            (path, row)
        })
        .collect()
}

pub fn show(editor: &mut Editor, ui: &mut Ui) {
    let Some(project) = &editor.project else {
        return;
    };
    let folder_key = UserFolder::folder_key(&project.root);
    reveal_active_tab(editor);
    let open_first = filters(editor, ui, &folder_key);
    ui.add_space(4.0);
    let Editor {
        project,
        content_tree,
        settings,
        form,
        ..
    } = editor;
    let project = project.as_ref().expect("a project");
    let view = settings.content_trees.entry(folder_key).or_default();
    let source =
        GameFolder::new(project, content_tree, view).with_previews(ui.ctx(), &mut form.thumbnails);
    let first_hit = open_first.then(|| first_file(&source, "")).flatten();
    let mut open = view.open.clone();
    let (response, changed) = egui::ScrollArea::vertical()
        .id_salt("content tree")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            tree::show(
                ui,
                egui::Id::new("content tree"),
                &mut content_tree.tree,
                &source,
                &mut open,
            )
        })
        .inner;
    let hovered = ui.ui_contains_pointer();
    drop(source);
    if changed {
        view.open = open;
        editor.save_settings();
    }
    if let Some(hit) = first_hit {
        open_path(editor, &hit, false);
        reveal(editor, &hit);
    }
    if let Some(selected) = &response.selected
        && let [one] = selected.as_slice()
    {
        open_path(editor, one, false);
    }
    for path in &response.activated {
        open_path(editor, path, true);
    }
    if let Some((paths, drop)) = response.dropped
        && let Err(error) = rows::move_into(editor, &paths, &drop.folder)
    {
        editor.toast(error);
    }
    if let Some((path, name)) = response.renamed
        && name != name_of(&path)
    {
        match rows::rename(editor, &path, &name) {
            Ok(renamed) => reveal(editor, &renamed),
            Err(error) => {
                editor.toast(error);
                editor.content_tree.tree.begin_rename(&path, &name);
            }
        }
    }
    if let Some(action) = response.action {
        act(editor, action);
    }
    if (response.focused || hovered) && !ui.ctx().text_edit_focused() {
        keys(editor, ui);
    }
}

/// What the tree's menus and keys ask for.
enum TreeAction {
    New { folder: String, table: String },
    OtherType(String),
    NewFolder(String),
    NewNote(String),
    ImportFiles(String),
    ImportFolder(String),
    File(String, BrowserAction),
    ReplaceWith(String),
    MoveTo(Vec<String>),
    Delete(Vec<String>),
    Rename(String),
    ShowInFileManager(String),
    Paste(String),
    OpenAll,
    CloseAll,
    ShowGameFolder,
    MakeFixture(String),
}

fn act(editor: &mut Editor, action: TreeAction) {
    match action {
        TreeAction::New { folder, table } => new_file(editor, &folder, &table),
        TreeAction::OtherType(folder) => new_of_type_dialog(editor, folder),
        TreeAction::NewFolder(parent) => match rows::new_folder(editor, &parent) {
            Ok(path) => begin_rename(editor, &path),
            Err(error) => editor.toast(error),
        },
        TreeAction::NewNote(folder) => match rows::new_note(editor, &folder) {
            Ok(path) => {
                open_path(editor, &path, false);
                begin_rename(editor, &path);
            }
            Err(error) => editor.toast(error),
        },
        TreeAction::ImportFiles(folder) => {
            if let Some(sources) =
                rows::picked_files(rfd::FileDialog::new().set_title("Import files"))
            {
                import::begin(editor, folder, sources);
            }
        }
        TreeAction::ImportFolder(folder) => {
            if let Some(source) = rfd::FileDialog::new()
                .set_title("Import a folder")
                .pick_folder()
            {
                import::begin(editor, folder, vec![source]);
            }
        }
        TreeAction::File(path, action) => {
            let content = editor
                .project
                .as_ref()
                .is_some_and(|project| project.files.index().is_content(&path));
            match table_of(&path) {
                Some(table) if content => {
                    let table = table.to_owned();
                    browser::act(editor, &table, &path, action);
                }
                _ => media_act(editor, &path, action),
            }
        }
        TreeAction::ReplaceWith(path) => {
            let Some(source) = rfd::FileDialog::new()
                .set_title(format!("Replace {} with…", shown_path(&path)))
                .pick_file()
            else {
                return;
            };
            if let Err(error) = rows::replace_media(editor, &path, &source) {
                editor.toast(error);
            }
        }
        TreeAction::MoveTo(paths) => move_to_dialog(editor, paths),
        TreeAction::Delete(paths) => rows::delete_dialog(editor, paths),
        TreeAction::Rename(path) => match fixed_place(&path) {
            Some(reason) => editor.toast(format!("{} stays: {reason}", shown_path(&path))),
            None => begin_rename(editor, &path),
        },
        TreeAction::ShowInFileManager(path) => {
            if let Some(project) = &editor.project {
                let _ = opener::open(project.root.join(&path));
            }
        }
        TreeAction::Paste(folder) => {
            let copied = editor.content_tree.copied.clone();
            let label = format!("Paste {} into {}", copied.len(), folder_title(&folder));
            match rows::copy_into(editor, &copied, &folder, label) {
                Ok(copies) => editor.content_tree.tree.select(copies),
                Err(error) => editor.toast(error),
            }
        }
        TreeAction::OpenAll | TreeAction::CloseAll => {
            let Some(project) = &editor.project else {
                return;
            };
            let open = matches!(action, TreeAction::OpenAll);
            let folders: Vec<String> = project.files.index().folders.iter().cloned().collect();
            let folder_key = UserFolder::folder_key(&project.root);
            editor.content_tree.tree.set_open(folders.clone(), open);
            let view = editor.settings.content_trees.entry(folder_key).or_default();
            view.open = if open {
                folders.into_iter().collect()
            } else {
                BTreeSet::new()
            };
            editor.save_settings();
        }
        TreeAction::ShowGameFolder => {
            if let Some(project) = &editor.project {
                let _ = opener::open(&project.root);
            }
        }
        TreeAction::MakeFixture(path) => {
            if let Err(error) = rows::make_fixture(editor, &path) {
                editor.toast(error);
            }
        }
    }
}

/// What a content file's menu does to media and notes, which have no table.
fn media_act(editor: &mut Editor, path: &str, action: BrowserAction) {
    match action {
        BrowserAction::Open | BrowserAction::OpenTab => open_path(editor, path, true),
        BrowserAction::Usages => {
            open_path(editor, path, false);
            editor.panels.bottom = BottomTab::UsedBy;
        }
        BrowserAction::CopyPath => editor.copy_text(path),
        BrowserAction::Rename => begin_rename(editor, path),
        BrowserAction::Duplicate => {
            let label = format!("Duplicate {}", shown_path(path));
            match rows::copy_into(editor, &[path.to_owned()], parent_of(path), label) {
                Ok(copies) => editor.content_tree.tree.select(copies),
                Err(error) => editor.toast(error),
            }
        }
        BrowserAction::MoveTo => move_to_dialog(editor, vec![path.to_owned()]),
        BrowserAction::Delete => rows::delete_dialog(editor, vec![path.to_owned()]),
        BrowserAction::ShowInTree => reveal(editor, path),
        BrowserAction::OpenTable => {}
    }
}

/// Opens what is at `path` in its tab: a preview while browsing, kept when `keep`.
fn open_path(editor: &mut Editor, path: &str, keep: bool) {
    let Some(project) = &editor.project else {
        return;
    };
    let index = project.files.index();
    let tab = if path == SETTINGS_FILE {
        EditorTab::Game
    } else if index.is_content(path) {
        let table = table_of(path).unwrap_or_default();
        editor.select(crate::project::RowSelection::row(table, path));
        EditorTab::for_row(table, path)
    } else if index.media.contains(path) || index.notes.contains(path) {
        editor.select(crate::project::RowSelection::row("asset", path));
        EditorTab::Asset {
            path: path.to_owned(),
        }
    } else {
        return;
    };
    editor.content_tree.revealed = Some(path.to_owned());
    if keep {
        editor.open_tab(tab);
    } else {
        editor.preview_tab(tab);
    }
}

fn keys(editor: &mut Editor, ui: &mut Ui) {
    let selected = editor.content_tree.tree.selected().to_vec();
    let folder = current_folder(editor);
    let pressed = ui.input_mut(|i| {
        [
            EditorKey::Rename,
            EditorKey::Duplicate,
            EditorKey::Delete,
            EditorKey::CopyPath,
            EditorKey::Copy,
            EditorKey::Paste,
            EditorKey::NewAgain,
            EditorKey::Usages,
        ]
        .into_iter()
        .find(|key| key.consume(i))
    });
    let action = match (pressed, selected.as_slice()) {
        (Some(EditorKey::Rename), [one]) => TreeAction::Rename(one.clone()),
        (Some(EditorKey::Duplicate), [one]) => {
            TreeAction::File(one.clone(), BrowserAction::Duplicate)
        }
        (Some(EditorKey::Usages), [one]) => TreeAction::File(one.clone(), BrowserAction::Usages),
        (Some(EditorKey::CopyPath), [one]) => {
            TreeAction::File(one.clone(), BrowserAction::CopyPath)
        }
        (Some(EditorKey::Delete), many) if !many.is_empty() => TreeAction::Delete(many.to_vec()),
        (Some(EditorKey::Copy), many) if !many.is_empty() => {
            editor.content_tree.copied = many.to_vec();
            editor.toast(format!(
                "Copied {} file{}; Ctrl V copies them into a folder",
                many.len(),
                words::plural(many.len())
            ));
            return;
        }
        (Some(EditorKey::Paste), _) if !editor.content_tree.copied.is_empty() => {
            TreeAction::Paste(folder)
        }
        (Some(EditorKey::NewAgain), _) => match editor.content_tree.last_new.clone() {
            Some(table) => TreeAction::New { folder, table },
            None => TreeAction::OtherType(folder),
        },
        _ => return,
    };
    act(editor, action);
}

/// The search field and Show ▾. Answers whether Enter was pressed in the search.
fn filters(editor: &mut Editor, ui: &mut Ui, folder_key: &str) -> bool {
    let kinds = kinds(editor);
    let Editor {
        content_tree: tree,
        settings,
        ..
    } = editor;
    let view = settings
        .content_trees
        .entry(folder_key.to_owned())
        .or_default();
    let ContentTree {
        search,
        focus_search,
        only_unsaved,
        only_unused,
        ..
    } = tree;
    let (entered, changed) = tree::filter_row(
        ui,
        |ui| {
            let response = input::search(
                ui,
                search,
                "Search files and paths",
                Some(&EditorKey::Search.name()),
            );
            if std::mem::take(focus_search) {
                response.request_focus();
            }
            if response.has_focus() && ui.input(|i| EditorKey::Cancel.pressed(i)) {
                search.clear();
            }
            response.lost_focus() && ui.input(|i| EditorKey::Confirm.pressed(i))
        },
        |ui| {
            tree::show_menu(
                ui,
                view,
                &kinds,
                &kind_mark,
                &mut [
                    Narrowing {
                        label: "Only unsaved files",
                        on: only_unsaved,
                    },
                    Narrowing {
                        label: "Only unused files",
                        on: only_unused,
                    },
                ],
            )
        },
    );
    if changed {
        editor.save_settings();
    }
    entered
}

/// The first file the tree lists inside `folder`, at any depth.
fn first_file(source: &GameFolder, folder: &str) -> Option<String> {
    source.children(folder).into_iter().find_map(|path| {
        if source.is_folder(&path) {
            first_file(source, &path)
        } else {
            Some(path)
        }
    })
}

const MEDIA_KIND: &str = "media";
const NOTES_KIND: &str = "notes";
const OTHER_KIND: &str = "other files";

/// Every kind of file Show ▾ offers: every type of content the game can hold, media, notes and
/// files the game never reads.
fn kind_mark(kind: &str) -> KindMark<'_> {
    KindMark::Icon(match kind {
        MEDIA_KIND => icons::IMAGE,
        NOTES_KIND => icons::NOTE,
        OTHER_KIND => icons::FILE,
        table => icons::of_table(table),
    })
}

fn kinds(editor: &Editor) -> Vec<String> {
    let registry = registry();
    editor
        .shapes
        .order
        .iter()
        .filter(|table| !registry.closed(table))
        .cloned()
        .chain([
            MEDIA_KIND.to_owned(),
            NOTES_KIND.to_owned(),
            OTHER_KIND.to_owned(),
        ])
        .collect()
}

pub fn folder_title(folder: &str) -> &str {
    if folder.is_empty() {
        "the game folder"
    } else {
        folder
    }
}

/// Reveals the file of the active tab, once each time a tab becomes active.
fn reveal_active_tab(editor: &mut Editor) {
    let Some(tab) = super::active_tab(editor) else {
        return;
    };
    let path = match &tab {
        EditorTab::Game => Some(SETTINGS_FILE.to_owned()),
        EditorTab::Asset { path } => Some(path.clone()),
        other => other.row().map(|(_, key)| key),
    };
    let Some(path) = path else {
        return;
    };
    if editor.content_tree.revealed.as_ref() == Some(&path) {
        return;
    }
    editor.content_tree.revealed = Some(path.clone());
    reveal(editor, &path);
}

/// The game folder as the tree shows it.
struct GameFolder<'a> {
    project: &'a Project,
    index: Arc<FolderIndex>,
    /// Fixtures the engine looks for that aren't there, and the folders above them that aren't
    /// either, by the folder they belong in.
    missing: BTreeMap<String, BTreeSet<String>>,
    unsaved: HashSet<String>,
    problems: HashMap<String, String>,
    listing: Arc<Mutex<Option<Listing>>>,
    shown: Option<Arc<HashSet<String>>>,
    narrowed: bool,
    filtered: bool,
    /// What the search looks for, lowercased.
    needle: String,
    copied: bool,
    last_new: Option<String>,
    previews: Option<RefCell<Previews<'a>>>,
}

/// What the tree draws the pictures of its files from, decoding a few each frame.
struct Previews<'a> {
    ctx: egui::Context,
    thumbnails: &'a mut Thumbnails,
    budget: u32,
}

impl<'a> GameFolder<'a> {
    /// Draws the pictures of files small enough to stand in for their icons.
    fn with_previews(mut self, ctx: &egui::Context, thumbnails: &'a mut Thumbnails) -> Self {
        self.previews = Some(RefCell::new(Previews {
            ctx: ctx.clone(),
            thumbnails,
            budget: icons::DECODES_PER_FRAME,
        }));
        self
    }

    fn preview(&self, path: &str) -> Option<Preview> {
        let mut previews = self.previews.as_ref()?.borrow_mut();
        let Previews {
            ctx,
            thumbnails,
            budget,
        } = &mut *previews;
        icons::preview(ctx, thumbnails, self.project, path, budget)
    }

    fn new(project: &'a Project, tree: &ContentTree, view: &TreeView) -> GameFolder<'a> {
        let index = project.files.index();
        let mut missing: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for fixture in registry().fixture_paths() {
            if index.is_content(fixture) {
                continue;
            }
            let mut child = fixture.to_owned();
            loop {
                let parent = parent_of(&child).to_owned();
                missing.entry(parent.clone()).or_default().insert(child);
                if parent.is_empty() || index.folders.contains(&parent) {
                    break;
                }
                child = parent;
            }
        }
        let unsaved_paths = project.unsaved();
        let mut unsaved = HashSet::new();
        for entry in unsaved_paths.iter() {
            let row = row_of(entry);
            above(&row, &mut |folder| {
                unsaved.insert(folder.to_owned());
            });
            unsaved.insert(row);
        }
        let mut problems: HashMap<String, String> = HashMap::new();
        for error in project.report.iter().flat_map(|report| &report.errors) {
            let row = row_of(&error.file);
            let text = error.problem.to_string();
            above(&row, &mut |folder| {
                problems
                    .entry(folder.to_owned())
                    .or_insert_with(|| format!("a problem inside: {text}"));
            });
            problems.entry(row).or_insert(text);
        }
        for unknown in &index.unknown {
            problems
                .entry(unknown.clone())
                .or_insert_with(|| "the game holds no kind of file by this name".to_owned());
        }
        let needle = tree.search.trim().to_lowercase();
        let narrowed = !needle.is_empty() || tree.only_unsaved || tree.only_unused;
        let filtered = narrowed || !view.hidden_kinds.is_empty() || view.hide_empty_folders;
        let mut folder = GameFolder {
            project,
            index,
            missing,
            unsaved,
            problems,
            listing: tree.listing.clone(),
            shown: None,
            narrowed,
            filtered,
            needle: needle.clone(),
            copied: !tree.copied.is_empty(),
            last_new: tree.last_new.clone(),
            previews: None,
        };
        let from = ListedFrom {
            files: project.files.revision(),
            generation: project.generation,
            unsaved: unsaved_paths,
            needle: needle.clone(),
            only_unsaved: tree.only_unsaved,
            only_unused: tree.only_unused,
            hidden_kinds: view.hidden_kinds.clone(),
            hide_empty_folders: view.hide_empty_folders,
        };
        folder.shown = {
            let mut listing = tree.listing.lock().unwrap_or_else(PoisonError::into_inner);
            let listing = match &mut *listing {
                Some(listing) if listing.from == from => listing,
                stale => stale.insert(Listing {
                    from,
                    shown: filtered.then(|| Arc::new(folder.find_shown(tree, view, &needle))),
                    children: HashMap::new(),
                }),
            };
            listing.shown.clone()
        };
        folder
    }

    fn find_children(&self, id: &str) -> Vec<String> {
        let mut children: Vec<String> = self
            .index
            .children_of(id)
            .map(str::to_owned)
            .chain(self.missing.get(id).into_iter().flatten().cloned())
            .filter(|path| self.shown.as_ref().is_none_or(|shown| shown.contains(path)))
            .collect::<BTreeSet<String>>()
            .into_iter()
            .collect();
        children.sort_by_cached_key(|path| (!self.is_folder(path), file_name(path).to_lowercase()));
        children
    }

    /// The rows left once the tree is narrowed and its kinds chosen: what passes, the folders
    /// above it, and every other folder unless it is hidden for showing nothing.
    fn find_shown(&self, tree: &ContentTree, view: &TreeView, needle: &str) -> HashSet<String> {
        let unused = &self.project.unused;
        let mut shown = HashSet::new();
        let mut leaves: Vec<&str> = Vec::new();
        for held in self.index.children.values().chain(self.missing.values()) {
            leaves.extend(
                held.iter()
                    .map(String::as_str)
                    .filter(|path| !self.is_folder(path)),
            );
        }
        for path in leaves {
            let kind = self.kind_of(path);
            let passes = !view.hidden_kinds.contains(kind)
                && (needle.is_empty() || path.to_lowercase().contains(needle))
                && (!tree.only_unsaved || self.unsaved.contains(path))
                && (!tree.only_unused || unused.row(path).is_some() || unused.has_asset(path));
            if passes {
                above(path, &mut |folder| {
                    shown.insert(folder.to_owned());
                });
                shown.insert(path.to_owned());
            }
        }
        if !self.narrowed && !view.hide_empty_folders {
            shown.extend(self.index.folders.iter().cloned());
        }
        shown
    }

    fn is_missing(&self, path: &str) -> bool {
        self.missing
            .get(parent_of(path))
            .is_some_and(|held| held.contains(path))
            && !self.index.folders.contains(path)
    }

    /// The kind Show ▾ lists a file under: its type, media, notes or other files.
    fn kind_of<'p>(&self, path: &'p str) -> &'p str {
        if self.index.media.contains(path) {
            MEDIA_KIND
        } else if self.index.notes.contains(path) {
            NOTES_KIND
        } else if self.index.other.contains(path) {
            OTHER_KIND
        } else {
            table_of(path).unwrap_or_default()
        }
    }

    fn folder_menu(&self, ui: &mut Ui, folder: &str) -> Option<TreeAction> {
        let mut picked = menu::submenu(ui, "New", |ui| self.new_menu(ui, folder)).flatten();
        if !folder.is_empty() {
            ui.separator();
            let fixed = fixed_place(folder);
            let rename = ui
                .add_enabled(
                    fixed.is_none(),
                    widgets::shortcut_button("Rename", EditorKey::Rename),
                )
                .on_disabled_hover_text(fixed.unwrap_or_default());
            if rename.clicked() {
                picked = Some(TreeAction::Rename(folder.to_owned()));
            }
            let move_to = ui
                .add_enabled(fixed.is_none(), egui::Button::new("Move to…"))
                .on_disabled_hover_text(fixed.unwrap_or_default());
            if move_to.clicked() {
                picked = Some(TreeAction::MoveTo(vec![folder.to_owned()]));
            }
            if ui.button("Show in the file manager").clicked() {
                picked = Some(TreeAction::ShowInFileManager(folder.to_owned()));
            }
            ui.separator();
            let count = self.index.count_of(folder);
            let delete = ui
                .add_enabled(
                    fixed.is_none(),
                    widgets::shortcut_button(
                        match count {
                            0 => "Delete empty folder".to_owned(),
                            count => format!(
                                "Delete folder and its {count} file{}",
                                words::plural(count)
                            ),
                        },
                        EditorKey::Delete,
                    ),
                )
                .on_disabled_hover_text(fixed.unwrap_or_default());
            if delete.clicked() {
                picked = Some(TreeAction::Delete(vec![folder.to_owned()]));
            }
        }
        if picked.is_some() {
            ui.close();
        }
        picked
    }

    /// New ▸: the types the folder already holds first, then any other, a folder, media and a
    /// note.
    fn new_menu(&self, ui: &mut Ui, folder: &str) -> Option<TreeAction> {
        let held: BTreeSet<&str> = self
            .index
            .children_of(folder)
            .filter(|path| self.index.is_content(path))
            .filter_map(table_of)
            .filter(|table| !registry().closed(table))
            .collect();
        let mut picked = None;
        for table in &held {
            let again = (self.last_new.as_deref() == Some(*table)).then_some(EditorKey::NewAgain);
            if widgets::kind_button(ui, icons::of_table(table), table, again).clicked() {
                picked = Some(TreeAction::New {
                    folder: folder.to_owned(),
                    table: (*table).to_owned(),
                });
            }
        }
        if ui.button("Other type…").clicked() {
            picked = Some(TreeAction::OtherType(folder.to_owned()));
        }
        ui.separator();
        if widgets::kind_button(ui, Mark::Folder.into(), "New folder", None).clicked() {
            picked = Some(TreeAction::NewFolder(folder.to_owned()));
        }
        if widgets::kind_button(ui, icons::FILE, "Import files…", None).clicked() {
            picked = Some(TreeAction::ImportFiles(folder.to_owned()));
        }
        if widgets::kind_button(ui, Mark::Folder.into(), "Import a folder…", None).clicked() {
            picked = Some(TreeAction::ImportFolder(folder.to_owned()));
        }
        if widgets::kind_button(ui, icons::NOTE, "New note", None).clicked() {
            picked = Some(TreeAction::NewNote(folder.to_owned()));
        }
        if picked.is_some() {
            ui.close();
        }
        picked
    }

    fn media_menu(&self, ui: &mut Ui, path: &str) -> Option<TreeAction> {
        let mut picked = None;
        let file = |action| Some(TreeAction::File(path.to_owned(), action));
        if ui
            .add(widgets::shortcut_button("Open", EditorKey::Confirm))
            .clicked()
        {
            picked = file(BrowserAction::Open);
        }
        if self.index.media.contains(path) && ui.button("Used by").clicked() {
            picked = file(BrowserAction::Usages);
        }
        if ui
            .add(widgets::shortcut_button("Copy path", EditorKey::CopyPath))
            .clicked()
        {
            picked = file(BrowserAction::CopyPath);
        }
        ui.separator();
        if ui
            .add(widgets::shortcut_button("Rename", EditorKey::Rename))
            .clicked()
        {
            picked = Some(TreeAction::Rename(path.to_owned()));
        }
        if ui.button("Move to…").clicked() {
            picked = Some(TreeAction::MoveTo(vec![path.to_owned()]));
        }
        if self.index.media.contains(path) && ui.button("Replace with…").clicked() {
            picked = Some(TreeAction::ReplaceWith(path.to_owned()));
        }
        ui.separator();
        if ui
            .add(widgets::shortcut_button("Delete…", EditorKey::Delete))
            .clicked()
        {
            picked = Some(TreeAction::Delete(vec![path.to_owned()]));
        }
        if picked.is_some() {
            ui.close();
        }
        picked
    }

    fn empty_space_menu(&self, ui: &mut Ui) -> Option<TreeAction> {
        let mut picked = menu::submenu(ui, "New", |ui| self.new_menu(ui, "")).flatten();
        let paste = ui.add_enabled(
            self.copied,
            widgets::shortcut_button("Paste", EditorKey::Paste),
        );
        if paste.clicked() {
            picked = Some(TreeAction::Paste(String::new()));
        }
        ui.separator();
        if ui.button("Open all").clicked() {
            picked = Some(TreeAction::OpenAll);
        }
        if ui.button("Close all").clicked() {
            picked = Some(TreeAction::CloseAll);
        }
        if ui.button("Show the game folder").clicked() {
            picked = Some(TreeAction::ShowGameFolder);
        }
        if picked.is_some() {
            ui.close();
        }
        picked
    }

    fn many_menu(&self, ui: &mut Ui, paths: &[String]) -> Option<TreeAction> {
        let fixed = paths.iter().find_map(|path| fixed_place(path));
        let mut picked = None;
        let move_to = ui
            .add_enabled(fixed.is_none(), egui::Button::new("Move to…"))
            .on_disabled_hover_text(fixed.unwrap_or_default());
        if move_to.clicked() {
            picked = Some(TreeAction::MoveTo(paths.to_vec()));
        }
        ui.separator();
        let delete = ui
            .add_enabled(
                fixed.is_none(),
                widgets::shortcut_button(
                    format!("Delete {} files…", paths.len()),
                    EditorKey::Delete,
                ),
            )
            .on_disabled_hover_text(fixed.unwrap_or_default());
        if delete.clicked() {
            picked = Some(TreeAction::Delete(paths.to_vec()));
        }
        if picked.is_some() {
            ui.close();
        }
        picked
    }

    /// How many references to what is at `paths`, or inside it, a move rewrites.
    fn references_to(&self, paths: &[String]) -> usize {
        let usages = &self.project.usages;
        usages
            .content()
            .chain(usages.media())
            .filter(|(path, _)| {
                paths
                    .iter()
                    .any(|moved| path == moved || path.starts_with(&format!("{moved}/")))
            })
            .map(|(_, uses)| uses.len())
            .sum()
    }
}

impl TreeSource for GameFolder<'_> {
    type Action = TreeAction;

    fn children(&self, id: &str) -> Vec<String> {
        let mut listing = self.listing.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(listing) = &mut *listing else {
            return self.find_children(id);
        };
        if let Some(children) = listing.children.get(id) {
            return children.clone();
        }
        let children = self.find_children(id);
        listing.children.insert(id.to_owned(), children.clone());
        children
    }

    fn parent(&self, id: &str) -> String {
        parent_of(id).to_owned()
    }

    fn is_folder(&self, id: &str) -> bool {
        self.index.folders.contains(id)
            || (self.missing.contains_key(id) && !self.index.is_content(id))
    }

    fn row(&self, id: &str) -> TreeRow {
        let file = file_name(id);
        let name = name_of(id).to_owned();
        let after_name = file.strip_prefix(name.as_str()).unwrap_or_default();
        let mut row = TreeRow {
            problem: self.problems.get(id).cloned(),
            unsaved: self.unsaved.contains(id),
            locked: fixed_place(id).map(str::to_owned),
            ..TreeRow::default()
        };
        if self.is_folder(id) {
            let count = self.index.count_of(id);
            row.folder = true;
            row.name = file.to_owned();
            row.detail = count.to_string();
            row.dim = self.filtered || !self.index.folders.contains(id);
            row.spoken = Some(format!("{file}, folder, {count} files"));
            return row;
        }
        row.name = name;
        row.icon = Some(icons::of_file(&self.index, id));
        if id == SETTINGS_FILE {
            row.suffix = Some((" settings".to_owned(), theme::FAINT));
            row.spoken = Some("the game's settings".to_owned());
        } else if self.is_missing(id) {
            let table = table_of(id).unwrap_or_default();
            row.suffix = Some((format!(".{table}"), theme::FAINT));
            row.dim = true;
            row.problem = Some(format!(
                "the engine looks for this {table} here: Make it writes its default"
            ));
            row.spoken = Some(format!("{}, missing {table}", row.name));
        } else if self.index.is_content(id) {
            let table = table_of(id).unwrap_or_default();
            let ink = icons::of_table(table).color(IconTone::Normal);
            row.suffix = Some((format!(".{table}"), ink));
            row.spoken = Some(format!("{}, {table}", row.name));
            row.preview = self.preview(id);
        } else if self.index.media.contains(id) {
            row.suffix = Some((after_name.to_owned(), theme::FAINT));
            row.spoken = Some(format!("{file}, media"));
            row.preview = self.preview(id);
        } else if self.index.notes.contains(id) {
            row.suffix = Some((after_name.to_owned(), theme::FAINT));
            row.spoken = Some(format!("{file}, note"));
        } else if self.index.other.contains(id) {
            row.name = file.to_owned();
            row.dim = true;
            row.spoken = Some(format!("{file}, a file the game never reads"));
        } else {
            row.icon = Some(Mark::Warning.into());
            row.name = file.to_owned();
        }
        if self.filtered && row.dim {
            row.suffix = row.suffix.map(|(text, _)| (text, Color32::GRAY));
        }
        row.matched = tree::matched(&row.name, &self.needle);
        row
    }

    fn menu(&self, ui: &mut Ui, ids: &[String]) -> Option<TreeAction> {
        match ids {
            [] => self.empty_space_menu(ui),
            [one] if self.is_missing(one) && !self.is_folder(one) => {
                let make = ui.button("Make it").clicked();
                if make {
                    ui.close();
                }
                make.then(|| TreeAction::MakeFixture(one.clone()))
            }
            [one] if self.is_folder(one) => self.folder_menu(ui, one),
            [one] if self.index.is_content(one) => browser::row_menu(ui, &RowMenu::of(one), true)
                .map(|action| TreeAction::File(one.clone(), action)),
            [one] if one == SETTINGS_FILE => {
                let open = ui
                    .add(widgets::shortcut_button("Open", EditorKey::Confirm))
                    .clicked();
                if open {
                    ui.close();
                }
                open.then(|| TreeAction::File(one.clone(), BrowserAction::Open))
            }
            [one] => self.media_menu(ui, one),
            many => self.many_menu(ui, many),
        }
    }

    fn drop_tip(&self, dragged: &[String], drop: &TreeDrop) -> Result<String, String> {
        let into = folder_title(&drop.folder);
        for path in dragged {
            if let Some(reason) = fixed_place(path) {
                return Err(format!("{} stays: {reason}", file_name(path)));
            }
            if drop.folder == *path || drop.folder.starts_with(&format!("{path}/")) {
                return Err(format!("{} can't go inside itself", file_name(path)));
            }
        }
        let moving: Vec<String> = dragged
            .iter()
            .filter(|path| parent_of(path) != drop.folder)
            .cloned()
            .collect();
        if moving.is_empty() {
            return Err(format!("already in {into}"));
        }
        let what = match moving.as_slice() {
            [one] => file_name(one).to_owned(),
            many => format!("{} files", many.len()),
        };
        let references = self.references_to(&moving);
        Ok(match references {
            0 => format!("Move {what} into {into}"),
            count => format!(
                "Move {what} into {into}; the {count} reference{} to {} follow",
                words::plural(count),
                if moving.len() == 1 { "it" } else { "them" }
            ),
        })
    }

    fn ordered(&self) -> bool {
        false
    }

    fn rename_tip(&self, id: &str) -> Option<String> {
        let usages = &self.project.usages;
        let inside = format!("{id}/");
        let naming: Vec<&str> = usages
            .content()
            .chain(usages.media())
            .filter(|(path, _)| *path == id || path.starts_with(&inside))
            .flat_map(|(_, uses)| uses.iter())
            .map(|usage| content_holding(&usage.file).unwrap_or(&usage.file))
            .collect();
        let holders: BTreeSet<&str> = naming.iter().copied().collect();
        let place = match holders.iter().next() {
            Some(one) if holders.len() == 1 => crate::tabs::file_title(one).to_owned(),
            _ => format!("{} files", holders.len()),
        };
        let what = if self.is_folder(id) {
            "what is inside"
        } else {
            "this file"
        };
        let keeping = if table_of(id) == Some(BlueprintDef::TABLE) {
            ", keeping their labels"
        } else {
            ""
        };
        (!naming.is_empty()).then(|| {
            format!(
                "{} reference{} in {place} name{} {what} and will follow{keeping}",
                naming.len(),
                words::plural(naming.len()),
                if naming.len() == 1 { "s" } else { "" },
            )
        })
    }

    fn narrowed(&self) -> bool {
        self.narrowed
    }

    fn carried(&self, ids: &[String]) -> Option<Dragged> {
        let [one] = ids else {
            return None;
        };
        if self.index.is_content(one) {
            Some(Dragged::Row {
                table: table_of(one)?.to_owned(),
                key: one.clone(),
            })
        } else if self.index.media.contains(one) {
            Some(Dragged::Asset(one.clone()))
        } else {
            None
        }
    }
}

/// The row of the tree a file of the game folder shows as: a bundle for its parts, a folder for
/// its entry, or the file itself.
fn row_of(file: &str) -> String {
    match content_holding(file) {
        Some(path) => path.to_owned(),
        None => file.trim_end_matches('/').to_owned(),
    }
}

/// Visits every folder above `path`, the nearest first, ending with the game folder's top.
fn above(path: &str, visit: &mut dyn FnMut(&str)) {
    let mut folder = parent_of(path);
    while !folder.is_empty() {
        visit(folder);
        folder = parent_of(folder);
    }
}
