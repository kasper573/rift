use super::rigging::{Playable, Rigging};
use super::skeleton_id;
use engine::core::content::{Content, NamedContent};
use engine::systems::actor::ModelDef;
use engine::systems::actor::model::ActorModel;
use engine::systems::actor::skeleton::Clip;
use engine::systems::item::{ItemDef, Wear};
use ui::egui::Ui;
use ui::{controls, theme};

/// The models that wear the skeleton at `key`, by their paths, with how each was built if it
/// loaded.
pub(crate) fn wearers<'c>(
    content: &'c Content,
    key: &str,
) -> Vec<(String, Option<&'c ActorModel>)> {
    let Some(id) = skeleton_id(content, key) else {
        return Vec::new();
    };
    content
        .table::<ModelDef>()
        .iter()
        .filter(|(_, model)| model.skeleton == id)
        .map(|(model, _)| {
            (
                model.path(content).to_owned(),
                ActorModel::loaded(content, model),
            )
        })
        .collect()
}

/// The items held in a hand, by their paths, with their names.
pub(crate) fn hand_items(content: &Content) -> Vec<(String, String)> {
    content
        .table::<ItemDef>()
        .iter()
        .filter(|(_, item)| matches!(item.wear(), Some(Wear::Held { .. })))
        .map(|(id, item)| (id.path(content).to_owned(), item.shown_name().to_owned()))
        .collect()
}

/// A combo labelled `label` of every clip that plays on the rig, filtered by any part of its file's
/// path as it is typed, offering no clip too as `none` names it.
pub(crate) fn clip_picker(
    ui: &mut Ui,
    label: &str,
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
    rigging: &Rigging,
    clip: &mut Option<Clip>,
    none: Option<&str>,
) {
    ui.horizontal(|ui| {
        ui.label(theme::muted(label));
        let selected = clip
            .as_ref()
            .map_or(none.unwrap_or("pick one").to_owned(), |clip| {
                let length = rigging.playable(clip).map_or(0.0, |found| found.length.0);
                format!("{} · {length:.2} s", clip.shown())
            });
        controls::filtered_combo(ui, id, selected, 260.0, "filter by path", |ui, filter| {
            let mut picked = false;
            if let Some(none) = none {
                picked |= ui.selectable_value(clip, None, none).clicked();
            }
            for found in matching(rigging, filter).take(MAX_LISTED) {
                picked |= ui
                    .selectable_value(
                        clip,
                        Some(found.clip.clone()),
                        format!("{} · {:.2} s", found.clip, found.length.0),
                    )
                    .clicked();
            }
            picked
        });
    });
}

/// The most clips a list offers at once; typing narrows it.
pub(crate) const MAX_LISTED: usize = 200;

/// The clips that play on the rig whose file's path holds `filter`, ignoring case.
pub fn matching<'r>(rigging: &'r Rigging, filter: &str) -> impl Iterator<Item = &'r Playable> {
    let filter = filter.trim().to_lowercase();
    rigging.clips.iter().filter(move |found| {
        filter.is_empty() || found.clip.to_string().to_lowercase().contains(&filter)
    })
}
