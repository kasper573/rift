//! The fit page: the model on the stage in a chosen stance, playing what its skeleton gives a
//! chosen action, pose or move, and every joint of the skeleton's rig as the model's glb has it.

use engine::core::content::media::GltfInfos;
use engine::core::render::world::mannequin::RigClip;
use engine::core::time::Millis;
use engine::systems::actor::model::{ClipSource, fit_report};
use engine::systems::actor::skeleton::{ClipName, Cue};
use engine::systems::actor::stance::StanceDef;
use engine::systems::actor::{Action, ModelDef, Motion, Pose};
use strum::VariantArray;
use ui::egui::{self, Ui, Vec2};
use ui::{controls, form, theme};

use crate::app::Editor;
use crate::tabs::shown_path;
use crate::views::cues;

/// What the stage plays: an engine action, a pose or a move, as a stance resolves it.
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
        }
    }
}

/// What the stage plays for the model at `key` as its view asks: the clip, where it comes from,
/// how long it plays and its cues.
pub struct Playing {
    pub clip: RigClip,
    pub name: ClipName,
    pub own: bool,
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
        let playhead = editor.views.model.view(key).scrub.playhead;
        let clip = scene.playing.as_ref().map(|playing| playing.clip.clone());
        turntable.stand(editor, ui, row, clip, Some(playhead.seconds()));
    }
    ui.add_space(6.0);
    if let Some(playing) = &scene.playing {
        let playhead = editor.views.model.view(key).scrub.playhead;
        ui.label(theme::muted(format!(
            "{} → {}{} · {:.2} s",
            scene.shown.label(),
            playing.name,
            if playing.own { " (own)" } else { "" },
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
        form::hint(
            ui,
            "Nothing in the skeleton's libraries or the model plays this.",
        );
    }
    ui.add_space(8.0);
    egui::ScrollArea::vertical()
        .id_salt(("fit", key))
        .auto_shrink([false, false])
        .show(ui, |ui| joints(ui, &scene));
    editor
        .status
        .left
        .push(format!("{} · {}", shown_path(key), scene.shown.label()));
    editor.status.left.push(cues::KEYS.to_owned());
}

/// What the page shows of the model as the game built it.
struct Scene {
    stances: Vec<(String, String)>,
    stance: Option<String>,
    choices: Vec<Shown>,
    shown: Shown,
    playing: Option<Playing>,
    rows: Vec<JointRow>,
}

/// A joint of the rig as the model has it.
struct JointRow {
    name: String,
    depth: usize,
    hand: Option<String>,
    fits: Result<String, String>,
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
                .map(|pose| Shown {
                    motion: Motion::Action(Action::Idle),
                    pose: *pose,
                }),
        );
        let mut moves: Vec<_> = skeleton.def.moves().cloned().collect();
        moves.sort();
        moves.dedup();
        choices.extend(moves.into_iter().map(|name| Shown {
            motion: Motion::Move(name),
            pose: Pose::Idle,
        }));
        let shown = Shown {
            motion: view.motion.clone(),
            pose: view.pose,
        };
        let resolved = model.resolve(stance_id, &shown.motion, shown.pose);
        let playing = resolved.source.map(|source| Playing {
            clip: model.rig_clip(source),
            name: resolved.clip.clone(),
            own: matches!(source, ClipSource::Model { .. }),
            length: resolved.length.millis(),
            cues: resolved.cues.to_vec(),
        });
        let info = content.built::<GltfInfos>().0.get(&model.model().path())?;
        let skeleton_path = model.skeleton_id().path(content);
        let report = fit_report(skeleton, skeleton_path, &info.tree);
        let rig = &skeleton.rig;
        let rows = rig
            .joints()
            .map(|joint| {
                let name = rig.nodes[joint].name.clone();
                let depth = rig.path(joint).len() - 1;
                let hand = skeleton
                    .def
                    .hands
                    .iter()
                    .find(|hand| hand.joint.0 == name)
                    .map(|hand| hand.name.clone());
                let misfit = report
                    .fit
                    .misfits
                    .iter()
                    .position(|misfit| misfit.joint() == name);
                let fits = match misfit {
                    Some(index) => Err(report.problems[index].found.clone()),
                    None => match info.tree.at_path(&rig.path(joint)) {
                        Some(found) => {
                            let turn = rig.nodes[joint]
                                .rest
                                .rotation
                                .angle_between(info.tree.nodes[found].rest.rotation)
                                .to_degrees();
                            Ok(format!("rests within {turn:.1}°"))
                        }
                        None => Err("under a joint that does not fit".to_owned()),
                    },
                };
                JointRow {
                    name,
                    depth,
                    hand,
                    fits,
                }
            })
            .collect();
        Some(Scene {
            stances,
            stance,
            choices,
            shown,
            playing,
            rows,
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

fn joints(ui: &mut Ui, scene: &Scene) {
    let unfit = scene.rows.iter().filter(|row| row.fits.is_err()).count();
    form::section_count(ui, "Joints of the rig", scene.rows.len());
    if unfit > 0 {
        ui.colored_label(
            theme::DANGER,
            format!("{unfit} joints do not fit: the loader refuses the model"),
        );
    }
    egui::Grid::new("fit-joints")
        .num_columns(2)
        .spacing(Vec2::new(12.0, 2.0))
        .show(ui, |ui| {
            for row in &scene.rows {
                ui.horizontal(|ui| {
                    ui.add_space(row.depth as f32 * 12.0);
                    ui.label(theme::mono(&row.name).size(12.5));
                    if let Some(hand) = &row.hand {
                        ui.label(theme::faint(format!("hand · {hand}")).size(12.0));
                    }
                });
                match &row.fits {
                    Ok(note) => ui.label(theme::faint(note).size(12.0)),
                    Err(problem) => ui.colored_label(theme::DANGER, problem),
                };
                ui.end_row();
            }
        });
}
