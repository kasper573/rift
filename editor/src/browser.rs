use ui::egui;
use ui::egui::Ui;

use super::{content_tree, widgets};
use crate::app::{BottomTab, Editor};
use crate::keymap::EditorKey;
use crate::moves::{fixed_place, uncopyable};
use crate::rows;
use crate::tabs::EditorTab;
use engine::core::content::table_of;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserAction {
    Open,
    OpenTab,
    Duplicate,
    Rename,
    MoveTo,
    Usages,
    ShowInTree,
    OpenTable,
    CopyPath,
    Delete,
}

/// What a content file's menu offers: everything, except that a fixture stays where the engine
/// finds it and a type of fixtures alone takes no copies, which the menu says rather than hiding.
pub(crate) struct RowMenu {
    table: String,
    fixed: Option<&'static str>,
    uncopyable: Option<&'static str>,
}

impl RowMenu {
    pub fn of(path: &str) -> RowMenu {
        RowMenu {
            table: table_of(path).unwrap_or_default().to_owned(),
            fixed: fixed_place(path),
            uncopyable: uncopyable(path),
        }
    }
}

/// The menu of what can be done to one content file, each with its shortcut. In the content tree
/// it opens the file; elsewhere it shows the file in the tree. Answers the one picked.
pub(crate) fn row_menu(ui: &mut Ui, menu: &RowMenu, in_tree: bool) -> Option<BrowserAction> {
    use BrowserAction::*;
    let finding = if in_tree { Open } else { ShowInTree };
    let groups = [
        &[finding, Usages, CopyPath, OpenTable][..],
        &[Rename, Duplicate, MoveTo][..],
        &[Delete][..],
    ];
    let mut picked = None;
    let mut drawn = false;
    for group in groups {
        if group.is_empty() {
            continue;
        }
        if drawn {
            ui.separator();
        }
        drawn = true;
        for &action in group {
            let refusal = match action {
                Duplicate => menu.uncopyable,
                _ => menu.fixed.filter(|_| action.moves_the_file()),
            };
            let label = action.label(&menu.table);
            let button = match action.key() {
                Some(key) => widgets::shortcut_button(&label, key),
                None => egui::Button::new(label),
            };
            let response = ui.add_enabled(refusal.is_none(), button);
            let response = match refusal {
                Some(refusal) => response.on_disabled_hover_text(refusal),
                None => response,
            };
            if response.clicked() {
                picked = Some(action);
                ui.close();
            }
        }
    }
    picked
}

impl BrowserAction {
    fn label(self, table: &str) -> String {
        match self {
            BrowserAction::Open => "Open".to_owned(),
            BrowserAction::OpenTab => "Open in new tab".to_owned(),
            BrowserAction::Duplicate => "Duplicate".to_owned(),
            BrowserAction::Rename => "Rename".to_owned(),
            BrowserAction::MoveTo => "Move to…".to_owned(),
            BrowserAction::Usages => "Used by".to_owned(),
            BrowserAction::ShowInTree => "Show in the content tree".to_owned(),
            BrowserAction::OpenTable => format!("Open the {table} table"),
            BrowserAction::CopyPath => "Copy path".to_owned(),
            BrowserAction::Delete => "Delete…".to_owned(),
        }
    }

    fn key(self) -> Option<EditorKey> {
        match self {
            BrowserAction::Open => Some(EditorKey::Confirm),
            BrowserAction::OpenTab => Some(EditorKey::OpenInNewTab),
            BrowserAction::Duplicate => Some(EditorKey::Duplicate),
            BrowserAction::Rename => Some(EditorKey::Rename),
            BrowserAction::Usages => Some(EditorKey::Usages),
            BrowserAction::CopyPath => Some(EditorKey::CopyPath),
            BrowserAction::Delete => Some(EditorKey::Delete),
            BrowserAction::ShowInTree | BrowserAction::OpenTable | BrowserAction::MoveTo => None,
        }
    }

    /// Whether the action moves, renames or removes the file, which a fixture refuses.
    fn moves_the_file(self) -> bool {
        matches!(
            self,
            BrowserAction::Rename | BrowserAction::MoveTo | BrowserAction::Delete
        )
    }
}

pub fn act(editor: &mut Editor, table: &str, key: &str, action: BrowserAction) {
    match action {
        BrowserAction::Open => editor.preview_row(table, key),
        BrowserAction::OpenTab => editor.open_row(table, key),
        BrowserAction::Rename => match fixed_place(key) {
            Some(reason) => editor.toast(format!("{key} stays: {reason}")),
            None => content_tree::begin_rename(editor, key),
        },
        BrowserAction::Duplicate => {
            if let Err(error) = rows::duplicate(editor, key) {
                editor.toast(error);
            }
        }
        BrowserAction::Usages => {
            editor.preview_row(table, key);
            editor.bottom_tab = BottomTab::UsedBy;
        }
        BrowserAction::ShowInTree => content_tree::reveal(editor, key),
        BrowserAction::OpenTable => editor.open_tab(EditorTab::Spreadsheet {
            table: table.to_owned(),
        }),
        BrowserAction::CopyPath => editor.copy_text(key),
        BrowserAction::MoveTo => content_tree::move_to_dialog(editor, vec![key.to_owned()]),
        BrowserAction::Delete => rows::delete_dialog(editor, vec![key.to_owned()]),
    }
}
