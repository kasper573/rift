use std::collections::HashMap;
use ui::chip;
use ui::controls;
use ui::dialog;
use ui::egui;
use ui::form;
use ui::input;
use ui::panel;
use ui::sheet;
use ui::theme;

use engine::core::audio::playback::SfxDef;
use engine::core::content::{ContentFile, FolderReader};
use engine::systems::actor::ModelDef;
use engine::systems::actor::model::{MAX_FRAME_MS, SheetFrame};
use engine::systems::script::ScriptDef;
use ui::egui::{RichText, Sense, Ui, Vec2};
use ui::glyph::Mark;

use super::busts::{BustFace, FaceLine};
use super::strip::StripConfirm;
use super::{FrameCueKind, ModelStrip, strip::TimingSetBy, strip_of};
use crate::app::Editor;
use crate::forms::FormChange;
use crate::keymap::EditorKey;
use crate::picker;
use crate::project::{ContentEdit, RowSelection};
use crate::shell::{row_form, widgets};
use crate::tabs::shown_path;
use ui::input::{InputField, NumberField};

pub fn show(editor: &mut Editor, ui: &mut Ui, selection: &RowSelection) {
    let Some(key) = selection.keys.first().cloned() else {
        return;
    };
    let Some(part) = selection.parts.first().cloned() else {
        return;
    };
    let Some(row) = crate::views::read_shown::<ModelDef>(editor, ui, &key) else {
        return;
    };
    super::ensure_view(editor, &key);
    if let Some(rest) = part.strip_prefix("/sprites/actions/") {
        let bits: Vec<&str> = rest.split('/').collect();
        let Some(strip) =
            ModelStrip::parse(bits.first().unwrap_or(&""), bits.get(1).unwrap_or(&""))
        else {
            return;
        };
        let frames = strip_of(&row, strip);
        let crumb = format!("{} › {}", shown_path(&key), strip.name());
        egui::ScrollArea::vertical()
            .id_salt(("model-inspect", &key))
            .show(ui, |ui| {
                match (
                    bits.get(2).and_then(|i| i.parse::<usize>().ok()),
                    bits.get(3),
                ) {
                    (Some(index), Some(cue)) => {
                        if let Ok(kind) = cue.parse::<FrameCueKind>() {
                            ui.label(
                                theme::mono(format!("{crumb} › frame {}", index + 1))
                                    .color(theme::CODE)
                                    .size(12.0),
                            );
                            cue_card(editor, ui, &key, &frames, strip, index, kind);
                        }
                    }
                    (Some(index), None) if selection.parts.len() == 1 => {
                        sheet::section(ui, &format!("{} · frame {}", strip.name(), index + 1));
                        frame_card(editor, ui, &key, &frames, strip, index);
                    }
                    (Some(_), None) => {
                        let indices: Vec<usize> = selection
                            .parts
                            .iter()
                            .filter_map(|part| part.rsplit('/').next()?.parse().ok())
                            .collect();
                        ui.label(
                            theme::mono(format!(
                                "{crumb} › frames {}",
                                indices
                                    .iter()
                                    .map(|i| (i + 1).to_string())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ))
                            .color(theme::CODE)
                            .size(12.0),
                        );
                        frames_card(editor, ui, &key, &frames, strip, &indices);
                    }
                    _ => {
                        strip_card(editor, ui, &key, &row, &frames, strip);
                    }
                }
            });
        return;
    }
    let faces = super::busts::faces(&row);
    if let Some(face) = faces.iter().find(|face| face.part() == part).cloned() {
        egui::ScrollArea::vertical()
            .id_salt(("model-inspect", &key))
            .show(ui, |ui| {
                face_card(editor, ui, &key, &face, &faces);
            });
    }
}

fn strip_card(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    row: &ModelDef,
    frames: &[SheetFrame],
    strip: ModelStrip,
) {
    let timing = super::action_timing(row, strip.action);
    let Some(reader) = editor.reader() else {
        return;
    };
    sheet::section(ui, &strip.name());
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        form::kv_text(ui, "frames", frames.len().to_string());
        form::kv_text(ui, "lasts", format!("{} ms", super::total_ms(frames)));
        form::kv_text(ui, "plays", super::play_rule_long(strip.action));
        let cues: Vec<String> = frames
            .iter()
            .enumerate()
            .flat_map(|(index, frame)| {
                super::cues_of(frame, &reader)
                    .into_iter()
                    .map(move |cue| format!("{} on frame {}", cue.label(), index + 1))
            })
            .collect();
        form::kv_text(
            ui,
            "cues",
            if cues.is_empty() {
                "none".to_owned()
            } else {
                cues.join(" · ")
            },
        );
        if timing.out_of_step(strip.direction) {
            ui.horizontal(|ui| {
                ui.label(RichText::new("!").color(theme::WARN).strong());
                ui.label(theme::muted(format!(
                    "out of step: the action's other directions last {} ms over {} frames",
                    timing.common.iter().sum::<u32>(),
                    timing.common.len()
                )));
            });
        }
        ui.add_space(4.0);
        if controls::keyed(
            ui,
            "Open in the animation editor",
            Some(&EditorKey::Confirm.name()),
            true,
        )
        .clicked()
        {
            super::matrix::open_strip(editor, key, strip);
        }
    });
}

fn frame_card(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    frames: &[SheetFrame],
    strip: ModelStrip,
    index: usize,
) {
    let Some(frame) = frames.get(index).cloned() else {
        return;
    };
    let Some(reader) = editor.reader() else {
        return;
    };
    let mut changed = frame.clone();
    let mut label = None;
    panel::card(true).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::title(ui, "frame");
        ui.add_space(4.0);
        form::kv(ui, "lasts", |ui| {
            let mut ms = frame.ms;
            if NumberField::new(&mut ms)
                .range(1..=MAX_FRAME_MS)
                .unit("ms")
                .show(ui)
            {
                changed.ms = ms;
                label = Some(format!(
                    "Retime frame {} of {} of {key}",
                    index + 1,
                    strip.name()
                ));
            }
        });
        form::kv(ui, "apex", |ui| {
            let mut on = frame.apex;
            let settable = strip.action.takes_apex() || frame.apex;
            if ui
                .add_enabled(settable, egui::Checkbox::without_text(&mut on))
                .changed()
            {
                changed.apex = on;
                label = Some(if on {
                    format!("Apex on frame {} of {} of {key}", index + 1, strip.name())
                } else {
                    format!(
                        "No apex on frame {} of {} of {key}",
                        index + 1,
                        strip.name()
                    )
                });
            }
            ui.label(theme::muted(if strip.action.takes_apex() {
                "the hit lands here"
            } else {
                "only attack strips land a hit"
            }));
        });
        form::kv(ui, "sound", |ui| {
            let mut sound = sound_path(&reader, &frame);
            if sound_picker(
                editor,
                ui,
                key,
                &mut sound,
                &format!("{}/{index}/sound", strip.pointer()),
            ) {
                changed.sound = reader.id_of(&sound);
                label = Some(if sound.is_empty() {
                    format!(
                        "No sound on frame {} of {} of {key}",
                        index + 1,
                        strip.name()
                    )
                } else {
                    format!(
                        "Sound {sound} on frame {} of {} of {key}",
                        index + 1,
                        strip.name()
                    )
                });
            }
        });
        form::kv(ui, "step", |ui| {
            let mut on = frame.step;
            if ui.checkbox(&mut on, "").changed() {
                changed.step = on;
                label = Some(if on {
                    format!("Step on frame {} of {} of {key}", index + 1, strip.name())
                } else {
                    format!(
                        "No step on frame {} of {} of {key}",
                        index + 1,
                        strip.name()
                    )
                });
            }
        });
    });
    if let Some(label) = label {
        let mut written = frames.to_vec();
        if changed.apex && !frame.apex {
            for other in &mut written {
                other.apex = false;
            }
        }
        written[index] = changed;
        super::write_strip(editor, key, strip, &written, ContentEdit::new(label));
    }
}

fn frames_card(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    frames: &[SheetFrame],
    strip: ModelStrip,
    indices: &[usize],
) {
    let (set_by, text) = {
        let view = &editor.views.model.view(key);
        (view.timing.set_by, view.timing.text.clone())
    };
    let mut new_set_by = set_by;
    let mut new_text = text.clone();
    let mut apply = false;
    let planned = plan(frames, indices, set_by, &text);
    panel::card(true).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::header(ui, &format!("{} frames", indices.len()), "", |_| {});
        ui.add_space(4.0);
        form::kv(ui, "set by", |ui| {
            for (option, label) in [
                (TimingSetBy::Ms, "ms each"),
                (TimingSetBy::Fps, "fps"),
                (TimingSetBy::Scale, "scale"),
            ] {
                let on = set_by == option;
                if chip::mode(ui, label, on).clicked() {
                    new_set_by = option;
                    new_text.clear();
                }
            }
        });
        let (name, unit) = match set_by {
            TimingSetBy::Ms => ("ms each", "ms"),
            TimingSetBy::Fps => ("fps", "fps"),
            TimingSetBy::Scale => ("scale", "×"),
        };
        form::kv(ui, name, |ui| {
            let response = InputField::new(&mut new_text)
                .unit(unit)
                .hint(match set_by {
                    TimingSetBy::Ms => "100",
                    TimingSetBy::Fps => "10",
                    TimingSetBy::Scale => "1.25",
                })
                .show(ui);
            if input::committed(ui, &response) {
                apply = true;
            }
        });
        for index in indices {
            if let Some(frame) = frames.get(*index) {
                let after = planned.as_ref().and_then(|p| p.get(index)).copied();
                form::kv_text(
                    ui,
                    &format!("frame {}", index + 1),
                    match after {
                        Some(after) if after != frame.ms => format!("{} → {after} ms", frame.ms),
                        _ => format!("{} ms", frame.ms),
                    },
                );
            }
        }
        let before = super::total_ms(frames);
        let after = planned.as_ref().map(|p| {
            frames
                .iter()
                .enumerate()
                .map(|(i, frame)| p.get(&i).copied().unwrap_or(frame.ms))
                .sum::<u32>()
        });
        form::kv_text(
            ui,
            "strip",
            match after {
                Some(after) if after != before => format!("{before} → {after} ms"),
                _ => format!("{before} ms"),
            },
        );
    });
    if apply && let Some(planned) = plan(frames, indices, set_by, &new_text) {
        let mut written = frames.to_vec();
        for (index, ms) in &planned {
            if let Some(frame) = written.get_mut(*index) {
                frame.ms = *ms;
            }
        }
        if super::write_strip(
            editor,
            key,
            strip,
            &written,
            ContentEdit::new(format!(
                "Retime frames {} of {} of {key}",
                indices
                    .iter()
                    .map(|i| (i + 1).to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                strip.name()
            )),
        ) {
            new_text.clear();
        }
    }
    let view = editor.views.model.view_mut(key);
    view.timing.set_by = new_set_by;
    view.timing.text = new_text;
}

fn plan(
    frames: &[SheetFrame],
    indices: &[usize],
    set_by: TimingSetBy,
    text: &str,
) -> Option<HashMap<usize, u32>> {
    let value: f64 = text.trim().trim_start_matches('×').trim().parse().ok()?;
    if value <= 0.0 {
        return None;
    }
    let mut out = HashMap::new();
    for index in indices {
        let frame = frames.get(*index)?;
        let ms = match set_by {
            TimingSetBy::Ms => value.round(),
            TimingSetBy::Fps => (1000.0 / value + 0.5).floor(),
            TimingSetBy::Scale => (frame.ms as f64 * value).round(),
        };
        out.insert(*index, (ms.max(1.0) as u32).min(MAX_FRAME_MS));
    }
    Some(out)
}

fn cue_card(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    frames: &[SheetFrame],
    strip: ModelStrip,
    index: usize,
    kind: FrameCueKind,
) {
    let Some(frame) = frames.get(index).cloned() else {
        return;
    };
    let Some(reader) = editor.reader() else {
        return;
    };
    let starts = super::starts_at(frames, index);
    let total = super::total_ms(frames);
    let mut remove = false;
    let mut new_sound: Option<String> = None;
    let mut play = None;
    panel::card(true).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::header(ui, kind.key(), "", |ui| {
            if ui.button("Remove this cue").clicked() {
                remove = true;
                ui.close();
            }
        });
        ui.add_space(4.0);
        match kind {
            FrameCueKind::Sound => {
                let sound = sound_path(&reader, &frame);
                form::kv(ui, "sound", |ui| {
                    let play_chip = chip::plain(ui, Mark::Play.text(12.0));
                    if play_chip.on_hover_text("Play the sound at its row's volume").clicked() {
                        play = Some(sound.clone());
                    }
                    let mut picked = sound.clone();
                    let path = format!("{}/{index}/sound", strip.pointer());
                    if sound_picker(editor, ui, key, &mut picked, &path) && !picked.is_empty() {
                        new_sound = Some(picked);
                    }
                });
                let on_frame = format!("{} · starts at {starts} ms", index + 1);
                form::kv_text(ui, "on frame", on_frame);
                form::kv_text(ui, "volume", super::sounds::volume_of(editor, &sound));
                form::kv_text(ui, "pitch", super::sounds::pitch_of(editor, &sound));
                let wave = super::sounds::wave_of_sfx(editor, ui.ctx(), &sound);
                ui.add_space(4.0);
                let size = Vec2::new(ui.available_width() - 8.0, 44.0);
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                match &wave {
                    Some(wave) => {
                        super::sounds::draw_wave(ui.painter(), rect, &wave.bins, theme::ACCENT);
                    }
                    None => {
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "no file to measure",
                            egui::FontId::proportional(12.0),
                            theme::MUTED,
                        );
                    }
                }
                if let Some(wave) = &wave {
                    let left = total.saturating_sub(starts);
                    let heard = if wave.ms > left {
                        format!("{left} of its {} ms, past the strip's end", wave.ms)
                    } else {
                        format!("all of its {} ms, within the strip", wave.ms)
                    };
                    kv_wrapped(ui, "heard for", &heard);
                }
            }
            FrameCueKind::Apex => {
                let on_frame = format!("{} · {starts} ms into the swing", index + 1);
                form::kv_text(ui, "on frame", on_frame);
                let lands = "the hit, on server and client, when the swing reaches this frame";
                form::kv_text(ui, "lands", lands);
                if !strip.action.takes_apex() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("!").color(theme::WARN).strong());
                        let why = "an apex belongs in attack strips only; the folder will not load with it here";
                        ui.label(theme::muted(why));
                    });
                }
            }
            FrameCueKind::Step => {
                form::kv_text(ui, "on frame", format!("{} · {starts} ms in", index + 1));
                let plays = "the footstep of the surface underfoot; one per update however many steps pass";
                form::kv_text(ui, "plays", plays);
                ui.add_space(4.0);
                ui.label(theme::muted("hear it on").size(12.0));
                let chosen = editor.views.model.view(key).surface.clone();
                let surfaces = super::sounds::surfaces(editor);
                let mut pick_surface = None;
                for surface in &surfaces {
                    ui.horizontal(|ui| {
                        let on = chosen.as_deref() == Some(surface.key.as_str());
                        if chip::mode(ui, Mark::Play.text(12.0), on).clicked() {
                            pick_surface = Some((surface.key.clone(), surface.footstep.clone()));
                        }
                        ui.label(theme::mono(surface.name.to_lowercase()).size(12.5));
                    });
                }
                if let Some((surface, footstep)) = pick_surface {
                    let view = editor.views.model.view_mut(key);
                    view.surface = Some(surface);
                    play = Some(footstep);
                }
            }
        }
    });
    ui.add_space(8.0);
    if ui.button("Copy cues to the other 7 directions…").clicked() {
        let view = editor.views.model.view_mut(key);
        view.confirm = Some(StripConfirm::Cues);
    }
    if let Some(sound) = play {
        editor.views.sound.play.push(sound);
    }
    if let Some(sound) = new_sound
        && let Some(id) = reader.id_of(&sound)
    {
        let mut written = frames.to_vec();
        written[index].sound = Some(id);
        if super::write_strip(
            editor,
            key,
            strip,
            &written,
            ContentEdit::new(format!(
                "Sound {sound} on frame {} of {} of {key}",
                index + 1,
                strip.name()
            )),
        ) {
            let view = editor.views.model.view_mut(key);
            view.last_sound = Some(sound);
        }
    }
    if remove {
        let mut written = frames.to_vec();
        match kind {
            FrameCueKind::Apex => written[index].apex = false,
            FrameCueKind::Sound => written[index].sound = None,
            FrameCueKind::Step => written[index].step = false,
        }
        if super::write_strip(
            editor,
            key,
            strip,
            &written,
            ContentEdit::new(format!(
                "No {} on frame {} of {} of {key}",
                kind.key(),
                index + 1,
                strip.name()
            )),
        ) {
            let view = editor.views.model.view_mut(key);
            view.cue = None;
            super::select_frames(editor, key, strip);
        }
    }
}

fn size_text(
    face: &str,
    own: Option<[usize; 2]>,
    sizes: &[(String, Option<[usize; 2]>)],
) -> String {
    let Some(size) = own else {
        return "missing file".to_owned();
    };
    let others: Vec<(&str, [usize; 2])> = sizes
        .iter()
        .filter(|(name, _)| name != face)
        .filter_map(|(name, other)| Some((name.as_str(), (*other)?)))
        .collect();
    let differing = others.iter().find(|(_, other)| *other != size);
    match differing {
        _ if others.is_empty() => format!("{} × {}", size[0], size[1]),
        None => format!("{} × {} · same as the others", size[0], size[1]),
        Some((name, other)) => format!(
            "{} × {} · differs from {name} ({} × {}); every bust of one model is one size",
            size[0], size[1], other[0], other[1]
        ),
    }
}

fn kv_wrapped(ui: &mut Ui, label: &str, text: &str) {
    ui.horizontal_top(|ui| {
        ui.add_sized(
            Vec2::new(96.0, ui.spacing().interact_size.y),
            egui::Label::new(theme::muted(label)),
        );
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).size(12.5));
        });
    });
}

/// The path of the sound `frame` plays, empty when it plays none.
fn sound_path(reader: &FolderReader, frame: &SheetFrame) -> String {
    frame
        .sound
        .and_then(|sound| reader.path_of(sound))
        .unwrap_or_default()
        .to_owned()
}

fn sound_picker(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    sound: &mut String,
    path: &str,
) -> bool {
    row_form::with_row_form(editor, ModelDef::TABLE, key, |cx| {
        picker::row(ui, cx, SfxDef::TABLE, sound, path)
    })
}

fn face_card(editor: &mut Editor, ui: &mut Ui, key: &str, face: &BustFace, faces: &[BustFace]) {
    ui.label(
        theme::mono(format!("{} › bust › {}", shown_path(key), face.name))
            .color(theme::CODE)
            .size(12.0),
    );
    ui.add_space(4.0);
    let ctx = ui.ctx().clone();
    let sizes: Vec<(String, Option<[usize; 2]>)> = faces
        .iter()
        .map(|other| {
            let size = editor.thumb(&ctx, &other.file).map(|thumb| thumb.size);
            (other.name.clone(), size)
        })
        .collect();
    let own_size = sizes
        .iter()
        .find(|(name, _)| *name == face.name)
        .and_then(|(_, size)| *size);
    let mut rename_to: Option<String> = None;
    let mut remove_with: Option<String> = None;
    let lines = super::busts::lines(editor, key);
    let uses_of_face = lines.iter().filter(|line| line.face == face.name).count();
    panel::card(true).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let kind = if face.own { "" } else { "generic" };
        panel::header(ui, &face.name, kind, |ui| {
            if face.own && ui.button("Remove this face…").clicked() {
                remove_with = Some(String::new());
                ui.close();
            }
        });
        ui.add_space(4.0);
        form::kv(ui, "name", |ui| {
            if face.own {
                let id = ui.id().with(("face-name", &face.file));
                let typed = input::drafted(ui, id, &face.name, |ui, draft| {
                    InputField::new(draft).show(ui)
                });
                let wanted = typed.map(|typed| typed.trim().to_owned());
                if let Some(wanted) = wanted
                    && !wanted.is_empty()
                    && wanted != face.name
                {
                    rename_to = Some(wanted);
                }
            } else {
                ui.label(theme::mono(&face.name));
                ui.label(theme::faint("one of the six every bust has"));
            }
        });
        form::kv(ui, "file", |ui| {
            ui.label(theme::mono(face.part()).color(theme::CODE).size(12.5));
            ui.label(theme::faint("drop a PNG onto its card to replace it").size(12.0));
        });
        form::kv_text(ui, "size", size_text(&face.name, own_size, &sizes));
    });
    let own_lines: Vec<&FaceLine> = lines
        .iter()
        .filter(|line| faces.iter().any(|f| f.own && f.name == line.face))
        .collect();
    let title = super::busts::speaker_name(editor, key);
    form::section_count(
        ui,
        &format!("Lines with {title}'s own faces"),
        own_lines.len(),
    );
    let shown = 6;
    let mut open = None;
    for line in own_lines.iter().take(shown) {
        ui.horizontal(|ui| {
            widgets::badge(ui, ScriptDef::TABLE);
            if ui
                .link(theme::mono(shown_path(&line.script)).size(12.5))
                .clicked()
            {
                open = Some(line.script.clone());
            }
            ui.label(theme::muted("›").size(12.0));
            ui.label(theme::mono(&line.node).size(12.5));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(theme::faint(&line.face).size(12.0));
            });
        });
    }
    if own_lines.len() > shown {
        ui.label(theme::faint(format!("… {} more", own_lines.len() - shown)));
    }
    if own_lines.is_empty() {
        form::hint(ui, "no line asks for an own face yet");
    }
    if let Some(script) = open {
        editor.open_row(ScriptDef::TABLE, &script);
    }
    if let Some(wanted) = rename_to
        && super::busts::rename(editor, key, face, &wanted)
    {
        let view = editor.views.model.view_mut(key);
        view.face = Some(wanted);
    }
    if remove_with.is_some() {
        remove_dialog(editor, key, face, faces, uses_of_face);
    }
}

fn remove_dialog(editor: &mut Editor, key: &str, face: &BustFace, faces: &[BustFace], uses: usize) {
    let view = editor.views.model.view_mut(key);
    view.removing = Some(super::FaceRemoval {
        face: face.clone(),
        replacement: faces
            .iter()
            .find(|f| f.name != face.name)
            .map(|f| f.name.clone())
            .unwrap_or_default(),
        uses,
    });
}

pub fn remove_face_dialog(editor: &mut Editor, ctx: &egui::Context, key: &str) {
    let Some(removing) = editor
        .views
        .model
        .views
        .get(key)
        .and_then(|view| view.removing.clone())
    else {
        return;
    };
    let faces = match editor.read::<ModelDef>(key) {
        Some(Ok(model)) => super::busts::faces(&model),
        _ => Vec::new(),
    };
    let mut decision = None;
    let mut replacement = removing.replacement.clone();
    dialog::card(ctx, "remove-face", 400.0, |ui| {
        ui.label(
            RichText::new(format!("Remove face {}", removing.face.name))
                .size(16.0)
                .strong(),
        );
        ui.add_space(6.0);
        if removing.uses > 0 {
            ui.label(format!(
                "{} lines ask for it. They take another face instead:",
                removing.uses
            ));
            controls::combo("face-replacement")
                .selected_text(replacement.clone())
                .show_ui(ui, |ui| {
                    for other in faces.iter().filter(|f| f.name != removing.face.name) {
                        ui.selectable_value(&mut replacement, other.name.clone(), &other.name);
                    }
                });
        } else {
            ui.label(theme::muted("No line asks for it."));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if controls::danger(ui, "Remove").clicked() {
                decision = Some(true);
            }
            if ui.button("Cancel").clicked() {
                decision = Some(false);
            }
        });
        if ui.input(|i| EditorKey::Cancel.pressed(i)) {
            decision = Some(false);
        }
    });
    {
        let view = editor.views.model.view_mut(key);
        if let Some(removing) = view.removing.as_mut() {
            removing.replacement = replacement.clone();
        }
    }
    let Some(apply) = decision else {
        return;
    };
    let view = editor.views.model.view_mut(key);
    view.removing = None;
    if apply && super::busts::remove(editor, key, &removing.face, &replacement) {
        let view = editor.views.model.view_mut(key);
        view.face = None;
        editor.select(RowSelection::row(ModelDef::TABLE, key));
    }
}

pub fn row_body(ui: &mut Ui, model: &mut ModelDef) -> FormChange {
    let mut changed = FormChange::default();
    form::kv(ui, "hitbox", |ui| {
        let hitbox = &mut model.hitbox;
        if NumberField::of([&mut hitbox.width, &mut hitbox.height])
            .range(0.1..=8.0)
            .unit("m")
            .show(ui)
        {
            changed = FormChange::at("/hitbox");
        }
    });
    form::kv_text(ui, "image", shown_path(model.sprites.image.as_str()));
    let size = model.sprites.frame_size;
    form::kv_text(
        ui,
        "frame",
        format!("{} × {} px, imported", size.width, size.height),
    );
    let strips: usize = model
        .sprites
        .actions
        .values()
        .map(|strips| strips.len())
        .sum();
    let frames: usize = ModelStrip::all()
        .map(|strip| strip_of(model, strip).len())
        .sum();
    form::kv_text(ui, "strips", format!("{strips} · {frames} frames"));
    changed
}
