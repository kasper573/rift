//! The bust view: the faces of one individual's portrait as cards, PNGs dropped on it adding or
//! replacing faces, and a picked face inspected beside them.

pub mod faces;
pub mod inspect;

use std::collections::HashMap;
use std::path::PathBuf;

use engine::core::content::ContentFile;
use engine::systems::class::ClassDef;
use engine::systems::dialogue::bust::BustDef;
use engine::systems::npc::NpcDef;
use ui::egui::{self, Ui};
use ui::theme;

use crate::app::Editor;
use crate::project::RowSelection;
use crate::project::files::content_of;
use crate::shell::{row_form, sidebar};
use crate::tabs::shown_path;

#[derive(Default)]
pub struct BustViews {
    pub views: HashMap<String, BustView>,
    pub drops: Vec<PathBuf>,
}

#[derive(Default)]
pub struct BustView {
    pub face: Option<String>,
    pub removing: Option<FaceRemoval>,
}

#[derive(Clone)]
pub struct FaceRemoval {
    pub face: faces::BustFace,
    pub replacement: String,
    pub uses: usize,
}

impl BustViews {
    pub fn view(&self, key: &str) -> &BustView {
        self.views
            .get(key)
            .expect("ensure_view ran before the bust view was read")
    }

    pub fn view_mut(&mut self, key: &str) -> &mut BustView {
        self.views
            .get_mut(key)
            .expect("ensure_view ran before the bust view was changed")
    }
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(row) = super::read_shown::<BustDef>(editor, ui, key) else {
        return;
    };
    ensure_view(editor, key);
    let ctx = ui.ctx().clone();
    inspect::remove_face_dialog(editor, &ctx, key);
    sidebar::row(ui, "bust", |ui| {
        row_form::show_row(editor, ui, BustDef::TABLE, key)
    });
    let selection = editor.selection.clone();
    sidebar::inspector(editor, ui, "bust", |editor, ui| {
        if selection.table == BustDef::TABLE
            && selection.keys.first().is_some_and(|k| k == key)
            && !selection.parts.is_empty()
        {
            inspect::show(editor, ui, &selection);
        } else {
            crate::shell::row_form::usages::show_usages_grouped(editor, ui, key, true);
        }
    });
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.label(theme::mono(shown_path(key)).color(theme::CODE));
            ui.add_space(4.0);
            egui::ScrollArea::both()
                .id_salt(("bust-page", key))
                .auto_shrink([false, false])
                .show(ui, |ui| faces::grid(editor, ui, key, &row));
        });
}

pub fn ensure_view(editor: &mut Editor, key: &str) {
    editor.views.bust.views.entry(key.to_owned()).or_default();
}

pub fn select_face(editor: &mut Editor, key: &str, face: &faces::BustFace) {
    ensure_view(editor, key);
    editor.views.bust.view_mut(key).face = Some(face.name.clone());
    editor.select(RowSelection::parts(BustDef::TABLE, key, vec![face.part()]));
}

/// The npcs and classes shown in the bust at `key`, as their tables and keys.
pub fn users(editor: &Editor, key: &str) -> Vec<(String, String)> {
    let Some(project) = editor.project.as_ref() else {
        return Vec::new();
    };
    project
        .usages
        .of_content(key)
        .iter()
        .filter_map(|usage| content_of(&usage.file))
        .filter(|(table, _)| matches!(*table, NpcDef::TABLE | ClassDef::TABLE))
        .map(|(table, key)| (table.to_owned(), key.to_owned()))
        .collect()
}
