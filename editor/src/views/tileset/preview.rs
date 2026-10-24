//! Playback over a strip of frames, each shown a number of milliseconds, as the tileset's
//! animations preview.

use ui::chip;
use ui::egui;
use ui::egui::Ui;
use ui::glyph::{self, Mark};
use ui::theme;

use crate::keymap::EditorKey;

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

pub fn index_at(frames: &[u32], position: f32, holds: bool) -> usize {
    let total = total_ms(frames).max(1) as f32;
    let position = if holds {
        position.min(total - 1.0)
    } else {
        position.rem_euclid(total)
    };
    let mut cursor = 0.0;
    for (index, ms) in frames.iter().enumerate() {
        cursor += *ms as f32;
        if position < cursor {
            return index;
        }
    }
    frames.len().saturating_sub(1)
}

/// Advances the playback by `dt` seconds over `frames`; a strip that `holds` stops on its last
/// frame instead of looping.
pub fn tick(preview: &mut StripPreview, dt: f32, frames: &[u32], holds: bool) -> PreviewTick {
    let total = total_ms(frames).max(1) as f32;
    let before = preview.position;
    if preview.playing {
        preview.position += dt * 1000.0 * preview.speed;
    }
    let mut passed = Vec::new();
    if preview.playing && !frames.is_empty() {
        let after = preview.position;
        let starts: Vec<f32> = (0..frames.len())
            .map(|index| starts_at(frames, index) as f32)
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

pub fn keys(ui: &Ui, preview: &mut StripPreview, frames: &[u32]) {
    if ui.ctx().egui_wants_keyboard_input() {
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
        let total = total_ms(frames).max(1) as f32;
        let index = index_at(frames, preview.position, false);
        let next = if period {
            (index + 1) % frames.len()
        } else {
            (index + frames.len() - 1) % frames.len()
        };
        preview.position = (starts_at(frames, next) as f32).min(total - 1.0);
    }
}

fn total_ms(frames: &[u32]) -> u32 {
    frames.iter().sum()
}

fn starts_at(frames: &[u32], index: usize) -> u32 {
    frames.iter().take(index).sum()
}
