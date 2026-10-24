use engine::core::time::Millis;
use engine::systems::actor::model::SheetFrame;
use engine::systems::actor::{Action, ModelDef};
use ui::chip;
use ui::egui;
use ui::egui::Ui;
use ui::glyph::{self, Mark};
use ui::theme;

use super::ModelStrip;
use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::stage::{self, Posed, PosedActor, Scene};

pub struct StripPreview {
    pub playing: bool,
    pub muted: bool,
    pub speed: f32,
    pub position: f32,
    pub speeds: usize,
}

impl Default for StripPreview {
    fn default() -> StripPreview {
        StripPreview {
            playing: true,
            muted: false,
            speed: 1.0,
            position: 0.0,
            speeds: 0,
        }
    }
}

pub const SPEEDS: [f32; 4] = [1.0, 0.5, 0.25, 2.0];

pub struct PreviewTick {
    pub index: usize,
    pub passed: Vec<usize>,
}

pub fn index_at(frames: &[SheetFrame], position: f32, holds: bool) -> usize {
    let total = super::total_ms(frames).max(1) as f32;
    let position = if holds {
        position.min(total - 1.0)
    } else {
        position.rem_euclid(total)
    };
    let mut cursor = 0.0;
    for (index, frame) in frames.iter().enumerate() {
        cursor += frame.ms as f32;
        if position < cursor {
            return index;
        }
    }
    frames.len().saturating_sub(1)
}

pub fn tick(
    preview: &mut StripPreview,
    dt: f32,
    frames: &[SheetFrame],
    action: Action,
) -> PreviewTick {
    let total = super::total_ms(frames).max(1) as f32;
    let holds = action == Action::Dead;
    let before = preview.position;
    if preview.playing {
        preview.position += dt * 1000.0 * preview.speed;
    }
    let mut passed = Vec::new();
    if preview.playing && !frames.is_empty() {
        let after = preview.position;
        let starts: Vec<f32> = (0..frames.len())
            .map(|index| super::starts_at(frames, index) as f32)
            .collect();
        if holds {
            for (index, start) in starts.iter().enumerate() {
                if *start > before && *start <= after && before < total {
                    passed.push(index);
                }
            }
            if before < total && after >= total {
                preview.position = total;
            }
        } else {
            let from_lap = (before / total).floor();
            let to_lap = (after / total).floor();
            let mut lap = from_lap;
            while lap <= to_lap {
                for (index, start) in starts.iter().enumerate() {
                    let at = lap * total + *start;
                    if at > before && at <= after {
                        passed.push(index);
                    }
                }
                lap += 1.0;
            }
            preview.position = after.rem_euclid(total);
        }
    }
    PreviewTick {
        index: index_at(frames, preview.position, holds),
        passed,
    }
}

pub fn restart(preview: &mut StripPreview) {
    preview.position = 0.0;
    preview.playing = true;
}

pub fn fire(editor: &mut Editor, key: &str, frames: &[SheetFrame], passed: &[usize]) {
    let muted = editor
        .views
        .model
        .views
        .get(key)
        .is_some_and(|view| view.preview.muted);
    if muted {
        return;
    }
    let surface = editor
        .views
        .model
        .views
        .get(key)
        .and_then(|view| view.surface.clone());
    let mut stepped = false;
    for index in passed {
        let Some(frame) = frames.get(*index) else {
            continue;
        };
        if let Some(sound) = frame.sound
            && let Some(reader) = editor.reader()
            && let Some(sound) = reader.path_of(sound)
        {
            editor.views.sound.play.push(sound.to_owned());
        }
        if frame.step && !stepped {
            stepped = true;
            if let Some(footstep) = super::sounds::footstep_of(editor, surface.as_deref()) {
                editor.views.sound.play.push(footstep);
            }
        }
    }
}

/// The strip's frame at `at` into it, drawn as the game draws the model on its starting ground.
pub fn draw(editor: &mut Editor, ui: &mut Ui, key: &str, strip: ModelStrip, at: Millis) {
    let Some(content) = editor.loaded() else {
        return;
    };
    let Some(model) = content.by_path::<ModelDef>(key) else {
        return;
    };
    let posed = Posed {
        actors: vec![PosedActor {
            action: strip.action,
            facing: stage::facing_of(strip.direction),
            held: Some(at.seconds()),
            ..PosedActor::standing(model, stage::center(&content))
        }],
        ..Posed::default()
    };
    stage::show(
        editor,
        ui,
        &format!("model/{key}/strip"),
        Scene::Posed(posed),
        true,
    );
}

pub fn controls(
    ui: &mut Ui,
    preview: &mut StripPreview,
    index: usize,
    count: usize,
    with_speed: bool,
) {
    ui.horizontal(|ui| {
        let (mark, word) = if preview.playing {
            (Mark::Pause, "Pause")
        } else {
            (Mark::Play, "Play")
        };
        if glyph::button(ui, mark, 14.0, theme::TEXT, word)
            .on_hover_text(EditorKey::PlayPause.name())
            .clicked()
        {
            preview.playing = !preview.playing;
        }
        let (mark, word) = if preview.muted {
            (Mark::Mute, "Unmute")
        } else {
            (Mark::Speaker, "Mute")
        };
        if glyph::button(ui, mark, 14.0, theme::TEXT, word)
            .on_hover_text(format!("{} mutes the preview", EditorKey::Mute.name()))
            .clicked()
        {
            preview.muted = !preview.muted;
        }
        if with_speed && chip::plain(ui, format!("{}×", preview.speed)).clicked() {
            preview.speeds = (preview.speeds + 1) % SPEEDS.len();
            preview.speed = SPEEDS[preview.speeds];
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if count > 0 {
                chip::plain(ui, format!("{} / {count}", index + 1));
            }
        });
    });
}

pub fn keys(ui: &Ui, preview: &mut StripPreview, frames: &[SheetFrame]) {
    if !super::keyboard_free(ui) {
        return;
    }
    let (space, comma, period, mute) = ui.input(|i| {
        (
            EditorKey::PlayPause.pressed(i),
            EditorKey::PreviousFrame.pressed(i),
            EditorKey::NextFrame.pressed(i),
            EditorKey::Mute.pressed(i),
        )
    });
    if space {
        preview.playing = !preview.playing;
    }
    if mute {
        preview.muted = !preview.muted;
    }
    if (comma || period) && !frames.is_empty() {
        preview.playing = false;
        let total = super::total_ms(frames).max(1) as f32;
        let index = index_at(frames, preview.position, false);
        let next = if period {
            (index + 1) % frames.len()
        } else {
            (index + frames.len() - 1) % frames.len()
        };
        preview.position = (super::starts_at(frames, next) as f32).min(total - 1.0);
    }
}
