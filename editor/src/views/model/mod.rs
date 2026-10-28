//! The model view: the body its parts make on a stage, in any stance and clip its skeleton plays,
//! and its hitbox sized by hand.

pub mod hitbox;
pub mod inspect;
pub mod look;

use std::collections::HashMap;

use engine::core::content::{Content, ContentFile};
use engine::core::geometry::Capsule;
use engine::core::time::Seconds;
use engine::systems::actor::model::ActorModel;
use engine::systems::actor::skeleton::Cue;
use engine::systems::actor::{Action, ModelDef, Motion, Pose};
use serde::Deserialize;
use ui::egui::{self, Ui, Vec2};
use ui::{chip, form, theme};

use super::area::viewport::StageClip;
use super::stage;
use crate::app::Editor;
use crate::shell::{row_form, sidebar};
use crate::tabs::shown_path;

#[derive(Default)]
pub struct ModelViews {
    pub views: HashMap<String, ModelView>,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelPage {
    #[default]
    Look,
    Hitbox,
}

impl ModelPage {
    const ALL: [ModelPage; 2] = [ModelPage::Look, ModelPage::Hitbox];

    fn label(self) -> &'static str {
        match self {
            ModelPage::Look => "look",
            ModelPage::Hitbox => "hitbox",
        }
    }
}

pub struct ModelView {
    pub page: ModelPage,
    /// The stance the stage shows the model in, by its file's path.
    pub stance: Option<String>,
    pub motion: Motion,
    pub pose: Pose,
    pub scrub: super::cues::Scrub,
    pub stage: stage::StageState,
    pub drag: Option<hitbox::Drag>,
}

impl ModelViews {
    pub fn view(&self, key: &str) -> &ModelView {
        self.views
            .get(key)
            .expect("ensure_view ran before the model view was read")
    }

    pub fn view_mut(&mut self, key: &str) -> &mut ModelView {
        self.views
            .get_mut(key)
            .expect("ensure_view ran before the model view was changed")
    }
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(row) = super::read_shown::<ModelDef>(editor, ui, key) else {
        return;
    };
    ensure_view(editor, key);
    let page = editor.views.model.view(key).page;
    sidebar::row(ui, "model", |ui| {
        row_form::show_row(editor, ui, ModelDef::TABLE, key)
    });
    sidebar::inspector(editor, ui, "model", |editor, ui| {
        inspect::show(editor, ui, key, &row);
    });
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            page_bar(editor, ui, key, page);
            ui.add_space(4.0);
            match page {
                ModelPage::Look => look::show(editor, ui, key),
                ModelPage::Hitbox => hitbox::show(editor, ui, key, &row),
            }
        });
}

fn page_bar(editor: &mut Editor, ui: &mut Ui, key: &str, page: ModelPage) {
    ui.horizontal(|ui| {
        ui.label(theme::mono(shown_path(key)).color(theme::CODE));
        ui.add_space(8.0);
        for other in ModelPage::ALL {
            if chip::mode(ui, other.label(), page == other).clicked() {
                editor.views.model.view_mut(key).page = other;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let stage = &mut editor.views.model.view_mut(key).stage;
            if chip::toggle(ui, "turning", stage.spinning).clicked() {
                stage.spinning = !stage.spinning;
            }
        });
    });
}

pub fn ensure_view(editor: &mut Editor, key: &str) {
    if editor.views.model.views.contains_key(key) {
        return;
    }
    let model = editor.read::<ModelDef>(key).and_then(Result::ok);
    let height = model.as_ref().map_or(1.8, |model| model.hitbox.height.0);
    editor.views.model.views.insert(
        key.to_owned(),
        ModelView {
            page: ModelPage::default(),
            stance: None,
            motion: Motion::Action(Action::Idle),
            pose: Pose::Idle,
            scrub: super::cues::Scrub::default(),
            stage: stage::StageState::framing(height.max(1.0) * 0.7, false),
            drag: None,
        },
    );
}

/// Sizes the hitbox of the model at `key`.
pub fn set_hitbox(editor: &mut Editor, key: &str, hitbox: Capsule) -> bool {
    let Some(Ok(row)) = editor.read::<ModelDef>(key) else {
        return false;
    };
    if row.hitbox == hitbox {
        return false;
    }
    let mut model = (*row).clone();
    model.hitbox = hitbox;
    editor.write_content(&format!("Size {}'s hitbox", shown_path(key)), key, &model)
}

/// The model at `key` as the game built it from the files as they last loaded, when it loaded.
pub fn built<'c>(editor: &'c Editor, key: &str) -> Option<(&'c Content, &'c ActorModel)> {
    let content = editor.project.as_ref()?.content.as_ref()?;
    let id = content.by_path::<ModelDef>(key)?;
    Some((content, ActorModel::loaded(content, id)?))
}

/// The model's stage, drawn over `size` with its camera moved as the pointer asks.
pub(super) fn turntable(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    size: Vec2,
) -> Option<stage::Turntable> {
    ensure_view(editor, key);
    let mut state = std::mem::take(&mut editor.views.model.view_mut(key).stage);
    let turntable = stage::turntable(editor, ui, &format!("model:{key}"), size, &mut state);
    editor.views.model.view_mut(key).stage = state;
    turntable
}

/// Stands the model at `key` in the middle of `turntable`, as the game built it, in `clip` held at
/// `held`, sounding the cues it `crossed`.
pub(super) fn stand(
    turntable: stage::Turntable,
    editor: &mut Editor,
    ui: &Ui,
    key: &str,
    clip: Option<StageClip>,
    held: Option<Seconds>,
    crossed: Vec<Cue>,
) {
    let cast = built(editor, key)
        .map(|(_, model)| stage::standing(model.body(), clip, held, crossed, Vec::new()))
        .into_iter()
        .collect();
    turntable.stand(editor, ui, cast);
}

pub(super) fn nothing_to_stand(ui: &mut Ui) {
    form::hint(
        ui,
        "The model did not load, so nothing stands on the stage: its problems are in the output \
         panel.",
    );
}
