pub mod busts;
pub mod inspect;

use std::collections::HashMap;
use std::path::PathBuf;
use ui::chip;
use ui::egui;
use ui::form;
use ui::theme;

use engine::core::content::ContentFile;
use engine::systems::actor::ModelDef;
use engine::systems::npc::NpcDef;
use engine::systems::player::PlayerDef;
use ui::egui::Ui;

use crate::app::Editor;
use crate::audio::WaveformCache;
use crate::project::RowSelection;
use crate::project::files::content_of;
use crate::shell::{row_form, sidebar};
use crate::tabs::shown_path;

#[derive(Default)]
pub struct ModelViews {
    pub views: HashMap<String, ModelView>,
    pub bank: WaveformCache,
    pub drops: Vec<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum ModelPage {
    #[default]
    Busts,
}

#[derive(Default)]
pub struct ModelView {
    pub page: ModelPage,
    pub face: Option<String>,
    pub removing: Option<FaceRemoval>,
}

impl ModelViews {
    pub fn view(&self, key: &str) -> &ModelView {
        self.views
            .get(key)
            .expect("ensure_view ran before the model view was read")
    }

    pub fn view_mut(&mut self, key: &str) -> &mut ModelView {
        self.views.entry(key.to_owned()).or_default()
    }
}

#[derive(Clone)]
pub struct FaceRemoval {
    pub face: busts::BustFace,
    pub replacement: String,
    pub uses: usize,
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(row) = super::read_shown::<ModelDef>(editor, ui, key) else {
        return;
    };
    ensure_view(editor, key);
    let ctx = ui.ctx().clone();
    inspect::remove_face_dialog(editor, &ctx, key);
    let page = editor.views.model.view(key).page;
    sidebar::row(ui, "model", |ui| {
        row_form::show_row(editor, ui, ModelDef::TABLE, key)
    });
    let selection = editor.selection.clone();
    sidebar::inspector(editor, ui, "model", |editor, ui| {
        if selection.table == ModelDef::TABLE
            && selection.keys.first().is_some_and(|k| k == key)
            && !selection.parts.is_empty()
        {
            inspect::show(editor, ui, &selection);
        } else {
            form::nothing_selected(ui, "Select a face to inspect it here.");
        }
    });
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            page_bar(ui, key, page);
            ui.add_space(4.0);
            egui::ScrollArea::both()
                .id_salt(("model-page", key))
                .auto_shrink([false, false])
                .show(ui, |ui| match page {
                    ModelPage::Busts => busts::show(editor, ui, key, &row),
                });
        });
}

fn page_bar(ui: &mut Ui, key: &str, page: ModelPage) {
    ui.horizontal(|ui| {
        ui.label(theme::mono(shown_path(key)).color(theme::CODE));
        ui.add_space(8.0);
        chip::mode(ui, "busts", page == ModelPage::Busts);
    });
}

pub fn ensure_view(editor: &mut Editor, key: &str) {
    if editor.views.model.views.contains_key(key) {
        return;
    }
    editor.views.model.view_mut(key);
    if let Some(Ok(model)) = editor.read::<ModelDef>(key)
        && let Some(index) = busts::module_index(&model)
    {
        let path = format!("{}/modules/{index}", ModelDef::TABLE);
        if !editor.form.unfolded.contains(&path) {
            editor.form.folded.insert(path);
        }
    }
}

pub fn select_face(editor: &mut Editor, key: &str, face: &busts::BustFace) {
    editor.select(RowSelection::parts(ModelDef::TABLE, key, vec![face.part()]));
}

pub fn users(editor: &Editor, key: &str) -> Vec<(String, String)> {
    let Some(project) = editor.project.as_ref() else {
        return Vec::new();
    };
    project
        .usages
        .of_content(key)
        .iter()
        .filter_map(|usage| content_of(&usage.file))
        .filter(|(table, _)| matches!(*table, NpcDef::TABLE | PlayerDef::TABLE))
        .map(|(table, key)| (table.to_owned(), key.to_owned()))
        .collect()
}
