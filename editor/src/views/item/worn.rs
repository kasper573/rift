//! The worn page of an item worn in a slot: a body of each skeleton the item has a look for,
//! wearing it laid over the body's own parts and painted as the game paints it, and what the look
//! lays, covers and hides.

use std::collections::BTreeMap;

use engine::core::content::{Content, NamedContent};
use engine::systems::actor::appearance::{self, Appearance};
use engine::systems::actor::model::ActorModel;
use engine::systems::actor::{Action, ModelDef, Motion, Pose};
use engine::systems::item::{ItemDef, Wear, WornLook};
use ui::egui::{self, Ui, Vec2};
use ui::{chip, controls, form, theme};

use crate::app::Editor;
use crate::tabs::shown_path;
use crate::views::stage::{self, StageState};

/// What the worn page's stage shows: the look of a skeleton, by its path, on a model of it; the
/// stage is framed for the first model it shows.
#[derive(Default)]
pub struct WornView {
    pub skeleton: Option<String>,
    pub model: Option<String>,
    pub stage: Option<StageState>,
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(content) = editor.loaded() else {
        form::hint(ui, "The stage stands once the content loads.");
        return;
    };
    let Some(item) = content.by_path::<ItemDef>(key) else {
        form::hint(
            ui,
            "The item did not load: its problems are in the output panel.",
        );
        return;
    };
    let Some(Wear::Worn { slot, looks }) = item.get(&content).wear() else {
        return;
    };
    if looks.is_empty() {
        form::hint(
            ui,
            "No look yet: add one for each skeleton whose bodies should show the item. Worn \
             without one, it counts all the same and is never drawn.",
        );
        return;
    }
    let skeletons: Vec<String> = looks
        .iter()
        .map(|look| look.skeleton.path(&content).to_owned())
        .collect();
    let view = editor.views.item.view_mut(key);
    let skeleton = view
        .worn
        .skeleton
        .clone()
        .filter(|chosen| skeletons.contains(chosen))
        .unwrap_or_else(|| skeletons[0].clone());
    let look = &looks[skeletons
        .iter()
        .position(|path| *path == skeleton)
        .unwrap_or_default()];
    let models: Vec<String> = content
        .table::<ModelDef>()
        .iter()
        .filter(|(id, def)| {
            def.skeleton == look.skeleton && ActorModel::loaded(&content, *id).is_some()
        })
        .map(|(id, _)| id.path(&content).to_owned())
        .collect();
    let model = view
        .worn
        .model
        .clone()
        .filter(|chosen| models.contains(chosen))
        .or_else(|| models.first().cloned());
    let mut picked = (skeleton.clone(), model.clone());
    ui.horizontal(|ui| {
        ui.label(theme::muted("on"));
        for path in &skeletons {
            let name = content
                .by_path::<engine::systems::actor::SkeletonDef>(path)
                .map_or(shown_path(path).to_owned(), |id| {
                    id.get(&content).shown_name().to_owned()
                });
            if chip::mode(ui, &name, *path == skeleton).clicked() {
                picked.0 = path.clone();
            }
        }
        ui.add_space(8.0);
        ui.label(theme::muted("worn by"));
        let shown = model.as_deref().map_or("no model", shown_path).to_owned();
        let mut chosen = model.clone().unwrap_or_default();
        controls::combo(("worn-model", key))
            .selected_text(theme::mono(shown).size(13.0))
            .width(220.0)
            .show_ui(ui, |ui| {
                for path in &models {
                    ui.selectable_value(&mut chosen, path.clone(), shown_path(path));
                }
            });
        if !chosen.is_empty() {
            picked.1 = Some(chosen);
        }
    });
    let view = editor.views.item.view_mut(key);
    view.worn.skeleton = Some(picked.0);
    view.worn.model = picked.1;
    let Some(model) = model.and_then(|path| content.by_path::<ModelDef>(&path)) else {
        form::hint(ui, "No model of the skeleton loads to wear the look.");
        return;
    };
    stand(editor, ui, key, &content, model, slot, item);
    ui.add_space(6.0);
    laid(ui, &content, look);
}

/// The model at `model` wearing the item in `slot` over its own look, idling.
fn stand(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    content: &Content,
    model: engine::systems::actor::ModelId,
    slot: &engine::systems::actor::skeleton::EquipmentSlot,
    item: engine::systems::item::ItemId,
) {
    let Some(built) = ActorModel::loaded(content, model) else {
        return;
    };
    let size = Vec2::new(
        ui.available_width(),
        (ui.available_height() * 0.62).max(200.0),
    );
    let mut state = editor
        .views
        .item
        .view_mut(key)
        .worn
        .stage
        .take()
        .unwrap_or_else(|| StageState::framing(built.hitbox().height.0.max(1.0) * 0.7, false));
    let turntable = stage::turntable(editor, ui, &format!("worn:{key}"), size, &mut state);
    editor.views.item.view_mut(key).worn.stage = Some(state);
    let Some(turntable) = turntable else {
        return;
    };
    let worn = BTreeMap::from([(slot.clone(), item)]);
    let body = built.dressed(
        content,
        &appearance::look(content, &Appearance::of(model), &worn),
    );
    let clip = built.standing_clip(None, &Motion::Action(Action::Idle), Pose::Idle);
    let standing = stage::standing(body, clip, None, Vec::new(), Vec::new());
    turntable.stand(editor, ui, vec![standing]);
}

/// What the look lays over a body: each part with the regions it covers and hides, and the colours
/// it paints them in over the wearer's.
fn laid(ui: &mut Ui, content: &Content, look: &WornLook) {
    egui::Grid::new("worn-laid")
        .num_columns(3)
        .spacing(Vec2::new(12.0, 4.0))
        .show(ui, |ui| {
            for part in &look.parts {
                let def = part.get(content);
                ui.label(theme::mono(shown_path(part.path(content))).size(13.0));
                ui.label(theme::muted(format!(
                    "covers {}",
                    words(def.covers.iter().map(|region| region.as_str()))
                )));
                ui.label(theme::muted(if def.hides.is_empty() {
                    String::new()
                } else {
                    format!(
                        "hides {}",
                        words(def.hides.iter().map(|region| region.as_str()))
                    )
                }));
                ui.end_row();
            }
        });
    if look.colours.is_empty() {
        ui.label(theme::faint("painted in the wearer's colours").size(12.0));
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(theme::muted("paints"));
        for (tint, colour) in &look.colours.0 {
            chip::swatch(ui, theme::mono(tint.as_str()).size(13.0), (*colour).into());
        }
    });
}

fn words<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let names: Vec<&str> = names.collect();
    if names.is_empty() {
        "nothing".to_owned()
    } else {
        names.join(", ")
    }
}
