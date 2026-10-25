//! The clips page: every action's and pose's clip with the moments on it, the chosen one laid
//! out large with its cues to drag, and the rig on the stage held at the playhead.

use engine::core::time::Millis;
use engine::systems::actor::model::{Clip, Cue, ModelDef};
use engine::systems::actor::{Action, Pose};
use strum::VariantArray;
use ui::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use ui::glyph::Mark;
use ui::{chip, controls, form, theme};

use crate::app::Editor;
use crate::keymap::EditorKey;

/// How far a step key moves the playhead.
pub const STEP: Millis = Millis(10.0);
/// How close to a cue, in points, the pointer takes it.
const GRIP: f32 = 8.0;
const STRIP_HEIGHT: f32 = 18.0;
const TIMELINE_HEIGHT: f32 = 64.0;

/// A clip's place in the model: the clip of an action or of a pose.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClipSlot {
    Action(Action),
    Pose(Pose),
}

impl Default for ClipSlot {
    fn default() -> ClipSlot {
        ClipSlot::Action(Action::Idle)
    }
}

impl ClipSlot {
    pub fn label(self) -> String {
        match self {
            ClipSlot::Action(action) => action.as_ref().to_owned(),
            ClipSlot::Pose(pose) => format!("{} pose", pose.as_ref()),
        }
    }

    pub fn clip(self, model: &ModelDef) -> Option<&Clip> {
        match self {
            ClipSlot::Action(action) => model.clips.get(&action),
            ClipSlot::Pose(pose) => model.poses.get(&pose),
        }
    }

    pub fn set(self, model: &mut ModelDef, clip: Clip) {
        match self {
            ClipSlot::Action(action) => {
                model.clips.insert(action, clip);
            }
            ClipSlot::Pose(pose) => {
                model.poses.insert(pose, clip);
            }
        }
    }

    pub fn takes_apex(self) -> bool {
        matches!(self, ClipSlot::Action(action) if action.takes_apex())
    }
}

/// Every slot the model fills, actions first.
pub fn slots(model: &ModelDef) -> Vec<ClipSlot> {
    Action::VARIANTS
        .iter()
        .map(|action| ClipSlot::Action(*action))
        .chain(model.poses.keys().map(|pose| ClipSlot::Pose(*pose)))
        .collect()
}

/// A cue being dragged along the timeline, and the cues as the drag has placed them.
pub struct CueDrag {
    pub cue: usize,
    pub cues: Vec<Cue>,
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, row: &ModelDef) {
    let names = super::clip_names(editor, row);
    let height = ui.available_height();
    let stage_height = (height * 0.42).max(120.0);
    let slot = editor.views.model.view(key).slot;
    let shown = slot.clip(row).map(|clip| clip.name.clone());
    let index = shown
        .as_ref()
        .and_then(|name| names.iter().position(|other| other == name));
    let length = shown
        .as_ref()
        .and_then(|name| super::clip_length(editor, row, name));
    advance(editor, ui, key, length);
    if let Some(turntable) = super::turntable(
        editor,
        ui,
        key,
        Vec2::new(ui.available_width(), stage_height),
    ) {
        let playhead = editor.views.model.view(key).playhead;
        turntable.stand(editor, ui, row, index, Some(playhead.seconds()));
    } else {
        super::nothing_to_stand(ui);
    }
    ui.add_space(6.0);
    egui::ScrollArea::vertical()
        .id_salt(("clips", key))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            rows(editor, ui, key, row, &names);
            ui.add_space(8.0);
            if let (Some(clip), Some(length)) = (slot.clip(row), length) {
                let clip = clip.clone();
                timeline(editor, ui, key, slot, &clip, length);
            } else if shown.is_some() {
                form::hint(
                    ui,
                    "This clip is not in the model's glTF: pick one that is.",
                );
            }
        });
    editor.status.left.push(format!(
        "{} · {} · {}",
        crate::tabs::shown_path(key),
        slot.label(),
        shown.as_deref().unwrap_or("no clip")
    ));
    editor.status.left.push(
        "Space plays · , . step 10 ms · A apex here · S step here · drag a cue · right-click \
         removes it"
            .to_owned(),
    );
}

/// Moves the playhead on while the clip plays, and as the keys ask.
fn advance(editor: &mut Editor, ui: &mut Ui, key: &str, length: Option<Millis>) {
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    let typing = ui.ctx().text_edit_focused();
    let (play, back, on) = if typing {
        (false, false, false)
    } else {
        ui.input_mut(|i| {
            (
                EditorKey::PlayPause.consume(i),
                EditorKey::StepBack.consume(i),
                EditorKey::StepOn.consume(i),
            )
        })
    };
    let view = editor.views.model.view_mut(key);
    let Some(length) = length else {
        view.playing = false;
        return;
    };
    if play {
        view.playing = !view.playing;
    }
    if back {
        view.playing = false;
        view.playhead = Millis((view.playhead.0 - STEP.0).max(0.0));
    }
    if on {
        view.playing = false;
        view.playhead = Millis((view.playhead.0 + STEP.0).min(length.0));
    }
    if view.playing {
        view.playhead = Millis((view.playhead.0 + dt * 1000.0) % length.0.max(1.0));
        ui.ctx().request_repaint();
    }
    view.playhead = Millis(view.playhead.0.min(length.0));
}

fn rows(editor: &mut Editor, ui: &mut Ui, key: &str, row: &ModelDef, names: &[String]) {
    let selected = editor.views.model.view(key).slot;
    let mut pick: Option<ClipSlot> = None;
    let mut play: Option<(ClipSlot, String)> = None;
    let mut drop: Option<ClipSlot> = None;
    egui::Grid::new(("clip-rows", key))
        .num_columns(4)
        .spacing(Vec2::new(10.0, 6.0))
        .show(ui, |ui| {
            for slot in slots(row) {
                let clip = slot.clip(row);
                let name = clip.map(|clip| clip.name.clone()).unwrap_or_default();
                let length = super::clip_length(editor, row, &name);
                let is_selected = slot == selected;
                if chip::mode(ui, slot.label(), is_selected).clicked() {
                    pick = Some(slot);
                }
                let mut chosen = name.clone();
                controls::combo(("clip-of", key, slot.label()))
                    .selected_text(if chosen.is_empty() {
                        "pick a clip".to_owned()
                    } else {
                        chosen.clone()
                    })
                    .width(160.0)
                    .show_ui(ui, |ui| {
                        for other in names {
                            ui.selectable_value(&mut chosen, other.clone(), other);
                        }
                    });
                if chosen != name {
                    play = Some((slot, chosen));
                }
                match length {
                    Some(length) => {
                        ui.label(theme::muted(format!("{:.2} s", length.0 / 1000.0)));
                    }
                    None => {
                        ui.colored_label(theme::DANGER, "not in the glTF");
                    }
                }
                ui.horizontal(|ui| {
                    let (rect, response) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width().max(120.0) - 30.0, STRIP_HEIGHT),
                        Sense::click(),
                    );
                    if let (Some(clip), Some(length)) = (clip, length) {
                        strip(ui.painter(), rect, length, &clip.cues, None, is_selected);
                    } else {
                        ui.painter().rect_filled(rect, 3.0, theme::INPUT);
                    }
                    if response.clicked() {
                        pick = Some(slot);
                    }
                    if let ClipSlot::Pose(_) = slot
                        && ui::glyph::button(ui, Mark::Cross, 18.0, theme::MUTED, "Drop the pose")
                            .on_hover_text("Drop this pose")
                            .clicked()
                    {
                        drop = Some(slot);
                    }
                });
                ui.end_row();
            }
        });
    let missing: Vec<Pose> = Pose::VARIANTS
        .iter()
        .copied()
        .filter(|pose| *pose != Pose::Idle && !row.poses.contains_key(pose))
        .collect();
    if !missing.is_empty() {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for pose in missing {
                if controls::button(ui, format!("+ {} pose", pose.as_ref())).clicked()
                    && let Some(name) = names.first()
                {
                    play = Some((ClipSlot::Pose(pose), name.clone()));
                    pick = Some(ClipSlot::Pose(pose));
                }
            }
        });
    }
    if let Some((slot, name)) = play {
        super::set_clip(editor, key, slot, &name);
    }
    if let Some(slot) = drop {
        super::remove_clip(editor, key, slot);
        if selected == slot {
            pick = Some(ClipSlot::default());
        }
    }
    if let Some(slot) = pick {
        let view = editor.views.model.view_mut(key);
        if view.slot != slot {
            view.slot = slot;
            view.playhead = Millis(0.0);
            view.cue_drag = None;
        }
    }
}

/// The chosen clip laid out large: cues to drag, the playhead to scrub, and the keys that mark
/// moments.
fn timeline(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    slot: ClipSlot,
    clip: &Clip,
    length: Millis,
) {
    ui.label(theme::muted(format!(
        "{} · {} · {:.2} s",
        slot.label(),
        clip.name,
        length.0 / 1000.0
    )));
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), TIMELINE_HEIGHT),
        Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    let view = editor.views.model.view_mut(key);
    let working: Vec<Cue> = view
        .cue_drag
        .as_ref()
        .map_or_else(|| clip.cues.clone(), |drag| drag.cues.clone());
    strip(&painter, rect, length, &working, Some(view.playhead), true);
    let pointer = response
        .hover_pos()
        .or_else(|| response.interact_pointer_pos());
    let at_pointer = pointer.map(|pointer| time_at(rect, length, pointer.x));
    let hot = pointer.and_then(|pointer| {
        working
            .iter()
            .position(|cue| (x_of(rect, length, cue.at) - pointer.x).abs() <= GRIP)
    });
    for (index, cue) in working.iter().enumerate() {
        let lit =
            hot == Some(index) || view.cue_drag.as_ref().is_some_and(|drag| drag.cue == index);
        if lit {
            let x = x_of(rect, length, cue.at);
            painter.circle_stroke(
                Pos2::new(x, rect.center().y),
                GRIP,
                Stroke::new(1.0, theme::ACCENT),
            );
        }
    }
    let mut landed: Option<Vec<Cue>> = None;
    if response.drag_started_by(egui::PointerButton::Primary)
        && let Some(index) = hot
    {
        view.cue_drag = Some(CueDrag {
            cue: index,
            cues: working.clone(),
        });
    }
    if response.dragged_by(egui::PointerButton::Primary) {
        match (&mut view.cue_drag, at_pointer) {
            (Some(drag), Some(at)) => {
                drag.cues[drag.cue].at = at;
            }
            (None, Some(at)) => {
                view.playing = false;
                view.playhead = at;
            }
            _ => {}
        }
        ui.ctx().request_repaint();
    }
    if response.clicked()
        && let Some(at) = at_pointer
    {
        view.playing = false;
        view.playhead = at;
    }
    if response.drag_stopped()
        && let Some(drag) = view.cue_drag.take()
    {
        landed = Some(drag.cues);
    }
    if response.secondary_clicked()
        && let Some(index) = hot
    {
        let mut cues = working.clone();
        cues.remove(index);
        landed = Some(cues);
    }
    let typing = ui.ctx().text_edit_focused();
    let (apex, step) = if typing {
        (false, false)
    } else {
        ui.input_mut(|i| (EditorKey::ApexCue.consume(i), EditorKey::StepCue.consume(i)))
    };
    let playhead = view.playhead;
    if apex && slot.takes_apex() {
        let mut cues = working.clone();
        match cues.iter_mut().find(|cue| cue.apex) {
            Some(cue) => cue.at = playhead,
            None => cues.push(Cue {
                at: playhead,
                step: false,
                apex: true,
                sound: None,
            }),
        }
        landed = Some(cues);
    }
    if step {
        let mut cues = working.clone();
        match cues
            .iter()
            .position(|cue| cue.step && (cue.at.0 - playhead.0).abs() <= STEP.0 * 1.5)
        {
            Some(index) => {
                let cue = &mut cues[index];
                cue.step = false;
                if !cue.apex && cue.sound.is_none() {
                    cues.remove(index);
                }
            }
            None => cues.push(Cue {
                at: playhead,
                step: true,
                apex: false,
                sound: None,
            }),
        }
        landed = Some(cues);
    }
    if let Some(cues) = landed {
        super::set_cues(editor, key, slot, cues);
    }
    ui.add_space(4.0);
    form::hint(
        ui,
        &format!("{:.0} ms · {} cues", playhead.0, working.len()),
    );
}

/// A clip's length as a band, with its cues marked: an apex as a triangle, a step as a dot, a
/// sound as a note.
fn strip(
    painter: &egui::Painter,
    rect: Rect,
    length: Millis,
    cues: &[Cue],
    playhead: Option<Millis>,
    selected: bool,
) {
    painter.rect_filled(rect, 3.0, theme::INPUT);
    painter.rect_stroke(
        rect,
        3.0,
        Stroke::new(
            1.0,
            if selected {
                theme::ACCENT
            } else {
                theme::BORDER
            },
        ),
        egui::StrokeKind::Inside,
    );
    let ticks = (length.0 / 100.0).floor() as usize;
    for tick in 1..=ticks {
        let x = x_of(rect, length, Millis(tick as f32 * 100.0));
        let tall = tick % 10 == 0;
        painter.line_segment(
            [
                Pos2::new(x, rect.bottom() - if tall { 8.0 } else { 4.0 }),
                Pos2::new(x, rect.bottom()),
            ],
            Stroke::new(1.0, theme::BORDER),
        );
    }
    for cue in cues {
        let x = x_of(rect, length, cue.at);
        let middle = rect.center().y;
        painter.line_segment(
            [
                Pos2::new(x, rect.top() + 2.0),
                Pos2::new(x, rect.bottom() - 2.0),
            ],
            Stroke::new(1.0, ui::ink::wash(cue_color(cue), 0.6)),
        );
        if cue.apex {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(x - 5.0, middle + 5.0),
                    Pos2::new(x + 5.0, middle + 5.0),
                    Pos2::new(x, middle - 5.0),
                ],
                theme::DANGER,
                Stroke::NONE,
            ));
        }
        if cue.step {
            painter.circle_filled(Pos2::new(x, middle), 3.5, theme::TEXT);
        }
        if cue.sound.is_some() {
            Mark::Music.paint(
                painter,
                Rect::from_center_size(Pos2::new(x, rect.top() + 7.0), Vec2::splat(10.0)),
                theme::OK,
            );
        }
    }
    if let Some(playhead) = playhead {
        let x = x_of(rect, length, playhead);
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(2.0, theme::ACCENT),
        );
    }
}

fn cue_color(cue: &Cue) -> Color32 {
    if cue.apex {
        theme::DANGER
    } else if cue.sound.is_some() {
        theme::OK
    } else {
        theme::TEXT
    }
}

fn x_of(rect: Rect, length: Millis, at: Millis) -> f32 {
    rect.left() + 6.0 + (rect.width() - 12.0) * (at.0 / length.0.max(1.0)).clamp(0.0, 1.0)
}

fn time_at(rect: Rect, length: Millis, x: f32) -> Millis {
    let ratio = ((x - rect.left() - 6.0) / (rect.width() - 12.0).max(1.0)).clamp(0.0, 1.0);
    Millis((ratio * length.0 / STEP.0).round() * STEP.0)
}
