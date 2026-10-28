//! The look page: the model on the stage in a chosen stance, playing what its skeleton gives a
//! chosen action, pose, move or gesture, its hitbox round it.

use engine::core::time::Millis;
use engine::systems::actor::model::ModelDef;
use engine::systems::actor::skeleton::{Clip, Cue};
use engine::systems::actor::stance::StanceDef;
use engine::systems::actor::{Action, Motion, Pose};
use strum::VariantArray;
use ui::egui::{self, Ui, Vec2};
use ui::{controls, form, theme};

use super::hitbox;
use crate::app::Editor;
use crate::tabs::shown_path;
use crate::views::area::viewport::StageClip;
use crate::views::cues;
use crate::views::stage;

/// What the stage plays: an engine action, a pose, a move or a gesture, as a stance resolves it.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub motion: Motion,
    pub pose: Pose,
}

impl Shown {
    pub fn label(&self) -> String {
        match (&self.motion, self.pose) {
            (Motion::Action(Action::Idle), pose) if pose != Pose::Idle => {
                format!("{} · pose", pose.as_ref())
            }
            (Motion::Action(action), _) => action.as_ref().to_owned(),
            (Motion::Move(name), _) => format!("{name} · move"),
            (Motion::Begin(pose), _) => format!("{} · taking it", pose.as_ref()),
            (Motion::End(pose), _) => format!("{} · leaving it", pose.as_ref()),
            (Motion::Gesture(name), _) => format!("{name} · gesture"),
        }
    }
}

/// What the stage plays for the model at `key` as its view asks: the clip, how long it plays and
/// its cues.
pub struct Playing {
    pub clip: StageClip,
    pub name: Clip,
    pub length: Millis,
    pub cues: Vec<Cue>,
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, row: &ModelDef) {
    let Some(scene) = Scene::of(editor, key) else {
        super::nothing_to_stand(ui);
        return;
    };
    let height = ui.available_height();
    let stage_height = (height * 0.5).max(140.0);
    pickers(editor, ui, key, &scene);
    let length = scene.playing.as_ref().map(|playing| playing.length);
    cues::advance(ui, &mut editor.views.model.view_mut(key).scrub, length);
    if let Some(turntable) = super::turntable(
        editor,
        ui,
        key,
        Vec2::new(ui.available_width(), stage_height),
    ) {
        let scrub = &editor.views.model.view(key).scrub;
        let playhead = scrub.playhead;
        let crossed = scene.playing.as_ref().map_or_else(Vec::new, |playing| {
            scrub.crossed(&playing.cues, playing.length)
        });
        let clip = scene.playing.as_ref().map(|playing| playing.clip.clone());
        hitbox(editor, ui, key, row, &turntable);
        super::stand(
            turntable,
            editor,
            ui,
            key,
            clip,
            Some(playhead.seconds()),
            crossed,
        );
    }
    ui.add_space(6.0);
    if let Some(playing) = &scene.playing {
        let playhead = editor.views.model.view(key).scrub.playhead;
        ui.label(theme::muted(format!(
            "{} → {} · {:.2} s",
            scene.shown.label(),
            playing.name.shown(),
            playing.length.0 / 1000.0
        )));
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), cues::STRIP_HEIGHT),
            egui::Sense::hover(),
        );
        cues::strip(
            ui.painter(),
            rect,
            playing.length,
            &playing.cues,
            Some(playhead),
            false,
        );
    } else {
        form::hint(ui, "Nothing in the skeleton's libraries plays this.");
    }
    editor
        .status
        .left
        .push(format!("{} · {}", shown_path(key), scene.shown.label()));
    editor.status.left.push(cues::KEYS.to_owned());
}

/// The model's hitbox round it on the stage, picked by a click for the inspector and sized by its
/// handles.
fn hitbox(editor: &mut Editor, ui: &Ui, key: &str, row: &ModelDef, turntable: &stage::Turntable) {
    let shown = editor.views.toggles.hitbox;
    let view = editor.views.model.view_mut(key);
    if !shown {
        view.hitbox.let_go();
        view.hitbox_chosen = false;
        return;
    }
    let touched = hitbox::overlay(
        ui,
        &turntable.canvas,
        &turntable.response,
        stage::center(),
        row.hitbox,
        view.hitbox_chosen,
        &mut view.hitbox,
    );
    if view.hitbox.dragging() {
        editor.views.toggles.turning = false;
    }
    if turntable.response.clicked() || touched.picked {
        view.hitbox_chosen = touched.picked;
    }
    if let Some(sized) = touched.sized {
        super::set_hitbox(editor, key, sized);
    }
}

/// What the page shows of the model as the game built it.
struct Scene {
    stances: Vec<(String, String)>,
    stance: Option<String>,
    choices: Vec<Shown>,
    shown: Shown,
    playing: Option<Playing>,
}

impl Scene {
    fn of(editor: &Editor, key: &str) -> Option<Scene> {
        let (content, model) = super::built(editor, key)?;
        let view = editor.views.model.view(key);
        let skeleton = model.skeleton();
        let stances: Vec<(String, String)> = skeleton
            .def
            .stances
            .iter()
            .map(|set| {
                let def = set.stance.get(content);
                (set.stance.path(content).to_owned(), def.name.clone())
            })
            .collect();
        let stance = view
            .stance
            .clone()
            .filter(|chosen| stances.iter().any(|(path, _)| path == chosen));
        let stance_id = stance
            .as_deref()
            .and_then(|path| content.by_path::<StanceDef>(path));
        let mut choices: Vec<Shown> = Action::VARIANTS
            .iter()
            .map(|action| Shown {
                motion: Motion::Action(*action),
                pose: Pose::Idle,
            })
            .collect();
        choices.extend(
            Pose::VARIANTS
                .iter()
                .filter(|pose| **pose != Pose::Idle)
                .flat_map(|pose| {
                    [
                        Motion::Begin(*pose),
                        Motion::Action(Action::Idle),
                        Motion::End(*pose),
                    ]
                    .into_iter()
                    .filter(|motion| model.plays(stance_id, motion))
                    .map(|motion| Shown {
                        pose: match motion {
                            Motion::Action(_) => *pose,
                            _ => Pose::Idle,
                        },
                        motion,
                    })
                }),
        );
        let mut moves: Vec<_> = skeleton.def.moves().cloned().collect();
        moves.sort();
        moves.dedup();
        choices.extend(moves.into_iter().map(|name| Shown {
            motion: Motion::Move(name),
            pose: Pose::Idle,
        }));
        let mut gestures: Vec<_> = skeleton.def.gestures().cloned().collect();
        gestures.sort();
        gestures.dedup();
        choices.extend(gestures.into_iter().map(|name| Shown {
            motion: Motion::Gesture(name),
            pose: Pose::Idle,
        }));
        let shown = Shown {
            motion: view.motion.clone(),
            pose: view.pose,
        };
        let resolved = model.resolve(stance_id, &shown.motion, shown.pose, 0);
        let playing = Some(Playing {
            clip: StageClip::played(skeleton, resolved.played),
            name: resolved.clip.clone(),
            length: resolved.length.millis(),
            cues: resolved.cues.to_vec(),
        });
        Some(Scene {
            stances,
            stance,
            choices,
            shown,
            playing,
        })
    }
}

fn pickers(editor: &mut Editor, ui: &mut Ui, key: &str, scene: &Scene) {
    let mut stance = scene.stance.clone();
    let mut shown = scene.shown.clone();
    ui.horizontal(|ui| {
        ui.label(theme::muted("stance"));
        let name = |path: &Option<String>| {
            path.as_ref()
                .and_then(|path| scene.stances.iter().find(|(known, _)| known == path))
                .map_or("base".to_owned(), |(_, name)| name.clone())
        };
        controls::combo(("model-stance", key))
            .selected_text(name(&stance))
            .width(160.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut stance, None, "base");
                for (path, name) in &scene.stances {
                    ui.selectable_value(&mut stance, Some(path.clone()), name);
                }
            });
        ui.add_space(8.0);
        ui.label(theme::muted("plays"));
        controls::combo(("model-shown", key))
            .selected_text(shown.label())
            .width(180.0)
            .show_ui(ui, |ui| {
                for choice in &scene.choices {
                    ui.selectable_value(&mut shown, choice.clone(), choice.label());
                }
            });
    });
    let view = editor.views.model.view_mut(key);
    if stance != scene.stance || shown != scene.shown {
        view.stance = stance;
        view.motion = shown.motion;
        view.pose = shown.pose;
        view.scrub.restart();
    }
}
