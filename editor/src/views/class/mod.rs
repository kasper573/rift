pub mod inspect;
pub mod tree;

use std::collections::BTreeMap;
use ui::chip;
use ui::egui;
use ui::form;
use ui::theme;

use engine::core::content::{ContentFile, fields_file};
use engine::field;
use engine::systems::class::{ClassDef, ClassLevel, SkillPoints};
use engine::systems::stat::{StatKind, Stats};
use serde_json::Value;
use ui::egui::Ui;

use crate::app::Editor;
use crate::forms::{FormChange, FormCx};
use crate::levels::{self, Level};
use crate::project::{ContentEdit, RowSelection};
use crate::row_form;
use crate::tabs::shown_path;
use crate::views::ViewMap;

pub const TREE_PART: &str = "/tree/";

const FIRST_LEVEL_UP: u32 = 100;

pub struct ClassViews {
    pub pages: ViewMap<ClassPage>,
    /// Whether a dragged node or frame lands on the grid.
    pub snap: bool,
    pub drag: Option<tree::NodeDrag>,
    pub frame_drag: Option<tree::FrameDrag>,
}

impl Default for ClassViews {
    fn default() -> ClassViews {
        ClassViews {
            pages: ViewMap::default(),
            snap: true,
            drag: None,
            frame_drag: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum ClassPage {
    #[default]
    Levels,
    Tree,
}

impl ClassViews {
    pub fn page(&self, key: &str) -> ClassPage {
        self.pages.get(key).copied().unwrap_or_default()
    }
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(project) = &editor.project else {
        return;
    };
    if !project.files.contains(&fields_file(key)) {
        ui.label(theme::muted(format!("{} is gone", fields_file(key))));
        return;
    }
    let page = editor.views.class.page(key);
    let selection = editor.selection.clone();
    match page {
        ClassPage::Tree => ui::panel::inspector(ui, editor, "class", |editor, ui| {
            if selection.keys.first().is_some_and(|k| k == key) && node_of(&selection).is_some() {
                inspect::show(editor, ui, &selection);
            } else {
                form::nothing_selected(ui, "Select a skill in the tree to inspect it here.");
            }
        }),
        ClassPage::Levels => {}
    }
    egui::Frame::new()
        .inner_margin(crate::widgets::VIEW_PADDING)
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            page_body(editor, ui, key, page);
        });
}

fn page_body(editor: &mut Editor, ui: &mut Ui, key: &str, page: ClassPage) {
    ui.horizontal(|ui| {
        for (target, label) in [
            (ClassPage::Levels, "levels"),
            (ClassPage::Tree, "skill tree"),
        ] {
            if chip::mode(ui, label, page == target).clicked() {
                editor.views.class.pages.insert(key, target);
                if target == ClassPage::Levels {
                    editor.select(RowSelection::row(ClassDef::TABLE, key));
                }
            }
        }
    });
    ui.add_space(4.0);
    match page {
        ClassPage::Levels => row_form::show_row_tab(editor, ui, ClassDef::TABLE, key),
        ClassPage::Tree => tree::show(editor, ui, key),
    }
}

pub fn node_of(selection: &RowSelection) -> Option<&str> {
    if selection.table != ClassDef::TABLE || selection.keys.len() != 1 {
        return None;
    }
    selection
        .parts
        .first()?
        .strip_prefix(TREE_PART)
        .filter(|node| !node.is_empty())
}

pub(crate) fn select_node(editor: &mut Editor, key: &str, node: Option<&str>) {
    match node {
        Some(node) => editor.select(RowSelection::parts(
            ClassDef::TABLE,
            key,
            vec![format!("{TREE_PART}{node}")],
        )),
        None => editor.select(RowSelection::row(ClassDef::TABLE, key)),
    }
}

pub fn edit(key: &str, label: &str) -> ContentEdit {
    ContentEdit::new(format!("{label} · {}", shown_path(key)))
}

pub fn levels(ui: &mut Ui, cx: &mut FormCx, value: &mut Value) -> FormChange {
    levels::row_cards(
        ui,
        cx,
        value,
        field!(ClassDef, levels),
        class_levels,
        |index, level| {
            Some(if index == 0 && level.xp == 0 {
                "the start".to_owned()
            } else {
                format!("from {} xp", level.xp)
            })
        },
    )
}

fn class_levels(class: &mut ClassDef) -> &mut Vec<ClassLevel> {
    &mut class.levels
}

impl Level for ClassLevel {
    type Stat = StatKind;
    const NAME: &'static str = "Level";
    const STATS: &'static str = field!(ClassLevel, stats);

    fn stats(&mut self) -> &mut BTreeMap<StatKind, f32> {
        &mut self.stats.0
    }

    fn unit(stat: StatKind) -> Option<&'static str> {
        stat.unit()
    }

    fn next(last: Option<&ClassLevel>) -> ClassLevel {
        ClassLevel {
            xp: last.map_or(0, |level| level.xp + FIRST_LEVEL_UP),
            stats: Stats::default(),
            skill_points: SkillPoints(1),
        }
    }
}
