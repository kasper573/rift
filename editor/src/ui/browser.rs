use std::collections::BTreeSet;

use egui::{Align, Key, Layout, Modifiers, Sense, Ui};
use engine::core::content::ContentRow;
use engine::systems::area::world_map::WorldMapDef;

use super::tree::{self, IconImage, TreeAction, TreeIcon, TreeNode, TreeState};
use super::{theme, widgets};
use crate::app::{Editor, LeftTab, ModalDialog, RenameDialog, RenameTarget, RightTab};
use crate::picker::Dragged;
use crate::picker::{self, PictureFrame};
use crate::project::{GAME_FILE, Project, RowSelection, row_of};
use crate::tabs::EditorTab;
use crate::value::RowValue;
use crate::{forms, rows, thumbs};

pub struct BrowserState {
    pub search: String,
    pub only_unsaved: bool,
    pub only_fixtures: bool,
    pub focus_search: bool,
    pub tree: TreeState,
}

impl Default for BrowserState {
    fn default() -> BrowserState {
        BrowserState {
            search: String::new(),
            only_unsaved: false,
            only_fixtures: false,
            focus_search: false,
            tree: TreeState {
                open: ["area".to_owned()].into_iter().collect(),
            },
        }
    }
}

#[derive(Clone)]
enum FolderAction {
    New,
    OpenTable,
}

enum Hit {
    Row {
        table: String,
        key: String,
    },
    Module {
        table: String,
        key: String,
        index: usize,
        kind: String,
        name: Option<String>,
    },
    File {
        path: String,
        folder: bool,
    },
}

pub enum BrowserAction {
    Open,
    OpenTab,
    Duplicate,
    Rename,
    Usages,
    Delete,
}

pub fn focus_search(editor: &mut Editor) {
    editor.views.browser.focus_search = true;
}

pub fn show(editor: &mut Editor, ui: &mut Ui) {
    if editor.project.is_none() {
        return;
    }
    row_keys(editor, ui);
    let state = &mut editor.views.browser;
    let response = widgets::search_box(
        ui,
        &mut state.search,
        "Search rows, keys, files",
        Some("Ctrl P"),
    );
    if state.focus_search {
        response.request_focus();
        state.focus_search = false;
    }
    let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.search.clear();
    }
    let searching = !state.search.trim().is_empty();
    if searching || state.only_unsaved || state.only_fixtures {
        ui.horizontal(|ui| {
            if widgets::toggle_chip(ui, "● unsaved", state.only_unsaved).clicked() {
                state.only_unsaved = !state.only_unsaved;
            }
            if widgets::toggle_chip(ui, "fixtures", state.only_fixtures).clicked() {
                state.only_fixtures = !state.only_fixtures;
            }
        });
    }
    ui.add_space(4.0);
    if searching {
        results(editor, ui, enter);
    } else {
        folders(editor, ui);
    }
}

fn folders(editor: &mut Editor, ui: &mut Ui) {
    let only_unsaved = editor.views.browser.only_unsaved;
    let only_fixtures = editor.views.browser.only_fixtures;
    let Some(project) = &editor.project else {
        return;
    };
    let mut paths: Vec<String> = Vec::new();
    for table in &editor.shapes.order {
        paths.extend(
            project
                .files
                .paths_under(&format!("tables/{table}/"))
                .filter(|file| {
                    row_of(file).is_some_and(|(_, key)| {
                        (!only_unsaved || project.is_unsaved(file))
                            && (!only_fixtures || editor.shapes.is_fixture(table, key))
                    })
                })
                .map(|file| file.trim_start_matches("tables/").to_owned()),
        );
    }
    let selection = editor.selection.clone();
    let mut nodes = {
        let Editor {
            project, shapes, ..
        } = editor;
        let project = project.as_ref().expect("a project");
        tree::from_paths(
            paths.iter().map(String::as_str),
            &mut |path| {
                let file = format!("tables/{path}");
                let (table, key) = row_of(&file)?;
                let mut node = TreeNode::leaf(&file, key);
                node.selected = selection.table == table && selection.keys.iter().any(|k| k == key);
                node.icon = TreeIcon::Thumb(file.clone());
                node.unsaved = project.is_unsaved(&file);
                node.drag = Some(Dragged::Row {
                    table: table.to_owned(),
                    key: key.to_owned(),
                });
                Some(node)
            },
            &mut |folder| {
                let open = TreeAction {
                    glyph: "▦",
                    tip: "Open table".to_owned(),
                    menu_label: "Open table".to_owned(),
                    action: FolderAction::OpenTable,
                };
                if shapes.has_fixtures(&folder.id) {
                    folder.note =
                        Some("Fixtures: a fixed set of rows, so none can be added".to_owned());
                    folder.actions = vec![open];
                } else {
                    folder.actions = vec![
                        TreeAction {
                            glyph: "+",
                            tip: format!("New {}", folder.label),
                            menu_label: format!("New {}…", folder.label),
                            action: FolderAction::New,
                        },
                        open,
                    ];
                }
            },
        )
    };
    if !only_fixtures {
        let project = editor.project.as_ref().expect("a project");
        let unsaved = project.is_unsaved(GAME_FILE);
        if !only_unsaved || unsaved {
            let mut game = TreeNode::leaf(GAME_FILE, "game");
            game.icon = TreeIcon::Glyph("⚙");
            game.selected = selection.is_game();
            game.unsaved = unsaved;
            nodes.insert(0, game);
        }
    }
    let mut action: Option<(String, String, BrowserAction)> = None;
    let mut budget = super::assets::ICON_DECODES_PER_FRAME;
    let out = egui::ScrollArea::vertical()
        .id_salt("browser")
        .show(ui, |ui| {
            let Editor {
                project,
                form,
                views,
                shapes,
                ..
            } = editor;
            let project = project.as_ref().expect("a project");
            tree::show(
                ui,
                &mut views.browser.tree,
                &nodes,
                false,
                tree::TreeHooks {
                    menu: Some(&mut |ui, id| {
                        if let Some((table, key)) = row_of(id) {
                            let fixture = shapes.is_fixture(table, key);
                            row_menu(ui, &mut action, table, key, fixture);
                        }
                    }),
                    thumbs: Some(&mut |ctx, file| {
                        let (table, key) = row_of(file)?;
                        row_icon(ctx, form, project, table, key, &mut budget)
                    }),
                },
            )
        })
        .inner;
    if let Some((folder, action)) = out.action {
        match action {
            FolderAction::New => rows::new_row_dialog(editor, &folder),
            FolderAction::OpenTable => editor.open_tab(EditorTab::Spreadsheet { table: folder }),
        }
    }
    if out
        .clicked
        .as_ref()
        .is_some_and(|(path, _)| path == GAME_FILE)
    {
        editor.select(RowSelection::game());
        editor.panels.right = RightTab::Inspector;
    }
    if let Some((path, modifiers)) = out.clicked
        && let Some((table, key)) = row_of(&path)
    {
        if table == WorldMapDef::TABLE {
            action = Some((table.to_owned(), key.to_owned(), BrowserAction::Open));
        } else {
            select_row(editor, table, key, modifiers.ctrl, modifiers.shift);
        }
    }
    if let Some(path) = out.double_clicked
        && let Some((table, key)) = row_of(&path)
    {
        action = Some((table.to_owned(), key.to_owned(), BrowserAction::Open));
    }
    if let Some((table, key, action)) = action {
        act(editor, &table, &key, action);
    }
}

pub fn row_icon(
    ctx: &egui::Context,
    form: &mut forms::FormState,
    project: &Project,
    table: &str,
    key: &str,
    budget: &mut u32,
) -> Option<IconImage> {
    let picture = picker::picture_of(&project.files, table, key)?;
    match picture.frame {
        PictureFrame::Sprite(model) => {
            let thumb = form
                .thumbnails
                .image_within(ctx, &project.files, &picture.path, budget)?;
            let frame = thumbs::model_frame(&model, &thumb);
            let scale = 18.0 / frame.size.y.max(1.0);
            Some(IconImage {
                texture: thumb.texture.id(),
                uv: frame.uv,
                size: frame.size * scale,
            })
        }
        PictureFrame::Whole => {
            let thumb = form
                .thumbnails
                .icon(ctx, &project.files, &picture.path, budget)?;
            Some(super::assets::icon_image(&thumb))
        }
        PictureFrame::Tile(_) => None,
    }
}

fn row_keys(editor: &mut Editor, ui: &mut Ui) {
    let over_browser = ui.ui_contains_pointer();
    let on_row_form = matches!(super::active_tab(editor), Some(EditorTab::Row { .. }));
    if (!over_browser && !on_row_form) || ui.ctx().text_edit_focused() {
        return;
    }
    let selection = editor.selection.clone();
    if selection.keys.len() != 1
        || !selection.parts.is_empty()
        || selection.table.is_empty()
        || selection.is_game()
    {
        return;
    }
    let pressed = ui.input_mut(|i| {
        [
            (
                BrowserAction::Rename,
                i.consume_key(Modifiers::NONE, Key::F2),
            ),
            (
                BrowserAction::Usages,
                i.consume_key(Modifiers::SHIFT, Key::F12),
            ),
            (
                BrowserAction::Duplicate,
                i.consume_key(Modifiers::COMMAND, Key::D),
            ),
            (
                BrowserAction::Delete,
                i.consume_key(Modifiers::NONE, Key::Delete),
            ),
            (
                BrowserAction::OpenTab,
                i.consume_key(Modifiers::COMMAND, Key::Enter),
            ),
        ]
        .into_iter()
        .find(|(_, hit)| *hit)
        .map(|(action, _)| action)
    });
    if let Some(action) = pressed {
        act(editor, &selection.table, &selection.keys[0], action);
    }
}

fn row_menu(
    ui: &mut Ui,
    action: &mut Option<(String, String, BrowserAction)>,
    table: &str,
    key: &str,
    fixture: bool,
) {
    let mut pick = |what: BrowserAction, ui: &mut Ui| {
        *action = Some((table.to_owned(), key.to_owned(), what));
        ui.close();
    };
    if ui
        .add(egui::Button::new("Open").shortcut_text("Enter"))
        .clicked()
    {
        pick(BrowserAction::Open, ui);
    }
    if ui
        .add(egui::Button::new("Open in new tab").shortcut_text("Ctrl Enter"))
        .clicked()
    {
        pick(BrowserAction::OpenTab, ui);
    }
    ui.separator();
    if ui
        .add(egui::Button::new("Duplicate").shortcut_text("Ctrl D"))
        .clicked()
    {
        pick(BrowserAction::Duplicate, ui);
    }
    if !fixture
        && ui
            .add(egui::Button::new("Rename key…").shortcut_text("F2"))
            .clicked()
    {
        pick(BrowserAction::Rename, ui);
    }
    if ui
        .add(egui::Button::new("Show usages").shortcut_text("Shift F12"))
        .clicked()
    {
        pick(BrowserAction::Usages, ui);
    }
    if !fixture {
        ui.separator();
        if ui
            .add(egui::Button::new("Delete…").shortcut_text("Del"))
            .clicked()
        {
            pick(BrowserAction::Delete, ui);
        }
    }
}

fn select_row(editor: &mut Editor, table: &str, key: &str, ctrl: bool, shift: bool) {
    let mut selection = if editor.selection.table == table && (ctrl || shift) {
        editor.selection.clone()
    } else {
        RowSelection::rows(table, Vec::new())
    };
    if ctrl && selection.keys.contains(&key.to_owned()) {
        selection.keys.retain(|k| k != key);
    } else if shift && let Some(last) = selection.keys.last().cloned() {
        let keys: Vec<String> = editor
            .project
            .as_ref()
            .map(|project| {
                project
                    .files
                    .paths_under(&format!("tables/{table}/"))
                    .filter_map(|file| row_of(file).map(|(_, key)| key.to_owned()))
                    .collect()
            })
            .unwrap_or_default();
        let (a, b) = (
            keys.iter().position(|k| *k == last).unwrap_or(0),
            keys.iter().position(|k| k == key).unwrap_or(0),
        );
        for k in &keys[a.min(b)..=a.max(b)] {
            if !selection.keys.contains(k) {
                selection.keys.push(k.clone());
            }
        }
    } else if !selection.keys.contains(&key.to_owned()) {
        selection.keys.push(key.to_owned());
    }
    editor.select(selection);
    editor.panels.right = RightTab::Inspector;
}

pub fn act(editor: &mut Editor, table: &str, key: &str, action: BrowserAction) {
    match action {
        BrowserAction::Open | BrowserAction::OpenTab => {
            editor.select(RowSelection::row(table, key));
            editor.open_tab(EditorTab::for_row(table, key));
        }
        BrowserAction::Rename => {
            if editor.shapes.is_fixture(table, key) {
                editor.toast(format!("{table}/{key} is a fixture and keeps its key"));
            } else {
                editor.dialogs.push(ModalDialog::Rename(RenameDialog {
                    target: RenameTarget::Row {
                        table: table.to_owned(),
                        key: key.to_owned(),
                    },
                    draft: key.to_owned(),
                    error: None,
                }));
            }
        }
        BrowserAction::Duplicate => rows::duplicate(editor, table, key),
        BrowserAction::Usages => {
            editor.select(RowSelection::row(table, key));
            editor.panels.right = RightTab::UsedBy;
        }
        BrowserAction::Delete => rows::delete_dialog(editor, table, vec![key.to_owned()]),
    }
}

fn results(editor: &mut Editor, ui: &mut Ui, enter: bool) {
    let needle = editor.views.browser.search.trim().to_lowercase();
    let only_unsaved = editor.views.browser.only_unsaved;
    let only_fixtures = editor.views.browser.only_fixtures;
    let hits: Vec<Hit> = {
        let Some(project) = &editor.project else {
            return;
        };
        let mut rows = Vec::new();
        let mut modules = Vec::new();
        let mut files = Vec::new();
        for table in editor.shapes.order.clone() {
            for file in project.files.paths_under(&format!("tables/{table}/")) {
                let Some((_, key)) = row_of(file) else {
                    continue;
                };
                if (only_unsaved && !project.is_unsaved(file))
                    || (only_fixtures && !editor.shapes.is_fixture(&table, key))
                {
                    continue;
                }
                let row = project.files.json(file);
                let name_hit = row.as_ref().is_some_and(|row| {
                    ["name", "title", "label"].iter().any(|f| {
                        row[*f]
                            .as_str()
                            .is_some_and(|t| t.to_lowercase().contains(&needle))
                    })
                });
                if key.to_lowercase().contains(&needle) || name_hit {
                    rows.push(Hit::Row {
                        table: table.clone(),
                        key: key.to_owned(),
                    });
                }
                if let Some(row) = &row {
                    for (index, module) in
                        row["modules"].as_array().into_iter().flatten().enumerate()
                    {
                        let kind = module.kind_or("module").to_owned();
                        let name = module["name"].as_str().map(str::to_owned);
                        let kind_hit = kind.contains(&needle);
                        let name_hit = name
                            .as_ref()
                            .is_some_and(|n| n.to_lowercase().contains(&needle));
                        let reference_hit = module.as_object().is_some_and(|fields| {
                            fields.values().any(|v| {
                                v.as_str()
                                    .is_some_and(|s| s.to_lowercase().contains(&needle))
                            })
                        });
                        if kind_hit || name_hit || reference_hit {
                            modules.push(Hit::Module {
                                table: table.clone(),
                                key: key.to_owned(),
                                index,
                                kind,
                                name,
                            });
                        }
                    }
                }
            }
        }
        let mut folders: BTreeSet<String> = std::collections::BTreeSet::new();
        for path in project.files.paths_under("assets/") {
            let relative = path.trim_start_matches("assets/");
            if super::assets::kind(relative).is_none() {
                continue;
            }
            if let Some((folder, _)) = relative.rsplit_once('/')
                && folder.to_lowercase().contains(&needle)
            {
                folders.insert(folder.to_owned());
            } else if relative.to_lowercase().contains(&needle) {
                files.push(Hit::File {
                    path: relative.to_owned(),
                    folder: false,
                });
            }
        }
        for folder in folders {
            files.push(Hit::File {
                path: format!("{folder}/"),
                folder: true,
            });
        }
        rows.into_iter().chain(modules).chain(files).collect()
    };
    editor.status.left.push(format!(
        "{} result{} for \"{}\"",
        hits.len(),
        crate::ui::widgets::plural(hits.len()),
        editor.views.browser.search.trim()
    ));
    let mut clicked: Option<(String, String, bool, bool)> = None;
    let mut action: Option<(String, String, BrowserAction)> = None;
    let mut open_module: Option<(String, String, usize)> = None;
    let mut open_file: Option<String> = None;
    let mut heading: Option<&str> = None;
    egui::ScrollArea::vertical()
        .id_salt("results")
        .show(ui, |ui| {
            for hit in &hits {
                let kind = match hit {
                    Hit::Row { .. } => "rows",
                    Hit::Module { .. } => "modules",
                    Hit::File { .. } => "files",
                };
                if heading != Some(kind) {
                    heading = Some(kind);
                    ui.add_space(4.0);
                    ui.label(theme::mono(kind).color(theme::CODE).size(12.5));
                }
                match hit {
                    Hit::Row { table, key } => {
                        let selected =
                            editor.selection.table == *table && editor.selection.keys.contains(key);
                        let response = result_line(
                            editor,
                            ui,
                            Some((table, key)),
                            &format!("{table}/{key}"),
                            None,
                            selected,
                        );
                        if response.clicked() {
                            let modifiers = ui.input(|i| i.modifiers);
                            clicked =
                                Some((table.clone(), key.clone(), modifiers.ctrl, modifiers.shift));
                        }
                        if response.double_clicked() {
                            action = Some((table.clone(), key.clone(), BrowserAction::Open));
                        }
                        let fixture = editor.shapes.is_fixture(table, key);
                        response.context_menu(|ui| row_menu(ui, &mut action, table, key, fixture));
                    }
                    Hit::Module {
                        table,
                        key,
                        index,
                        kind,
                        name,
                    } => {
                        let label = name.clone().unwrap_or_else(|| kind.clone());
                        let response = result_line(
                            editor,
                            ui,
                            None,
                            &label,
                            Some((kind.as_str(), format!("{table}/{key}"))),
                            false,
                        );
                        if response.clicked() {
                            open_module = Some((table.clone(), key.clone(), *index));
                        }
                    }
                    Hit::File { path, folder } => {
                        let icon = if *folder { "▸" } else { "▫" };
                        let response = ui
                            .horizontal(|ui| {
                                ui.add_space(6.0);
                                ui.label(theme::muted(icon));
                                let sense = if *folder {
                                    Sense::click()
                                } else {
                                    Sense::click_and_drag()
                                };
                                let response = ui.add(
                                    egui::Label::new(
                                        theme::mono(path).color(theme::CODE).size(13.0),
                                    )
                                    .sense(sense),
                                );
                                if !*folder {
                                    picker::carry(ui, &response, &Dragged::Asset(path.clone()));
                                }
                                response
                            })
                            .inner;
                        if response.clicked() {
                            open_file = Some(path.clone());
                        }
                    }
                }
            }
            if hits.is_empty() {
                ui.label(theme::muted("nothing matches"));
            }
        });
    if enter && let Some(first) = hits.first() {
        match first {
            Hit::Row { table, key } => {
                action = Some((table.clone(), key.clone(), BrowserAction::Open))
            }
            Hit::Module {
                table, key, index, ..
            } => open_module = Some((table.clone(), key.clone(), *index)),
            Hit::File { path, .. } => open_file = Some(path.clone()),
        }
    }
    if let Some((table, key, ctrl, shift)) = clicked {
        select_row(editor, &table, &key, ctrl, shift);
    }
    if let Some((table, key, action)) = action {
        act(editor, &table, &key, action);
    }
    if let Some((table, key, index)) = open_module {
        editor.select(RowSelection {
            table: table.clone(),
            keys: vec![key.clone()],
            parts: vec![format!("/modules/{index}")],
        });
        editor.open_tab(EditorTab::for_row(&table, &key));
        editor.panels.right = RightTab::Inspector;
    }
    if let Some(path) = open_file {
        editor.panels.left = LeftTab::Assets;
        editor.views.assets.search = path.trim_end_matches('/').to_owned();
        if !path.ends_with('/') {
            editor.select(RowSelection::row("asset", &path));
            editor.panels.right = RightTab::Inspector;
        }
    }
}

fn result_line(
    editor: &mut Editor,
    ui: &mut Ui,
    row: Option<(&str, &str)>,
    label: &str,
    module: Option<(&str, String)>,
    selected: bool,
) -> egui::Response {
    let full = ui.available_rect_before_wrap();
    let sense = if row.is_some() {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(full.width(), 22.0), sense);
    if let Some((table, key)) = row {
        picker::carry(
            ui,
            &response,
            &Dragged::Row {
                table: table.to_owned(),
                key: key.to_owned(),
            },
        );
    }
    if selected {
        ui.painter().rect_filled(rect, 3.0, theme::ACCENT_SOFT);
    } else if response.hovered() {
        ui.painter().rect_filled(
            rect,
            3.0,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 8),
        );
    }
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    child.add_space(6.0);
    if let Some((kind, owner)) = &module {
        widgets::badge(&mut child, kind);
        child.label(theme::mono(label).size(13.0));
        child.label(theme::faint("·"));
        child.label(theme::muted(owner).monospace().size(12.5));
    } else if let Some((table, key)) = row {
        let mut budget = super::assets::ICON_DECODES_PER_FRAME;
        let icon = editor.project.as_ref().and_then(|project| {
            row_icon(
                &child.ctx().clone(),
                &mut editor.form,
                project,
                table,
                key,
                &mut budget,
            )
        });
        if let Some(icon) = icon {
            child.add(egui::Image::from_texture((icon.texture, icon.size)).uv(icon.uv));
        }
    }
    if module.is_none() {
        child.label(theme::mono(label).size(13.0).color(if selected {
            theme::TEXT
        } else {
            theme::CODE
        }));
    }
    response
}
