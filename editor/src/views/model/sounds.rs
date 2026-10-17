use bevy::prelude::World;
use bevy_kira_audio::prelude::{Audio, AudioControl, AudioSource};
use egui::{Color32, Rect as ERect, Sense, Ui, Vec2};
use engine::core::audio;
use engine::core::audio::loader::{self, DecodedAudio};
use engine::core::audio::mix::{AudioCategory, AudioMix};
use engine::core::audio::playback::{SfxRoleDef, SfxScalar};
use engine::core::content::ContentRow;
use engine::core::math::Rng;

use crate::app::Editor;
use crate::audio::{LOUDEST_WINDOW_MS, Loudness, Waveform, gain_db};
use crate::project::{ContentEdit, row_file};
use crate::ui::{theme, widgets};

pub const PEAK_CEILING_DBFS: f64 = -12.0;

pub fn wave_of_sfx(editor: &mut Editor, sfx: &str) -> Option<Waveform> {
    let path = sfx_file(editor, sfx)?;
    let Editor { project, views, .. } = editor;
    let project = project.as_ref()?;
    views.model.bank.wave(&project.files, &path)
}

pub fn sfx_file(editor: &Editor, sfx: &str) -> Option<String> {
    editor
        .row_value("sfx", sfx)
        .and_then(|row| row["file"].as_str().map(str::to_owned))
}

pub fn draw_wave(painter: &egui::Painter, rect: ERect, bins: &[f32], color: Color32) {
    if bins.is_empty() || rect.width() <= 0.0 {
        return;
    }
    let mid = rect.center().y;
    let half = rect.height() / 2.0;
    let mut top = Vec::with_capacity(bins.len());
    let mut bottom = Vec::with_capacity(bins.len());
    for (index, amplitude) in bins.iter().enumerate() {
        let x = rect.min.x + rect.width() * index as f32 / (bins.len() - 1).max(1) as f32;
        let h = (amplitude.min(1.0) * half).max(0.5);
        top.push(egui::pos2(x, mid - h));
        bottom.push(egui::pos2(x, mid + h));
    }
    bottom.reverse();
    top.extend(bottom);
    painter.add(egui::Shape::convex_polygon(
        top,
        color.gamma_multiply(0.85),
        egui::Stroke::NONE,
    ));
}

pub struct SfxRole {
    pub key: String,
    pub def: SfxRoleDef,
}

pub fn sfx_roles(editor: &Editor) -> Vec<SfxRole> {
    let Some(project) = editor.project.as_ref() else {
        return Vec::new();
    };
    let mut keys: Vec<&str> = project.files.keys_of(SfxRoleDef::TABLE).collect();
    keys.sort_unstable();
    keys.into_iter()
        .filter_map(|key| sfx_role(editor, key))
        .collect()
}

pub fn role_of_sfx(editor: &Editor, sfx: &str) -> Option<SfxRole> {
    let row = editor.row_value("sfx", sfx)?;
    sfx_role(editor, row["role"].as_str()?)
}

fn sfx_role(editor: &Editor, key: &str) -> Option<SfxRole> {
    let row = editor.row_value(SfxRoleDef::TABLE, key)?;
    Some(SfxRole {
        key: key.to_owned(),
        def: serde_json::from_value(row).ok()?,
    })
}

pub struct LoudnessVerdict {
    pub ok: bool,
    pub loudness: String,
    pub peak_ok: bool,
    pub peak: String,
}

pub fn verdict(loudness: Loudness, gain: f32, role: &SfxRoleDef) -> LoudnessVerdict {
    let level = loudness.loudest_at(gain);
    let target = role.loudness.0;
    let off = level - target;
    let ok = off.abs() <= 1.0;
    let loudness_text = if ok {
        format!(
            "{level:.1} LUFS over its loudest {LOUDEST_WINDOW_MS} ms, at the {} level",
            role.name
        )
    } else if off > 0.0 {
        format!(
            "{level:.1} LUFS over its loudest {LOUDEST_WINDOW_MS} ms, {off:.1} LU over {target:.0}",
        )
    } else {
        format!(
            "{level:.1} LUFS over its loudest {LOUDEST_WINDOW_MS} ms, {:.1} LU under {target:.0}",
            -off
        )
    };
    let peak = loudness.peak_dbfs + gain_db(gain);
    let peak_ok = peak <= PEAK_CEILING_DBFS;
    let peak_text = if peak_ok {
        format!("{peak:.1} dBFS, under the {PEAK_CEILING_DBFS:.0} ceiling")
    } else {
        format!("{peak:.1} dBFS, over the {PEAK_CEILING_DBFS:.0} ceiling")
    };
    LoudnessVerdict {
        ok,
        loudness: loudness_text,
        peak_ok,
        peak: peak_text,
    }
}

pub fn first_sfx(editor: &Editor) -> Option<String> {
    sfx_keys(editor).into_iter().next()
}

pub fn sfx_keys(editor: &Editor) -> Vec<String> {
    let Some(project) = editor.project.as_ref() else {
        return Vec::new();
    };
    let mut keys: Vec<String> = project.files.keys_of("sfx").map(str::to_owned).collect();
    keys.sort();
    keys
}

pub struct FootstepSurface {
    pub key: String,
    pub name: String,
    pub footstep: String,
}

pub fn surfaces(editor: &Editor) -> Vec<FootstepSurface> {
    let Some(project) = editor.project.as_ref() else {
        return Vec::new();
    };
    let mut keys: Vec<&str> = project.files.keys_of("surface").collect();
    keys.sort_unstable();
    keys.into_iter()
        .filter_map(|key| {
            let row = project.files.json(&row_file("surface", key))?;
            Some(FootstepSurface {
                key: key.to_owned(),
                name: row["name"].as_str().unwrap_or(key).to_owned(),
                footstep: row["footstep"].as_str()?.to_owned(),
            })
        })
        .collect()
}

pub fn footstep_of(editor: &Editor, surface: Option<&str>) -> Option<String> {
    let surfaces = surfaces(editor);
    let chosen = match surface {
        Some(key) => surfaces.iter().find(|s| s.key == key),
        None => surfaces.first(),
    };
    chosen.map(|surface| surface.footstep.clone())
}

pub fn bottom(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let keys = sfx_keys(editor);
    let roles = sfx_roles(editor);
    let (search, filter) = {
        let view = &editor.views.model.view(key);
        (view.sound_search.clone(), view.sound_role.clone())
    };
    let mut search_text = search.clone();
    let mut new_filter = filter.clone();
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.allocate_ui(Vec2::new(220.0, 26.0), |ui| {
            widgets::search_box(ui, &mut search_text, "Search sounds", None);
        });
        let options = roles
            .iter()
            .map(|role| (Some(role.key.clone()), role.def.name.to_lowercase()))
            .chain([(None, format!("all · {}", keys.len()))]);
        for (option, label) in options {
            let on = filter == option;
            if widgets::toggle_chip(ui, &label, on).clicked() {
                new_filter = option;
            }
        }
    });
    let mut listed: Vec<(String, Option<Waveform>)> = Vec::new();
    for sfx in keys
        .iter()
        .filter(|sfx| search.is_empty() || sfx.contains(&search.to_lowercase()))
    {
        if sfx_file(editor, sfx).is_none() {
            continue;
        }
        if let Some(wanted) = &filter
            && role_of_sfx(editor, sfx).is_none_or(|role| &role.key != wanted)
        {
            continue;
        }
        listed.push((sfx.clone(), wave_of_sfx(editor, sfx)));
    }
    let view = editor.views.model.view(key);
    let (selected_cue, strip) = (view.cue, view.strip);
    let current_sound = selected_cue
        .filter(|(_, kind)| *kind == super::FrameCueKind::Sound)
        .and_then(|(index, _)| {
            let row = editor.row_value(super::TABLE, key)?;
            super::strip_of(&row, strip).get(index)?.sound.clone()
        });
    let mut play = None;
    let mut choose = None;
    egui::ScrollArea::vertical()
        .id_salt(("sounds", key))
        .show(ui, |ui| {
            ui.add_space(4.0);
            let columns = ((ui.available_width() - 16.0) / 360.0).floor().max(1.0) as usize;
            egui::Grid::new(("sound-rows", key))
                .spacing(Vec2::new(16.0, 4.0))
                .show(ui, |ui| {
                    for (index, (sfx, wave)) in listed.iter().enumerate() {
                        let on = current_sound.as_deref() == Some(sfx);
                        let response = sound_row(ui, sfx, wave.as_ref(), on, &mut play);
                        if response.clicked() {
                            choose = Some(sfx.clone());
                        }
                        if (index + 1) % columns == 0 {
                            ui.end_row();
                        }
                    }
                });
            if listed.is_empty() {
                ui.add_space(8.0);
                widgets::hint(ui, "No sound rows match.");
            }
        });
    {
        let view = editor.views.model.view_mut(key);
        view.sound_search = search_text;
        view.sound_role = new_filter;
    }
    if let Some(sfx) = play {
        editor.views.model.sfx_to_play.push(sfx);
    }
    if let Some(sfx) = choose {
        let view = editor.views.model.view_mut(key);
        view.last_sound = Some(sfx.clone());
        if let Some((index, super::FrameCueKind::Sound)) = selected_cue
            && let Some(row) = editor.row_value(super::TABLE, key)
        {
            let mut frames = super::strip_of(&row, strip);
            if let Some(frame) = frames.get_mut(index) {
                frame.sound = Some(sfx.clone());
                let label = format!(
                    "Sound {sfx} on frame {} of {} of {key}",
                    index + 1,
                    strip.name()
                );
                super::write_strip(editor, key, strip, &frames, ContentEdit::new(label));
            }
        } else {
            editor.toast(format!("{sfx} goes on the next sound cue (S on a frame)"));
        }
    }
}

fn sound_row(
    ui: &mut Ui,
    sfx: &str,
    wave: Option<&Waveform>,
    on: bool,
    play: &mut Option<String>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(344.0, 24.0), Sense::click());
    let painter = ui.painter_at(rect);
    if on {
        painter.rect_filled(rect, 3.0, theme::SELECTION);
    } else if response.hovered() {
        painter.rect_filled(rect, 3.0, theme::RAISED);
    }
    let button = ERect::from_min_size(rect.min + Vec2::new(2.0, 2.0), Vec2::new(26.0, 20.0));
    painter.rect(
        button,
        3.0,
        theme::RAISED,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );
    painter.text(
        button.center(),
        egui::Align2::CENTER_CENTER,
        "▶",
        egui::FontId::proportional(11.0),
        theme::TEXT,
    );
    if ui
        .interact(button, response.id.with("play"), Sense::click())
        .clicked()
    {
        *play = Some(sfx.to_owned());
    }
    painter.text(
        egui::pos2(rect.min.x + 36.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        sfx,
        egui::FontId::monospace(12.5),
        if on { Color32::WHITE } else { theme::TEXT },
    );
    let wave_rect = ERect::from_min_max(
        egui::pos2(rect.min.x + 130.0, rect.min.y + 4.0),
        egui::pos2(rect.max.x - 60.0, rect.max.y - 4.0),
    );
    if let Some(wave) = wave {
        draw_wave(&painter, wave_rect, &wave.bins, theme::ACCENT);
        painter.text(
            egui::pos2(rect.max.x - 6.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            format!("{} ms", wave.ms),
            egui::FontId::monospace(11.0),
            theme::MUTED,
        );
    }
    response
}

pub fn volume_of(editor: &Editor, sfx: &str) -> (String, f32) {
    scalar_text(scalar_of(editor, sfx, "volume"))
}

pub fn pitch_of(editor: &Editor, sfx: &str) -> String {
    scalar_text(scalar_of(editor, sfx, "pitch")).0
}

fn scalar_of(editor: &Editor, sfx: &str, field: &str) -> SfxScalar {
    editor
        .row_value("sfx", sfx)
        .and_then(|row| serde_json::from_value(row[field].clone()).ok())
        .unwrap_or_default()
}

fn scalar_text(scalar: SfxScalar) -> (String, f32) {
    match scalar {
        SfxScalar::Fixed(fixed) => (format!("{fixed} · the row's"), fixed),
        SfxScalar::Random { min, max } => (format!("{min} to {max}, at random · the row's"), max),
    }
}

pub fn sync(editor: &mut Editor, world: &mut World) {
    let queued = std::mem::take(&mut editor.views.model.sfx_to_play);
    if queued.is_empty() {
        return;
    }
    let gain = AudioMix::default().gain(AudioCategory::Effects);
    let mut rng = Rng::new(editor.frame);
    for sfx in queued {
        let Some(file) = sfx_file(editor, &sfx) else {
            continue;
        };
        let Some(bytes) = editor
            .project
            .as_ref()
            .and_then(|project| project.files.get(&format!("assets/{file}")).cloned())
        else {
            continue;
        };
        let Ok(sound) = loader::decode(bytes.to_vec()).and_then(DecodedAudio::into_sound) else {
            continue;
        };
        let volume = scalar_of(editor, &sfx, "volume").resolve(&mut rng) * gain;
        let pitch = scalar_of(editor, &sfx, "pitch").resolve(&mut rng);
        let handle = world
            .resource_mut::<bevy::asset::Assets<AudioSource>>()
            .add(AudioSource { sound });
        world
            .resource::<Audio>()
            .play(handle)
            .with_volume(audio::decibels(volume))
            .with_playback_rate(f64::from(pitch));
    }
}
