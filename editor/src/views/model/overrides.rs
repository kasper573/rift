//! The overrides page: every clip the model's own glb carries, which plays in place of the
//! skeleton's clip of its name, with its cues; the chosen one on the stage and laid out large to
//! time. A clip no set names is flagged, as it is downloaded and never played.

use engine::core::render::world::mannequin::RigClip;
use engine::core::time::Millis;
use engine::systems::actor::ModelDef;
use engine::systems::actor::skeleton::{ClipName, Cue};
use ui::egui::{self, Sense, Ui, Vec2};
use ui::{chip, form, theme};

use crate::app::Editor;
use crate::tabs::shown_path;
use crate::views::cues;

/// A clip of the model's own, as the page lists it.
struct Own {
    name: ClipName,
    index: usize,
    length: Millis,
    cues: Vec<Cue>,
    /// The library of the skeleton's clip it replaces, if one has its name.
    replaces: Option<String>,
    /// Where the skeleton's sets name it.
    named: Vec<String>,
    strikes: bool,
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, row: &ModelDef) {
    let owns = owns(editor, key, row);
    if owns.is_empty() {
        form::hint(
            ui,
            "model.glb carries no clips: the model plays its skeleton's. Import a glb carrying a \
             clip of a library clip's name to override it.",
        );
        return;
    }
    let chosen = editor
        .views
        .model
        .view(key)
        .own_clip
        .clone()
        .filter(|name| owns.iter().any(|own| own.name == *name))
        .unwrap_or_else(|| owns[0].name.clone());
    let own = owns
        .iter()
        .find(|own| own.name == chosen)
        .expect("the chosen clip is the model's");
    let height = ui.available_height();
    cues::advance(
        ui,
        &mut editor.views.model.view_mut(key).scrub,
        Some(own.length),
    );
    if let Some(turntable) = super::turntable(
        editor,
        ui,
        key,
        Vec2::new(ui.available_width(), (height * 0.42).max(120.0)),
    ) {
        let playhead = editor.views.model.view(key).scrub.playhead;
        let clip = RigClip {
            file: row.model.path(),
            index: own.index,
        };
        super::stand(
            turntable,
            editor,
            ui,
            row,
            Some(clip),
            Some(playhead.seconds()),
        );
    }
    ui.add_space(6.0);
    let mut pick = None;
    egui::ScrollArea::vertical()
        .id_salt(("overrides", key))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new(("own-clips", key))
                .num_columns(3)
                .spacing(Vec2::new(10.0, 6.0))
                .show(ui, |ui| {
                    for other in &owns {
                        if chip::mode(ui, other.name.as_str(), other.name == chosen).clicked() {
                            pick = Some(other.name.clone());
                        }
                        ui.label(theme::muted(note(other)));
                        let (rect, response) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width().max(120.0), cues::STRIP_HEIGHT),
                            Sense::click(),
                        );
                        cues::strip(
                            ui.painter(),
                            rect,
                            other.length,
                            &other.cues,
                            None,
                            other.name == chosen,
                        );
                        if response.clicked() {
                            pick = Some(other.name.clone());
                        }
                        ui.end_row();
                    }
                });
            ui.add_space(8.0);
            ui.label(theme::muted(format!(
                "{} · {:.2} s · its cues are the model's own",
                own.name,
                own.length.0 / 1000.0
            )));
            let scrub = &mut editor.views.model.view_mut(key).scrub;
            if let Some(cues) = cues::timeline(ui, scrub, own.length, &own.cues, own.strikes) {
                super::set_override_cues(editor, key, &own.name, cues);
            }
        });
    if let Some(name) = pick {
        let view = editor.views.model.view_mut(key);
        if view.own_clip.as_ref() != Some(&name) {
            view.own_clip = Some(name);
            view.scrub.restart();
        }
    }
    editor
        .status
        .left
        .push(format!("{} · {chosen}", shown_path(key)));
    editor.status.left.push(cues::KEYS.to_owned());
}

fn owns(editor: &Editor, key: &str, row: &ModelDef) -> Vec<Own> {
    let skeleton = super::built(editor, key).map(|(_, model)| model.skeleton());
    super::own_clips(editor, row)
        .into_iter()
        .enumerate()
        .map(|(index, (name, length))| {
            let replaces = skeleton.and_then(|skeleton| {
                let clip = skeleton.clips.get(&name)?;
                Some(skeleton.def.libraries[clip.library].key.clone())
            });
            let named: Vec<String> = skeleton
                .map(|skeleton| {
                    skeleton
                        .def
                        .sets()
                        .flat_map(|(set, clips)| {
                            clips
                                .entries()
                                .filter(|(_, clip)| *clip.clip() == name)
                                .map(move |(at, _)| format!("{set}{at}"))
                                .collect::<Vec<String>>()
                        })
                        .collect()
                })
                .unwrap_or_default();
            let strikes = skeleton.is_some_and(|skeleton| {
                skeleton
                    .def
                    .sets()
                    .any(|(_, set)| set.attacks().any(|(_, clip)| *clip.clip() == name))
            });
            Own {
                cues: row.overrides.get(&name).cloned().unwrap_or_default(),
                name,
                index,
                length,
                replaces,
                named,
                strikes,
            }
        })
        .collect()
}

fn note(own: &Own) -> String {
    let length = format!("{:.2} s", own.length.0 / 1000.0);
    match (&own.replaces, own.named.is_empty()) {
        (_, true) => format!("{length} · not named by any set · never plays"),
        (Some(library), false) => format!("{length} · replaces clips/{library}.glb's"),
        (None, false) => format!("{length} · named by {}", own.named.join(", ")),
    }
}
